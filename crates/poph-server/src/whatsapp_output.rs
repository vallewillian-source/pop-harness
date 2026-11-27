use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poph_agent::Backoff;
use poph_agent::entry_output::SendFailure;
use poph_agent::queue_integration::{chave_idempotencia, command_item};
use poph_evolution::{AttachmentSource, DiskBlobs, WhatsAppEvolution};
use poph_store::{OutboxItem, Store, StoreError};
use poph_types::{IntegrationId, OrgId, OutboxId};

use crate::evolution::EvolutionChannel;
use crate::state::ServerState;

#[derive(Debug, Clone)]
pub struct WhatsAppOutputOptions {
    pub batch_size: u32,
    pub max_attempts: u32,
    pub backoff: Backoff,
    pub interval: Duration,
}

impl Default for WhatsAppOutputOptions {
    fn default() -> Self {
        Self {
            batch_size: 100,
            max_attempts: 8,
            backoff: Backoff::default(),
            interval: Duration::from_secs(1),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WhatsAppOutputReport {
    pub sent: u32,
    pub deferred: u32,
    pub transient_failures: u32,
    pub permanent_failures: u32,
    pub exhausted: u32,
}

struct OutputChannel {
    org: OrgId,
    integration: IntegrationId,
    channel: Option<Arc<WhatsAppEvolution>>,
}

pub struct WhatsAppOutput {
    store: Arc<Store>,
    channels: Vec<OutputChannel>,
    options: WhatsAppOutputOptions,
    schedule: Mutex<HashMap<OutboxId, Instant>>,
}

impl WhatsAppOutput {
    pub fn new(
        store: Arc<Store>,
        channels: &[EvolutionChannel],
        blob_folder: PathBuf,
        options: WhatsAppOutputOptions,
    ) -> Self {
        let attachment_source: Arc<dyn AttachmentSource> = Arc::new(DiskBlobs::new(blob_folder));
        let channels = channels
            .iter()
            .map(|instance| OutputChannel {
                org: instance.org,
                integration: instance.integration,
                channel: instance.client.as_ref().map(|client| {
                    Arc::new(WhatsAppEvolution::for_output(
                        client.clone(),
                        instance.integration,
                        Arc::clone(&attachment_source),
                    ))
                }),
            })
            .collect();
        Self {
            store,
            channels,
            options,
            schedule: Mutex::new(HashMap::new()),
        }
    }

    fn orgs(&self) -> Vec<OrgId> {
        let mut orgs: Vec<OrgId> = Vec::new();
        for channel in &self.channels {
            if !orgs.contains(&channel.org) {
                orgs.push(channel.org);
            }
        }
        orgs
    }

    fn channel_for(&self, org: OrgId, integration: IntegrationId) -> Option<&OutputChannel> {
        self.channels
            .iter()
            .find(|channel| channel.org == org && channel.integration == integration)
    }

    fn waiting(&self, id: OutboxId) -> bool {
        let now = Instant::now();
        self.schedule
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&id)
            .is_some_and(|next| now < *next)
    }

    fn schedule_retry(&self, id: OutboxId, delay: Duration) {
        self.schedule
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(id, Instant::now() + delay);
    }

    fn clear_retry(&self, id: OutboxId) {
        self.schedule
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&id);
    }

    pub async fn cycle(&self) -> Result<WhatsAppOutputReport, StoreError> {
        let mut report = WhatsAppOutputReport::default();
        for org in self.orgs() {
            let items = self
                .store
                .outbox()
                .list_pending(org, self.options.batch_size)?;
            for item in items {
                let id = item.id;
                if let Err(error) = self.process(org, item, &mut report).await {
                    tracing::warn!(
                        target: "poph_server::saida_whatsapp",
                        outbox_id = %id,
                        reason = %error,
                        "item de saída do WhatsApp não pôde ser atualizado; segue no próximo ciclo"
                    );
                }
            }
        }
        Ok(report)
    }

    async fn process(
        &self,
        org: OrgId,
        item: OutboxItem,
        report: &mut WhatsAppOutputReport,
    ) -> Result<(), StoreError> {
        let conversation = self.store.conversations().get(org, item.conversation_id)?;
        let external_peer = conversation.peer_externo.clone();
        let Some(integration) = conversation.integration_id else {
            return Ok(());
        };
        let Some(instance) = self.channel_for(org, integration) else {
            return Ok(());
        };
        let Some(channel) = instance.channel.clone() else {
            report.deferred += 1;
            return Ok(());
        };

        let id = item.id;
        let conversation_id = item.conversation_id;
        let kind = kind_label(&item.tipo);
        if self.waiting(id) {
            report.deferred += 1;
            return Ok(());
        }
        if item.attempts >= self.options.max_attempts {
            self.store
                .outbox()
                .fail_send(org, id, RETRY_LIMIT_REASON, true)?;
            self.clear_retry(id);
            report.exhausted += 1;
            tracing::warn!(
                target: "poph_server::saida_whatsapp",
                outbox_id = %id,
                conversation_id = %conversation_id,
                kind,
                "envio do WhatsApp desistido: tentativas esgotadas"
            );
            return Ok(());
        }

        let attempt = self.store.outbox().start_send(org, id)?;
        let Some(peer) = external_peer else {
            self.store.outbox().fail_send(
                org,
                id,
                "a conversa não tem identificador do cliente no canal",
                true,
            )?;
            self.clear_retry(id);
            report.permanent_failures += 1;
            tracing::warn!(
                target: "poph_server::saida_whatsapp",
                outbox_id = %id,
                conversation_id = %conversation_id,
                kind,
                "item de saída do WhatsApp sem destino no canal: não será repetido"
            );
            return Ok(());
        };
        let command = match command_item(&item, &peer, &chave_idempotencia(&item)) {
            Ok(command) => command,
            Err(failure) => {
                self.store
                    .outbox()
                    .fail_send(org, id, &failure.motivo, true)?;
                self.clear_retry(id);
                report.permanent_failures += 1;
                tracing::warn!(
                    target: "poph_server::saida_whatsapp",
                    outbox_id = %id,
                    conversation_id = %conversation_id,
                    kind,
                    reason = %failure.motivo,
                    "item de saída do WhatsApp não pôde ser montado: não será repetido"
                );
                return Ok(());
            }
        };
        match channel.send_with_receipt(command).await {
            Ok(external_id) => {
                self.store
                    .outbox()
                    .confirm_send_with_id(org, id, external_id.as_deref())?;
                self.clear_retry(id);
                report.sent += 1;
                tracing::info!(
                    target: "poph_server::saida_whatsapp",
                    outbox_id = %id,
                    conversation_id = %conversation_id,
                    kind,
                    has_message_id = external_id.is_some(),
                    "item de saída do WhatsApp enviado"
                );
            }
            Err(error) if error.is_transient() => {
                let failure = SendFailure::transient(error.to_string());
                self.store
                    .outbox()
                    .fail_send(org, id, &failure.motivo, false)?;
                self.schedule_retry(id, self.options.backoff.wait(attempt));
                report.transient_failures += 1;
                tracing::warn!(
                    target: "poph_server::saida_whatsapp",
                    outbox_id = %id,
                    conversation_id = %conversation_id,
                    kind,
                    "tentativa" = attempt,
                    reason = %failure.motivo,
                    "envio do WhatsApp adiado: falha transitória, nova tentativa depois"
                );
            }
            Err(error) => {
                let failure = SendFailure::definitive(error.to_string());
                self.store
                    .outbox()
                    .fail_send(org, id, &failure.motivo, true)?;
                self.clear_retry(id);
                report.permanent_failures += 1;
                tracing::warn!(
                    target: "poph_server::saida_whatsapp",
                    outbox_id = %id,
                    conversation_id = %conversation_id,
                    kind,
                    "tentativa" = attempt,
                    reason = %failure.motivo,
                    "envio do WhatsApp falhou de vez: não será repetido"
                );
            }
        }
        Ok(())
    }
}

pub const RETRY_LIMIT_REASON: &str = "tentativas de envio esgotadas";

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "text" => "texto",
        "file" => "mídia",
        "presence" => "presença",
        _ => "outro",
    }
}

pub fn start(state: &ServerState) -> Option<tokio::task::JoinHandle<()>> {
    if state.evolution_channels.is_empty() {
        return None;
    }
    let output = Arc::new(WhatsAppOutput::new(
        Arc::clone(&state.store),
        &state.evolution_channels,
        state.config.blobs(),
        WhatsAppOutputOptions::default(),
    ));
    let interval = output.options.interval;
    Some(tokio::spawn(async move {
        loop {
            if let Err(error) = output.cycle().await {
                tracing::warn!(
                    target: "poph_server::saida_whatsapp",
                    reason = %error,
                    "ciclo de saída do WhatsApp falhou; segue no próximo"
                );
            }
            tokio::time::sleep(interval).await;
        }
    }))
}
