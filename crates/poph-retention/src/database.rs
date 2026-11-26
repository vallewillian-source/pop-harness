use std::path::{Path, PathBuf};

use poph_store::{NewAuditEntry, Page, Store, StoreError};
use poph_types::OrgId;

use crate::error::RetentionError;
use crate::warehouse::{DatabaseSweep, OrgRetention, Warehouse};

const PAGE_SIZE: u32 = 200;

pub struct HarnessDatabase<'a> {
    store: &'a Store,
    blobs: PathBuf,
}

impl<'a> HarnessDatabase<'a> {
    pub fn new(store: &'a Store, blobs: impl AsRef<Path>) -> Self {
        Self {
            store,
            blobs: blobs.as_ref().to_path_buf(),
        }
    }
}

impl Warehouse for HarnessDatabase<'_> {
    fn orgs(&self) -> Result<Vec<OrgRetention>, RetentionError> {
        let mut all = Vec::new();
        let mut after = None;
        loop {
            let page = self
                .store
                .orgs()
                .list(Page {
                    after,
                    limit: PAGE_SIZE,
                })
                .map_err(database_error)?;
            let full = page.len() as u32 == PAGE_SIZE;
            after = page.last().map(|org| org.id.as_uuid());
            all.extend(page.into_iter().map(|org| OrgRetention {
                id: org.id,
                slug: org.slug,
                config_json: org.config_json,
            }));
            if !full {
                break;
            }
        }
        Ok(all)
    }

    fn org(&self, id: OrgId) -> Result<OrgRetention, RetentionError> {
        match self.store.orgs().get(id) {
            Ok(org) => Ok(OrgRetention {
                id: org.id,
                slug: org.slug,
                config_json: org.config_json,
            }),
            Err(error) if error.is_not_found() => Err(RetentionError::OrgNotFound(id.to_string())),
            Err(error) => Err(database_error(error)),
        }
    }

    fn sweep(
        &self,
        org: OrgId,
        cutoff_ms: i64,
        simulate: bool,
    ) -> Result<DatabaseSweep, RetentionError> {
        let sweep_result = self
            .store
            .retention()
            .scan_table(org, cutoff_ms, simulate)
            .map_err(database_error)?;
        Ok(DatabaseSweep {
            messages: sweep_result.messages,
            anexos: sweep_result.attachments,
            turns: sweep_result.turns,
            inbox: sweep_result.inbox,
            outbox: sweep_result.outbox,
            conversas: sweep_result.conversations,
            conversation_ids: sweep_result.conversation_ids,
            referenced_hashes: sweep_result.hashes_referenciados,
        })
    }

    fn blob_folder(&self, org: OrgId) -> PathBuf {
        self.blobs.join(org.to_string())
    }

    fn record_audit(
        &self,
        org: OrgId,
        ator_tipo: &str,
        detail_json: &str,
    ) -> Result<(), RetentionError> {
        self.store
            .audit()
            .register(
                Some(org),
                NewAuditEntry {
                    ator_tipo: ator_tipo.to_owned(),
                    actor_id: None,
                    acao: "retention".to_owned(),
                    entity: "org".to_owned(),
                    entity_id: Some(org.to_string()),
                    detail_json: detail_json.to_owned(),
                },
            )
            .map(|_| ())
            .map_err(database_error)
    }
}

fn database_error(error: StoreError) -> RetentionError {
    RetentionError::Database(error.to_string())
}
