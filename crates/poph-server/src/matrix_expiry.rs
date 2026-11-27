use std::sync::Arc;
use std::time::Duration;

use poph_matrix::client::MatrixClient;
use poph_matrix::error::{AssetError, Classification};
use poph_matrix::expiration::{self, AssetToExpire};
use poph_matrix::users::GhostIds;
use poph_store::{RoomAsset, Store};
use poph_types::{MatrixConfig, OrgId, Secret};
use tokio::sync::watch;

use crate::matrix_metrics::MetricsMatrix;

pub const DEFAULT_PERIOD: Duration = Duration::from_secs(3_600);

pub const DEFAULT_BATCH: u32 = 50;

pub const WAIT_BASE: Duration = Duration::from_secs(60);

pub const MAX_WAIT: Duration = Duration::from_secs(3_600);

const MS_PER_DAY: i64 = 86_400_000;

const MAX_EXPONENT: u32 = 20;

pub fn wait_after(attempts: u32, base: Duration, cap: Duration) -> Duration {
    let step = attempts.saturating_sub(1).min(MAX_EXPONENT);
    let factor = 1u32.checked_shl(step).unwrap_or(u32::MAX);
    base.saturating_mul(factor).min(cap)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExpiryReport {
    pub expired: u32,
    pub redacted: u32,
    pub deleted: u32,
    pub already_deleted_items: u32,
    pub deferred: u32,
    pub completed: u32,
    pub errors_of_database: u32,
}

pub struct AssetExpiry {
    store: Arc<Store>,
    client: MatrixClient,
    token_admin: Secret,
    ids: GhostIds,
    retention_days: u32,
    batch: u32,
    period: Duration,
    wait_base: Duration,
    max_wait: Duration,
    metrics: Arc<MetricsMatrix>,
    now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
    stop: watch::Sender<bool>,
}

impl std::fmt::Debug for AssetExpiry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExpiracaoMidia")
            .field("cliente", &self.client)
            .field("token_admin", &self.token_admin)
            .field("retencao_dias", &self.retention_days)
            .field("lote", &self.batch)
            .field("periodo", &self.period)
            .finish_non_exhaustive()
    }
}

impl AssetExpiry {
    pub fn new(
        store: Arc<Store>,
        client: MatrixClient,
        token_admin: Secret,
        config: &MatrixConfig,
    ) -> Result<Self, poph_matrix::users::GhostsError> {
        let ids = GhostIds::of_config(config)?;
        let (stop, _) = watch::channel(false);
        Ok(Self {
            store,
            client,
            token_admin,
            ids,
            retention_days: config.attachment_retention_days.max(1),
            batch: DEFAULT_BATCH,
            period: DEFAULT_PERIOD,
            wait_base: WAIT_BASE,
            max_wait: MAX_WAIT,
            metrics: MetricsMatrix::unregistered(),
            now_ms: Arc::new(crate::matrix_mirror::system_clock),
            stop,
        })
    }

    pub fn with_metrics(mut self, metrics: Arc<MetricsMatrix>) -> Self {
        self.metrics = metrics;
        self
    }

    pub fn with_clock(mut self, now_ms: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        self.now_ms = now_ms;
        self
    }

    pub fn with_batch(mut self, batch: u32) -> Self {
        self.batch = batch.max(1);
        self
    }

    pub fn with_period(mut self, period: Duration) -> Self {
        self.period = period;
        self
    }

    pub fn with_wait(mut self, base: Duration, cap: Duration) -> Self {
        self.wait_base = base;
        self.max_wait = cap;
        self
    }

    pub fn ask_stop(&self) {
        self.stop.send_replace(true);
    }

    pub async fn run(self: Arc<Self>) {
        let mut stop = self.stop.subscribe();
        loop {
            if *stop.borrow() {
                break;
            }
            let report = self.pass().await;
            tracing::debug!(
                target: "poph_server::matrix_expiracao",
                expired_count = report.expired,
                redacted_count = report.redacted,
                deleted_count = report.deleted,
                deferred_count = report.deferred,
                "passagem da expiração da mídia"
            );
            tokio::select! {
                _ = tokio::time::sleep(self.period) => {}
                _ = stop.changed() => break,
            }
        }
    }

    pub async fn pass(&self) -> ExpiryReport {
        let mut report = ExpiryReport::default();
        let now = (self.now_ms)();
        let cutoff = now - i64::from(self.retention_days) * MS_PER_DAY;

        let orgs = match self.store.matrix().linked_orgs() {
            Ok(orgs) => orgs,
            Err(_) => {
                tracing::warn!(
                    target: "poph_server::matrix_expiracao",
                    reason = "banco_indisponivel",
                    "não consegui listar as orgs do módulo; a expiração tenta na próxima passagem"
                );
                report.errors_of_database += 1;
                return report;
            }
        };

        let mut orgs_with_handling: Vec<OrgId> = Vec::new();
        for state in orgs.iter().filter(|state| state.linked) {
            let org = state.org_id;
            let assets = match self
                .store
                .matrix_asset()
                .pending(org, cutoff, now, self.batch)
            {
                Ok(assets) => assets,
                Err(_) => {
                    report.errors_of_database += 1;
                    tracing::warn!(
                        target: "poph_server::matrix_expiracao",
                        org = %org,
                        reason = "banco_indisponivel",
                        "não consegui ler a fila da expiração; tenta na próxima passagem"
                    );
                    continue;
                }
            };
            report.expired += u32::try_from(assets.len()).unwrap_or(u32::MAX);
            for asset in &assets {
                self.handle(asset, now, &mut report).await;
            }
            orgs_with_handling.push(org);
        }

        let mut pending_count = 0u32;
        for org in orgs_with_handling {
            match self.store.matrix_asset().org_pending(org) {
                Ok(total) => pending_count = pending_count.saturating_add(total),
                Err(_) => report.errors_of_database += 1,
            }
        }
        self.metrics
            .assets_pending_expiry
            .set(i64::from(pending_count));
        report
    }

    async fn handle(&self, asset: &RoomAsset, now: i64, report: &mut ExpiryReport) {
        let mut failure: Option<AssetError> = None;

        if asset.still_to_draft() {
            match self.redact(asset).await {
                Ok(()) => {
                    match self
                        .store
                        .matrix_asset()
                        .mark_draft(asset.org_id, asset.id, now)
                    {
                        Ok(()) => {
                            report.redacted += 1;
                            self.metrics.redacted_assets.increment(1);
                            tracing::info!(
                                target: "poph_server::matrix_expiracao",
                                org = %asset.org_id,
                                conversation = %asset.conversation,
                                "evento de mídia redigido na sala depois do prazo de retenção"
                            );
                        }
                        Err(_) => report.errors_of_database += 1,
                    }
                }
                Err(error) => {
                    self.count_error(&error);
                    tracing::warn!(
                        target: "poph_server::matrix_expiracao",
                        org = %asset.org_id,
                        conversation = %asset.conversation,
                        reason = expiration::error_code(&error),
                        class = %error.classification(),
                        "não consegui redigir o evento da mídia; o arquivo é apagado do mesmo jeito"
                    );
                    failure = Some(error);
                }
            }
        }

        if asset.still_to_delete() {
            match expiration::delete(&self.client, &self.token_admin, &asset.mxc).await {
                Ok(did_delete) => {
                    match self
                        .store
                        .matrix_asset()
                        .mark_deleted(asset.org_id, asset.id, now)
                    {
                        Ok(()) => {
                            report.deleted += 1;
                            if !did_delete {
                                report.already_deleted_items += 1;
                            }
                            self.metrics.assets_deleted.increment(1);
                            tracing::info!(
                                target: "poph_server::matrix_expiracao",
                                org = %asset.org_id,
                                conversation = %asset.conversation,
                                already_deleted = !did_delete,
                                "arquivo de mídia apagado do Synapse depois do prazo de retenção"
                            );
                        }
                        Err(_) => report.errors_of_database += 1,
                    }
                }
                Err(error) => {
                    self.count_error(&error);
                    tracing::warn!(
                        target: "poph_server::matrix_expiracao",
                        org = %asset.org_id,
                        conversation = %asset.conversation,
                        reason = expiration::error_code(&error),
                        class = %error.classification(),
                        "não consegui apagar o arquivo da mídia; tenta de novo depois"
                    );
                    failure = failure.or(Some(error));
                }
            }
        }

        match failure {
            Some(error) => {
                let attempts = asset.attempts.saturating_add(1);
                let wait = wait_after(attempts, self.wait_base, self.max_wait);
                let next = now.saturating_add(i64::try_from(wait.as_millis()).unwrap_or(i64::MAX));
                if self
                    .store
                    .matrix_asset()
                    .adiar(
                        asset.org_id,
                        asset.id,
                        next,
                        &expiration::error_code(&error),
                        now,
                    )
                    .is_err()
                {
                    report.errors_of_database += 1;
                }
                report.deferred += 1;
                self.metrics.assets_expiry_deferred.increment(1);
            }
            None => {
                if !asset.still_to_draft() && !asset.still_to_delete() {
                    report.completed += 1;
                }
            }
        }
    }

    async fn redact(&self, asset: &RoomAsset) -> Result<(), AssetError> {
        let ghost = self.ids.agent(&asset.agent).map_err(|_| {
            AssetError::Address(
                "o agente da conversa não vira usuário virtual nesta instalação".to_owned(),
            )
        })?;
        let id = asset.id.to_string();
        let expired_asset = AssetToExpire {
            id: &id,
            name: &asset.nome,
            mime: &asset.mime,
            category: asset.category.as_text(),
            size_bytes: asset.size_bytes,
            sent_at: asset.sent_at,
        };
        expiration::redact(
            &self.client,
            &ghost,
            &asset.room,
            &asset.evento,
            &expired_asset,
        )
        .await
        .map(|_| ())
        .map_err(AssetError::Call)
    }

    fn count_error(&self, error: &AssetError) {
        match error.classification() {
            Classification::Transient => self.metrics.transient_errors.increment(1),
            Classification::Definitive => self.metrics.definitive_errors.increment(1),
        }
        if let AssetError::Call(poph_matrix::error::CallError::Response { status: 429, .. }) = error
        {
            self.metrics.responses_429.increment(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_doubles_until_cap() {
        let base = Duration::from_secs(60);
        let cap = Duration::from_secs(3_600);
        assert_eq!(wait_after(1, base, cap), Duration::from_secs(60));
        assert_eq!(wait_after(2, base, cap), Duration::from_secs(120));
        assert_eq!(wait_after(3, base, cap), Duration::from_secs(240));
        assert_eq!(wait_after(6, base, cap), Duration::from_secs(1_920));
        assert_eq!(wait_after(7, base, cap), cap);
        assert_eq!(wait_after(1_000, base, cap), cap);
        assert_eq!(wait_after(0, base, cap), base);
    }

    #[test]
    fn deadline_counts_from_retention_day() {
        let now = 10 * MS_PER_DAY;
        assert_eq!(now - i64::from(4) * MS_PER_DAY, 6 * MS_PER_DAY);
    }
}
