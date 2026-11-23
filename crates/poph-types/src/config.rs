//! Configuração do POP Harness: formato, padrões e validação.
//!
//! Aqui só há tipos, padrões e regras. Ler o arquivo TOML e o ambiente, e aplicar a
//! precedência (padrão < arquivo < ambiente), fica no crate `poph`, que é a raiz de composição.
//!
//! Segredos (chaves de API, chave de cifra) NÃO entram aqui: vêm só do ambiente.

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{DomainError, DomainResult};
use crate::integracao::PluginsConfig;

/// Configuração completa do processo. Campos ausentes no arquivo usam o padrão de cada seção.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct HarnessConfig {
    /// Endereço e porta em que o servidor vai escutar (HR-20).
    pub servidor: ServidorConfig,
    /// Onde ficam os dados locais (bancos e blobs).
    pub dados: DadosConfig,
    /// Nível e formato dos logs.
    pub log: LogConfig,
    /// Retenção de conversas por org.
    pub retencao: RetencaoConfig,
    /// Limites de simultaneidade e de taxa.
    pub limites: LimitesConfig,
    /// Provedor de LLM padrão (o agente pode trocar de modelo).
    pub llm: LlmConfig,
    /// Jev (decisões rápidas). Desligado até cada gate passar na medição (PRD §10).
    pub jev: JevConfig,
    /// Transferência para humano e orientação ao cliente antes dela (HR-29).
    pub handoff: HandoffConfig,
    /// Turno no `poph serve` (HR-26h): de quanto em quanto tempo a fila de entrada é lida.
    pub turno: TurnoConfig,
    /// Integração WhatsApp (Evolution). Ausente = integração não configurada.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evolution: Option<EvolutionConfig>,
    /// Módulo Matrix (MX-07). Ausente ou `habilitado = false` = módulo morto: nada dele carrega.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matrix: Option<MatrixConfig>,
    /// Fluxos e Fichas (FX-26): motor, tetos de script, de dados e de regra, e cache das revisões.
    pub fluxos: FluxosConfig,
    /// Plugins de integração declarados (HR-44c): quem responde ao handshake de cada `kind`.
    /// Vazio = nenhum manifesto disponível, e a API recusa configuração de integração.
    #[serde(default, skip_serializing_if = "PluginsConfig::vazio")]
    pub plugins: PluginsConfig,
    /// Tokens globais da API administrativa (só o hash; o valor em claro nunca fica aqui).
    pub admin: AdminConfig,
}

/// Tokens globais da API administrativa (PRD §7.2: os globais vêm da configuração).
///
/// O token de org vive no banco (`admin_tokens`); o global é fixo, igual em todas as instâncias,
/// e por isso fica na configuração — **apenas o hash argon2**, como os demais segredos não
/// reversíveis. O valor em claro existe só na saída de `poph token global`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct AdminConfig {
    /// Um token global por entrada. Sem entrada, só tokens de org autenticam.
    pub tokens_globais: Vec<TokenGlobalConfig>,
}

/// Um token global: rótulo para auditoria e hash argon2 do valor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenGlobalConfig {
    /// Rótulo que aparece na auditoria e no relatório de autenticação.
    pub label: String,
    /// Hash argon2id no formato PHC (`$argon2id$...`).
    pub hash: String,
}

/// Turno no `poph serve` (HR-26h).
///
/// A cada intervalo, o servidor lê as mensagens recebidas que ainda não foram atendidas e roda o
/// turno de cada uma. Intervalo menor responde mais rápido e consulta o banco com mais frequência.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TurnoConfig {
    /// Milissegundos entre duas leituras da fila de entrada. Entre 100 e 60000.
    pub intervalo_despacho_ms: u64,
    /// Tentativas de um turno quando o modelo falha, contando a primeira. Entre 1 e 10; o padrão é 3.
    /// Esgotadas, a mensagem é concluída sem resposta e o turno fica como falha definitiva.
    pub tentativas_falha_modelo: u32,
    /// Espera antes da segunda tentativa, em milissegundos. Cada espera seguinte é quatro vezes a
    /// anterior, até o teto de 10 minutos. Entre 1000 e 600000; o padrão é 30000 (30 s).
    pub atraso_falha_modelo_ms: u64,
}

impl Default for TurnoConfig {
    fn default() -> Self {
        Self {
            intervalo_despacho_ms: 1000,
            tentativas_falha_modelo: 3,
            atraso_falha_modelo_ms: 30_000,
        }
    }
}

/// Endereço e porta do servidor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ServidorConfig {
    /// Endereço de escuta. Padrão: só a própria máquina.
    pub endereco: IpAddr,
    /// Porta de escuta, entre 1024 e 65535.
    pub porta: u16,
}

/// Pastas e bancos locais. Os caminhos não definidos ficam dentro de `diretorio`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DadosConfig {
    /// Pasta de dados. Padrão: `data`, relativa a onde o processo roda.
    pub diretorio: PathBuf,
    /// Banco durável. Padrão: `harness.db` dentro de `diretorio`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness_db: Option<PathBuf>,
    /// Banco reconstruível do RAG. Padrão: `rag.db` dentro de `diretorio`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rag_db: Option<PathBuf>,
    /// Pasta dos arquivos (blobs). Padrão: `blobs` dentro de `diretorio`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blobs: Option<PathBuf>,
    /// Pasta dos modelos de embedding (`poph model install`). Padrão: `modelos` dentro de `diretorio`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modelos: Option<PathBuf>,
}

/// Nível e formato dos logs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LogConfig {
    /// `trace`, `debug`, `info`, `warn` ou `error`. A variável `RUST_LOG`, se definida, tem prioridade.
    pub nivel: String,
    /// `legivel` (texto para pessoas) ou `json` (uma linha por evento).
    pub formato: Formato,
}

/// Formato de saída dos logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Formato {
    /// Texto para ler no terminal.
    #[default]
    Legivel,
    /// Uma linha JSON por evento.
    Json,
}

/// Retenção de conversas, mensagens e anexos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RetencaoConfig {
    /// Dias antes de apagar. `0` = não apagar nunca. Padrão: 30 (PRD §7.4).
    pub dias: u32,
}

/// Limites de simultaneidade e de taxa. Por org, para uma org não atrasar a outra.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LimitesConfig {
    /// Turnos rodando ao mesmo tempo na instância inteira. Padrão: 200 (decisão D-07).
    pub turnos_simultaneos: u32,
    /// Turnos rodando ao mesmo tempo por org. Não pode passar de `turnos_simultaneos`.
    pub turnos_simultaneos_por_org: u32,
    /// Mensagens aceitas por minuto em cada org.
    pub taxa_por_org_por_minuto: u32,
}

/// Provedor de LLM padrão.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LlmConfig {
    /// Nome do provedor. Na V1 só existe `antigravity2api`.
    pub provedor: String,
    /// Endereço base do serviço (formato compatível com OpenAI, termina em `/v1`).
    pub url_base: String,
    /// Modelo padrão. Cada agente pode trocar.
    pub modelo: String,
    /// Tempo máximo de uma chamada, em segundos.
    pub timeout_segundos: u32,
    /// Novas tentativas só em erro transitório (429, 5xx, queda de conexão).
    pub retentativas: u32,
    /// Chamadas simultâneas a este provedor, na instância inteira.
    pub simultaneos: u32,
    /// Temperatura padrão, entre 0 e 2.
    pub temperatura: f32,
    /// Teto de tokens de saída por resposta.
    pub max_tokens_saida: u32,
}

/// Jev: decisões rápidas. Tudo desligado por padrão até a medição de cada gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct JevConfig {
    /// Liga o Jev na instância. Com `true`, a chave `TYPESAFE_API_KEY` vira obrigatória.
    pub ativo: bool,
    /// Endereço base da API do Jev.
    pub url_base: String,
    /// Modelo usado nas perguntas.
    pub modelo: String,
    /// Tempo máximo de uma pergunta, em segundos.
    pub timeout_segundos: u32,
    /// Novas tentativas só em 429 ou 5xx.
    pub retentativas: u32,
    /// Quais gates estão ligados (cada um só liga depois de medido).
    pub gates: GatesJev,
}

/// Transferência para humano: a mensagem automática e a orientação mínima ao cliente (HR-29).
///
/// Os textos são **dado de configuração**, não texto de modelo: nenhum modelo escreve o que o
/// cliente lê antes de ser transferido (`HUMAN_HANDOFF_GUIDANCE_PRD.md` §5.3, aprovado pelo mantenedor
/// em 19/11/2026). Os padrões aqui são exatamente os aprovados; trocar o valor no `poph.toml`
/// troca o que o cliente recebe, sem recompilar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct HandoffConfig {
    /// Mensagem automática enviada ao cliente no momento da transferência.
    pub mensagem_automatica: String,
    /// Faz a segunda pergunta do G1 (tipo de risco). Desligada: sai só a mensagem automática.
    pub orientacao_ativa: bool,
    /// Confiança mínima para aceitar o tipo de risco. `0.5` de propósito: aviso a mais custa
    /// pouco, aviso faltando custa muito (PRD §5.4).
    pub orientacao_confianca: f64,
    /// Prazo só da segunda pergunta, em segundos (a transferência nunca é segurada por ela).
    pub orientacao_timeout_segundos: u32,
    /// Texto fixo do tipo `ingestao_contato_produto`. Vazio = o tipo não orienta.
    pub orientacao_ingestao_contato_produto: String,
    /// Texto fixo do tipo `animal_peconhento`. Vazio = o tipo não orienta.
    pub orientacao_animal_peconhento: String,
    /// Texto fixo do tipo `outro_risco_saude`. Vazio = o tipo não orienta.
    pub orientacao_outro_risco_saude: String,
}

impl Default for HandoffConfig {
    fn default() -> Self {
        Self {
            // Texto de hoje do POP Atende (`HUMAN_HANDOFF_AUTO_MESSAGE`).
            mensagem_automatica: "Sua conversa foi transferida para um atendente humano da nossa \
equipe, que já irá respondê-la por aqui. 🙋"
                .to_owned(),
            orientacao_ativa: true,
            orientacao_confianca: 0.5,
            orientacao_timeout_segundos: 4,
            // Textos aprovados em 19/11/2026 (HUMAN_HANDOFF_GUIDANCE_PRD.md §5.3): número do SAMU
            // e do Disque-Intoxicação conferidos em fonte pública (Anvisa/CFM).
            orientacao_ingestao_contato_produto: "⚠️ Se alguém engoliu ou teve contato com o \
produto, procure atendimento médico agora ou ligue para o SAMU (192) ou para o \
Disque-Intoxicação (0800 722 6001). Leve a embalagem do produto. Não provoque vômito e não dê \
nada para beber ou comer sem orientação médica."
                .to_owned(),
            orientacao_animal_peconhento:
                "⚠️ Não toque nem tente pegar o animal. Mantenha crianças \
e outras pessoas longe dele e, se puder, feche a porta do cômodo. Se alguém foi picado, vá agora \
a um pronto-socorro ou ligue para o SAMU (192)."
                    .to_owned(),
            orientacao_outro_risco_saude: "⚠️ Se alguém corre risco de saúde agora, ligue para o \
SAMU (192) ou vá a um pronto-socorro sem esperar a nossa resposta."
                .to_owned(),
        }
    }
}

/// Fluxos e Fichas (FX-26, PRD §9): o que o arquivo e o ambiente podem ajustar no módulo.
///
/// Os números são os **propostos** pelo PRD; cada um tem uma faixa, e valor fora dela é recusado
/// com o nome da chave. O motor (FX-13/FX-14) e o cache (FX-19) já recebem seus tetos por parâmetro;
/// cada consumidor lê o campo que lhe cabe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FluxosConfig {
    /// Liga o motor de fluxos nesta instância. Desligado, o G9, o prompt, as ferramentas e os efeitos
    /// param; a Ficha continua sendo criada e fechada. Padrão: ligado.
    pub ativo: bool,
    /// Fluxos publicados por org (teto). Entre 1 e 10000; padrão 100.
    pub max_fluxos_por_org: u32,
    /// Tamanho máximo do script de um fluxo, em bytes. Entre 1024 e 1048576; padrão 65536.
    pub max_fonte_bytes: u64,
    /// Etapas, decisões e fins por fluxo. Entre 1 e 60 (o compilador não aceita mais que 60 nós).
    pub max_nos: u32,
    /// Tamanho máximo do `dados_json` de uma execução, em bytes. Entre 1024 e 10485760; padrão 65536.
    pub max_dados_bytes: u64,
    /// Passos de uma avaliação de regra antes de cortar (laço infinito). Entre 100 e 1000000; padrão 10000.
    pub max_ticks_regra: u64,
    /// Memória de uma avaliação de regra, em bytes. Entre 65536 e 67108864; padrão 1048576 (1 MiB).
    pub max_heap_regra: u64,
    /// Decisões encadeadas que uma mesma passagem atravessa antes de falhar. Entre 1 e 100; padrão 10.
    pub max_saltos_decisao: u32,
    /// Teto de RAM das revisões compiladas, em MB. Entre 0 e 4096; `0` não guarda nenhuma revisão
    /// (cada uso recompila). Padrão 16.
    pub cache_revisoes_mb: u32,
}

impl Default for FluxosConfig {
    fn default() -> Self {
        Self {
            ativo: true,
            max_fluxos_por_org: 100,
            max_fonte_bytes: 65_536,
            max_nos: 60,
            max_dados_bytes: 65_536,
            max_ticks_regra: 10_000,
            max_heap_regra: 1_048_576,
            max_saltos_decisao: 10,
            cache_revisoes_mb: 16,
        }
    }
}

impl FluxosConfig {
    /// Confere todas as faixas. Devolve o primeiro problema com o nome da chave.
    ///
    /// Vale também com o motor desligado: um teto inválido é erro de configuração, não de uso.
    pub fn validar(&self) -> DomainResult<()> {
        validar_fluxos(self)
    }
}

/// Liga e desliga cada gate do Jev (PRD §10).
///
/// `Default` (derivado) é **tudo desligado**: é o que vale para "sem gates" (agente com
/// `ativo: false`, testes que montam o motor à mão). O padrão de **instalação** é outro e vive em
/// [`GatesJev::padrao`]: G3, G5, G8 e G9 ligados (G3, G5 e G8 por decisão do mantenedor, 20/11/2026;
/// G9 pelo PRD de fluxos, §9), os demais desligados.
/// Campo ausente no arquivo de configuração cai no padrão de instalação, campo a campo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct GatesJev {
    /// G1: transferir para humano e orientação de risco.
    #[serde(default)]
    pub g1: bool,
    /// G2: esperar mais ou processar agora.
    #[serde(default)]
    pub g2: bool,
    /// G3: precisa de base, produto ou nada. Ligado por padrão desde 20/11/2026.
    #[serde(default = "ligado_por_padrao")]
    pub g3: bool,
    /// G4: qual departamento recebe a transferência.
    #[serde(default)]
    pub g4: bool,
    /// G5: qual produto o cliente quer. Ligado por padrão desde 20/11/2026.
    #[serde(default = "ligado_por_padrao")]
    pub g5: bool,
    /// G6: revisão da resposta antes do envio.
    #[serde(default)]
    pub g6: bool,
    /// G7: injeção de prompt ou abuso.
    #[serde(default)]
    pub g7: bool,
    /// G8: conversa encerrada. Ligado por padrão desde 20/11/2026 (decisão do mantenedor).
    #[serde(default = "ligado_por_padrao")]
    pub g8: bool,
    /// G9: quem inicia um fluxo (Jev escolhe, código inicia). Ligado por padrão pelo PRD de fluxos
    /// (§9); só tem efeito quando o motor de fluxos (`[fluxos] ativo`) estiver ligado.
    #[serde(default = "ligado_por_padrao")]
    pub g9: bool,
}

fn ligado_por_padrao() -> bool {
    true
}

impl GatesJev {
    /// Padrão de instalação: G3, G5, G8 e G9 ligados; G1, G2, G4, G6 e G7, desligados.
    pub const fn padrao() -> Self {
        Self {
            g1: false,
            g2: false,
            g3: true,
            g4: false,
            g5: true,
            g6: false,
            g7: false,
            g8: true,
            g9: true,
        }
    }
}

/// Integração WhatsApp via Evolution API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvolutionConfig {
    /// Endereço base da Evolution API. A chave `EVOLUTION_API_KEY` fica no ambiente.
    pub url_base: String,
    /// Instâncias que entregam mensagens ao Harness (HR-26b). Vazio = nenhum webhook aceito.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instancias: Vec<InstanciaEvolutionConfig>,
}

/// Uma instância da Evolution ligada a uma integração WhatsApp do Harness.
///
/// O segredo do webhook **não** entra aqui: o arquivo guarda só o nome da variável de ambiente
/// (`segredo_env`), e o valor vem do ambiente na subida.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanciaEvolutionConfig {
    /// Org dona da instância (id no formato do Harness).
    pub org_id: String,
    /// Nome da instância na Evolution (é o que aparece no caminho do webhook).
    pub instancia: String,
    /// Integração WhatsApp do agente que recebe as mensagens desta instância.
    pub integracao_id: String,
    /// Nome da variável de ambiente que guarda o segredo do webhook (ex.: `POPH_EVO_SEGREDO_LOJA`).
    pub segredo_env: String,
}

/// Módulo Matrix do Harness (MX-07, PRD §9): a ponte entre a conversa e o Synapse/Element.
///
/// Ausente do arquivo ou com `habilitado = false`, o módulo inteiro fica morto: nenhuma tarefa, nenhuma
/// rota e nenhum token lido do ambiente. Só com `habilitado = true` os demais campos são conferidos.
///
/// Os tokens (`POPH_MATRIX_AS_TOKEN` e `POPH_MATRIX_HS_TOKEN`) **não** entram aqui: vêm só do ambiente
/// e são lidos pelo crate `poph-matrix` no tipo `Sensivel`. `POPH_MATRIX_ADMIN_TOKEN` é da fase 2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MatrixConfig {
    /// Liga o módulo nesta instância. Padrão: desligado.
    pub habilitado: bool,
    /// Endereço base da API cliente do Synapse (`http://` ou `https://`).
    pub homeserver_url: String,
    /// Domínio dos ids Matrix (ex.: `matrix-dev.local`), como no `server_name` do Synapse.
    pub server_name: String,
    /// Prefixo de usuários virtuais, aliases e `sender_localpart`. Só `[a-z0-9]`, de 1 a 12.
    pub prefixo: String,
    /// Salas espelhadas em paralelo. Entre 1 e 64.
    pub concorrencia: u32,
    /// Espera entre duas leituras do cursor quando não há trabalho. Entre 50 e 60000 ms.
    pub intervalo_ms: u64,
    /// Power level do fantasma do agente na sala. Entre 0 e 100; o padrão (50) escreve sem administrar.
    pub poder_agente: u8,
    /// Dias que a mídia fica no Synapse antes de ser apagada (fase 2). Entre 1 e 365.
    pub midia_retencao_dias: u32,
}

impl Default for MatrixConfig {
    fn default() -> Self {
        Self {
            habilitado: false,
            homeserver_url: String::new(),
            server_name: String::new(),
            prefixo: "ph".to_owned(),
            concorrencia: 8,
            intervalo_ms: 500,
            poder_agente: 50,
            midia_retencao_dias: 4,
        }
    }
}

impl MatrixConfig {
    /// Confere a seção quando ela está ligada; desligada, qualquer valor é aceito.
    ///
    /// É a mesma regra que [`HarnessConfig::validar`] aplica; fica pública para o módulo
    /// (`poph-matrix`) validar a seção sozinho antes de carregar os segredos.
    pub fn validar(&self) -> DomainResult<()> {
        validar_matrix(self)
    }
}

impl Default for ServidorConfig {
    fn default() -> Self {
        Self {
            endereco: IpAddr::from([127, 0, 0, 1]),
            porta: 7480,
        }
    }
}

impl Default for DadosConfig {
    fn default() -> Self {
        Self {
            diretorio: PathBuf::from("data"),
            harness_db: None,
            rag_db: None,
            blobs: None,
            modelos: None,
        }
    }
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            nivel: "info".to_owned(),
            formato: Formato::Legivel,
        }
    }
}

impl Default for RetencaoConfig {
    fn default() -> Self {
        Self { dias: 30 }
    }
}

impl Default for LimitesConfig {
    fn default() -> Self {
        Self {
            turnos_simultaneos: 200,
            turnos_simultaneos_por_org: 50,
            taxa_por_org_por_minuto: 600,
        }
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provedor: "antigravity2api".to_owned(),
            url_base: "http://127.0.0.1:8045/v1".to_owned(),
            modelo: "gemini-3.8-flash-tiered".to_owned(),
            timeout_segundos: 150,
            retentativas: 2,
            simultaneos: 32,
            temperatura: 0.0,
            max_tokens_saida: 4096,
        }
    }
}

impl Default for JevConfig {
    fn default() -> Self {
        Self {
            ativo: false,
            url_base: "https://api.typesafe.ai".to_owned(),
            modelo: "jev-latest".to_owned(),
            timeout_segundos: 8,
            retentativas: 1,
            gates: GatesJev::padrao(),
        }
    }
}

impl HarnessConfig {
    /// Confere todas as regras. Devolve o primeiro problema com o nome do campo.
    pub fn validar(&self) -> DomainResult<()> {
        validar_servidor(&self.servidor)?;
        validar_dados(&self.dados)?;
        validar_log(&self.log)?;
        validar_limites(&self.limites)?;
        validar_llm(&self.llm)?;
        validar_jev(&self.jev)?;
        validar_handoff(&self.handoff)?;
        validar_turno(&self.turno)?;
        validar_admin(&self.admin)?;
        validar_plugins(&self.plugins)?;
        if let Some(evolution) = &self.evolution {
            validar_url("evolution.url_base", &evolution.url_base)?;
            validar_instancias_evolution(&evolution.instancias)?;
        }
        if let Some(matrix) = &self.matrix {
            matrix.validar()?;
        }
        validar_fluxos(&self.fluxos)?;
        Ok(())
    }

    /// Caminho do banco durável (`harness.db`).
    pub fn harness_db(&self) -> PathBuf {
        caminho_ou_padrao(&self.dados.harness_db, &self.dados.diretorio, "harness.db")
    }

    /// Caminho do banco do RAG (`rag.db`).
    pub fn rag_db(&self) -> PathBuf {
        caminho_ou_padrao(&self.dados.rag_db, &self.dados.diretorio, "rag.db")
    }

    /// Pasta dos arquivos (blobs).
    pub fn blobs(&self) -> PathBuf {
        caminho_ou_padrao(&self.dados.blobs, &self.dados.diretorio, "blobs")
    }

    /// Pasta dos modelos de embedding (`poph model install` grava aqui).
    pub fn modelos(&self) -> PathBuf {
        caminho_ou_padrao(&self.dados.modelos, &self.dados.diretorio, "modelos")
    }
}

fn caminho_ou_padrao(definido: &Option<PathBuf>, diretorio: &Path, padrao: &str) -> PathBuf {
    definido.clone().unwrap_or_else(|| diretorio.join(padrao))
}

fn invalido(campo: &'static str, motivo: impl Into<String>) -> DomainError {
    DomainError::Validacao {
        campo,
        motivo: motivo.into(),
    }
}

fn validar_plugins(plugins: &PluginsConfig) -> DomainResult<()> {
    let mut vistos: Vec<String> = Vec::with_capacity(plugins.integracoes.len());
    for (indice, plugin) in plugins.integracoes.iter().enumerate() {
        let kind = plugin.kind.trim();
        if kind.is_empty() {
            return Err(invalido(
                "plugins.integracoes.kind",
                format!("item {} sem `kind`", indice + 1),
            ));
        }
        if plugin.programa.as_os_str().is_empty() {
            return Err(invalido(
                "plugins.integracoes.programa",
                format!("o plugin `{kind}` não declara o programa"),
            ));
        }
        if vistos.iter().any(|visto| visto == kind) {
            return Err(invalido(
                "plugins.integracoes.kind",
                format!("o tipo `{kind}` está declarado duas vezes"),
            ));
        }
        vistos.push(kind.to_owned());
    }
    Ok(())
}

fn validar_servidor(servidor: &ServidorConfig) -> DomainResult<()> {
    if !(1024..=65535).contains(&servidor.porta) {
        return Err(invalido(
            "servidor.porta",
            format!("{} fora da faixa 1024 a 65535", servidor.porta),
        ));
    }
    Ok(())
}

fn validar_dados(dados: &DadosConfig) -> DomainResult<()> {
    if dados.diretorio.as_os_str().is_empty() {
        return Err(invalido("dados.diretorio", "caminho vazio"));
    }
    for (campo, caminho) in [
        ("dados.harness_db", &dados.harness_db),
        ("dados.rag_db", &dados.rag_db),
        ("dados.blobs", &dados.blobs),
        ("dados.modelos", &dados.modelos),
    ] {
        if caminho.as_ref().is_some_and(|c| c.as_os_str().is_empty()) {
            return Err(invalido(campo, "caminho vazio"));
        }
    }
    Ok(())
}

fn validar_log(log: &LogConfig) -> DomainResult<()> {
    const NIVEIS: [&str; 5] = ["trace", "debug", "info", "warn", "error"];
    if !NIVEIS.contains(&log.nivel.as_str()) {
        return Err(invalido(
            "log.nivel",
            format!(
                "`{}` não é um nível; use trace, debug, info, warn ou error",
                log.nivel
            ),
        ));
    }
    Ok(())
}

fn validar_limites(limites: &LimitesConfig) -> DomainResult<()> {
    if !(1..=10_000).contains(&limites.turnos_simultaneos) {
        return Err(invalido(
            "limites.turnos_simultaneos",
            "deve estar entre 1 e 10000",
        ));
    }
    if !(1..=limites.turnos_simultaneos).contains(&limites.turnos_simultaneos_por_org) {
        return Err(invalido(
            "limites.turnos_simultaneos_por_org",
            "deve ser de 1 até o total de turnos simultâneos",
        ));
    }
    if !(1..=100_000).contains(&limites.taxa_por_org_por_minuto) {
        return Err(invalido(
            "limites.taxa_por_org_por_minuto",
            "deve estar entre 1 e 100000",
        ));
    }
    Ok(())
}

fn validar_llm(llm: &LlmConfig) -> DomainResult<()> {
    if llm.provedor != "antigravity2api" {
        return Err(invalido(
            "llm.provedor",
            format!(
                "`{}` não é um provedor da V1; use antigravity2api",
                llm.provedor
            ),
        ));
    }
    validar_url("llm.url_base", &llm.url_base)?;
    if llm.modelo.trim().is_empty() {
        return Err(invalido("llm.modelo", "nome do modelo vazio"));
    }
    if !(1..=600).contains(&llm.timeout_segundos) {
        return Err(invalido("llm.timeout_segundos", "deve estar entre 1 e 600"));
    }
    if llm.retentativas > 5 {
        return Err(invalido("llm.retentativas", "no máximo 5"));
    }
    if !(1..=1000).contains(&llm.simultaneos) {
        return Err(invalido("llm.simultaneos", "deve estar entre 1 e 1000"));
    }
    if !(0.0..=2.0).contains(&llm.temperatura) {
        return Err(invalido("llm.temperatura", "deve estar entre 0 e 2"));
    }
    if !(1..=131_072).contains(&llm.max_tokens_saida) {
        return Err(invalido(
            "llm.max_tokens_saida",
            "deve estar entre 1 e 131072",
        ));
    }
    Ok(())
}

fn validar_jev(jev: &JevConfig) -> DomainResult<()> {
    validar_url("jev.url_base", &jev.url_base)?;
    if jev.modelo.trim().is_empty() {
        return Err(invalido("jev.modelo", "nome do modelo vazio"));
    }
    if !(1..=60).contains(&jev.timeout_segundos) {
        return Err(invalido("jev.timeout_segundos", "deve estar entre 1 e 60"));
    }
    if jev.retentativas > 3 {
        return Err(invalido("jev.retentativas", "no máximo 3"));
    }
    Ok(())
}

fn validar_turno(turno: &TurnoConfig) -> DomainResult<()> {
    if !(100..=60_000).contains(&turno.intervalo_despacho_ms) {
        return Err(invalido(
            "turno.intervalo_despacho_ms",
            "deve estar entre 100 e 60000 milissegundos",
        ));
    }
    if !(1..=10).contains(&turno.tentativas_falha_modelo) {
        return Err(invalido(
            "turno.tentativas_falha_modelo",
            "deve estar entre 1 e 10",
        ));
    }
    if !(1_000..=600_000).contains(&turno.atraso_falha_modelo_ms) {
        return Err(invalido(
            "turno.atraso_falha_modelo_ms",
            "deve estar entre 1000 e 600000 milissegundos",
        ));
    }
    Ok(())
}

fn validar_handoff(handoff: &HandoffConfig) -> DomainResult<()> {
    if !(0.0..=1.0).contains(&handoff.orientacao_confianca) {
        return Err(invalido(
            "handoff.orientacao_confianca",
            "deve estar entre 0 e 1",
        ));
    }
    if !(1..=60).contains(&handoff.orientacao_timeout_segundos) {
        return Err(invalido(
            "handoff.orientacao_timeout_segundos",
            "deve estar entre 1 e 60",
        ));
    }
    if handoff.mensagem_automatica.trim().is_empty() {
        return Err(invalido(
            "handoff.mensagem_automatica",
            "a mensagem automática da transferência não pode ficar vazia",
        ));
    }
    Ok(())
}

fn validar_admin(admin: &AdminConfig) -> DomainResult<()> {
    for (posicao, token) in admin.tokens_globais.iter().enumerate() {
        let rotulo = token.label.trim();
        if rotulo.is_empty() || rotulo.chars().count() > 80 {
            return Err(invalido(
                "admin.tokens_globais[].label",
                format!(
                    "entrada {}: o rótulo deve ter de 1 a 80 caracteres",
                    posicao + 1
                ),
            ));
        }
        if !token.hash.trim().starts_with("$argon2") {
            return Err(invalido(
                "admin.tokens_globais[].hash",
                format!(
                    "entrada {}: o hash deve ser argon2 no formato PHC (`$argon2id$...`); \
                     gere com `poph token global`",
                    posicao + 1
                ),
            ));
        }
    }
    Ok(())
}

/// Confere as instâncias da Evolution: ids válidos, nome de instância seguro para caminho de URL,
/// nome de variável de segredo válido e sem repetir (org, instância).
fn validar_instancias_evolution(instancias: &[InstanciaEvolutionConfig]) -> DomainResult<()> {
    use crate::ids::{IntegrationId, OrgId};
    use std::collections::BTreeSet;
    use std::str::FromStr;

    let mut vistas = BTreeSet::new();
    for instancia in instancias {
        if OrgId::from_str(instancia.org_id.trim()).is_err() {
            return Err(invalido(
                "evolution.instancias.org_id",
                "não é um id de org válido",
            ));
        }
        if IntegrationId::from_str(instancia.integracao_id.trim()).is_err() {
            return Err(invalido(
                "evolution.instancias.integracao_id",
                "não é um id de integração válido",
            ));
        }
        let nome = instancia.instancia.trim();
        let nome_seguro = !nome.is_empty()
            && nome.len() <= 64
            && nome
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !nome_seguro {
            return Err(invalido(
                "evolution.instancias.instancia",
                "use letras, números, `-` ou `_` (até 64 caracteres), como o nome da instância na Evolution",
            ));
        }
        let variavel = instancia.segredo_env.trim();
        let variavel_valida = !variavel.is_empty()
            && variavel
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            && !variavel.starts_with(|c: char| c.is_ascii_digit());
        if !variavel_valida {
            return Err(invalido(
                "evolution.instancias.segredo_env",
                "nome de variável de ambiente em maiúsculas (ex.: POPH_EVO_SEGREDO_LOJA)",
            ));
        }
        if !vistas.insert((instancia.org_id.trim().to_owned(), nome.to_owned())) {
            return Err(invalido(
                "evolution.instancias",
                "a mesma instância aparece duas vezes para a mesma org",
            ));
        }
    }
    Ok(())
}

fn validar_url(campo: &'static str, url: &str) -> DomainResult<()> {
    let sem_espacos = url.trim();
    if !(sem_espacos.starts_with("http://") || sem_espacos.starts_with("https://")) {
        return Err(invalido(
            campo,
            "o endereço deve começar com http:// ou https://",
        ));
    }
    Ok(())
}

// ───────────────────────────── matrix (MX-07) ─────────────────────────────

/// Confere a seção `[matrix]` — só quando `habilitado = true`.
///
/// Desligado, nada é conferido: a seção pode existir vazia (é o que uma instalação grava ao desligar o
/// módulo) sem impedir a subida. Os motivos **não** repetem o valor recebido: o erro nomeia a chave.
fn validar_matrix(matrix: &MatrixConfig) -> DomainResult<()> {
    if !matrix.habilitado {
        return Ok(());
    }
    validar_url("matrix.homeserver_url", &matrix.homeserver_url)?;
    if !server_name_valido(&matrix.server_name) {
        return Err(invalido(
            "matrix.server_name",
            "use o domínio do Synapse (letras, números, `.`, `-` ou `:porta`), sem barra nem espaço",
        ));
    }
    if !prefixo_valido(&matrix.prefixo) {
        return Err(invalido(
            "matrix.prefixo",
            "use de 1 a 12 caracteres entre `a` e `z` ou `0` e `9`",
        ));
    }
    if !(1..=64).contains(&matrix.concorrencia) {
        return Err(invalido("matrix.concorrencia", "fora da faixa 1 a 64"));
    }
    if !(50..=60_000).contains(&matrix.intervalo_ms) {
        return Err(invalido("matrix.intervalo_ms", "fora da faixa 50 a 60000"));
    }
    if matrix.poder_agente > 100 {
        return Err(invalido(
            "matrix.poder_agente",
            "power level do Matrix vai de 0 a 100",
        ));
    }
    if !(1..=365).contains(&matrix.midia_retencao_dias) {
        return Err(invalido(
            "matrix.midia_retencao_dias",
            "fora da faixa 1 a 365 dias",
        ));
    }
    Ok(())
}

/// Confere a seção `[fluxos]` (FX-26). Vale sempre, ligado ou não: são tetos, não uso.
///
/// Cada faixa vem do PRD §9 e do que o código aceita hoje. O erro nomeia a chave e a faixa, nunca o
/// valor recebido (o valor pode ser de um script ou de um ambiente de produção).
fn validar_fluxos(fluxos: &FluxosConfig) -> DomainResult<()> {
    fn faixa_u64(campo: &'static str, valor: u64, minimo: u64, maximo: u64) -> DomainResult<()> {
        if (minimo..=maximo).contains(&valor) {
            Ok(())
        } else {
            Err(invalido(
                campo,
                format!("fora da faixa {minimo} a {maximo}"),
            ))
        }
    }
    faixa_u64(
        "fluxos.max_fluxos_por_org",
        u64::from(fluxos.max_fluxos_por_org),
        1,
        10_000,
    )?;
    faixa_u64(
        "fluxos.max_fonte_bytes",
        fluxos.max_fonte_bytes,
        1_024,
        1_048_576,
    )?;
    faixa_u64("fluxos.max_nos", u64::from(fluxos.max_nos), 1, 60)?;
    faixa_u64(
        "fluxos.max_dados_bytes",
        fluxos.max_dados_bytes,
        1_024,
        10_485_760,
    )?;
    faixa_u64(
        "fluxos.max_ticks_regra",
        fluxos.max_ticks_regra,
        100,
        1_000_000,
    )?;
    faixa_u64(
        "fluxos.max_heap_regra",
        fluxos.max_heap_regra,
        65_536,
        67_108_864,
    )?;
    faixa_u64(
        "fluxos.max_saltos_decisao",
        u64::from(fluxos.max_saltos_decisao),
        1,
        100,
    )?;
    faixa_u64(
        "fluxos.cache_revisoes_mb",
        u64::from(fluxos.cache_revisoes_mb),
        0,
        4096,
    )?;
    Ok(())
}

/// `true` para `[a-z0-9]{1,12}`.
fn prefixo_valido(prefixo: &str) -> bool {
    (1..=12).contains(&prefixo.len())
        && prefixo
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// `true` para um domínio de Matrix: rótulos com letras, números, `.` ou `-`, com `:porta` opcional.
///
/// O primeiro rótulo pode ser `localhost` ou um IP; o Synapse de desenvolvimento usa algo como
/// `matrix-dev.local:18008`.
fn server_name_valido(server_name: &str) -> bool {
    let nome = server_name.trim();
    if nome.is_empty() || nome.len() > 255 || nome != server_name {
        return false;
    }
    let (dominio, porta) = match nome.rsplit_once(':') {
        Some((dominio, porta)) => (dominio, Some(porta)),
        None => (nome, None),
    };
    if let Some(porta) = porta {
        if porta.is_empty() || porta.len() > 5 || !porta.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        if porta.parse::<u16>().map(|valor| valor == 0).unwrap_or(true) {
            return false;
        }
    }
    !dominio.is_empty()
        && !dominio.starts_with('.')
        && !dominio.ends_with('.')
        && !dominio.starts_with('-')
        && !dominio.ends_with('-')
        && dominio
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
}

// ───────────────────────────── cluster (HR-50a) ─────────────────────────────

/// Roteamento estático entre instâncias (`cluster.toml`, PRD §12).
///
/// Sem arquivo, a instância roda em **modo instância única**: todas as orgs vivem nela e este
/// tipo fica vazio. Com o arquivo, o mapa é explícito: cada org vive em exatamente uma instância,
/// e as orgs fora do mapa vão para `default_instance`.
///
/// O segredo compartilhado **não** entra aqui: é lido só de `POPH_CLUSTER_SECRET` (ambiente) e
/// nunca é gravado no arquivo, em resposta, em log ou em erro.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct ClusterConfig {
    /// Identificador desta instância no mapa. Obrigatório quando há `cluster.toml`.
    pub instance_id: String,
    /// Instância que recebe as orgs fora do mapa. Obrigatória quando o cluster tem mais de uma
    /// instância.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_instance: Option<String>,
    /// Instâncias do cluster: id e endereço base (`http://` ou `https://`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instances: Vec<ClusterInstance>,
    /// Mapa explícito org → instância, no formato `[orgs]` do TOML (chave: id da org `org_…`).
    ///
    /// Chave repetida é recusada pelo próprio TOML: o arquivo nem é lido como configuração.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub orgs: BTreeMap<String, String>,
}

/// Uma instância do cluster: identificador e endereço base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClusterInstance {
    /// Identificador curto (letras, números, `-`, `_`, `.`; até 64 caracteres).
    pub id: String,
    /// Endereço base do servidor da instância (`http://host:porta` ou `https://host`).
    pub url: String,
}

impl ClusterConfig {
    /// `true` quando a configuração descreve um cluster (declara instâncias ou mapa de orgs).
    pub fn declara_cluster(&self) -> bool {
        !self.instances.is_empty() || !self.orgs.is_empty()
    }

    /// `true` quando é o modo instância única (nada declarado).
    pub fn vazio(&self) -> bool {
        self.instance_id.trim().is_empty()
            && self.default_instance.is_none()
            && !self.declara_cluster()
    }

    /// Instância que recebe uma org: a do mapa, senão a padrão, senão a local.
    ///
    /// `None` só no modo instância única sem `instance_id` (não há nome de instância a devolver).
    pub fn instancia_da_org(&self, org: &str) -> Option<&str> {
        let org = org.trim();
        if let Some(destino) = self.orgs.get(org) {
            return Some(destino.trim());
        }
        if let Some(padrao) = self
            .default_instance
            .as_deref()
            .map(str::trim)
            .filter(|valor| !valor.is_empty())
        {
            return Some(padrao);
        }
        // Cluster de uma instância só: ela é a padrão implícita.
        if let Some(unica) = self.instances.first() {
            return Some(unica.id.trim());
        }
        None
    }

    /// Todos os problemas do mapa, um por linha (`campo: motivo`). Vazio = válido.
    ///
    /// Reúne tudo em vez de parar no primeiro: é o que `poph cluster check` lista.
    pub fn problemas(&self) -> Vec<String> {
        let mut problemas = Vec::new();
        if self.vazio() {
            return problemas;
        }
        let local = self.instance_id.trim();
        if local.is_empty() {
            problemas.push(
                "instance_id: obrigatório quando existe cluster.toml (é o nome desta instância no \
                 mapa)"
                    .to_owned(),
            );
        } else if !id_de_instancia_valido(local) {
            problemas.push(
                "instance_id: use letras, números, `-`, `_` ou `.` (até 64 caracteres)".to_owned(),
            );
        }
        let mut vistos: BTreeSet<String> = BTreeSet::new();
        for (posicao, instancia) in self.instances.iter().enumerate() {
            let id = instancia.id.trim();
            let campo = format!("instances[{}].id", posicao + 1);
            if !id_de_instancia_valido(id) {
                problemas.push(format!(
                    "{campo}: use letras, números, `-`, `_` ou `.` (até 64 caracteres)"
                ));
            } else if !vistos.insert(id.to_owned()) {
                problemas.push(format!("{campo}: a instância `{id}` aparece duas vezes"));
            }
            if !url_de_instancia_valida(&instancia.url) {
                problemas.push(format!(
                    "instances[{}].url: o endereço deve começar com http:// ou https:// e ter um \
                     destino (ex.: https://harness-2.exemplo)",
                    posicao + 1
                ));
            }
        }
        if !self.instances.is_empty() && !local.is_empty() && !vistos.contains(local) {
            problemas.push(format!(
                "instance_id: `{local}` não está declarada em `instances`"
            ));
        }
        let padrao = self
            .default_instance
            .as_deref()
            .map(str::trim)
            .filter(|valor| !valor.is_empty());
        match padrao {
            Some(padrao) if !vistos.contains(padrao) => problemas.push(format!(
                "default_instance: `{padrao}` não está declarada em `instances`"
            )),
            None if self.instances.len() > 1 => problemas.push(
                "default_instance: obrigatória quando o cluster tem mais de uma instância (é para \
                 onde vão as orgs fora do mapa)"
                    .to_owned(),
            ),
            _ => {}
        }
        for (org, instancia) in &self.orgs {
            let chave = org.trim();
            if chave.is_empty() {
                problemas.push("orgs: chave de org vazia".to_owned());
                continue;
            }
            if chave.chars().any(char::is_whitespace) {
                problemas.push(format!("orgs.{org}: a chave da org não pode ter espaços"));
            }
            let destino = instancia.trim();
            if !vistos.contains(destino) {
                problemas.push(format!(
                    "orgs.{org}: a instância `{destino}` não existe em `instances`"
                ));
            }
        }
        problemas
    }

    /// Confere o mapa inteiro; devolve o primeiro problema com a chave no motivo.
    pub fn validar(&self) -> DomainResult<()> {
        let problemas = self.problemas();
        if problemas.is_empty() {
            Ok(())
        } else {
            Err(DomainError::Validacao {
                campo: "cluster",
                motivo: problemas.join("; "),
            })
        }
    }

    /// Impressão digital estável do mapa, para comparar instâncias (HR-50b).
    ///
    /// Não é criptográfica: é um par de hashes de 64 bits (FNV-1a e uma mistura estável) sobre o
    /// conteúdo **normalizado** — instância local, padrão resolvida, instâncias ordenadas por id,
    /// orgs ordenadas por chave. O segredo não entra. Mesmo mapa = mesma impressão; org trocada de
    /// instância = impressão diferente.
    ///
    /// Como a instância local entra no texto canônico, **duas instâncias do mesmo cluster têm
    /// impressões diferentes** (cada uma tem o seu `instance_id`). Para comparar o mapa entre elas,
    /// use [`ClusterConfig::fingerprint_do_mapa`] — é o valor que o cabeçalho `x-poph-mapa` do
    /// encaminhamento carrega (HR-50b).
    pub fn fingerprint(&self) -> String {
        Self::hash_estavel(&self.normalizado())
    }

    /// Impressão digital do **conteúdo compartilhado** do mapa, sem a instância local (HR-50b).
    ///
    /// Duas instâncias com o mesmo `cluster.toml` — a não ser pelo `instance_id`, que é o nome
    /// local — produzem a mesma impressão. É o que permite recusar encaminhamento com mapa
    /// divergente (`421 mapa_divergente`) sem confundir o nome da instância com o mapa.
    pub fn fingerprint_do_mapa(&self) -> String {
        Self::hash_estavel(&self.normalizado_do_mapa())
    }

    fn hash_estavel(texto: &str) -> String {
        let mut a: u64 = 0xcbf2_9ce4_8422_2325;
        let mut b: u64 = 0x8422_2325_cbf2_9ce4;
        for byte in texto.bytes() {
            a ^= u64::from(byte);
            a = a.wrapping_mul(0x0000_0100_0000_01b3);
            b = b.rotate_left(5) ^ u64::from(byte);
            b = b.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        }
        format!("{a:016x}{b:016x}")
    }

    /// Texto canônico que alimenta o `fingerprint` (nunca contém segredo).
    pub fn normalizado(&self) -> String {
        let mut texto = String::from("poph-cluster-1\n");
        texto.push_str(&format!("local={}\n", self.instance_id.trim()));
        texto.push_str(&self.normalizado_do_mapa());
        texto
    }

    /// Texto canônico do conteúdo compartilhado do mapa (sem a instância local, sem segredo).
    pub fn normalizado_do_mapa(&self) -> String {
        let mut instancias: Vec<(&str, &str)> = self
            .instances
            .iter()
            .map(|instancia| (instancia.id.trim(), instancia.url.trim()))
            .collect();
        instancias.sort_unstable();
        let padrao = self
            .default_instance
            .as_deref()
            .map(str::trim)
            .filter(|valor| !valor.is_empty())
            .or_else(|| {
                (self.instances.len() == 1)
                    .then(|| self.instances[0].id.trim())
                    .filter(|id| !id.is_empty())
            })
            .unwrap_or("");
        let mut texto = String::from("poph-cluster-mapa-1\n");
        texto.push_str(&format!("default={padrao}\n"));
        for (id, url) in instancias {
            texto.push_str(&format!("instance={id}\t{url}\n"));
        }
        for (org, destino) in &self.orgs {
            texto.push_str(&format!("org={}\t{}\n", org.trim(), destino.trim()));
        }
        texto
    }
}

fn id_de_instancia_valido(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn url_de_instancia_valida(url: &str) -> bool {
    let url = url.trim();
    if url.chars().any(char::is_whitespace) {
        return false;
    }
    let resto = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"));
    matches!(resto, Some(destino) if !destino.is_empty() && !destino.starts_with('/'))
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn padroes_sao_validos() {
        assert_eq!(HarnessConfig::default().validar(), Ok(()));
    }

    #[test]
    fn falha_do_modelo_fora_da_faixa_nomeia_o_campo() {
        let mut cfg = HarnessConfig::default();
        cfg.turno.tentativas_falha_modelo = 0;
        match cfg.validar() {
            Err(DomainError::Validacao { campo, .. }) => {
                assert_eq!(campo, "turno.tentativas_falha_modelo")
            }
            outro => panic!("esperado erro de validação, veio {outro:?}"),
        }
        let mut cfg = HarnessConfig::default();
        cfg.turno.atraso_falha_modelo_ms = 999;
        match cfg.validar() {
            Err(DomainError::Validacao { campo, .. }) => {
                assert_eq!(campo, "turno.atraso_falha_modelo_ms")
            }
            outro => panic!("esperado erro de validação, veio {outro:?}"),
        }
    }

    #[test]
    fn porta_fora_da_faixa_nomeia_o_campo() {
        let mut cfg = HarnessConfig::default();
        cfg.servidor.porta = 80;
        match cfg.validar() {
            Err(DomainError::Validacao { campo, .. }) => assert_eq!(campo, "servidor.porta"),
            outro => panic!("esperado erro de validação, veio {outro:?}"),
        }
    }

    #[test]
    fn turnos_por_org_nao_pode_passar_do_total() {
        let mut cfg = HarnessConfig::default();
        cfg.limites.turnos_simultaneos = 10;
        cfg.limites.turnos_simultaneos_por_org = 11;
        match cfg.validar() {
            Err(DomainError::Validacao { campo, .. }) => {
                assert_eq!(campo, "limites.turnos_simultaneos_por_org")
            }
            outro => panic!("esperado erro de validação, veio {outro:?}"),
        }
    }

    #[test]
    fn url_sem_esquema_http_e_recusada() {
        let mut cfg = HarnessConfig::default();
        cfg.llm.url_base = "127.0.0.1:8045/v1".to_owned();
        match cfg.validar() {
            Err(DomainError::Validacao { campo, .. }) => assert_eq!(campo, "llm.url_base"),
            outro => panic!("esperado erro de validação, veio {outro:?}"),
        }
    }

    #[test]
    fn caminhos_derivados_ficam_na_pasta_de_dados() {
        let cfg = HarnessConfig::default();
        assert_eq!(cfg.harness_db(), PathBuf::from("data/harness.db"));
        assert_eq!(cfg.rag_db(), PathBuf::from("data/rag.db"));
        assert_eq!(cfg.blobs(), PathBuf::from("data/blobs"));
        assert_eq!(cfg.modelos(), PathBuf::from("data/modelos"));
    }

    #[test]
    fn caminho_definido_vence_o_derivado() {
        let mut cfg = HarnessConfig::default();
        cfg.dados.rag_db = Some(PathBuf::from("/var/poph/rag.sqlite"));
        assert_eq!(cfg.rag_db(), PathBuf::from("/var/poph/rag.sqlite"));
    }

    #[test]
    fn token_global_sem_hash_argon2_e_recusado() {
        let mut cfg = HarnessConfig::default();
        cfg.admin.tokens_globais.push(TokenGlobalConfig {
            label: "console".to_owned(),
            hash: "texto-em-claro".to_owned(),
        });
        match cfg.validar() {
            Err(DomainError::Validacao { campo, motivo }) => {
                assert_eq!(campo, "admin.tokens_globais[].hash");
                assert!(motivo.contains("argon2"), "motivo: {motivo}");
            }
            outro => panic!("esperado erro de validação, veio {outro:?}"),
        }
    }

    #[test]
    fn token_global_com_hash_e_rotulo_passa() {
        let mut cfg = HarnessConfig::default();
        cfg.admin.tokens_globais.push(TokenGlobalConfig {
            label: "console".to_owned(),
            hash: "$argon2id$v=19$m=19456,t=2,p=1$c2Fs$aGFzaA".to_owned(),
        });
        assert_eq!(cfg.validar(), Ok(()));
    }
}

#[cfg(test)]
mod testes_gates {
    use super::*;

    /// Campo ausente na desserialização cai no padrão de instalação (G3 e G5 ligados), não em
    /// `false`. Vale para qualquer origem que deserialize `GatesJev` sem a tabela padrão na frente.
    #[test]
    fn campo_ausente_na_desserializacao_cai_no_padrao_de_instalacao() {
        let vazio: GatesJev = serde_json::from_str("{}").expect("desserializa");
        assert_eq!(vazio, GatesJev::padrao());
        let so_g1: GatesJev = serde_json::from_str(r#"{"g1": true}"#).expect("desserializa");
        assert!(so_g1.g1 && so_g1.g3 && so_g1.g5);
        assert!(!so_g1.g2 && so_g1.g8);
    }

    /// `Default` continua sendo "tudo desligado": é o que vale para `ativo: false` no agente.
    #[test]
    fn default_derivado_continua_tudo_desligado() {
        assert_eq!(
            GatesJev::default(),
            GatesJev {
                g1: false,
                g2: false,
                g3: false,
                g4: false,
                g5: false,
                g6: false,
                g7: false,
                g8: false,
                g9: false,
            }
        );
    }
}

#[cfg(test)]
mod testes_cluster {
    use super::*;

    fn exemplo() -> ClusterConfig {
        ClusterConfig {
            instance_id: "a".to_owned(),
            default_instance: Some("b".to_owned()),
            instances: vec![
                ClusterInstance {
                    id: "b".to_owned(),
                    url: "https://harness-b.exemplo".to_owned(),
                },
                ClusterInstance {
                    id: "a".to_owned(),
                    url: "http://127.0.0.1:7480".to_owned(),
                },
            ],
            orgs: BTreeMap::from([
                ("org_01".to_owned(), "a".to_owned()),
                ("org_02".to_owned(), "b".to_owned()),
            ]),
        }
    }

    #[test]
    fn mapa_bom_e_valido() {
        assert_eq!(exemplo().validar(), Ok(()));
    }

    #[test]
    fn instancia_unica_vazia_e_valida() {
        let vazio = ClusterConfig::default();
        assert!(vazio.vazio());
        assert!(!vazio.declara_cluster());
        assert_eq!(vazio.validar(), Ok(()));
        // Sem mapa, a org não tem destino nomeado.
        assert_eq!(vazio.instancia_da_org("org_01"), None);
    }

    #[test]
    fn cada_regra_violada_aparece_nos_problemas() {
        let mut sem_local = exemplo();
        sem_local.instance_id = "  ".to_owned();
        assert!(
            sem_local
                .problemas()
                .iter()
                .any(|p| p.starts_with("instance_id: obrigatório"))
        );

        let mut id_ruim = exemplo();
        id_ruim.instance_id = "a com espaço".to_owned();
        assert!(
            id_ruim
                .problemas()
                .iter()
                .any(|p| p.starts_with("instance_id: use letras"))
        );

        let mut repetida = exemplo();
        repetida.instances[1].id = "b".to_owned();
        assert!(
            repetida
                .problemas()
                .iter()
                .any(|p| p.contains("aparece duas vezes"))
        );

        let mut url_ruim = exemplo();
        url_ruim.instances[0].url = "harness-b.exemplo".to_owned();
        assert!(
            url_ruim
                .problemas()
                .iter()
                .any(|p| p.contains("instances[1].url"))
        );

        let mut local_fora = exemplo();
        local_fora.instance_id = "c".to_owned();
        assert!(
            local_fora
                .problemas()
                .iter()
                .any(|p| p.contains("não está declarada"))
        );

        let mut padrao_fora = exemplo();
        padrao_fora.default_instance = Some("c".to_owned());
        assert!(
            padrao_fora
                .problemas()
                .iter()
                .any(|p| p.starts_with("default_instance"))
        );

        let mut org_fora = exemplo();
        org_fora.orgs.insert("org_03".to_owned(), "c".to_owned());
        assert!(
            org_fora
                .problemas()
                .iter()
                .any(|p| p.starts_with("orgs.org_03"))
        );

        let mut org_com_espaco = exemplo();
        org_com_espaco
            .orgs
            .insert("org 04".to_owned(), "a".to_owned());
        assert!(
            org_com_espaco
                .problemas()
                .iter()
                .any(|p| p.contains("não pode ter espaços"))
        );

        // Cluster com duas instâncias e sem `default_instance`: org fora do mapa não teria dono.
        let mut sem_padrao = exemplo();
        sem_padrao.default_instance = None;
        assert!(
            sem_padrao
                .problemas()
                .iter()
                .any(|p| p.starts_with("default_instance: obrigatória"))
        );
    }

    #[test]
    fn uma_instancia_so_dispensa_default() {
        let mut uma = ClusterConfig {
            instance_id: "a".to_owned(),
            instances: vec![ClusterInstance {
                id: "a".to_owned(),
                url: "http://127.0.0.1:7480".to_owned(),
            }],
            ..Default::default()
        };
        assert_eq!(uma.validar(), Ok(()));
        assert_eq!(uma.instancia_da_org("org_nova"), Some("a"));
        uma.default_instance = Some("a".to_owned());
        assert_eq!(uma.validar(), Ok(()));
    }

    #[test]
    fn mapa_define_o_destino_da_org() {
        let mapa = exemplo();
        assert_eq!(mapa.instancia_da_org("org_01"), Some("a"));
        assert_eq!(mapa.instancia_da_org("org_02"), Some("b"));
        assert_eq!(mapa.instancia_da_org("org_nova"), Some("b"));
    }

    #[test]
    fn fingerprint_e_estavel_e_sensivel_ao_mapa() {
        let mapa = exemplo();
        assert_eq!(mapa.fingerprint(), exemplo().fingerprint());
        // Ordem dos itens no arquivo não importa.
        let mut invertido = exemplo();
        invertido.instances.reverse();
        assert_eq!(mapa.fingerprint(), invertido.fingerprint());
        // Trocar uma org de instância muda.
        let mut trocado = exemplo();
        trocado.orgs.insert("org_01".to_owned(), "b".to_owned());
        assert_ne!(mapa.fingerprint(), trocado.fingerprint());
        // Instância local faz parte da impressão.
        let mut outra_local = exemplo();
        outra_local.instance_id = "b".to_owned();
        assert_ne!(mapa.fingerprint(), outra_local.fingerprint());
        // O texto normalizado nunca traz segredo (não há segredo no tipo).
        assert!(!mapa.normalizado().to_lowercase().contains("secret"));
        assert_eq!(mapa.fingerprint().len(), 32);
    }
}

#[cfg(test)]
mod testes_matrix {
    use super::*;

    /// Seção completa e ligada, com o que a MX-07 considera válido.
    fn ligada() -> MatrixConfig {
        MatrixConfig {
            habilitado: true,
            homeserver_url: "http://127.0.0.1:18008".to_owned(),
            server_name: "matrix-dev.local".to_owned(),
            prefixo: "ph".to_owned(),
            ..Default::default()
        }
    }

    fn config_com(matrix: MatrixConfig) -> HarnessConfig {
        HarnessConfig {
            matrix: Some(matrix),
            ..Default::default()
        }
    }

    fn campo_do_erro(cfg: &HarnessConfig) -> &'static str {
        match cfg.validar() {
            Err(DomainError::Validacao { campo, .. }) => campo,
            outro => panic!("esperado erro de validação, veio {outro:?}"),
        }
    }

    #[test]
    fn secao_ligada_valida_passa() {
        assert_eq!(config_com(ligada()).validar(), Ok(()));
        // `localhost` com porta também vale: é o que o Synapse de desenvolvimento usa.
        let mut com_porta = ligada();
        com_porta.server_name = "localhost:18008".to_owned();
        assert_eq!(config_com(com_porta).validar(), Ok(()));
    }

    #[test]
    fn secao_ausente_mantem_o_modulo_desligado() {
        let cfg = HarnessConfig::default();
        assert!(cfg.matrix.is_none());
        assert_eq!(cfg.validar(), Ok(()));
    }

    #[test]
    fn desligada_nao_confere_nada() {
        // Uma instalação que desliga o módulo pode deixar a seção vazia: `habilitado = false` é o
        // suficiente para nada do módulo carregar, e valores ruins não impedem a subida.
        let cfg = config_com(MatrixConfig::default());
        assert_eq!(cfg.validar(), Ok(()));

        let desligada_com_lixo = MatrixConfig {
            habilitado: false,
            homeserver_url: "sem-esquema".to_owned(),
            server_name: "com espaço e barra/".to_owned(),
            prefixo: "PH_MAIÚSCULO".to_owned(),
            concorrencia: 0,
            intervalo_ms: 0,
            poder_agente: 200,
            midia_retencao_dias: 0,
        };
        assert_eq!(config_com(desligada_com_lixo).validar(), Ok(()));
    }

    #[test]
    fn padroes_da_secao_sao_os_do_prd() {
        let padrao = MatrixConfig::default();
        assert!(!padrao.habilitado);
        assert_eq!(padrao.prefixo, "ph");
        assert_eq!(padrao.concorrencia, 8);
        assert_eq!(padrao.intervalo_ms, 500);
        assert_eq!(padrao.poder_agente, 50);
        assert_eq!(padrao.midia_retencao_dias, 4);
    }

    #[test]
    fn url_invalida_nomeia_a_chave_sem_o_valor() {
        for endereco in ["", "127.0.0.1:18008", "ftp://synapse.local"] {
            let mut matrix = ligada();
            matrix.homeserver_url = endereco.to_owned();
            let cfg = config_com(matrix);
            assert_eq!(campo_do_erro(&cfg), "matrix.homeserver_url");
            let mensagem = cfg.validar().unwrap_err().to_string();
            if !endereco.is_empty() {
                assert!(
                    !mensagem.contains(endereco),
                    "o erro não deve repetir o valor: {mensagem}"
                );
            }
        }
    }

    #[test]
    fn prefixo_invalido_nomeia_a_chave_sem_o_valor() {
        for prefixo in ["", "PH", "ph-matrix", "ph_matrix", "0123456789abc"] {
            let mut matrix = ligada();
            matrix.prefixo = prefixo.to_owned();
            let cfg = config_com(matrix);
            assert_eq!(campo_do_erro(&cfg), "matrix.prefixo");
            let mensagem = cfg.validar().unwrap_err().to_string();
            if !prefixo.is_empty() {
                assert!(
                    !mensagem.contains(prefixo),
                    "o erro não deve repetir o valor: {mensagem}"
                );
            }
        }
        // Os limites da faixa passam.
        for prefixo in ["ph", "a", "0123456789ab"] {
            let mut matrix = ligada();
            matrix.prefixo = prefixo.to_owned();
            assert_eq!(config_com(matrix).validar(), Ok(()), "prefixo {prefixo}");
        }
    }

    #[test]
    fn server_name_invalido_nomeia_a_chave() {
        for nome in [
            "",
            " matrix-dev.local",
            "matrix dev local",
            "https://matrix-dev.local",
            "-x.local",
            "x.local:",
            "x.local:70000",
        ] {
            let mut matrix = ligada();
            matrix.server_name = nome.to_owned();
            let cfg = config_com(matrix);
            assert_eq!(campo_do_erro(&cfg), "matrix.server_name", "nome {nome:?}");
        }
    }

    #[test]
    fn limites_fora_da_faixa_nomeiam_a_chave() {
        let mut casos: Vec<(MatrixConfig, &'static str)> = Vec::new();
        let mut matrix = ligada();
        matrix.concorrencia = 0;
        casos.push((matrix, "matrix.concorrencia"));
        let mut matrix = ligada();
        matrix.concorrencia = 65;
        casos.push((matrix, "matrix.concorrencia"));
        let mut matrix = ligada();
        matrix.intervalo_ms = 49;
        casos.push((matrix, "matrix.intervalo_ms"));
        let mut matrix = ligada();
        matrix.intervalo_ms = 60_001;
        casos.push((matrix, "matrix.intervalo_ms"));
        let mut matrix = ligada();
        matrix.poder_agente = 101;
        casos.push((matrix, "matrix.poder_agente"));
        let mut matrix = ligada();
        matrix.midia_retencao_dias = 0;
        casos.push((matrix, "matrix.midia_retencao_dias"));
        let mut matrix = ligada();
        matrix.midia_retencao_dias = 366;
        casos.push((matrix, "matrix.midia_retencao_dias"));
        for (matrix, esperado) in casos {
            assert_eq!(campo_do_erro(&config_com(matrix)), esperado);
        }
    }

    #[test]
    fn chave_desconhecida_na_secao_e_recusada() {
        let erro = serde_json::from_str::<HarnessConfig>(
            r#"{"matrix": {"habilitado": true, "naoexiste": 1}}"#,
        )
        .expect_err("chave desconhecida deve ser recusada");
        assert!(
            erro.to_string().contains("naoexiste"),
            "o erro deve citar a chave: {erro}"
        );
    }

    #[test]
    fn serializacao_nao_grava_secao_ausente() {
        let cfg = HarnessConfig::default();
        let json = serde_json::to_string(&cfg).expect("serializa");
        assert!(!json.contains("matrix"), "json: {json}");
    }
}

#[cfg(test)]
mod testes_fluxos {
    use super::*;

    /// Cada chave numérica de `[fluxos]` com a faixa da validação: (nome, mínimo, máximo).
    const FAIXAS: &[(&str, u64, u64)] = &[
        ("max_fluxos_por_org", 1, 10_000),
        ("max_fonte_bytes", 1_024, 1_048_576),
        ("max_nos", 1, 60),
        ("max_dados_bytes", 1_024, 10_485_760),
        ("max_ticks_regra", 100, 1_000_000),
        ("max_heap_regra", 65_536, 67_108_864),
        ("max_saltos_decisao", 1, 100),
        ("cache_revisoes_mb", 0, 4096),
    ];

    /// Põe `valor` na chave `nome` (usada só com as chaves de `FAIXAS`).
    fn definir(fluxos: &mut FluxosConfig, nome: &str, valor: u64) {
        match nome {
            "max_fluxos_por_org" => fluxos.max_fluxos_por_org = u32::try_from(valor).unwrap(),
            "max_fonte_bytes" => fluxos.max_fonte_bytes = valor,
            "max_nos" => fluxos.max_nos = u32::try_from(valor).unwrap(),
            "max_dados_bytes" => fluxos.max_dados_bytes = valor,
            "max_ticks_regra" => fluxos.max_ticks_regra = valor,
            "max_heap_regra" => fluxos.max_heap_regra = valor,
            "max_saltos_decisao" => fluxos.max_saltos_decisao = u32::try_from(valor).unwrap(),
            "cache_revisoes_mb" => fluxos.cache_revisoes_mb = u32::try_from(valor).unwrap(),
            outro => panic!("chave sem faixa no teste: {outro}"),
        }
    }

    fn com(fluxos: FluxosConfig) -> HarnessConfig {
        HarnessConfig {
            fluxos,
            ..Default::default()
        }
    }

    /// Campo que a validação aponta quando a seção está fora da regra.
    fn campo_do_erro(fluxos: FluxosConfig) -> &'static str {
        match com(fluxos).validar() {
            Err(DomainError::Validacao { campo, .. }) => campo,
            outro => panic!("esperado erro de validação, veio {outro:?}"),
        }
    }

    #[test]
    fn padroes_da_secao_sao_os_do_prd() {
        let padrao = FluxosConfig::default();
        assert!(padrao.ativo);
        assert_eq!(padrao.max_fluxos_por_org, 100);
        assert_eq!(padrao.max_fonte_bytes, 65_536);
        assert_eq!(padrao.max_nos, 60);
        assert_eq!(padrao.max_dados_bytes, 65_536);
        assert_eq!(padrao.max_ticks_regra, 10_000);
        assert_eq!(padrao.max_heap_regra, 1_048_576);
        assert_eq!(padrao.max_saltos_decisao, 10);
        assert_eq!(padrao.cache_revisoes_mb, 16);
    }

    #[test]
    fn secao_ausente_vale_o_padrao_e_passa() {
        let cfg = HarnessConfig::default();
        assert_eq!(cfg.fluxos, FluxosConfig::default());
        assert_eq!(cfg.validar(), Ok(()));
    }

    #[test]
    fn cada_faixa_aceita_as_bordas() {
        for (nome, minimo, maximo) in FAIXAS {
            let mut fluxos = FluxosConfig::default();
            definir(&mut fluxos, nome, *minimo);
            assert_eq!(com(fluxos.clone()).validar(), Ok(()), "{nome} no mínimo");
            definir(&mut fluxos, nome, *maximo);
            assert_eq!(com(fluxos).validar(), Ok(()), "{nome} no máximo");
        }
    }

    #[test]
    fn cada_faixa_recusa_fora_dos_limites_e_nomeia_a_chave() {
        for (nome, minimo, maximo) in FAIXAS {
            let esperado = format!("fluxos.{nome}");
            let mut acima = FluxosConfig::default();
            definir(&mut acima, nome, maximo + 1);
            assert_eq!(campo_do_erro(acima), esperado, "{nome} acima do máximo");
            if let Some(abaixo) = minimo.checked_sub(1) {
                let mut abaixo_do_minimo = FluxosConfig::default();
                definir(&mut abaixo_do_minimo, nome, abaixo);
                assert_eq!(
                    campo_do_erro(abaixo_do_minimo),
                    esperado,
                    "{nome} abaixo do mínimo"
                );
            }
        }
    }

    #[test]
    fn a_mensagem_do_erro_traz_a_chave_e_a_faixa_mas_nao_o_valor() {
        let fluxos = FluxosConfig {
            max_nos: 61,
            ..FluxosConfig::default()
        };
        let erro = com(fluxos).validar().expect_err("61 nós passa do limite");
        let texto = erro.to_string();
        assert!(texto.contains("fluxos.max_nos"), "{texto}");
        assert!(texto.contains("1 a 60"), "{texto}");
        assert!(
            !texto.contains("61"),
            "o valor recebido não entra na mensagem: {texto}"
        );
    }

    #[test]
    fn cache_zero_e_valido_e_quer_dizer_nao_guardar() {
        let fluxos = FluxosConfig {
            cache_revisoes_mb: 0,
            ..FluxosConfig::default()
        };
        assert_eq!(com(fluxos).validar(), Ok(()));
    }

    #[test]
    fn motor_desligado_ainda_confere_os_tetos() {
        let fluxos = FluxosConfig {
            ativo: false,
            max_saltos_decisao: 0,
            ..FluxosConfig::default()
        };
        assert_eq!(campo_do_erro(fluxos), "fluxos.max_saltos_decisao");
    }

    #[test]
    fn numero_negativo_na_secao_e_recusado_pelo_tipo() {
        // O erro do serde diz o tipo; quem nomeia a chave é a camada que lê o arquivo (crate `poph`).
        let erro = serde_json::from_str::<HarnessConfig>(r#"{"fluxos": {"max_nos": -1}}"#)
            .expect_err("negativo não cabe em u32");
        assert!(erro.to_string().contains("u32"), "{erro}");
    }

    #[test]
    fn chave_desconhecida_na_secao_fluxos_e_recusada() {
        let erro = serde_json::from_str::<HarnessConfig>(r#"{"fluxos": {"naoexiste": 1}}"#)
            .expect_err("chave desconhecida deve ser recusada");
        assert!(erro.to_string().contains("naoexiste"), "{erro}");
    }

    #[test]
    fn g9_vem_ligado_no_padrao_de_instalacao_e_desligado_no_derivado() {
        assert!(GatesJev::padrao().g9);
        assert!(!GatesJev::default().g9);
        let sem_gates: HarnessConfig =
            serde_json::from_str(r#"{"jev": {"gates": {"g1": true}}}"#).expect("lê");
        assert!(
            sem_gates.jev.gates.g9,
            "g9 ausente no arquivo cai no padrão"
        );
        let desligado: HarnessConfig =
            serde_json::from_str(r#"{"jev": {"gates": {"g9": false}}}"#).expect("lê");
        assert!(!desligado.jev.gates.g9);
    }
}
