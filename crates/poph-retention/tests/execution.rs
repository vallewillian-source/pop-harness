use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use poph_retention::config::DAY_MS;
use poph_retention::{
    DatabaseSweep, Options, OrgRetention, RetentionError, Situacao, Warehouse, run_all, run_org,
};
use poph_types::OrgId;

const SHA_ORPHAN: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
const SHA_LIVE: &str = "0f0e0d0c0b0a09080706050403020100ffeeddccbbaa99887766554433221100";
const NOW_MS: i64 = 1_795_131_511_000;

struct Fake {
    orgs: Vec<OrgRetention>,
    folders: HashMap<OrgId, PathBuf>,
    sweeps: HashMap<OrgId, Result<DatabaseSweep, RetentionError>>,
    calls: RefCell<Vec<(OrgId, i64, bool)>>,
    audits: RefCell<Vec<(OrgId, String, String)>>,
    audit_fails: bool,
}

impl Fake {
    fn with_orgs(orgs: Vec<(OrgRetention, PathBuf)>) -> Self {
        let mut folders = HashMap::new();
        let mut list = Vec::new();
        for (org, folder) in orgs {
            folders.insert(org.id, folder);
            list.push(org);
        }
        Self {
            orgs: list,
            folders,
            sweeps: HashMap::new(),
            calls: RefCell::new(Vec::new()),
            audits: RefCell::new(Vec::new()),
            audit_fails: false,
        }
    }
}

impl Warehouse for Fake {
    fn orgs(&self) -> Result<Vec<OrgRetention>, RetentionError> {
        Ok(self.orgs.clone())
    }
    fn org(&self, id: OrgId) -> Result<OrgRetention, RetentionError> {
        self.orgs
            .iter()
            .find(|o| o.id == id)
            .cloned()
            .ok_or_else(|| RetentionError::Database("org não encontrada".to_owned()))
    }
    fn sweep(
        &self,
        org: OrgId,
        cutoff_ms: i64,
        simulate: bool,
    ) -> Result<DatabaseSweep, RetentionError> {
        self.calls.borrow_mut().push((org, cutoff_ms, simulate));
        self.sweeps
            .get(&org)
            .cloned()
            .unwrap_or_else(|| Ok(DatabaseSweep::default()))
    }
    fn blob_folder(&self, org: OrgId) -> PathBuf {
        self.folders[&org].clone()
    }
    fn record_audit(
        &self,
        org: OrgId,
        ator_tipo: &str,
        detail_json: &str,
    ) -> Result<(), RetentionError> {
        if self.audit_fails {
            return Err(RetentionError::Database("disco cheio".to_owned()));
        }
        self.audits
            .borrow_mut()
            .push((org, ator_tipo.to_owned(), detail_json.to_owned()));
        Ok(())
    }
}

fn org(slug: &str, config: &str) -> OrgRetention {
    OrgRetention {
        id: OrgId::generate(),
        slug: slug.to_owned(),
        config_json: config.to_owned(),
    }
}

fn write_file(folder: &Path, file_name: &str, age_ms: i64) {
    fs::create_dir_all(folder).unwrap();
    let path = folder.join(file_name);
    fs::write(&path, b"bytes").unwrap();
    let instant = UNIX_EPOCH + Duration::from_millis((NOW_MS - age_ms) as u64);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(instant)
        .unwrap();
}

fn options_for(simulate: bool) -> Options {
    Options::new(NOW_MS, 30, simulate, "cli")
}

#[test]
fn dry_run_lists_the_orphan_and_deletes_nothing() {
    let base = tempfile::tempdir().unwrap();
    let folder = base.path().join("org-a");
    write_file(&folder, SHA_ORPHAN, 90 * DAY_MS);
    let a = org("alfa", "{}");
    let mut fake = Fake::with_orgs(vec![(a.clone(), folder.clone())]);
    fake.sweeps.insert(
        a.id,
        Ok(DatabaseSweep {
            messages: 12,
            anexos: 2,
            conversation_ids: vec!["c1".into()],
            ..Default::default()
        }),
    );

    let report = run_org(&fake, a.id, &options_for(true)).unwrap();

    assert_eq!(report.situacao, Situacao::Swept);
    assert_eq!(report.orphan_files, vec![SHA_ORPHAN.to_owned()]);
    assert_eq!(report.deleted_files, 0);
    assert!(
        folder.join(SHA_ORPHAN).exists(),
        "dry-run não pode apagar arquivo"
    );
    let call = fake.calls.borrow()[0];
    assert!(call.2, "o banco recebe a simulação");
    assert_eq!(call.1, NOW_MS - 30 * DAY_MS);
    let (_, _, detail) = &fake.audits.borrow()[0];
    assert!(detail.contains("\"simulacao\":true"), "{detail}");
    assert!(detail.contains("\"mensagens\":12"), "{detail}");
}

#[test]
fn execution_deletes_only_the_orphan_and_audits_counts() {
    let base = tempfile::tempdir().unwrap();
    let folder = base.path().join("org-a");
    write_file(&folder, SHA_ORPHAN, 90 * DAY_MS);
    write_file(&folder, SHA_LIVE, 90 * DAY_MS);
    let a = org("alfa", "{}");
    let mut fake = Fake::with_orgs(vec![(a.clone(), folder.clone())]);
    fake.sweeps.insert(
        a.id,
        Ok(DatabaseSweep {
            messages: 5,
            anexos: 1,
            turns: 3,
            inbox: 4,
            outbox: 6,
            conversas: 1,
            referenced_hashes: vec![SHA_LIVE.to_owned()],
            ..Default::default()
        }),
    );

    let report = run_org(&fake, a.id, &options_for(false)).unwrap();

    assert_eq!(report.situacao, Situacao::Swept);
    assert_eq!(report.deleted_files, 1);
    assert!(!folder.join(SHA_ORPHAN).exists());
    assert!(folder.join(SHA_LIVE).exists(), "arquivo com dono fica");
    let (_, actor, detail) = fake.audits.borrow()[0].clone();
    assert_eq!(actor, "cli");
    let json: serde_json::Value = serde_json::from_str(&detail).unwrap();
    assert_eq!(json["simulacao"], false);
    assert_eq!(json["mensagens"], 5);
    assert_eq!(json["anexos"], 1);
    assert_eq!(json["turnos"], 3);
    assert_eq!(json["inbox"], 4);
    assert_eq!(json["outbox"], 6);
    assert_eq!(json["conversas"], 1);
    assert_eq!(json["arquivos_apagados"], 1);
}

#[test]
fn zero_retention_skips_the_org_without_calling_the_database() {
    let base = tempfile::tempdir().unwrap();
    let folder = base.path().join("org-a");
    write_file(&folder, SHA_ORPHAN, 900 * DAY_MS);
    let a = org("alfa", r#"{"retencao":{"dias":0}}"#);
    let fake = Fake::with_orgs(vec![(a.clone(), folder.clone())]);

    let report = run_org(&fake, a.id, &options_for(false)).unwrap();

    assert_eq!(report.situacao, Situacao::Skipped);
    assert!(fake.calls.borrow().is_empty(), "org com 0 não vai ao banco");
    assert!(
        folder.join(SHA_ORPHAN).exists(),
        "org com 0 não perde arquivo"
    );
    assert!(fake.audits.borrow().is_empty());
}

#[test]
fn invalid_config_skips_only_that_org() {
    let base = tempfile::tempdir().unwrap();
    let bad_folder = base.path().join("ruim");
    let good_folder = base.path().join("boa");
    write_file(&bad_folder, SHA_ORPHAN, 90 * DAY_MS);
    write_file(&good_folder, SHA_ORPHAN, 90 * DAY_MS);
    let bad = org("ruim", r#"{"retencao":{"dias":-5}}"#);
    let good = org("boa", "{}");
    let fake = Fake::with_orgs(vec![
        (bad.clone(), bad_folder.clone()),
        (good.clone(), good_folder.clone()),
    ]);

    let report = run_all(&fake, &options_for(false)).unwrap();

    let bad_report = report.orgs.iter().find(|o| o.slug == "ruim").unwrap();
    let good_report = report.orgs.iter().find(|o| o.slug == "boa").unwrap();
    assert_eq!(bad_report.situacao, Situacao::Failed);
    assert!(
        bad_report
            .motivo
            .as_deref()
            .unwrap()
            .contains("não pode ser negativo")
    );
    assert!(
        bad_folder.join(SHA_ORPHAN).exists(),
        "org ruim não perde nada"
    );
    assert_eq!(good_report.situacao, Situacao::Swept);
    assert!(!good_folder.join(SHA_ORPHAN).exists());
    assert!(fake.calls.borrow().iter().all(|c| c.0 != bad.id));
}

#[test]
fn database_failure_deletes_no_file() {
    let base = tempfile::tempdir().unwrap();
    let folder = base.path().join("org-a");
    write_file(&folder, SHA_ORPHAN, 90 * DAY_MS);
    let a = org("alfa", "{}");
    let mut fake = Fake::with_orgs(vec![(a.clone(), folder.clone())]);
    fake.sweeps.insert(
        a.id,
        Err(RetentionError::Database("banco ocupado".to_owned())),
    );

    let report = run_org(&fake, a.id, &options_for(false)).unwrap();

    assert_eq!(report.situacao, Situacao::Failed);
    assert!(
        folder.join(SHA_ORPHAN).exists(),
        "banco falhou: arquivo fica"
    );
    assert!(fake.audits.borrow().is_empty());
}

#[test]
fn audit_failure_shows_in_the_report() {
    let base = tempfile::tempdir().unwrap();
    let folder = base.path().join("org-a");
    write_file(&folder, SHA_ORPHAN, 90 * DAY_MS);
    let a = org("alfa", "{}");
    let mut fake = Fake::with_orgs(vec![(a.clone(), folder.clone())]);
    fake.audit_fails = true;

    let report = run_org(&fake, a.id, &options_for(false)).unwrap();

    assert_eq!(report.situacao, Situacao::Failed);
    assert!(
        report
            .motivo
            .as_deref()
            .unwrap()
            .contains("auditoria não gravada")
    );
}

#[test]
fn org_does_not_touch_another_orgs_folder() {
    let base = tempfile::tempdir().unwrap();
    let folder_a = base.path().join("org-a");
    let folder_b = base.path().join("org-b");
    write_file(&folder_a, SHA_ORPHAN, 90 * DAY_MS);
    write_file(&folder_b, SHA_ORPHAN, 90 * DAY_MS);
    let a = org("alfa", "{}");
    let b = org("beta", "{}");
    let fake = Fake::with_orgs(vec![
        (a.clone(), folder_a.clone()),
        (b.clone(), folder_b.clone()),
    ]);

    run_org(&fake, a.id, &options_for(false)).unwrap();

    assert!(!folder_a.join(SHA_ORPHAN).exists());
    assert!(
        folder_b.join(SHA_ORPHAN).exists(),
        "a org B não é varrida nesta chamada"
    );
    assert!(fake.calls.borrow().iter().all(|c| c.0 == a.id));
}

#[test]
fn recent_file_stays_even_without_owner() {
    let base = tempfile::tempdir().unwrap();
    let folder = base.path().join("org-a");
    write_file(&folder, SHA_ORPHAN, 10 * 60_000);
    let a = org("alfa", "{}");
    let fake = Fake::with_orgs(vec![(a.clone(), folder.clone())]);

    let report = run_org(&fake, a.id, &options_for(false)).unwrap();

    assert!(report.orphan_files.is_empty());
    assert!(folder.join(SHA_ORPHAN).exists());
}

#[test]
fn future_clock_moves_the_cutoff_back_by_the_org_days() {
    let a = org("alfa", r#"{"retencao":{"dias":7}}"#);
    let base = tempfile::tempdir().unwrap();
    let fake = Fake::with_orgs(vec![(a.clone(), base.path().to_path_buf())]);
    let two_years_ahead = NOW_MS + 730 * DAY_MS;
    let options = Options::new(two_years_ahead, 30, true, "system");

    let report = run_org(&fake, a.id, &options).unwrap();

    assert_eq!(report.cutoff_ms, Some(two_years_ahead - 7 * DAY_MS));
    assert_eq!(report.dias, Some(7));
}
