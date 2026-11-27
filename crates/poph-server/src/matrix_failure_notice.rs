use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use poph_matrix::{CallError, GhostIds, MatrixClient, error::Classification};
use poph_store::{HumanResponseFailure, OutcomeNotice, Store, StoreError};
use poph_types::{ConversationId, OrgId, OutboxId};
use serde_json::json;

use crate::matrix_metrics::MetricsMatrix;
use crate::{WHATSAPP_ATTEMPTS_EXHAUSTED, last_response_error};

pub const PREFIX_NOTICE: &str = "Não consegui entregar sua mensagem ao cliente: ";

pub const DEFAULT_BATCH: u32 = 50;

pub const DEFAULT_PERIOD: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureReason {
    AttemptsExhausted,
    RefusedByChannel,
    NotInformed,
}

impl FailureReason {
    pub fn classify(last_error: Option<&str>) -> Self {
        match last_error {
            Some(text) if text == WHATSAPP_ATTEMPTS_EXHAUSTED => Self::AttemptsExhausted,
            Some(_) => Self::RefusedByChannel,
            None => Self::NotInformed,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::AttemptsExhausted => "tentativas_esgotadas",
            Self::RefusedByChannel => "recusado_pelo_canal",
            Self::NotInformed => "nao_informado",
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Self::AttemptsExhausted => "o WhatsApp não respondeu depois de várias tentativas",
            Self::RefusedByChannel => "o WhatsApp recusou o envio",
            Self::NotInformed => "motivo não informado",
        }
    }
}

pub fn text_of_notice(reason: FailureReason) -> String {
    format!("{PREFIX_NOTICE}{}", reason.text())
}

pub fn notice_txn(item: OutboxId) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in item.to_string().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("ph_falha_{hash:016x}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureContext {
    pub room: String,
    pub agent: String,
}

pub trait ResponseFailuresPort: Send + Sync {
    fn pending_count(&self, limit: u32) -> Result<Vec<HumanResponseFailure>, StoreError>;

    fn context(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Result<Option<FailureContext>, StoreError>;

    fn register(
        &self,
        org: OrgId,
        failure: OutboxId,
        outcome: OutcomeNotice,
        reason: FailureReason,
    ) -> Result<(), StoreError>;
}

#[derive(Debug, Clone)]
pub struct HarnessFailuresPort {
    store: Arc<Store>,
}

impl HarnessFailuresPort {
    pub fn fresh(store: Arc<Store>) -> Self {
        Self { store }
    }
}

impl ResponseFailuresPort for HarnessFailuresPort {
    fn pending_count(&self, limit: u32) -> Result<Vec<HumanResponseFailure>, StoreError> {
        let mut failures = Vec::new();
        for state in self.store.matrix().linked_orgs()? {
            let faltam = limit.saturating_sub(failures.len() as u32);
            if faltam == 0 {
                break;
            }
            failures.extend(
                self.store
                    .matrix()
                    .human_response_failures(state.org_id, faltam)?,
            );
        }
        Ok(failures)
    }

    fn context(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Result<Option<FailureContext>, StoreError> {
        let Some(room) = self.store.matrix().room(org, conversation)? else {
            return Ok(None);
        };
        let agent = self.store.conversations().get(org, conversation)?.agent_id;
        Ok(Some(FailureContext {
            room: room.room_id,
            agent: agent.to_string(),
        }))
    }

    fn register(
        &self,
        org: OrgId,
        failure: OutboxId,
        outcome: OutcomeNotice,
        reason: FailureReason,
    ) -> Result<(), StoreError> {
        self.store
            .matrix()
            .record_notice_outcome(org, failure, outcome, reason.code())
            .map(|_| ())
    }
}

pub struct SynapseNotice {
    client: MatrixClient,
    ids: GhostIds,
}

impl SynapseNotice {
    pub fn new(client: MatrixClient, ids: GhostIds) -> Self {
        Self { client, ids }
    }

    async fn write(
        &self,
        context: &FailureContext,
        txn: &str,
        body: &str,
    ) -> Result<(), CallError> {
        let ghost = self.ids.agent(&context.agent).map_err(|_| {
            CallError::Configuration("id de agente fora do padrão do módulo Matrix".to_owned())
        })?;
        self.client
            .send_event(
                ghost.user_id(),
                &context.room,
                "m.room.message",
                txn,
                &json!({ "msgtype": "m.notice", "body": body }),
            )
            .await
            .map(|_| ())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoticeReport {
    pub notified: u32,
    pub without_room: u32,
    pub refused: u32,
    pub deferred: u32,
    pub errors_of_database: u32,
}

pub struct FailureNotice {
    port: Arc<dyn ResponseFailuresPort>,
    notice: SynapseNotice,
    batch: u32,
    metrics: Arc<MetricsMatrix>,
    stop: watch::Sender<bool>,
}

impl FailureNotice {
    pub fn new(port: Arc<dyn ResponseFailuresPort>, notice: SynapseNotice) -> Self {
        let (stop, _) = watch::channel(false);
        Self {
            port,
            notice,
            batch: DEFAULT_BATCH,
            metrics: MetricsMatrix::unregistered(),
            stop,
        }
    }

    pub fn with_metrics(mut self, metrics: Arc<MetricsMatrix>) -> Self {
        self.metrics = metrics;
        self
    }

    pub fn with_batch(mut self, batch: u32) -> Self {
        self.batch = batch.max(1);
        self
    }

    pub async fn pass(&self) -> NoticeReport {
        let mut report = NoticeReport::default();
        let failures = match self.port.pending_count(self.batch) {
            Ok(failures) => failures,
            Err(_) => {
                tracing::warn!(
                    target: "poph_server::matrix_falhas_aviso",
                    reason = "banco_indisponivel",
                    "não consegui listar as falhas de resposta humana; tenta na próxima passagem"
                );
                report.errors_of_database += 1;
                return report;
            }
        };
        for failure in &failures {
            if self.handle(failure, &mut report).await.is_err() {
                report.errors_of_database += 1;
            }
        }
        report
    }

    pub fn ask_stop(&self) {
        self.stop.send_replace(true);
    }

    pub async fn run(self: Arc<Self>, interval: Duration) {
        let mut stop = self.stop.subscribe();
        loop {
            if *stop.borrow() {
                break;
            }
            self.pass().await;
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = stop.changed() => break,
            }
        }
    }

    async fn handle(
        &self,
        failure: &HumanResponseFailure,
        report: &mut NoticeReport,
    ) -> Result<(), StoreError> {
        let reason = FailureReason::classify(last_response_error(failure));
        let Some(context) = self.port.context(failure.org_id, failure.conversation)? else {
            self.port.register(
                failure.org_id,
                failure.outbox_id,
                OutcomeNotice::WithoutRoom,
                reason,
            )?;
            report.without_room += 1;
            tracing::info!(
                target: "poph_server::matrix_falhas_aviso",
                org = %failure.org_id,
                conversation = %failure.conversation,
                outbox = %failure.outbox_id,
                reason = reason.code(),
                "falha de resposta humana sem sala: nada a avisar"
            );
            return Ok(());
        };

        let txn = notice_txn(failure.outbox_id);
        match self
            .notice
            .write(&context, &txn, &text_of_notice(reason))
            .await
        {
            Ok(()) => {
                self.port.register(
                    failure.org_id,
                    failure.outbox_id,
                    OutcomeNotice::Notify,
                    reason,
                )?;
                report.notified += 1;
                tracing::info!(
                    target: "poph_server::matrix_falhas_aviso",
                    org = %failure.org_id,
                    conversation = %failure.conversation,
                    outbox = %failure.outbox_id,
                    reason = reason.code(),
                    "aviso de falha de resposta humana escrito na sala"
                );
            }
            Err(error) if error.classification() == Classification::Transient => {
                self.count_error(&error);
                report.deferred += 1;
                tracing::warn!(
                    target: "poph_server::matrix_falhas_aviso",
                    org = %failure.org_id,
                    outbox = %failure.outbox_id,
                    reason = reason.code(),
                    class = "transitoria",
                    "aviso de falha não escrito agora; tenta na próxima passagem"
                );
            }
            Err(error) => {
                self.count_error(&error);
                self.port.register(
                    failure.org_id,
                    failure.outbox_id,
                    OutcomeNotice::Refused,
                    reason,
                )?;
                report.refused += 1;
                tracing::warn!(
                    target: "poph_server::matrix_falhas_aviso",
                    org = %failure.org_id,
                    outbox = %failure.outbox_id,
                    reason = reason.code(),
                    class = "definitiva",
                    "o Synapse recusou o aviso de falha de resposta humana: não será repetido"
                );
            }
        }
        Ok(())
    }

    fn count_error(&self, error: &CallError) {
        match error.classification() {
            Classification::Transient => self.metrics.transient_errors.increment(1),
            Classification::Definitive => self.metrics.definitive_errors.increment(1),
        }
        if matches!(error, CallError::Response { status: 429, .. }) {
            self.metrics.responses_429.increment(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_queue_text_becomes_attempts_exhausted() {
        assert_eq!(
            FailureReason::classify(Some(WHATSAPP_ATTEMPTS_EXHAUSTED)),
            FailureReason::AttemptsExhausted
        );
        assert_eq!(
            FailureReason::classify(Some("a Evolution recusou o número 5511999990000")),
            FailureReason::RefusedByChannel,
            "qualquer outro erro definitivo vira o motivo genérico, sem o texto"
        );
        assert_eq!(FailureReason::classify(None), FailureReason::NotInformed);
    }

    #[test]
    fn notice_carries_only_short_reason() {
        assert_eq!(
            text_of_notice(FailureReason::RefusedByChannel),
            "Não consegui entregar sua mensagem ao cliente: o WhatsApp recusou o envio"
        );
        for reason in [
            FailureReason::AttemptsExhausted,
            FailureReason::RefusedByChannel,
            FailureReason::NotInformed,
        ] {
            let text = text_of_notice(reason);
            assert!(text.starts_with(PREFIX_NOTICE));
            assert!(!text.contains('@'), "nada de id de usuário no aviso");
            assert!(!text.contains("5511"), "nada de telefone no aviso");
        }
    }

    #[test]
    fn reason_codes_are_short_and_distinct() {
        let codes = [
            FailureReason::AttemptsExhausted.code(),
            FailureReason::RefusedByChannel.code(),
            FailureReason::NotInformed.code(),
        ];
        for code in codes {
            assert!(code.len() <= 64, "cabe na coluna `motivo`");
            assert!(code.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
        }
        assert_ne!(codes[0], codes[1]);
        assert_ne!(codes[1], codes[2]);
    }

    #[test]
    fn notice_txn_is_stable_per_item_and_distinct_across_items() {
        let um = OutboxId::generate();
        let other = OutboxId::generate();
        assert_eq!(notice_txn(um), notice_txn(um));
        assert_ne!(notice_txn(um), notice_txn(other));
        assert!(notice_txn(um).starts_with("ph_falha_"));
    }
}
