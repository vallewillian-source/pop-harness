use base64::Engine as _;
use poph_types::{
    InboundAttachment, InboundEvent, IntegrationId, Secret, StateEvent, UnsupportedContent,
};
use serde_json::Value;
use thiserror::Error;

pub const MESSAGE_EVENT: &str = "messages.upsert";

pub const SECRET_HEADER: &str = "X-Evolution-Webhook-Secret";

pub const ATTACHMENT_DAYS: i64 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WebhookError {
    #[error("webhook recusado: a integração não tem segredo configurado")]
    NoSecretConfigured,
    #[error("webhook recusado: falta o cabeçalho `{SECRET_HEADER}`")]
    SecretMissing,
    #[error("webhook recusado: segredo inválido")]
    SecretMismatch,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Policy {
    pub accept_from_me: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentToDownload {
    pub name: String,
    pub mime: String,
    pub message_id: String,
    pub declared_size: Option<u64>,
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReceivedEntry {
    pub event: InboundEvent,
    pub attachments: Vec<AttachmentToDownload>,
}

impl ReceivedEntry {
    pub fn with_attachments(mut self, attachments: Vec<InboundAttachment>) -> InboundEvent {
        self.event.attachments = attachments;
        self.event
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Inbound(ReceivedEntry),
    State(StateEvent),
    Ignored(IgnoreReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IgnoreReason {
    UnknownEvent(String),
    MissingId,
    MissingSender,
    FromMe,
    NoContent,
    AttachmentNotSupported(String),
    MimeNotSupported(String),
    MissingTimestamp,
}

pub fn check_secret(expected: &Secret, received: Option<&str>) -> Result<(), WebhookError> {
    if expected.expose().trim().is_empty() {
        return Err(WebhookError::NoSecretConfigured);
    }
    let Some(received) = received else {
        return Err(WebhookError::SecretMissing);
    };
    if constant_time_equal(expected.expose().as_bytes(), received.trim().as_bytes()) {
        Ok(())
    } else {
        Err(WebhookError::SecretMismatch)
    }
}

fn constant_time_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        difference |= x ^ y;
    }
    difference == 0
}

pub fn interpret(integration: IntegrationId, body: &Value, policy: &Policy) -> Decision {
    let event = body
        .get("event")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned();

    if is_state_event(&event) {
        return Decision::State(channel_state(body, &event));
    }
    if !event.eq_ignore_ascii_case(MESSAGE_EVENT) {
        return Decision::Ignored(IgnoreReason::UnknownEvent(event));
    }

    let data = body.get("data").cloned().unwrap_or(Value::Null);
    let key = data.get("key").cloned().unwrap_or(Value::Null);

    let Some(id) = text_in(&key, "id") else {
        return Decision::Ignored(IgnoreReason::MissingId);
    };
    let Some(jid) = text_in(&key, "remoteJid") else {
        return Decision::Ignored(IgnoreReason::MissingSender);
    };
    let from_me = key.get("fromMe").and_then(Value::as_bool).unwrap_or(false);
    if from_me && !policy.accept_from_me {
        return Decision::Ignored(IgnoreReason::FromMe);
    }
    let Some(ocorrido_em) = timestamp_ms(&data) else {
        return Decision::Ignored(IgnoreReason::MissingTimestamp);
    };

    let content = unwrap_message(data.get("message").cloned().unwrap_or(Value::Null));
    let group = jid.ends_with("@g.us");
    let peer = if group { jid.clone() } else { jid_number(&jid) };
    if peer.is_empty() {
        return Decision::Ignored(IgnoreReason::MissingSender);
    }
    let remetente = if group {
        text_in(&key, "participant")
    } else {
        None
    };

    let mut text = extract_text(&content);
    let mut attachments: Vec<AttachmentToDownload> = Vec::new();

    if let Some(image) = content.get("imageMessage") {
        let mime = text_in(image, "mimetype").unwrap_or_else(|| "image/jpeg".to_owned());
        if text.is_none() {
            text = text_in(image, "caption");
        }
        attachments.push(AttachmentToDownload {
            name: attachment_name(image, &id, &mime, "imagem"),
            mime,
            message_id: id.clone(),
            declared_size: declared_size(image),
            data: data.clone(),
        });
    } else if let Some(document) = content.get("documentMessage") {
        let mime =
            text_in(document, "mimetype").unwrap_or_else(|| "application/octet-stream".to_owned());
        if text.is_none() {
            text = text_in(document, "caption");
        }
        if !supported_mime(&mime) {
            return Decision::Ignored(IgnoreReason::MimeNotSupported(mime));
        }
        attachments.push(AttachmentToDownload {
            name: attachment_name(document, &id, &mime, "documento"),
            mime,
            message_id: id.clone(),
            declared_size: declared_size(document),
            data: data.clone(),
        });
    } else if let Some(kind) = unsupported_attachment(&content) {
        if from_me {
            return Decision::Ignored(IgnoreReason::AttachmentNotSupported(
                kind.label().to_owned(),
            ));
        }
        text = Some(kind.history_text().to_owned());
    }

    if text.is_none() && attachments.is_empty() {
        return Decision::Ignored(IgnoreReason::NoContent);
    }

    let mut metadata = serde_json::json!({
        "canal": "whatsapp",
        "remote_jid": jid,
        "from_me": from_me,
        "grupo": group,
        "instance": body.get("instance").and_then(Value::as_str),
    });
    if !from_me
        && !group
        && let Some(name) = text_in(&data, "pushName")
    {
        metadata["push_name"] = Value::String(name.trim().to_owned());
    }

    let event = InboundEvent {
        integration,
        id_externo: id,
        peer: peer.clone(),
        remetente,
        text: text.unwrap_or_default(),
        attachments: Vec::new(),
        occurred_at: ocorrido_em,
        metadata,
    };

    Decision::Inbound(ReceivedEntry { event, attachments })
}

pub fn embedded_attachment(name: &str, mime: &str, bytes: &[u8]) -> InboundAttachment {
    InboundAttachment::Bytes {
        name: name.to_owned(),
        mime: mime.to_owned(),
        data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
    }
}

fn is_state_event(event: &str) -> bool {
    matches!(
        event.to_ascii_lowercase().as_str(),
        "connection.update" | "qrcode.updated" | "connection_update"
    )
}

fn channel_state(body: &Value, event: &str) -> StateEvent {
    let data = body.get("data").cloned().unwrap_or(Value::Null);
    if event.to_ascii_lowercase().starts_with("qrcode") {
        let code = data
            .get("qrcode")
            .and_then(|qrcode| text_in(qrcode, "base64"))
            .or_else(|| text_in(&data, "base64"))
            .unwrap_or_default();
        return StateEvent::Qr {
            code,
            expires_at: None,
        };
    }
    let situacao = text_in(&data, "state")
        .or_else(|| text_in(&data, "status"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    match situacao.as_str() {
        "open" | "connected" | "conectado" => StateEvent::Connected { since: now_ms() },
        "close" | "closed" | "disconnected" | "desconectado" => StateEvent::Disconnected {
            reason: Some("a Evolution reportou a instância desconectada".to_owned()),
        },
        other => StateEvent::Error {
            code: "connection_state".to_owned(),
            message: format!("estado de conexão `{other}`"),
            transient: true,
        },
    }
}

fn unwrap_message(message: Value) -> Value {
    let mut current = message;
    for _ in 0..4 {
        let next = current
            .get("ephemeralMessage")
            .or_else(|| current.get("viewOnceMessage"))
            .or_else(|| current.get("viewOnceMessageV2"))
            .and_then(|embrulho| embrulho.get("message"))
            .cloned();
        match next {
            Some(next) => current = next,
            None => break,
        }
    }
    current
}

fn extract_text(content: &Value) -> Option<String> {
    let candidates = [
        content
            .get("conversation")
            .and_then(Value::as_str)
            .map(str::to_owned),
        content
            .get("extendedTextMessage")
            .and_then(|value| text_in(value, "text")),
        content
            .get("imageMessage")
            .and_then(|value| text_in(value, "caption")),
        content
            .get("videoMessage")
            .and_then(|value| text_in(value, "caption")),
        content
            .get("documentMessage")
            .and_then(|value| text_in(value, "caption")),
        content
            .get("buttonsResponseMessage")
            .and_then(|value| text_in(value, "selectedDisplayText")),
        content
            .get("listResponseMessage")
            .and_then(|value| value.get("singleSelectReply"))
            .and_then(|value| text_in(value, "selectedRowId")),
        content
            .get("templateButtonReplyMessage")
            .and_then(|value| text_in(value, "selectedDisplayText")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|candidate| !candidate.trim().is_empty())
        .map(|candidate| candidate.trim().to_owned())
}

fn unsupported_attachment(content: &Value) -> Option<UnsupportedContent> {
    const KEYS: [(&str, UnsupportedContent); 11] = [
        ("audioMessage", UnsupportedContent::Audio),
        ("videoMessage", UnsupportedContent::Video),
        ("ptvMessage", UnsupportedContent::Video),
        ("stickerMessage", UnsupportedContent::Sticker),
        ("contactMessage", UnsupportedContent::Contact),
        ("contactsArrayMessage", UnsupportedContent::Contact),
        ("locationMessage", UnsupportedContent::Location),
        ("liveLocationMessage", UnsupportedContent::Location),
        ("pollCreationMessage", UnsupportedContent::Poll),
        ("pollCreationMessageV2", UnsupportedContent::Poll),
        ("pollCreationMessageV3", UnsupportedContent::Poll),
    ];
    KEYS.iter()
        .find(|(key, _)| content.get(*key).is_some())
        .map(|(_, kind)| *kind)
}

fn supported_mime(mime: &str) -> bool {
    const ACCEPTED: [&str; 6] = [
        "application/pdf",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "application/msword",
        "application/vnd.ms-excel",
    ];
    let lowercase = mime.trim().to_ascii_lowercase();
    lowercase.starts_with("image/") || ACCEPTED.contains(&lowercase.as_str())
}

fn attachment_name(content: &Value, id: &str, mime: &str, prefix: &str) -> String {
    if let Some(name) = text_in(content, "fileName") {
        return name;
    }
    let lowercase = mime.trim().to_ascii_lowercase();
    let extension = match lowercase.as_str() {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "application/pdf" => "pdf",
        other => other.rsplit('/').next().unwrap_or("bin"),
    };
    format!("{prefix}-{id}.{extension}")
}

fn declared_size(content: &Value) -> Option<u64> {
    let value = content.get("fileLength")?;
    if let Some(numero) = value.as_u64() {
        return Some(numero);
    }
    if let Some(text) = value.as_str() {
        return text.trim().parse::<u64>().ok();
    }
    let low = value.get("low").and_then(Value::as_i64)? as u64 & 0xFFFF_FFFF;
    let high = value.get("high").and_then(Value::as_i64).unwrap_or(0) as u64 & 0xFFFF_FFFF;
    Some((high << 32) | low)
}

fn timestamp_ms(data: &Value) -> Option<i64> {
    let value = data.get("messageTimestamp")?;
    let seconds = if let Some(numero) = value.as_i64() {
        numero
    } else {
        let text = value.as_str()?;
        text.trim().parse::<i64>().ok()?
    };
    if seconds <= 0 {
        return None;
    }
    Some(if seconds > 10_000_000_000 {
        seconds
    } else {
        seconds * 1000
    })
}

fn jid_number(jid: &str) -> String {
    let without_suffix = jid.split('@').next().unwrap_or_default();
    let without_device = without_suffix.split(':').next().unwrap_or_default();
    without_device
        .chars()
        .filter(char::is_ascii_digit)
        .collect()
}

fn text_in(value: &Value, campo: &str) -> Option<String> {
    value
        .get(campo)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|text| !text.trim().is_empty())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}
