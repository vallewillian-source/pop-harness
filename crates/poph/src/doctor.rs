use std::fs::{self, File};
use std::path::Path;

use poph_types::{HarnessConfig, Secret};
use serde::Serialize;
use tracing::debug;

use crate::config::{
    self, ErroConfig, Segredos, VAR_ANTIGRAVITY, VAR_CIFRA, VAR_EVOLUTION, VAR_TYPESAFE,
};

pub const REPORT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Nivel {
    Ok,
    #[serde(rename = "aviso")]
    Warning,
    #[serde(rename = "erro")]
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    pub nome: String,
    pub nivel: Nivel,
    #[serde(rename = "detalhe")]
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Binary {
    pub nome: &'static str,
    pub versao: &'static str,
    pub rustc: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    #[serde(rename = "versao_relatorio")]
    pub report_version: u32,
    pub status: &'static str,
    #[serde(rename = "binario")]
    pub binary: Binary,
    #[serde(rename = "verificacoes")]
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    pub config: Result<&'a HarnessConfig, &'a ErroConfig>,
    pub file: Option<&'a Path>,
    pub segredos: &'a Segredos,
    pub llm_key: Option<&'a Secret>,
    pub environment: Option<&'a crate::config::Ambiente>,
    pub agents_by_provider: Option<&'a [(String, u64)]>,
}

pub fn generate(input: Input<'_>) -> Report {
    let mut checks = vec![binary_check(), file_check(input.file)];

    match input.config {
        Err(error) => checks.push(Check {
            nome: "configuração".to_owned(),
            nivel: Nivel::Error,
            detail: error.to_string(),
        }),
        Ok(config) => {
            checks.push(Check {
                nome: "configuração".to_owned(),
                nivel: Nivel::Ok,
                detail: "valores válidos".to_owned(),
            });
            checks.push(data_folder(&config.data.directory));
            checks.push(database_check("harness.db", &config.harness_db()));
            checks.push(database_check("rag.db", &config.rag_db()));
            checks.extend(flow_checks(&config.flows));
            checks.extend(provider_checks(config, input));
        }
    }

    let jev_enabled = matches!(input.config, Ok(config) if config.jev.enabled);
    let evolution_configured = matches!(input.config, Ok(config) if config.evolution.is_some());

    checks.push(required_variable(VAR_CIFRA, input.segredos.cifra.is_some()));
    let llm_key_name = match input.config {
        Ok(config) => config.llm.api_key_env.as_str(),
        Err(_) => VAR_ANTIGRAVITY,
    };
    checks.push(required_variable(llm_key_name, input.llm_key.is_some()));
    if jev_enabled {
        checks.push(required_variable(
            VAR_TYPESAFE,
            input.segredos.typesafe.is_some(),
        ));
    }
    if evolution_configured {
        checks.push(required_variable(
            VAR_EVOLUTION,
            input.segredos.evolution.is_some(),
        ));
    }

    for check in &checks {
        debug!("verificacao" = %check.nome, nivel = ?check.nivel, "verificação do doctor");
    }

    let has_error = checks.iter().any(|v| v.nivel == Nivel::Error);
    Report {
        report_version: REPORT_VERSION,
        status: if has_error { "erro" } else { "ok" },
        binary: Binary {
            nome: env!("CARGO_PKG_NAME"),
            versao: env!("CARGO_PKG_VERSION"),
            rustc: env!("POPH_RUSTC_VERSAO"),
        },
        checks,
    }
}

impl Report {
    pub fn has_error(&self) -> bool {
        self.status == "erro"
    }

    pub fn texto(&self) -> String {
        let mut saida = String::from("POP Harness: diagnóstico do ambiente\n\n");
        for check in &self.checks {
            let mark = match check.nivel {
                Nivel::Ok => "[ok]   ",
                Nivel::Warning => "[aviso]",
                Nivel::Error => "[erro] ",
            };
            saida.push_str(&format!("{mark} {}: {}\n", check.nome, check.detail));
        }
        let errors = self
            .checks
            .iter()
            .filter(|v| v.nivel == Nivel::Error)
            .count();
        let warnings = self
            .checks
            .iter()
            .filter(|v| v.nivel == Nivel::Warning)
            .count();
        saida.push('\n');
        if errors == 0 {
            saida.push_str(&format!("Ambiente pronto ({warnings} aviso(s)).\n"));
        } else {
            saida.push_str(&format!(
                "{errors} erro(s) a corrigir ({warnings} aviso(s)).\n"
            ));
        }
        saida
    }

    pub fn json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|error| {
            format!("{{\"versao_relatorio\":{REPORT_VERSION},\"status\":\"erro\",\"detalhe\":\"{error}\"}}")
        })
    }
}

fn binary_check() -> Check {
    Check {
        nome: "binário e toolchain".to_owned(),
        nivel: Nivel::Ok,
        detail: format!(
            "poph {}, compilado com {}",
            env!("CARGO_PKG_VERSION"),
            env!("POPH_RUSTC_VERSAO")
        ),
    }
}

fn file_check(file: Option<&Path>) -> Check {
    let (nivel, detail) = match file {
        None => (
            Nivel::Ok,
            "nenhum arquivo indicado; usando padrões e ambiente".to_owned(),
        ),
        Some(path) if path.is_file() => (Nivel::Ok, format!("lido de {}", path.display())),
        Some(path) => (Nivel::Error, format!("{} não encontrado", path.display())),
    };
    Check {
        nome: "arquivo de configuração".to_owned(),
        nivel,
        detail,
    }
}

fn provider_checks(config: &HarnessConfig, input: Input<'_>) -> Vec<Check> {
    let mut checks = Vec::new();
    for (nome, sobreposicao) in &config.providers {
        let efetivo = sobreposicao.effective(&config.llm);
        let chave = match input.environment {
            Some(environment) => config::chave_llm(environment, &efetivo.api_key_env),
            None => None,
        };
        let (nivel, key_text) = match (input.environment, chave.is_some()) {
            (None, _) => (Nivel::Ok, String::new()),
            (Some(_), true) => (Nivel::Ok, format!("chave {} definida", efetivo.api_key_env)),
            (Some(_), false) => (
                Nivel::Warning,
                format!("chave {} ausente no ambiente", efetivo.api_key_env),
            ),
        };
        let suffix = if key_text.is_empty() {
            String::new()
        } else {
            format!(" · {key_text}")
        };
        checks.push(Check {
            nome: format!("provedor {nome}"),
            nivel,
            detail: format!(
                "{} · {} chamadas simultâneas{suffix}",
                efetivo.url_base, efetivo.concurrent_calls
            ),
        });
    }
    let Some(count) = input.agents_by_provider else {
        return checks;
    };
    let mut orphans = Vec::new();
    for (choice, total) in count {
        if *total == 0
            || HarnessConfig::provider_is_default(choice)
            || config.providers.contains_key(choice.trim())
        {
            continue;
        }
        orphans.push(format!("{choice} ({total})"));
    }
    if !orphans.is_empty() {
        checks.push(Check {
            nome: "provedor dos agentes".to_owned(),
            nivel: Nivel::Warning,
            detail: format!(
                "{} agente(s) apontam para provedor inexistente: {}",
                orphans.len(),
                orphans.join(", ")
            ),
        });
    }
    checks
}

fn data_folder(diretorio: &Path) -> Check {
    let path = std::path::absolute(diretorio).unwrap_or_else(|_| diretorio.to_path_buf());
    let (nivel, detail) = if path.is_dir() {
        if gravavel(&path) {
            (
                Nivel::Ok,
                format!("{} existe e aceita escrita", path.display()),
            )
        } else {
            (
                Nivel::Error,
                format!("{} existe, mas não aceita escrita", path.display()),
            )
        }
    } else if path.exists() {
        (
            Nivel::Error,
            format!("{} existe e não é uma pasta", path.display()),
        )
    } else {
        match path.ancestors().skip(1).find(|parent| parent.is_dir()) {
            Some(parent) if gravavel(parent) => (
                Nivel::Ok,
                format!(
                    "{} ainda não existe; será criada (a pasta acima aceita escrita)",
                    path.display()
                ),
            ),
            Some(parent) => (
                Nivel::Error,
                format!(
                    "{} não existe e {} não aceita escrita",
                    path.display(),
                    parent.display()
                ),
            ),
            None => (
                Nivel::Error,
                format!("{} não tem pasta-pai existente", path.display()),
            ),
        }
    };
    Check {
        nome: "pasta de dados".to_owned(),
        nivel,
        detail,
    }
}

fn gravavel(folder: &Path) -> bool {
    let test_file = folder.join(format!(".poph-doctor-{}.tmp", std::process::id()));
    let ok = File::create(&test_file).is_ok();
    let _ = fs::remove_file(&test_file);
    ok
}

fn database_check(nome: &str, path: &Path) -> Check {
    let nome = format!("banco {nome}");
    let (nivel, detail) = if !path.exists() {
        (
            Nivel::Warning,
            format!(
                "{} ainda não existe; crie com `poph migrate`",
                path.display()
            ),
        )
    } else if path.is_dir() {
        (
            Nivel::Error,
            format!("{} é uma pasta, não um banco", path.display()),
        )
    } else {
        match poph_store::file_version(path) {
            Ok(Some(versao)) => (
                Nivel::Ok,
                format!("formato SQLite, esquema versão {versao}"),
            ),
            Ok(None) => (
                Nivel::Warning,
                format!(
                    "{} existe, mas ainda não foi migrado; rode `poph migrate`",
                    path.display()
                ),
            ),
            Err(motivo) => (
                Nivel::Error,
                format!("{} não é um banco SQLite válido: {motivo}", path.display()),
            ),
        }
    };
    Check {
        nome,
        nivel,
        detail,
    }
}

fn flow_checks(fluxos: &poph_types::FlowsConfig) -> Vec<Check> {
    let (nivel, detail) = if fluxos.enabled {
        (Nivel::Ok, "sim: o motor de fluxos roda".to_owned())
    } else {
        (
            Nivel::Warning,
            "não: o motor de fluxos está desligado; as Fichas continuam sendo criadas".to_owned(),
        )
    };
    vec![
        Check {
            nome: "fluxos.ativo".to_owned(),
            nivel,
            detail,
        },
        Check {
            nome: "fluxos.tetos".to_owned(),
            nivel: Nivel::Ok,
            detail: format!(
                "fluxos por org {}; script {} bytes; nós {}; dados {} bytes; passos de regra {}; \
                 memória de regra {} bytes; saltos de decisão {}; cache de revisões {} MB",
                fluxos.max_flows_per_org,
                fluxos.max_source_bytes,
                fluxos.max_nodes,
                fluxos.max_data_bytes,
                fluxos.max_rule_ticks,
                fluxos.max_rule_heap,
                fluxos.max_decision_hops,
                fluxos.revision_cache_mb,
            ),
        },
    ]
}

fn required_variable(nome: &str, presente: bool) -> Check {
    let (nivel, detail) = if presente {
        (Nivel::Ok, "definida (valor não exibido)".to_owned())
    } else {
        (Nivel::Error, format!("ausente; defina {nome} no ambiente"))
    };
    Check {
        nome: format!("variável {nome}"),
        nivel,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::config::Ambiente;

    use super::*;

    fn temp_folder(label: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "poph-doctor-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&folder).expect("cria pasta de teste");
        folder
    }

    fn config_em(folder: &Path) -> HarnessConfig {
        let mut config = HarnessConfig::default();
        config.data.directory = folder.join("dados");
        config
    }

    fn full_secrets() -> Segredos {
        crate::config::segredos(&Ambiente::from([
            (VAR_CIFRA.to_owned(), "SEGREDO-CIFRA-XYZ-1".to_owned()),
            (VAR_ANTIGRAVITY.to_owned(), "SEGREDO-LLM-XYZ-2".to_owned()),
        ]))
    }

    fn llm_key_for_test() -> Secret {
        Secret::new("SEGREDO-LLM-XYZ-2".to_owned())
    }

    fn achar<'a>(report: &'a Report, nome: &str) -> &'a Check {
        report
            .checks
            .iter()
            .find(|v| v.nome == nome)
            .unwrap_or_else(|| panic!("verificação {nome} não encontrada"))
    }

    #[test]
    fn clean_environment_exits_ok_and_creates_nothing_outside_data_folder() {
        let folder = temp_folder("ok");
        let config = config_em(&folder);
        let report = generate(Input {
            config: Ok(&config),
            file: None,
            segredos: &full_secrets(),
            llm_key: Some(&llm_key_for_test()),
            environment: None,
            agents_by_provider: None,
        });
        assert!(!report.has_error(), "{}", report.texto());
        assert_eq!(achar(&report, "banco harness.db").nivel, Nivel::Warning);
        assert_eq!(achar(&report, "banco rag.db").nivel, Nivel::Warning);
    }

    #[test]
    fn doctor_lists_providers_and_warns_missing_provider() {
        let folder = temp_folder("provedores");
        let mut config = config_em(&folder);
        config.providers.insert(
            "openai-forte".to_owned(),
            poph_types::ProviderConfig {
                url_base: Some("https://api.exemplo.com/v1".to_owned()),
                concurrent_calls: Some(4),
                ..poph_types::ProviderConfig::default()
            },
        );
        let environment = Ambiente::from([
            (VAR_CIFRA.to_owned(), "SEGREDO-CIFRA-XYZ-1".to_owned()),
            (VAR_ANTIGRAVITY.to_owned(), "SEGREDO-LLM-XYZ-2".to_owned()),
        ]);
        let agents = vec![
            ("antigravity2api".to_owned(), 1_u64),
            ("openai-forte".to_owned(), 2_u64),
            ("sumiu-do-toml".to_owned(), 3_u64),
        ];
        let report = generate(Input {
            config: Ok(&config),
            file: None,
            segredos: &full_secrets(),
            llm_key: Some(&llm_key_for_test()),
            environment: Some(&environment),
            agents_by_provider: Some(&agents),
        });

        let line = achar(&report, "provedor openai-forte");
        assert_eq!(line.nivel, Nivel::Ok, "{}", line.detail);
        assert!(
            line.detail.contains("https://api.exemplo.com/v1"),
            "{}",
            line.detail
        );
        assert!(
            line.detail.contains("4 chamadas simultâneas"),
            "{}",
            line.detail
        );
        assert!(line.detail.contains(VAR_ANTIGRAVITY), "{}", line.detail);
        assert!(line.detail.contains("definida"), "{}", line.detail);

        let orphans = achar(&report, "provedor dos agentes");
        assert_eq!(orphans.nivel, Nivel::Warning);
        assert!(
            orphans.detail.contains("sumiu-do-toml"),
            "{}",
            orphans.detail
        );
        assert!(
            !orphans.detail.contains("openai-forte"),
            "{}",
            orphans.detail
        );
    }

    #[test]
    fn doctor_lists_flows_enabled_and_caps() {
        let folder = temp_folder("fluxos");
        let config = config_em(&folder);
        let report = generate(Input {
            config: Ok(&config),
            file: None,
            segredos: &full_secrets(),
            llm_key: Some(&llm_key_for_test()),
            environment: None,
            agents_by_provider: None,
        });
        let ativo = achar(&report, "fluxos.ativo");
        assert_eq!(ativo.nivel, Nivel::Ok);
        assert!(ativo.detail.starts_with("sim"), "{}", ativo.detail);
        let caps = achar(&report, "fluxos.tetos");
        assert!(
            caps.detail.contains("cache de revisões 16 MB"),
            "{}",
            caps.detail
        );
        assert!(caps.detail.contains("nós 60"), "{}", caps.detail);
        assert!(
            caps.detail.contains("saltos de decisão 10"),
            "{}",
            caps.detail
        );
        assert!(report.texto().contains("fluxos.tetos"));
    }

    #[test]
    fn doctor_warns_flow_engine_disabled_without_error() {
        let folder = temp_folder("fluxos-off");
        let mut config = config_em(&folder);
        config.flows.enabled = false;
        let report = generate(Input {
            config: Ok(&config),
            file: None,
            segredos: &full_secrets(),
            llm_key: Some(&llm_key_for_test()),
            environment: None,
            agents_by_provider: None,
        });
        let ativo = achar(&report, "fluxos.ativo");
        assert_eq!(ativo.nivel, Nivel::Warning);
        assert!(ativo.detail.starts_with("não"), "{}", ativo.detail);
        assert!(!report.has_error(), "{}", report.texto());
    }

    #[test]
    fn missing_variable_is_error_naming_it() {
        let folder = temp_folder("sem-cifra");
        let config = config_em(&folder);
        let segredos = crate::config::segredos(&Ambiente::from([(
            VAR_ANTIGRAVITY.to_owned(),
            "SEGREDO-LLM-XYZ-2".to_owned(),
        )]));
        let report = generate(Input {
            config: Ok(&config),
            file: None,
            segredos: &segredos,
            llm_key: Some(&llm_key_for_test()),
            environment: None,
            agents_by_provider: None,
        });
        assert!(report.has_error());
        let v = achar(&report, "variável POPH_CHAVE_CIFRA");
        assert_eq!(v.nivel, Nivel::Error);
        assert!(v.detail.contains("POPH_CHAVE_CIFRA"));
    }

    #[test]
    fn llm_key_name_comes_from_config() {
        let folder = temp_folder("chave-llm");
        let mut config = config_em(&folder);
        config.llm.api_key_env = "MINHA_CHAVE_LLM".to_owned();
        let report = generate(Input {
            config: Ok(&config),
            file: None,
            segredos: &full_secrets(),
            llm_key: None,
            environment: None,
            agents_by_provider: None,
        });
        let v = achar(&report, "variável MINHA_CHAVE_LLM");
        assert_eq!(v.nivel, Nivel::Error);
        assert!(v.detail.contains("MINHA_CHAVE_LLM"), "{}", v.detail);
        assert!(!report.texto().contains("variável ANTIGRAVITY_API_KEY"));
    }

    #[test]
    fn jev_key_required_only_with_jev_enabled() {
        let folder = temp_folder("jev");
        let mut config = config_em(&folder);
        let segredos = full_secrets();
        let disabled = generate(Input {
            config: Ok(&config),
            file: None,
            segredos: &segredos,
            llm_key: Some(&llm_key_for_test()),
            environment: None,
            agents_by_provider: None,
        });
        assert!(
            disabled
                .checks
                .iter()
                .all(|v| v.nome != "variável TYPESAFE_API_KEY")
        );
        config.jev.enabled = true;
        let enabled = generate(Input {
            config: Ok(&config),
            file: None,
            segredos: &segredos,
            llm_key: Some(&llm_key_for_test()),
            environment: None,
            agents_by_provider: None,
        });
        assert_eq!(
            achar(&enabled, "variável TYPESAFE_API_KEY").nivel,
            Nivel::Error
        );
    }

    #[test]
    fn secret_never_appears_in_text_or_json() {
        let folder = temp_folder("segredo");
        let config = config_em(&folder);
        let segredos = full_secrets();
        let report = generate(Input {
            config: Ok(&config),
            file: None,
            segredos: &segredos,
            llm_key: Some(&llm_key_for_test()),
            environment: None,
            agents_by_provider: None,
        });
        let texto = report.texto();
        let json = report.json();
        for value in ["SEGREDO-CIFRA-XYZ-1", "SEGREDO-LLM-XYZ-2"] {
            assert!(!texto.contains(value), "vazou no texto");
            assert!(!json.contains(value), "vazou no JSON");
        }
    }

    #[test]
    fn json_has_stable_format() {
        let folder = temp_folder("json");
        let config = config_em(&folder);
        let report = generate(Input {
            config: Ok(&config),
            file: None,
            segredos: &full_secrets(),
            llm_key: Some(&llm_key_for_test()),
            environment: None,
            agents_by_provider: None,
        });
        let value: serde_json::Value = serde_json::from_str(&report.json()).expect("JSON válido");
        assert_eq!(value["versao_relatorio"], 1);
        assert_eq!(value["status"], "ok");
        assert_eq!(value["binario"]["nome"], "poph");
        assert!(value["verificacoes"].is_array());
        let first = &value["verificacoes"][0];
        assert!(first["nome"].is_string());
        assert!(["ok", "aviso", "erro"].contains(&first["nivel"].as_str().unwrap_or("")));
    }

    #[test]
    fn migrated_database_shows_schema_version() {
        let folder = temp_folder("banco");
        let harness = folder.join("harness.db");
        let rag = folder.join("rag.db");
        let store = poph_store::Store::open_at(&harness, &rag).expect("cria os bancos");
        drop(store);

        let v = database_check("harness.db", &harness);
        assert_eq!(v.nivel, Nivel::Ok, "{}", v.detail);
        let expected = format!("versão {}", poph_store::harness_target_version());
        assert!(v.detail.contains(&expected), "{}", v.detail);

        let r = database_check("rag.db", &rag);
        assert_eq!(r.nivel, Nivel::Ok, "{}", r.detail);
        assert!(r.detail.contains("versão 0"), "{}", r.detail);
    }

    #[test]
    fn unmigrated_database_is_warning() {
        let folder = temp_folder("sem-migracao");
        let path = folder.join("rag.db");
        fs::write(&path, b"").expect("escreve arquivo vazio");
        let v = database_check("rag.db", &path);
        assert_eq!(v.nivel, Nivel::Warning, "{}", v.detail);
        assert!(v.detail.contains("poph migrate"), "{}", v.detail);
    }

    #[test]
    fn non_sqlite_file_is_error() {
        let folder = temp_folder("lixo");
        let path = folder.join("rag.db");
        fs::write(&path, b"isto nao e um banco").expect("escreve");
        assert_eq!(database_check("rag.db", &path).nivel, Nivel::Error);
    }

    #[test]
    fn data_folder_as_file_is_error() {
        let folder = temp_folder("arquivo-no-lugar");
        let path = folder.join("dados");
        fs::write(&path, b"x").expect("escreve");
        assert_eq!(data_folder(&path).nivel, Nivel::Error);
    }

    #[test]
    fn config_error_appears_in_report() {
        let error = ErroConfig::Chave {
            origem: "poph.toml".to_owned(),
            chave: "servidor.portaa".to_owned(),
            motivo: "unknown field".to_owned(),
        };
        let report = generate(Input {
            config: Err(&error),
            file: None,
            segredos: &full_secrets(),
            llm_key: Some(&llm_key_for_test()),
            environment: None,
            agents_by_provider: None,
        });
        assert!(report.has_error());
        assert!(report.texto().contains("servidor.portaa"));
    }
}
