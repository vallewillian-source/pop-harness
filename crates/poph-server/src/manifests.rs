use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use poph_plugins::{EventCollector, PluginOptions, Supervisor};
use poph_types::{DeclaredPlugin, IntegrationId, Manifest, OrgId, PluginsConfig};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Mutex;

pub const FAILURE_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestFailure {
    Undeclared,
    Unavailable { reason: String },
}

impl ManifestFailure {
    pub fn message(&self) -> String {
        match self {
            Self::Undeclared => "esta instalação não declara manifesto para este tipo de \
                                   integração; declare o plugin em `[plugins].integracoes` da \
                                   configuração"
                .to_owned(),
            Self::Unavailable { .. } => {
                "o plugin deste tipo não respondeu ao handshake; verifique \
                                          o programa declarado em `[plugins].integracoes`"
                    .to_owned()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ManifestStatus {
    Ok,
    #[serde(rename = "indisponivel")]
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManifestSummary {
    pub kind: String,
    #[serde(rename = "situacao")]
    pub status: ManifestStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "nome")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "versao")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "mensagem")]
    pub message: Option<String>,
}

struct CacheEntry {
    result: Result<Manifest, String>,
    when: Instant,
}

pub struct ManifestCatalog {
    declared_plugins: BTreeMap<String, DeclaredPlugin>,
    cache: Mutex<HashMap<String, CacheEntry>>,
}

impl std::fmt::Debug for ManifestCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogoManifestos")
            .field(
                "declarados",
                &self.declared_plugins.keys().collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl ManifestCatalog {
    pub fn new(plugins: &PluginsConfig) -> Self {
        let declared_plugins = plugins
            .integrations
            .iter()
            .map(|plugin| (plugin.kind.trim().to_owned(), plugin.clone()))
            .collect();
        Self {
            declared_plugins,
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn declared_plugins(&self) -> Vec<&str> {
        self.declared_plugins.keys().map(String::as_str).collect()
    }

    pub async fn list(&self) -> Vec<ManifestSummary> {
        let mut items = Vec::with_capacity(self.declared_plugins.len());
        for kind in self.declared_plugins.keys() {
            let item = match self.manifest(kind).await {
                Ok(manifest) => ManifestSummary {
                    kind: kind.clone(),
                    status: ManifestStatus::Ok,
                    name: Some(manifest.name),
                    version: Some(manifest.version),
                    message: None,
                },
                Err(failure) => ManifestSummary {
                    kind: kind.clone(),
                    status: ManifestStatus::Unavailable,
                    name: None,
                    version: None,
                    message: Some(failure.message()),
                },
            };
            items.push(item);
        }
        items
    }

    pub async fn manifest(&self, kind: &str) -> Result<Manifest, ManifestFailure> {
        let declared = self
            .declared_plugins
            .get(kind)
            .cloned()
            .ok_or(ManifestFailure::Undeclared)?;
        let mut cache = self.cache.lock().await;
        if let Some(entry) = cache.get(kind) {
            let expired = entry.result.is_err() && entry.when.elapsed() >= FAILURE_TTL;
            if !expired {
                return entry
                    .result
                    .clone()
                    .map_err(|reason| ManifestFailure::Unavailable { reason });
            }
        }
        let result = resolve(&declared).await;
        let cached = result.clone().map_err(|failure| match failure {
            ManifestFailure::Unavailable { reason } => reason,
            ManifestFailure::Undeclared => "não declarado".to_owned(),
        });
        cache.insert(
            kind.to_owned(),
            CacheEntry {
                result: cached,
                when: Instant::now(),
            },
        );
        result
    }
}

async fn resolve(declared: &DeclaredPlugin) -> Result<Manifest, ManifestFailure> {
    let mut options = PluginOptions::new(
        declared.program.clone(),
        IntegrationId::generate(),
        OrgId::generate(),
    )
    .with_kind(declared.kind.trim());
    options.arguments = declared.arguments.clone();
    match Supervisor::start(options, Arc::new(EventCollector::new())).await {
        Ok(supervisor) => {
            let manifest = supervisor.manifest();
            supervisor.shutdown().await;
            match manifest {
                Some(manifest) => Ok(manifest),
                None => {
                    tracing::warn!(
                        kind = %declared.kind,
                        "o plugin declarou handshake sem manifesto"
                    );
                    Err(ManifestFailure::Unavailable {
                        reason: "handshake sem manifesto".to_owned(),
                    })
                }
            }
        }
        Err(error) => {
            tracing::warn!(
                kind = %declared.kind,
                "programa" = %declared.program.display(),
                "erro" = %error,
                "não consegui ler o manifesto do plugin"
            );
            Err(ManifestFailure::Unavailable {
                reason: error.to_string(),
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidField {
    pub field: String,
    pub reason: String,
}

pub fn validate_object(value: &Value, schema: &Value, prefix: &str) -> Result<(), InvalidField> {
    let object = value.as_object().ok_or_else(|| InvalidField {
        field: field(prefix, ""),
        reason: "precisa ser um objeto".to_owned(),
    })?;
    let empty = serde_json::Map::new();
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for name in required.iter().filter_map(Value::as_str) {
            let present = object
                .get(name)
                .is_some_and(|v| !v.is_null() && v.as_str() != Some(""));
            if !present {
                return Err(InvalidField {
                    field: field(prefix, name),
                    reason: "obrigatório".to_owned(),
                });
            }
        }
    }
    let allows_additional_fields = match schema.get("additionalProperties") {
        Some(Value::Bool(allowed)) => *allowed,
        Some(Value::Object(_)) => true,
        _ => false,
    };
    for (name, field_value) in object {
        let Some(field_schema) = properties.get(name) else {
            if allows_additional_fields {
                continue;
            }
            return Err(InvalidField {
                field: field(prefix, name),
                reason: "não está no esquema do plugin".to_owned(),
            });
        };
        validate_field(name, field_value, field_schema, prefix)?;
    }
    Ok(())
}

fn validate_field(
    name: &str,
    value: &Value,
    schema: &Value,
    prefix: &str,
) -> Result<(), InvalidField> {
    let error = |reason: String| InvalidField {
        field: field(prefix, name),
        reason,
    };
    if let Some(kind) = schema.get("type").and_then(Value::as_str) {
        let valid = match kind {
            "string" => value.is_string(),
            "number" => value.is_number(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "boolean" => value.is_boolean(),
            "array" => value.is_array(),
            "object" => value.is_object(),
            "null" => value.is_null(),
            other => {
                return Err(error(format!(
                    "o esquema do plugin usa o tipo `{other}`, que a API não valida"
                )));
            }
        };
        if !valid {
            return Err(error(format!("precisa ser do tipo `{kind}`")));
        }
    }
    if let Some(allowed_values) = schema.get("enum").and_then(Value::as_array)
        && !allowed_values.iter().any(|allowed| allowed == value)
    {
        let list: Vec<String> = allowed_values
            .iter()
            .map(|allowed| allowed.to_string())
            .collect();
        return Err(error(format!(
            "valor fora da lista aceita: {}",
            list.join(", ")
        )));
    }
    if let (Some(text), Some(minimum)) = (
        value.as_str(),
        schema.get("minLength").and_then(Value::as_u64),
    ) && (text.chars().count() as u64) < minimum
    {
        return Err(error(format!(
            "precisa de pelo menos {minimum} caractere(s)"
        )));
    }
    if let (Some(text), Some(maximum)) = (
        value.as_str(),
        schema.get("maxLength").and_then(Value::as_u64),
    ) && (text.chars().count() as u64) > maximum
    {
        return Err(error(format!("aceita no máximo {maximum} caractere(s)")));
    }
    if let Some(number) = value.as_f64() {
        if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64)
            && number < minimum
        {
            return Err(error(format!("precisa ser maior ou igual a {minimum}")));
        }
        if let Some(maximum) = schema.get("maximum").and_then(Value::as_f64)
            && number > maximum
        {
            return Err(error(format!("precisa ser menor ou igual a {maximum}")));
        }
    }
    if value.is_object() && schema.get("properties").is_some() {
        validate_object(value, schema, &field(prefix, name))?;
    }
    Ok(())
}

fn field(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else if name.is_empty() {
        prefix.to_owned()
    } else {
        format!("{prefix}.{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "modo": { "type": "string", "enum": ["inverter"] },
                "remetente": { "type": "string", "minLength": 3 },
                "tentativas": { "type": "integer", "minimum": 1, "maximum": 5 },
                "avisar": { "type": "boolean" },
                "rotulos": { "type": "array" }
            },
            "required": ["remetente"]
        })
    }

    fn error(value: Value) -> InvalidField {
        validate_object(&value, &schema(), "config_json").expect_err("deveria recusar")
    }

    #[test]
    fn accepts_object_matching_schema() {
        validate_object(
            &json!({"modo": "inverter", "remetente": "Loja", "tentativas": 2, "avisar": false, "rotulos": []}),
            &schema(),
            "config_json",
        )
        .expect("válido");
    }

    #[test]
    fn rejects_missing_or_empty_required_field() {
        assert_eq!(error(json!({})).field, "config_json.remetente");
        assert_eq!(error(json!({})).reason, "obrigatório");
        assert_eq!(
            error(json!({"remetente": "  "})).field,
            "config_json.remetente"
        );
    }

    #[test]
    fn rejects_incorrect_field_type() {
        let failure = error(json!({"remetente": "Loja", "tentativas": "duas"}));
        assert_eq!(failure.field, "config_json.tentativas");
        assert!(failure.reason.contains("integer"), "{}", failure.reason);
    }

    #[test]
    fn rejects_value_outside_enum() {
        let failure = error(json!({"remetente": "Loja", "modo": "outro"}));
        assert_eq!(failure.field, "config_json.modo");
        assert!(failure.reason.contains("inverter"), "{}", failure.reason);
    }

    #[test]
    fn checks_numeric_and_length_bounds() {
        assert_eq!(
            error(json!({"remetente": "Loja", "tentativas": 9})).field,
            "config_json.tentativas"
        );
        assert_eq!(
            error(json!({"remetente": "Lo"})).field,
            "config_json.remetente"
        );
    }

    #[test]
    fn rejects_unknown_field_unless_allowed() {
        assert_eq!(
            error(json!({"remetente": "Loja", "inventado": 1})).field,
            "config_json.inventado"
        );
        let mut allowed = schema();
        allowed["additionalProperties"] = json!(true);
        validate_object(
            &json!({"remetente": "Loja", "extra": 1}),
            &allowed,
            "config_json",
        )
        .expect("additionalProperties libera");
    }

    #[test]
    fn nested_field_error_reports_path() {
        let schema = json!({
            "type": "object",
            "properties": {
                "canal": {
                    "type": "object",
                    "properties": { "nome": { "type": "string" } },
                    "required": ["nome"]
                }
            }
        });
        let failure = validate_object(&json!({"canal": {}}), &schema, "config_json")
            .expect_err("faltou nome");
        assert_eq!(failure.field, "config_json.canal.nome");
    }

    #[test]
    fn rejects_non_object_at_prefix() {
        let failure = validate_object(&json!(["a"]), &schema(), "secrets").expect_err("lista");
        assert_eq!(failure.field, "secrets");
    }
}
