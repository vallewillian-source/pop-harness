use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use poph_store::migrations::migrate;
use poph_store::{
    Agente, Conversa, Departamento, Direction, HARNESS_MIGRATIONS, NewAgent, NewConversation,
    NewDepartment, NewMessage, NewOrg, Org, Origin, Role, SpaceOrigin, StateRoom, Store,
    StoreError, file_version,
};
use poph_types::{ConversationId, DomainError, MessageId, OrgId};

#[derive(Clone)]
struct Clock(Arc<AtomicI64>);

impl Clock {
    fn new(initial: i64) -> Self {
        Self(Arc::new(AtomicI64::new(initial)))
    }

    fn define(&self, value: i64) {
        self.0.store(value, Ordering::SeqCst);
    }
}

struct Scenario {
    folder: PathBuf,
    clock: Clock,
    store: Option<Arc<Store>>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        Self::with_version(label, None)
    }

    fn with_version(label: &str, until: Option<usize>) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder =
            std::env::temp_dir().join(format!("poph-matrix-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("cria a pasta do teste");
        let harness = folder.join("harness.db");
        let rag = folder.join("rag.db");
        if let Some(until) = until {
            let conn = rusqlite::Connection::open(&harness).expect("abre o banco antigo");
            migrate(
                &conn,
                "harness.db",
                &harness,
                &HARNESS_MIGRATIONS[..until],
                1,
            )
            .expect("migra até a versão antiga");
        }
        let clock = Clock::new(1_000);
        let now = Arc::clone(&clock.0);
        let store = Store::open_at(&harness, &rag)
            .expect("abre o banco")
            .with_clock(Arc::new(move || now.load(Ordering::SeqCst)));
        Self {
            folder,
            clock,
            store: Some(Arc::new(store)),
        }
    }

    fn store(&self) -> &Store {
        self.store.as_deref().expect("store viva")
    }

    fn path(&self) -> PathBuf {
        self.folder.join("harness.db")
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

fn create_agent(store: &Store, org: &Org, slug: &str) -> Agente {
    store
        .agents()
        .create(
            org.id,
            NewAgent {
                name: format!("Agente {slug}"),
                slug: slug.to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente")
}

fn create_conversation(store: &Store, org: &Org, agent: &Agente) -> Conversa {
    store
        .conversations()
        .create(
            org.id,
            NewConversation {
                agent_id: agent.id,
                integration_id: None,
                canal: "cli".to_owned(),
                peer_externo: Some("teste".to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa")
}

fn create_department(store: &Store, org: &Org, nome: &str) -> Departamento {
    store
        .departments()
        .create(
            org.id,
            NewDepartment {
                name: nome.to_owned(),
                description: String::new(),
            },
        )
        .expect("cria o departamento")
}

fn write_message(store: &Store, org: &Org, conversation: &Conversa, texto: &str) -> MessageId {
    store
        .messages()
        .write(
            org.id,
            NewMessage {
                conversation_id: conversation.id,
                direction: Direction::Inbound,
                role: Role::User,
                texto: texto.to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
                origin: Origin::Agente,
                author_name: None,
            },
        )
        .expect("grava a mensagem")
        .id
}

fn table_rows(path: &PathBuf, table: &str, org: OrgId) -> i64 {
    let conn = rusqlite::Connection::open(path).expect("abre para conferir");
    conn.query_row(
        &format!("SELECT count(*) FROM {table} WHERE org_id = ?1"),
        rusqlite::params![org.to_string()],
        |row| row.get(0),
    )
    .expect("conta as linhas")
}

#[test]
fn migration_creates_five_tables_of_module() {
    let scenario = Scenario::new("esquema");
    let store = scenario.store();

    let tabelas = store.tables().expect("lista tabelas");
    for expected in [
        "matrix_orgs",
        "matrix_rooms",
        "matrix_spaces",
        "matrix_estado",
        "matrix_falhas",
    ] {
        assert!(
            tabelas.iter().any(|table| table == expected),
            "tabela {expected} não criada; existem: {tabelas:?}"
        );
    }
    assert_eq!(
        file_version(&scenario.path()).expect("lê versão"),
        Some(poph_store::harness_target_version())
    );
    assert!(store.broken_foreign_keys().expect("fk check").is_empty());
}

#[test]
fn migration_applies_to_database_that_already_had_data() {
    let scenario = Scenario::with_version("com-dados", Some(4));
    let org_id = uuid::Uuid::now_v7();
    let agent_id = uuid::Uuid::now_v7();
    let conversation_id = uuid::Uuid::now_v7();
    let message_id = uuid::Uuid::now_v7();
    {
        let conn = rusqlite::Connection::open(scenario.path()).expect("abre o banco antigo");
        conn.execute_batch(&format!(
            "INSERT INTO orgs (id, name, slug, created_at, updated_at)
                VALUES ('{org_id}', 'Org antiga', 'org-antiga', 1, 1);
             INSERT INTO agents (id, org_id, name, slug, provider, model, created_at, updated_at)
                VALUES ('{agent_id}', '{org_id}', 'Agente', 'agente', 'antigravity2api',
                        'gemini', 1, 1);
             INSERT INTO conversations (id, org_id, agent_id, channel, created_at, updated_at)
                VALUES ('{conversation_id}', '{org_id}', '{agent_id}', 'cli', 1, 1);
             INSERT INTO messages (id, org_id, conversation_id, direction, role, text, created_at)
                VALUES ('{message_id}', '{org_id}', '{conversation_id}', 'in', 'user',
                        'mensagem antiga', 1);"
        ))
        .expect("grava dados na versão anterior");
    }

    let store = scenario.store();
    assert_eq!(
        file_version(&scenario.path()).expect("lê versão"),
        Some(poph_store::harness_target_version())
    );
    assert!(store.broken_foreign_keys().expect("fk check").is_empty());

    let org: OrgId = org_id.to_string().parse().expect("org válida");
    let conversation: ConversationId = conversation_id
        .to_string()
        .parse()
        .expect("conversa válida");
    store
        .conversations()
        .get(org, conversation)
        .expect("a conversa antiga continua no banco");

    let state = store.matrix().enable(org).expect("liga o módulo");
    assert!(state.linked);
    let room = store
        .matrix()
        .ensure_room(org, conversation, "!antiga:teste", None)
        .expect("sala da conversa antiga");
    assert_eq!(room.state, StateRoom::Creating);
    assert_eq!(room.conversation, conversation);
}

#[test]
fn deleting_org_deletes_rows_of_five_tables() {
    let scenario = Scenario::new("cascade");
    let store = scenario.store();

    let mut orgs = Vec::new();
    for slug in ["org-a", "org-b"] {
        let org = create_org(store, slug);
        let agent = create_agent(store, &org, slug);
        let conversation = create_conversation(store, &org, &agent);
        let departamento = create_department(store, &org, "Comercial");
        let mensagem = write_message(store, &org, &conversation, "oi");

        store.matrix().enable(org.id).expect("liga");
        store
            .matrix()
            .ensure_room(org.id, conversation.id, &format!("!{slug}:teste"), None)
            .expect("sala");
        store
            .matrix()
            .ensure_space(
                org.id,
                departamento.id,
                &format!("!espaco-{slug}:teste"),
                &format!("#espaco-{slug}:teste"),
                SpaceOrigin::Automatic,
            )
            .expect("espaço");
        store
            .matrix()
            .advance_messages(org.id, 10)
            .expect("cursor de mensagens");
        store
            .matrix()
            .register_failure(org.id, mensagem, "M_UNKNOWN", 1)
            .expect("falha");
        orgs.push(org);
    }

    let (org_a, org_b) = (&orgs[0], &orgs[1]);
    let path = scenario.path();
    for table in [
        "matrix_orgs",
        "matrix_rooms",
        "matrix_spaces",
        "matrix_estado",
        "matrix_falhas",
    ] {
        assert_eq!(
            table_rows(&path, table, org_a.id),
            1,
            "{table} da org A antes da exclusão"
        );
        assert_eq!(
            table_rows(&path, table, org_b.id),
            1,
            "{table} da org B antes da exclusão"
        );
    }

    store.orgs().delete_forced(org_b.id).expect("apaga a org B");

    for table in [
        "matrix_orgs",
        "matrix_rooms",
        "matrix_spaces",
        "matrix_estado",
        "matrix_falhas",
    ] {
        assert_eq!(
            table_rows(&path, table, org_b.id),
            0,
            "{table} da org excluída ficou para trás"
        );
        assert_eq!(
            table_rows(&path, table, org_a.id),
            1,
            "{table} da outra org não podia ser apagada"
        );
    }
    assert!(store.broken_foreign_keys().expect("fk check").is_empty());
}

#[test]
fn org_isolation_in_all_tables_of_module() {
    let scenario = Scenario::new("isolamento");
    let store = scenario.store();
    let org_a = create_org(store, "org-a");
    let org_b = create_org(store, "org-b");
    let agent_a = create_agent(store, &org_a, "agente-a");
    let conversation_a = create_conversation(store, &org_a, &agent_a);
    let department_a = create_department(store, &org_a, "Comercial");
    let message_a = write_message(store, &org_a, &conversation_a, "oi");

    store.matrix().enable(org_a.id).expect("liga A");
    store
        .matrix()
        .ensure_room(org_a.id, conversation_a.id, "!sala-a:teste", None)
        .expect("sala A");
    store
        .matrix()
        .ensure_space(
            org_a.id,
            department_a.id,
            "!espaco-a:teste",
            "#espaco-a:teste",
            SpaceOrigin::Automatic,
        )
        .expect("espaço A");
    store
        .matrix()
        .advance_messages(org_a.id, 7)
        .expect("cursor A");
    store
        .matrix()
        .advance_conversations(org_a.id, 500, &conversation_a.id.to_string())
        .expect("cursor de conversas A");
    store
        .matrix()
        .register_failure(org_a.id, message_a, "M_UNKNOWN", 1)
        .expect("falha A");

    let state_b = store.matrix().state(org_b.id).expect("estado B");
    assert!(!state_b.linked);
    assert_eq!(state_b.updated_at, 0);
    assert!(
        store
            .matrix()
            .linked_orgs()
            .expect("orgs ligadas")
            .iter()
            .all(|state| state.org_id != org_b.id)
    );

    assert!(
        store
            .matrix()
            .room(org_b.id, conversation_a.id)
            .expect("sala B")
            .is_none(),
        "a sala da org A não pode ser lida pela org B"
    );
    assert!(
        store
            .matrix()
            .room_by_room_id(org_b.id, "!sala-a:teste")
            .expect("sala por id B")
            .is_none(),
        "o id de sala da org A não resolve na org B"
    );
    assert!(
        store
            .matrix()
            .space(org_b.id, department_a.id)
            .expect("espaço B")
            .is_none()
    );
    assert!(
        store
            .matrix()
            .list_spaces(org_b.id)
            .expect("espaços B")
            .is_empty()
    );
    let cursors_b = store.matrix().read_cursors(org_b.id).expect("cursores B");
    assert_eq!(cursors_b.messages, 0);
    assert_eq!(cursors_b.conversations.updated_at, 0);
    assert_eq!(cursors_b.version, 0);
    assert_eq!(
        store.matrix().count_failures(org_b.id).expect("falhas B"),
        0
    );

    let error = store
        .matrix()
        .ensure_room(org_b.id, conversation_a.id, "!sala-invasora:teste", None)
        .expect_err("conversa de outra org tem que ser recusada");
    assert!(
        matches!(
            error,
            StoreError::Domain(DomainError::Validation { .. })
                | StoreError::Domain(DomainError::NotFound { .. })
        ),
        "erro inesperado: {error:?}"
    );

    let cursors_a = store.matrix().read_cursors(org_a.id).expect("cursores A");
    assert_eq!(cursors_a.messages, 7);
    assert_eq!(cursors_a.conversations.updated_at, 500);
    assert_eq!(
        store.matrix().count_failures(org_a.id).expect("falhas A"),
        1
    );
}

#[test]
fn ensure_room_twice_is_idempotent_and_refuses_room_of_another_conversation() {
    let scenario = Scenario::new("sala-idempotente");
    let store = scenario.store();
    let org = create_org(store, "org");
    let agent = create_agent(store, &org, "agente");
    let conversation = create_conversation(store, &org, &agent);
    let other = create_conversation(store, &org, &agent);

    let first = store
        .matrix()
        .ensure_room(
            org.id,
            conversation.id,
            "!sala:teste",
            Some("!espaco:teste"),
        )
        .expect("primeira");
    let second = store
        .matrix()
        .ensure_room(
            org.id,
            conversation.id,
            "!sala:teste",
            Some("!espaco:teste"),
        )
        .expect("segunda");
    assert_eq!(first, second, "a segunda chamada devolve a mesma linha");
    assert_eq!(table_rows(&scenario.path(), "matrix_rooms", org.id), 1);

    let third = store
        .matrix()
        .ensure_room(org.id, conversation.id, "!outra-sala:teste", None)
        .expect("terceira");
    assert_eq!(third.room_id, "!sala:teste");
    assert_eq!(third.space.as_deref(), Some("!espaco:teste"));

    let error = store
        .matrix()
        .ensure_room(org.id, other.id, "!sala:teste", None)
        .expect_err("sala de outra conversa tem que ser recusada");
    assert!(
        matches!(error, StoreError::Domain(DomainError::Conflict { .. })),
        "erro inesperado: {error:?}"
    );
    assert_eq!(table_rows(&scenario.path(), "matrix_rooms", org.id), 1);
}

#[test]
fn mark_room_ready_is_idempotent() {
    let scenario = Scenario::new("pronta");
    let store = scenario.store();
    let org = create_org(store, "org");
    let agent = create_agent(store, &org, "agente");
    let conversation = create_conversation(store, &org, &agent);
    let ghost = create_conversation(store, &org, &agent);

    let room = store
        .matrix()
        .ensure_room(org.id, conversation.id, "!sala:teste", None)
        .expect("sala");
    assert_eq!(room.state, StateRoom::Creating);

    scenario.clock.define(2_000);
    let ready = store
        .matrix()
        .mark_room_ready(org.id, conversation.id)
        .expect("pronta");
    assert_eq!(ready.state, StateRoom::Ready);
    assert_eq!(ready.updated_at, 2_000);

    scenario.clock.define(3_000);
    let from_new = store
        .matrix()
        .mark_room_ready(org.id, conversation.id)
        .expect("pronta de novo");
    assert_eq!(from_new.state, StateRoom::Ready);
    assert_eq!(from_new.updated_at, 2_000, "já pronta não é reescrita");

    let error = store
        .matrix()
        .mark_room_ready(org.id, ghost.id)
        .expect_err("conversa sem sala");
    assert!(error.is_not_found(), "erro inesperado: {error:?}");
}

#[test]
fn marking_room_ready_writes_customer_name_that_room_uses() {
    let scenario = Scenario::new("pronta-com-nome");
    let store = scenario.store();
    let org = create_org(store, "org");
    let agent = create_agent(store, &org, "agente");
    let conversation = create_conversation(store, &org, &agent);
    let without_name = create_conversation(store, &org, &agent);

    store
        .matrix()
        .ensure_room(org.id, conversation.id, "!sala:teste", None)
        .expect("sala");
    let creating = store
        .matrix()
        .room(org.id, conversation.id)
        .expect("lê a sala")
        .expect("sala existe");
    assert_eq!(creating.name_client_mirrored, None, "nasce sem nome");

    scenario.clock.define(2_000);
    let ready = store
        .matrix()
        .mark_room_ready_with_name(org.id, conversation.id, Some("Maria"))
        .expect("pronta com nome");
    assert_eq!(ready.state, StateRoom::Ready);
    assert_eq!(ready.name_client_mirrored.as_deref(), Some("Maria"));
    assert_eq!(ready.updated_at, 2_000);

    scenario.clock.define(3_000);
    let from_new = store
        .matrix()
        .mark_room_ready_with_name(org.id, conversation.id, Some("Outro"))
        .expect("pronta de novo");
    assert_eq!(from_new.name_client_mirrored.as_deref(), Some("Maria"));
    assert_eq!(from_new.updated_at, 2_000);

    store
        .matrix()
        .ensure_room(org.id, without_name.id, "!sala-sem-nome:teste", None)
        .expect("sala sem nome");
    let ready = store
        .matrix()
        .mark_room_ready(org.id, without_name.id)
        .expect("pronta sem nome");
    assert_eq!(ready.name_client_mirrored, None);
}

#[test]
fn setting_mirrored_customer_name_is_idempotent_and_filtered_by_org() {
    let scenario = Scenario::new("nome-espelhado");
    let store = scenario.store();
    let org = create_org(store, "org");
    let other_org = create_org(store, "outra-org");
    let agent = create_agent(store, &org, "agente");
    let conversation = create_conversation(store, &org, &agent);
    store
        .matrix()
        .ensure_room(org.id, conversation.id, "!sala:teste", None)
        .expect("sala");
    store
        .matrix()
        .mark_room_ready(org.id, conversation.id)
        .expect("pronta");

    scenario.clock.define(2_000);
    let with_name = store
        .matrix()
        .set_mirrored_customer_name(org.id, conversation.id, "Ana")
        .expect("grava o nome");
    assert_eq!(with_name.name_client_mirrored.as_deref(), Some("Ana"));
    assert_eq!(with_name.updated_at, 2_000);

    scenario.clock.define(3_000);
    let from_new = store
        .matrix()
        .set_mirrored_customer_name(org.id, conversation.id, "Ana")
        .expect("grava de novo");
    assert_eq!(from_new.updated_at, 2_000, "o mesmo nome não reescreve");

    let swapped = store
        .matrix()
        .set_mirrored_customer_name(org.id, conversation.id, "Ana Maria")
        .expect("nome novo");
    assert_eq!(swapped.name_client_mirrored.as_deref(), Some("Ana Maria"));

    let error = store
        .matrix()
        .set_mirrored_customer_name(other_org.id, conversation.id, "Invasora")
        .expect_err("sala de outra org");
    assert!(error.is_not_found(), "erro inesperado: {error:?}");
    assert!(
        store
            .matrix()
            .room(other_org.id, conversation.id)
            .expect("lê pela outra org")
            .is_none()
    );

    let empty = store
        .matrix()
        .set_mirrored_customer_name(org.id, conversation.id, "   ")
        .expect_err("nome vazio");
    assert!(
        matches!(empty, StoreError::Domain(DomainError::Validation { .. })),
        "erro inesperado: {empty:?}"
    );
}

#[test]
fn mirrored_name_migration_keeps_existing_room_with_empty_column() {
    let scenario = Scenario::with_version("nome-espelhado-antigo", Some(9));
    let org_id = uuid::Uuid::now_v7();
    let agent_id = uuid::Uuid::now_v7();
    let conversation_id = uuid::Uuid::now_v7();
    {
        let conn = rusqlite::Connection::open(scenario.path()).expect("abre o banco antigo");
        conn.execute_batch(&format!(
            "INSERT INTO orgs (id, name, slug, created_at, updated_at)
                VALUES ('{org_id}', 'Org antiga', 'org-antiga', 1, 1);
             INSERT INTO agents (id, org_id, name, slug, provider, model, created_at, updated_at)
                VALUES ('{agent_id}', '{org_id}', 'Agente', 'agente', 'antigravity2api',
                        'gemini', 1, 1);
             INSERT INTO conversations (id, org_id, agent_id, channel, created_at, updated_at)
                VALUES ('{conversation_id}', '{org_id}', '{agent_id}', 'whatsapp', 1, 1);
             INSERT INTO matrix_rooms
                (conversation_id, org_id, room_id, state, created_at, updated_at)
                VALUES ('{conversation_id}', '{org_id}', '!sala-antiga:teste', 'ready', 1, 1);"
        ))
        .expect("grava a sala na versão anterior");
    }

    let store = scenario.store();
    assert_eq!(
        file_version(&scenario.path()).expect("lê versão"),
        Some(poph_store::harness_target_version())
    );
    let org: OrgId = org_id.to_string().parse().expect("org válida");
    let conversation: ConversationId = conversation_id
        .to_string()
        .parse()
        .expect("conversa válida");
    let room = store
        .matrix()
        .room(org, conversation)
        .expect("lê a sala")
        .expect("a sala antiga continua no banco");
    assert_eq!(room.room_id, "!sala-antiga:teste");
    assert_eq!(room.state, StateRoom::Ready);
    assert_eq!(room.name_client_mirrored, None);
}

#[test]
fn setting_room_space_is_idempotent_and_accepts_removing_room_from_space() {
    let scenario = Scenario::new("espaco-da-sala");
    let store = scenario.store();
    let org = create_org(store, "org");
    let agent = create_agent(store, &org, "agente");
    let conversation = create_conversation(store, &org, &agent);
    let other_org = create_org(store, "outra-org");

    store
        .matrix()
        .ensure_room(org.id, conversation.id, "!sala:teste", None)
        .expect("sala");

    scenario.clock.define(2_000);
    let posta = store
        .matrix()
        .set_space_of_room(org.id, conversation.id, Some("!espaco:teste"))
        .expect("põe no espaço");
    assert_eq!(posta.space.as_deref(), Some("!espaco:teste"));
    assert_eq!(posta.updated_at, 2_000);

    scenario.clock.define(3_000);
    let from_new = store
        .matrix()
        .set_space_of_room(org.id, conversation.id, Some("!espaco:teste"))
        .expect("põe de novo");
    assert_eq!(from_new.space.as_deref(), Some("!espaco:teste"));
    assert_eq!(from_new.updated_at, 2_000, "o mesmo espaço não reescreve");

    let moved = store
        .matrix()
        .set_space_of_room(org.id, conversation.id, Some("!outro-espaco:teste"))
        .expect("move de espaço");
    assert_eq!(moved.space.as_deref(), Some("!outro-espaco:teste"));
    assert_eq!(moved.updated_at, 3_000);

    let without_space = store
        .matrix()
        .set_space_of_room(org.id, conversation.id, None)
        .expect("tira do espaço");
    assert_eq!(without_space.space, None);

    let error = store
        .matrix()
        .set_space_of_room(other_org.id, conversation.id, Some("!espaco:teste"))
        .expect_err("sala de outra org");
    assert!(error.is_not_found(), "erro inesperado: {error:?}");
    assert_eq!(
        store
            .matrix()
            .room(org.id, conversation.id)
            .expect("sala")
            .expect("existe")
            .space,
        None,
        "a tentativa de outra org não pode mexer na sala"
    );
}

#[test]
fn linking_and_unlinking_are_idempotent_and_backfill_cutoff_follows_last_link() {
    let scenario = Scenario::new("org");
    let store = scenario.store();
    let org = create_org(store, "org");

    let initial = store.matrix().state(org.id).expect("estado inicial");
    assert!(!initial.linked);
    assert_eq!(initial.linked_at, None);
    assert_eq!(initial.updated_at, 0);

    scenario.clock.define(1_000);
    let linked = store.matrix().enable(org.id).expect("liga");
    assert!(linked.linked);
    assert_eq!(linked.linked_at, Some(1_000));
    assert_eq!(store.matrix().linked_orgs().expect("ligadas").len(), 1);

    scenario.clock.define(2_000);
    let repeated_item = store.matrix().enable(org.id).expect("liga de novo");
    assert_eq!(repeated_item, linked, "a segunda ligação é um no-op");

    scenario.clock.define(3_000);
    let disabled = store.matrix().disable(org.id).expect("desliga");
    assert!(!disabled.linked);
    assert_eq!(disabled.linked_at, Some(1_000), "o corte não é apagado");
    assert_eq!(disabled.updated_at, 3_000);

    scenario.clock.define(4_000);
    let disabled_again = store.matrix().disable(org.id).expect("desliga de novo");
    assert_eq!(disabled_again, disabled);
    assert!(
        store.matrix().linked_orgs().expect("ligadas").is_empty(),
        "org desligada não aparece para o laço"
    );

    scenario.clock.define(5_000);
    let religada = store.matrix().enable(org.id).expect("religa");
    assert!(religada.linked);
    assert_eq!(religada.linked_at, Some(5_000));
}

#[test]
fn cursors_only_advance() {
    let scenario = Scenario::new("cursores");
    let store = scenario.store();
    let org = create_org(store, "org");

    let zero = store.matrix().read_cursors(org.id).expect("cursores");
    assert_eq!(zero.messages, 0);
    assert_eq!(zero.version, 0);

    let andou = store.matrix().advance_messages(org.id, 10).expect("10");
    assert_eq!(andou.messages, 10);
    assert_eq!(andou.version, 1);

    let returned = store.matrix().advance_messages(org.id, 5).expect("5");
    assert_eq!(returned.messages, 10, "o cursor não volta");
    assert_eq!(returned.version, 1, "nem a versão sobe à toa");

    let same = store
        .matrix()
        .advance_messages(org.id, 10)
        .expect("10 de novo");
    assert_eq!(same, returned, "repetir o mesmo lote é no-op");

    let forward = store.matrix().advance_messages(org.id, 12).expect("12");
    assert_eq!(forward.messages, 12);
    assert_eq!(forward.version, 2);

    let conversations = store
        .matrix()
        .advance_conversations(org.id, 500, "conv-b")
        .expect("conversas");
    assert_eq!(conversations.conversations.updated_at, 500);
    assert_eq!(conversations.conversations.id, "conv-b");

    let atras = store
        .matrix()
        .advance_conversations(org.id, 500, "conv-a")
        .expect("conversas atrás");
    assert_eq!(atras.conversations.id, "conv-b");
    assert_eq!(atras.conversations.updated_at, 500);

    let advanced = store
        .matrix()
        .advance_conversations(org.id, 501, "conv-a")
        .expect("conversas à frente");
    assert_eq!(advanced.conversations.updated_at, 501);
    assert_eq!(advanced.conversations.id, "conv-a");
}

#[test]
fn failure_of_same_message_updates_instead_of_duplicating() {
    let scenario = Scenario::new("falhas");
    let store = scenario.store();
    let org = create_org(store, "org");
    let mensagem = MessageId::generate();

    scenario.clock.define(1_000);
    let first = store
        .matrix()
        .register_failure(org.id, mensagem, "M_UNKNOWN", 1)
        .expect("primeira falha");
    assert_eq!(first.motivo, "M_UNKNOWN");
    assert_eq!(first.attempts, 1);
    assert_eq!(first.created_at, 1_000);

    scenario.clock.define(2_000);
    let second = store
        .matrix()
        .register_failure(org.id, mensagem, "timeout", 3)
        .expect("segunda falha");
    assert_eq!(second.id, first.id, "é a mesma linha");
    assert_eq!(second.motivo, "timeout");
    assert_eq!(second.attempts, 3);
    assert_eq!(second.created_at, 1_000, "a criação fica");
    assert_eq!(second.updated_at, 2_000);
    assert_eq!(store.matrix().count_failures(org.id).expect("conta"), 1);

    let listed = store.matrix().list_failures(org.id, 50).expect("lista");
    assert_eq!(listed.len(), 1);

    assert!(
        store
            .matrix()
            .remove_failure(org.id, mensagem)
            .expect("remove")
    );
    assert!(
        !store
            .matrix()
            .remove_failure(org.id, mensagem)
            .expect("remove de novo")
    );
    assert_eq!(store.matrix().count_failures(org.id).expect("conta"), 0);

    let empty = store
        .matrix()
        .register_failure(org.id, mensagem, "  ", 1)
        .expect_err("motivo vazio");
    assert!(matches!(
        empty,
        StoreError::Domain(DomainError::Validation { .. })
    ));
    let long = store
        .matrix()
        .register_failure(org.id, mensagem, &"x".repeat(65), 1)
        .expect_err("motivo longo");
    assert!(matches!(
        long,
        StoreError::Domain(DomainError::Validation { .. })
    ));
}

#[test]
fn ensure_space_is_idempotent_per_department_and_refuses_space_of_another() {
    let scenario = Scenario::new("espacos");
    let store = scenario.store();
    let org = create_org(store, "org");
    let commercial = create_department(store, &org, "Comercial");
    let support = create_department(store, &org, "Suporte");

    let first_value = store
        .matrix()
        .ensure_space(
            org.id,
            commercial.id,
            "!espaco:teste",
            "#espaco:teste",
            SpaceOrigin::Automatic,
        )
        .expect("primeiro");
    let second_value = store
        .matrix()
        .ensure_space(
            org.id,
            commercial.id,
            "!espaco:teste",
            "#espaco:teste",
            SpaceOrigin::Linked,
        )
        .expect("segundo");
    assert_eq!(
        first_value, second_value,
        "a segunda chamada devolve o mesmo espaço"
    );
    assert_eq!(
        second_value.origin,
        SpaceOrigin::Automatic,
        "a primeira ganha"
    );
    assert_eq!(store.matrix().list_spaces(org.id).expect("lista").len(), 1);

    let error = store
        .matrix()
        .ensure_space(
            org.id,
            support.id,
            "!espaco:teste",
            "#outro-alias:teste",
            SpaceOrigin::Linked,
        )
        .expect_err("espaço de outro departamento");
    assert!(
        matches!(error, StoreError::Domain(DomainError::Conflict { .. })),
        "erro inesperado: {error:?}"
    );

    let another = store
        .matrix()
        .ensure_space(
            org.id,
            support.id,
            "!espaco-suporte:teste",
            "#espaco-suporte:teste",
            SpaceOrigin::Linked,
        )
        .expect("espaço do suporte");
    assert_eq!(another.departamento, support.id);
    assert_eq!(store.matrix().list_spaces(org.id).expect("lista").len(), 2);
}

#[test]
fn no_column_of_module_tables_stores_customer_text_phone_or_token() {
    let scenario = Scenario::new("colunas");
    let store = scenario.store();

    let expected_value: [(&str, &[&str]); 5] = [
        (
            "matrix_orgs",
            &["org_id", "enabled", "enabled_at", "updated_at"],
        ),
        (
            "matrix_rooms",
            &[
                "conversation_id",
                "org_id",
                "room_id",
                "space_room_id",
                "last_status",
                "client_name_espelhado",
                "state",
                "created_at",
                "updated_at",
            ],
        ),
        (
            "matrix_spaces",
            &[
                "id",
                "org_id",
                "department_id",
                "space_room_id",
                "alias",
                "origin",
                "created_at",
                "updated_at",
            ],
        ),
        (
            "matrix_estado",
            &[
                "org_id",
                "messages_cursor",
                "messages_cursor_id",
                "conversations_cursor_at",
                "conversations_cursor_id",
                "version",
                "updated_at",
            ],
        ),
        (
            "matrix_falhas",
            &[
                "id",
                "org_id",
                "message_id",
                "motivo",
                "tentativas",
                "created_at",
                "updated_at",
            ],
        ),
    ];

    for (table, columns_expected) in expected_value {
        let columns = store.columns(table).expect("lê colunas");
        let mut columns_sorted = columns.clone();
        columns_sorted.sort();
        let mut expected_sorted = columns_expected.to_vec();
        expected_sorted.sort();
        assert_eq!(
            columns_sorted, expected_sorted,
            "colunas de {table} mudaram: fora `client_name_espelhado` (MX-33), nenhuma coluna do \
             módulo guarda texto de cliente"
        );
        assert!(
            columns.iter().any(|column| column == "org_id"),
            "tabela {table} sem org_id"
        );
    }
}

#[test]
fn message_delay_counts_only_what_still_needs_mirroring() {
    let scenario = Scenario::new("atraso");
    let store = scenario.store();
    let org = create_org(store, "org");
    let agent = create_agent(store, &org, "agente");
    let conversation = create_conversation(store, &org, &agent);
    let other_org = create_org(store, "outra-org");
    let other_agent = create_agent(store, &other_org, "agente");
    let other_conversation = create_conversation(store, &other_org, &other_agent);

    assert_eq!(
        store.matrix().delay_of_messages(org.id, 0).expect("vazio"),
        poph_store::DelayMatrix::default(),
        "sem mensagem, o atraso é zero e não há a mais antiga"
    );

    scenario.clock.define(5_000);
    let first = write_message(store, &org, &conversation, "primeira");
    scenario.clock.define(6_000);
    write_message(store, &org, &conversation, "segunda");
    store
        .messages()
        .write(
            org.id,
            NewMessage {
                conversation_id: conversation.id,
                direction: Direction::Outbound,
                role: Role::Tool,
                texto: "chamada de ferramenta".to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
                origin: Origin::Agente,
                author_name: None,
            },
        )
        .expect("mensagem de ferramenta");
    write_message(store, &other_org, &other_conversation, "da outra org");

    let delay = store
        .matrix()
        .delay_of_messages(org.id, 0)
        .expect("atraso da org");
    assert_eq!(delay.pending, 2, "só as duas mensagens espelháveis");
    assert_eq!(delay.oldest_at, Some(5_000));

    let rowid = store
        .messages()
        .since(0, 10)
        .expect("lê as mensagens")
        .into_iter()
        .find(|mensagem| mensagem.id == first)
        .expect("primeira mensagem na leitura incremental")
        .rowid;
    let later = store
        .matrix()
        .delay_of_messages(org.id, rowid)
        .expect("atraso depois do cursor");
    assert_eq!(later.pending, 1);
    assert_eq!(later.oldest_at, Some(6_000));

    assert_eq!(
        store
            .matrix()
            .delay_of_messages(other_org.id, 0)
            .expect("atraso da outra org")
            .pending,
        1
    );
}

#[test]
fn disabling_space_deletes_row_and_forgets_rooms_without_touching_rest() {
    let scenario = Scenario::new("desliga-espaco");
    let store = scenario.store();
    let org = create_org(store, "org");
    let agent = create_agent(store, &org, "agente");
    let conversation = create_conversation(store, &org, &agent);
    let other_conversation = create_conversation(store, &org, &agent);
    let commercial = create_department(store, &org, "Comercial");
    let support = create_department(store, &org, "Suporte");
    let other_org = create_org(store, "outra-org");
    let other_agent = create_agent(store, &other_org, "agente");
    let conversation_of_other = create_conversation(store, &other_org, &other_agent);
    let department_of_other = create_department(store, &other_org, "Comercial");

    store
        .matrix()
        .ensure_space(
            org.id,
            commercial.id,
            "!espaco:teste",
            "#dep_comercial:teste",
            SpaceOrigin::Automatic,
        )
        .expect("espaço do comercial");
    store
        .matrix()
        .ensure_space(
            org.id,
            support.id,
            "!suporte:teste",
            "#dep_suporte:teste",
            SpaceOrigin::Linked,
        )
        .expect("espaço do suporte");
    store
        .matrix()
        .ensure_room(
            org.id,
            conversation.id,
            "!sala:teste",
            Some("!espaco:teste"),
        )
        .expect("sala no espaço");
    store
        .matrix()
        .ensure_room(
            org.id,
            other_conversation.id,
            "!sala-2:teste",
            Some("!suporte:teste"),
        )
        .expect("outra sala no espaço do suporte");
    store
        .matrix()
        .ensure_space(
            other_org.id,
            department_of_other.id,
            "!espaco-da-outra:teste",
            "#dep_da_outra:teste",
            SpaceOrigin::Automatic,
        )
        .expect("espaço da outra org");
    store
        .matrix()
        .ensure_room(
            other_org.id,
            conversation_of_other.id,
            "!sala-da-outra:teste",
            Some("!espaco-da-outra:teste"),
        )
        .expect("sala da outra org");

    scenario.clock.define(9_000);
    let switched_off = store
        .matrix()
        .disable_space(org.id, commercial.id)
        .expect("desliga o espaço");
    assert!(switched_off.removed, "a linha do espaço saiu");
    assert_eq!(switched_off.rooms_forgotten, 1, "só a sala daquele espaço");

    assert_eq!(
        store.matrix().space(org.id, commercial.id).expect("lê"),
        None,
        "o departamento ficou sem espaço gravado"
    );
    assert_eq!(
        store
            .matrix()
            .room(org.id, conversation.id)
            .expect("sala")
            .expect("existe")
            .space,
        None,
        "a sala deixou de estar registrada no espaço"
    );
    assert_eq!(
        store
            .matrix()
            .room(org.id, other_conversation.id)
            .expect("sala do suporte")
            .expect("existe")
            .space
            .as_deref(),
        Some("!suporte:teste"),
        "o outro espaço não foi tocado"
    );
    assert!(
        store
            .matrix()
            .space(org.id, support.id)
            .expect("lê")
            .is_some(),
        "o espaço do suporte continua gravado"
    );
    assert_eq!(
        store
            .matrix()
            .room(other_org.id, conversation_of_other.id)
            .expect("sala da outra org")
            .expect("existe")
            .space
            .as_deref(),
        Some("!espaco-da-outra:teste"),
        "a outra org não é afetada"
    );
    assert_eq!(
        store
            .matrix()
            .room(org.id, conversation.id)
            .expect("sala")
            .expect("existe")
            .state,
        StateRoom::Creating,
        "a linha da sala continua lá (a da MX-08 fica em `creating` até ficar pronta)"
    );

    let from_new = store
        .matrix()
        .disable_space(org.id, commercial.id)
        .expect("desliga de novo");
    assert_eq!(
        from_new,
        poph_store::SpaceDisabled::default(),
        "sem espaço gravado, nada acontece"
    );
}
