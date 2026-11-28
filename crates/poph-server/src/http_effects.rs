use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use poph_types::config::{HTTP_EFFECT_KINDS, HttpEffectsConfig};
use poph_types::{EffectId, ExecutionId, FlowId, OrgId, RecordId, Secret};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use reqwest::{Client, Url};
use serde_json::{Map, Value, json};

use crate::flow_effects::DeliveryFailure;

pub const CALENDAR_KIND: &str = "sincronizar_agenda";
pub const MAX_BODY_BYTES: usize = 16 * 1024;
const MAX_ERROR_BODY_BYTES: usize = 4 * 1024;
const IDEMPOTENCY_KEY: &str = "Idempotency-Key";
const NOT_CONFIGURED: &str = "credencial do efeito HTTP ausente no ambiente";

#[derive(Default)]
pub struct HttpEffectRegistry {
    endpoints: HashMap<OrgId, HashMap<String, Arc<HttpEffect>>>,
}

pub struct HttpEffect {
    client: Client,
    url: Url,
    timeout: Duration,
    authorization: Option<HeaderValue>,
}

pub struct CalendarContext<'a> {
    pub org: OrgId,
    pub ficha: RecordId,
    pub execution: ExecutionId,
    pub effect: EffectId,
    pub key: &'a str,
    pub flow: FlowId,
}

impl HttpEffectRegistry {
    pub fn from_environment(config: &HttpEffectsConfig) -> Result<Self, String> {
        Self::from_config(config, |name| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .map(Secret::new)
        })
    }

    pub fn from_config(
        config: &HttpEffectsConfig,
        mut secret: impl FnMut(&str) -> Option<Secret>,
    ) -> Result<Self, String> {
        if config.is_empty() {
            return Ok(Self::default());
        }
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .build()
            .map_err(|_| "não foi possível iniciar o cliente dos efeitos HTTP".to_owned())?;

        let mut endpoints = HashMap::new();
        for (&org, definitions) in config {
            let mut org_endpoints = HashMap::new();
            for (kind, definition) in definitions {
                if !HTTP_EFFECT_KINDS.contains(&kind.as_str()) {
                    return Err(format!("tipo de efeito sem entrega HTTP: {kind}"));
                }
                let url = Url::parse(&definition.url)
                    .ok()
                    .filter(|url| {
                        matches!(url.scheme(), "http" | "https")
                            && url.has_host()
                            && url.username().is_empty()
                            && url.password().is_none()
                            && url.query().is_none()
                            && url.fragment().is_none()
                    })
                    .ok_or_else(|| format!("endereço inválido para o efeito HTTP `{kind}`"))?;
                let authorization = match secret(&definition.authorization_env) {
                    Some(value) => {
                        let mut header = HeaderValue::from_str(value.expose()).map_err(|_| {
                            format!("credencial inválida para o efeito HTTP `{kind}`")
                        })?;
                        header.set_sensitive(true);
                        Some(header)
                    }
                    None => {
                        tracing::warn!(
                            target: "poph_server::http_effects",
                            kind = %kind,
                            variable = %definition.authorization_env,
                            "variável da credencial do efeito HTTP não definida: o efeito fica pendente até ela existir"
                        );
                        None
                    }
                };
                org_endpoints.insert(
                    kind.clone(),
                    Arc::new(HttpEffect {
                        client: client.clone(),
                        url,
                        timeout: Duration::from_millis(definition.timeout_ms),
                        authorization,
                    }),
                );
            }
            endpoints.insert(org, org_endpoints);
        }
        Ok(Self { endpoints })
    }

    pub fn get(&self, org: OrgId, kind: &str) -> Option<Arc<HttpEffect>> {
        self.endpoints.get(&org)?.get(kind).map(Arc::clone)
    }
}

impl HttpEffect {
    pub fn is_authorized(&self) -> bool {
        self.authorization.is_some()
    }

    pub(crate) async fn post(
        &self,
        idempotency_key: &str,
        body: Vec<u8>,
    ) -> Result<(), DeliveryFailure> {
        let Some(authorization) = self.authorization.as_ref() else {
            return Err(DeliveryFailure::Transient(NOT_CONFIGURED.to_owned()));
        };
        let response = self
            .client
            .post(self.url.clone())
            .header(AUTHORIZATION, authorization.clone())
            .header(IDEMPOTENCY_KEY, idempotency_key)
            .header(CONTENT_TYPE, "application/json")
            .timeout(self.timeout)
            .body(body)
            .send()
            .await
            .map_err(|error| transport_failure(&error))?;
        let status = response.status().as_u16();
        match status {
            200..=202 => Ok(()),
            408 | 425 | 429 | 500..=599 => {
                let code = error_code(response).await;
                Err(DeliveryFailure::Transient(describe(
                    "agenda: serviço indisponível",
                    status,
                    code.as_deref(),
                )))
            }
            400..=499 => {
                let code = error_code(response).await;
                Err(DeliveryFailure::Permanent(describe(
                    "agenda: serviço recusou o evento",
                    status,
                    code.as_deref(),
                )))
            }
            _ => Err(DeliveryFailure::Permanent(describe(
                "agenda: resposta inesperada do serviço",
                status,
                None,
            ))),
        }
    }
}

fn transport_failure(error: &reqwest::Error) -> DeliveryFailure {
    if error.is_timeout() {
        DeliveryFailure::Transient("agenda: tempo esgotado aguardando o serviço".to_owned())
    } else {
        DeliveryFailure::Transient("agenda: serviço indisponível (conexão)".to_owned())
    }
}

fn describe(prefix: &str, status: u16, code: Option<&str>) -> String {
    match code {
        Some(code) => format!("{prefix} (HTTP {status}, {code})"),
        None => format!("{prefix} (HTTP {status})"),
    }
}

async fn error_code(mut response: reqwest::Response) -> Option<String> {
    let mut bytes = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        if chunk.len() > MAX_ERROR_BODY_BYTES.saturating_sub(bytes.len()) {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    let body: Value = serde_json::from_slice(&bytes).ok()?;
    let code = body.pointer("/error/code")?.as_str()?;
    let valid = (1..=64).contains(&code.len())
        && code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_');
    valid.then(|| code.to_owned())
}

pub fn calendar_body(context: &CalendarContext<'_>, payload: &Value) -> Result<Vec<u8>, String> {
    let operation = payload
        .get("operacao")
        .and_then(Value::as_str)
        .ok_or("sincronizar_agenda sem `operacao` no efeito")?;
    if !matches!(operation, "create" | "update" | "remove") {
        return Err(format!("operação de agenda desconhecida: `{operation}`"));
    }
    let optional = |field: &str| {
        payload
            .get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let mut body = Map::new();
    body.insert(
        "context".to_owned(),
        json!({
            "org_id": context.org.to_string(),
            "ficha_id": context.ficha.to_string(),
            "execution_id": context.execution.to_string(),
            "effect_id": context.effect.to_string(),
            "effect_key": context.key,
            "flow_id": context.flow.to_string(),
        }),
    );
    body.insert("operation".to_owned(), json!(operation));
    if operation != "remove" {
        let start = optional("inicio").ok_or("sincronizar_agenda sem `inicio` no efeito")?;
        let mut event = Map::new();
        event.insert("start".to_owned(), json!(start));
        for (member, field) in [
            ("end", "fim"),
            ("title", "titulo"),
            ("location", "local"),
            ("executor_id", "executor"),
        ] {
            if let Some(value) = optional(field) {
                event.insert(member.to_owned(), json!(value));
            }
        }
        body.insert("event".to_owned(), Value::Object(event));
    }
    let bytes = serde_json::to_vec(&Value::Object(body))
        .map_err(|_| "não consegui montar o corpo da agenda".to_owned())?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err("corpo do efeito de agenda acima de 16 KiB".to_owned());
    }
    Ok(bytes)
}
