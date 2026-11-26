use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::backup::ImportTarget;
use poph_store::migrations::{HARNESS_MIGRATIONS, migrate};
use poph_store::{
    Conversa, Direction, NewAgent, NewConversation, NewMessage, NewOrg, NewOutboxItem, Org, Origin,
    Page, Role, Store,
};
use poph_types::{ConversationId, MessageId, TurnId};
use serde_json::Value;

struct Scenario {
    folder: PathBuf,
    store: Option<Store>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder =
            std::env::temp_dir().join(format!("poph-origem-{label}-{}-{n}", std::process::id()));
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

fn org_and_conversation(store: &Store, slug: &str) -> (Org, Conversa) {
    let org = store
        .orgs()
        .create(NewOrg {
            name: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org");
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
                canal: "cli".to_owned(),
                peer_externo: Some("cliente-1".to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa");
    (org, conversation)
}

fn output_from_agent(conversation: ConversationId, texto: &str) -> NewMessage {
    NewMessage {
        conversation_id: conversation,
        direction: Direction::Outbound,
        role: Role::Assistant,
        origin: Origin::Agente,
        author_name: None,
        texto: texto.to_owned(),
        attachments_json: "[]".to_owned(),
        external_id: None,
        turn_id: None,
    }
}

fn raw_row(store: &Store, id: MessageId) -> (String, Option<String>) {
    let conn = rusqlite::Connection::open(store.harness_path()).expect("abre o banco");
    conn.query_row(
        "SELECT origin, author_name FROM messages WHERE id = ?1",
        [id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .expect("lê a linha")
}

#[test]
fn turn_message_remains_agent_without_author() {
    let scenario = Scenario::new("turno");
    let store = scenario.store();
    let (org, conversation) = org_and_conversation(store, "turno");

    let stored = store
        .messages()
        .write(org.id, output_from_agent(conversation.id, "olá"))
        .expect("grava");
    assert_eq!(stored.origin, Origin::Agente);
    assert_eq!(stored.author_name, None);
    assert_eq!(
        raw_row(store, stored.id),
        ("agent".to_owned(), None),
        "o banco guarda `agent` e autor nulo"
    );

    let turno = TurnId::generate();
    let (with_output, _item) = store
        .messages()
        .write_with_output(
            org.id,
            NewMessage {
                turn_id: Some(turno),
                ..output_from_agent(conversation.id, "resposta do turno")
            },
            NewOutboxItem {
                conversation_id: conversation.id,
                turn_id: turno,
                seq: 0,
                tipo: "text".to_owned(),
                content_json: "{}".to_owned(),
            },
        )
        .expect("grava com saída");
    assert_eq!(with_output.origin, Origin::Agente);
    assert_eq!(with_output.author_name, None);
    assert_eq!(raw_row(store, with_output.id), ("agent".to_owned(), None));

    let read_rows = store
        .messages()
        .list(org.id, conversation.id, Page::default())
        .expect("lista");
    assert_eq!(read_rows.len(), 2);
    assert!(
        read_rows
            .iter()
            .all(|m| m.origin == Origin::Agente && m.author_name.is_none())
    );
}

#[test]
fn human_response_goes_and_returns_in_backup_with_author() {
    let origin = Scenario::new("backup-origem");
    let (org, conversation) = org_and_conversation(origin.store(), "humano");
    let human = origin
        .store()
        .messages()
        .write(
            org.id,
            NewMessage {
                origin: Origin::Human,
                author_name: Some("Ana Atendente".to_owned()),
                ..output_from_agent(conversation.id, "*[Ana Atendente]*: posso ajudar")
            },
        )
        .expect("grava a resposta humana");
    assert_eq!(
        raw_row(origin.store(), human.id),
        ("human".to_owned(), Some("Ana Atendente".to_owned()))
    );

    let snapshot = origin
        .store()
        .org_snapshot(org.id, true)
        .expect("retrato da org");
    let row = snapshot.tables["messages"]
        .iter()
        .find(|row| row["id"] == Value::String(human.id.to_string()))
        .expect("a mensagem humana está no arquivo");
    assert_eq!(row["origin"], Value::String("human".to_owned()));
    assert_eq!(
        row["author_name"],
        Value::String("Ana Atendente".to_owned())
    );

    let destination = Scenario::new("backup-destino");
    destination
        .store()
        .import_snapshot(snapshot, ImportTarget::Origin)
        .expect("importa o arquivo");

    let read_rows = destination
        .store()
        .messages()
        .list(org.id, conversation.id, Page::default())
        .expect("lista no destino");
    assert_eq!(read_rows.len(), 1);
    assert_eq!(read_rows[0].origin, Origin::Human);
    assert_eq!(read_rows[0].author_name.as_deref(), Some("Ana Atendente"));
}

#[test]
fn old_backup_without_columns_imports_as_agent() {
    let origin = Scenario::new("backup-antigo-origem");
    let (org, conversation) = org_and_conversation(origin.store(), "antigo");
    origin
        .store()
        .messages()
        .write(
            org.id,
            output_from_agent(conversation.id, "mensagem de antes da MX-03"),
        )
        .expect("grava");

    let mut snapshot = origin
        .store()
        .org_snapshot(org.id, true)
        .expect("retrato da org");
    for row in snapshot.tables.get_mut("messages").expect("tem mensagens") {
        let object = row.as_object_mut().expect("linha é objeto");
        object.remove("origin");
        object.remove("author_name");
    }

    let destination = Scenario::new("backup-antigo-destino");
    destination
        .store()
        .import_snapshot(snapshot, ImportTarget::Origin)
        .expect("importa o arquivo antigo");

    let read_rows = destination
        .store()
        .messages()
        .list(org.id, conversation.id, Page::default())
        .expect("lista no destino");
    assert_eq!(read_rows.len(), 1);
    assert_eq!(read_rows[0].origin, Origin::Agente);
    assert_eq!(read_rows[0].author_name, None);
}

#[test]
fn origin_migration_keeps_messages_that_already_existed() {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let folder =
        std::env::temp_dir().join(format!("poph-origem-migracao-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).expect("cria a pasta");
    let path = folder.join("harness.db");
    let conn = rusqlite::Connection::open(&path).expect("abre o banco");

    migrate(&conn, "harness.db", &path, &HARNESS_MIGRATIONS[..2], 1).expect("migra até a 2");
    conn.execute_batch("PRAGMA foreign_keys = OFF;")
        .expect("desliga a checagem");
    conn.execute_batch(
        "INSERT INTO messages (id, org_id, conversation_id, direction, role, text, created_at)
            VALUES ('m-1', 'org-1', 'conv-1', 'out', 'assistant', 'primeira', 10),
                   ('m-2', 'org-1', 'conv-1', 'in', 'user', 'segunda', 20);",
    )
    .expect("grava mensagens no formato antigo");

    let report = migrate(&conn, "harness.db", &path, HARNESS_MIGRATIONS, 2).expect("migra até a 3");
    assert_eq!(report.version_before, 2);
    assert_eq!(report.version_after, poph_store::harness_target_version());

    let total: i64 = conn
        .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
        .expect("conta");
    assert_eq!(total, 2, "nenhuma linha perdida");

    let rows: Vec<(String, String, Option<String>)> = {
        let mut statement = conn
            .prepare("SELECT id, origin, author_name FROM messages ORDER BY id")
            .expect("prepara");
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("consulta")
            .map(|r| r.expect("linha"))
            .collect()
    };
    assert_eq!(
        rows,
        vec![
            ("m-1".to_owned(), "agent".to_owned(), None),
            ("m-2".to_owned(), "agent".to_owned(), None),
        ],
        "as mensagens antigas ganham `agent` e autor nulo"
    );

    let refused = conn.execute(
        "INSERT INTO messages (id, org_id, conversation_id, direction, role, text, created_at, origin)
            VALUES ('m-3', 'org-1', 'conv-1', 'out', 'assistant', 'x', 30, 'robo')",
        [],
    );
    assert!(
        refused.is_err(),
        "origem fora do conjunto deve ser recusada"
    );

    drop(conn);
    let _ = fs::remove_dir_all(&folder);
}
