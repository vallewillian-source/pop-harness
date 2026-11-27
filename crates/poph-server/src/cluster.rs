use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use poph_types::Secret;
use poph_types::config::{ClusterConfig, ClusterInstance};
use serde::Serialize;
use thiserror::Error;

use crate::state::ServerState;

pub const SECRET_ENV: &str = "POPH_CLUSTER_SECRET";

#[derive(Debug, Error)]
pub enum ClusterError {
    #[error("não consegui ler o arquivo do cluster {}: {reason}", .file.display())]
    Read { file: PathBuf, reason: String },
    #[error("{}: erro de sintaxe no TOML: {reason}", .file.display())]
    Syntax { file: PathBuf, reason: String },
    #[error("{}: chave `{key}` inválida: {reason}", .file.display())]
    InvalidKey {
        file: PathBuf,
        key: String,
        reason: String,
    },
    #[error("{}:\n{}", .file.display(), .problems.join("\n"))]
    Validation {
        file: PathBuf,
        problems: Vec<String>,
    },
    #[error(
        "{}: o cluster declara instâncias, mas falta o segredo compartilhado; defina {SECRET_ENV} \
         no ambiente (o segredo nunca vai no arquivo)",
        .file.display()
    )]
    MissingSecret { file: PathBuf },
}

impl ClusterError {
    pub fn problems(&self) -> Vec<String> {
        match self {
            Self::Read { reason, .. } | Self::Syntax { reason, .. } => vec![reason.clone()],
            Self::InvalidKey { key, reason, .. } => vec![format!("{key}: {reason}")],
            Self::Validation { problems, .. } => problems.clone(),
            Self::MissingSecret { .. } => vec![format!(
                "defina {SECRET_ENV} no ambiente: o cluster declara instâncias e o segredo não \
                 vai no arquivo"
            )],
        }
    }
}

#[derive(Debug, Clone)]
pub struct LiveCluster {
    pub path: PathBuf,
    pub config: ClusterConfig,
    pub secret: Option<Secret>,
}

impl LiveCluster {
    pub fn single_instance(path: PathBuf) -> Self {
        Self {
            path,
            config: ClusterConfig::default(),
            secret: None,
        }
    }

    pub fn declares_cluster(&self) -> bool {
        self.config.declares_cluster()
    }

    pub fn mode(&self) -> &'static str {
        if self.declares_cluster() {
            "cluster"
        } else {
            "instancia_unica"
        }
    }

    pub fn fingerprint(&self) -> String {
        self.config.fingerprint()
    }

    pub fn map_fingerprint(&self) -> String {
        self.config.map_fingerprint()
    }

    pub fn summary(&self) -> ClusterSummary {
        ClusterSummary::from(self)
    }
}

pub fn load(path: &Path, secret: Option<&Secret>) -> Result<ClusterConfig, ClusterError> {
    if !path.exists() {
        return Ok(ClusterConfig::default());
    }
    let text = fs::read_to_string(path).map_err(|error| ClusterError::Read {
        file: path.to_path_buf(),
        reason: error.to_string(),
    })?;
    let table: toml::Table = toml::from_str(&text).map_err(|error| ClusterError::Syntax {
        file: path.to_path_buf(),
        reason: error.to_string(),
    })?;
    let empty_file = table.is_empty();
    let config: ClusterConfig =
        toml::Value::Table(table)
            .try_into()
            .map_err(|error: toml::de::Error| {
                let message = error.to_string();
                ClusterError::InvalidKey {
                    file: path.to_path_buf(),
                    key: key_from_serde_error(&message),
                    reason: message,
                }
            })?;
    let problems = if empty_file {
        Vec::new()
    } else if config.empty() {
        vec![
            "instance_id: obrigatório quando existe cluster.toml (é o nome desta instância no              mapa)"
                .to_owned(),
        ]
    } else {
        config.problems()
    };
    if !problems.is_empty() {
        return Err(ClusterError::Validation {
            file: path.to_path_buf(),
            problems,
        });
    }
    if config.declares_cluster() && secret.is_none() {
        return Err(ClusterError::MissingSecret {
            file: path.to_path_buf(),
        });
    }
    Ok(config)
}

fn key_from_serde_error(message: &str) -> String {
    if let Some(rest) = message.split("unknown field `").nth(1)
        && let Some((key, _)) = rest.split_once('`')
    {
        return key.to_owned();
    }
    if let Some(rest) = message.split("missing field `").nth(1)
        && let Some((key, _)) = rest.split_once('`')
    {
        return key.to_owned();
    }
    "(arquivo)".to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClusterSummary {
    #[serde(rename = "modo")]
    pub mode: &'static str,
    pub instance_id: String,
    pub default_instance: Option<String>,
    #[serde(rename = "segredo_configurado")]
    pub secret_configured: bool,
    #[serde(rename = "instancias")]
    pub instances: Vec<InstanceSummary>,
    pub orgs: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstanceSummary {
    pub id: String,
    pub url: String,
    pub local: bool,
}

impl ClusterSummary {
    fn from(running: &LiveCluster) -> Self {
        let local = running.config.instance_id.trim();
        let mut instances: Vec<InstanceSummary> = running
            .config
            .instances
            .iter()
            .map(|ClusterInstance { id, url }| InstanceSummary {
                id: id.trim().to_owned(),
                url: url.trim().to_owned(),
                local: !local.is_empty() && id.trim() == local,
            })
            .collect();
        instances.sort_by(|a, b| a.id.cmp(&b.id));
        Self {
            mode: running.mode(),
            instance_id: local.to_owned(),
            default_instance: running
                .config
                .default_instance
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            secret_configured: running.secret.is_some(),
            instances,
            orgs: running
                .config
                .orgs
                .iter()
                .map(|(org, destination)| (org.trim().to_owned(), destination.trim().to_owned()))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrgMove {
    pub org: String,
    #[serde(rename = "de")]
    pub from: Option<String>,
    #[serde(rename = "para")]
    pub to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReloadReport {
    #[serde(rename = "mudou")]
    pub changed: bool,
    #[serde(rename = "fingerprint_antes")]
    pub previous_fingerprint: String,
    #[serde(rename = "fingerprint_depois")]
    pub new_fingerprint: String,
    #[serde(rename = "antes")]
    pub before: ClusterSummary,
    #[serde(rename = "depois")]
    pub after: ClusterSummary,
    #[serde(rename = "orgs_que_trocaram")]
    pub moved_orgs: Vec<OrgMove>,
    #[serde(rename = "instancias_adicionadas")]
    pub added_instances: Vec<String>,
    #[serde(rename = "instancias_removidas")]
    pub removed_instances: Vec<String>,
}

impl ReloadReport {
    fn between(before: &LiveCluster, after: &LiveCluster) -> Self {
        let previous_fingerprint = before.fingerprint();
        let new_fingerprint = after.fingerprint();
        let previous_ids: Vec<&str> = before
            .config
            .instances
            .iter()
            .map(|instance| instance.id.trim())
            .collect();
        let new_ids: Vec<&str> = after
            .config
            .instances
            .iter()
            .map(|instance| instance.id.trim())
            .collect();
        let added_instances = new_ids
            .iter()
            .filter(|id| !previous_ids.contains(id))
            .map(|id| (*id).to_owned())
            .collect();
        let removed_instances = previous_ids
            .iter()
            .filter(|id| !new_ids.contains(id))
            .map(|id| (*id).to_owned())
            .collect();
        let mut orgs: Vec<String> = before
            .config
            .orgs
            .keys()
            .chain(after.config.orgs.keys())
            .cloned()
            .collect();
        orgs.sort();
        orgs.dedup();
        let moved_orgs = orgs
            .into_iter()
            .filter_map(|org| {
                let from = before.config.org_instance(&org).map(str::to_owned);
                let to = after.config.org_instance(&org).map(str::to_owned);
                (from != to).then_some(OrgMove { org, from, to })
            })
            .collect();
        Self {
            changed: previous_fingerprint != new_fingerprint,
            previous_fingerprint,
            new_fingerprint,
            before: before.summary(),
            after: after.summary(),
            moved_orgs,
            added_instances,
            removed_instances,
        }
    }
}

pub fn reload(state: &ServerState) -> Result<ReloadReport, ClusterError> {
    let previous = state.cluster();
    let config = load(&previous.path, previous.secret.as_ref())?;
    let new = LiveCluster {
        path: previous.path.clone(),
        config,
        secret: previous.secret.clone(),
    };
    let report = ReloadReport::between(&previous, &new);
    state.replace_cluster(new);
    Ok(report)
}

pub const REMOTE_CHECK_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemoteDiagnostic {
    #[serde(rename = "instancia")]
    pub instance: String,
    pub url: String,
    #[serde(rename = "situacao")]
    pub status: &'static str,
    #[serde(rename = "mensagem")]
    pub message: Option<String>,
    #[serde(rename = "fingerprint_mapa")]
    pub map_fingerprint: Option<String>,
    #[serde(rename = "instancia_declarada")]
    pub declared_instance: Option<String>,
}

impl RemoteDiagnostic {
    pub fn ok(&self) -> bool {
        self.status == "ok"
    }

    pub fn line(&self) -> String {
        let detail = match self.status {
            "ok" => format!(
                "ok (mapa {})",
                self.map_fingerprint.as_deref().unwrap_or("não informado")
            ),
            "inacessivel" => format!(
                "inacessível — {}",
                self.message.as_deref().unwrap_or("sem resposta")
            ),
            "mapa_divergente" => format!(
                "mapa divergente — ela usa {}",
                self.map_fingerprint.as_deref().unwrap_or("(não informado)")
            ),
            _ => format!(
                "identidade divergente — declarou `{}`, o mapa diz `{}`",
                self.declared_instance.as_deref().unwrap_or("(nada)"),
                self.instance
            ),
        };
        format!("  {} ({}): {detail}", self.instance, self.url)
    }
}

pub async fn check_remote_instances(config: &ClusterConfig) -> Vec<RemoteDiagnostic> {
    let local = config.instance_id.trim().to_owned();
    let local_fingerprint = config.map_fingerprint();
    let client = match reqwest::Client::builder()
        .connect_timeout(REMOTE_CHECK_TIMEOUT)
        .timeout(REMOTE_CHECK_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return config
                .instances
                .iter()
                .filter(|instance| instance.id.trim() != local)
                .map(|instance| RemoteDiagnostic {
                    instance: instance.id.trim().to_owned(),
                    url: instance.url.trim().to_owned(),
                    status: "inacessivel",
                    message: Some(format!("não consegui montar o cliente HTTP: {error}")),
                    map_fingerprint: None,
                    declared_instance: None,
                })
                .collect();
        }
    };
    let requests = config
        .instances
        .iter()
        .filter(|instance| instance.id.trim() != local)
        .map(|instance| {
            let client = client.clone();
            let id = instance.id.trim().to_owned();
            let url = instance.url.trim().to_owned();
            let local_fingerprint = local_fingerprint.clone();
            async move {
                let address = format!("{}/healthz", url.trim_end_matches('/'));
                let response = client.get(&address).send().await;
                let body = match response {
                    Ok(response) if response.status().is_success() => {
                        response
                            .text()
                            .await
                            .ok()
                            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                    }
                    Ok(response) => {
                        return RemoteDiagnostic {
                            instance: id,
                            url,
                            status: "inacessivel",
                            message: Some(format!(
                                "o /healthz respondeu {}",
                                response.status().as_u16()
                            )),
                            map_fingerprint: None,
                            declared_instance: None,
                        };
                    }
                    Err(error) => {
                        return RemoteDiagnostic {
                            instance: id,
                            url,
                            status: "inacessivel",
                            message: Some(error.to_string()),
                            map_fingerprint: None,
                            declared_instance: None,
                        };
                    }
                };
                let remote_fingerprint = body
                    .as_ref()
                    .and_then(|body| body.get("cluster_map_fingerprint"))
                    .and_then(|value| value.as_str())
                    .map(str::to_owned);
                let declared = body
                    .as_ref()
                    .and_then(|body| body.get("instance_id"))
                    .and_then(|value| value.as_str())
                    .map(str::to_owned);
                let Some(remote_fingerprint) = remote_fingerprint else {
                    return RemoteDiagnostic {
                        instance: id,
                        url,
                        status: "inacessivel",
                        message: Some(
                            "a instância respondeu sem dizer o mapa (versão anterior a HR-50b-2?)"
                                .to_owned(),
                        ),
                        map_fingerprint: None,
                        declared_instance: declared,
                    };
                };
                let status = if remote_fingerprint != local_fingerprint {
                    "mapa_divergente"
                } else if declared.as_deref() != Some(id.as_str()) {
                    "identidade_divergente"
                } else {
                    "ok"
                };
                let message = match status {
                    "mapa_divergente" => Some(format!(
                        "o mapa desta instância é {local_fingerprint} e o dela é {remote_fingerprint}; atualize \
                         o cluster.toml dos dois lados e recarregue"
                    )),
                    "identidade_divergente" => Some(format!(
                        "a instância declarou o nome `{}`, mas o mapa desta instância a chama de \
                         `{id}`; corrija o `instance_id` dela",
                        declared.as_deref().unwrap_or("(nada)")
                    )),
                    _ => None,
                };
                RemoteDiagnostic {
                    instance: id,
                    url,
                    status,
                    message,
                    map_fingerprint: Some(remote_fingerprint),
                    declared_instance: declared,
                }
            }
        })
        .collect::<Vec<_>>();
    futures::future::join_all(requests).await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn folder() -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder =
            std::env::temp_dir().join(format!("poph-cluster-teste-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("pasta de teste");
        folder
    }

    fn file(text: &str) -> PathBuf {
        let path = folder().join("cluster.toml");
        fs::write(&path, text).expect("escreve cluster.toml");
        path
    }

    fn secret() -> Secret {
        Secret::new("segredo-compartilhado-de-teste".to_owned())
    }

    const VALID_CONFIG: &str = r#"
instance_id = "a"
default_instance = "b"

[[instances]]
id = "a"
url = "http://127.0.0.1:7480"

[[instances]]
id = "b"
url = "https://harness-b.exemplo"

[orgs]
org_01 = "a"
org_02 = "b"
"#;

    #[test]
    fn missing_file_uses_single_instance_mode() {
        let path = folder().join("nao-existe.toml");
        let config = load(&path, None).expect("sem arquivo não é erro");
        assert!(config.empty());
        assert!(!config.declares_cluster());
    }

    #[test]
    fn valid_file_loads_cluster_map() {
        let path = file(VALID_CONFIG);
        let config = load(&path, Some(&secret())).expect("carrega");
        assert_eq!(config.instance_id, "a");
        assert_eq!(config.default_instance.as_deref(), Some("b"));
        assert_eq!(config.instances.len(), 2);
        assert_eq!(config.org_instance("org_01"), Some("a"));
        assert_eq!(config.org_instance("org_02"), Some("b"));
        assert_eq!(config.org_instance("org_nova"), Some("b"));
    }

    #[test]
    fn invalid_toml_reports_file() {
        let path = file("[orgs\norg_01 = \"a\"\n");
        match load(&path, Some(&secret())) {
            Err(ClusterError::Syntax { file, reason }) => {
                assert_eq!(file, path);
                assert!(!reason.is_empty());
            }
            other => panic!("esperado erro de sintaxe, veio {other:?}"),
        }
    }

    #[test]
    fn unknown_field_reports_its_key() {
        let path = file("instance_id = \"a\"\ninstancias = []\n");
        match load(&path, Some(&secret())) {
            Err(ClusterError::InvalidKey { key, .. }) => assert_eq!(key, "instancias"),
            other => panic!("esperado erro de chave, veio {other:?}"),
        }
    }

    #[test]
    fn each_violation_reports_its_key() {
        let mut cases: Vec<(String, &str)> = vec![
            ("instance_id = \"\"\n".to_owned(), "instance_id"),
            (
                "instance_id = \"a\"\n[[instances]]\nid = \"a\"\nurl = \"ftp://x\"\n".to_owned(),
                "instances[1].url",
            ),
            (
                "instance_id = \"a\"\ndefault_instance = \"c\"\n[[instances]]\nid = \"a\"\nurl = \"http://x\"\n"
                    .to_owned(),
                "default_instance",
            ),
            (
                "instance_id = \"a\"\n[[instances]]\nid = \"a\"\nurl = \"http://x\"\n[orgs]\norg_01 = \"c\"\n"
                    .to_owned(),
                "orgs.org_01",
            ),
            (
                "instance_id = \"a\"\n[[instances]]\nid = \"b\"\nurl = \"http://y\"\n[[instances]]\nid = \"b\"\nurl = \"http://z\"\n"
                    .to_owned(),
                "aparece duas vezes",
            ),
        ];
        for (text, expected) in cases.drain(..) {
            let path = file(&text);
            match load(&path, Some(&secret())) {
                Err(ClusterError::Validation { problems, .. }) => {
                    let joined = problems.join("\n");
                    assert!(
                        joined.contains(expected),
                        "esperado `{expected}` nos problemas: {joined}"
                    );
                }
                other => panic!("esperado erro de validação para {expected}, veio {other:?}"),
            }
        }
    }

    #[test]
    fn cluster_requires_environment_secret() {
        let path = file(VALID_CONFIG);
        match load(&path, None) {
            Err(ClusterError::MissingSecret { .. }) => {}
            other => panic!("esperado erro de segredo ausente, veio {other:?}"),
        }
        assert!(load(&path, Some(&secret())).is_ok());
    }

    #[test]
    fn summary_omits_secret_value() {
        let path = file(VALID_CONFIG);
        let config = load(&path, Some(&secret())).expect("carrega");
        let running = LiveCluster {
            path: path.clone(),
            config,
            secret: Some(secret()),
        };
        let text = serde_json::to_string(&running.summary()).expect("json do resumo");
        assert!(text.contains("\"segredo_configurado\":true"));
        assert!(!text.contains("segredo-compartilhado-de-teste"));
        assert!(!format!("{running:?}").contains("segredo-compartilhado-de-teste"));
    }
}
