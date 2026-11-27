use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use poph_types::{DomainError, HarnessConfig, Secret};
use serde::Deserialize;
use thiserror::Error;
use toml::{Table, Value};

pub type Environment = BTreeMap<String, String>;

pub const VAR_CONFIG: &str = "POPH_CONFIG";

pub const DEFAULT_FILE: &str = "poph.toml";

pub const VAR_CLUSTER: &str = "POPH_CLUSTER";

pub const CLUSTER_FILE: &str = "cluster.toml";

pub const VAR_CLUSTER_SECRET: &str = "POPH_CLUSTER_SECRET";

pub const VAR_CIPHER: &str = "POPH_CHAVE_CIFRA";

pub const VAR_ANTIGRAVITY: &str = "ANTIGRAVITY_API_KEY";

pub const VAR_TYPESAFE: &str = "TYPESAFE_API_KEY";

pub const VAR_EVOLUTION: &str = "EVOLUTION_API_KEY";

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("arquivo de configuração não encontrado: {}", .0.display())]
    MissingFile(PathBuf),

    #[error("não consegui ler o arquivo de configuração {}: {reason}", .file.display())]
    Read { file: PathBuf, reason: String },

    #[error("{}: erro de sintaxe no TOML: {reason}", .file.display())]
    Syntax { file: PathBuf, reason: String },

    #[error("{origin}: chave `{key}` inválida: {reason}")]
    Key {
        origin: String,
        key: String,
        reason: String,
    },

    #[error("{variable}: valor inválido ({reason})")]
    Environment {
        variable: &'static str,
        reason: String,
    },

    #[error("configuração inválida: {0}")]
    Validation(#[from] DomainError),

    #[error("erro interno ao montar a configuração: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, Default)]
pub struct Secrets {
    pub cipher: Option<Secret>,

    pub typesafe: Option<Secret>,

    pub evolution: Option<Secret>,
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    Text,
    Integer,
    Boolean,
}

const CONFIG_VARIABLES: &[(&str, &[&str], Kind)] = &[
    (
        "POPH_SERVIDOR_ENDERECO",
        &["servidor", "endereco"],
        Kind::Text,
    ),
    ("POPH_SERVIDOR_PORTA", &["servidor", "porta"], Kind::Integer),
    ("POPH_DADOS_DIRETORIO", &["dados", "diretorio"], Kind::Text),
    ("POPH_LOG_NIVEL", &["log", "nivel"], Kind::Text),
    ("POPH_LOG_FORMATO", &["log", "formato"], Kind::Text),
    ("POPH_RETENCAO_DIAS", &["retencao", "dias"], Kind::Integer),
    ("POPH_LLM_URL_BASE", &["llm", "url_base"], Kind::Text),
    ("POPH_LLM_MODELO", &["llm", "modelo"], Kind::Text),
    ("POPH_JEV_ATIVO", &["jev", "ativo"], Kind::Boolean),
    ("POPH_JEV_URL_BASE", &["jev", "url_base"], Kind::Text),
    ("POPH_FLUXOS_ATIVO", &["fluxos", "ativo"], Kind::Boolean),
    (
        "POPH_FLUXOS_MAX_FLUXOS_POR_ORG",
        &["fluxos", "max_fluxos_por_org"],
        Kind::Integer,
    ),
    (
        "POPH_FLUXOS_MAX_FONTE_BYTES",
        &["fluxos", "max_fonte_bytes"],
        Kind::Integer,
    ),
    ("POPH_FLUXOS_MAX_NOS", &["fluxos", "max_nos"], Kind::Integer),
    (
        "POPH_FLUXOS_MAX_DADOS_BYTES",
        &["fluxos", "max_dados_bytes"],
        Kind::Integer,
    ),
    (
        "POPH_FLUXOS_MAX_TICKS_REGRA",
        &["fluxos", "max_ticks_regra"],
        Kind::Integer,
    ),
    (
        "POPH_FLUXOS_MAX_HEAP_REGRA",
        &["fluxos", "max_heap_regra"],
        Kind::Integer,
    ),
    (
        "POPH_FLUXOS_MAX_SALTOS_DECISAO",
        &["fluxos", "max_saltos_decisao"],
        Kind::Integer,
    ),
    (
        "POPH_FLUXOS_CACHE_REVISOES_MB",
        &["fluxos", "cache_revisoes_mb"],
        Kind::Integer,
    ),
];

#[derive(Debug, Clone)]
struct Source {
    origin: String,
    path: Vec<String>,
    value: Value,
}

pub fn resolve_file(
    flag: Option<&Path>,
    environment: &Environment,
    current_dir: &Path,
) -> Option<PathBuf> {
    if let Some(path) = flag {
        return Some(path.to_path_buf());
    }
    if let Some(path) = environment
        .get(VAR_CONFIG)
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        return Some(PathBuf::from(path));
    }
    let default_path = current_dir.join(DEFAULT_FILE);
    default_path.is_file().then_some(default_path)
}

pub fn load(file: Option<&Path>, environment: &Environment) -> Result<HarnessConfig, ConfigError> {
    let base = defaults_as_table()?;
    let mut sources = Vec::new();
    if let Some(path) = file {
        sources.extend(file_sources(path)?);
    }
    sources.extend(environment_sources(environment)?);

    let mut merged = base.clone();
    for source in &sources {
        set_value(&mut merged, &source.path, source.value.clone());
    }
    let config = match HarnessConfig::deserialize(Value::Table(merged)) {
        Ok(config) => config,
        Err(_) => return Err(find_invalid_source(&base, &sources)),
    };
    config.validar()?;
    Ok(config)
}

pub fn cluster_path(
    environment: &Environment,
    config_file: Option<&Path>,
    current_dir: &Path,
) -> PathBuf {
    if let Some(path) = environment
        .get(VAR_CLUSTER)
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        return PathBuf::from(path);
    }
    let directory = config_file
        .and_then(Path::parent)
        .filter(|directory| !directory.as_os_str().is_empty())
        .unwrap_or(current_dir);
    directory.join(CLUSTER_FILE)
}

pub fn cluster_secret(environment: &Environment) -> Option<Secret> {
    read_secret(environment, VAR_CLUSTER_SECRET)
}

pub fn secrets(environment: &Environment) -> Secrets {
    Secrets {
        cipher: read_secret(environment, VAR_CIPHER),
        typesafe: read_secret(environment, VAR_TYPESAFE),
        evolution: read_secret(environment, VAR_EVOLUTION),
    }
}

pub fn llm_key(environment: &Environment, key_env: &str) -> Option<Secret> {
    read_secret(environment, key_env)
}

fn read_secret(environment: &Environment, variable: &str) -> Option<Secret> {
    environment
        .get(variable)
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| Secret::new(v.to_owned()))
}

fn defaults_as_table() -> Result<Table, ConfigError> {
    match Value::try_from(HarnessConfig::default()) {
        Ok(Value::Table(table)) => Ok(table),
        Ok(other) => Err(ConfigError::Internal(format!(
            "padrões viraram {} em vez de tabela",
            other.type_str()
        ))),
        Err(error) => Err(ConfigError::Internal(error.to_string())),
    }
}

fn file_sources(path: &Path) -> Result<Vec<Source>, ConfigError> {
    if !path.exists() {
        return Err(ConfigError::MissingFile(path.to_path_buf()));
    }
    let text = fs::read_to_string(path).map_err(|error| ConfigError::Read {
        file: path.to_path_buf(),
        reason: error.to_string(),
    })?;
    let table: Table = toml::from_str(&text).map_err(|error| ConfigError::Syntax {
        file: path.to_path_buf(),
        reason: error.to_string(),
    })?;
    let origin = path.display().to_string();
    let mut leaves = Vec::new();
    flatten(&table, &mut Vec::new(), &mut leaves);
    Ok(leaves
        .into_iter()
        .map(|(path, value)| Source {
            origin: origin.clone(),
            path,
            value,
        })
        .collect())
}

fn flatten(table: &Table, prefix: &mut Vec<String>, output: &mut Vec<(Vec<String>, Value)>) {
    for (key, value) in table {
        prefix.push(key.clone());
        match value {
            Value::Table(sub) => flatten(sub, prefix, output),
            _ => output.push((prefix.clone(), value.clone())),
        }
        prefix.pop();
    }
}

fn environment_sources(environment: &Environment) -> Result<Vec<Source>, ConfigError> {
    let mut sources = Vec::new();
    for (variable, path, kind) in CONFIG_VARIABLES {
        let Some(text) = environment
            .get(*variable)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        else {
            continue;
        };
        let value =
            match kind {
                Kind::Text => Value::String(text.to_owned()),
                Kind::Integer => text.parse::<i64>().map(Value::Integer).map_err(|_| {
                    ConfigError::Environment {
                        variable,
                        reason: "esperado um número inteiro".to_owned(),
                    }
                })?,
                Kind::Boolean => match text {
                    "true" => Value::Boolean(true),
                    "false" => Value::Boolean(false),
                    _ => {
                        return Err(ConfigError::Environment {
                            variable,
                            reason: "use true ou false".to_owned(),
                        });
                    }
                },
            };
        sources.push(Source {
            origin: (*variable).to_owned(),
            path: path.iter().map(|p| (*p).to_owned()).collect(),
            value,
        });
    }
    Ok(sources)
}

fn set_value(table: &mut Table, path: &[String], value: Value) {
    match path {
        [] => {}
        [last] => {
            table.insert(last.clone(), value);
        }
        [first, rest @ ..] => {
            let entry = table
                .entry(first.clone())
                .or_insert_with(|| Value::Table(Table::new()));
            if let Value::Table(sub) = entry {
                set_value(sub, rest, value);
            }
        }
    }
}

fn find_invalid_source(base: &Table, sources: &[Source]) -> ConfigError {
    for source in sources {
        let mut candidate = base.clone();
        set_value(&mut candidate, &source.path, source.value.clone());
        if let Err(error) = HarnessConfig::deserialize(Value::Table(candidate)) {
            return ConfigError::Key {
                origin: source.origin.clone(),
                key: source.path.join("."),
                reason: error.to_string(),
            };
        }
    }
    ConfigError::Internal(
        "a configuração não leu, mas nenhuma chave isolada explica o erro".to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use poph_types::JevGates;

    use super::*;

    fn temp_dir() -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("poph-config-{}-{n}", std::process::id()));
        fs::create_dir_all(&directory).expect("cria pasta de teste");
        directory
    }

    fn file(content: &str) -> PathBuf {
        let path = temp_dir().join("poph.toml");
        fs::write(&path, content).expect("escreve arquivo de teste");
        path
    }

    fn env(pairs: &[(&str, &str)]) -> Environment {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn defaults_without_file_or_environment() {
        let cfg = load(None, &Environment::new()).expect("carrega");
        assert_eq!(cfg, HarnessConfig::default());
    }

    #[test]
    fn environment_overrides_file_and_file_overrides_defaults() {
        let path = file(
            "[servidor]\nporta = 2000\n\n[log]\nnivel = \"debug\"\n\n[llm]\nmodelo = \"modelo-do-arquivo\"\n",
        );
        let environment = env(&[("POPH_SERVIDOR_PORTA", "3000")]);
        let cfg = load(Some(&path), &environment).expect("carrega");

        assert_eq!(cfg.server.port, 3000);

        assert_eq!(cfg.log.level, "debug");
        assert_eq!(cfg.llm.model, "modelo-do-arquivo");

        assert_eq!(cfg.retention.days, 30);
        assert_eq!(cfg.limits.concurrent_turns, 200);
    }

    #[test]
    fn empty_environment_variable_does_not_override() {
        let path = file("[servidor]\nporta = 2001\n");
        let cfg = load(Some(&path), &env(&[("POPH_SERVIDOR_PORTA", "  ")])).expect("carrega");
        assert_eq!(cfg.server.port, 2001);
    }

    #[test]
    fn invalid_toml_syntax_reports_file() {
        let path = file("[servidor\nporta = 2000\n");
        match load(Some(&path), &Environment::new()) {
            Err(ConfigError::Syntax { file, reason }) => {
                assert_eq!(file, path);
                assert!(!reason.is_empty());
            }
            other => panic!("esperado erro de sintaxe, veio {other:?}"),
        }
    }

    #[test]
    fn unknown_key_reports_file_and_key() {
        let path = file("[servidor]\nportaa = 2000\n");
        match load(Some(&path), &Environment::new()) {
            Err(ConfigError::Key { origin, key, .. }) => {
                assert_eq!(origin, path.display().to_string());
                assert_eq!(key, "servidor.portaa");
            }
            other => panic!("esperado erro de chave, veio {other:?}"),
        }
    }

    #[test]
    fn invalid_file_value_type_reports_key() {
        let path = file("[servidor]\nporta = \"abc\"\n");
        match load(Some(&path), &Environment::new()) {
            Err(ConfigError::Key { key, .. }) => assert_eq!(key, "servidor.porta"),
            other => panic!("esperado erro de chave, veio {other:?}"),
        }
    }

    #[test]
    fn invalid_environment_integer_reports_variable() {
        match load(None, &env(&[("POPH_SERVIDOR_PORTA", "abc")])) {
            Err(ConfigError::Environment { variable, .. }) => {
                assert_eq!(variable, "POPH_SERVIDOR_PORTA")
            }
            other => panic!("esperado erro de variável, veio {other:?}"),
        }
    }

    #[test]
    fn out_of_range_environment_port_fails_validation() {
        match load(None, &env(&[("POPH_SERVIDOR_PORTA", "80")])) {
            Err(ConfigError::Validation(DomainError::Validation { field, .. })) => {
                assert_eq!(field, "servidor.porta")
            }
            other => panic!("esperado erro de validação, veio {other:?}"),
        }
    }

    #[test]
    fn missing_explicit_file_reports_path() {
        let path = temp_dir().join("nao-existe.toml");
        assert!(matches!(
            load(Some(&path), &Environment::new()),
            Err(ConfigError::MissingFile(_))
        ));
    }

    #[test]
    fn evolution_requires_a_section() {
        let without_section = load(None, &Environment::new()).expect("carrega");
        assert!(without_section.evolution.is_none());
        let path = file("[evolution]\nurl_base = \"https://evo.exemplo.test\"\n");
        let with_section = load(Some(&path), &Environment::new()).expect("carrega");
        assert_eq!(
            with_section.evolution.map(|e| e.url_base),
            Some("https://evo.exemplo.test".to_owned())
        );
    }

    #[test]
    fn omitted_llm_fields_use_defaults() {
        let path = file(
            "[llm]\nprovedor = \"openai_compativel\"\nurl_base = \"https://api.exemplo.test/v1\"\n",
        );
        let cfg = load(Some(&path), &Environment::new()).expect("carrega");
        assert_eq!(cfg.llm.provider, "openai_compativel");
        assert_eq!(cfg.llm.api_key_env, VAR_ANTIGRAVITY);
        assert!(cfg.llm.refuse_model_turn);
        let key = llm_key(
            &env(&[("OUTRA_CHAVE", "valor-de-teste")]),
            &cfg.llm.api_key_env,
        );
        assert!(key.is_none(), "a chave vem do nome configurado");
    }

    #[test]
    fn jev_defaults_to_disabled_and_environment_can_enable_it() {
        let cfg = load(None, &Environment::new()).expect("carrega");
        assert!(!cfg.jev.enabled);
        let enabled = load(None, &env(&[("POPH_JEV_ATIVO", "true")])).expect("carrega");
        assert!(enabled.jev.enabled);
    }

    #[test]
    fn only_g3_g5_g8_and_g9_are_enabled_by_default() {
        let cfg = load(None, &Environment::new()).expect("carrega");
        assert_eq!(cfg.jev.gates, JevGates::default_value());
        assert!(cfg.jev.gates.g3, "G3 deve vir ligado por padrão");
        assert!(cfg.jev.gates.g5, "G5 deve vir ligado por padrão");
        assert!(cfg.jev.gates.g8, "G8 deve vir ligado por padrão");
        assert!(cfg.jev.gates.g9, "G9 deve vir ligado por padrão");
        for (name, enabled) in [
            ("g1", cfg.jev.gates.g1),
            ("g2", cfg.jev.gates.g2),
            ("g4", cfg.jev.gates.g4),
            ("g6", cfg.jev.gates.g6),
            ("g7", cfg.jev.gates.g7),
        ] {
            assert!(!enabled, "{name} deve continuar desligado por padrão");
        }
    }

    #[test]
    fn file_without_gates_keeps_g3_g5_and_g8_enabled() {
        let path = file("[jev]\nativo = true\n");
        let cfg = load(Some(&path), &Environment::new()).expect("carrega");
        assert!(cfg.jev.enabled);
        assert!(cfg.jev.gates.g3 && cfg.jev.gates.g5 && cfg.jev.gates.g8);
        assert!(!cfg.jev.gates.g1 && !cfg.jev.gates.g2);
    }

    #[test]
    fn partial_gates_section_overrides_only_listed_gates() {
        let path = file("[jev]\nativo = true\n\n[jev.gates]\ng1 = true\n");
        let cfg = load(Some(&path), &Environment::new()).expect("carrega");
        assert!(cfg.jev.gates.g1, "o que o arquivo liga, fica ligado");
        assert!(
            cfg.jev.gates.g3 && cfg.jev.gates.g5 && cfg.jev.gates.g8,
            "G3, G5 e G8 ausentes ficam no padrão"
        );
        assert!(!cfg.jev.gates.g2);
    }

    #[test]
    fn file_can_disable_g3_g5_and_g8() {
        let path = file("[jev]\nativo = true\n\n[jev.gates]\ng3 = false\ng5 = false\ng8 = false\n");
        let cfg = load(Some(&path), &Environment::new()).expect("carrega");
        assert!(!cfg.jev.gates.g3, "g3 = false no arquivo desliga o G3");
        assert!(!cfg.jev.gates.g5, "g5 = false no arquivo desliga o G5");
        assert!(!cfg.jev.gates.g8, "g8 = false no arquivo desliga o G8");

        assert_eq!(
            cfg.jev.gates,
            JevGates {
                g9: true,
                ..JevGates::default()
            }
        );
    }

    #[test]
    fn file_resolution_prefers_flag_then_environment_then_current_dir() {
        let directory = temp_dir();
        let empty = Environment::new();
        assert_eq!(resolve_file(None, &empty, &directory), None);
        fs::write(directory.join(DEFAULT_FILE), "").expect("escreve");
        assert_eq!(
            resolve_file(None, &empty, &directory),
            Some(directory.join(DEFAULT_FILE))
        );
        let from_environment = env(&[(VAR_CONFIG, "/tmp/outro.toml")]);
        assert_eq!(
            resolve_file(None, &from_environment, &directory),
            Some(PathBuf::from("/tmp/outro.toml"))
        );
        assert_eq!(
            resolve_file(
                Some(Path::new("/tmp/flag.toml")),
                &from_environment,
                &directory
            ),
            Some(PathBuf::from("/tmp/flag.toml"))
        );
    }

    #[test]
    fn secrets_come_from_environment_and_empty_values_are_absent() {
        let environment = env(&[(VAR_CIPHER, "segredo-de-teste-cifra")]);
        let s = secrets(&environment);
        assert!(s.cipher.is_some());
        assert!(s.typesafe.is_none());
        assert!(!format!("{s:?}").contains("segredo-de-teste-cifra"));
    }

    #[test]
    fn llm_key_uses_configured_variable_name() {
        let environment = env(&[
            (VAR_ANTIGRAVITY, "chave-padrao"),
            ("MINHA_CHAVE_LLM", "chave-propria"),
        ]);
        let default_path = llm_key(&environment, VAR_ANTIGRAVITY).expect("chave padrão");
        assert_eq!(default_path.expose(), "chave-padrao");
        let custom_key = llm_key(&environment, "MINHA_CHAVE_LLM").expect("chave própria");
        assert_eq!(custom_key.expose(), "chave-propria");
        assert!(llm_key(&environment, "OUTRA_CHAVE").is_none());
        assert!(llm_key(&environment, VAR_ANTIGRAVITY).is_some_and(|c| !c.expose().is_empty()));
    }

    #[test]
    fn flows_use_defaults_without_file_or_environment() {
        let cfg = load(None, &Environment::new()).expect("carrega");
        assert_eq!(cfg.flows, poph_types::FlowsConfig::default());
    }

    fn flow_value(flows: &poph_types::FlowsConfig, key: &str) -> u64 {
        match key {
            "max_fluxos_por_org" => u64::from(flows.max_flows_per_org),
            "max_fonte_bytes" => flows.max_source_bytes,
            "max_nos" => u64::from(flows.max_nodes),
            "max_dados_bytes" => flows.max_data_bytes,
            "max_ticks_regra" => flows.max_rule_ticks,
            "max_heap_regra" => flows.max_rule_heap,
            "max_saltos_decisao" => u64::from(flows.max_decision_hops),
            "cache_revisoes_mb" => u64::from(flows.revision_cache_mb),
            other => panic!("chave sem leitura no teste: {other}"),
        }
    }

    #[test]
    fn environment_overrides_every_numeric_flow_key() {
        let cases = [
            (
                "POPH_FLUXOS_MAX_FLUXOS_POR_ORG",
                "max_fluxos_por_org",
                "50",
                "200",
            ),
            (
                "POPH_FLUXOS_MAX_FONTE_BYTES",
                "max_fonte_bytes",
                "2048",
                "4096",
            ),
            ("POPH_FLUXOS_MAX_NOS", "max_nos", "20", "30"),
            (
                "POPH_FLUXOS_MAX_DADOS_BYTES",
                "max_dados_bytes",
                "2048",
                "4096",
            ),
            (
                "POPH_FLUXOS_MAX_TICKS_REGRA",
                "max_ticks_regra",
                "500",
                "600",
            ),
            (
                "POPH_FLUXOS_MAX_HEAP_REGRA",
                "max_heap_regra",
                "131072",
                "262144",
            ),
            (
                "POPH_FLUXOS_MAX_SALTOS_DECISAO",
                "max_saltos_decisao",
                "4",
                "5",
            ),
            (
                "POPH_FLUXOS_CACHE_REVISOES_MB",
                "cache_revisoes_mb",
                "8",
                "32",
            ),
        ];
        for (variable, key, file_value, environment_value) in cases {
            let path = file(&format!("[fluxos]\n{key} = {file_value}\n"));
            let file_only = load(Some(&path), &Environment::new()).expect("carrega");
            assert_eq!(
                flow_value(&file_only.flows, key),
                file_value.parse::<u64>().unwrap(),
                "{key} vem do arquivo"
            );
            let overridden =
                load(Some(&path), &env(&[(variable, environment_value)])).expect("carrega");
            assert_eq!(
                flow_value(&overridden.flows, key),
                environment_value.parse::<u64>().unwrap(),
                "{variable} vence o arquivo"
            );
        }
    }

    #[test]
    fn flow_enabled_environment_override_accepts_only_true_or_false() {
        let path = file("[fluxos]\nativo = true\n");
        let disabled = load(Some(&path), &env(&[("POPH_FLUXOS_ATIVO", "false")])).expect("carrega");
        assert!(!disabled.flows.enabled);
        match load(None, &env(&[("POPH_FLUXOS_ATIVO", "sim")])) {
            Err(ConfigError::Environment { variable, .. }) => {
                assert_eq!(variable, "POPH_FLUXOS_ATIVO")
            }
            other => panic!("esperado erro de variável, veio {other:?}"),
        }
    }

    #[test]
    fn nonnumeric_environment_flow_value_reports_variable() {
        match load(None, &env(&[("POPH_FLUXOS_MAX_NOS", "abc")])) {
            Err(ConfigError::Environment { variable, .. }) => {
                assert_eq!(variable, "POPH_FLUXOS_MAX_NOS")
            }
            other => panic!("esperado erro de variável, veio {other:?}"),
        }
    }

    #[test]
    fn negative_environment_flow_value_reports_key_and_variable() {
        match load(None, &env(&[("POPH_FLUXOS_MAX_NOS", "-1")])) {
            Err(ConfigError::Key { origin, key, .. }) => {
                assert_eq!(origin, "POPH_FLUXOS_MAX_NOS");
                assert_eq!(key, "fluxos.max_nos");
            }
            other => panic!("esperado erro de chave, veio {other:?}"),
        }
    }

    #[test]
    fn out_of_range_environment_flow_value_reports_key() {
        match load(None, &env(&[("POPH_FLUXOS_MAX_NOS", "61")])) {
            Err(ConfigError::Validation(DomainError::Validation { field, .. })) => {
                assert_eq!(field, "fluxos.max_nos")
            }
            other => panic!("esperado erro de validação, veio {other:?}"),
        }
    }

    #[test]
    fn out_of_range_file_flow_value_reports_key() {
        let path = file("[fluxos]\nmax_saltos_decisao = 0\n");
        match load(Some(&path), &Environment::new()) {
            Err(ConfigError::Validation(DomainError::Validation { field, .. })) => {
                assert_eq!(field, "fluxos.max_saltos_decisao")
            }
            other => panic!("esperado erro de validação, veio {other:?}"),
        }
    }

    #[test]
    fn invalid_file_flow_value_type_reports_key() {
        let path = file("[fluxos]\ncache_revisoes_mb = \"muito\"\n");
        match load(Some(&path), &Environment::new()) {
            Err(ConfigError::Key { key, .. }) => assert_eq!(key, "fluxos.cache_revisoes_mb"),
            other => panic!("esperado erro de chave, veio {other:?}"),
        }
    }

    #[test]
    fn file_can_disable_flow_engine_and_g9() {
        let path = file("[fluxos]\nativo = false\n\n[jev.gates]\ng9 = false\n");
        let cfg = load(Some(&path), &Environment::new()).expect("carrega");
        assert!(!cfg.flows.enabled);
        assert!(!cfg.jev.gates.g9);
    }
}
