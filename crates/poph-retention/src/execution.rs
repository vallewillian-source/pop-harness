use std::collections::HashSet;

use poph_types::OrgId;
use serde::Serialize;
use serde_json::json;

use crate::blobs::{find_orphans, remove};
use crate::config::{cutoff_ms, days_of_org};
use crate::error::RetentionError;
use crate::warehouse::{DatabaseSweep, OrgRetention, Warehouse};

pub const BLOB_GRACE_MS: i64 = 3_600_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub now_ms: i64,
    pub default_days: u32,
    pub blob_grace_ms: i64,
    pub simulate: bool,
    pub ator_tipo: String,
}

impl Options {
    pub fn new(now_ms: i64, default_days: u32, simulate: bool, ator_tipo: &str) -> Self {
        Self {
            now_ms,
            default_days,
            blob_grace_ms: BLOB_GRACE_MS,
            simulate,
            ator_tipo: ator_tipo.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Situacao {
    #[serde(rename = "varrida")]
    Swept,
    #[serde(rename = "ignorada")]
    Skipped,
    #[serde(rename = "erro")]
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrgReport {
    pub org_id: String,
    pub slug: String,
    pub situacao: Situacao,
    pub dias: Option<u32>,
    #[serde(rename = "corte_ms")]
    pub cutoff_ms: Option<i64>,
    #[serde(rename = "banco")]
    pub database: Option<DatabaseSweep>,
    #[serde(rename = "arquivos_sem_dono")]
    pub orphan_files: Vec<String>,
    #[serde(rename = "arquivos_apagados")]
    pub deleted_files: u64,
    pub motivo: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionReport {
    #[serde(rename = "agora_ms")]
    pub now_ms: i64,
    #[serde(rename = "simulacao")]
    pub simulation: bool,
    pub orgs: Vec<OrgReport>,
}

pub fn run_org(
    warehouse: &dyn Warehouse,
    id: OrgId,
    options: &Options,
) -> Result<OrgReport, RetentionError> {
    let org = warehouse.org(id)?;
    Ok(sweep_one(warehouse, &org, options))
}

pub fn run_all(
    warehouse: &dyn Warehouse,
    options: &Options,
) -> Result<ExecutionReport, RetentionError> {
    let orgs = warehouse.orgs()?;
    let orgs = orgs
        .iter()
        .map(|org| sweep_one(warehouse, org, options))
        .collect();
    Ok(ExecutionReport {
        now_ms: options.now_ms,
        simulation: options.simulate,
        orgs,
    })
}

fn sweep_one(warehouse: &dyn Warehouse, org: &OrgRetention, options: &Options) -> OrgReport {
    let base = |situacao: Situacao, motivo: Option<String>| OrgReport {
        org_id: org.id.to_string(),
        slug: org.slug.clone(),
        situacao,
        dias: None,
        cutoff_ms: None,
        database: None,
        orphan_files: Vec::new(),
        deleted_files: 0,
        motivo,
    };

    let dias = match days_of_org(&org.slug, &org.config_json, options.default_days) {
        Ok(dias) => dias,
        Err(error) => return base(Situacao::Failed, Some(error.to_string())),
    };
    let Some(cutoff) = cutoff_ms(options.now_ms, dias) else {
        let mut report = base(
            Situacao::Skipped,
            Some("retenção 0: nada é apagado nesta org".to_owned()),
        );
        report.dias = Some(dias);
        return report;
    };

    let database = match warehouse.sweep(org.id, cutoff, options.simulate) {
        Ok(database) => database,
        Err(error) => {
            let mut report = base(Situacao::Failed, Some(error.to_string()));
            report.dias = Some(dias);
            report.cutoff_ms = Some(cutoff);
            return report;
        }
    };

    let mut reasons = Vec::new();
    let referenced: HashSet<String> = database.referenced_hashes.iter().cloned().collect();
    let folder = warehouse.blob_folder(org.id);
    let file_cutoff = options.now_ms.saturating_sub(options.blob_grace_ms);
    let orphans = match find_orphans(&folder, &referenced, file_cutoff) {
        Ok(orphans) => orphans,
        Err(error) => {
            reasons.push(error.to_string());
            Vec::new()
        }
    };

    let mut deleted = 0_u64;
    if !options.simulate {
        for orphan in &orphans {
            match remove(orphan) {
                Ok(()) => deleted += 1,
                Err(error) => reasons.push(error.to_string()),
            }
        }
    }

    let detail = json!({
        "simulacao": options.simulate,
        "dias": dias,
        "corte_ms": cutoff,
        "mensagens": database.messages,
        "anexos": database.anexos,
        "turnos": database.turns,
        "inbox": database.inbox,
        "outbox": database.outbox,
        "conversas": database.conversas,
        "arquivos_sem_dono": orphans.len(),
        "arquivos_apagados": deleted,
    });
    if let Err(error) = warehouse.record_audit(org.id, &options.ator_tipo, &detail.to_string()) {
        reasons.push(format!("auditoria não gravada: {error}"));
    }

    let situacao = if reasons.is_empty() {
        Situacao::Swept
    } else {
        Situacao::Failed
    };
    OrgReport {
        org_id: org.id.to_string(),
        slug: org.slug.clone(),
        situacao,
        dias: Some(dias),
        cutoff_ms: Some(cutoff),
        database: Some(database),
        orphan_files: orphans.into_iter().map(|o| o.nome).collect(),
        deleted_files: deleted,
        motivo: if reasons.is_empty() {
            None
        } else {
            Some(reasons.join("; "))
        },
    }
}
