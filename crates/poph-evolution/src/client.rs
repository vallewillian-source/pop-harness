use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use poph_types::{BaseMetrics, Secret};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};

use crate::error::EvolutionError;
use crate::text::{TEXT_LIMIT, to_whatsapp};

const RETRY_DELAY: Duration = Duration::from_millis(500);

pub const ATTACHMENT_CAP: u64 = 16 * 1024 * 1024;

pub const DEFAULT_BODY_CAP: u64 = 1024 * 1024;

pub fn attachment_body_cap(cap: u64) -> u64 {
    cap / 3 * 4 + DEFAULT_BODY_CAP
}

const PROVIDER_MESSAGE_SIZE: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetryPolicy {
    Allowed,
    Never,
}

#[derive(Clone, Debug)]
pub struct ConfigEvolution {
    pub url_base: String,
    pub instancia: String,
    pub timeout: Duration,
    pub retentativas: u32,
    pub origin: Option<String>,
    pub metrics: Option<BaseMetrics>,
    key: Secret,
}

impl ConfigEvolution {
    pub fn new(
        url_base: impl Into<String>,
        instancia: impl Into<String>,
        key: Secret,
    ) -> Result<Self, EvolutionError> {
        let url_base = url_base.into().trim().trim_end_matches('/').to_owned();
        if !(url_base.starts_with("http://") || url_base.starts_with("https://")) {
            return Err(EvolutionError::InvalidConfiguration(
                "o endereço da Evolution deve começar com http:// ou https://".to_owned(),
            ));
        }
        let instancia = instancia.into().trim().to_owned();
        if instancia.is_empty() {
            return Err(EvolutionError::InvalidConfiguration(
                "a integração do WhatsApp precisa do nome da instância".to_owned(),
            ));
        }
        if key.expose().trim().is_empty() {
            return Err(EvolutionError::InvalidConfiguration(
                "a integração do WhatsApp precisa da chave da Evolution (`apikey`)".to_owned(),
            ));
        }
        Ok(Self {
            url_base,
            instancia,
            timeout: Duration::from_secs(20),
            retentativas: 1,
            origin: None,
            metrics: None,
            key,
        })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_retries(mut self, retentativas: u32) -> Self {
        self.retentativas = retentativas.min(1);
        self
    }

    pub fn with_metrics(mut self, metrics: BaseMetrics) -> Self {
        self.metrics = Some(metrics);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedAttachment {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub name: Option<String>,
}

struct Inner {
    http: reqwest::Client,
    config: ConfigEvolution,
}

#[derive(Clone)]
pub struct EvolutionClient(Arc<Inner>);

impl EvolutionClient {
    pub fn new(config: ConfigEvolution) -> Result<Self, EvolutionError> {
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|error| {
                EvolutionError::InvalidConfiguration(format!("cliente HTTP: {error}"))
            })?;
        Ok(Self(Arc::new(Inner { http, config })))
    }

    pub fn instancia(&self) -> &str {
        &self.0.config.instancia
    }

    pub async fn create_instance(&self) -> Result<Value, EvolutionError> {
        self.request(
            reqwest::Method::POST,
            "/instance/create",
            Some(json!({
                "instanceName": self.0.config.instancia,
                "integration": "WHATSAPP-BAILEYS",
                "qrcode": true,
            })),
            RetryPolicy::Never,
            DEFAULT_BODY_CAP,
        )
        .await
    }

    pub async fn connect(&self, numero: Option<&str>) -> Result<Pairing, EvolutionError> {
        let mut path = format!("/instance/connect/{}", self.0.config.instancia);
        let digits: String = numero
            .unwrap_or_default()
            .chars()
            .filter(char::is_ascii_digit)
            .collect();
        if !digits.is_empty() {
            path.push_str(&format!("?number={digits}"));
        }
        let body = self
            .request(
                reqwest::Method::GET,
                &path,
                None,
                RetryPolicy::Allowed,
                DEFAULT_BODY_CAP,
            )
            .await?;
        Ok(extract_pairing(&body))
    }

    pub async fn connection_state(&self) -> Result<String, EvolutionError> {
        let path = format!("/instance/connectionState/{}", self.0.config.instancia);
        let body = self
            .request(
                reqwest::Method::GET,
                &path,
                None,
                RetryPolicy::Allowed,
                DEFAULT_BODY_CAP,
            )
            .await?;
        let state = body
            .get("instance")
            .and_then(|instancia| instancia.get("state"))
            .or_else(|| body.get("state"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if state.is_empty() {
            return Err(EvolutionError::InvalidResponse(
                "a resposta do estado da instância não traz `state`".to_owned(),
            ));
        }
        Ok(state)
    }

    pub async fn list_instances(&self) -> Result<Value, EvolutionError> {
        self.request(
            reqwest::Method::GET,
            "/instance/fetchInstances",
            None,
            RetryPolicy::Allowed,
            DEFAULT_BODY_CAP,
        )
        .await
    }

    pub async fn desconectar(&self) -> Result<Value, EvolutionError> {
        let path = format!("/instance/logout/{}", self.0.config.instancia);
        self.request(
            reqwest::Method::DELETE,
            &path,
            None,
            RetryPolicy::Never,
            DEFAULT_BODY_CAP,
        )
        .await
    }

    pub async fn delete(&self) -> Result<Value, EvolutionError> {
        let path = format!("/instance/delete/{}", self.0.config.instancia);
        self.request(
            reqwest::Method::DELETE,
            &path,
            None,
            RetryPolicy::Never,
            DEFAULT_BODY_CAP,
        )
        .await
    }

    pub async fn set_webhook(
        &self,
        url: &str,
        events: &[&str],
        headers: &[(String, String)],
    ) -> Result<Value, EvolutionError> {
        if url.trim().is_empty() {
            return Err(EvolutionError::InvalidConfiguration(
                "o endereço do webhook está vazio".to_owned(),
            ));
        }
        let events: Vec<Value> = events
            .iter()
            .map(|event| Value::String((*event).to_owned()))
            .collect();
        let headers: serde_json::Map<String, Value> = headers
            .iter()
            .map(|(name, value)| (name.clone(), Value::String(value.clone())))
            .collect();
        let path = format!("/webhook/set/{}", self.0.config.instancia);
        self.request(
            reqwest::Method::POST,
            &path,
            Some(json!({
                "webhook": {
                    "enabled": true,
                    "url": url.trim(),
                    "byEvents": false,
                    "base64": false,
                    "headers": Value::Object(headers),
                    "events": events,
                }
            })),
            RetryPolicy::Allowed,
            DEFAULT_BODY_CAP,
        )
        .await
    }

    pub async fn send_text(
        &self,
        numero: &str,
        text: &str,
    ) -> Result<Option<String>, EvolutionError> {
        let numero = numero.trim();
        if numero.is_empty() {
            return Err(EvolutionError::InvalidConfiguration(
                "envio sem número de destino".to_owned(),
            ));
        }
        let body = to_whatsapp(text);
        if body.trim().is_empty() {
            return Err(EvolutionError::InvalidConfiguration(
                "envio de texto vazio".to_owned(),
            ));
        }
        let path = format!("/message/sendText/{}", self.0.config.instancia);
        let response = self
            .request(
                reqwest::Method::POST,
                &path,
                Some(json!({"number": numero, "text": body})),
                RetryPolicy::Never,
                DEFAULT_BODY_CAP,
            )
            .await?;
        Ok(sent_message_id(&response))
    }

    pub async fn send_attachment(
        &self,
        numero: &str,
        bytes: &[u8],
        mime: &str,
        name: &str,
        caption: Option<&str>,
    ) -> Result<Option<String>, EvolutionError> {
        let numero = numero.trim();
        if numero.is_empty() {
            return Err(EvolutionError::InvalidConfiguration(
                "envio sem número de destino".to_owned(),
            ));
        }
        let size = bytes.len() as u64;
        if size > ATTACHMENT_CAP {
            return Err(EvolutionError::AttachmentTooLarge {
                bytes: size,
                cap: ATTACHMENT_CAP,
            });
        }
        if size == 0 {
            return Err(EvolutionError::InvalidConfiguration(
                "envio de arquivo vazio".to_owned(),
            ));
        }
        let attachment_kind = if mime.trim().to_ascii_lowercase().starts_with("image/") {
            "image"
        } else {
            "document"
        };
        let mut body = json!({
            "number": numero,
            "mediatype": attachment_kind,
            "mimetype": mime,
            "fileName": name,
            "media": base64::engine::general_purpose::STANDARD.encode(bytes),
        });
        if let Some(caption) = caption.filter(|caption| !caption.trim().is_empty()) {
            body["caption"] = Value::String(truncate_caption(caption));
        }
        let path = format!("/message/sendMedia/{}", self.0.config.instancia);
        let response = self
            .request(
                reqwest::Method::POST,
                &path,
                Some(body),
                RetryPolicy::Never,
                DEFAULT_BODY_CAP,
            )
            .await?;
        Ok(sent_message_id(&response))
    }

    pub async fn send_presence(&self, numero: &str, typing: bool) -> Result<(), EvolutionError> {
        let numero = numero.trim();
        if numero.is_empty() {
            return Ok(());
        }
        let path = format!("/chat/sendPresence/{}", self.0.config.instancia);
        let presence = if typing { "composing" } else { "paused" };
        self.request(
            reqwest::Method::POST,
            &path,
            Some(json!({"number": numero, "presence": presence})),
            RetryPolicy::Never,
            DEFAULT_BODY_CAP,
        )
        .await
        .map(|_| ())
    }

    pub async fn download_attachment(
        &self,
        message_id: &str,
        data: &Value,
        cap: u64,
    ) -> Result<DownloadedAttachment, EvolutionError> {
        let id_in_key = data
            .get("key")
            .and_then(|key| key.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if id_in_key != message_id {
            return Err(EvolutionError::InvalidResponse(
                "o id pedido não é o da mensagem enviada no `data`".to_owned(),
            ));
        }
        let path = format!(
            "/chat/getBase64FromMediaMessage/{}",
            self.0.config.instancia
        );
        let response = match self
            .request(
                reqwest::Method::POST,
                &path,
                Some(json!({"message": data})),
                RetryPolicy::Never,
                attachment_body_cap(cap),
            )
            .await
        {
            Ok(response) => response,
            Err(EvolutionError::Response { status, message })
                if is_expired_attachment(status, &message) =>
            {
                return Err(EvolutionError::AttachmentExpired(message));
            }
            Err(error) => return Err(error),
        };

        let body = response.get("data").unwrap_or(&response);
        let Some(encoded) = body.get("base64").and_then(Value::as_str) else {
            return Err(EvolutionError::InvalidResponse(
                "a resposta não traz a mídia em `base64`".to_owned(),
            ));
        };
        let without_spaces: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
        let estimated = (without_spaces.len() as u64).saturating_mul(3) / 4;
        if estimated > cap {
            return Err(EvolutionError::AttachmentTooLarge {
                bytes: estimated,
                cap,
            });
        }
        let image_body = without_spaces
            .split_once(',')
            .filter(|(prefix, _)| prefix.starts_with("data:"))
            .map(|(_, rest)| rest.to_owned())
            .unwrap_or(without_spaces);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(image_body.as_bytes())
            .map_err(|error| {
                EvolutionError::InvalidResponse(format!("base64 da mídia inválido: {error}"))
            })?;
        if bytes.len() as u64 > cap {
            return Err(EvolutionError::AttachmentTooLarge {
                bytes: bytes.len() as u64,
                cap,
            });
        }
        Ok(DownloadedAttachment {
            bytes,
            mime: body
                .get("mimetype")
                .and_then(Value::as_str)
                .unwrap_or("application/octet-stream")
                .to_owned(),
            name: body
                .get("fileName")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        retry: RetryPolicy,
        body_cap: u64,
    ) -> Result<Value, EvolutionError> {
        let url = format!("{}{}", self.0.config.url_base, path);
        let retries = match retry {
            RetryPolicy::Allowed => self.0.config.retentativas,
            RetryPolicy::Never => 0,
        };
        let mut last: Option<EvolutionError> = None;
        for attempt_number in 0..=retries {
            if attempt_number > 0 {
                tokio::time::sleep(RETRY_DELAY).await;
            }
            match self.attempt(&method, &url, body.clone(), body_cap).await {
                Ok(value) => return Ok(value),
                Err(error) => {
                    let repeat = error.is_transient();
                    last = Some(error);
                    if !repeat {
                        break;
                    }
                }
            }
        }
        Err(last.unwrap_or_else(|| {
            EvolutionError::Network("a chamada não chegou a ser feita".to_owned())
        }))
    }

    async fn attempt(
        &self,
        method: &reqwest::Method,
        url: &str,
        body: Option<Value>,
        body_cap: u64,
    ) -> Result<Value, EvolutionError> {
        let metrics = self.0.config.metrics.clone();
        if let Some(m) = &metrics {
            m.evolution_calls.increment(1);
        }
        let resultado = self.attempt_uncounted(method, url, body, body_cap).await;
        if let (Err(_), Some(m)) = (&resultado, &metrics) {
            m.evolution_errors.increment(1);
        }
        resultado
    }

    async fn attempt_uncounted(
        &self,
        method: &reqwest::Method,
        url: &str,
        body: Option<Value>,
        body_cap: u64,
    ) -> Result<Value, EvolutionError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "apikey",
            HeaderValue::from_str(self.0.config.key.expose().trim()).map_err(|_| {
                EvolutionError::InvalidConfiguration(
                    "a chave da Evolution tem caractere que não cabe em cabeçalho HTTP".to_owned(),
                )
            })?,
        );
        if let Some(origin) = self.0.config.origin.as_deref()
            && let Ok(value) = HeaderValue::from_str(origin)
        {
            headers.insert("Origin", value);
        }
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let mut builder = self.0.http.request(method.clone(), url).headers(headers);
        if let Some(body) = body {
            builder = builder.json(&body);
        }
        let response = match builder.send().await {
            Ok(response) => response,
            Err(error) => {
                let detail = if error.is_timeout() {
                    "tempo esgotado".to_owned()
                } else if error.is_connect() {
                    "não consegui conectar".to_owned()
                } else {
                    error.without_url().to_string()
                };
                return Err(EvolutionError::Network(detail));
            }
        };

        let status = response.status();
        let mut response = response;
        let Some(raw) = read_body(&mut response, body_cap).await? else {
            return Err(if status.is_success() {
                EvolutionError::ResponseTooLarge { cap: body_cap }
            } else {
                EvolutionError::Response {
                    status: status.as_u16(),
                    message: "a resposta de erro passou do tamanho aceito".to_owned(),
                }
            });
        };
        let text = String::from_utf8_lossy(&raw).into_owned();
        let body: Value = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or(Value::Null)
        };

        if !status.is_success() {
            return Err(EvolutionError::Response {
                status: status.as_u16(),
                message: provider_message(&body, self.0.config.key.expose()),
            });
        }
        if body.is_null() {
            return Ok(json!({}));
        }
        Ok(body)
    }
}

async fn read_body(
    response: &mut reqwest::Response,
    cap: u64,
) -> Result<Option<Vec<u8>>, EvolutionError> {
    if response.content_length().is_some_and(|size| size > cap) {
        return Ok(None);
    }
    let mut raw = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| EvolutionError::Network(error.without_url().to_string()))?
    {
        if raw.len() as u64 + chunk.len() as u64 > cap {
            return Ok(None);
        }
        raw.extend_from_slice(&chunk);
    }
    Ok(Some(raw))
}

fn provider_message(body: &Value, key: &str) -> String {
    let mut raw: Option<String> = None;
    for campo in ["message", "error", "detail"] {
        if let Some(value) = body.get(campo) {
            if let Some(text) = value.as_str()
                && !text.trim().is_empty()
            {
                raw = Some(text.to_owned());
                break;
            }
            if let Some(list) = value.as_array() {
                let joined: Vec<&str> = list.iter().filter_map(Value::as_str).collect();
                if !joined.is_empty() {
                    raw = Some(joined.join("; "));
                    break;
                }
            }
        }
    }
    let Some(raw) = raw else {
        return "sem detalhe".to_owned();
    };
    let clean = if key.is_empty() {
        raw
    } else {
        raw.replace(key, "[chave ocultada]")
    };
    let masked: Vec<String> = clean
        .split_whitespace()
        .map(|word| {
            let digits = word.chars().filter(char::is_ascii_digit).count();
            if word.contains('@') || digits >= 7 {
                "[identificador ocultado]".to_owned()
            } else {
                word.to_owned()
            }
        })
        .collect();
    let joined = masked.join(" ");
    if joined.chars().count() <= PROVIDER_MESSAGE_SIZE {
        joined
    } else {
        joined
            .chars()
            .take(PROVIDER_MESSAGE_SIZE - 1)
            .collect::<String>()
            + "…"
    }
}

fn is_expired_attachment(status: u16, message: &str) -> bool {
    if status != 400 && status != 404 {
        return false;
    }
    let lowercase = message.to_ascii_lowercase();
    lowercase.contains("message not found")
        || lowercase.contains("media not found")
        || lowercase.contains("media not available")
        || lowercase.contains("expirad")
        || lowercase.contains("expired")
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pairing {
    pub qr_base64: Option<String>,
    pub qr_text: Option<String>,
    pub pairing_code: Option<String>,
    pub counter: Option<u64>,
}

fn extract_pairing(body: &Value) -> Pairing {
    let text = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let counter = body
        .get("count")
        .or_else(|| body.get("qrcode").and_then(|value| value.get("count")))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
        });
    Pairing {
        qr_base64: text(body.get("qrcode").and_then(|value| value.get("base64")))
            .or_else(|| text(body.get("base64"))),
        qr_text: text(body.get("code"))
            .or_else(|| text(body.get("qrcode").and_then(|value| value.get("code")))),
        pairing_code: text(body.get("pairingCode")),
        counter,
    }
}

fn sent_message_id(body: &Value) -> Option<String> {
    [
        body.get("key").and_then(|key| key.get("id")),
        body.get("message")
            .and_then(|value| value.get("key"))
            .and_then(|key| key.get("id")),
        body.get("id"),
        body.get("data")
            .and_then(|value| value.get("key"))
            .and_then(|key| key.get("id")),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .map(str::to_owned)
    .find(|value| !value.trim().is_empty())
}

fn truncate_caption(caption: &str) -> String {
    let converted = to_whatsapp(caption);
    let cap = TEXT_LIMIT / 4;
    if converted.chars().count() <= cap {
        converted
    } else {
        converted.chars().take(cap - 1).collect::<String>() + "…"
    }
}
