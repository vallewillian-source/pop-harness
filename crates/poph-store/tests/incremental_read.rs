use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::migrations::{HARNESS_MIGRATIONS, migrate};
use poph_store::{
    Direction, Mensagem, NewAgent, NewConversation, NewMessage, NewOrg, Origin, Role, Store,
};
use poph_types::{ConversationId, OrgId};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Environment {
    folder: PathBuf,
    store: Store,
    org: OrgId,
    agent: poph_types::AgentId,
}

impl Environment {
    fn new(label: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let folder =
            std::env::temp_dir().join(format!("poph-mx05-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("cria pasta");
        let store =
            Store::open_at(&folder.join("harness.db"), &folder.join("rag.db")).expect("abre banco");
        let org = store
            .orgs()
            .create(NewOrg {
                name: format!("Org {label}"),
                slug: format!("mx05-{label}-{n}"),
                config_json: String::new(),
            })
            .expect("org")
            .id;
        let agent = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Agente".to_owned(),
                    slug: format!("agente-{n}"),
                    ..NewAgent::default()
                },
            )
            .expect("agente")
            .id;
        Self {
            folder,
            store,
            org,
            agent,
        }
    }

    fn conversation(&self, peer: &str) -> ConversationId {
        self.store
            .conversations()
            .create(
                self.org,
                NewConversation {
                    agent_id: self.agent,
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some(peer.to_owned()),
                    target_department: None,
                },
            )
            .expect("conversa")
            .id
    }

    fn write(&self, conversation: ConversationId, role: Role, texto: &str) -> Mensagem {
        let direction = match role {
            Role::User => Direction::Inbound,
            _ => Direction::Outbound,
        };
        self.store
            .messages()
            .write(
                self.org,
                NewMessage {
                    conversation_id: conversation,
                    direction,
                    role,
                    origin: Origin::Agente,
                    author_name: None,
                    texto: texto.to_owned(),
                    attachments_json: "[]".to_owned(),
                    external_id: None,
                    turn_id: None,
                },
            )
            .expect("grava mensagem")
    }

    fn read_everything(&self, after: u64, limit: u32) -> Vec<poph_store::MessageSince> {
        let mut cursor = after;
        let mut every = Vec::new();
        loop {
            let page = self.store.messages().since(cursor, limit).expect("desde");
            if page.is_empty() {
                break;
            }
            cursor = page.last().expect("ultima").rowid;
            every.extend(page);
        }
        every
    }
}

impl Drop for Environment {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.folder);
    }
}

#[test]
fn since_returns_in_ascending_rowid_order() {
    let amb = Environment::new("ordem");
    let conversation = amb.conversation("5511900000001");
    let texts = ["um", "dois", "tres", "quatro", "cinco"];
    for (i, texto) in texts.iter().enumerate() {
        let role = if i % 2 == 0 {
            Role::User
        } else {
            Role::Assistant
        };
        amb.write(conversation, role, texto);
    }

    let page = amb.store.messages().since(0, 200).expect("desde");
    let read_many: Vec<&str> = page.iter().map(|m| m.texto.as_str()).collect();
    assert_eq!(read_many, texts);
    assert!(
        page.windows(2).all(|pair| pair[0].rowid < pair[1].rowid),
        "rowid precisa crescer estritamente"
    );
    assert!(page.iter().all(|m| m.conversation_id == conversation));
    assert!(page.iter().all(|m| m.org_id == amb.org));
}

#[test]
fn paginating_to_the_end_loses_and_repeats_no_row() {
    let amb = Environment::new("paginas");
    let conversation = amb.conversation("5511900000002");
    let stored_many: Vec<_> = (0..25)
        .map(|i| {
            let role = if i % 3 == 0 {
                Role::Assistant
            } else {
                Role::User
            };
            amb.write(conversation, role, &format!("msg {i}"))
        })
        .collect();

    let read_rows = amb.read_everything(0, 7);

    assert_eq!(
        read_rows.len(),
        stored_many.len(),
        "nenhuma linha pode faltar"
    );
    let read_ids: Vec<_> = read_rows.iter().map(|m| m.id).collect();
    let stored_ids: Vec<_> = stored_many.iter().map(|m| m.id).collect();
    assert_eq!(read_ids, stored_ids, "mesma sequência, sem repetição");
    let fim = read_rows.last().expect("ultima").rowid;
    assert!(
        amb.store
            .messages()
            .since(fim, 7)
            .expect("desde")
            .is_empty()
    );
}

#[test]
fn messages_stored_between_pages_appear_later_without_losing_others() {
    let amb = Environment::new("entre-paginas");
    let conversation = amb.conversation("5511900000003");
    let beginning: Vec<_> = (0..10)
        .map(|i| amb.write(conversation, Role::User, &format!("antes {i}")))
        .collect();

    let first = amb.store.messages().since(0, 4).expect("primeira");
    assert_eq!(first.len(), 4);
    let cursor = first.last().expect("ultima").rowid;

    let later: Vec<_> = (0..3)
        .map(|i| amb.write(conversation, Role::Assistant, &format!("depois {i}")))
        .collect();

    let rest = amb.read_everything(cursor, 4);
    let rest_ids: Vec<_> = rest.iter().map(|m| m.id).collect();
    let expected_value: Vec<_> = beginning[4..]
        .iter()
        .chain(later.iter())
        .map(|m| m.id)
        .collect();
    assert_eq!(rest_ids, expected_value);

    let first_ids: Vec<_> = first.iter().map(|m| m.id).collect();
    let beginning_ids: Vec<_> = beginning[..4].iter().map(|m| m.id).collect();
    assert_eq!(first_ids, beginning_ids);
}

#[test]
fn tool_and_system_never_appear_in_read() {
    let amb = Environment::new("papeis");
    let conversation = amb.conversation("5511900000004");
    amb.write(conversation, Role::User, "oi");
    amb.write(conversation, Role::System, "instrucao interna");
    amb.write(conversation, Role::Tool, "resultado da ferramenta");
    amb.write(conversation, Role::Assistant, "tudo bem");
    amb.write(conversation, Role::Tool, "outra ferramenta");

    let read_rows = amb.read_everything(0, 1);
    let texts: Vec<&str> = read_rows.iter().map(|m| m.texto.as_str()).collect();
    assert_eq!(texts, ["oi", "tudo bem"]);
    assert!(
        read_rows
            .iter()
            .all(|m| matches!(m.role, Role::User | Role::Assistant)),
        "só user e assistant"
    );
}

#[test]
fn origin_author_and_external_id_arrive_intact() {
    let amb = Environment::new("origem");
    let conversation = amb.conversation("5511900000005");
    let human = amb
        .store
        .messages()
        .write(
            amb.org,
            NewMessage {
                conversation_id: conversation,
                direction: Direction::Outbound,
                role: Role::Assistant,
                origin: Origin::Human,
                author_name: Some("Ana".to_owned()),
                texto: "posso ajudar".to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: Some("$evento-1".to_owned()),
                turn_id: None,
            },
        )
        .expect("grava humana");

    let read_rows = amb.store.messages().since(0, 10).expect("desde");
    assert_eq!(read_rows.len(), 1);
    let m = &read_rows[0];
    assert_eq!(m.id, human.id);
    assert_eq!(m.origin, Origin::Human);
    assert_eq!(m.author_name.as_deref(), Some("Ana"));
    assert_eq!(m.external_id.as_deref(), Some("$evento-1"));
    assert_eq!(m.direction, Direction::Outbound);
    assert_eq!(m.role, Role::Assistant);
}

#[test]
fn read_brings_messages_of_more_than_one_org_with_org_of_each_row() {
    let amb = Environment::new("duas-orgs-a");
    let other = amb
        .store
        .orgs()
        .create(NewOrg {
            name: "Outra".to_owned(),
            slug: format!("mx05-outra-{}", std::process::id()),
            config_json: String::new(),
        })
        .expect("outra org")
        .id;
    let agent_other = amb
        .store
        .agents()
        .create(
            other,
            NewAgent {
                name: "Outro".to_owned(),
                slug: "outro".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("agente outra")
        .id;
    let conversation_a = amb.conversation("a");
    let conversation_b = amb
        .store
        .conversations()
        .create(
            other,
            NewConversation {
                agent_id: agent_other,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: Some("b".to_owned()),
                target_department: None,
            },
        )
        .expect("conversa outra")
        .id;
    amb.write(conversation_a, Role::User, "da A");
    amb.store
        .messages()
        .write(
            other,
            NewMessage {
                conversation_id: conversation_b,
                direction: Direction::Inbound,
                role: Role::User,
                origin: Origin::Agente,
                author_name: None,
                texto: "da B".to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
            },
        )
        .expect("grava B");

    let read_rows = amb.store.messages().since(0, 10).expect("desde");
    assert_eq!(read_rows.len(), 2);
    assert_eq!(read_rows[0].org_id, amb.org);
    assert_eq!(read_rows[1].org_id, other);
    assert_eq!(read_rows[1].conversation_id, conversation_b);
}

#[test]
fn changed_since_pages_by_instant_and_id_without_losing_ties() {
    let amb = Environment::new("conversas");
    let ids: Vec<ConversationId> = (0..5)
        .map(|i| amb.conversation(&format!("peer-{i}")))
        .collect();
    amb.store
        .conversations()
        .touch(amb.org, ids[0], 1000)
        .expect("tocar");
    amb.store
        .conversations()
        .touch(amb.org, ids[1], 1000)
        .expect("tocar");
    amb.store
        .conversations()
        .touch(amb.org, ids[2], 1000)
        .expect("tocar");
    amb.store
        .conversations()
        .touch(amb.org, ids[3], 500)
        .expect("tocar");
    amb.store
        .conversations()
        .touch(amb.org, ids[4], 2000)
        .expect("tocar");

    let mut cursor: Option<(i64, ConversationId)> = None;
    let mut seen: Vec<ConversationId> = Vec::new();
    loop {
        let page = amb
            .store
            .conversations()
            .changed_since(cursor, 1)
            .expect("alteradas");
        let Some(last) = page.last() else { break };
        cursor = Some((last.updated_at, last.id));
        seen.extend(page.iter().map(|c| c.id));
    }

    assert_eq!(seen.len(), 5, "nenhuma conversa pode faltar nem repetir");
    let mut empatadas = vec![ids[0], ids[1], ids[2]];
    empatadas.sort_by_key(|id| id.to_string());
    let mut expected_ids: Vec<ConversationId> = vec![ids[3]];
    expected_ids.extend(empatadas);
    expected_ids.push(ids[4]);
    assert_eq!(seen, expected_ids);
}

#[test]
fn changed_since_none_starts_from_beginning_and_after_cursor_does_not_repeat() {
    let amb = Environment::new("conversas-inicio");
    let a = amb.conversation("x");
    let b = amb.conversation("y");
    amb.store
        .conversations()
        .touch(amb.org, a, 10)
        .expect("tocar");
    amb.store
        .conversations()
        .touch(amb.org, b, 20)
        .expect("tocar");

    let every = amb
        .store
        .conversations()
        .changed_since(None, 1000)
        .expect("todas");
    let ids: Vec<_> = every.iter().map(|c| c.id).collect();
    assert!(ids.contains(&a) && ids.contains(&b));

    let after_a = amb
        .store
        .conversations()
        .changed_since(Some((10, a)), 1000)
        .expect("depois");
    assert!(
        after_a.iter().all(|c| c.id != a),
        "o cursor não repete a própria conversa"
    );
    assert!(after_a.iter().any(|c| c.id == b));
}

#[test]
fn migration_0006_over_database_with_data_keeps_rows_and_read_works() {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let folder =
        std::env::temp_dir().join(format!("poph-mx05-migracao-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).expect("cria a pasta");
    let path = folder.join("harness.db");
    let conn = rusqlite::Connection::open(&path).expect("abre o banco");

    migrate(&conn, "harness.db", &path, &HARNESS_MIGRATIONS[..5], 1).expect("migra até a 5");
    conn.execute_batch("PRAGMA foreign_keys = OFF;")
        .expect("desliga");
    conn.execute_batch(
        "INSERT INTO orgs (id, name, slug, created_at, updated_at)
            VALUES ('0190a000-0000-7000-8000-00000000a001', 'Org da migração', 'org-migracao-0006', 1, 1);
         INSERT INTO agents (id, org_id, name, slug, provider, model, created_at, updated_at)
            VALUES ('0190a000-0000-7000-8000-00000000a002',
                    '0190a000-0000-7000-8000-00000000a001', 'Agente da migração',
                    'agente-migracao-0006', 'antigravity2api', 'gemini', 1, 1);
         INSERT INTO conversations (id, org_id, agent_id, channel, status, version, created_at, updated_at)
            VALUES ('0190a000-0000-7000-8000-00000000c001', '0190a000-0000-7000-8000-00000000a001', '0190a000-0000-7000-8000-00000000a002', 'whatsapp', 'open', 1, 10, 30),
                   ('0190a000-0000-7000-8000-00000000c002', '0190a000-0000-7000-8000-00000000a001', '0190a000-0000-7000-8000-00000000a002', 'whatsapp', 'open', 1, 10, 20);
         INSERT INTO messages (id, org_id, conversation_id, direction, role, text, created_at)
            VALUES ('0190a000-0000-7000-8000-00000000d001', '0190a000-0000-7000-8000-00000000a001', '0190a000-0000-7000-8000-00000000c001', 'in', 'user', 'oi', 10),
                   ('0190a000-0000-7000-8000-00000000d002', '0190a000-0000-7000-8000-00000000a001', '0190a000-0000-7000-8000-00000000c002', 'out', 'assistant', 'tudo bem', 20);",
    )
    .expect("grava dados no formato anterior");

    let report = migrate(&conn, "harness.db", &path, &HARNESS_MIGRATIONS[..6], 2).expect("até a 6");
    assert_eq!(report.version_before, 5);
    assert_eq!(report.version_after, 6);

    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |l| l.get(0)).expect("conta") };
    assert_eq!(
        count("SELECT count(*) FROM messages"),
        2,
        "nenhuma mensagem perdida"
    );
    assert_eq!(
        count("SELECT count(*) FROM conversations"),
        2,
        "nenhuma conversa perdida"
    );
    assert_eq!(
        count(
            "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = 'ix_conversations_atualizada'"
        ),
        1,
        "o índice novo existe"
    );
    let integridade: String = conn
        .query_row("PRAGMA integrity_check", [], |l| l.get(0))
        .expect("integridade");
    assert_eq!(integridade, "ok");
    drop(conn);

    let store = Store::open_at(&path, &folder.join("rag.db")).expect("abre o store");
    let messages = store.messages().since(0, 10).expect("desde");
    let texts: Vec<&str> = messages.iter().map(|m| m.texto.as_str()).collect();
    assert_eq!(texts, ["oi", "tudo bem"]);
    let conversations = store
        .conversations()
        .changed_since(None, 10)
        .expect("alteradas");
    assert_eq!(conversations.len(), 2);
    assert_eq!(conversations[0].updated_at, 20, "a mais antiga primeiro");
    assert_eq!(conversations[1].updated_at, 30);

    drop(store);
    let _ = fs::remove_dir_all(&folder);
}

#[test]
fn conversation_read_plan_uses_index_and_does_not_scan_table() {
    let amb = Environment::new("plano");
    let path = amb.store.harness_path().to_path_buf();
    let con = rusqlite::Connection::open(&path).expect("abre para ler o plano");
    let details: Vec<String> = {
        let mut statement = con
            .prepare(
                "EXPLAIN QUERY PLAN SELECT id FROM conversations
                  WHERE (updated_at, id) > (?1, ?2) ORDER BY updated_at, id LIMIT ?3",
            )
            .expect("prepara");
        statement
            .query_map(rusqlite::params![0_i64, "", 10_i64], |l| {
                l.get::<_, String>(3)
            })
            .expect("plano")
            .map(|l| l.expect("linha"))
            .collect()
    };
    let texto = details.join(" | ");
    assert!(
        texto.contains("ix_conversations_atualizada"),
        "plano: {texto}"
    );
    assert!(!texto.contains("SCAN conversations"), "plano: {texto}");
    assert!(!texto.contains("TEMP B-TREE"), "plano: {texto}");
}
