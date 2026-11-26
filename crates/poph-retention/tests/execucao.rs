//! Execução da retenção contra um armazenamento falso (o banco real entra com o adaptador).
//! Os arquivos são de verdade, numa pasta temporária por org.

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

const SHA_ORFAO: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
const SHA_VIVO: &str = "0f0e0d0c0b0a09080706050403020100ffeeddccbbaa99887766554433221100";
const AGORA: i64 = 1_795_131_511_000;

struct Falso {
    orgs: Vec<OrgRetention>,
    pastas: HashMap<OrgId, PathBuf>,
    varreduras: HashMap<OrgId, Result<DatabaseSweep, RetentionError>>,
    chamadas: RefCell<Vec<(OrgId, i64, bool)>>,
    auditorias: RefCell<Vec<(OrgId, String, String)>>,
    auditoria_falha: bool,
}

impl Falso {
    fn com_orgs(orgs: Vec<(OrgRetention, PathBuf)>) -> Self {
        let mut pastas = HashMap::new();
        let mut lista = Vec::new();
        for (org, pasta) in orgs {
            pastas.insert(org.id, pasta);
            lista.push(org);
        }
        Self {
            orgs: lista,
            pastas,
            varreduras: HashMap::new(),
            chamadas: RefCell::new(Vec::new()),
            auditorias: RefCell::new(Vec::new()),
            auditoria_falha: false,
        }
    }
}

impl Warehouse for Falso {
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
        corte_ms: i64,
        simular: bool,
    ) -> Result<DatabaseSweep, RetentionError> {
        self.chamadas.borrow_mut().push((org, corte_ms, simular));
        self.varreduras
            .get(&org)
            .cloned()
            .unwrap_or_else(|| Ok(DatabaseSweep::default()))
    }
    fn blob_folder(&self, org: OrgId) -> PathBuf {
        self.pastas[&org].clone()
    }
    fn record_audit(
        &self,
        org: OrgId,
        ator_tipo: &str,
        detalhe_json: &str,
    ) -> Result<(), RetentionError> {
        if self.auditoria_falha {
            return Err(RetentionError::Database("disco cheio".to_owned()));
        }
        self.auditorias
            .borrow_mut()
            .push((org, ator_tipo.to_owned(), detalhe_json.to_owned()));
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

/// Grava um arquivo com a idade pedida (ms antes de AGORA).
fn gravar(pasta: &Path, nome: &str, idade_ms: i64) {
    fs::create_dir_all(pasta).unwrap();
    let caminho = pasta.join(nome);
    fs::write(&caminho, b"bytes").unwrap();
    let instante = UNIX_EPOCH + Duration::from_millis((AGORA - idade_ms) as u64);
    fs::OpenOptions::new()
        .write(true)
        .open(&caminho)
        .unwrap()
        .set_modified(instante)
        .unwrap();
}

fn opcoes(simular: bool) -> Options {
    Options::new(AGORA, 30, simular, "cli")
}

#[test]
fn dry_run_lista_o_orfao_e_nao_apaga_nada() {
    let base = tempfile::tempdir().unwrap();
    let pasta = base.path().join("org-a");
    gravar(&pasta, SHA_ORFAO, 90 * DAY_MS);
    let a = org("alfa", "{}");
    let mut falso = Falso::com_orgs(vec![(a.clone(), pasta.clone())]);
    falso.varreduras.insert(
        a.id,
        Ok(DatabaseSweep {
            messages: 12,
            anexos: 2,
            conversation_ids: vec!["c1".into()],
            ..Default::default()
        }),
    );

    let relatorio = run_org(&falso, a.id, &opcoes(true)).unwrap();

    assert_eq!(relatorio.situacao, Situacao::Swept);
    assert_eq!(relatorio.orphan_files, vec![SHA_ORFAO.to_owned()]);
    assert_eq!(relatorio.deleted_files, 0);
    assert!(
        pasta.join(SHA_ORFAO).exists(),
        "dry-run não pode apagar arquivo"
    );
    let chamada = falso.chamadas.borrow()[0];
    assert!(chamada.2, "o banco recebe a simulação");
    assert_eq!(chamada.1, AGORA - 30 * DAY_MS);
    let (_, _, detalhe) = &falso.auditorias.borrow()[0];
    assert!(detalhe.contains("\"simulacao\":true"), "{detalhe}");
    assert!(detalhe.contains("\"mensagens\":12"), "{detalhe}");
}

#[test]
fn execucao_apaga_so_o_orfao_e_audita_as_contagens() {
    let base = tempfile::tempdir().unwrap();
    let pasta = base.path().join("org-a");
    gravar(&pasta, SHA_ORFAO, 90 * DAY_MS);
    gravar(&pasta, SHA_VIVO, 90 * DAY_MS);
    let a = org("alfa", "{}");
    let mut falso = Falso::com_orgs(vec![(a.clone(), pasta.clone())]);
    falso.varreduras.insert(
        a.id,
        Ok(DatabaseSweep {
            messages: 5,
            anexos: 1,
            turns: 3,
            inbox: 4,
            outbox: 6,
            conversas: 1,
            referenced_hashes: vec![SHA_VIVO.to_owned()],
            ..Default::default()
        }),
    );

    let relatorio = run_org(&falso, a.id, &opcoes(false)).unwrap();

    assert_eq!(relatorio.situacao, Situacao::Swept);
    assert_eq!(relatorio.deleted_files, 1);
    assert!(!pasta.join(SHA_ORFAO).exists());
    assert!(pasta.join(SHA_VIVO).exists(), "arquivo com dono fica");
    let (_, ator, detalhe) = falso.auditorias.borrow()[0].clone();
    assert_eq!(ator, "cli");
    let json: serde_json::Value = serde_json::from_str(&detalhe).unwrap();
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
fn retencao_zero_pula_a_org_sem_chamar_o_banco() {
    let base = tempfile::tempdir().unwrap();
    let pasta = base.path().join("org-a");
    gravar(&pasta, SHA_ORFAO, 900 * DAY_MS);
    let a = org("alfa", r#"{"retencao":{"dias":0}}"#);
    let falso = Falso::com_orgs(vec![(a.clone(), pasta.clone())]);

    let relatorio = run_org(&falso, a.id, &opcoes(false)).unwrap();

    assert_eq!(relatorio.situacao, Situacao::Skipped);
    assert!(
        falso.chamadas.borrow().is_empty(),
        "org com 0 não vai ao banco"
    );
    assert!(
        pasta.join(SHA_ORFAO).exists(),
        "org com 0 não perde arquivo"
    );
    assert!(falso.auditorias.borrow().is_empty());
}

#[test]
fn config_invalida_pula_so_aquela_org() {
    let base = tempfile::tempdir().unwrap();
    let pasta_ruim = base.path().join("ruim");
    let pasta_boa = base.path().join("boa");
    gravar(&pasta_ruim, SHA_ORFAO, 90 * DAY_MS);
    gravar(&pasta_boa, SHA_ORFAO, 90 * DAY_MS);
    let ruim = org("ruim", r#"{"retencao":{"dias":-5}}"#);
    let boa = org("boa", "{}");
    let falso = Falso::com_orgs(vec![
        (ruim.clone(), pasta_ruim.clone()),
        (boa.clone(), pasta_boa.clone()),
    ]);

    let relatorio = run_all(&falso, &opcoes(false)).unwrap();

    let de_ruim = relatorio.orgs.iter().find(|o| o.slug == "ruim").unwrap();
    let de_boa = relatorio.orgs.iter().find(|o| o.slug == "boa").unwrap();
    assert_eq!(de_ruim.situacao, Situacao::Failed);
    assert!(
        de_ruim
            .motivo
            .as_deref()
            .unwrap()
            .contains("não pode ser negativo")
    );
    assert!(
        pasta_ruim.join(SHA_ORFAO).exists(),
        "org ruim não perde nada"
    );
    assert_eq!(de_boa.situacao, Situacao::Swept);
    assert!(!pasta_boa.join(SHA_ORFAO).exists());
    assert!(falso.chamadas.borrow().iter().all(|c| c.0 != ruim.id));
}

#[test]
fn falha_no_banco_nao_apaga_arquivo_nenhum() {
    let base = tempfile::tempdir().unwrap();
    let pasta = base.path().join("org-a");
    gravar(&pasta, SHA_ORFAO, 90 * DAY_MS);
    let a = org("alfa", "{}");
    let mut falso = Falso::com_orgs(vec![(a.clone(), pasta.clone())]);
    falso.varreduras.insert(
        a.id,
        Err(RetentionError::Database("banco ocupado".to_owned())),
    );

    let relatorio = run_org(&falso, a.id, &opcoes(false)).unwrap();

    assert_eq!(relatorio.situacao, Situacao::Failed);
    assert!(pasta.join(SHA_ORFAO).exists(), "banco falhou: arquivo fica");
    assert!(falso.auditorias.borrow().is_empty());
}

#[test]
fn falha_na_auditoria_aparece_no_relatorio() {
    let base = tempfile::tempdir().unwrap();
    let pasta = base.path().join("org-a");
    gravar(&pasta, SHA_ORFAO, 90 * DAY_MS);
    let a = org("alfa", "{}");
    let mut falso = Falso::com_orgs(vec![(a.clone(), pasta.clone())]);
    falso.auditoria_falha = true;

    let relatorio = run_org(&falso, a.id, &opcoes(false)).unwrap();

    assert_eq!(relatorio.situacao, Situacao::Failed);
    assert!(
        relatorio
            .motivo
            .as_deref()
            .unwrap()
            .contains("auditoria não gravada")
    );
}

#[test]
fn org_nao_toca_a_pasta_de_outra_org() {
    let base = tempfile::tempdir().unwrap();
    let pasta_a = base.path().join("org-a");
    let pasta_b = base.path().join("org-b");
    gravar(&pasta_a, SHA_ORFAO, 90 * DAY_MS);
    gravar(&pasta_b, SHA_ORFAO, 90 * DAY_MS);
    let a = org("alfa", "{}");
    let b = org("beta", "{}");
    let falso = Falso::com_orgs(vec![
        (a.clone(), pasta_a.clone()),
        (b.clone(), pasta_b.clone()),
    ]);

    run_org(&falso, a.id, &opcoes(false)).unwrap();

    assert!(!pasta_a.join(SHA_ORFAO).exists());
    assert!(
        pasta_b.join(SHA_ORFAO).exists(),
        "a org B não é varrida nesta chamada"
    );
    assert!(falso.chamadas.borrow().iter().all(|c| c.0 == a.id));
}

#[test]
fn arquivo_recente_fica_mesmo_sem_dono() {
    let base = tempfile::tempdir().unwrap();
    let pasta = base.path().join("org-a");
    gravar(&pasta, SHA_ORFAO, 10 * 60_000); // 10 minutos
    let a = org("alfa", "{}");
    let falso = Falso::com_orgs(vec![(a.clone(), pasta.clone())]);

    let relatorio = run_org(&falso, a.id, &opcoes(false)).unwrap();

    assert!(relatorio.orphan_files.is_empty());
    assert!(pasta.join(SHA_ORFAO).exists());
}

#[test]
fn relogio_no_futuro_recua_o_corte_pelos_dias_da_org() {
    let a = org("alfa", r#"{"retencao":{"dias":7}}"#);
    let base = tempfile::tempdir().unwrap();
    let falso = Falso::com_orgs(vec![(a.clone(), base.path().to_path_buf())]);
    let daqui_dois_anos = AGORA + 730 * DAY_MS;
    let opcoes = Options::new(daqui_dois_anos, 30, true, "system");

    let relatorio = run_org(&falso, a.id, &opcoes).unwrap();

    assert_eq!(relatorio.cutoff_ms, Some(daqui_dois_anos - 7 * DAY_MS));
    assert_eq!(relatorio.dias, Some(7));
}
