use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use poph_store::migrations::migrate;
use poph_store::{
    Agente, Conversa, HARNESS_MIGRATIONS, LIMIT_NAME_CLIENT, NewAgent, NewChannelInboxItem,
    NewConversation, NewOrg, Org, Received, Store, file_version, harness_target_version,
    normalise_name_client,
};
use poph_types::{ConversationId, IntegrationId, OrgId};

struct Database {
    folder: PathBuf,
    clock: Arc<AtomicI64>,
    store: Store,
}

impl Database {
    fn open(label: &str) -> Self {
        let folder = folder_new(label);
        let clock = Arc::new(AtomicI64::new(1_000));
        let now = Arc::clone(&clock);
        let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
            .expect("abre o banco")
            .with_clock(Arc::new(move || now.load(Ordering::SeqCst)));
        Self {
            folder,
            clock,
            store,
        }
    }

    fn define_clock(&self, value: i64) {
        self.clock.store(value, Ordering::SeqCst);
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.folder);
    }
}

fn folder_new(label: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let folder = std::env::temp_dir().join(format!(
        "poph-nome-cliente-{label}-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).expect("cria a pasta do teste");
    folder
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

fn create_agent(store: &Store, org: OrgId) -> Agente {
    store
        .agents()
        .create(
            org,
            NewAgent {
                name: "Atendente".to_owned(),
                slug: "atendente".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente")
}

fn create_conversation(store: &Store, org: OrgId, agent: &Agente) -> Conversa {
    store
        .conversations()
        .create(
            org,
            NewConversation {
                agent_id: agent.id,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: Some("5511999990000".to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa")
}

#[test]
fn normalization_cleans_spaces_controls_and_direction_marks() {
    assert_eq!(
        normalise_name_client("  Ana   Maria  ").as_deref(),
        Some("Ana Maria")
    );
    assert_eq!(
        normalise_name_client("Ana\u{200B}Maria").as_deref(),
        Some("AnaMaria")
    );
    assert_eq!(
        normalise_name_client("\u{202E}Ana\u{202C}").as_deref(),
        Some("Ana")
    );
    assert_eq!(
        normalise_name_client("\u{2066}Ana\u{2069}").as_deref(),
        Some("Ana")
    );
    assert_eq!(
        normalise_name_client("Ana\tSilva\nReis").as_deref(),
        Some("Ana Silva Reis")
    );
    assert_eq!(
        normalise_name_client("Ma\u{0007}ria").as_deref(),
        Some("Maria")
    );
}

#[test]
fn normalization_refuses_empty_name_and_cuts_at_hundred_characters() {
    assert_eq!(normalise_name_client(""), None);
    assert_eq!(normalise_name_client("   \n\t "), None);
    assert_eq!(normalise_name_client("\u{200B}\u{2066}\u{0007}"), None);

    let long = "a".repeat(150);
    let cut = normalise_name_client(&long).expect("nome não vazio");
    assert_eq!(cut.chars().count(), LIMIT_NAME_CLIENT);

    let acentuado = "é".repeat(120);
    let cut = normalise_name_client(&acentuado).expect("nome não vazio");
    assert_eq!(cut.chars().count(), LIMIT_NAME_CLIENT);

    let with_space = format!("{} b", "a".repeat(99));
    let cut = normalise_name_client(&with_space).expect("nome não vazio");
    assert_eq!(cut.chars().count(), 99);
    assert!(!cut.ends_with(' '));
}

#[test]
fn set_creates_name_and_last_arrival_wins() {
    let database = Database::open("ultimo-vence");
    let store = &database.store;
    let org = create_org(store, "a");
    let agent = create_agent(store, org.id);
    let conversation = create_conversation(store, org.id, &agent);
    assert_eq!(conversation.customer_name, None);

    assert!(
        store
            .conversations()
            .set_customer_name(org.id, conversation.id, "Ana")
            .expect("grava")
    );
    let read_row = store
        .conversations()
        .get(org.id, conversation.id)
        .expect("lê");
    assert_eq!(read_row.customer_name.as_deref(), Some("Ana"));

    assert!(
        !store
            .conversations()
            .set_customer_name(org.id, conversation.id, "Ana")
            .expect("repete")
    );
    assert!(
        store
            .conversations()
            .set_customer_name(org.id, conversation.id, "Ana Maria")
            .expect("atualiza")
    );
    let read_row = store
        .conversations()
        .get(org.id, conversation.id)
        .expect("lê");
    assert_eq!(read_row.customer_name.as_deref(), Some("Ana Maria"));
}

#[test]
fn writing_name_raises_updated_at_and_not_version_nor_last_message() {
    let database = Database::open("sem-version");
    let store = &database.store;
    let org = create_org(store, "a");
    let agent = create_agent(store, org.id);
    let conversation = create_conversation(store, org.id, &agent);
    store
        .conversations()
        .touch(org.id, conversation.id, 2_000)
        .expect("marca a última mensagem");
    let before = store
        .conversations()
        .get(org.id, conversation.id)
        .expect("lê");
    assert_eq!(before.version, 1);
    assert_eq!(before.last_message_at, Some(2_000));

    database.define_clock(5_000);
    store
        .conversations()
        .set_customer_name(org.id, conversation.id, "Ana")
        .expect("grava");

    let later = store
        .conversations()
        .get(org.id, conversation.id)
        .expect("lê");
    assert_eq!(
        later.updated_at, 5_000,
        "o cursor do espelho enxerga a mudança"
    );
    assert_eq!(later.version, 1, "não gera conflito de If-Match no Console");
    assert_eq!(
        later.last_message_at,
        Some(2_000),
        "não mexe na última mensagem"
    );
}

#[test]
fn empty_name_writes_nothing_and_same_name_does_not_write_again() {
    let database = Database::open("sem-escrita");
    let store = &database.store;
    let org = create_org(store, "a");
    let agent = create_agent(store, org.id);
    let conversation = create_conversation(store, org.id, &agent);

    assert!(
        !store
            .conversations()
            .set_customer_name(org.id, conversation.id, "  \u{200B} ")
            .expect("nome vazio")
    );
    let after_empty = store
        .conversations()
        .get(org.id, conversation.id)
        .expect("lê");
    assert_eq!(after_empty.customer_name, None);
    assert_eq!(after_empty.updated_at, 1_000, "vazio não escreve nada");

    database.define_clock(3_000);
    assert!(
        store
            .conversations()
            .set_customer_name(org.id, conversation.id, "Ana")
            .expect("primeira vez")
    );
    database.define_clock(7_000);
    assert!(
        !store
            .conversations()
            .set_customer_name(org.id, conversation.id, "Ana")
            .expect("repetição")
    );
    let later = store
        .conversations()
        .get(org.id, conversation.id)
        .expect("lê");
    assert_eq!(
        later.updated_at, 3_000,
        "repetir o mesmo nome não toca na linha"
    );
}

#[test]
fn other_org_neither_reads_nor_changes_name() {
    let database = Database::open("isolamento");
    let store = &database.store;
    let org_a = create_org(store, "a");
    let org_b = create_org(store, "b");
    let agent_a = create_agent(store, org_a.id);
    let conversation_a = create_conversation(store, org_a.id, &agent_a);
    store
        .conversations()
        .set_customer_name(org_a.id, conversation_a.id, "Ana")
        .expect("grava na org A");

    let modify = store
        .conversations()
        .set_customer_name(org_b.id, conversation_a.id, "Intruso")
        .expect("tentativa da org B");
    assert!(!modify, "a org B não atinge a conversa da org A");
    assert!(
        store
            .conversations()
            .get(org_b.id, conversation_a.id)
            .is_err(),
        "a org B não lê a conversa da org A"
    );
    let from_a = store
        .conversations()
        .get(org_a.id, conversation_a.id)
        .expect("lê");
    assert_eq!(from_a.customer_name.as_deref(), Some("Ana"));
}

#[test]
fn migration_applies_to_database_with_conversations_and_leaves_null() {
    let folder = folder_new("migracao");
    let harness = folder.join("harness.db");
    let rag = folder.join("rag.db");
    let before = HARNESS_MIGRATIONS.len() - 1;
    let org_id = uuid::Uuid::now_v7();
    let agent_id = uuid::Uuid::now_v7();
    let conversation_id = uuid::Uuid::now_v7();
    {
        let conn = rusqlite::Connection::open(&harness).expect("abre o banco antigo");
        migrate(
            &conn,
            "harness.db",
            &harness,
            &HARNESS_MIGRATIONS[..before],
            1,
        )
        .expect("migra até a versão anterior");
        conn.execute_batch(&format!(
            "INSERT INTO orgs (id, name, slug, created_at, updated_at)
                VALUES ('{org_id}', 'Org antiga', 'org-antiga', 1, 1);
             INSERT INTO agents (id, org_id, name, slug, provider, model, created_at, updated_at)
                VALUES ('{agent_id}', '{org_id}', 'Agente', 'agente', 'antigravity2api',
                        'gemini', 1, 1);
             INSERT INTO conversations (id, org_id, agent_id, channel, created_at, updated_at)
                VALUES ('{conversation_id}', '{org_id}', '{agent_id}', 'whatsapp', 1, 1);"
        ))
        .expect("grava dados na versão anterior");
    }

    let store = Store::open_at(&harness, &rag).expect("abre e migra");
    assert_eq!(
        file_version(&harness).expect("lê versão"),
        Some(harness_target_version())
    );
    let org: OrgId = org_id.to_string().parse().expect("org válida");
    let conversation: ConversationId = conversation_id
        .to_string()
        .parse()
        .expect("conversa válida");
    let previous = store
        .conversations()
        .get(org, conversation)
        .expect("a conversa antiga continua no banco");
    assert_eq!(
        previous.customer_name, None,
        "conversa antiga fica sem nome"
    );

    assert!(
        store
            .conversations()
            .set_customer_name(org, conversation, "Ana")
            .expect("grava sobre a conversa antiga")
    );
    let later = store.conversations().get(org, conversation).expect("lê");
    assert_eq!(later.customer_name.as_deref(), Some("Ana"));
    let _ = fs::remove_dir_all(&folder);
}

fn create_integration(store: &Store, org: OrgId, agent: poph_types::AgentId) -> IntegrationId {
    store
        .create_integration(org, agent, "whatsapp".to_owned(), "active".to_owned())
        .expect("cria a integração")
        .id
        .parse()
        .expect("id da integração válido")
}

fn entry_from_channel(
    integration: IntegrationId,
    agent: &Agente,
    external_id: &str,
    nome: Option<&str>,
) -> NewChannelInboxItem {
    NewChannelInboxItem {
        integration_id: integration,
        agent_id: agent.id,
        canal: "whatsapp".to_owned(),
        external_id: external_id.to_owned(),
        peer: "5511999990000".to_owned(),
        event_json: "{}".to_owned(),
        texto: "oi".to_owned(),
        attachments: Vec::new(),
        customer_name: nome.map(str::to_owned),
    }
}

#[test]
fn channel_delivery_writes_name_together_with_message() {
    let database = Database::open("entrega-nome");
    let org = create_org(&database.store, "entrega-nome");
    let agent = create_agent(&database.store, org.id);
    let integration = create_integration(&database.store, org.id, agent.id);

    let received = database
        .store
        .inbox()
        .receive_from_channel(
            org.id,
            entry_from_channel(integration, &agent, "M-NOME-1", Some("  Ana   Maria ")),
        )
        .expect("entrega");
    assert!(matches!(received.situation, Received::New));

    let conversation = database
        .store
        .conversations()
        .get(org.id, received.mensagem.conversation_id)
        .expect("conversa");
    assert_eq!(conversation.customer_name.as_deref(), Some("Ana Maria"));
    assert_eq!(conversation.version, 1, "o nome não sobe a versão");
}

#[test]
fn failure_writing_name_in_delivery_does_not_drop_message() {
    let database = Database::open("entrega-falha");
    let org = create_org(&database.store, "entrega-falha");
    let agent = create_agent(&database.store, org.id);
    let integration = create_integration(&database.store, org.id, agent.id);
    {
        let conn = rusqlite::Connection::open(database.store.harness_path()).expect("abre");
        conn.execute_batch(
            "CREATE TRIGGER falha_nome BEFORE UPDATE OF client_name ON conversations
             BEGIN SELECT RAISE(ABORT, 'falha simulada'); END;",
        )
        .expect("cria o gatilho");
    }

    let received = database
        .store
        .inbox()
        .receive_from_channel(
            org.id,
            entry_from_channel(integration, &agent, "M-FALHA-1", Some("Ana")),
        )
        .expect("a entrega não pode falhar por causa do nome");
    assert!(matches!(received.situation, Received::New));
    assert_eq!(
        database.store.inbox().count_pending(org.id).expect("conta"),
        1
    );

    let conversation = database
        .store
        .conversations()
        .get(org.id, received.mensagem.conversation_id)
        .expect("a conversa existe mesmo sem o nome");
    assert_eq!(
        conversation.customer_name, None,
        "o nome ficou para a próxima mensagem"
    );
}

#[test]
fn redelivery_of_same_event_does_not_rewrite_name() {
    let database = Database::open("entrega-reentrega");
    let org = create_org(&database.store, "entrega-reentrega");
    let agent = create_agent(&database.store, org.id);
    let integration = create_integration(&database.store, org.id, agent.id);
    let first = database
        .store
        .inbox()
        .receive_from_channel(
            org.id,
            entry_from_channel(integration, &agent, "M-RE-1", Some("Ana")),
        )
        .expect("primeira");
    let repeated_item = database
        .store
        .inbox()
        .receive_from_channel(
            org.id,
            entry_from_channel(integration, &agent, "M-RE-1", Some("Outra")),
        )
        .expect("repetida");
    assert!(matches!(repeated_item.situation, Received::Duplicate));
    assert_eq!(repeated_item.mensagem.id, first.mensagem.id);

    let conversation = database
        .store
        .conversations()
        .get(org.id, first.mensagem.conversation_id)
        .expect("conversa");
    assert_eq!(conversation.customer_name.as_deref(), Some("Ana"));
}
