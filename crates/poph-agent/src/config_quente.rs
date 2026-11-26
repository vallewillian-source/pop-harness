//! Cache de configuração quente do agente (PRD §6.1 e §7.1).
//!
//! O turno precisa de um punhado de dados que muda pouco: o agente (modelo, limites, top-k, Jev),
//! as instruções compostas (o prefixo estável do prompt) e a lista de ferramentas habilitadas
//! (decisão D-04). Reler isso do SQLite a cada turno é barato, mas não é de graça; aqui fica a
//! cópia quente, particionada por org.
//!
//! Regras:
//! - **Invalidação por versão.** [`ConfigQuente::carregar_verificando`] lê só o número de versão do
//!   agente (uma consulta por chave primária) e recarrega quando ele mudou. `carregar` usa só a
//!   validade, para quando o chamador já sabe que a versão não mudou.
//! - **Teto de memória.** O cache é limitado por bytes estimados (weigher) e não cresce além do
//!   teto, mesmo com 10 mil agentes (teste em `tests/memoria_e_concorrencia.rs`).
//! - **D-05 vale aqui.** Carregar agente `inactive`/`archived` falha com
//!   [`poph_types::DomainError::AgenteInativo`]: nada de cachear agente que não pode responder.
//! - **Ferramentas filtram (D-04).** [`AgenteQuente::ferramentas`] é a lista fechada do que o LLM
//!   pode chamar; ferramenta desabilitada não entra.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use moka::sync::Cache;
use poph_store::{Agente, Instructions, Store};
use poph_types::{AgentId, OrgId, StructuredInstructions};

use crate::erro::{ErroAgente, Resultado};
use crate::metricas::MetricasMemoria;

/// Chave do cache: sempre dentro de uma org (nada de chave global para id de agente).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Chave {
    org: OrgId,
    agente: AgentId,
}

/// Ajustes do cache.
#[derive(Debug, Clone)]
pub struct OpcoesConfigQuente {
    /// Quanto tempo uma entrada vale sem verificação de versão.
    pub validade: Duration,
    /// Teto de memória estimada, em bytes.
    pub teto_bytes: u64,
}

impl Default for OpcoesConfigQuente {
    fn default() -> Self {
        Self {
            // 60 s: o Console e a CLI alteram o agente, e a versão é conferida quando importa.
            validade: Duration::from_secs(60),
            // 64 MB: cabe com folga os 10 mil agentes da meta M3 (D-07) com instruções médias.
            teto_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Configuração quente de um agente, pronta para o turno.
#[derive(Debug, Clone)]
pub struct AgenteQuente {
    /// Agente como está no banco (só `active` chega aqui).
    pub agente: Agente,
    /// Instruções gravadas (`None` quando o agente ainda não tem).
    pub instrucoes: Option<Instructions>,
    /// JSON estruturado já interpretado (`None` quando ausente ou ilegível).
    pub estruturado: Option<StructuredInstructions>,
    /// Texto do prefixo estável do prompt (o markdown gravado, inclusive editado à mão).
    pub markdown: String,
    /// Nomes das ferramentas habilitadas, na ordem das instruções (D-04).
    pub ferramentas: Vec<String>,
    /// Top-k do RAG deste agente.
    pub top_k: u32,
    /// Corte mínimo de relevância do RAG.
    pub corte: f64,
    /// Versão das instruções que originou esta cópia (0 quando não há instruções).
    pub versao_instrucoes: u64,
    /// Bytes estimados que a entrada ocupa (usado pelo weigher).
    pub bytes: u64,
}

impl AgenteQuente {
    /// Versão do agente que originou esta cópia.
    pub fn versao(&self) -> u64 {
        self.agente.version
    }

    /// Par (versão do agente, versão das instruções) usado na invalidação.
    pub fn versao_composta(&self) -> (u64, u64) {
        (self.agente.version, self.versao_instrucoes)
    }

    /// `true` quando o agente pode abrir turno (D-05).
    pub fn pode_abrir_turno(&self) -> bool {
        self.agente.can_open_turn()
    }

    /// Ferramenta habilitada? É o filtro que a HR-13 aplica no laço de ferramentas.
    pub fn ferramenta_habilitada(&self, nome: &str) -> bool {
        self.ferramentas.iter().any(|habilitada| habilitada == nome)
    }
}

/// Cache de configuração quente por agente, particionado por org.
pub struct ConfigQuente {
    store: Arc<Store>,
    cache: Cache<Chave, Arc<AgenteQuente>>,
    metricas: MetricasMemoria,
}

impl ConfigQuente {
    /// Cria o cache. O `Store` é compartilhado (o pool de leitores já é concorrente).
    pub fn nova(store: Arc<Store>, opcoes: OpcoesConfigQuente, metricas: MetricasMemoria) -> Self {
        let teto = opcoes.teto_bytes.max(1024);
        let cache = Cache::builder()
            .max_capacity(teto)
            .weigher(|_chave: &Chave, valor: &Arc<AgenteQuente>| {
                u32::try_from(valor.bytes.min(u64::from(u32::MAX))).unwrap_or(u32::MAX)
            })
            .time_to_live(opcoes.validade)
            .build();
        Self {
            store,
            cache,
            metricas,
        }
    }

    /// Lê do cache; se não houver, carrega do banco. Não confere a versão (usa só a validade).
    pub fn carregar(&self, org: OrgId, agente: AgentId) -> Resultado<Arc<AgenteQuente>> {
        let chave = Chave { org, agente };
        if let Some(quente) = self.cache.get(&chave) {
            self.metricas.config_acertos.increment(1);
            self.atualizar_tamanho();
            return Ok(quente);
        }
        self.recarregar(chave)
    }

    /// Lê do cache **conferindo a versão do agente** no banco: mudou a versão, recarrega.
    ///
    /// É o caminho que o turno usa quando pode ter havido edição do agente (Console, CLI).
    pub fn carregar_verificando(
        &self,
        org: OrgId,
        agente: AgentId,
    ) -> Resultado<Arc<AgenteQuente>> {
        let chave = Chave { org, agente };
        let versao = self.store.agents().version(org, agente)?;
        let versao_instrucoes = self.store.instructions().version(org, agente)?.unwrap_or(0);
        if let Some(quente) = self.cache.get(&chave)
            && quente.versao_composta() == (versao, versao_instrucoes)
        {
            self.metricas.config_acertos.increment(1);
            self.atualizar_tamanho();
            return Ok(quente);
        }
        if self.cache.contains_key(&chave) {
            self.metricas.config_invalidadas.increment(1);
        }
        self.recarregar(chave)
    }

    /// Descarta a entrada do agente (o próximo carregamento relê o banco).
    pub fn invalidar(&self, org: OrgId, agente: AgentId) {
        self.cache.invalidate(&Chave { org, agente });
        self.atualizar_tamanho();
    }

    /// Descarta todas as entradas de uma org (exclusão de org, importação em lote).
    pub fn esquecer_org(&self, org: OrgId) {
        let chaves: HashSet<Chave> = self.cache.iter().map(|(chave, _)| *chave).collect();
        for chave in chaves {
            if chave.org == org {
                self.cache.invalidate(&chave);
            }
        }
        self.atualizar_tamanho();
    }

    /// Força o cache a aplicar as tarefas pendentes (expiração e despejo por peso).
    ///
    /// As leituras de tamanho/memória são exatas só depois disto; as métricas chamam na sequência.
    pub fn sincronizar(&self) {
        self.cache.run_pending_tasks();
        self.atualizar_tamanho();
    }

    /// Agentes com config quente carregada.
    pub fn tamanho(&self) -> u64 {
        self.cache.entry_count()
    }

    /// Memória estimada ocupada, em bytes.
    pub fn bytes(&self) -> u64 {
        self.cache.weighted_size()
    }

    fn recarregar(&self, chave: Chave) -> Resultado<Arc<AgenteQuente>> {
        self.metricas.config_erros.increment(1);
        let agente = self.store.agents().get_for_turn(chave.org, chave.agente)?;
        let instrucoes = match self.store.instructions().get(chave.org, chave.agente) {
            Ok(instrucoes) => Some(instrucoes),
            Err(erro) if erro.is_not_found() => None,
            Err(erro) => return Err(ErroAgente::Store(erro)),
        };
        let quente = Arc::new(montar(agente, instrucoes));
        self.cache.insert(chave, Arc::clone(&quente));
        self.atualizar_tamanho();
        Ok(quente)
    }

    fn atualizar_tamanho(&self) {
        let agentes = self.cache.entry_count();
        self.metricas
            .agentes_carregados
            .set(i64::try_from(agentes).unwrap_or(i64::MAX));
        self.metricas
            .config_bytes
            .set(i64::try_from(self.cache.weighted_size()).unwrap_or(i64::MAX));
    }
}

fn montar(agente: Agente, instrucoes: Option<Instructions>) -> AgenteQuente {
    // O JSON pode vir de importação antiga (duplo encoding): `de_json_tolerante` aceita os dois.
    let estruturado = instrucoes.as_ref().and_then(|instrucoes| {
        StructuredInstructions::from_tolerant_json(&instrucoes.structured_json).ok()
    });
    let ferramentas: Vec<String> = estruturado
        .as_ref()
        .map(|dados| {
            dados
                .enabled_tools()
                .into_iter()
                .map(|ferramenta| ferramenta.name.clone())
                .collect()
        })
        .unwrap_or_default();
    // O markdown gravado é a fonte do prefixo: pode ter sido editado à mão e nunca é recomposto aqui.
    let markdown = instrucoes
        .as_ref()
        .map(|instrucoes| instrucoes.markdown.clone())
        .unwrap_or_default();
    let top_k = agente.rag_top_k;
    let corte = agente.rag_cutoff;
    let versao_instrucoes = instrucoes
        .as_ref()
        .map(|instrucoes| instrucoes.version)
        .unwrap_or(0);
    let bytes = estimar_bytes(
        &agente,
        instrucoes.as_ref(),
        &estruturado,
        &markdown,
        &ferramentas,
    );
    AgenteQuente {
        agente,
        instrucoes,
        estruturado,
        markdown,
        ferramentas,
        top_k,
        corte,
        versao_instrucoes,
        bytes,
    }
}

/// Estimativa grosseira de memória da entrada: texto conta 1 byte por byte, e cada campo tem um
/// custo fixo. Serve de peso no cache — precisão de byte não muda o comportamento.
fn estimar_bytes(
    agente: &Agente,
    instrucoes: Option<&Instructions>,
    estruturado: &Option<StructuredInstructions>,
    markdown: &str,
    ferramentas: &[String],
) -> u64 {
    const FIXO: u64 = 512;
    let mut bytes = FIXO;
    bytes += agente.name.len() as u64
        + agente.slug.len() as u64
        + agente.description.len() as u64
        + agente.provedor.len() as u64
        + agente.modelo.len() as u64
        + agente.jev_json.len() as u64;
    bytes += markdown.len() as u64;
    if let Some(instrucoes) = instrucoes {
        bytes += instrucoes.structured_json.len() as u64;
    }
    if let Some(estruturado) = estruturado {
        // O JSON já foi contado; as strings do tipo são as mesmas, então não soma de novo.
        bytes += estruturado.identity.name.len() as u64;
    }
    for ferramenta in ferramentas {
        bytes += ferramenta.len() as u64;
    }
    bytes
}
