//! Operações dos fluxos sem HTTP (FX-31): compilar, validar, criar, publicar, comparar, pré-visualizar
//! e formatar.
//!
//! A API (`api/fluxos.rs`) e a CLI (`poph flow`) chamam estas funções, então a regra de publicação
//! tem um caminho só: nada entra numa revisão sem passar pelo compilador e pela validação semântica,
//! e a versão do fluxo é conferida antes de compilar (a corrida final é resolvida pelo repositório,
//! FX-09).
//!
//! Erros de negócio saem como [`FalhaFluxo`]. A API traduz cada caso para o status HTTP; a CLI, para
//! a mensagem de saída. Erro de banco passa como [`FalhaFluxo::Banco`].

use std::fmt;
use std::sync::Arc;

use poph_flows::{
    Compiled, Departamento, Ir, Limites, RevisionKey, compile, diff, format_with,
    resolve_departments, stage_block, validar,
};

// Tipos que a CLI precisa para mostrar o resultado. Reexportados para o `poph` não depender de
// `poph-flows` diretamente.
pub use poph_flows::{Change, Diagnostic, Resolution, RevisionCache};
use poph_store::{
    FlowRevision, Fluxo, NewAuditEntry, NewFlow, Page, RevisionPublication, Store, StoreError,
};
use poph_types::{DomainError, FlowId, HarnessConfig, OrgId};
use serde_json::{Value, json};

/// Quantos departamentos a resolução lê por página (a org tem poucos; o laço garante o resto).
const LIMITE_DEPARTAMENTOS: u32 = 200;

/// Quem fez a operação, como o repositório guarda na publicação e na auditoria.
#[derive(Debug, Clone)]
pub struct Ator {
    /// Tipo do autor no vocabulário de `audit_log.actor_kind` (`admin_token`, `cli`...).
    pub tipo: String,
    /// Id do autor, quando há um (o id do token da org na API; vazio na CLI).
    pub id: Option<String>,
}

impl Ator {
    /// Token de administração de uma org (a API).
    pub fn token(id: String) -> Self {
        Self {
            tipo: "admin_token".to_owned(),
            id: Some(id),
        }
    }

    /// Linha de comando local (`poph flow`), que opera direto no banco.
    pub fn cli() -> Self {
        Self {
            tipo: "cli".to_owned(),
            id: None,
        }
    }
}

/// Por que uma operação de fluxo não foi feita.
#[derive(Debug)]
pub enum FalhaFluxo {
    /// `If-Match` (ou a versão informada) não bate com a versão atual do fluxo.
    VersaoVelha {
        id: String,
        esperada: u64,
        atual: u64,
    },
    /// Script em branco: não há o que publicar.
    ScriptVazio,
    /// Script com erro do compilador ou da validação semântica. A lista vai junto, com linha e coluna.
    ScriptInvalido(Vec<Diagnostic>),
    /// O fluxo só tem rascunho: ainda não há revisão publicada.
    SemRevisao,
    /// A IR gravada não confere com o texto publicado. O código é o do cache (`ir_divergente` ou
    /// `fluxo_invalido`).
    IrDivergente(&'static str),
    /// A IR gravada não lê, ou o compilador mudou de contrato: defeito interno.
    Interno,
    /// A org já tem o máximo de fluxos da configuração.
    TetoDeFluxos { atual: u64, teto: u64 },
    /// Erro do banco ou de domínio, sem tradução própria.
    Banco(StoreError),
}

impl From<StoreError> for FalhaFluxo {
    fn from(erro: StoreError) -> Self {
        Self::Banco(erro)
    }
}

impl fmt::Display for FalhaFluxo {
    /// Frase em português para a CLI. A API usa os campos, não esta frase.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VersaoVelha {
                esperada, atual, ..
            } => write!(
                f,
                "a versão não confere: você informou a versão {esperada}, mas o fluxo está na \
                 versão {atual}. Leia de novo e repita com a versão atual"
            ),
            Self::ScriptVazio => write!(f, "o script está vazio: não há o que publicar"),
            Self::ScriptInvalido(diagnosticos) => {
                let erros = diagnosticos.iter().filter(|d| d.is_error()).count();
                write!(
                    f,
                    "o script não passa na validação: {erros} erro(s) a corrigir; nada foi gravado"
                )
            }
            Self::SemRevisao => write!(
                f,
                "o fluxo ainda não tem revisão publicada: publique o rascunho antes"
            ),
            Self::IrDivergente(_) => write!(
                f,
                "a revisão publicada não confere com o texto guardado: a leitura falha fechada"
            ),
            Self::Interno => write!(
                f,
                "erro interno ao ler ou gravar o fluxo (veja o log do servidor)"
            ),
            Self::TetoDeFluxos { atual, teto } => {
                write!(f, "a organização já tem {atual} fluxos e o teto é {teto}")
            }
            Self::Banco(erro) => write!(f, "{erro}"),
        }
    }
}

/// Resultado da validação de um script: a IR (quando compila), os diagnósticos e as resoluções de
/// departamento (só quando há org).
#[derive(Debug)]
pub struct Validacao {
    pub ir: Option<Ir>,
    pub diagnosticos: Vec<Diagnostic>,
    pub resolucoes: Vec<Resolution>,
}

impl Validacao {
    /// `true` quando nenhum diagnóstico é erro (avisos não impedem).
    pub fn valido(&self) -> bool {
        !self.diagnosticos.iter().any(Diagnostic::is_error)
    }
}

/// O que uma publicação produziu: a revisão gravada, os avisos do script e a versão nova do fluxo.
#[derive(Debug)]
pub struct PublicacaoFeita {
    pub revisao: FlowRevision,
    pub avisos: Vec<Diagnostic>,
    pub versao_do_fluxo: u64,
}

/// O que a criação produziu: o fluxo novo e, se foi pedido, a revisão 1.
#[derive(Debug)]
pub struct CriacaoFeita {
    pub fluxo: Fluxo,
    pub publicacao: Option<PublicacaoFeita>,
}

/// Duas revisões lado a lado, com o que mudou e quantos atendimentos estão em cada uma.
#[derive(Debug)]
pub struct Comparacao {
    pub de: FlowRevision,
    pub para: FlowRevision,
    pub mudancas: Vec<Change>,
    pub ativas_de: u64,
    pub ativas_para: u64,
}

/// O bloco de prompt que o agente veria numa etapa.
#[derive(Debug)]
pub struct Previa {
    pub etapa: String,
    pub revisao: u64,
    pub bloco: String,
}

/// Operações dos fluxos de uma instância: os repositórios, o cache de revisões e os tetos da
/// configuração `[fluxos]`.
pub struct Operacoes<'a> {
    store: &'a Store,
    cache: &'a RevisionCache,
    limites: Limites,
    max_fluxos_por_org: u64,
}

impl<'a> Operacoes<'a> {
    /// Liga as operações ao banco, ao cache e à configuração.
    pub fn novo(store: &'a Store, cache: &'a RevisionCache, config: &HarnessConfig) -> Self {
        Self {
            store,
            cache,
            limites: Limites::from_config(&config.flows),
            max_fluxos_por_org: u64::from(config.flows.max_flows_per_org),
        }
    }

    /// Compila o script com os tetos da configuração (tamanho e nós).
    pub fn compilar(&self, fonte: &str) -> Compiled {
        compile(fonte, &self.limites)
    }

    /// Lê todos os departamentos da org, para resolver `encaminhar("...")`.
    pub fn departamentos(&self, org: OrgId) -> Result<Vec<Departamento>, StoreError> {
        let mut todos: Vec<Departamento> = Vec::new();
        let mut apos: Option<uuid::Uuid> = None;
        loop {
            let lote = self.store.departments().list(
                org,
                Page {
                    after: apos,
                    limit: LIMITE_DEPARTAMENTOS,
                },
            )?;
            if lote.is_empty() {
                return Ok(todos);
            }
            let cheio = lote.len() >= LIMITE_DEPARTAMENTOS as usize;
            apos = lote.last().map(|departamento| departamento.id.as_uuid());
            todos.extend(lote.into_iter().map(|departamento| Departamento {
                id: departamento.id.to_string(),
                nome: departamento.name,
            }));
            if !cheio {
                return Ok(todos);
            }
        }
    }

    /// Compila e valida um script sem gravar nada. Com `org`, também resolve os departamentos citados.
    pub fn validar(&self, org: Option<OrgId>, fonte: &str) -> Result<Validacao, StoreError> {
        let compilado = self.compilar(fonte);
        let mut diagnosticos = compilado.diagnostics;
        let mut resolucoes = Vec::new();
        if let (Some(ir), Some(org)) = (&compilado.ir, org) {
            let (encontradas, avisos) = resolve_departments(ir, &self.departamentos(org)?);
            diagnosticos.extend(avisos);
            resolucoes = encontradas;
        }
        Ok(Validacao {
            ir: compilado.ir,
            diagnosticos,
            resolucoes,
        })
    }

    /// Devolve o script na forma canônica, ou os diagnósticos se ele não compila.
    pub fn formatar(&self, fonte: &str) -> Result<String, FalhaFluxo> {
        format_with(fonte, &self.limites)
            .map_err(|falha| FalhaFluxo::ScriptInvalido(falha.diagnostics))
    }

    /// Cria o fluxo a partir do script (o `id` do script vira o slug). Com `publicar`, grava também a
    /// revisão 1 na mesma chamada. A criação é auditada antes da publicação.
    pub fn criar(
        &self,
        org: OrgId,
        fonte: &str,
        publicar: bool,
        ator: &Ator,
    ) -> Result<CriacaoFeita, FalhaFluxo> {
        let compilado = self.compilar(fonte);
        if compilado.has_error() {
            return Err(FalhaFluxo::ScriptInvalido(compilado.diagnostics));
        }
        let Some(ir) = compilado.ir else {
            tracing::error!("o compilador devolveu script sem erro e sem IR");
            return Err(FalhaFluxo::Interno);
        };
        // Teto de fluxos por org (PRD §9). Conta todos os fluxos, ativos ou não.
        let hoje = self.store.flows().count_rows(org)?;
        if hoje >= self.max_fluxos_por_org {
            return Err(FalhaFluxo::TetoDeFluxos {
                atual: hoje,
                teto: self.max_fluxos_por_org,
            });
        }
        let fluxo = self.store.flows().create(
            org,
            NewFlow {
                slug: ir.fluxo.id.clone(),
                name: ir.fluxo.nome.clone(),
                description: ir.fluxo.descricao.clone().unwrap_or_default(),
                rascunho_fonte: fonte.to_owned(),
            },
        )?;
        auditar(
            self.store,
            org,
            ator,
            "flow_create",
            "flow",
            fluxo.id.to_string(),
        )?;
        let publicacao = if publicar {
            Some(self.publicar(org, &fluxo, fluxo.version, fonte, ator)?)
        } else {
            None
        };
        Ok(CriacaoFeita { fluxo, publicacao })
    }

    /// Compila, valida, resolve departamentos e publica — o caminho único de publicação.
    ///
    /// `esperada` é a `version` do fluxo que quem chama leu. A recusa por versão velha acontece
    /// **antes** de compilar (é o mesmo conflito que o repositório daria, sem gastar compilação); o
    /// `UPDATE` condicional da FX-09 é quem resolve a corrida de verdade. Nos casos de recusa, nada
    /// é gravado.
    pub fn publicar(
        &self,
        org: OrgId,
        fluxo: &Fluxo,
        esperada: u64,
        fonte: &str,
        ator: &Ator,
    ) -> Result<PublicacaoFeita, FalhaFluxo> {
        if esperada != fluxo.version {
            return Err(FalhaFluxo::VersaoVelha {
                id: fluxo.id.to_string(),
                esperada,
                atual: fluxo.version,
            });
        }
        if fonte.trim().is_empty() {
            return Err(FalhaFluxo::ScriptVazio);
        }
        let compilado = self.compilar(fonte);
        if compilado.has_error() {
            return Err(FalhaFluxo::ScriptInvalido(compilado.diagnostics));
        }
        let Some(ir) = compilado.ir else {
            // `tem_erro` falso sempre traz IR; se não trouxe, o compilador mudou de contrato.
            tracing::error!("o compilador devolveu script sem erro e sem IR");
            return Err(FalhaFluxo::Interno);
        };
        // Segunda porta: a IR compilada passa de novo pelo validador semântico. Hoje isto nunca
        // recusa (o compilador já confere), mas a publicação não depende de o compilador ter razão.
        let erros_da_ir: Vec<Diagnostic> =
            validar(&ir).into_iter().filter(|d| d.is_error()).collect();
        if !erros_da_ir.is_empty() {
            return Err(FalhaFluxo::ScriptInvalido(erros_da_ir));
        }
        let (resolucoes, avisos_de_departamento) =
            resolve_departments(&ir, &self.departamentos(org)?);
        let definicao_json = serde_json::to_string(&ir).map_err(|erro| {
            tracing::error!(erro = %erro, "a IR compilada não serializa");
            FalhaFluxo::Interno
        })?;
        let resolucoes_json = serde_json::to_string(&json!({ "departamentos": resolucoes }))
            .map_err(|_| FalhaFluxo::Interno)?;
        let revisao = self.store.revisions().publish_revision(
            org,
            fluxo.id,
            RevisionPublication {
                expected: esperada,
                fonte: fonte.to_owned(),
                definicao_json,
                schema_versao: Some(ir.schema_versao),
                resolucoes_json,
                ator_tipo: ator.tipo.clone(),
                actor_id: ator.id.clone(),
            },
        )?;
        let versao_do_fluxo = self.store.flows().get(org, fluxo.id)?.version;
        let mut avisos: Vec<Diagnostic> = compilado
            .diagnostics
            .into_iter()
            .filter(|d| !d.is_error())
            .collect();
        avisos.extend(avisos_de_departamento);
        Ok(PublicacaoFeita {
            revisao,
            avisos,
            versao_do_fluxo,
        })
    }

    /// Carrega a IR de uma revisão pelo cache (FX-19), conferindo que ela é a que o texto publicado
    /// produz. IR ilegível é [`FalhaFluxo::Interno`]; IR que não confere é [`FalhaFluxo::IrDivergente`]
    /// (a leitura falha fechada, como no turno).
    pub fn ir_da_revisao(
        &self,
        fluxo: &Fluxo,
        revisao: &FlowRevision,
    ) -> Result<Arc<Ir>, FalhaFluxo> {
        let gravada: Ir = serde_json::from_str(&revisao.definicao_json).map_err(|erro| {
            tracing::error!(
                erro = %erro,
                revisao = %revisao.id,
                "a IR gravada da revisão não é JSON legível"
            );
            FalhaFluxo::Interno
        })?;
        let chave = RevisionKey::with_hash(
            fluxo.slug.clone(),
            u32::try_from(revisao.numero).unwrap_or(u32::MAX),
            revisao.fonte_sha256.clone(),
        );
        self.cache
            .load(&chave, &gravada, &revisao.fonte, &self.limites)
            .map_err(|falha| {
                tracing::error!(
                    falha = %falha.code(),
                    revisao = %revisao.id,
                    "a revisão publicada não confere com o texto guardado"
                );
                FalhaFluxo::IrDivergente(falha.code())
            })
    }

    /// Compara duas revisões. Sem `de` e `para`: `para` é a revisão atual e `de` é a anterior a ela.
    pub fn comparar(
        &self,
        org: OrgId,
        fluxo: &Fluxo,
        de: Option<u64>,
        para: Option<u64>,
    ) -> Result<Comparacao, FalhaFluxo> {
        let para = match para {
            Some(numero) => numero,
            None => fluxo.revisao_atual.ok_or(FalhaFluxo::SemRevisao)?,
        };
        let de = de.unwrap_or_else(|| para.saturating_sub(1).max(1));
        let revisao_de = self.store.revisions().get(org, fluxo.id, de)?;
        let revisao_para = self.store.revisions().get(org, fluxo.id, para)?;
        let ir_de = self.ir_da_revisao(fluxo, &revisao_de)?;
        let ir_para = self.ir_da_revisao(fluxo, &revisao_para)?;
        let mudancas = diff(&ir_de, &ir_para);
        let contagens = self.store.executions().active_by_revision(org, fluxo.id)?;
        let ativas = |revisao: &FlowRevision| -> u64 {
            contagens
                .iter()
                .find(|(id_da_contagem, _)| *id_da_contagem == revisao.id)
                .map_or(0, |(_, quantas)| *quantas)
        };
        Ok(Comparacao {
            ativas_de: ativas(&revisao_de),
            ativas_para: ativas(&revisao_para),
            de: revisao_de,
            para: revisao_para,
            mudancas,
        })
    }

    /// Bloco de prompt da etapa (a de início, se não houver etapa), na revisão que vale hoje.
    pub fn preview(
        &self,
        org: OrgId,
        fluxo: &Fluxo,
        etapa: Option<&str>,
    ) -> Result<Previa, FalhaFluxo> {
        let revisao = self
            .store
            .revisions()
            .current(org, fluxo.id)?
            .ok_or(FalhaFluxo::SemRevisao)?;
        let ir = self.ir_da_revisao(fluxo, &revisao)?;
        let etapa = etapa
            .filter(|texto| !texto.is_empty())
            .map_or_else(|| ir.start.clone(), str::to_owned);
        let bloco = stage_block(&ir, &etapa)
            .map_err(|erro: DomainError| FalhaFluxo::Banco(StoreError::Domain(erro)))?;
        Ok(Previa {
            etapa,
            revisao: revisao.numero,
            bloco,
        })
    }
}

/// Registra uma ação de fluxo na auditoria da org, com o autor informado.
pub fn auditar(
    store: &Store,
    org: OrgId,
    ator: &Ator,
    acao: &str,
    entidade: &str,
    id: String,
) -> Result<(), StoreError> {
    store.audit().register(
        Some(org),
        NewAuditEntry {
            ator_tipo: ator.tipo.clone(),
            actor_id: ator.id.clone(),
            acao: acao.into(),
            entity: entidade.into(),
            entity_id: Some(id),
            detail_json: "{}".into(),
        },
    )?;
    Ok(())
}

// ───────────────────────────── JSON que a API e a CLI devolvem ─────────────────────────────

/// O fluxo sem o texto do rascunho (é o campo grande, que a lista não traz).
pub fn json_fluxo(f: &Fluxo) -> Value {
    json!({
        "id": f.id.to_string(),
        "slug": f.slug,
        "name": f.name,
        "description": f.description,
        "status": f.status.as_text(),
        "current_revision": f.revisao_atual,
        "version": f.version,
    })
}

/// O fluxo com o rascunho: detalhe, criação e alteração.
pub fn json_fluxo_com_rascunho(f: &Fluxo) -> Value {
    let mut corpo = json_fluxo(f);
    corpo["draft_source"] = json!(f.rascunho_fonte);
    corpo
}

/// Cabeçalho de uma revisão, sem o texto nem a estrutura (é o que a lista mostra).
pub fn json_revisao_resumo(r: &FlowRevision) -> Value {
    json!({
        "id": r.id.to_string(),
        "flow_id": r.fluxo_id.to_string(),
        "number": r.numero,
        "schema_version": r.schema_versao,
        "source_sha256": r.fonte_sha256,
        "published_at": r.publicada_em,
        "published_by": r.publicada_por,
        "resolutions": serde_json::from_str::<Value>(&r.resolucoes_json).unwrap_or(Value::Null),
    })
}

/// A revisão inteira: o texto publicado e a estrutura compilada que o motor executa.
pub fn json_revisao_completa(r: &FlowRevision) -> Value {
    let mut corpo = json_revisao_resumo(r);
    corpo["source"] = json!(r.fonte);
    corpo["definition"] = serde_json::from_str::<Value>(&r.definicao_json).unwrap_or(Value::Null);
    corpo
}

/// Resultado de `validar`: `valid`, a IR, os diagnósticos e as resoluções.
pub fn json_validacao(v: &Validacao) -> Value {
    json!({
        "valid": v.valido(),
        "ir": v.ir,
        "diagnostics": v.diagnosticos,
        "resolutions": v.resolucoes,
    })
}

/// Resultado de `diff`: as duas revisões e a contagem de atendimentos ativos em cada uma.
pub fn json_comparacao(fluxo: FlowId, c: &Comparacao) -> Value {
    json!({
        "flow_id": fluxo.to_string(),
        "from": { "revision": c.de.numero, "active_runs": c.ativas_de },
        "to": { "revision": c.para.numero, "active_runs": c.ativas_para },
        "changes": c.mudancas,
    })
}

/// Resultado de `preview`: a etapa, a revisão e o bloco de prompt.
pub fn json_previa(p: &Previa) -> Value {
    json!({
        "stage": p.etapa,
        "revision": p.revisao,
        "prompt": p.bloco,
    })
}
