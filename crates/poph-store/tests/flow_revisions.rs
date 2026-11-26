use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::{
    ACTION_PUBLISH, FlowRevision, Fluxo, NewFlow, NewOrg, Org, RevisionPublication, Store,
    StoreError,
};
use poph_types::{DomainError, FlowId, OrgId};
use rusqlite::Connection;

const SOURCE_1: &str = "fluxo(id = \"agendar_visita\")\n";
const SHA_1: &str = "966b85be2ca12cf68d88a35fd4dbfc31f514abdbd61bd05aeeb9977441469f3c";
const SOURCE_2: &str = "fluxo(id = \"agendar_visita\")\netapa(\"inicio\")";
const SHA_2: &str = "c12a81f87d880cd496796e09f68f93c803c1e0dd6d0ad341dac0a391694fa62f";
const IR_1: &str = "{\"schema_versao\":1,\"etapas\":[{\"id\":\"inicio\"}]}";
const IR_2: &str = "{\"schema_versao\":1,\"etapas\":[{\"id\":\"inicio\"},{\"id\":\"fim\"}]}";

struct Scenario {
    folder: PathBuf,
    store: Option<Store>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder =
            std::env::temp_dir().join(format!("poph-revisoes-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("cria a pasta do teste");
        let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
            .expect("abre o banco");
        Self {
            folder,
            store: Some(store),
        }
    }

    fn store(&self) -> &Store {
        self.store.as_ref().expect("store viva")
    }

    fn connection(&self) -> Connection {
        let conn = Connection::open(self.folder.join("harness.db")).expect("abre para conferir");
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .expect("pragmas");
        conn
    }

    fn org(&self, slug: &str) -> Org {
        self.store()
            .orgs()
            .create(NewOrg {
                name: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
    }

    fn flow(&self, org: &Org, slug: &str) -> Fluxo {
        self.store()
            .flows()
            .create(
                org.id,
                NewFlow {
                    slug: slug.to_owned(),
                    name: format!("Fluxo {slug}"),
                    description: String::new(),
                    rascunho_fonte: "rascunho ainda inválido".to_owned(),
                },
            )
            .expect("cria o fluxo")
    }

    fn publication(expected: u64, fonte: &str, ir: &str) -> RevisionPublication {
        RevisionPublication {
            expected,
            fonte: fonte.to_owned(),
            definicao_json: ir.to_owned(),
            schema_versao: Some(1),
            resolucoes_json: "{\"departamento_comercial\":\"dep-1\"}".to_owned(),
            ator_tipo: "admin_token".to_owned(),
            actor_id: Some("tok_1".to_owned()),
        }
    }

    fn publish(
        &self,
        org: &Org,
        flow: &Fluxo,
        expected: u64,
        fonte: &str,
        ir: &str,
    ) -> Result<FlowRevision, StoreError> {
        self.store().revisions().publish_revision(
            org.id,
            flow.id,
            Self::publication(expected, fonte, ir),
        )
    }

    fn flow_current(&self, org: &Org, flow: &Fluxo) -> Fluxo {
        self.store()
            .flows()
            .get(org.id, flow.id)
            .expect("lê o fluxo")
    }

    fn count_rows(&self, conn: &Connection, sql: &str, org: OrgId, flow: FlowId) -> i64 {
        conn.query_row(
            sql,
            rusqlite::params![org.to_string(), flow.to_string()],
            |row| row.get(0),
        )
        .expect("conta")
    }

    fn revisions(&self, conn: &Connection, org: &Org, flow: &Fluxo) -> i64 {
        self.count_rows(
            conn,
            "SELECT COUNT(*) FROM fluxo_revisoes WHERE org_id = ?1 AND fluxo_id = ?2",
            org.id,
            flow.id,
        )
    }

    fn audits(&self, conn: &Connection, org: &Org, flow: &Fluxo) -> i64 {
        self.count_rows(
            conn,
            "SELECT COUNT(*) FROM audit_log
              WHERE org_id = ?1 AND entity_id = ?2 AND action = 'flow_publish'",
            org.id,
            flow.id,
        )
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        self.store = None;
        let _ = fs::remove_dir_all(&self.folder);
    }
}

#[test]
fn publishing_writes_revision_one_and_updates_flow() {
    let scenario = Scenario::new("primeira");
    let org = scenario.org("a");
    let flow = scenario.flow(&org, "agendar_visita");

    let revision = scenario
        .publish(&org, &flow, 1, SOURCE_1, IR_1)
        .expect("publica a revisão 1");

    assert_eq!(revision.numero, 1);
    assert_eq!(revision.fonte, SOURCE_1);
    assert_eq!(revision.fonte_sha256, SHA_1, "o hash é do texto exato");
    assert_eq!(revision.definicao_json, IR_1);
    assert_eq!(revision.schema_versao, 1);
    assert_eq!(revision.publicada_por, "admin_token");
    assert_eq!(
        revision.resolucoes_json,
        "{\"departamento_comercial\":\"dep-1\"}"
    );
    assert!(revision.publicada_em > 0);
    assert_eq!(revision.org_id, org.id);
    assert_eq!(revision.fluxo_id, flow.id);

    let later = scenario.flow_current(&org, &flow);
    assert_eq!(later.revisao_atual, Some(1));
    assert_eq!(
        later.rascunho_fonte, SOURCE_1,
        "o rascunho vira o publicado"
    );
    assert_eq!(later.version, 2, "publicar avança a versão do fluxo");

    let conn = scenario.connection();
    let (id, fonte, sha, ir): (String, String, String, String) = conn
        .query_row(
            "SELECT id, fonte, fonte_sha256, definicao_json FROM fluxo_revisoes
              WHERE org_id = ?1 AND fluxo_id = ?2 AND numero = 1",
            rusqlite::params![org.id.to_string(), flow.id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("lê a revisão gravada");
    assert_eq!(id, revision.id.to_string());
    assert_eq!(fonte, SOURCE_1);
    assert_eq!(sha, SHA_1);
    assert_eq!(ir, IR_1);
}

#[test]
fn publication_audit_keeps_action_entity_and_detail() {
    let scenario = Scenario::new("auditoria");
    let org = scenario.org("a");
    let flow = scenario.flow(&org, "agendar_visita");
    let revision = scenario
        .publish(&org, &flow, 1, SOURCE_1, IR_1)
        .expect("publica");

    let conn = scenario.connection();
    let (acao, entity, ator_tipo, actor_id, detail): (
        String,
        String,
        String,
        Option<String>,
        String,
    ) = conn
        .query_row(
            "SELECT action, entity, actor_kind, actor_id, detail_json FROM audit_log
              WHERE org_id = ?1 AND entity_id = ?2",
            rusqlite::params![org.id.to_string(), flow.id.to_string()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("lê a auditoria");
    assert_eq!(acao, ACTION_PUBLISH);
    assert_eq!(acao, "flow_publish");
    assert_eq!(entity, "flow");
    assert_eq!(ator_tipo, "admin_token");
    assert_eq!(actor_id.as_deref(), Some("tok_1"));
    for expected_value in [
        "\"fonte_sha256\":\"966b85be2ca12cf68d88a35fd4dbfc31f514abdbd61bd05aeeb9977441469f3c\"",
        "\"revisao\":1",
        "\"schema_versao\":1",
    ] {
        assert!(
            detail.contains(expected_value),
            "o detalhe da auditoria tem `{expected_value}`: {detail}"
        );
    }
    assert!(
        detail.contains(&format!("\"revisao_id\":\"{}\"", revision.id)),
        "o detalhe aponta a revisão publicada: {detail}"
    );
}

#[test]
fn two_publications_with_same_expected_version_only_one_wins() {
    let scenario = Scenario::new("corrida");
    let org = scenario.org("a");
    let flow = scenario.flow(&org, "agendar_visita");

    let first = scenario
        .publish(&org, &flow, 1, SOURCE_1, IR_1)
        .expect("a primeira vence");
    assert_eq!(first.numero, 1);

    let error = scenario
        .publish(&org, &flow, 1, SOURCE_2, IR_2)
        .expect_err("a segunda perde");
    assert!(
        matches!(
            error,
            StoreError::Domain(DomainError::VersionConflict {
                entity: "fluxo",
                expected: 1,
                current: 2,
                ..
            })
        ),
        "esperava conflito de versão em fluxo, veio {error:?}"
    );

    let conn = scenario.connection();
    assert_eq!(
        scenario.revisions(&conn, &org, &flow),
        1,
        "sem revisão órfã"
    );
    assert_eq!(
        scenario.audits(&conn, &org, &flow),
        1,
        "sem auditoria da perdedora"
    );
    let later = scenario.flow_current(&org, &flow);
    assert_eq!(later.revisao_atual, Some(1));
    assert_eq!(later.version, 2, "a versão avançou uma vez só");
    assert_eq!(
        later.rascunho_fonte, SOURCE_1,
        "o rascunho é o da vencedora"
    );
}

#[test]
fn two_threads_publishing_with_same_version_only_one_wins() {
    let scenario = Scenario::new("corrida-threads");
    let org = scenario.org("a");
    let flow = scenario.flow(&org, "agendar_visita");
    let store = Arc::new(
        Store::open_at(
            &scenario.folder.join("harness.db"),
            &scenario.folder.join("rag.db"),
        )
        .expect("abre um segundo store no mesmo banco"),
    );

    let mut handles = Vec::new();
    for (index, fonte) in [(1, SOURCE_1), (2, SOURCE_2)] {
        let store = Arc::clone(&store);
        let org_id = org.id;
        let fluxo_id = flow.id;
        handles.push(std::thread::spawn(move || {
            store
                .revisions()
                .publish_revision(
                    org_id,
                    fluxo_id,
                    Scenario::publication(1, fonte, if index == 1 { IR_1 } else { IR_2 }),
                )
                .map(|revision| revision.numero)
        }));
    }
    let results: Vec<Result<u64, StoreError>> = handles
        .into_iter()
        .map(|handle| handle.join().expect("a thread termina"))
        .collect();

    let vencedoras = results.iter().filter(|resultado| resultado.is_ok()).count();
    let perdedoras = results
        .iter()
        .filter(|resultado| {
            matches!(
                resultado,
                Err(StoreError::Domain(DomainError::VersionConflict { .. }))
            )
        })
        .count();
    assert_eq!(vencedoras, 1, "só uma publicação vence: {results:?}");
    assert_eq!(perdedoras, 1, "a outra recebe 412: {results:?}");

    let conn = scenario.connection();
    assert_eq!(scenario.revisions(&conn, &org, &flow), 1);
    assert_eq!(scenario.audits(&conn, &org, &flow), 1);
}

#[test]
fn revision_one_stays_identical_after_two() {
    let scenario = Scenario::new("imutavel");
    let org = scenario.org("a");
    let flow = scenario.flow(&org, "agendar_visita");
    let first = scenario
        .publish(&org, &flow, 1, SOURCE_1, IR_1)
        .expect("publica a 1");
    let second = scenario
        .publish(&org, &flow, 2, SOURCE_2, IR_2)
        .expect("publica a 2");

    assert_eq!(first.numero, 1);
    assert_eq!(second.numero, 2, "as duas coexistem, numeradas em ordem");
    assert_eq!(second.fonte, SOURCE_2);
    assert_eq!(second.fonte_sha256, SHA_2);
    assert_ne!(first.id, second.id);

    let relida = scenario
        .store()
        .revisions()
        .get(org.id, flow.id, 1)
        .expect("lê a revisão 1");
    assert_eq!(relida, first);
    assert_eq!(relida.fonte, SOURCE_1);
    assert_eq!(relida.fonte_sha256, SHA_1);
    assert_eq!(relida.definicao_json, IR_1);

    let conn = scenario.connection();
    let (fonte, sha, ir): (String, String, String) = conn
        .query_row(
            "SELECT fonte, fonte_sha256, definicao_json FROM fluxo_revisoes
              WHERE org_id = ?1 AND fluxo_id = ?2 AND numero = 1",
            rusqlite::params![org.id.to_string(), flow.id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("lê a revisão 1 gravada");
    assert_eq!(fonte, SOURCE_1);
    assert_eq!(sha, SHA_1);
    assert_eq!(ir, IR_1);

    let later = scenario.flow_current(&org, &flow);
    assert_eq!(later.revisao_atual, Some(2), "a atual passa a ser a 2");
    assert_eq!(later.version, 3);
}

#[test]
fn changing_or_deleting_published_revision_is_refused_by_database() {
    let scenario = Scenario::new("protegida");
    let org = scenario.org("a");
    let flow = scenario.flow(&org, "agendar_visita");
    let revision = scenario
        .publish(&org, &flow, 1, SOURCE_1, IR_1)
        .expect("publica");
    let conn = scenario.connection();
    let text_id = revision.id.to_string();

    let error = conn
        .execute(
            "UPDATE fluxo_revisoes SET fonte = 'outra coisa' WHERE id = ?1",
            rusqlite::params![text_id],
        )
        .expect_err("o gatilho recusa alterar revisão publicada");
    assert!(
        format!("{error}").contains("tr_fluxo_revisoes_sem_update")
            || format!("{error}").contains("imutável"),
        "o erro vem do gatilho de imutabilidade: {error}"
    );

    let error = conn
        .execute(
            "DELETE FROM fluxo_revisoes WHERE id = ?1",
            rusqlite::params![text_id],
        )
        .expect_err("o gatilho recusa apagar revisão publicada");
    assert!(
        format!("{error}").contains("tr_fluxo_revisoes_sem_delete")
            || format!("{error}").contains("imutável"),
        "o erro vem do gatilho de imutabilidade: {error}"
    );

    let relida = scenario
        .store()
        .revisions()
        .get(org.id, flow.id, 1)
        .expect("a revisão continua lá");
    assert_eq!(relida, revision);

    conn.execute_batch(
        "DROP TRIGGER tr_fluxo_revisoes_sem_update;
         DROP TRIGGER tr_fluxo_revisoes_sem_delete;",
    )
    .expect("derruba os gatilhos");
    let changed = conn
        .execute(
            "UPDATE fluxo_revisoes SET fonte = 'outra coisa' WHERE id = ?1",
            rusqlite::params![revision.id.to_string()],
        )
        .expect("sem o gatilho, a alteração passa");
    assert_eq!(changed, 1);
}

#[test]
fn failure_after_audit_leaves_no_revision_nor_audit() {
    let scenario = Scenario::new("atomica");
    let org = scenario.org("a");
    let flow = scenario.flow(&org, "agendar_visita");

    {
        let conn = scenario.connection();
        conn.execute_batch(
            "CREATE TRIGGER falha_injetada BEFORE INSERT ON fluxo_revisoes
             BEGIN SELECT RAISE(ABORT, 'falha injetada no teste'); END;",
        )
        .expect("cria o gatilho da falha");
    }

    let error = scenario
        .publish(&org, &flow, 1, SOURCE_1, IR_1)
        .expect_err("a publicação falha no meio");
    assert!(
        format!("{error}").contains("falha injetada"),
        "o erro do gatilho aparece: {error}"
    );

    let conn = scenario.connection();
    conn.execute_batch("DROP TRIGGER falha_injetada;")
        .expect("remove o gatilho");

    assert_eq!(scenario.revisions(&conn, &org, &flow), 0, "sem revisão");
    assert_eq!(scenario.audits(&conn, &org, &flow), 0, "sem auditoria");
    let later = scenario.flow_current(&org, &flow);
    assert_eq!(later.revisao_atual, None, "revisão atual continua nula");
    assert_eq!(later.version, 1, "a versão não avançou");
    assert_eq!(
        later.rascunho_fonte, "rascunho ainda inválido",
        "o rascunho não foi trocado"
    );

    let revision = scenario
        .publish(&org, &flow, 1, SOURCE_1, IR_1)
        .expect("depois da falha, publica");
    assert_eq!(revision.numero, 1);
}

#[test]
fn ir_without_schema_version_is_refused_without_writing_anything() {
    let scenario = Scenario::new("sem-schema");
    let org = scenario.org("a");
    let flow = scenario.flow(&org, "agendar_visita");

    let mut without_version = Scenario::publication(1, SOURCE_1, IR_1);
    without_version.schema_versao = None;
    let error = scenario
        .store()
        .revisions()
        .publish_revision(org.id, flow.id, without_version)
        .expect_err("sem schema_versao não publica");
    assert!(
        matches!(
            error,
            StoreError::Domain(DomainError::Validation {
                field: "schema_versao",
                ..
            })
        ),
        "esperava validação de schema_versao, veio {error:?}"
    );

    let mut version_zero = Scenario::publication(1, SOURCE_1, IR_1);
    version_zero.schema_versao = Some(0);
    let error = scenario
        .store()
        .revisions()
        .publish_revision(org.id, flow.id, version_zero)
        .expect_err("schema_versao zero não publica");
    assert!(
        matches!(
            error,
            StoreError::Domain(DomainError::Validation {
                field: "schema_versao",
                ..
            })
        ),
        "esperava validação de schema_versao, veio {error:?}"
    );

    let conn = scenario.connection();
    assert_eq!(scenario.revisions(&conn, &org, &flow), 0);
    assert_eq!(scenario.audits(&conn, &org, &flow), 0);
    assert_eq!(scenario.flow_current(&org, &flow).version, 1);
}

#[test]
fn incoherent_ir_empty_source_and_unknown_actor_are_refused() {
    let scenario = Scenario::new("recusas");
    let org = scenario.org("a");
    let flow = scenario.flow(&org, "agendar_visita");

    fn cases() -> Vec<(&'static str, &'static str, RevisionPublication)> {
        let mut ir_torta = Scenario::publication(1, SOURCE_1, IR_1);
        ir_torta.definicao_json = "ir torta".to_owned();
        let mut ir_of_other_version = Scenario::publication(1, SOURCE_1, IR_1);
        ir_of_other_version.definicao_json = "{\"schema_versao\":2}".to_owned();
        let mut without_text = Scenario::publication(1, SOURCE_1, IR_1);
        without_text.fonte = "   ".to_owned();
        let mut actor_unknown = Scenario::publication(1, SOURCE_1, IR_1);
        actor_unknown.ator_tipo = "robo".to_owned();
        let mut resolutions_that_are_not_objects = Scenario::publication(1, SOURCE_1, IR_1);
        resolutions_that_are_not_objects.resolucoes_json = "[1, 2]".to_owned();
        vec![
            ("IR que não é JSON", "definicao_json", ir_torta),
            (
                "IR que declara outra versão de esquema",
                "schema_versao",
                ir_of_other_version,
            ),
            ("texto em branco", "fonte", without_text),
            ("ator fora do vocabulário", "ator_tipo", actor_unknown),
            (
                "resoluções que não são objeto",
                "resolucoes_json",
                resolutions_that_are_not_objects,
            ),
        ]
    }

    for (label, campo, publication) in cases() {
        let error = scenario
            .store()
            .revisions()
            .publish_revision(org.id, flow.id, publication)
            .expect_err("a publicação devia ser recusada");
        match error {
            StoreError::Domain(DomainError::Validation {
                field: named,
                reason: motivo,
            }) => {
                assert_eq!(named, campo, "campo recusado em `{label}`");
                assert!(!motivo.is_empty(), "`{label}` diz o motivo");
                if campo == "ator_tipo" {
                    assert!(
                        motivo.contains("admin_token"),
                        "a recusa do ator lista o vocabulário: {motivo}"
                    );
                }
            }
            another => panic!("`{label}` devia ser validação de `{campo}`, veio {another:?}"),
        }
    }

    let conn = scenario.connection();
    assert_eq!(scenario.revisions(&conn, &org, &flow), 0);
    assert_eq!(scenario.audits(&conn, &org, &flow), 0);
    assert_eq!(scenario.flow_current(&org, &flow).version, 1);
}

#[test]
fn empty_resolutions_become_empty_object() {
    let scenario = Scenario::new("resolucoes");
    let org = scenario.org("a");
    let flow = scenario.flow(&org, "agendar_visita");

    let mut publication = Scenario::publication(1, SOURCE_1, IR_1);
    publication.resolucoes_json = "  ".to_owned();
    let revision = scenario
        .store()
        .revisions()
        .publish_revision(org.id, flow.id, publication)
        .expect("publica");
    assert_eq!(revision.resolucoes_json, "{}");
}

#[test]
fn reads_revision_by_number_current_and_list() {
    let scenario = Scenario::new("leitura");
    let org = scenario.org("a");
    let flow = scenario.flow(&org, "agendar_visita");

    assert_eq!(
        scenario
            .store()
            .revisions()
            .current(org.id, flow.id)
            .expect("consulta"),
        None
    );
    assert!(
        scenario
            .store()
            .revisions()
            .list(org.id, flow.id, None, 10)
            .expect("lista")
            .is_empty()
    );

    let first = scenario.publish(&org, &flow, 1, SOURCE_1, IR_1).expect("1");
    let second = scenario.publish(&org, &flow, 2, SOURCE_2, IR_2).expect("2");

    assert_eq!(
        scenario
            .store()
            .revisions()
            .get(org.id, flow.id, 1)
            .expect("por número"),
        first
    );
    assert_eq!(
        scenario
            .store()
            .revisions()
            .current(org.id, flow.id)
            .expect("atual")
            .expect("tem atual"),
        second
    );
    let error = scenario
        .store()
        .revisions()
        .get(org.id, flow.id, 9)
        .expect_err("a 9 não existe");
    assert!(
        matches!(
            error,
            StoreError::Domain(DomainError::NotFound {
                entity: "revisão de fluxo",
                ..
            })
        ),
        "esperava não encontrado, veio {error:?}"
    );

    let every = scenario
        .store()
        .revisions()
        .list(org.id, flow.id, None, 10)
        .expect("lista");
    assert_eq!(every, vec![first.clone(), second.clone()]);
    let after_first = scenario
        .store()
        .revisions()
        .list(org.id, flow.id, Some(1), 10)
        .expect("lista com cursor");
    assert_eq!(after_first, vec![second]);
}

#[test]
fn flow_from_other_org_is_neither_read_nor_published() {
    let scenario = Scenario::new("org");
    let org_a = scenario.org("a");
    let org_b = scenario.org("b");
    let flow = scenario.flow(&org_a, "agendar_visita");
    scenario
        .publish(&org_a, &flow, 1, SOURCE_1, IR_1)
        .expect("1");

    let error = scenario
        .publish(&org_b, &flow, 2, SOURCE_2, IR_2)
        .expect_err("publicar em outra org é recusado");
    assert!(error.is_not_found(), "esperava não encontrado: {error:?}");

    for error in [
        scenario
            .store()
            .revisions()
            .get(org_b.id, flow.id, 1)
            .expect_err("obter em outra org"),
        scenario
            .store()
            .revisions()
            .current(org_b.id, flow.id)
            .expect_err("atual em outra org"),
        scenario
            .store()
            .revisions()
            .list(org_b.id, flow.id, None, 10)
            .expect_err("listar em outra org"),
    ] {
        match error {
            StoreError::Domain(DomainError::NotFound { entity, .. }) => {
                assert_eq!(entity, "fluxo", "a recusa é do fluxo");
            }
            another => panic!("esperava não encontrado do fluxo, veio {another:?}"),
        }
    }

    let conn = scenario.connection();
    assert_eq!(scenario.revisions(&conn, &org_a, &flow), 1);
    assert_eq!(scenario.flow_current(&org_a, &flow).revisao_atual, Some(1));
}
