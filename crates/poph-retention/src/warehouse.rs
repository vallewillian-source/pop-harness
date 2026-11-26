use std::path::PathBuf;

use poph_types::OrgId;
use serde::Serialize;

use crate::error::RetentionError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgRetention {
    pub id: OrgId,
    pub slug: String,
    pub config_json: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DatabaseSweep {
    #[serde(rename = "mensagens")]
    pub messages: u64,
    pub anexos: u64,
    #[serde(rename = "turnos")]
    pub turns: u64,
    pub inbox: u64,
    pub outbox: u64,
    pub conversas: u64,
    #[serde(rename = "ids_conversas")]
    pub conversation_ids: Vec<String>,
    #[serde(skip)]
    pub referenced_hashes: Vec<String>,
}

pub trait Warehouse {
    fn orgs(&self) -> Result<Vec<OrgRetention>, RetentionError>;
    fn org(&self, id: OrgId) -> Result<OrgRetention, RetentionError>;
    fn sweep(
        &self,
        org: OrgId,
        cutoff_ms: i64,
        simulate: bool,
    ) -> Result<DatabaseSweep, RetentionError>;
    fn blob_folder(&self, org: OrgId) -> PathBuf;
    fn record_audit(
        &self,
        org: OrgId,
        ator_tipo: &str,
        detail_json: &str,
    ) -> Result<(), RetentionError>;
}
