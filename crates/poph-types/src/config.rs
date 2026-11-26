use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{DomainError, DomainResult};
use crate::integration::PluginsConfig;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct HarnessConfig {
    #[serde(rename = "servidor")]
    pub server: ServerConfig,
    #[serde(rename = "dados")]
    pub data: DataConfig,
    pub log: LogConfig,
    #[serde(rename = "retencao")]
    pub retention: RetentionConfig,
    #[serde(rename = "limites")]
    pub limits: LimitsConfig,
    pub llm: LlmConfig,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[serde(rename = "provedores")]
    pub providers: BTreeMap<String, ProviderConfig>,
    pub jev: JevConfig,
    pub handoff: HandoffConfig,
    #[serde(rename = "turno")]
    pub turn: TurnConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evolution: Option<EvolutionConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matrix: Option<MatrixConfig>,
    #[serde(rename = "fluxos")]
    pub flows: FlowsConfig,
    #[serde(default, skip_serializing_if = "PluginsConfig::empty")]
    pub plugins: PluginsConfig,
    pub admin: AdminConfig,
    pub site_chat: SiteChatConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SiteChatConfig {
    pub max_text_bytes: usize,
    pub allowed_tools: BTreeSet<String>,
}

impl Default for SiteChatConfig {
    fn default() -> Self {
        Self {
            max_text_bytes: 16 * 1024,
            allowed_tools: BTreeSet::new(),
        }
    }
}

impl SiteChatConfig {
    pub fn permits_tool(&self, name: &str) -> bool {
        matches!(
            name,
            "consultar_base"
                | "buscar_produto"
                | "cart_add_item"
                | "cart_remove_item"
                | "cart_summary"
                | "cart_clear"
        ) && self.allowed_tools.contains(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct AdminConfig {
    pub tokens_globais: Vec<GlobalTokenConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalTokenConfig {
    pub label: String,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TurnConfig {
    #[serde(rename = "intervalo_despacho_ms")]
    pub dispatch_interval_ms: u64,
    #[serde(rename = "tentativas_falha_modelo")]
    pub model_failure_attempts: u32,
    #[serde(rename = "atraso_falha_modelo_ms")]
    pub model_failure_delay_ms: u64,
}

impl Default for TurnConfig {
    fn default() -> Self {
        Self {
            dispatch_interval_ms: 1000,
            model_failure_attempts: 3,
            model_failure_delay_ms: 30_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ServerConfig {
    #[serde(rename = "endereco")]
    pub address: IpAddr,
    #[serde(rename = "porta")]
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DataConfig {
    #[serde(rename = "diretorio")]
    pub directory: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness_db: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rag_db: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blobs: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "modelos")]
    pub models: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LogConfig {
    #[serde(rename = "nivel")]
    pub level: String,
    #[serde(rename = "formato")]
    pub format: Format,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    #[default]
    #[serde(rename = "legivel")]
    Legible,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RetentionConfig {
    #[serde(rename = "dias")]
    pub days: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LimitsConfig {
    #[serde(rename = "turnos_simultaneos")]
    pub concurrent_turns: u32,
    #[serde(rename = "turnos_simultaneos_por_org")]
    pub concurrent_turns_per_org: u32,
    #[serde(rename = "taxa_por_org_por_minuto")]
    pub rate_per_org_per_minute: u32,
}

pub const ACCEPTED_LLM_PROVIDERS: [&str; 2] = ["openai_compativel", "antigravity2api"];

pub fn llm_provider_accepted(name: &str) -> bool {
    ACCEPTED_LLM_PROVIDERS.contains(&name)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LlmConfig {
    #[serde(rename = "provedor")]
    pub provider: String,
    pub url_base: String,
    #[serde(rename = "chave_env")]
    pub api_key_env: String,
    #[serde(rename = "recusa_turno_modelo")]
    pub refuse_model_turn: bool,
    #[serde(rename = "modelo")]
    pub model: String,
    #[serde(rename = "timeout_segundos")]
    pub timeout_seconds: u32,
    #[serde(rename = "retentativas")]
    pub retries: u32,
    #[serde(rename = "simultaneos")]
    pub concurrent_calls: u32,
    #[serde(rename = "temperatura")]
    pub temperature: f32,
    #[serde(rename = "max_tokens_saida")]
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ProviderConfig {
    #[serde(rename = "provedor")]
    pub provider: Option<String>,
    pub url_base: Option<String>,
    #[serde(rename = "chave_env")]
    pub api_key_env: Option<String>,
    #[serde(rename = "recusa_turno_modelo")]
    pub refuse_model_turn: Option<bool>,
    #[serde(rename = "modelo")]
    pub model: Option<String>,
    #[serde(rename = "timeout_segundos")]
    pub timeout_seconds: Option<u32>,
    #[serde(rename = "retentativas")]
    pub retries: Option<u32>,
    #[serde(rename = "simultaneos")]
    pub concurrent_calls: Option<u32>,
    #[serde(rename = "temperatura")]
    pub temperature: Option<f32>,
    #[serde(rename = "max_tokens_saida")]
    pub max_output_tokens: Option<u32>,
}

impl ProviderConfig {
    pub fn effective(&self, llm: &LlmConfig) -> LlmConfig {
        LlmConfig {
            provider: self
                .provider
                .clone()
                .unwrap_or_else(|| llm.provider.clone()),
            url_base: self
                .url_base
                .clone()
                .unwrap_or_else(|| llm.url_base.clone()),
            api_key_env: self
                .api_key_env
                .clone()
                .unwrap_or_else(|| llm.api_key_env.clone()),
            refuse_model_turn: self.refuse_model_turn.unwrap_or(llm.refuse_model_turn),
            model: self.model.clone().unwrap_or_else(|| llm.model.clone()),
            timeout_seconds: self.timeout_seconds.unwrap_or(llm.timeout_seconds),
            retries: self.retries.unwrap_or(llm.retries),
            concurrent_calls: self.concurrent_calls.unwrap_or(llm.concurrent_calls),
            temperature: self.temperature.unwrap_or(llm.temperature),
            max_output_tokens: self.max_output_tokens.unwrap_or(llm.max_output_tokens),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct JevConfig {
    #[serde(rename = "ativo")]
    pub enabled: bool,
    pub url_base: String,
    #[serde(rename = "modelo")]
    pub model: String,
    #[serde(rename = "timeout_segundos")]
    pub timeout_seconds: u32,
    #[serde(rename = "retentativas")]
    pub retries: u32,
    pub gates: JevGates,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct HandoffConfig {
    #[serde(rename = "mensagem_automatica")]
    pub automatic_message: String,
    #[serde(rename = "orientacao_ativa")]
    pub guidance_enabled: bool,
    #[serde(rename = "orientacao_confianca")]
    pub guidance_confidence: f64,
    #[serde(rename = "orientacao_timeout_segundos")]
    pub guidance_timeout_seconds: u32,
    #[serde(rename = "orientacao_ingestao_contato_produto")]
    pub guidance_product_ingestion: String,
    #[serde(rename = "orientacao_animal_peconhento")]
    pub guidance_venomous_animal: String,
    #[serde(rename = "orientacao_outro_risco_saude")]
    pub guidance_other_health_risk: String,
}

impl Default for HandoffConfig {
    fn default() -> Self {
        Self {
            automatic_message: "Sua conversa foi transferida para um atendente humano da nossa \
equipe, que já irá respondê-la por aqui. 🙋"
                .to_owned(),
            guidance_enabled: true,
            guidance_confidence: 0.5,
            guidance_timeout_seconds: 4,
            guidance_product_ingestion: "⚠️ Se alguém engoliu ou teve contato com o \
produto, procure atendimento médico agora ou ligue para o SAMU (192) ou para o \
Disque-Intoxicação (0800 722 6001). Leve a embalagem do produto. Não provoque vômito e não dê \
nada para beber ou comer sem orientação médica."
                .to_owned(),
            guidance_venomous_animal: "⚠️ Não toque nem tente pegar o animal. Mantenha crianças \
e outras pessoas longe dele e, se puder, feche a porta do cômodo. Se alguém foi picado, vá agora \
a um pronto-socorro ou ligue para o SAMU (192)."
                .to_owned(),
            guidance_other_health_risk: "⚠️ Se alguém corre risco de saúde agora, ligue para o \
SAMU (192) ou vá a um pronto-socorro sem esperar a nossa resposta."
                .to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FlowsConfig {
    #[serde(rename = "ativo")]
    pub enabled: bool,
    #[serde(rename = "max_fluxos_por_org")]
    pub max_flows_per_org: u32,
    #[serde(rename = "max_fonte_bytes")]
    pub max_source_bytes: u64,
    #[serde(rename = "max_nos")]
    pub max_nodes: u32,
    #[serde(rename = "max_dados_bytes")]
    pub max_data_bytes: u64,
    #[serde(rename = "max_ticks_regra")]
    pub max_rule_ticks: u64,
    #[serde(rename = "max_heap_regra")]
    pub max_rule_heap: u64,
    #[serde(rename = "max_saltos_decisao")]
    pub max_decision_hops: u32,
    #[serde(rename = "cache_revisoes_mb")]
    pub revision_cache_mb: u32,
}

impl Default for FlowsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_flows_per_org: 100,
            max_source_bytes: 32_768,
            max_nodes: 60,
            max_data_bytes: 65_536,
            max_rule_ticks: 10_000,
            max_rule_heap: 1_048_576,
            max_decision_hops: 10,
            revision_cache_mb: 16,
        }
    }
}

impl FlowsConfig {
    pub fn validar(&self) -> DomainResult<()> {
        validate_flows(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct JevGates {
    #[serde(default)]
    pub g1: bool,
    #[serde(default)]
    pub g2: bool,
    #[serde(default = "enabled_by_default")]
    pub g3: bool,
    #[serde(default)]
    pub g4: bool,
    #[serde(default = "enabled_by_default")]
    pub g5: bool,
    #[serde(default)]
    pub g6: bool,
    #[serde(default)]
    pub g7: bool,
    #[serde(default = "enabled_by_default")]
    pub g8: bool,
    #[serde(default = "enabled_by_default")]
    pub g9: bool,
}

fn enabled_by_default() -> bool {
    true
}

impl JevGates {
    pub const fn default_value() -> Self {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvolutionConfig {
    pub url_base: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[serde(rename = "instancias")]
    pub instances: Vec<EvolutionInstanceConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvolutionInstanceConfig {
    pub org_id: String,
    #[serde(rename = "instancia")]
    pub instance: String,
    #[serde(rename = "integracao_id")]
    pub integration_id: String,
    #[serde(rename = "segredo_env")]
    pub secret_env: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MatrixConfig {
    #[serde(rename = "habilitado")]
    pub enabled: bool,
    pub homeserver_url: String,
    pub server_name: String,
    #[serde(rename = "prefixo")]
    pub prefix: String,
    #[serde(rename = "concorrencia")]
    pub concurrency: u32,
    #[serde(rename = "intervalo_ms")]
    pub interval_ms: u64,
    #[serde(rename = "poder_agente")]
    pub agent_power: u8,
    #[serde(rename = "midia_retencao_dias")]
    pub attachment_retention_days: u32,
}

impl Default for MatrixConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            homeserver_url: String::new(),
            server_name: String::new(),
            prefix: "ph".to_owned(),
            concurrency: 8,
            interval_ms: 500,
            agent_power: 50,
            attachment_retention_days: 4,
        }
    }
}

impl MatrixConfig {
    pub fn validar(&self) -> DomainResult<()> {
        validate_matrix(self)
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            address: IpAddr::from([127, 0, 0, 1]),
            port: 7480,
        }
    }
}

impl Default for DataConfig {
    fn default() -> Self {
        Self {
            directory: PathBuf::from("data"),
            harness_db: None,
            rag_db: None,
            blobs: None,
            models: None,
        }
    }
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info".to_owned(),
            format: Format::Legible,
        }
    }
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self { days: 30 }
    }
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            concurrent_turns: 200,
            concurrent_turns_per_org: 50,
            rate_per_org_per_minute: 600,
        }
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "antigravity2api".to_owned(),
            url_base: "http://127.0.0.1:8045/v1".to_owned(),
            api_key_env: "ANTIGRAVITY_API_KEY".to_owned(),
            refuse_model_turn: true,
            model: "gemini-3.8-flash-tiered".to_owned(),
            timeout_seconds: 150,
            retries: 2,
            concurrent_calls: 32,
            temperature: 0.0,
            max_output_tokens: 4096,
        }
    }
}

impl Default for JevConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url_base: "https://api.typesafe.ai".to_owned(),
            model: "jev-latest".to_owned(),
            timeout_seconds: 8,
            retries: 1,
            gates: JevGates::default_value(),
        }
    }
}

impl HarnessConfig {
    pub fn validar(&self) -> DomainResult<()> {
        validate_server(&self.server)?;
        validate_data(&self.data)?;
        validate_log(&self.log)?;
        validate_limits(&self.limits)?;
        validate_llm(&self.llm)?;
        self.validate_providers()?;
        validate_jev(&self.jev)?;
        validate_handoff(&self.handoff)?;
        validate_turn(&self.turn)?;
        validate_admin(&self.admin)?;
        if !(1..=256 * 1024).contains(&self.site_chat.max_text_bytes) {
            return Err(DomainError::Validation {
                field: "site_chat.max_text_bytes",
                reason: "informe de 1 a 262144 bytes".to_owned(),
            });
        }
        if self
            .site_chat
            .allowed_tools
            .iter()
            .any(|name| !self.site_chat.permits_tool(name))
        {
            return Err(DomainError::Validation {
                field: "site_chat.allowed_tools",
                reason: "a lista contém uma ferramenta indisponível para conversas do site"
                    .to_owned(),
            });
        }
        validate_plugins(&self.plugins)?;
        if let Some(evolution) = &self.evolution {
            validate_url("evolution.url_base", &evolution.url_base)?;
            validate_evolution_instances(&evolution.instances)?;
        }
        if let Some(matrix) = &self.matrix {
            matrix.validar()?;
        }
        validate_flows(&self.flows)?;
        Ok(())
    }

    pub fn provider_names(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }

    pub fn effective_provider(&self, name: &str) -> Option<LlmConfig> {
        self.providers
            .get(name.trim())
            .map(|provider| provider.effective(&self.llm))
    }

    pub fn provider_is_default(choice: &str) -> bool {
        let choice = choice.trim();
        choice.is_empty() || llm_provider_accepted(choice)
    }

    pub fn validate_provider_choice(&self, choice: &str) -> DomainResult<()> {
        if Self::provider_is_default(choice) || self.providers.contains_key(choice.trim()) {
            return Ok(());
        }
        let declared = self.provider_names();
        let list = if declared.is_empty() {
            "nenhum declarado em [provedores]".to_owned()
        } else {
            declared.join(", ")
        };
        Err(invalid(
            "provedor",
            format!(
                "`{}` não é um provedor desta instância; use vazio (o padrão do [llm]), {} ou um de [provedores]: {}",
                choice.trim(),
                ACCEPTED_LLM_PROVIDERS.join(" ou "),
                list
            ),
        ))
    }

    fn validate_providers(&self) -> DomainResult<()> {
        for (name, provider) in &self.providers {
            if !valid_provider_name(name) {
                return Err(invalid(
                    "provedores",
                    format!("nome `{name}` inválido: use letras minúsculas, números, `-` e `_`"),
                ));
            }
            if llm_provider_accepted(name) {
                return Err(invalid(
                    "provedores",
                    format!(
                        "`{name}` é nome de protocolo e não pode ser usado como nome de provedor; escolha outro"
                    ),
                ));
            }
            let url_base = provider.url_base.as_deref().unwrap_or(&self.llm.url_base);
            if url_base.trim().is_empty() {
                return Err(invalid(
                    "provedores",
                    format!("`{name}`: url_base vazia; informe em [provedores.{name}] ou em [llm]"),
                ));
            }
            let api_key_env = provider
                .api_key_env
                .as_deref()
                .unwrap_or(&self.llm.api_key_env);
            if api_key_env.trim().is_empty() {
                return Err(invalid(
                    "provedores",
                    format!(
                        "`{name}`: chave_env vazia; informe em [provedores.{name}] ou em [llm]"
                    ),
                ));
            }
            let effective = provider.effective(&self.llm);
            validate_llm(&effective)
                .map_err(|error| invalid("provedores", format!("`{name}`: {error}")))?;
        }
        Ok(())
    }

    pub fn harness_db(&self) -> PathBuf {
        path_or_default(&self.data.harness_db, &self.data.directory, "harness.db")
    }

    pub fn rag_db(&self) -> PathBuf {
        path_or_default(&self.data.rag_db, &self.data.directory, "rag.db")
    }

    pub fn blobs(&self) -> PathBuf {
        path_or_default(&self.data.blobs, &self.data.directory, "blobs")
    }

    pub fn models(&self) -> PathBuf {
        path_or_default(&self.data.models, &self.data.directory, "modelos")
    }
}

fn path_or_default(given: &Option<PathBuf>, directory: &Path, default_value: &str) -> PathBuf {
    given
        .clone()
        .unwrap_or_else(|| directory.join(default_value))
}

fn invalid(field: &'static str, reason: impl Into<String>) -> DomainError {
    DomainError::Validation {
        field,
        reason: reason.into(),
    }
}

fn validate_plugins(plugins: &PluginsConfig) -> DomainResult<()> {
    let mut seen: Vec<String> = Vec::with_capacity(plugins.integrations.len());
    for (index, plugin) in plugins.integrations.iter().enumerate() {
        let kind = plugin.kind.trim();
        if kind.is_empty() {
            return Err(invalid(
                "plugins.integracoes.kind",
                format!("item {} sem `kind`", index + 1),
            ));
        }
        if plugin.program.as_os_str().is_empty() {
            return Err(invalid(
                "plugins.integracoes.programa",
                format!("o plugin `{kind}` não declara o programa"),
            ));
        }
        if seen.iter().any(|visto| visto == kind) {
            return Err(invalid(
                "plugins.integracoes.kind",
                format!("o tipo `{kind}` está declarado duas vezes"),
            ));
        }
        seen.push(kind.to_owned());
    }
    Ok(())
}

fn validate_server(server: &ServerConfig) -> DomainResult<()> {
    if !(1024..=65535).contains(&server.port) {
        return Err(invalid(
            "servidor.porta",
            format!("{} fora da faixa 1024 a 65535", server.port),
        ));
    }
    Ok(())
}

fn validate_data(data: &DataConfig) -> DomainResult<()> {
    if data.directory.as_os_str().is_empty() {
        return Err(invalid("dados.diretorio", "caminho vazio"));
    }
    for (field, path) in [
        ("dados.harness_db", &data.harness_db),
        ("dados.rag_db", &data.rag_db),
        ("dados.blobs", &data.blobs),
        ("dados.modelos", &data.models),
    ] {
        if path.as_ref().is_some_and(|c| c.as_os_str().is_empty()) {
            return Err(invalid(field, "caminho vazio"));
        }
    }
    Ok(())
}

fn validate_log(log: &LogConfig) -> DomainResult<()> {
    const NIVEIS: [&str; 5] = ["trace", "debug", "info", "warn", "error"];
    if !NIVEIS.contains(&log.level.as_str()) {
        return Err(invalid(
            "log.nivel",
            format!(
                "`{}` não é um nível; use trace, debug, info, warn ou error",
                log.level
            ),
        ));
    }
    Ok(())
}

fn validate_limits(limits: &LimitsConfig) -> DomainResult<()> {
    if !(1..=10_000).contains(&limits.concurrent_turns) {
        return Err(invalid(
            "limites.turnos_simultaneos",
            "deve estar entre 1 e 10000",
        ));
    }
    if !(1..=limits.concurrent_turns).contains(&limits.concurrent_turns_per_org) {
        return Err(invalid(
            "limites.turnos_simultaneos_por_org",
            "deve ser de 1 até o total de turnos simultâneos",
        ));
    }
    if !(1..=100_000).contains(&limits.rate_per_org_per_minute) {
        return Err(invalid(
            "limites.taxa_por_org_por_minuto",
            "deve estar entre 1 e 100000",
        ));
    }
    Ok(())
}

fn validate_llm(llm: &LlmConfig) -> DomainResult<()> {
    if llm.provider.trim().is_empty() {
        return Err(invalid(
            "llm.provedor",
            "nome do provedor vazio; use openai_compativel",
        ));
    }
    if !llm_provider_accepted(&llm.provider) {
        return Err(invalid(
            "llm.provedor",
            format!(
                "`{}` não é um provedor aceito; use {} ou {}",
                llm.provider, ACCEPTED_LLM_PROVIDERS[0], ACCEPTED_LLM_PROVIDERS[1]
            ),
        ));
    }
    validate_url("llm.url_base", &llm.url_base)?;
    if !valid_env_name(&llm.api_key_env) {
        return Err(invalid(
            "llm.chave_env",
            "deve ser o nome de uma variável de ambiente (letras, números e `_`, começando por letra ou `_`)",
        ));
    }
    if llm.model.trim().is_empty() {
        return Err(invalid("llm.modelo", "nome do modelo vazio"));
    }
    if !(1..=600).contains(&llm.timeout_seconds) {
        return Err(invalid("llm.timeout_segundos", "deve estar entre 1 e 600"));
    }
    if llm.retries > 5 {
        return Err(invalid("llm.retentativas", "no máximo 5"));
    }
    if !(1..=1000).contains(&llm.concurrent_calls) {
        return Err(invalid("llm.simultaneos", "deve estar entre 1 e 1000"));
    }
    if !(0.0..=2.0).contains(&llm.temperature) {
        return Err(invalid("llm.temperatura", "deve estar entre 0 e 2"));
    }
    if !(1..=131_072).contains(&llm.max_output_tokens) {
        return Err(invalid(
            "llm.max_tokens_saida",
            "deve estar entre 1 e 131072",
        ));
    }
    Ok(())
}

fn valid_provider_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn validate_jev(jev: &JevConfig) -> DomainResult<()> {
    validate_url("jev.url_base", &jev.url_base)?;
    if jev.model.trim().is_empty() {
        return Err(invalid("jev.modelo", "nome do modelo vazio"));
    }
    if !(1..=60).contains(&jev.timeout_seconds) {
        return Err(invalid("jev.timeout_segundos", "deve estar entre 1 e 60"));
    }
    if jev.retries > 3 {
        return Err(invalid("jev.retentativas", "no máximo 3"));
    }
    Ok(())
}

fn validate_turn(turn: &TurnConfig) -> DomainResult<()> {
    if !(100..=60_000).contains(&turn.dispatch_interval_ms) {
        return Err(invalid(
            "turno.intervalo_despacho_ms",
            "deve estar entre 100 e 60000 milissegundos",
        ));
    }
    if !(1..=10).contains(&turn.model_failure_attempts) {
        return Err(invalid(
            "turno.tentativas_falha_modelo",
            "deve estar entre 1 e 10",
        ));
    }
    if !(1_000..=600_000).contains(&turn.model_failure_delay_ms) {
        return Err(invalid(
            "turno.atraso_falha_modelo_ms",
            "deve estar entre 1000 e 600000 milissegundos",
        ));
    }
    Ok(())
}

fn validate_handoff(handoff: &HandoffConfig) -> DomainResult<()> {
    if !(0.0..=1.0).contains(&handoff.guidance_confidence) {
        return Err(invalid(
            "handoff.orientacao_confianca",
            "deve estar entre 0 e 1",
        ));
    }
    if !(1..=60).contains(&handoff.guidance_timeout_seconds) {
        return Err(invalid(
            "handoff.orientacao_timeout_segundos",
            "deve estar entre 1 e 60",
        ));
    }
    if handoff.automatic_message.trim().is_empty() {
        return Err(invalid(
            "handoff.mensagem_automatica",
            "a mensagem automática da transferência não pode ficar vazia",
        ));
    }
    Ok(())
}

fn validate_admin(admin: &AdminConfig) -> DomainResult<()> {
    for (position, token) in admin.tokens_globais.iter().enumerate() {
        let label = token.label.trim();
        if label.is_empty() || label.chars().count() > 80 {
            return Err(invalid(
                "admin.tokens_globais[].label",
                format!(
                    "entrada {}: o rótulo deve ter de 1 a 80 caracteres",
                    position + 1
                ),
            ));
        }
        if !token.hash.trim().starts_with("$argon2") {
            return Err(invalid(
                "admin.tokens_globais[].hash",
                format!(
                    "entrada {}: o hash deve ser argon2 no formato PHC (`$argon2id$...`); \
                     gere com `poph token global`",
                    position + 1
                ),
            ));
        }
    }
    Ok(())
}

fn validate_evolution_instances(instances: &[EvolutionInstanceConfig]) -> DomainResult<()> {
    use crate::ids::{IntegrationId, OrgId};
    use std::collections::BTreeSet;
    use std::str::FromStr;

    let mut seen = BTreeSet::new();
    for instance in instances {
        if OrgId::from_str(instance.org_id.trim()).is_err() {
            return Err(invalid(
                "evolution.instancias.org_id",
                "não é um id de org válido",
            ));
        }
        if IntegrationId::from_str(instance.integration_id.trim()).is_err() {
            return Err(invalid(
                "evolution.instancias.integracao_id",
                "não é um id de integração válido",
            ));
        }
        let name = instance.instance.trim();
        let safe_name = !name.is_empty()
            && name.len() <= 64
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !safe_name {
            return Err(invalid(
                "evolution.instancias.instancia",
                "use letras, números, `-` ou `_` (até 64 caracteres), como o nome da instância na Evolution",
            ));
        }
        let variable = instance.secret_env.trim();
        let valid_variable = !variable.is_empty()
            && variable
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            && !variable.starts_with(|c: char| c.is_ascii_digit());
        if !valid_variable {
            return Err(invalid(
                "evolution.instancias.segredo_env",
                "nome de variável de ambiente em maiúsculas (ex.: POPH_EVO_SEGREDO_LOJA)",
            ));
        }
        if !seen.insert((instance.org_id.trim().to_owned(), name.to_owned())) {
            return Err(invalid(
                "evolution.instancias",
                "a mesma instância aparece duas vezes para a mesma org",
            ));
        }
    }
    Ok(())
}

fn validate_url(field: &'static str, url: &str) -> DomainResult<()> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(invalid(
            field,
            "o endereço deve começar com http:// ou https://",
        ));
    }
    Ok(())
}

fn validate_matrix(matrix: &MatrixConfig) -> DomainResult<()> {
    if !matrix.enabled {
        return Ok(());
    }
    validate_url("matrix.homeserver_url", &matrix.homeserver_url)?;
    if !valid_server_name(&matrix.server_name) {
        return Err(invalid(
            "matrix.server_name",
            "use o domínio do Synapse (letras, números, `.`, `-` ou `:porta`), sem barra nem espaço",
        ));
    }
    if !prefix_valid(&matrix.prefix) {
        return Err(invalid(
            "matrix.prefixo",
            "use de 1 a 12 caracteres entre `a` e `z` ou `0` e `9`",
        ));
    }
    if !(1..=64).contains(&matrix.concurrency) {
        return Err(invalid("matrix.concorrencia", "fora da faixa 1 a 64"));
    }
    if !(50..=60_000).contains(&matrix.interval_ms) {
        return Err(invalid("matrix.intervalo_ms", "fora da faixa 50 a 60000"));
    }
    if matrix.agent_power > 100 {
        return Err(invalid(
            "matrix.poder_agente",
            "power level do Matrix vai de 0 a 100",
        ));
    }
    if !(1..=365).contains(&matrix.attachment_retention_days) {
        return Err(invalid(
            "matrix.midia_retencao_dias",
            "fora da faixa 1 a 365 dias",
        ));
    }
    Ok(())
}

fn validate_flows(flows: &FlowsConfig) -> DomainResult<()> {
    fn range_u64(field: &'static str, value: u64, min: u64, max: u64) -> DomainResult<()> {
        if (min..=max).contains(&value) {
            Ok(())
        } else {
            Err(invalid(field, format!("fora da faixa {min} a {max}")))
        }
    }
    range_u64(
        "fluxos.max_fluxos_por_org",
        u64::from(flows.max_flows_per_org),
        1,
        10_000,
    )?;
    range_u64(
        "fluxos.max_fonte_bytes",
        flows.max_source_bytes,
        1_024,
        1_048_576,
    )?;
    range_u64("fluxos.max_nos", u64::from(flows.max_nodes), 1, 60)?;
    range_u64(
        "fluxos.max_dados_bytes",
        flows.max_data_bytes,
        1_024,
        10_485_760,
    )?;
    range_u64(
        "fluxos.max_ticks_regra",
        flows.max_rule_ticks,
        100,
        1_000_000,
    )?;
    range_u64(
        "fluxos.max_heap_regra",
        flows.max_rule_heap,
        65_536,
        67_108_864,
    )?;
    range_u64(
        "fluxos.max_saltos_decisao",
        u64::from(flows.max_decision_hops),
        1,
        100,
    )?;
    range_u64(
        "fluxos.cache_revisoes_mb",
        u64::from(flows.revision_cache_mb),
        0,
        4096,
    )?;
    Ok(())
}

fn prefix_valid(prefix: &str) -> bool {
    (1..=12).contains(&prefix.len())
        && prefix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

fn valid_server_name(server_name: &str) -> bool {
    let name = server_name.trim();
    if name.is_empty() || name.len() > 255 || name != server_name {
        return false;
    }
    let (domain, port) = match name.rsplit_once(':') {
        Some((domain, port)) => (domain, Some(port)),
        None => (name, None),
    };
    if let Some(port) = port {
        if port.is_empty() || port.len() > 5 || !port.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        if port.parse::<u16>().map(|value| value == 0).unwrap_or(true) {
            return false;
        }
    }
    !domain.is_empty()
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.starts_with('-')
        && !domain.ends_with('-')
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct ClusterConfig {
    pub instance_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_instance: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instances: Vec<ClusterInstance>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub orgs: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClusterInstance {
    pub id: String,
    pub url: String,
}

impl ClusterConfig {
    pub fn declares_cluster(&self) -> bool {
        !self.instances.is_empty() || !self.orgs.is_empty()
    }

    pub fn empty(&self) -> bool {
        self.instance_id.trim().is_empty()
            && self.default_instance.is_none()
            && !self.declares_cluster()
    }

    pub fn org_instance(&self, org: &str) -> Option<&str> {
        let org = org.trim();
        if let Some(target) = self.orgs.get(org) {
            return Some(target.trim());
        }
        if let Some(default_value) = self
            .default_instance
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(default_value);
        }
        if let Some(only_instance) = self.instances.first() {
            return Some(only_instance.id.trim());
        }
        None
    }

    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.empty() {
            return problems;
        }
        let local = self.instance_id.trim();
        if local.is_empty() {
            problems.push(
                "instance_id: obrigatório quando existe cluster.toml (é o nome desta instância no \
                 mapa)"
                    .to_owned(),
            );
        } else if !valid_instance_id(local) {
            problems.push(
                "instance_id: use letras, números, `-`, `_` ou `.` (até 64 caracteres)".to_owned(),
            );
        }
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for (position, instance) in self.instances.iter().enumerate() {
            let id = instance.id.trim();
            let field = format!("instances[{}].id", position + 1);
            if !valid_instance_id(id) {
                problems.push(format!(
                    "{field}: use letras, números, `-`, `_` ou `.` (até 64 caracteres)"
                ));
            } else if !seen.insert(id.to_owned()) {
                problems.push(format!("{field}: a instância `{id}` aparece duas vezes"));
            }
            if !valid_instance_url(&instance.url) {
                problems.push(format!(
                    "instances[{}].url: o endereço deve começar com http:// ou https:// e ter um \
                     destino (ex.: https://harness-2.exemplo)",
                    position + 1
                ));
            }
        }
        if !self.instances.is_empty() && !local.is_empty() && !seen.contains(local) {
            problems.push(format!(
                "instance_id: `{local}` não está declarada em `instances`"
            ));
        }
        let default_value = self
            .default_instance
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        match default_value {
            Some(default_value) if !seen.contains(default_value) => problems.push(format!(
                "default_instance: `{default_value}` não está declarada em `instances`"
            )),
            None if self.instances.len() > 1 => problems.push(
                "default_instance: obrigatória quando o cluster tem mais de uma instância (é para \
                 onde vão as orgs fora do mapa)"
                    .to_owned(),
            ),
            _ => {}
        }
        for (org, instance) in &self.orgs {
            let key = org.trim();
            if key.is_empty() {
                problems.push("orgs: chave de org vazia".to_owned());
                continue;
            }
            if key.chars().any(char::is_whitespace) {
                problems.push(format!("orgs.{org}: a chave da org não pode ter espaços"));
            }
            let target = instance.trim();
            if !seen.contains(target) {
                problems.push(format!(
                    "orgs.{org}: a instância `{target}` não existe em `instances`"
                ));
            }
        }
        problems
    }

    pub fn validar(&self) -> DomainResult<()> {
        let problems = self.problems();
        if problems.is_empty() {
            Ok(())
        } else {
            Err(DomainError::Validation {
                field: "cluster",
                reason: problems.join("; "),
            })
        }
    }

    pub fn fingerprint(&self) -> String {
        Self::stable_hash(&self.normalized())
    }

    pub fn map_fingerprint(&self) -> String {
        Self::stable_hash(&self.normalized_map())
    }

    fn stable_hash(text: &str) -> String {
        let mut a: u64 = 0xcbf2_9ce4_8422_2325;
        let mut b: u64 = 0x8422_2325_cbf2_9ce4;
        for byte in text.bytes() {
            a ^= u64::from(byte);
            a = a.wrapping_mul(0x0000_0100_0000_01b3);
            b = b.rotate_left(5) ^ u64::from(byte);
            b = b.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        }
        format!("{a:016x}{b:016x}")
    }

    pub fn normalized(&self) -> String {
        let mut text = String::from("poph-cluster-1\n");
        text.push_str(&format!("local={}\n", self.instance_id.trim()));
        text.push_str(&self.normalized_map());
        text
    }

    pub fn normalized_map(&self) -> String {
        let mut instances: Vec<(&str, &str)> = self
            .instances
            .iter()
            .map(|instance| (instance.id.trim(), instance.url.trim()))
            .collect();
        instances.sort_unstable();
        let default_value = self
            .default_instance
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                (self.instances.len() == 1)
                    .then(|| self.instances[0].id.trim())
                    .filter(|id| !id.is_empty())
            })
            .unwrap_or("");
        let mut text = String::from("poph-cluster-mapa-1\n");
        text.push_str(&format!("default={default_value}\n"));
        for (id, url) in instances {
            text.push_str(&format!("instance={id}\t{url}\n"));
        }
        for (org, target) in &self.orgs {
            text.push_str(&format!("org={}\t{}\n", org.trim(), target.trim()));
        }
        text
    }
}

fn valid_instance_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn valid_instance_url(url: &str) -> bool {
    let url = url.trim();
    if url.chars().any(char::is_whitespace) {
        return false;
    }
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"));
    matches!(rest, Some(target) if !target.is_empty() && !target.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        assert_eq!(HarnessConfig::default().validar(), Ok(()));
    }

    #[test]
    fn model_failure_out_of_range_names_the_field() {
        let mut cfg = HarnessConfig::default();
        cfg.turn.model_failure_attempts = 0;
        match cfg.validar() {
            Err(DomainError::Validation { field, .. }) => {
                assert_eq!(field, "turno.tentativas_falha_modelo")
            }
            other => panic!("esperado erro de validação, veio {other:?}"),
        }
        let mut cfg = HarnessConfig::default();
        cfg.turn.model_failure_delay_ms = 999;
        match cfg.validar() {
            Err(DomainError::Validation { field, .. }) => {
                assert_eq!(field, "turno.atraso_falha_modelo_ms")
            }
            other => panic!("esperado erro de validação, veio {other:?}"),
        }
    }

    #[test]
    fn out_of_range_port_names_the_field() {
        let mut cfg = HarnessConfig::default();
        cfg.server.port = 80;
        match cfg.validar() {
            Err(DomainError::Validation { field, .. }) => assert_eq!(field, "servidor.porta"),
            other => panic!("esperado erro de validação, veio {other:?}"),
        }
    }

    #[test]
    fn per_org_turns_cannot_exceed_total() {
        let mut cfg = HarnessConfig::default();
        cfg.limits.concurrent_turns = 10;
        cfg.limits.concurrent_turns_per_org = 11;
        match cfg.validar() {
            Err(DomainError::Validation { field, .. }) => {
                assert_eq!(field, "limites.turnos_simultaneos_por_org")
            }
            other => panic!("esperado erro de validação, veio {other:?}"),
        }
    }

    #[test]
    fn url_without_http_scheme_is_rejected() {
        let mut cfg = HarnessConfig::default();
        cfg.llm.url_base = "127.0.0.1:8045/v1".to_owned();
        match cfg.validar() {
            Err(DomainError::Validation { field, .. }) => assert_eq!(field, "llm.url_base"),
            other => panic!("esperado erro de validação, veio {other:?}"),
        }
    }

    #[test]
    fn compatible_provider_with_own_key_and_no_refusal_validates() {
        let mut cfg = HarnessConfig::default();
        cfg.llm.provider = "openai_compativel".to_owned();
        cfg.llm.api_key_env = "MINHA_CHAVE_LLM".to_owned();
        cfg.llm.refuse_model_turn = false;
        cfg.validar().expect("configuração válida");
        assert_eq!(cfg.llm.api_key_env, "MINHA_CHAVE_LLM");
        assert!(!cfg.llm.refuse_model_turn);
    }

    #[test]
    fn empty_or_unknown_provider_rejected_with_clear_message() {
        let mut cfg = HarnessConfig::default();
        cfg.llm.provider = String::new();
        match cfg.validar() {
            Err(DomainError::Validation { field, reason }) => {
                assert_eq!(field, "llm.provedor");
                assert!(reason.contains("vazio"), "{reason}");
            }
            other => panic!("esperado erro de validação, veio {other:?}"),
        }

        cfg.llm.provider = "openai".to_owned();
        match cfg.validar() {
            Err(DomainError::Validation { field, reason }) => {
                assert_eq!(field, "llm.provedor");
                assert!(reason.contains("openai_compativel"), "{reason}");
                assert!(reason.contains("antigravity2api"), "{reason}");
            }
            other => panic!("esperado erro de validação, veio {other:?}"),
        }
    }

    #[test]
    fn named_provider_inherits_from_llm_block() {
        let mut cfg = HarnessConfig::default();
        cfg.llm.model = "modelo-do-padrao".to_owned();
        cfg.llm.concurrent_calls = 32;
        cfg.providers.insert(
            "openai-forte".to_owned(),
            ProviderConfig {
                url_base: Some("https://api.exemplo.com/v1".to_owned()),
                api_key_env: Some("CHAVE_DO_FORTE".to_owned()),
                concurrent_calls: Some(4),
                ..ProviderConfig::default()
            },
        );
        cfg.validar().expect("configuração válida");
        let effective = cfg
            .effective_provider("openai-forte")
            .expect("provedor existe");
        assert_eq!(effective.url_base, "https://api.exemplo.com/v1");
        assert_eq!(effective.api_key_env, "CHAVE_DO_FORTE");
        assert_eq!(effective.model, "modelo-do-padrao");
        assert_eq!(effective.concurrent_calls, 4);
        assert_eq!(effective.provider, cfg.llm.provider);
        assert_eq!(effective.timeout_seconds, cfg.llm.timeout_seconds);
    }

    #[test]
    fn reserved_empty_or_odd_provider_name_is_rejected() {
        for name in [
            "openai_compativel",
            "antigravity2api",
            "",
            "Com Espaço",
            "Maiúsculo",
        ] {
            let mut cfg = HarnessConfig::default();
            cfg.providers
                .insert(name.to_owned(), ProviderConfig::default());
            match cfg.validar() {
                Err(DomainError::Validation { field, reason }) => {
                    assert_eq!(field, "provedores", "nome `{name}`");
                    assert!(
                        reason.contains(name)
                            || reason.contains("vazio")
                            || reason.contains("inválido"),
                        "{reason}"
                    );
                }
                other => panic!("esperado erro de validação para `{name}`, veio {other:?}"),
            }
        }
    }

    #[test]
    fn provider_without_url_or_key_is_clear_error() {
        let mut cfg = HarnessConfig::default();
        cfg.providers.insert(
            "sem-endereco".to_owned(),
            ProviderConfig {
                url_base: Some(String::new()),
                ..ProviderConfig::default()
            },
        );
        match cfg.validar() {
            Err(DomainError::Validation { field, reason }) => {
                assert_eq!(field, "provedores");
                assert!(reason.contains("url_base"), "{reason}");
                assert!(reason.contains("sem-endereco"), "{reason}");
            }
            other => panic!("esperado erro de validação, veio {other:?}"),
        }

        let mut cfg = HarnessConfig::default();
        cfg.providers.insert(
            "sem-chave".to_owned(),
            ProviderConfig {
                url_base: Some("https://api.exemplo.com/v1".to_owned()),
                api_key_env: Some(String::new()),
                ..ProviderConfig::default()
            },
        );
        match cfg.validar() {
            Err(DomainError::Validation { field, reason }) => {
                assert_eq!(field, "provedores");
                assert!(reason.contains("chave_env"), "{reason}");
            }
            other => panic!("esperado erro de validação, veio {other:?}"),
        }
    }

    #[test]
    fn agent_provider_choice_accepts_empty_protocol_and_declared() {
        let mut cfg = HarnessConfig::default();
        cfg.providers.insert(
            "openai-forte".to_owned(),
            ProviderConfig {
                url_base: Some("https://api.exemplo.com/v1".to_owned()),
                ..ProviderConfig::default()
            },
        );
        for choice in ["", "openai_compativel", "antigravity2api", "openai-forte"] {
            cfg.validate_provider_choice(choice)
                .unwrap_or_else(|error| panic!("`{choice}` deveria valer: {error}"));
        }
        let error = cfg
            .validate_provider_choice("inexistente")
            .expect_err("recusa nome desconhecido")
            .to_string();
        assert!(error.contains("inexistente"), "{error}");
        assert!(error.contains("openai-forte"), "{error}");
    }

    #[test]
    fn empty_or_spaced_api_key_env_is_rejected() {
        for name in ["", "MINHA CHAVE", "1_CHAVE"] {
            let mut cfg = HarnessConfig::default();
            cfg.llm.api_key_env = name.to_owned();
            match cfg.validar() {
                Err(DomainError::Validation { field, .. }) => assert_eq!(field, "llm.chave_env"),
                other => panic!("esperado erro de validação para `{name}`, veio {other:?}"),
            }
        }
    }

    #[test]
    fn derived_paths_stay_in_data_dir() {
        let cfg = HarnessConfig::default();
        assert_eq!(cfg.harness_db(), PathBuf::from("data/harness.db"));
        assert_eq!(cfg.rag_db(), PathBuf::from("data/rag.db"));
        assert_eq!(cfg.blobs(), PathBuf::from("data/blobs"));
        assert_eq!(cfg.models(), PathBuf::from("data/modelos"));
    }

    #[test]
    fn given_path_wins_over_derived() {
        let mut cfg = HarnessConfig::default();
        cfg.data.rag_db = Some(PathBuf::from("/var/poph/rag.sqlite"));
        assert_eq!(cfg.rag_db(), PathBuf::from("/var/poph/rag.sqlite"));
    }

    #[test]
    fn global_token_without_argon2_hash_is_rejected() {
        let mut cfg = HarnessConfig::default();
        cfg.admin.tokens_globais.push(GlobalTokenConfig {
            label: "console".to_owned(),
            hash: "texto-em-claro".to_owned(),
        });
        match cfg.validar() {
            Err(DomainError::Validation { field, reason }) => {
                assert_eq!(field, "admin.tokens_globais[].hash");
                assert!(reason.contains("argon2"), "motivo: {reason}");
            }
            other => panic!("esperado erro de validação, veio {other:?}"),
        }
    }

    #[test]
    fn global_token_with_hash_and_label_passes() {
        let mut cfg = HarnessConfig::default();
        cfg.admin.tokens_globais.push(GlobalTokenConfig {
            label: "console".to_owned(),
            hash: "$argon2id$v=19$m=19456,t=2,p=1$c2Fs$aGFzaA".to_owned(),
        });
        assert_eq!(cfg.validar(), Ok(()));
    }
}

#[cfg(test)]
mod gates_tests {
    use super::*;

    #[test]
    fn missing_field_falls_back_to_install_default() {
        let empty: JevGates = serde_json::from_str("{}").expect("desserializa");
        assert_eq!(empty, JevGates::default_value());
        let so_g1: JevGates = serde_json::from_str(r#"{"g1": true}"#).expect("desserializa");
        assert!(so_g1.g1 && so_g1.g3 && so_g1.g5);
        assert!(!so_g1.g2 && so_g1.g8);
    }

    #[test]
    fn derived_default_stays_all_off() {
        assert_eq!(
            JevGates::default(),
            JevGates {
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
mod cluster_tests {
    use super::*;

    fn example() -> ClusterConfig {
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
    fn good_map_is_valid() {
        assert_eq!(example().validar(), Ok(()));
    }

    #[test]
    fn empty_single_instance_is_valid() {
        let empty = ClusterConfig::default();
        assert!(empty.empty());
        assert!(!empty.declares_cluster());
        assert_eq!(empty.validar(), Ok(()));
        assert_eq!(empty.org_instance("org_01"), None);
    }

    #[test]
    fn each_broken_rule_shows_in_problems() {
        let mut without_local = example();
        without_local.instance_id = "  ".to_owned();
        assert!(
            without_local
                .problems()
                .iter()
                .any(|p| p.starts_with("instance_id: obrigatório"))
        );

        let mut bad_id = example();
        bad_id.instance_id = "a com espaço".to_owned();
        assert!(
            bad_id
                .problems()
                .iter()
                .any(|p| p.starts_with("instance_id: use letras"))
        );

        let mut duplicated = example();
        duplicated.instances[1].id = "b".to_owned();
        assert!(
            duplicated
                .problems()
                .iter()
                .any(|p| p.contains("aparece duas vezes"))
        );

        let mut bad_url = example();
        bad_url.instances[0].url = "harness-b.exemplo".to_owned();
        assert!(
            bad_url
                .problems()
                .iter()
                .any(|p| p.contains("instances[1].url"))
        );

        let mut local_undeclared = example();
        local_undeclared.instance_id = "c".to_owned();
        assert!(
            local_undeclared
                .problems()
                .iter()
                .any(|p| p.contains("não está declarada"))
        );

        let mut default_undeclared = example();
        default_undeclared.default_instance = Some("c".to_owned());
        assert!(
            default_undeclared
                .problems()
                .iter()
                .any(|p| p.starts_with("default_instance"))
        );

        let mut org_undeclared = example();
        org_undeclared
            .orgs
            .insert("org_03".to_owned(), "c".to_owned());
        assert!(
            org_undeclared
                .problems()
                .iter()
                .any(|p| p.starts_with("orgs.org_03"))
        );

        let mut org_with_space = example();
        org_with_space
            .orgs
            .insert("org 04".to_owned(), "a".to_owned());
        assert!(
            org_with_space
                .problems()
                .iter()
                .any(|p| p.contains("não pode ter espaços"))
        );

        let mut without_default = example();
        without_default.default_instance = None;
        assert!(
            without_default
                .problems()
                .iter()
                .any(|p| p.starts_with("default_instance: obrigatória"))
        );
    }

    #[test]
    fn single_instance_needs_no_default() {
        let mut single = ClusterConfig {
            instance_id: "a".to_owned(),
            instances: vec![ClusterInstance {
                id: "a".to_owned(),
                url: "http://127.0.0.1:7480".to_owned(),
            }],
            ..Default::default()
        };
        assert_eq!(single.validar(), Ok(()));
        assert_eq!(single.org_instance("org_nova"), Some("a"));
        single.default_instance = Some("a".to_owned());
        assert_eq!(single.validar(), Ok(()));
    }

    #[test]
    fn map_defines_the_org_target() {
        let map = example();
        assert_eq!(map.org_instance("org_01"), Some("a"));
        assert_eq!(map.org_instance("org_02"), Some("b"));
        assert_eq!(map.org_instance("org_nova"), Some("b"));
    }

    #[test]
    fn fingerprint_is_stable_and_map_sensitive() {
        let map = example();
        assert_eq!(map.fingerprint(), example().fingerprint());
        let mut invertido = example();
        invertido.instances.reverse();
        assert_eq!(map.fingerprint(), invertido.fingerprint());
        let mut swapped = example();
        swapped.orgs.insert("org_01".to_owned(), "b".to_owned());
        assert_ne!(map.fingerprint(), swapped.fingerprint());
        let mut other_local = example();
        other_local.instance_id = "b".to_owned();
        assert_ne!(map.fingerprint(), other_local.fingerprint());
        assert!(!map.normalized().to_lowercase().contains("secret"));
        assert_eq!(map.fingerprint().len(), 32);
    }
}

#[cfg(test)]
mod matrix_tests {
    use super::*;

    fn enabled_matrix() -> MatrixConfig {
        MatrixConfig {
            enabled: true,
            homeserver_url: "http://127.0.0.1:18008".to_owned(),
            server_name: "matrix-dev.local".to_owned(),
            prefix: "ph".to_owned(),
            ..Default::default()
        }
    }

    fn config_with(matrix: MatrixConfig) -> HarnessConfig {
        HarnessConfig {
            matrix: Some(matrix),
            ..Default::default()
        }
    }

    fn error_field(cfg: &HarnessConfig) -> &'static str {
        match cfg.validar() {
            Err(DomainError::Validation { field, .. }) => field,
            other => panic!("esperado erro de validação, veio {other:?}"),
        }
    }

    #[test]
    fn enabled_section_validates() {
        assert_eq!(config_with(enabled_matrix()).validar(), Ok(()));
        let mut with_port = enabled_matrix();
        with_port.server_name = "localhost:18008".to_owned();
        assert_eq!(config_with(with_port).validar(), Ok(()));
    }

    #[test]
    fn absent_section_keeps_module_off() {
        let cfg = HarnessConfig::default();
        assert!(cfg.matrix.is_none());
        assert_eq!(cfg.validar(), Ok(()));
    }

    #[test]
    fn disabled_section_checks_nothing() {
        let cfg = config_with(MatrixConfig::default());
        assert_eq!(cfg.validar(), Ok(()));

        let disabled_with_garbage = MatrixConfig {
            enabled: false,
            homeserver_url: "sem-esquema".to_owned(),
            server_name: "com espaço e barra/".to_owned(),
            prefix: "PH_MAIÚSCULO".to_owned(),
            concurrency: 0,
            interval_ms: 0,
            agent_power: 200,
            attachment_retention_days: 0,
        };
        assert_eq!(config_with(disabled_with_garbage).validar(), Ok(()));
    }

    #[test]
    fn section_defaults_match_the_prd() {
        let default_value = MatrixConfig::default();
        assert!(!default_value.enabled);
        assert_eq!(default_value.prefix, "ph");
        assert_eq!(default_value.concurrency, 8);
        assert_eq!(default_value.interval_ms, 500);
        assert_eq!(default_value.agent_power, 50);
        assert_eq!(default_value.attachment_retention_days, 4);
    }

    #[test]
    fn invalid_url_names_key_without_value() {
        for address in ["", "127.0.0.1:18008", "ftp://synapse.local"] {
            let mut matrix = enabled_matrix();
            matrix.homeserver_url = address.to_owned();
            let cfg = config_with(matrix);
            assert_eq!(error_field(&cfg), "matrix.homeserver_url");
            let message = cfg.validar().unwrap_err().to_string();
            if !address.is_empty() {
                assert!(
                    !message.contains(address),
                    "o erro não deve repetir o valor: {message}"
                );
            }
        }
    }

    #[test]
    fn invalid_prefix_names_key_without_value() {
        for prefix in ["", "PH", "ph-matrix", "ph_matrix", "0123456789abc"] {
            let mut matrix = enabled_matrix();
            matrix.prefix = prefix.to_owned();
            let cfg = config_with(matrix);
            assert_eq!(error_field(&cfg), "matrix.prefixo");
            let message = cfg.validar().unwrap_err().to_string();
            if !prefix.is_empty() {
                assert!(
                    !message.contains(prefix),
                    "o erro não deve repetir o valor: {message}"
                );
            }
        }
        for prefix in ["ph", "a", "0123456789ab"] {
            let mut matrix = enabled_matrix();
            matrix.prefix = prefix.to_owned();
            assert_eq!(config_with(matrix).validar(), Ok(()), "prefixo {prefix}");
        }
    }

    #[test]
    fn invalid_server_name_names_the_key() {
        for name in [
            "",
            " matrix-dev.local",
            "matrix dev local",
            "https://matrix-dev.local",
            "-x.local",
            "x.local:",
            "x.local:70000",
        ] {
            let mut matrix = enabled_matrix();
            matrix.server_name = name.to_owned();
            let cfg = config_with(matrix);
            assert_eq!(error_field(&cfg), "matrix.server_name", "nome {name:?}");
        }
    }

    #[test]
    fn limits_out_of_range_name_the_key() {
        let mut cases: Vec<(MatrixConfig, &'static str)> = Vec::new();
        let mut matrix = enabled_matrix();
        matrix.concurrency = 0;
        cases.push((matrix, "matrix.concorrencia"));
        let mut matrix = enabled_matrix();
        matrix.concurrency = 65;
        cases.push((matrix, "matrix.concorrencia"));
        let mut matrix = enabled_matrix();
        matrix.interval_ms = 49;
        cases.push((matrix, "matrix.intervalo_ms"));
        let mut matrix = enabled_matrix();
        matrix.interval_ms = 60_001;
        cases.push((matrix, "matrix.intervalo_ms"));
        let mut matrix = enabled_matrix();
        matrix.agent_power = 101;
        cases.push((matrix, "matrix.poder_agente"));
        let mut matrix = enabled_matrix();
        matrix.attachment_retention_days = 0;
        cases.push((matrix, "matrix.midia_retencao_dias"));
        let mut matrix = enabled_matrix();
        matrix.attachment_retention_days = 366;
        cases.push((matrix, "matrix.midia_retencao_dias"));
        for (matrix, expected) in cases {
            assert_eq!(error_field(&config_with(matrix)), expected);
        }
    }

    #[test]
    fn unknown_key_in_section_is_rejected() {
        let error = serde_json::from_str::<HarnessConfig>(
            r#"{"matrix": {"habilitado": true, "naoexiste": 1}}"#,
        )
        .expect_err("chave desconhecida deve ser recusada");
        assert!(
            error.to_string().contains("naoexiste"),
            "o erro deve citar a chave: {error}"
        );
    }

    #[test]
    fn serialization_does_not_write_absent_section() {
        let cfg = HarnessConfig::default();
        let json = serde_json::to_string(&cfg).expect("serializa");
        assert!(!json.contains("matrix"), "json: {json}");
    }
}

#[cfg(test)]
mod flows_tests {
    use super::*;

    const RANGES: &[(&str, u64, u64)] = &[
        ("max_fluxos_por_org", 1, 10_000),
        ("max_fonte_bytes", 1_024, 1_048_576),
        ("max_nos", 1, 60),
        ("max_dados_bytes", 1_024, 10_485_760),
        ("max_ticks_regra", 100, 1_000_000),
        ("max_heap_regra", 65_536, 67_108_864),
        ("max_saltos_decisao", 1, 100),
        ("cache_revisoes_mb", 0, 4096),
    ];

    fn set(flows: &mut FlowsConfig, name: &str, value: u64) {
        match name {
            "max_fluxos_por_org" => flows.max_flows_per_org = u32::try_from(value).unwrap(),
            "max_fonte_bytes" => flows.max_source_bytes = value,
            "max_nos" => flows.max_nodes = u32::try_from(value).unwrap(),
            "max_dados_bytes" => flows.max_data_bytes = value,
            "max_ticks_regra" => flows.max_rule_ticks = value,
            "max_heap_regra" => flows.max_rule_heap = value,
            "max_saltos_decisao" => flows.max_decision_hops = u32::try_from(value).unwrap(),
            "cache_revisoes_mb" => flows.revision_cache_mb = u32::try_from(value).unwrap(),
            other => panic!("chave sem faixa no teste: {other}"),
        }
    }

    fn com(flows: FlowsConfig) -> HarnessConfig {
        HarnessConfig {
            flows,
            ..Default::default()
        }
    }

    fn error_field(flows: FlowsConfig) -> &'static str {
        match com(flows).validar() {
            Err(DomainError::Validation { field, .. }) => field,
            other => panic!("esperado erro de validação, veio {other:?}"),
        }
    }

    #[test]
    fn section_defaults_match_the_prd() {
        let default_value = FlowsConfig::default();
        assert!(default_value.enabled);
        assert_eq!(default_value.max_flows_per_org, 100);
        assert_eq!(default_value.max_source_bytes, 32_768);
        assert_eq!(default_value.max_nodes, 60);
        assert_eq!(default_value.max_data_bytes, 65_536);
        assert_eq!(default_value.max_rule_ticks, 10_000);
        assert_eq!(default_value.max_rule_heap, 1_048_576);
        assert_eq!(default_value.max_decision_hops, 10);
        assert_eq!(default_value.revision_cache_mb, 16);
    }

    #[test]
    fn absent_section_uses_default_and_passes() {
        let cfg = HarnessConfig::default();
        assert_eq!(cfg.flows, FlowsConfig::default());
        assert_eq!(cfg.validar(), Ok(()));
    }

    #[test]
    fn each_range_accepts_edges() {
        for (name, min, max) in RANGES {
            let mut flows = FlowsConfig::default();
            set(&mut flows, name, *min);
            assert_eq!(com(flows.clone()).validar(), Ok(()), "{name} no mínimo");
            set(&mut flows, name, *max);
            assert_eq!(com(flows).validar(), Ok(()), "{name} no máximo");
        }
    }

    #[test]
    fn each_range_rejects_outside_and_names_key() {
        for (name, min, max) in RANGES {
            let expected = format!("fluxos.{name}");
            let mut above = FlowsConfig::default();
            set(&mut above, name, max + 1);
            assert_eq!(error_field(above), expected, "{name} acima do máximo");
            if let Some(below) = min.checked_sub(1) {
                let mut below_minimum = FlowsConfig::default();
                set(&mut below_minimum, name, below);
                assert_eq!(
                    error_field(below_minimum),
                    expected,
                    "{name} abaixo do mínimo"
                );
            }
        }
    }

    #[test]
    fn error_message_has_key_and_range_but_not_value() {
        let flows = FlowsConfig {
            max_nodes: 61,
            ..FlowsConfig::default()
        };
        let error = com(flows).validar().expect_err("61 nós passa do limite");
        let text = error.to_string();
        assert!(text.contains("fluxos.max_nos"), "{text}");
        assert!(text.contains("1 a 60"), "{text}");
        assert!(
            !text.contains("61"),
            "o valor recebido não entra na mensagem: {text}"
        );
    }

    #[test]
    fn zero_cache_is_valid_and_means_no_storage() {
        let flows = FlowsConfig {
            revision_cache_mb: 0,
            ..FlowsConfig::default()
        };
        assert_eq!(com(flows).validar(), Ok(()));
    }

    #[test]
    fn engine_off_still_checks_caps() {
        let flows = FlowsConfig {
            enabled: false,
            max_decision_hops: 0,
            ..FlowsConfig::default()
        };
        assert_eq!(error_field(flows), "fluxos.max_saltos_decisao");
    }

    #[test]
    fn negative_number_in_section_is_rejected_by_type() {
        let error = serde_json::from_str::<HarnessConfig>(r#"{"fluxos": {"max_nos": -1}}"#)
            .expect_err("negativo não cabe em u32");
        assert!(error.to_string().contains("u32"), "{error}");
    }

    #[test]
    fn unknown_key_in_flows_section_is_rejected() {
        let error = serde_json::from_str::<HarnessConfig>(r#"{"fluxos": {"naoexiste": 1}}"#)
            .expect_err("chave desconhecida deve ser recusada");
        assert!(error.to_string().contains("naoexiste"), "{error}");
    }

    #[test]
    fn g9_on_in_install_default_off_in_derived() {
        assert!(JevGates::default_value().g9);
        assert!(!JevGates::default().g9);
        let without_gates: HarnessConfig =
            serde_json::from_str(r#"{"jev": {"gates": {"g1": true}}}"#).expect("lê");
        assert!(
            without_gates.jev.gates.g9,
            "g9 ausente no arquivo cai no padrão"
        );
        let disabled: HarnessConfig =
            serde_json::from_str(r#"{"jev": {"gates": {"g9": false}}}"#).expect("lê");
        assert!(!disabled.jev.gates.g9);
    }
}
