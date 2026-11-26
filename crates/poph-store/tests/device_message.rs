use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::{ConversationStatus, NewAgent, NewDeviceMessage, NewOrg, Origin, Received, Store};
use poph_types::{AgentId, IntegrationId, OrgId, TurnId};
use rusqlite::Connection;

struct Scenario {
    folder: PathBuf,
    store: Option<Store>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder =
            std::env::temp_dir().join(format!("poph-aparelho-{label}-{}-{n}", std::process::id()));
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

struct OrgReady {
    org: OrgId,
    agent: AgentId,
    integration: IntegrationId,
}

fn org_ready(store: &Store, slug: &str) -> OrgReady {
    let org = store
        .orgs()
        .create(NewOrg {
            name: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id;
    let agent = store
        .agents()
        .create(
            org,
            NewAgent {
                name: "Bia".to_owned(),
                slug: "bia".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente")
        .id;
    let integration: IntegrationId = store
        .create_integration(org, agent, "whatsapp".to_owned(), "active".to_owned())
        .expect("cria a integração")
        .id
        .parse()
        .expect("id da integração");
    OrgReady {
        org,
        agent,
        integration,
    }
}

fn fresh(ready: &OrgReady, external_id: &str, texto: &str) -> NewDeviceMessage {
    NewDeviceMessage {
        integration_id: ready.integration,
        agent_id: ready.agent,
        canal: "whatsapp".to_owned(),
        peer: "5511999990000".to_owned(),
        external_id: external_id.to_owned(),
        texto: texto.to_owned(),
    }
}

fn connection(store: &Store) -> Connection {
    Connection::open(store.harness_path()).expect("abre o banco para conferir")
}

fn count_rows(store: &Store, sql: &str) -> i64 {
    connection(store)
        .query_row(sql, [], |row| row.get(0))
        .expect("conta")
}

fn single_line(store: &Store, sql: &str, parametro: &str) -> Vec<Option<String>> {
    connection(store)
        .query_row(sql, [parametro], |row| {
            let mut values = Vec::new();
            for i in 0..row.as_ref().column_count() {
                values.push(match row.get_ref(i)? {
                    rusqlite::types::ValueRef::Null => None,
                    rusqlite::types::ValueRef::Integer(n) => Some(n.to_string()),
                    rusqlite::types::ValueRef::Real(n) => Some(n.to_string()),
                    rusqlite::types::ValueRef::Text(t) => {
                        Some(String::from_utf8_lossy(t).into_owned())
                    }
                    rusqlite::types::ValueRef::Blob(_) => Some("<blob>".to_owned()),
                });
            }
            Ok(values)
        })
        .expect("lê a linha")
}

const SQL_MESSAGE: &str =
    "SELECT direction, role, origin, turn_id FROM messages WHERE external_id = ?1";

#[test]
fn writes_device_message_and_pauses_agent() {
    let scenario = Scenario::new("feliz");
    let store = scenario.store();
    let ready = org_ready(store, "aparelho-feliz");

    let received = store
        .inbox()
        .receive_from_device(ready.org, fresh(&ready, "MSG-AP-1", "já enviei, obrigado"))
        .expect("grava a mensagem do aparelho");

    assert_eq!(received.situation, Received::New);
    assert!(received.just_transferred, "a conversa nasceu aqui e pausou");
    assert_eq!(received.mensagem.direction.as_text(), "out");
    assert_eq!(received.mensagem.origin, Origin::Device);
    assert!(received.mensagem.turn_id.is_none(), "sem turno");
    assert_eq!(received.mensagem.texto, "já enviei, obrigado");

    let row = single_line(
        store,
        "SELECT status, version FROM conversations WHERE id = ?1",
        &received.conversation.to_string(),
    );
    assert_eq!(row[0].as_deref(), Some("transferred"));
    assert_eq!(row[1].as_deref(), Some("2"));

    let mensagem = single_line(store, SQL_MESSAGE, "MSG-AP-1");
    assert_eq!(mensagem[0].as_deref(), Some("out"));
    assert_eq!(mensagem[1].as_deref(), Some("assistant"));
    assert_eq!(mensagem[2].as_deref(), Some("device"));
    assert_eq!(mensagem[3], None, "sem turno");

    assert_eq!(count_rows(store, "SELECT COUNT(*) FROM inbox"), 0, "inbox");
    assert_eq!(
        count_rows(store, "SELECT COUNT(*) FROM outbox"),
        0,
        "outbox"
    );

    let audit = single_line(
        store,
        "SELECT actor_kind, action, entity FROM audit_log WHERE entity_id = ?1",
        &received.conversation.to_string(),
    );
    assert_eq!(audit[0].as_deref(), Some("system"));
    assert_eq!(audit[1].as_deref(), Some("device_takeover"));
    assert_eq!(audit[2].as_deref(), Some("conversation"));

    assert_eq!(
        count_rows(store, "SELECT COUNT(*) FROM fichas"),
        1,
        "ficha da conversa nova"
    );
}

#[test]
fn redelivery_of_same_external_id_does_not_duplicate() {
    let scenario = Scenario::new("reentrega");
    let store = scenario.store();
    let ready = org_ready(store, "aparelho-reentrega");

    let first = store
        .inbox()
        .receive_from_device(ready.org, fresh(&ready, "MSG-AP-2", "oi"))
        .expect("primeira entrega");
    let second = store
        .inbox()
        .receive_from_device(ready.org, fresh(&ready, "MSG-AP-2", "oi"))
        .expect("reentrega");

    assert_eq!(second.situation, Received::Duplicate);
    assert_eq!(second.mensagem.id, first.mensagem.id);
    assert_eq!(second.conversation, first.conversation);
    assert!(!second.just_transferred, "a segunda não transfere de novo");
    assert_eq!(count_rows(store, "SELECT COUNT(*) FROM messages"), 1);
    assert_eq!(
        count_rows(
            store,
            "SELECT COUNT(*) FROM audit_log WHERE action = 'device_takeover'"
        ),
        1
    );
    assert_eq!(count_rows(store, "SELECT COUNT(*) FROM conversations"), 1);

    let version = single_line(
        store,
        "SELECT version FROM conversations WHERE id = ?1",
        &first.conversation.to_string(),
    );
    assert_eq!(version[0].as_deref(), Some("2"));
}

#[test]
fn already_transferred_conversation_does_not_raise_version() {
    let scenario = Scenario::new("ja-transferida");
    let store = scenario.store();
    let ready = org_ready(store, "aparelho-ja-transferida");

    let first = store
        .inbox()
        .receive_from_device(ready.org, fresh(&ready, "MSG-AP-3", "primeira"))
        .expect("primeira");
    let second = store
        .inbox()
        .receive_from_device(ready.org, fresh(&ready, "MSG-AP-4", "segunda"))
        .expect("segunda");

    assert_eq!(second.situation, Received::New);
    assert!(!second.just_transferred);
    assert_eq!(second.conversation, first.conversation, "mesma conversa");
    let row = single_line(
        store,
        "SELECT status, version FROM conversations WHERE id = ?1",
        &first.conversation.to_string(),
    );
    assert_eq!(row[0].as_deref(), Some("transferred"));
    assert_eq!(row[1].as_deref(), Some("2"), "a versão não sobe de novo");
    assert_eq!(
        count_rows(
            store,
            "SELECT COUNT(*) FROM audit_log WHERE action = 'device_takeover'"
        ),
        2,
        "cada mensagem do aparelho deixa a sua auditoria"
    );
}

#[test]
fn closed_conversation_opens_another() {
    let scenario = Scenario::new("fechada");
    let store = scenario.store();
    let ready = org_ready(store, "aparelho-fechada");

    let first = store
        .inbox()
        .receive_from_device(ready.org, fresh(&ready, "MSG-AP-5", "antes de fechar"))
        .expect("primeira");
    let current = store
        .conversations()
        .get(ready.org, first.conversation)
        .expect("lê a conversa");
    store
        .conversations()
        .change_status(
            ready.org,
            first.conversation,
            current.version,
            ConversationStatus::Closed,
            None,
        )
        .expect("fecha a conversa");

    let later = store
        .inbox()
        .receive_from_device(ready.org, fresh(&ready, "MSG-AP-6", "depois de fechar"))
        .expect("mensagem depois do fechamento");

    assert_ne!(later.conversation, first.conversation, "conversa nova");
    assert!(later.just_transferred);
    let row = single_line(
        store,
        "SELECT status FROM conversations WHERE id = ?1",
        &later.conversation.to_string(),
    );
    assert_eq!(row[0].as_deref(), Some("transferred"));
}

#[test]
fn failure_in_audit_leaves_nothing_stored() {
    let scenario = Scenario::new("falha");
    let store = scenario.store();
    let ready = org_ready(store, "aparelho-falha");

    connection(store)
        .execute_batch(
            "CREATE TRIGGER recusa_device_takeover BEFORE INSERT ON audit_log
             WHEN NEW.action = 'device_takeover'
             BEGIN SELECT RAISE(ABORT, 'falha simulada'); END;",
        )
        .expect("cria o gatilho");

    let error = store
        .inbox()
        .receive_from_device(ready.org, fresh(&ready, "MSG-AP-7", "vai falhar"))
        .expect_err("a gravação tem de falhar");
    assert!(!format!("{error}").is_empty());

    assert_eq!(
        count_rows(store, "SELECT COUNT(*) FROM messages"),
        0,
        "mensagem"
    );
    assert_eq!(
        count_rows(store, "SELECT COUNT(*) FROM conversations"),
        0,
        "conversa"
    );
    assert_eq!(
        count_rows(store, "SELECT COUNT(*) FROM audit_log"),
        0,
        "auditoria"
    );
}

#[test]
fn sent_with_external_id_recognizes_only_id_of_confirmed_send() {
    let scenario = Scenario::new("eco");
    let store = scenario.store();
    let ready = org_ready(store, "aparelho-eco");
    let other = org_ready(store, "aparelho-eco-outra");

    let received = store
        .inbox()
        .receive_from_device(ready.org, fresh(&ready, "MSG-AP-8", "abre a conversa"))
        .expect("mensagem do aparelho");

    let turno = TurnId::generate();
    let saida = store
        .messages()
        .write_with_output(
            ready.org,
            poph_store::NewMessage {
                conversation_id: received.conversation,
                direction: poph_store::Direction::Outbound,
                role: poph_store::Role::Assistant,
                origin: Origin::Agente,
                author_name: None,
                texto: "resposta do agente".to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: Some(turno),
            },
            poph_store::NewOutboxItem {
                conversation_id: received.conversation,
                turn_id: turno,
                seq: 0,
                tipo: "text".to_owned(),
                content_json: "{\"texto\":\"oi\"}".to_owned(),
            },
        )
        .expect("grava a saída")
        .1;

    assert!(
        !store
            .outbox()
            .sent_with_external_id(ready.org, "EVO-XYZ")
            .expect("consulta o eco"),
        "antes da confirmação o id não existe"
    );

    store
        .outbox()
        .confirm_send_with_id(ready.org, saida.id, Some("EVO-XYZ"))
        .expect("confirma o envio com id");

    assert!(
        store
            .outbox()
            .sent_with_external_id(ready.org, "EVO-XYZ")
            .expect("consulta o eco"),
        "o id devolvido pela Evolution é o eco"
    );
    assert!(
        !store
            .outbox()
            .sent_with_external_id(other.org, "EVO-XYZ")
            .expect("consulta o eco da outra org"),
        "o eco não atravessa org"
    );
    assert!(
        !store
            .outbox()
            .sent_with_external_id(ready.org, "EVO-OUTRO")
            .expect("consulta outro id"),
        "id desconhecido não é eco"
    );
}
