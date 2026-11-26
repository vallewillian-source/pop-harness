use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use poph_types::{
    Capabilities, ChannelLimits, CommandFuture, Health, HealthFuture, InboundEvent, Integration,
    IntegrationError, IntegrationId, IntegrationState, JsonSchema, Manifest, OutboundCommand,
    PROTOCOL_VERSION, Presence, Secret, SendFile, SendText, SetPresence,
};

use crate::client::{ATTACHMENT_CAP, EvolutionClient};
use crate::error::EvolutionError;
use crate::text::TEXT_LIMIT;
use crate::webhook::{Decision, Policy, SECRET_HEADER, WebhookError, check_secret, interpret};

#[derive(Clone, Debug)]
pub struct ChannelConfig {
    pub integration: IntegrationId,
    pub webhook_url: Option<String>,
    pub webhook_secret: Secret,
    pub accept_from_me: bool,
    pub attachment_cap: u64,
}

impl ChannelConfig {
    pub fn new(integration: IntegrationId, webhook_secret: Secret) -> Result<Self, EvolutionError> {
        if webhook_secret.expose().trim().is_empty() {
            return Err(EvolutionError::InvalidConfiguration(
                "a integração do WhatsApp precisa do segredo do webhook".to_owned(),
            ));
        }
        Ok(Self {
            integration,
            webhook_url: None,
            webhook_secret,
            accept_from_me: false,
            attachment_cap: ATTACHMENT_CAP,
        })
    }

    pub fn with_webhook(mut self, url: impl Into<String>) -> Self {
        let url = url.into().trim().to_owned();
        self.webhook_url = if url.is_empty() { None } else { Some(url) };
        self
    }
}

pub type ReceiptFuture =
    Pin<Box<dyn Future<Output = Result<Option<String>, IntegrationError>> + Send + 'static>>;

pub trait AttachmentSource: Send + Sync + 'static {
    fn read(&self, blob_ref: &str) -> Result<Vec<u8>, EvolutionError>;
}

#[derive(Clone, Debug)]
pub struct DiskBlobs {
    root: PathBuf,
}

impl DiskBlobs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl AttachmentSource for DiskBlobs {
    fn read(&self, blob_ref: &str) -> Result<Vec<u8>, EvolutionError> {
        let relative = Path::new(blob_ref);
        let safe = !relative.is_absolute()
            && relative
                .components()
                .all(|componente| matches!(componente, Component::Normal(_)));
        if !safe {
            return Err(EvolutionError::BlobUnreadable {
                blob: blob_ref.to_owned(),
                reason: "referência de blob fora da raiz".to_owned(),
            });
        }
        let path = self.root.join(relative);
        std::fs::read(&path).map_err(|error| EvolutionError::BlobUnreadable {
            blob: blob_ref.to_owned(),
            reason: error.to_string(),
        })
    }
}

#[derive(Clone)]
pub struct WhatsAppEvolution {
    client: EvolutionClient,
    config: ChannelConfig,
    source: Arc<dyn AttachmentSource>,
    manifest: Arc<Manifest>,
    state: Arc<Mutex<IntegrationState>>,
}

impl WhatsAppEvolution {
    pub fn new(
        client: EvolutionClient,
        config: ChannelConfig,
        source: Arc<dyn AttachmentSource>,
    ) -> Self {
        Self {
            client,
            config,
            source,
            manifest: Arc::new(manifest()),
            state: Arc::new(Mutex::new(IntegrationState::Starting)),
        }
    }

    pub fn for_output(
        client: EvolutionClient,
        integration: IntegrationId,
        source: Arc<dyn AttachmentSource>,
    ) -> Self {
        let config = ChannelConfig {
            integration,
            webhook_url: None,
            webhook_secret: Secret::new(String::new()),
            accept_from_me: false,
            attachment_cap: ATTACHMENT_CAP,
        };
        Self::new(client, config, source)
    }

    pub async fn start(&self) -> Result<String, EvolutionError> {
        let state = self.client.connection_state().await?;
        {
            let mut guard = self.state.lock().expect("estado do canal");
            *guard = if state == "open" {
                IntegrationState::Ready
            } else {
                IntegrationState::Starting
            };
        }
        if let Some(url) = self.config.webhook_url.as_deref() {
            self.client
                .set_webhook(
                    url,
                    &["MESSAGES_UPSERT", "CONNECTION_UPDATE", "QRCODE_UPDATED"],
                    &[(
                        SECRET_HEADER.to_owned(),
                        self.config.webhook_secret.expose().clone(),
                    )],
                )
                .await?;
        }
        Ok(state)
    }

    pub fn apply_state(&self, state: &poph_types::StateEvent) {
        let mut guard = self.state.lock().expect("estado do canal");
        *guard = match state {
            poph_types::StateEvent::Connected { .. } => IntegrationState::Ready,
            poph_types::StateEvent::Disconnected { .. } => IntegrationState::Starting,
            poph_types::StateEvent::Qr { .. } => IntegrationState::Starting,
            poph_types::StateEvent::Error { .. } => IntegrationState::Starting,
        };
    }

    pub fn receive_webhook(
        &self,
        received_secret: Option<&str>,
        body: &serde_json::Value,
    ) -> Result<Decision, WebhookError> {
        check_secret(&self.config.webhook_secret, received_secret)?;
        let policy = Policy {
            accept_from_me: self.config.accept_from_me,
        };
        let decision = interpret(self.config.integration, body, &policy);
        match &decision {
            Decision::State(state) => self.apply_state(state),
            Decision::Inbound(_) => {
                let mut guard = self.state.lock().expect("estado do canal");
                *guard = IntegrationState::Ready;
            }
            Decision::Ignored(_) => {}
        }
        Ok(decision)
    }

    pub async fn download_attachment(
        &self,
        attachment: &crate::webhook::AttachmentToDownload,
    ) -> Result<crate::client::DownloadedAttachment, EvolutionError> {
        if let Some(declared) = attachment.declared_size
            && declared > self.config.attachment_cap
        {
            return Err(EvolutionError::AttachmentTooLarge {
                bytes: declared,
                cap: self.config.attachment_cap,
            });
        }
        self.client
            .download_attachment(
                &attachment.message_id,
                &attachment.data,
                self.config.attachment_cap,
            )
            .await
    }

    pub async fn receive_and_deliver(
        &self,
        decision: Decision,
        receiver: &dyn poph_types::EventSink,
    ) {
        match decision {
            Decision::State(state) => receiver.receive(poph_types::IntegrationEvent::Estado(state)),
            Decision::Inbound(inbound) => {
                let mut attachments = Vec::new();
                for attachment in &inbound.attachments {
                    match self.download_attachment(attachment).await {
                        Ok(downloaded) => attachments.push(crate::webhook::embedded_attachment(
                            &attachment.name,
                            &downloaded.mime,
                            &downloaded.bytes,
                        )),
                        Err(error) => tracing::warn!(
                            target: "poph_evolution",
                            message_id = %attachment.message_id,
                            reason = %error,
                            "mídia do WhatsApp não baixada; o evento segue sem o anexo"
                        ),
                    }
                }
                let event: InboundEvent = inbound.with_attachments(attachments);
                receiver.receive(poph_types::IntegrationEvent::Inbound(event))
            }
            Decision::Ignored(reason) => tracing::debug!(
                target: "poph_evolution",
                reason = ?reason,
                "entrega do WhatsApp ignorada"
            ),
        }
    }

    pub fn send_with_receipt(&self, command: OutboundCommand) -> ReceiptFuture {
        let this = self.clone();
        Box::pin(async move {
            match command {
                OutboundCommand::Text(c) => this.send_text_receipt(c).await,
                OutboundCommand::File(c) => this.send_file_receipt(c).await,
                OutboundCommand::Presence(c) => {
                    this.send_presence_without_error(c).await.map(|()| None)
                }
            }
        })
    }

    pub async fn send_text_receipt(
        &self,
        command: SendText,
    ) -> Result<Option<String>, IntegrationError> {
        self.client
            .send_text(&command.peer, &command.text)
            .await
            .map_err(|error| error.to_integration("enviar_texto"))
    }

    pub async fn send_file_receipt(
        &self,
        command: SendFile,
    ) -> Result<Option<String>, IntegrationError> {
        if command.size_bytes > self.config.attachment_cap {
            return Err(IntegrationError::MessageTooLarge {
                bytes: usize::try_from(command.size_bytes).unwrap_or(usize::MAX),
                cap: usize::try_from(self.config.attachment_cap).unwrap_or(usize::MAX),
            });
        }
        let bytes = self
            .source
            .read(&command.blob_ref)
            .map_err(|error| error.to_integration("enviar_arquivo"))?;
        self.client
            .send_attachment(
                &command.peer,
                &bytes,
                &command.mime,
                &command.name,
                command.caption.as_deref(),
            )
            .await
            .map_err(|error| error.to_integration("enviar_arquivo"))
    }

    async fn send_presence_without_error(
        &self,
        command: SetPresence,
    ) -> Result<(), IntegrationError> {
        let typing = matches!(command.state, Presence::Typing);
        match self.client.send_presence(&command.peer, typing).await {
            Ok(()) => Ok(()),
            Err(error) => {
                tracing::debug!(
                    target: "poph_evolution",
                    reason = %error,
                    "presença não aceita pela Evolution"
                );
                Ok(())
            }
        }
    }
}

impl Integration for WhatsAppEvolution {
    fn manifest(&self) -> Option<Manifest> {
        Some((*self.manifest).clone())
    }

    fn state(&self) -> IntegrationState {
        self.state.lock().expect("estado do canal").clone()
    }

    fn send_text(&self, command: SendText) -> CommandFuture {
        let this = self.clone();
        Box::pin(async move { this.send_text_receipt(command).await.map(|_| ()) })
    }

    fn send_file(&self, command: SendFile) -> CommandFuture {
        let this = self.clone();
        Box::pin(async move { this.send_file_receipt(command).await.map(|_| ()) })
    }

    fn set_presence(&self, command: SetPresence) -> CommandFuture {
        let this = self.clone();
        Box::pin(async move { this.send_presence_without_error(command).await })
    }

    fn health(&self) -> HealthFuture {
        let this = self.clone();
        Box::pin(async move {
            match this.client.connection_state().await {
                Ok(state) if state == "open" => Ok(Health::ok(format!(
                    "Evolution: {state} (instância {})",
                    this.client.instancia()
                ))),
                Ok(state) => Ok(Health {
                    ok: false,
                    detail: Some(format!("Evolution: {state}")),
                }),
                Err(error) => Err(error.to_integration("saude")),
            }
        })
    }
}

fn manifest() -> Manifest {
    Manifest {
        name: "WhatsApp (Evolution)".to_owned(),
        kind: "whatsapp".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        protocol: PROTOCOL_VERSION,
        capabilities: Capabilities {
            text: true,
            image_in: true,
            file_in: true,
            file_out: true,
            typing: true,
            read_receipt: false,
        },
        limits: ChannelLimits {
            max_attachment_bytes: ATTACHMENT_CAP,
            max_text_bytes: TEXT_LIMIT as u64,
            accepted_types: vec![
                "image/*".to_owned(),
                "application/pdf".to_owned(),
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                    .to_owned(),
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_owned(),
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                    .to_owned(),
            ],
        },
        configuration: JsonSchema(serde_json::json!({
            "type": "object",
            "required": ["instancia"],
            "properties": {
                "instancia": {
                    "type": "string",
                    "description": "Nome da instância na EvolutionAPI (o mesmo que aparece no painel dela)."
                },
                "url_base": {
                    "type": "string",
                    "format": "uri",
                    "description": "Endereço da EvolutionAPI (http:// na rede local, https:// exposta)."
                },
                "webhook_url": {
                    "type": "string",
                    "format": "uri",
                    "description": "Endereço em que a Evolution entrega as mensagens. Vazio = não mexe no webhook já configurado."
                }
            }
        })),
        secrets: JsonSchema(serde_json::json!({
            "type": "object",
            "required": ["apikey", "webhook_secret"],
            "properties": {
                "apikey": {
                    "type": "string",
                    "format": "password",
                    "x-secret": true,
                    "description": "Chave da EvolutionAPI (cabeçalho `apikey`)."
                },
                "webhook_secret": {
                    "type": "string",
                    "format": "password",
                    "x-secret": true,
                    "description": "Segredo conferido no cabeçalho `X-Evolution-Webhook-Secret` de cada entrega."
                }
            }
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_declares_what_the_channel_does() {
        let manifest = manifest();
        assert_eq!(manifest.kind, "whatsapp");
        assert_eq!(manifest.protocol, PROTOCOL_VERSION);
        assert!(manifest.capabilities.text);
        assert!(manifest.capabilities.file_in);
        assert!(manifest.capabilities.file_out);
        assert_eq!(manifest.validar(), Ok(()));
    }

    #[test]
    fn webhook_secret_is_required() {
        let error = ChannelConfig::new(IntegrationId::generate(), Secret::new(String::new()))
            .expect_err("sem segredo deve falhar");
        assert!(matches!(error, EvolutionError::InvalidConfiguration(_)));
    }

    #[test]
    fn blob_outside_the_root_is_refused() {
        let source = DiskBlobs::new("/tmp/blobs");
        assert!(source.read("../etc/passwd").is_err());
        assert!(source.read("/etc/passwd").is_err());
    }
}
