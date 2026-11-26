use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use poph_store::{
    NewInboxItem, NewInboxItemWithMessage, OutboxItem, OutboxState, Page, Received,
    ReceivedWithMessage, Store,
};
use poph_types::{ConversationId, InboxId, IntegrationId, MessageId, OrgId, OutboxId};

use crate::actors::{Actors, Task};
use crate::error::{AgentError, Resultado};
use crate::metrics::MetricsFlow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InputEvent {
    pub conversa: ConversationId,
    pub mensagem: MessageId,
}

impl InputEvent {
    pub fn for_json(self) -> Resultado<String> {
        serde_json::to_string(&self).map_err(|error| {
            AgentError::Internal(format!(
                "não consegui serializar o evento de entrada: {error}"
            ))
        })
    }

    pub fn of_json(payload: &str) -> Resultado<Self> {
        serde_json::from_str(payload).map_err(|error| AgentError::EventInvalid {
            motivo: error.to_string(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ReceivedMessage {
    pub integration_id: IntegrationId,
    pub id_externo: String,
    pub conversa: ConversationId,
    pub texto: String,
    pub attachments_json: String,
}

pub type SendFuture = Pin<Box<dyn Future<Output = Result<(), SendFailure>> + Send + 'static>>;

const BATCH_MAXIMUM_DO_CONSOLE: u32 = 1000;

pub trait OutputTransport: Send + Sync + 'static {
    fn send(&self, item: OutboxItem) -> SendFuture;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendFailure {
    pub motivo: String,
    pub transient: bool,
}

impl SendFailure {
    pub fn transient(motivo: impl Into<String>) -> Self {
        Self {
            motivo: motivo.into(),
            transient: true,
        }
    }

    pub fn definitive(motivo: impl Into<String>) -> Self {
        Self {
            motivo: motivo.into(),
            transient: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    pub base: Duration,
    pub factor: u32,
    pub cap: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            base: Duration::from_millis(500),
            factor: 2,
            cap: Duration::from_secs(60),
        }
    }
}

impl Backoff {
    pub fn wait(&self, attempt: u32) -> Duration {
        let mut wait = self.base;
        for _ in 1..attempt.max(1) {
            if wait >= self.cap {
                break;
            }
            wait = wait.saturating_mul(self.factor.max(1));
        }
        wait.min(self.cap)
    }
}

#[derive(Debug, Clone)]
pub struct FlowOptions {
    pub batch_entry: u32,
    pub batch_output: u32,
    pub max_attempts: u32,
    pub backoff: Backoff,
    pub interval_cycle: Duration,
    pub tentativas_falha_modelo: u32,
    pub delay_failure_model: Backoff,
}

impl FlowOptions {
    pub fn with_failure_model(mut self, tentativas: u32, delay_base: Duration) -> Self {
        self.tentativas_falha_modelo = tentativas;
        self.delay_failure_model = Backoff {
            base: delay_base,
            factor: 4,
            cap: Duration::from_secs(600),
        };
        self
    }
}

impl Default for FlowOptions {
    fn default() -> Self {
        Self {
            batch_entry: 200,
            batch_output: 100,
            max_attempts: 8,
            backoff: Backoff::default(),
            interval_cycle: Duration::from_millis(250),
            tentativas_falha_modelo: 3,
            delay_failure_model: Backoff {
                base: Duration::from_secs(30),
                factor: 4,
                cap: Duration::from_secs(600),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DispatchReport {
    pub dispatch: u32,
    pub deferred: u32,
    pub invalid: u32,
    pub desistentes: u32,
}

impl DispatchReport {
    pub fn sum(&mut self, other: Self) {
        self.dispatch += other.dispatch;
        self.deferred += other.deferred;
        self.invalid += other.invalid;
        self.desistentes += other.desistentes;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SendReport {
    pub sent: u32,
    pub deferred: u32,
    pub failed: u32,
    pub exhausted: u32,
}

impl SendReport {
    pub fn sum(&mut self, other: Self) {
        self.sent += other.sent;
        self.deferred += other.deferred;
        self.failed += other.failed;
        self.exhausted += other.exhausted;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResumeReport {
    pub entry: DispatchReport,
    pub saida: SendReport,
    pub resends: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Statistics {
    pub entry_pending: u64,
    pub output_sending: u64,
    pub output_pending: u64,
    pub output_failed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SendOutcome {
    Deferred,
    Sent,
    Failed,
    Exhausted,
}

#[derive(Debug, Clone, Copy)]
struct Schedule {
    next_in: Instant,
    attempt: u32,
}

type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

struct Internal {
    store: Arc<Store>,
    actors: Actors,
    options: FlowOptions,
    metrics: MetricsFlow,
    clock: Mutex<Clock>,
    em_voo: Mutex<HashSet<InboxId>>,
    in_send: Mutex<HashSet<OutboxId>>,
    schedule_entry: Mutex<HashMap<InboxId, Schedule>>,
    schedule_output: Mutex<HashMap<OutboxId, Schedule>>,
}

#[derive(Clone)]
pub struct Fluxo {
    internal: Arc<Internal>,
}

impl Fluxo {
    pub fn new(store: Arc<Store>, actors: Actors, options: FlowOptions) -> Self {
        Self::with_metrics(store, actors, options, MetricsFlow::own())
    }

    pub fn with_metrics(
        store: Arc<Store>,
        actors: Actors,
        options: FlowOptions,
        metrics: MetricsFlow,
    ) -> Self {
        Self {
            internal: Arc::new(Internal {
                store,
                actors,
                options,
                metrics,
                clock: Mutex::new(Arc::new(Instant::now)),
                em_voo: Mutex::new(HashSet::new()),
                in_send: Mutex::new(HashSet::new()),
                schedule_entry: Mutex::new(HashMap::new()),
                schedule_output: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn with_clock(&self, clock: Clock) {
        *self
            .internal
            .clock
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = clock;
    }

    pub fn metrics(&self) -> &MetricsFlow {
        &self.internal.metrics
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.internal.store
    }

    pub fn receive(&self, org: OrgId, new: NewInboxItem) -> Resultado<Received> {
        let (situacao, _item) = self.internal.store.inbox().receive(org, new)?;
        match situacao {
            Received::New => self.internal.metrics.entries_new.increment(1),
            Received::Duplicate => self.internal.metrics.entries_duplicated.increment(1),
        }
        Ok(situacao)
    }

    pub fn receive_message(
        &self,
        org: OrgId,
        received: ReceivedMessage,
    ) -> Resultado<ReceivedWithMessage> {
        let mensagem = MessageId::generate();
        let evento = InputEvent {
            conversa: received.conversa,
            mensagem,
        }
        .for_json()?;
        let stored = self.internal.store.inbox().receive_with_message(
            org,
            NewInboxItemWithMessage {
                integration_id: received.integration_id,
                external_id: received.id_externo,
                event_json: evento,
                conversation_id: received.conversa,
                message_id: mensagem,
                texto: received.texto,
                attachments_json: received.attachments_json,
            },
        )?;
        match stored.situation {
            Received::New => self.internal.metrics.entries_new.increment(1),
            Received::Duplicate => self.internal.metrics.entries_duplicated.increment(1),
        }
        Ok(stored)
    }

    pub fn concluir_entry(&self, org: OrgId, id: InboxId, error: Option<&str>) -> Resultado<()> {
        self.internal.store.inbox().mark_processed(org, id, error)?;
        self.internal.em_voo().remove(&id);
        self.internal.schedule_entry().remove(&id);
        self.internal.metrics.entries_finished.increment(1);
        Ok(())
    }

    pub fn options(&self) -> &FlowOptions {
        &self.internal.options
    }

    pub fn adiar_entries(
        &self,
        org: OrgId,
        ids: &[InboxId],
        next_in: i64,
        error: &str,
    ) -> Resultado<()> {
        self.internal
            .store
            .inbox()
            .adiar(org, ids, next_in, error)?;
        for id in ids {
            self.internal.release_entry(*id);
        }
        Ok(())
    }

    pub fn devolver_entry(&self, id: InboxId) {
        self.internal.release_entry(id);
    }

    pub fn dispatch(&self, org: OrgId) -> Resultado<DispatchReport> {
        let items = self
            .internal
            .store
            .inbox()
            .list_pending(org, self.internal.options.batch_entry)?;
        self.dispatch_items(org, items)
    }

    pub fn dispatch_items(
        &self,
        org: OrgId,
        items: Vec<poph_store::InboxItem>,
    ) -> Resultado<DispatchReport> {
        let now = self.internal.now();
        let mut report = DispatchReport::default();
        for item in items {
            let id = item.id;
            if !self.internal.reivindicar_entry(id) {
                report.deferred += 1;
                continue;
            }
            match self.internal.store.inbox().get(org, id) {
                Ok(current) if current.processed_at.is_some() => {
                    self.internal.schedule_entry().remove(&id);
                    self.internal.release_entry(id);
                    continue;
                }
                Ok(_) => {}
                Err(error) => {
                    self.internal.release_entry(id);
                    return Err(error.into());
                }
            }
            if let Some(schedule) = self.internal.schedule_entry().get(&id)
                && now < schedule.next_in
            {
                self.internal.release_entry(id);
                report.deferred += 1;
                continue;
            }

            let evento = match InputEvent::of_json(&item.event_json) {
                Ok(evento) => evento,
                Err(error) => {
                    let mark = self.internal.store.inbox().mark_processed(
                        org,
                        id,
                        Some(&format!("evento de entrada inválido: {error}")),
                    );
                    self.internal.schedule_entry().remove(&id);
                    self.internal.release_entry(id);
                    mark?;
                    self.internal.metrics.entries_invalid.increment(1);
                    report.invalid += 1;
                    continue;
                }
            };

            let task = Task::of_entry(org, evento.conversa, evento.mensagem, id);
            match self.internal.actors.send(task) {
                Ok(()) => {
                    self.internal.schedule_entry().remove(&id);
                    report.dispatch += 1;
                }
                Err(error) => {
                    let attempt = self
                        .internal
                        .schedule_entry()
                        .get(&id)
                        .map_or(1, |schedule| schedule.attempt.saturating_add(1));
                    self.internal.release_entry(id);
                    if attempt >= self.internal.options.max_attempts {
                        let mark = self.internal.store.inbox().mark_processed(
                            org,
                            id,
                            Some(&format!("despacho desistiu: {error}")),
                        );
                        self.internal.schedule_entry().remove(&id);
                        mark?;
                        self.internal.metrics.entries_desistentes.increment(1);
                        report.desistentes += 1;
                    } else {
                        let next_in = now + self.internal.options.backoff.wait(attempt);
                        self.internal
                            .schedule_entry()
                            .insert(id, Schedule { next_in, attempt });
                        self.internal.metrics.entries_deferred.increment(1);
                        report.deferred += 1;
                    }
                }
            }
        }
        Ok(report)
    }

    pub fn dispatch_all(&self) -> Resultado<DispatchReport> {
        let mut total = DispatchReport::default();
        for org in self.orgs()? {
            total.sum(self.dispatch(org)?);
        }
        Ok(total)
    }

    pub fn enqueue_output(
        &self,
        org: OrgId,
        new: poph_store::NewOutboxItem,
    ) -> Resultado<(bool, OutboxItem)> {
        Ok(self.internal.store.outbox().enqueue(org, new)?)
    }

    pub async fn send_pending(
        &self,
        org: OrgId,
        transport: &dyn OutputTransport,
    ) -> Resultado<SendReport> {
        let items = self
            .internal
            .store
            .outbox()
            .list_pending(org, self.internal.options.batch_output)?;
        self.send_items(org, items, transport).await
    }

    pub async fn send_pending_console(
        &self,
        org: OrgId,
        transport: &dyn OutputTransport,
    ) -> Resultado<SendReport> {
        let pending = self
            .internal
            .store
            .outbox()
            .list_pending(org, BATCH_MAXIMUM_DO_CONSOLE)?;
        let mut items = Vec::new();
        for item in pending {
            let conversa = self
                .internal
                .store
                .conversations()
                .get(org, item.conversation_id)?;
            if conversa.integration_id.is_none() {
                items.push(item);
            }
        }
        self.send_items(org, items, transport).await
    }

    async fn send_items(
        &self,
        org: OrgId,
        items: Vec<OutboxItem>,
        transport: &dyn OutputTransport,
    ) -> Resultado<SendReport> {
        let now = self.internal.now();
        let mut report = SendReport::default();
        for item in items {
            let id = item.id;
            if !self.internal.reivindicar_output(id) {
                report.deferred += 1;
                continue;
            }
            let desfecho = self.send(org, item, transport, now).await;
            self.internal.release_output(id);
            match desfecho? {
                SendOutcome::Deferred => report.deferred += 1,
                SendOutcome::Sent => report.sent += 1,
                SendOutcome::Failed => report.failed += 1,
                SendOutcome::Exhausted => report.exhausted += 1,
            }
        }
        Ok(report)
    }

    async fn send(
        &self,
        org: OrgId,
        item: OutboxItem,
        transport: &dyn OutputTransport,
        now: Instant,
    ) -> Resultado<SendOutcome> {
        let id = item.id;
        if let Some(schedule) = self.internal.schedule_output().get(&id)
            && now < schedule.next_in
        {
            return Ok(SendOutcome::Deferred);
        }
        if item.attempts >= self.internal.options.max_attempts {
            self.internal.store.outbox().fail_send(
                org,
                id,
                "tentativas de envio esgotadas",
                true,
            )?;
            self.internal.schedule_output().remove(&id);
            self.internal.metrics.outputs_exhausted.increment(1);
            return Ok(SendOutcome::Exhausted);
        }

        let attempt = match self.internal.store.outbox().start_send(org, id) {
            Ok(attempt) => attempt,
            Err(error) => {
                tracing::warn!(
                    org = %org,
                    saida = %id,
                    error = %error,
                    "item de saída não pôde entrar em sending; fica para o próximo ciclo"
                );
                return Ok(SendOutcome::Deferred);
            }
        };

        match transport.send(item).await {
            Ok(()) => {
                self.internal.store.outbox().confirm_send(org, id)?;
                self.internal.schedule_output().remove(&id);
                self.internal.metrics.outputs_sent.increment(1);
                Ok(SendOutcome::Sent)
            }
            Err(failure) if failure.transient => {
                self.internal
                    .store
                    .outbox()
                    .fail_send(org, id, &failure.motivo, false)?;
                let next_in = now + self.internal.options.backoff.wait(attempt);
                self.internal
                    .schedule_output()
                    .insert(id, Schedule { next_in, attempt });
                self.internal.metrics.outputs_transient.increment(1);
                Ok(SendOutcome::Failed)
            }
            Err(failure) => {
                self.internal
                    .store
                    .outbox()
                    .fail_send(org, id, &failure.motivo, true)?;
                self.internal.schedule_output().remove(&id);
                self.internal.metrics.outputs_definitive.increment(1);
                Ok(SendOutcome::Failed)
            }
        }
    }

    pub async fn retomar(&self, transport: &dyn OutputTransport) -> Resultado<ResumeReport> {
        let orgs = self.orgs()?;
        let mut resends = 0u32;
        for org in &orgs {
            resends += u32::try_from(
                self.internal
                    .store
                    .outbox()
                    .count_by_state(*org, OutboxState::Sending)?,
            )
            .unwrap_or(u32::MAX);
        }
        if resends > 0 {
            self.internal
                .metrics
                .resends_resumption
                .increment(u64::from(resends));
        }

        let mut entry = DispatchReport::default();
        let mut saida = SendReport::default();
        for org in &orgs {
            entry.sum(self.dispatch(*org)?);
            saida.sum(self.send_pending(*org, transport).await?);
        }
        self.update_medidores()?;
        Ok(ResumeReport {
            entry,
            saida,
            resends,
        })
    }

    pub async fn cycle(&self, transport: &dyn OutputTransport) -> Resultado<ResumeReport> {
        let mut resends = 0u32;
        let orgs = self.orgs()?;
        let mut entry = DispatchReport::default();
        let mut saida = SendReport::default();
        for org in &orgs {
            resends += u32::try_from(
                self.internal
                    .store
                    .outbox()
                    .count_by_state(*org, OutboxState::Sending)?,
            )
            .unwrap_or(u32::MAX);
            entry.sum(self.dispatch(*org)?);
            saida.sum(self.send_pending(*org, transport).await?);
        }
        self.update_medidores()?;
        Ok(ResumeReport {
            entry,
            saida,
            resends,
        })
    }

    pub fn start_loop(self: &Arc<Self>, transport: Arc<dyn OutputTransport>) -> Loop {
        let stop = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&stop);
        let fluxo = Arc::clone(self);
        let interval = self.internal.options.interval_cycle;
        tokio::spawn(async move {
            while !signal.load(Ordering::Relaxed) {
                if let Err(error) = fluxo.cycle(transport.as_ref()).await {
                    tracing::warn!(error = %error, "ciclo de entrada e saída falhou; segue no próximo");
                }
                tokio::time::sleep(interval).await;
            }
        });
        Loop { stop }
    }

    pub fn orgs(&self) -> Resultado<Vec<OrgId>> {
        let mut orgs = Vec::new();
        let mut after = None;
        loop {
            let limit = 200u32;
            let page = self.internal.store.orgs().list(Page { after, limit })?;
            if page.is_empty() {
                break;
            }
            after = page.last().map(|org| org.id.as_uuid());
            let last = u32::try_from(page.len()).unwrap_or(u32::MAX) < limit;
            orgs.extend(page.into_iter().map(|org| org.id));
            if last {
                break;
            }
        }
        Ok(orgs)
    }

    pub fn statistics(&self, org: OrgId) -> Resultado<Statistics> {
        Ok(Statistics {
            entry_pending: self.internal.store.inbox().count_pending(org)?,
            output_sending: self
                .internal
                .store
                .outbox()
                .count_by_state(org, OutboxState::Sending)?,
            output_pending: self
                .internal
                .store
                .outbox()
                .count_by_state(org, OutboxState::Pending)?,
            output_failed: self
                .internal
                .store
                .outbox()
                .count_by_state(org, OutboxState::Failed)?,
        })
    }

    pub fn update_medidores(&self) -> Resultado<()> {
        let mut entry = 0u64;
        let mut sending = 0u64;
        for org in self.orgs()? {
            entry += self.internal.store.inbox().count_pending(org)?;
            sending += self
                .internal
                .store
                .outbox()
                .count_by_state(org, OutboxState::Sending)?;
        }
        self.internal
            .metrics
            .inbox_pending
            .set(i64::try_from(entry).unwrap_or(i64::MAX));
        self.internal
            .metrics
            .outbox_sending
            .set(i64::try_from(sending).unwrap_or(i64::MAX));
        Ok(())
    }
}

impl Internal {
    fn now(&self) -> Instant {
        let clock = self.clock.lock().unwrap_or_else(|error| error.into_inner());
        clock()
    }

    fn reivindicar_entry(&self, id: InboxId) -> bool {
        let mut em_voo = self.em_voo();
        if em_voo.contains(&id) {
            return false;
        }
        em_voo.insert(id);
        true
    }

    fn release_entry(&self, id: InboxId) {
        self.em_voo().remove(&id);
    }

    fn reivindicar_output(&self, id: OutboxId) -> bool {
        let mut in_send = self
            .in_send
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if in_send.contains(&id) {
            return false;
        }
        in_send.insert(id);
        true
    }

    fn release_output(&self, id: OutboxId) {
        self.in_send
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&id);
    }

    fn em_voo(&self) -> MutexGuard<'_, HashSet<InboxId>> {
        self.em_voo
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn schedule_entry(&self) -> MutexGuard<'_, HashMap<InboxId, Schedule>> {
        self.schedule_entry
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn schedule_output(&self) -> MutexGuard<'_, HashMap<OutboxId, Schedule>> {
        self.schedule_output
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

pub struct Loop {
    stop: Arc<AtomicBool>,
}

impl Loop {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

impl Drop for Loop {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
