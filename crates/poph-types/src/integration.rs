use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ids::{IntegrationId, OrgId};

pub const PROTOCOL_VERSION: u32 = 1;

pub const DEFAULT_ATTACHMENT_LIMIT: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(rename = "nome")]
    pub name: String,
    pub kind: String,
    #[serde(rename = "versao")]
    pub version: String,
    #[serde(rename = "protocolo")]
    pub protocol: u32,
    #[serde(default)]
    #[serde(rename = "capacidades")]
    pub capabilities: Capabilities,
    #[serde(default)]
    #[serde(rename = "limites")]
    pub limits: ChannelLimits,
    #[serde(rename = "configuracao")]
    pub configuration: JsonSchema,
    #[serde(default)]
    #[serde(rename = "segredos")]
    pub secrets: JsonSchema,
}

impl Manifest {
    pub fn validar(&self) -> Result<(), IntegrationError> {
        if self.name.trim().is_empty() {
            return Err(IntegrationError::HandshakeRefused {
                reason: "o manifesto não traz o nome do plugin".to_owned(),
            });
        }
        if self.kind.trim().is_empty() {
            return Err(IntegrationError::HandshakeRefused {
                reason: "o manifesto não traz o tipo (`kind`) da integração".to_owned(),
            });
        }
        if self.configuration.0.is_null() {
            return Err(IntegrationError::HandshakeRefused {
                reason: "o manifesto não traz o JSON Schema da configuração".to_owned(),
            });
        }
        self.configuration.validar("configuração")?;
        if !self.secrets.0.is_null() {
            self.secrets.validar("segredos")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub text: bool,
    #[serde(default)]
    pub image_in: bool,
    #[serde(default)]
    pub file_in: bool,
    #[serde(default)]
    pub file_out: bool,
    #[serde(default)]
    pub typing: bool,
    #[serde(default)]
    pub read_receipt: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelLimits {
    #[serde(default = "default_attachment_limit")]
    #[serde(rename = "max_anexo_bytes")]
    pub max_attachment_bytes: u64,
    #[serde(default)]
    #[serde(rename = "max_texto_bytes")]
    pub max_text_bytes: u64,
    #[serde(default)]
    #[serde(rename = "tipos_aceitos")]
    pub accepted_types: Vec<String>,
}

fn default_attachment_limit() -> u64 {
    DEFAULT_ATTACHMENT_LIMIT
}

impl Default for ChannelLimits {
    fn default() -> Self {
        Self {
            max_attachment_bytes: DEFAULT_ATTACHMENT_LIMIT,
            max_text_bytes: 0,
            accepted_types: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JsonSchema(pub serde_json::Value);

impl JsonSchema {
    pub fn empty() -> Self {
        Self(serde_json::json!({"type": "object", "properties": {}}))
    }

    pub fn validar(&self, label: &'static str) -> Result<(), IntegrationError> {
        let mut reason = None;
        match self.0.as_object() {
            None => reason = Some("não é um objeto JSON"),
            Some(object) => {
                if object.get("type").and_then(|t| t.as_str()) != Some("object") {
                    reason = Some("`type` não é \"object\"");
                } else if let Some(properties) = object.get("properties")
                    && !properties.is_object()
                {
                    reason = Some("`properties` não é um objeto");
                }
            }
        }
        match reason {
            None => Ok(()),
            Some(reason) => Err(IntegrationError::HandshakeRefused {
                reason: format!("o JSON Schema de {label} é inválido: {reason}"),
            }),
        }
    }
}

impl Default for JsonSchema {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PluginsConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[serde(rename = "integracoes")]
    pub integrations: Vec<DeclaredPlugin>,
}

impl PluginsConfig {
    pub fn empty(&self) -> bool {
        self.integrations.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredPlugin {
    pub kind: String,
    #[serde(rename = "programa")]
    pub program: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[serde(rename = "argumentos")]
    pub arguments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InboundEvent {
    #[serde(rename = "integracao")]
    pub integration: IntegrationId,
    pub id_externo: String,
    pub peer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remetente: Option<String>,
    #[serde(default)]
    #[serde(rename = "texto")]
    pub text: String,
    #[serde(default)]
    #[serde(rename = "anexos")]
    pub attachments: Vec<InboundAttachment>,
    #[serde(rename = "ocorrido_em")]
    pub occurred_at: i64,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    #[serde(rename = "metadados")]
    pub metadata: serde_json::Value,
}

impl InboundEvent {
    pub fn validar(&self) -> Result<(), IntegrationError> {
        if self.id_externo.trim().is_empty() {
            return Err(IntegrationError::Protocol {
                reason: "evento de entrada sem `id_externo`".to_owned(),
            });
        }
        if self.peer.trim().is_empty() {
            return Err(IntegrationError::Protocol {
                reason: "evento de entrada sem `peer`".to_owned(),
            });
        }
        if self.occurred_at <= 0 {
            return Err(IntegrationError::Protocol {
                reason: "evento de entrada sem `ocorrido_em` válido".to_owned(),
            });
        }
        for attachment in &self.attachments {
            attachment.validar()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "fonte", rename_all = "snake_case")]
pub enum InboundAttachment {
    Blob {
        #[serde(rename = "nome")]
        name: String,
        mime: String,
        sha256: String,
        #[serde(rename = "tamanho_bytes")]
        size_bytes: u64,
    },
    Bytes {
        #[serde(rename = "nome")]
        name: String,
        mime: String,
        #[serde(rename = "dados_base64")]
        data_base64: String,
    },
}

impl InboundAttachment {
    fn validar(&self) -> Result<(), IntegrationError> {
        let (name, mime) = match self {
            InboundAttachment::Blob {
                name,
                mime,
                sha256,
                size_bytes,
            } => {
                if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(IntegrationError::Protocol {
                        reason: format!("anexo `{name}` com sha256 inválido (esperado 64 hexa)"),
                    });
                }
                if *size_bytes == 0 {
                    return Err(IntegrationError::Protocol {
                        reason: format!("anexo `{name}` com tamanho zero"),
                    });
                }
                (name, mime)
            }
            InboundAttachment::Bytes {
                name,
                mime,
                data_base64,
            } => {
                if data_base64.is_empty() {
                    return Err(IntegrationError::Protocol {
                        reason: format!("anexo `{name}` sem conteúdo"),
                    });
                }
                (name, mime)
            }
        };
        if name.trim().is_empty() || mime.trim().is_empty() {
            return Err(IntegrationError::Protocol {
                reason: "anexo sem nome ou sem tipo MIME".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum OutboundCommand {
    #[serde(rename = "enviar_texto")]
    Text(SendText),
    #[serde(rename = "enviar_arquivo")]
    File(SendFile),
    #[serde(rename = "definir_presenca")]
    Presence(SetPresence),
}

impl OutboundCommand {
    pub fn protocol_name(&self) -> &'static str {
        match self {
            OutboundCommand::Text(_) => "enviar_texto",
            OutboundCommand::File(_) => "enviar_arquivo",
            OutboundCommand::Presence(_) => "definir_presenca",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendText {
    #[serde(rename = "idempotencia")]
    pub idempotency: String,
    pub peer: String,
    #[serde(rename = "texto")]
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendFile {
    #[serde(rename = "idempotencia")]
    pub idempotency: String,
    pub peer: String,
    #[serde(rename = "nome")]
    pub name: String,
    pub mime: String,
    #[serde(rename = "tamanho_bytes")]
    pub size_bytes: u64,
    pub sha256: String,
    pub blob_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "legenda")]
    pub caption: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    #[serde(rename = "digitando")]
    Typing,
    #[serde(rename = "pausado")]
    Paused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetPresence {
    pub peer: String,
    #[serde(rename = "estado")]
    pub state: Presence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "estado", rename_all = "snake_case")]
pub enum StateEvent {
    #[serde(rename = "conectado")]
    Connected {
        #[serde(default)]
        #[serde(rename = "desde")]
        since: i64,
    },
    #[serde(rename = "desconectado")]
    Disconnected {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[serde(rename = "motivo")]
        reason: Option<String>,
    },
    Qr {
        #[serde(rename = "codigo")]
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[serde(rename = "expira_em")]
        expires_at: Option<i64>,
    },
    #[serde(rename = "erro")]
    Error {
        #[serde(rename = "codigo")]
        code: String,
        #[serde(rename = "mensagem")]
        message: String,
        #[serde(default)]
        #[serde(rename = "transitorio")]
        transient: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "detalhe")]
    pub detail: Option<String>,
}

impl Health {
    pub fn ok(detail: impl Into<String>) -> Self {
        Self {
            ok: true,
            detail: Some(detail.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandError {
    #[serde(rename = "codigo")]
    pub code: String,
    #[serde(rename = "mensagem")]
    pub message: String,
    #[serde(default)]
    #[serde(rename = "transitorio")]
    pub transient: bool,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Handshake {
    #[serde(rename = "protocolo")]
    pub protocol: u32,
    #[serde(rename = "integracao")]
    pub integration: IntegrationId,
    pub org: OrgId,
    pub kind: String,
    #[serde(default)]
    pub config: serde_json::Value,
    #[serde(default)]
    #[serde(rename = "segredos")]
    pub secrets: BTreeMap<String, String>,
}

impl fmt::Debug for Handshake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handshake")
            .field("protocolo", &self.protocol)
            .field("integracao", &self.integration)
            .field("org", &self.org)
            .field("kind", &self.kind)
            .field("config", &self.config)
            .field(
                "segredos",
                &format_args!("{} chave(s) [oculto]", self.secrets.len()),
            )
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum HostMessage {
    Handshake(Handshake),
    #[serde(rename = "comando")]
    Command {
        id: u64,
        #[serde(rename = "comando")]
        command: OutboundCommand,
    },
    #[serde(rename = "saude")]
    Health {
        id: u64,
    },
    #[serde(rename = "encerrar")]
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum PluginMessage {
    HandshakeOk {
        protocolo: u32,
        #[serde(rename = "manifesto")]
        manifest: Manifest,
    },
    Evento {
        evento: InboundEvent,
    },
    #[serde(rename = "estado")]
    State {
        estado: StateEvent,
    },
    #[serde(rename = "resultado")]
    Result {
        id: u64,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[serde(rename = "erro")]
        error: Option<CommandError>,
    },
    #[serde(rename = "saude")]
    Health {
        id: u64,
        #[serde(rename = "saude")]
        health: Health,
    },
    #[serde(rename = "erro")]
    Error {
        #[serde(rename = "codigo")]
        code: String,
        #[serde(rename = "mensagem")]
        message: String,
        #[serde(default)]
        #[serde(rename = "transitorio")]
        transient: bool,
    },
}

pub type CommandFuture =
    Pin<Box<dyn Future<Output = Result<(), IntegrationError>> + Send + 'static>>;

pub type HealthFuture =
    Pin<Box<dyn Future<Output = Result<Health, IntegrationError>> + Send + 'static>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrationState {
    Starting,
    Ready,
    Restarting { failures: u32, wait_ms: u64 },
    Down { failures: u32, reason: String },
    Closed,
}

impl IntegrationState {
    pub fn is_ready(&self) -> bool {
        matches!(self, IntegrationState::Ready)
    }

    pub fn is_closed(&self) -> bool {
        matches!(
            self,
            IntegrationState::Down { .. } | IntegrationState::Closed
        )
    }
}

pub trait EventSink: Send + Sync + 'static {
    fn receive(&self, event: IntegrationEvent);
}

#[derive(Debug, Clone, PartialEq)]
pub enum IntegrationEvent {
    Inbound(InboundEvent),
    Estado(StateEvent),
}

pub trait Integration: Send + Sync + 'static {
    fn manifest(&self) -> Option<Manifest>;

    fn state(&self) -> IntegrationState;

    fn send_text(&self, command: SendText) -> CommandFuture;

    fn send_file(&self, command: SendFile) -> CommandFuture;

    fn set_presence(&self, command: SetPresence) -> CommandFuture;

    fn health(&self) -> HealthFuture;
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum IntegrationError {
    #[error("não consegui iniciar o plugin `{program}`: {reason}")]
    StartFailed { program: String, reason: String },

    #[error("protocolo incompatível: o host fala {expected} e o plugin falou {received}")]
    IncompatibleProtocol { expected: u32, received: u32 },

    #[error("handshake recusado: {reason}")]
    HandshakeRefused { reason: String },

    #[error("protocolo de plugin inválido: {reason}")]
    Protocol { reason: String },

    #[error("mensagem de {bytes} bytes passa do limite de {cap} bytes")]
    MessageTooLarge { bytes: usize, cap: usize },

    #[error("integração indisponível: {reason}")]
    Unavailable { reason: String },

    #[error("tempo esgotado em {operation} depois de {limit_ms} ms")]
    Timeout { operation: String, limit_ms: u64 },

    #[error("o plugin recusou {operation}: {code} ({message})")]
    Refused {
        operation: String,
        code: String,
        message: String,
        transient: bool,
    },

    #[error("integração caída depois de {failures} falhas seguidas: {reason}")]
    Down { failures: u32, reason: String },

    #[error("integração encerrada")]
    Closed,
}

impl IntegrationError {
    pub fn is_transient(&self) -> bool {
        match self {
            IntegrationError::Unavailable { .. } | IntegrationError::Timeout { .. } => true,
            IntegrationError::Refused { transient, .. } => *transient,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest() -> Manifest {
        Manifest {
            name: "Eco".to_owned(),
            kind: "eco".to_owned(),
            version: "1.0.0".to_owned(),
            protocol: PROTOCOL_VERSION,
            capabilities: Capabilities {
                text: true,
                file_out: true,
                ..Capabilities::default()
            },
            limits: ChannelLimits::default(),
            configuration: JsonSchema(serde_json::json!({
                "type": "object",
                "properties": {"modo": {"type": "string"}}
            })),
            secrets: JsonSchema(serde_json::json!({
                "type": "object",
                "properties": {"token": {"type": "string"}}
            })),
        }
    }

    #[test]
    fn valid_manifest_passes() {
        assert_eq!(sample_manifest().validar(), Ok(()));
    }

    #[test]
    fn manifest_without_kind_is_rejected() {
        let mut manifest = sample_manifest();
        manifest.kind = "  ".to_owned();
        assert!(matches!(
            manifest.validar(),
            Err(IntegrationError::HandshakeRefused { .. })
        ));
    }

    #[test]
    fn schema_that_is_not_object_is_rejected() {
        let mut manifest = sample_manifest();
        manifest.configuration = JsonSchema(serde_json::json!({"type": "array"}));
        let error = manifest.validar().unwrap_err();
        assert!(error.to_string().contains("configuração"), "{error}");
    }

    #[test]
    fn handshake_hides_secrets_in_debug() {
        let secret = "tok-super-secreto-123".to_owned();
        let handshake = Handshake {
            protocol: PROTOCOL_VERSION,
            integration: IntegrationId::generate(),
            org: OrgId::generate(),
            kind: "eco".to_owned(),
            config: serde_json::json!({"modo": "normal"}),
            secrets: BTreeMap::from([("token".to_owned(), secret.clone())]),
        };
        let text = format!("{handshake:?} {handshake:#?}");
        assert!(!text.contains(&secret), "debug vazou: {text}");
        assert!(text.contains("1 chave(s) [oculto]"), "{text}");

        let json = serde_json::to_string(&handshake).unwrap();
        assert!(json.contains(&secret));
    }

    #[test]
    fn valid_event_passes_and_missing_external_id_fails() {
        let event = InboundEvent {
            integration: IntegrationId::generate(),
            id_externo: "abc-1".to_owned(),
            peer: "5511999999999".to_owned(),
            remetente: None,
            text: "oi".to_owned(),
            attachments: vec![InboundAttachment::Blob {
                name: "manual.pdf".to_owned(),
                mime: "application/pdf".to_owned(),
                sha256: "a".repeat(64),
                size_bytes: 10,
            }],
            occurred_at: 1_795_140_000_000,
            metadata: serde_json::Value::Null,
        };
        assert_eq!(event.validar(), Ok(()));

        let mut without_id = event.clone();
        without_id.id_externo = String::new();
        assert!(matches!(
            without_id.validar(),
            Err(IntegrationError::Protocol { .. })
        ));

        let mut short_sha = event;
        short_sha.attachments = vec![InboundAttachment::Blob {
            name: "manual.pdf".to_owned(),
            mime: "application/pdf".to_owned(),
            sha256: "abc".to_owned(),
            size_bytes: 10,
        }];
        assert!(short_sha.validar().is_err());
    }

    #[test]
    fn protocol_messages_have_documented_shape() {
        let command = HostMessage::Command {
            id: 7,
            command: OutboundCommand::Text(SendText {
                idempotency: "turno:0".to_owned(),
                peer: "5511".to_owned(),
                text: "oi".to_owned(),
            }),
        };
        let json = serde_json::to_value(&command).unwrap();
        assert_eq!(json["tipo"], "comando");
        assert_eq!(json["id"], 7);
        assert_eq!(json["comando"]["tipo"], "enviar_texto");
        assert_eq!(json["comando"]["texto"], "oi");

        let response: PluginMessage = serde_json::from_str(
            r#"{"tipo":"resultado","id":7,"ok":false,"erro":{"codigo":"401","mensagem":"sessão caiu","transitorio":false}}"#,
        )
        .unwrap();
        assert!(matches!(response, PluginMessage::Result { ok: false, .. }));

        let future: PluginMessage =
            serde_json::from_str(r#"{"tipo":"resultado","id":1,"ok":true,"extra":42}"#).unwrap();
        assert!(matches!(future, PluginMessage::Result { ok: true, .. }));
    }

    #[test]
    fn transient_error_is_recognised() {
        assert!(
            IntegrationError::Timeout {
                operation: "comando".to_owned(),
                limit_ms: 10
            }
            .is_transient()
        );
        assert!(
            !IntegrationError::IncompatibleProtocol {
                expected: 1,
                received: 2
            }
            .is_transient()
        );
        assert!(
            IntegrationError::Refused {
                operation: "enviar_texto".to_owned(),
                code: "429".to_owned(),
                message: "devagar".to_owned(),
                transient: true,
            }
            .is_transient()
        );
    }
}
