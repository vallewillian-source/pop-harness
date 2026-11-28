use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::Backoff;
use poph_evolution::{AttachmentSource, DiskBlobs, WhatsAppEvolution};
use poph_store::{FlowEffect, NewOutboxItem, Store, StoreError};
use poph_types::{EventActor, IntegrationId, OrgId, RecordEventKind, TurnId};
use serde_json::Value;

use crate::evolution::EvolutionChannel;
use crate::flow_metrics::FlowMetrics;
use crate::http_effects::{
    CALENDAR_KIND, CalendarContext, HttpEffect, HttpEffectRegistry, calendar_body,
};
use crate::state::ServerState;

#[derive(Debug, Clone)]
pub struct FlowEffectOptions {
    pub batch_size: u32,
    pub max_attempts: u32,
    pub backoff: Backoff,
    pub interval: Duration,
    pub missing_integration_delay: Duration,
}

impl Default for FlowEffectOptions {
    fn default() -> Self {
        Self {
            batch_size: 100,
            max_attempts: 8,
            backoff: Backoff::default(),
            interval: Duration::from_secs(1),
            missing_integration_delay: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FlowEffectReport {
    pub sent: u32,
    pub deferred: u32,
    pub transient_failures: u32,
    pub permanent_failures: u32,
    pub exhausted: u32,
}

struct NotificationChannel {
    org: OrgId,
    integration: IntegrationId,
    channel: Option<Arc<WhatsAppEvolution>>,
}

pub struct FlowEffects {
    store: Arc<Store>,
    channels: Vec<NotificationChannel>,
    metrics: Option<Arc<FlowMetrics>>,
    options: FlowEffectOptions,
    http_effects: Arc<HttpEffectRegistry>,
}

enum Plan {
    Deliver(Delivery),
    Wait(&'static str),
    Fail(String),
}

enum Delivery {
    CustomerText {
        conversation_id: poph_types::ConversationId,
        text: String,
    },
    Whatsapp {
        channel: Arc<WhatsAppEvolution>,
        para: String,
        message: String,
    },
    Transfer {
        conversation_id: poph_types::ConversationId,
        department: poph_types::DepartmentId,
        name: String,
    },
    Http {
        endpoint: Arc<HttpEffect>,
        key: String,
        body: Vec<u8>,
    },
}

impl FlowEffects {
    pub fn new(
        store: Arc<Store>,
        channels: &[EvolutionChannel],
        blob_folder: PathBuf,
        metrics: Option<Arc<FlowMetrics>>,
        options: FlowEffectOptions,
    ) -> Self {
        let attachment_source: Arc<dyn AttachmentSource> = Arc::new(DiskBlobs::new(blob_folder));
        let channels = channels
            .iter()
            .map(|instance| NotificationChannel {
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
            metrics,
            options,
            http_effects: Arc::default(),
        }
    }

    pub fn with_http_effects(mut self, registry: Arc<HttpEffectRegistry>) -> Self {
        self.http_effects = registry;
        self
    }

    fn channel_for(&self, org: OrgId, integration: IntegrationId) -> Option<&NotificationChannel> {
        self.channels
            .iter()
            .find(|channel| channel.org == org && channel.integration == integration)
    }

    fn increment(&self, counter: &dyn Fn(&FlowMetrics) -> &poph_types::Counter) {
        if let Some(metrics) = self.metrics.as_ref() {
            counter(metrics).increment(1);
        }
    }

    pub async fn cycle(&self) -> Result<Vec<FlowEffectReport>, StoreError> {
        let orgs = self.store.effects().orgs_with_pending()?;
        let mut reports = Vec::with_capacity(orgs.len());
        for org in orgs {
            reports.push(self.cycle_for_org(org).await?);
        }
        Ok(reports)
    }

    pub async fn cycle_for_org(&self, org: OrgId) -> Result<FlowEffectReport, StoreError> {
        let mut report = FlowEffectReport::default();
        let effects = self
            .store
            .effects()
            .list_pending(org, self.options.batch_size)?;
        for effect in effects {
            let id = effect.id;
            if let Err(error) = self.process(org, effect, &mut report).await {
                tracing::warn!(
                    target: "poph_server::fluxos_efeitos",
                    effect_id = %id,
                    reason = %error,
                    "efeito de fluxo não pôde ser atualizado; segue no próximo ciclo"
                );
            }
        }
        Ok(report)
    }

    async fn process(
        &self,
        org: OrgId,
        effect: FlowEffect,
        report: &mut FlowEffectReport,
    ) -> Result<(), StoreError> {
        let id = effect.id;
        if effect.state == poph_types::EffectState::Pending
            && effect.attempts == 0
            && effect.proxima_tentativa.is_none()
        {
            self.increment(&|metrics| &metrics.pending_effects);
        }
        if effect.attempts >= self.options.max_attempts {
            self.fail(org, &effect, RETRY_LIMIT_REASON, true, report)
                .await?;
            return Ok(());
        }

        let plan = self.plan(org, &effect)?;
        match plan {
            Plan::Wait(reason) => {
                let delay = self.missing_integration_delay();
                self.store
                    .effects()
                    .adiar(org, id, self.store.now() + delay)?;
                report.deferred += 1;
                tracing::debug!(
                    target: "poph_server::fluxos_efeitos",
                    effect_id = %id,
                    effect_type = %effect.tipo,
                    reason,
                    delay_ms = u64::try_from(delay).unwrap_or(u64::MAX),
                    "efeito de fluxo esperando: nada a fazer agora"
                );
            }
            Plan::Fail(reason) => {
                self.fail(org, &effect, &reason, false, report).await?;
            }
            Plan::Deliver(delivery) => {
                let attempt = self.store.effects().start_send(org, id)?;
                match self.deliver(org, &effect, delivery).await {
                    Ok(department) => {
                        self.store.effects().confirm_send(org, id)?;
                        self.increment(&|metrics| &metrics.sent_effects);
                        self.record_event(
                            org,
                            &effect,
                            RecordEventKind::EffectSent,
                            "enviado",
                            attempt,
                        );
                        if let Some(department) = department {
                            self.record_event(
                                org,
                                &effect,
                                RecordEventKind::ServiceTransferred,
                                &department,
                                attempt,
                            );
                        }
                        report.sent += 1;
                        tracing::info!(
                            target: "poph_server::fluxos_efeitos",
                            effect_id = %id,
                            effect_type = %effect.tipo,
                            attempt,
                            "efeito de fluxo entregue"
                        );
                    }
                    Err(DeliveryFailure::Transient(reason)) => {
                        let delay = self.options.backoff.wait(attempt);
                        self.store.effects().fail_send(
                            org,
                            id,
                            &reason,
                            false,
                            Some(self.store.now() + delay_ms(delay)),
                        )?;
                        report.transient_failures += 1;
                        tracing::warn!(
                            target: "poph_server::fluxos_efeitos",
                            effect_id = %id,
                            effect_type = %effect.tipo,
                            attempt,
                            reason = %reason,
                            "efeito de fluxo adiado: falha transitória, nova tentativa depois"
                        );
                    }
                    Err(DeliveryFailure::Permanent(reason)) => {
                        self.fail(org, &effect, &reason, false, report).await?;
                    }
                }
            }
        }
        Ok(())
    }

    fn plan(&self, org: OrgId, effect: &FlowEffect) -> Result<Plan, StoreError> {
        let payload = match serde_json::from_str::<Value>(&effect.payload_json) {
            Ok(payload) => payload,
            Err(error) => {
                return Ok(Plan::Fail(format!(
                    "o efeito está gravado com JSON inválido: {error}"
                )));
            }
        };
        let text = |field: &str| {
            payload
                .get(field)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        match effect.tipo.as_str() {
            "mensagem_ao_cliente" => {
                let Some(text) = text("texto") else {
                    return Ok(Plan::Fail(
                        "mensagem ao cliente sem `texto` no efeito".to_owned(),
                    ));
                };
                let case = self.store.cases().get(org, effect.ficha_id)?;
                let Some(conversation_id) = case.conversation_id else {
                    return Ok(Plan::Fail(
                        "a Ficha do efeito não tem conversa (apagada pela retenção)".to_owned(),
                    ));
                };
                let current = self.store.conversations().get(org, conversation_id)?;
                if current.status != poph_store::ConversationStatus::Open {
                    return Ok(Plan::Fail(format!(
                        "a conversa está {} e não recebe mensagem do fluxo",
                        conversation_status_label(current.status)
                    )));
                }
                if self.store.turns().in_progress(org, conversation_id)? {
                    return Ok(Plan::Wait("o turno da conversa ainda está em andamento"));
                }
                Ok(Plan::Deliver(Delivery::CustomerText {
                    conversation_id,
                    text,
                }))
            }
            "notificar_whatsapp" => {
                let (Some(para), Some(message)) = (text("para"), text("mensagem")) else {
                    return Ok(Plan::Fail(
                        "notificação sem `para` ou sem `mensagem` no efeito".to_owned(),
                    ));
                };
                let case = self.store.cases().get(org, effect.ficha_id)?;
                let Some(conversation_id) = case.conversation_id else {
                    return Ok(Plan::Fail(
                        "a Ficha do efeito não tem conversa: não há integração para notificar"
                            .to_owned(),
                    ));
                };
                let current = self.store.conversations().get(org, conversation_id)?;
                let Some(integration) = current.integration_id else {
                    return Ok(Plan::Wait(
                        "a conversa não tem integração de WhatsApp (console ou CLI)",
                    ));
                };
                let Some(channel) = self.channel_for(org, integration) else {
                    return Ok(Plan::Wait(
                        "a integração da conversa não está configurada nesta instância",
                    ));
                };
                let Some(channel) = channel.channel.as_ref().map(Arc::clone) else {
                    return Ok(Plan::Wait("a chave da Evolution não está no ambiente"));
                };
                Ok(Plan::Deliver(Delivery::Whatsapp {
                    channel,
                    para,
                    message,
                }))
            }
            "encaminhar" => {
                let Some(name) = text("departamento") else {
                    return Ok(Plan::Fail(
                        "encaminhamento sem `departamento` no efeito".to_owned(),
                    ));
                };
                let case = self.store.cases().get(org, effect.ficha_id)?;
                let Some(conversation_id) = case.conversation_id else {
                    return Ok(Plan::Fail(
                        "a Ficha do efeito não tem conversa: não há o que encaminhar".to_owned(),
                    ));
                };
                let Some(department) = self.store.departments().by_name(org, &name)? else {
                    return Ok(Plan::Fail(format!(
                        "o departamento `{name}` não existe na organização"
                    )));
                };
                Ok(Plan::Deliver(Delivery::Transfer {
                    conversation_id,
                    department: department.id,
                    name: department.name,
                }))
            }
            CALENDAR_KIND => {
                let Some(endpoint) = self.http_effects.get(org, CALENDAR_KIND) else {
                    return Ok(Plan::Wait(
                        "a agenda não tem endereço configurado para esta organização",
                    ));
                };
                if !endpoint.is_authorized() {
                    return Ok(Plan::Wait("a credencial da agenda não está no ambiente"));
                }
                let execution = self.store.executions().get(org, effect.execucao_id)?;
                let context = CalendarContext {
                    org,
                    ficha: effect.ficha_id,
                    execution: effect.execucao_id,
                    effect: effect.id,
                    key: &effect.key,
                    flow: execution.fluxo_id,
                };
                match calendar_body(&context, &payload) {
                    Ok(body) => Ok(Plan::Deliver(Delivery::Http {
                        endpoint,
                        key: effect.id.to_string(),
                        body,
                    })),
                    Err(reason) => Ok(Plan::Fail(reason)),
                }
            }
            other => Ok(Plan::Fail(format!(
                "tipo de efeito desconhecido na fila: `{other}`"
            ))),
        }
    }

    async fn deliver(
        &self,
        org: OrgId,
        effect: &FlowEffect,
        delivery: Delivery,
    ) -> Result<Option<String>, DeliveryFailure> {
        match delivery {
            Delivery::CustomerText {
                conversation_id,
                text,
            } => {
                let new = NewOutboxItem {
                    conversation_id,
                    turn_id: TurnId::generate(),
                    seq: 0,
                    tipo: "text".to_owned(),
                    content_json: serde_json::json!({ "texto": text }).to_string(),
                };
                self.store.outbox().enqueue(org, new).map_err(|error| {
                    DeliveryFailure::Transient(format!(
                        "não consegui enfileirar a mensagem ao cliente: {error}"
                    ))
                })?;
                Ok(None)
            }
            Delivery::Whatsapp {
                channel,
                para,
                message,
            } => {
                let command = poph_types::SendText {
                    idempotency: format!("fluxo-efeito-{}", effect.id),
                    peer: para,
                    text: message,
                };
                match channel.send_text_receipt(command).await {
                    Ok(_) => Ok(None),
                    Err(error) if error.is_transient() => {
                        Err(DeliveryFailure::Transient(error.to_string()))
                    }
                    Err(error) => Err(DeliveryFailure::Permanent(error.to_string())),
                }
            }
            Delivery::Transfer {
                conversation_id,
                department,
                name,
            } => {
                let detail = serde_json::json!({
                    "origem": "fluxo",
                    "departamento": name,
                    "efeito": effect.id.to_string(),
                })
                .to_string();
                self.store
                    .conversations()
                    .transfer(
                        org,
                        conversation_id,
                        Some(department),
                        Some("fluxo".to_owned()),
                        detail,
                    )
                    .map_err(|error| {
                        DeliveryFailure::Transient(format!(
                            "não consegui transferir a conversa: {error}"
                        ))
                    })?;
                Ok(Some(name))
            }
            Delivery::Http {
                endpoint,
                key,
                body,
            } => endpoint.post(&key, body).await.map(|()| None),
        }
    }

    async fn fail(
        &self,
        org: OrgId,
        effect: &FlowEffect,
        reason: &str,
        exhausted: bool,
        report: &mut FlowEffectReport,
    ) -> Result<(), StoreError> {
        self.store
            .effects()
            .fail_send(org, effect.id, reason, true, None)?;
        self.increment(&|metrics| &metrics.failed_effects);
        if exhausted {
            report.exhausted += 1;
        } else {
            report.permanent_failures += 1;
        }
        self.record_event(
            org,
            effect,
            RecordEventKind::EffectFailed,
            reason,
            effect.attempts,
        );
        tracing::warn!(
            target: "poph_server::fluxos_efeitos",
            effect_id = %effect.id,
            effect_type = %effect.tipo,
            attempts = effect.attempts,
            reason,
            "efeito de fluxo falhou de vez: a transição continua valendo"
        );
        Ok(())
    }

    fn record_event(
        &self,
        org: OrgId,
        effect: &FlowEffect,
        kind: RecordEventKind,
        detail: &str,
        attempt: u32,
    ) {
        let idempotency_key = format!("efeito:{}:{}", effect.id, kind.as_text());
        let payload = serde_json::json!({
            "efeito": effect.id.to_string(),
            "chave": effect.key,
            "tipo": effect.tipo,
            "tentativa": attempt,
            "detalhe": detail,
        })
        .to_string();
        let new = poph_store::NewRecordEvent {
            execucao_id: Some(effect.execucao_id),
            tipo: kind,
            ator_tipo: EventActor::System,
            ator_ref: Some("fluxo"),
            payload_json: payload,
            chave_idempotencia: Some(&idempotency_key),
        };
        if let Err(error) = self.store.cases().register_event(org, effect.ficha_id, new) {
            tracing::warn!(
                target: "poph_server::fluxos_efeitos",
                effect_id = %effect.id,
                effect_type = kind.as_text(),
                reason = %error,
                "não consegui registrar o desfecho do efeito na Ficha"
            );
        }
    }

    fn missing_integration_delay(&self) -> i64 {
        delay_ms(self.options.missing_integration_delay)
    }
}

pub(crate) enum DeliveryFailure {
    Transient(String),
    Permanent(String),
}

pub const RETRY_LIMIT_REASON: &str = "tentativas de entrega do efeito esgotadas";

fn conversation_status_label(status: poph_store::ConversationStatus) -> &'static str {
    match status {
        poph_store::ConversationStatus::Open => "aberta",
        poph_store::ConversationStatus::Transferred => "transferida para um atendente",
        poph_store::ConversationStatus::Closed => "encerrada",
    }
}

fn delay_ms(delay: Duration) -> i64 {
    i64::try_from(delay.as_millis()).unwrap_or(i64::MAX)
}

pub use poph_agent::flows_transition::effect_of_transition;

pub fn start(state: &ServerState) -> Option<tokio::task::JoinHandle<()>> {
    if !state.config.flows.enabled {
        return None;
    }
    let effects = Arc::new(
        FlowEffects::new(
            Arc::clone(&state.store),
            &state.evolution_channels,
            state.config.blobs(),
            state.flow_metrics(),
            FlowEffectOptions::default(),
        )
        .with_http_effects(Arc::clone(&state.http_effects)),
    );
    let interval = effects.options.interval;
    Some(tokio::spawn(async move {
        loop {
            if let Err(error) = effects.cycle().await {
                tracing::warn!(
                    target: "poph_server::fluxos_efeitos",
                    reason = %error,
                    "ciclo da fila de efeitos falhou; segue no próximo"
                );
            }
            tokio::time::sleep(interval).await;
        }
    }))
}
