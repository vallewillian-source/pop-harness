use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::migrations::{HARNESS_MIGRATIONS, migrate};
use poph_store::{
    Conversa, ConversationStatus, HumanRecording, HumanResponse, NewAgent, NewConversation,
    NewHumanResponse, NewOrg, Org, Origin, Store,
};
use poph_types::{ConversationId, OrgId};
use rusqlite::{Connection, params};

struct Scenario {
    folder: PathBuf,
    store: Option<Store>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder =
            std::env::temp_dir().join(format!("poph-resposta-{label}-{}-{n}", std::process::id()));
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
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.folder);
    }
}

fn create_org(store: &Store, slug: &str) -> Org {
    store
        .orgs()
        .create(NewOrg {
            name: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
}

fn org_and_conversation(store: &Store, slug: &str) -> (Org, Conversa) {
    let org = create_org(store, slug);
    let agent = store
        .agents()
        .create(
            org.id,
            NewAgent {
                name: "Bia".to_owned(),
                slug: "bia".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente");
    let conversation = store
        .conversations()
        .create(
            org.id,
            NewConversation {
                agent_id: agent.id,
                integration_id: None,
                canal: "matrix".to_owned(),
                peer_externo: Some("!sala:matrix-dev.local".to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa");
    (org, conversation)
}

fn connection(store: &Store) -> Connection {
    Connection::open(store.harness_path()).expect("abre o banco para conferir")
}

fn count_rows(store: &Store, sql: &str, parametro: &str) -> i64 {
    let conn = connection(store);
    if parametro.is_empty() {
        conn.query_row(sql, [], |row| row.get(0)).expect("conta")
    } else {
        conn.query_row(sql, [parametro], |row| row.get(0))
            .expect("conta")
    }
}

fn write(
    store: &Store,
    org: OrgId,
    conversation: ConversationId,
    evento: &str,
    texto: &str,
) -> poph_store::error::Resultado<HumanResponse> {
    let sent = format!("*[Ana]*:\n{texto}");
    store.messages().register_response_human(
        org,
        conversation,
        NewHumanResponse {
            event_id: evento,
            raw_text: texto,
            sent_text: &sent,
            author_id: "@ph_ana:matrix-dev.local",
            author_name: "Ana",
        },
    )
}

#[test]
fn happy_recording_writes_message_output_passage_and_audit() {
    let scenario = Scenario::new("feliz");
    let store = scenario.store();
    let (org, conversation) = org_and_conversation(store, "feliz");
    assert_eq!(conversation.version, 1);

    let response = write(store, org.id, conversation.id, "$evento-1", "oi, sou a Ana")
        .expect("registra a resposta");
    let HumanResponse::Stored(recording) = response else {
        panic!("esperava Gravada, veio {response:?}");
    };
    let HumanRecording {
        mensagem,
        saida,
        just_transferred,
    } = *recording;

    assert_eq!(mensagem.origin, Origin::Human);
    assert_eq!(mensagem.author_name.as_deref(), Some("Ana"));
    assert_eq!(mensagem.external_id.as_deref(), Some("$evento-1"));
    assert_eq!(mensagem.texto, "oi, sou a Ana");
    assert_eq!(
        mensagem.turn_id,
        Some(saida.turn_id),
        "mensagem e saída ligadas pelo turno"
    );

    assert_eq!(saida.seq, 0);
    assert_eq!(saida.tipo, "text");
    let content: serde_json::Value =
        serde_json::from_str(&saida.content_json).expect("payload é JSON");
    assert_eq!(content["texto"], "*[Ana]*:\noi, sou a Ana");

    assert!(just_transferred);
    let later = store
        .conversations()
        .get(org.id, conversation.id)
        .expect("lê a conversa");
    assert_eq!(later.status, ConversationStatus::Transferred);
    assert_eq!(later.version, 2);

    let (origin, author): (String, Option<String>) = connection(store)
        .query_row(
            "SELECT origin, author_name FROM messages WHERE id = ?1",
            [mensagem.id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("lê a mensagem");
    assert_eq!((origin.as_str(), author.as_deref()), ("human", Some("Ana")));
    assert_eq!(
        count_rows(
            store,
            "SELECT count(*) FROM outbox WHERE conversation_id = ?1",
            &conversation.id.to_string()
        ),
        1
    );
    let (actor_kind, actor_identifier, acao, entity): (String, String, String, String) =
        connection(store)
            .query_row(
                "SELECT actor_kind, actor_id, action, entity FROM audit_log
              WHERE action = 'human_takeover'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("lê a auditoria");
    assert_eq!(actor_kind, "matrix_user");
    assert_eq!(actor_identifier, "@ph_ana:matrix-dev.local");
    assert_eq!(acao, "human_takeover");
    assert_eq!(entity, "conversation");
}

#[test]
fn repeated_event_returns_duplicate_without_second_output() {
    let scenario = Scenario::new("duplicada");
    let store = scenario.store();
    let (org, conversation) = org_and_conversation(store, "duplicada");

    let first = write(store, org.id, conversation.id, "$evento-2", "primeira vez").expect("grava");
    assert!(matches!(first, HumanResponse::Stored(_)));

    let repeated_item =
        write(store, org.id, conversation.id, "$evento-2", "outro texto").expect("repete");
    assert_eq!(repeated_item, HumanResponse::Duplicate);

    let conversation_id = conversation.id.to_string();
    assert_eq!(
        count_rows(
            store,
            "SELECT count(*) FROM outbox WHERE conversation_id = ?1",
            &conversation_id
        ),
        1,
        "a segunda chamada não cria segunda linha de saída"
    );
    assert_eq!(
        count_rows(
            store,
            "SELECT count(*) FROM messages WHERE conversation_id = ?1",
            &conversation_id
        ),
        1
    );
    assert_eq!(
        count_rows(
            store,
            "SELECT count(*) FROM audit_log WHERE action = 'human_takeover' AND entity_id = ?1",
            &conversation_id
        ),
        1,
        "a duplicada não grava auditoria de novo"
    );
}

#[test]
fn closed_conversation_writes_nothing() {
    let scenario = Scenario::new("fechada");
    let store = scenario.store();
    let (org, conversation) = org_and_conversation(store, "fechada");
    store
        .conversations()
        .change_status(
            org.id,
            conversation.id,
            conversation.version,
            ConversationStatus::Closed,
            None,
        )
        .expect("fecha a conversa");

    let response =
        write(store, org.id, conversation.id, "$evento-3", "tarde demais").expect("chama");
    assert_eq!(response, HumanResponse::ConversationClosed);

    let conversation_id = conversation.id.to_string();
    assert_eq!(
        store
            .messages()
            .count_rows(org.id, conversation.id)
            .expect("conta"),
        0
    );
    assert_eq!(
        count_rows(
            store,
            "SELECT count(*) FROM outbox WHERE conversation_id = ?1",
            &conversation_id
        ),
        0
    );
    assert_eq!(
        count_rows(
            store,
            "SELECT count(*) FROM audit_log WHERE action = 'human_takeover'",
            ""
        ),
        0
    );
}

#[test]
fn conversation_from_other_org_returns_missing() {
    let scenario = Scenario::new("outra-org");
    let store = scenario.store();
    let (org_a, conversation_a) = org_and_conversation(store, "org-a");
    let org_b = create_org(store, "org-b");

    let response = write(
        store,
        org_b.id,
        conversation_a.id,
        "$evento-4",
        "vazamento?",
    )
    .expect("chama");
    assert_eq!(response, HumanResponse::ConversationMissing);

    let conversation_id = conversation_a.id.to_string();
    assert_eq!(
        store
            .messages()
            .count_rows(org_a.id, conversation_a.id)
            .expect("conta"),
        0
    );
    assert_eq!(
        count_rows(
            store,
            "SELECT count(*) FROM outbox WHERE conversation_id = ?1",
            &conversation_id
        ),
        0
    );
    let later = store
        .conversations()
        .get(org_a.id, conversation_a.id)
        .expect("lê");
    assert_eq!(later.status, ConversationStatus::Open);
    assert_eq!(later.version, 1);
}

#[test]
fn failure_in_audit_undoes_message_output_and_passage() {
    let scenario = Scenario::new("falha");
    let store = scenario.store();
    let (org, conversation) = org_and_conversation(store, "falha");

    connection(store)
        .execute_batch(
            "CREATE TRIGGER recusa_takeover BEFORE INSERT ON audit_log
               WHEN NEW.action = 'human_takeover'
             BEGIN
               SELECT RAISE(ABORT, 'falha forçada no teste');
             END;",
        )
        .expect("cria o gatilho");

    let resultado = write(store, org.id, conversation.id, "$evento-5", "vai falhar");
    assert!(resultado.is_err(), "a falha da auditoria precisa aparecer");

    let conversation_id = conversation.id.to_string();
    assert_eq!(
        count_rows(
            store,
            "SELECT count(*) FROM messages WHERE external_id = '$evento-5'",
            ""
        ),
        0,
        "sem mensagem gravada"
    );
    assert_eq!(
        count_rows(
            store,
            "SELECT count(*) FROM outbox WHERE conversation_id = ?1",
            &conversation_id
        ),
        0,
        "sem linha de saída"
    );
    let later = store
        .conversations()
        .get(org.id, conversation.id)
        .expect("lê");
    assert_eq!(later.status, ConversationStatus::Open, "status não trocou");
    assert_eq!(later.version, 1, "versão não subiu");

    connection(store)
        .execute_batch("DROP TRIGGER recusa_takeover")
        .expect("remove o gatilho");
    let after_trigger = write(store, org.id, conversation.id, "$evento-5", "agora vai");
    assert!(matches!(after_trigger, Ok(HumanResponse::Stored(_))));
}

#[test]
fn already_transferred_conversation_stays_transferred_without_raising_version() {
    let scenario = Scenario::new("transferida");
    let store = scenario.store();
    let (org, conversation) = org_and_conversation(store, "transferida");
    store
        .conversations()
        .transfer(
            org.id,
            conversation.id,
            None,
            Some("sistema".to_owned()),
            "{}".to_owned(),
        )
        .expect("transfere");
    let before = store
        .conversations()
        .get(org.id, conversation.id)
        .expect("lê");
    assert_eq!(before.status, ConversationStatus::Transferred);
    assert_eq!(before.version, 2);

    let first = write(store, org.id, conversation.id, "$evento-6", "primeira").expect("grava");
    let HumanResponse::Stored(recording) = first else {
        panic!("esperava Gravada, veio {first:?}");
    };
    let just_transferred = recording.just_transferred;
    assert!(
        !just_transferred,
        "já estava transferida: esta resposta não transfere"
    );

    let second = write(store, org.id, conversation.id, "$evento-7", "segunda").expect("grava");
    assert!(matches!(second, HumanResponse::Stored(_)));

    let later = store
        .conversations()
        .get(org.id, conversation.id)
        .expect("lê");
    assert_eq!(later.status, ConversationStatus::Transferred);
    assert_eq!(later.version, 2, "a versão não sobe a cada resposta");
    assert_eq!(
        count_rows(
            store,
            "SELECT count(*) FROM outbox WHERE conversation_id = ?1",
            &conversation.id.to_string()
        ),
        2,
        "cada resposta nova entra na fila"
    );
}

#[test]
fn audit_migration_keeps_rows_and_accepts_matrix_user() {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let folder = std::env::temp_dir().join(format!("poph-resposta-mig-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).expect("cria a pasta");
    let path = folder.join("harness.db");

    {
        let conn = Connection::open(&path).expect("abre");
        conn.pragma_update(None, "foreign_keys", "ON")
            .expect("pragma");
        migrate(&conn, "harness.db", &path, &HARNESS_MIGRATIONS[..3], 1).expect("migra até a 3");
        for index in 0..3 {
            conn.execute(
                "INSERT INTO audit_log (id, org_id, actor_kind, actor_id, action, entity, entity_id,
                                        detail_json, created_at)
                 VALUES (?1, NULL, 'cli', NULL, 'antiga', 'agent', ?2, '{}', ?3)",
                params![format!("aud-{index}"), format!("ent-{index}"), 10 + index],
            )
            .expect("insere auditoria antiga");
        }
    }

    let conn = Connection::open(&path).expect("reabre");
    let report = migrate(&conn, "harness.db", &path, HARNESS_MIGRATIONS, 2).expect("migra");
    assert_eq!(report.version_after, poph_store::harness_target_version());

    let total: i64 = conn
        .query_row(
            "SELECT count(*) FROM audit_log WHERE action = 'antiga'",
            [],
            |l| l.get(0),
        )
        .expect("conta");
    assert_eq!(total, 3, "nenhuma linha de auditoria perdida");
    let integridade: String = conn
        .query_row("PRAGMA integrity_check", [], |l| l.get(0))
        .expect("integridade");
    assert_eq!(integridade, "ok");

    conn.execute(
        "INSERT INTO audit_log (id, org_id, actor_kind, actor_id, action, entity, entity_id,
                                detail_json, created_at)
         VALUES ('aud-novo', NULL, 'matrix_user', '@ph_ana:x', 'human_takeover', 'conversation',
                 'c', '{}', 20)",
        [],
    )
    .expect("matrix_user é aceito");

    let refused = conn.execute(
        "INSERT INTO audit_log (id, org_id, actor_kind, actor_id, action, entity, entity_id,
                                detail_json, created_at)
         VALUES ('aud-ruim', NULL, 'engine', NULL, 'x', 'y', NULL, '{}', 21)",
        [],
    );
    assert!(
        refused.is_err(),
        "actor_kind fora da lista continua recusado"
    );

    let indexes: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master
              WHERE type = 'index' AND name IN ('ix_audit_log_time', 'ix_audit_log_org')",
            [],
            |l| l.get(0),
        )
        .expect("índices");
    assert_eq!(indexes, 2);

    drop(conn);
    let _ = fs::remove_dir_all(&folder);
}

fn write_file(
    store: &Store,
    org: OrgId,
    conversation: ConversationId,
    evento: &str,
) -> poph_store::error::Resultado<HumanResponse> {
    store.messages().record_human_response_with_file(
        org,
        conversation,
        poph_store::HumanResponseWithFile {
            event_id: evento,
            author_id: "@ph_ana:matrix-dev.local",
            author_name: "Ana",
            file: poph_store::ResponseFile {
                name: "laudo.pdf",
                mime: "application/pdf",
                size_bytes: 1_024,
                sha256: &"a".repeat(64),
                blob_path: "org/sha",
                caption: "**[Ana]**",
                extraction_status: "skipped",
            },
        },
    )
}

#[test]
fn response_with_file_writes_message_attachment_queue_passage_and_audit() {
    let scenario = Scenario::new("arquivo-feliz");
    let store = scenario.store();
    let (org, conversation) = org_and_conversation(store, "arquivo");

    let resultado = write_file(
        store,
        org.id,
        conversation.id,
        "$evento-arquivo:matrix-dev.local",
    )
    .expect("grava");
    let HumanResponse::Stored(recording) = resultado else {
        panic!("esperava gravada, veio {resultado:?}");
    };

    assert_eq!(
        recording.mensagem.direction,
        poph_store::Direction::Outbound
    );
    assert_eq!(recording.mensagem.origin, Origin::Human);
    assert_eq!(recording.mensagem.texto, "");
    assert_eq!(
        recording.mensagem.external_id.as_deref(),
        Some("$evento-arquivo:matrix-dev.local")
    );
    assert_eq!(recording.mensagem.author_name.as_deref(), Some("Ana"));

    let attachments = store
        .messages()
        .attachments(org.id, recording.mensagem.id)
        .expect("lista os anexos");
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].name, "laudo.pdf");
    assert_eq!(attachments[0].mime, "application/pdf");
    assert_eq!(attachments[0].size_bytes, 1_024);
    assert_eq!(attachments[0].blob_path, "org/sha");
    assert_eq!(attachments[0].extraction_status, "skipped");
    assert_eq!(
        recording.mensagem.attachments_json,
        serde_json::json!([attachments[0].id.to_string()]).to_string()
    );

    assert_eq!(recording.saida.tipo, "file");
    let payload: serde_json::Value =
        serde_json::from_str(&recording.saida.content_json).expect("payload em JSON");
    assert_eq!(payload["blob_ref"], "org/sha");
    assert_eq!(payload["nome"], "laudo.pdf");
    assert_eq!(payload["mime"], "application/pdf");
    assert_eq!(payload["tamanho_bytes"], 1_024);
    assert_eq!(payload["sha256"], "a".repeat(64));
    assert_eq!(payload["legenda"], "**[Ana]**");
    assert_eq!(payload["arquivo_id"], attachments[0].id.to_string());

    assert!(recording.just_transferred);
    let conversation_read = store
        .conversations()
        .get(org.id, conversation.id)
        .expect("lê");
    assert_eq!(conversation_read.status, ConversationStatus::Transferred);
    assert_eq!(
        count_rows(
            store,
            "SELECT count(*) FROM audit_log WHERE action = 'human_takeover'",
            ""
        ),
        1
    );
}

#[test]
fn redelivery_of_same_file_returns_duplicate_without_second_attachment_nor_output() {
    let scenario = Scenario::new("arquivo-duplicado");
    let store = scenario.store();
    let (org, conversation) = org_and_conversation(store, "arquivo-dup");

    write_file(store, org.id, conversation.id, "$evento:matrix-dev.local").expect("primeira");
    let second = write_file(store, org.id, conversation.id, "$evento:matrix-dev.local")
        .expect("segunda não falha");
    assert_eq!(second, HumanResponse::Duplicate);

    assert_eq!(
        count_rows(store, "SELECT count(*) FROM messages", ""),
        1,
        "uma mensagem só"
    );
    assert_eq!(
        count_rows(store, "SELECT count(*) FROM attachments", ""),
        1,
        "um anexo só"
    );
    assert_eq!(
        count_rows(store, "SELECT count(*) FROM outbox WHERE kind = 'file'", ""),
        1,
        "um item de arquivo só"
    );
}

#[test]
fn closed_conversation_writes_no_file_nor_attachment() {
    let scenario = Scenario::new("arquivo-fechada");
    let store = scenario.store();
    let (org, conversation) = org_and_conversation(store, "arquivo-fechada");
    let version = store
        .conversations()
        .get(org.id, conversation.id)
        .expect("lê")
        .version;
    store
        .conversations()
        .close_audited(org.id, conversation.id, version, "teste".to_owned())
        .expect("encerra");

    let resultado =
        write_file(store, org.id, conversation.id, "$evento:matrix-dev.local").expect("responde");
    assert_eq!(resultado, HumanResponse::ConversationClosed);
    assert_eq!(count_rows(store, "SELECT count(*) FROM messages", ""), 0);
    assert_eq!(count_rows(store, "SELECT count(*) FROM attachments", ""), 0);
    assert_eq!(
        count_rows(store, "SELECT count(*) FROM outbox WHERE kind = 'file'", ""),
        0
    );
}
