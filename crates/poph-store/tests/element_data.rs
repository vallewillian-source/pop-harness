use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use poph_store::{Agente, Conversa, NewAgent, NewConversation, NewOrg, Org, Store};
use poph_types::ConversationId;

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder = std::env::temp_dir().join(format!(
            "poph-dados-element-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("cria a pasta do teste");
        let clock = Arc::new(AtomicI64::new(1_000));
        let now = Arc::clone(&clock);
        let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
            .expect("abre o banco")
            .with_clock(Arc::new(move || now.fetch_add(10, Ordering::SeqCst)));
        Self {
            folder,
            store: Arc::new(store),
        }
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
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

#[test]
fn agent_names_come_from_single_query_and_are_filtered_by_org() {
    let scenario = Scenario::new("nomes");
    let store = &scenario.store;
    let a = create_org(store, "a");
    let b = create_org(store, "b");
    let a1 = create_agent(store, &a, "a1");
    let a2 = create_agent(store, &a, "a2");
    let b1 = create_agent(store, &b, "b1");

    let before = store.reads_done();
    let names = store
        .agents()
        .names(a.id, &[a1.id, a2.id, b1.id])
        .expect("nomes");
    assert_eq!(
        store.reads_done() - before,
        1,
        "os nomes de vários agentes saem numa leitura só"
    );
    assert_eq!(names.get(&a1.id).map(String::as_str), Some("Agente a1"));
    assert_eq!(names.get(&a2.id).map(String::as_str), Some("Agente a2"));
    assert!(
        !names.contains_key(&b1.id),
        "agente de outra org não entra no resultado"
    );

    let empty = store.agents().names(a.id, &[]).expect("lista vazia");
    assert!(empty.is_empty());
}

#[test]
fn ready_rooms_come_from_single_query_only_with_state_ready_and_by_org() {
    let scenario = Scenario::new("salas");
    let store = &scenario.store;
    let a = create_org(store, "a");
    let b = create_org(store, "b");
    let agent_a = create_agent(store, &a, "a");
    let agent_b = create_agent(store, &b, "b");

    let ready = create_conversation(store, &a, &agent_a);
    let creating = create_conversation(store, &a, &agent_a);
    let without_room = create_conversation(store, &a, &agent_a);
    let of_other_org = create_conversation(store, &b, &agent_b);

    store
        .matrix()
        .ensure_room(a.id, ready.id, "!pronta:teste", None)
        .expect("sala");
    store
        .matrix()
        .mark_room_ready(a.id, ready.id)
        .expect("pronta");
    store
        .matrix()
        .ensure_room(a.id, creating.id, "!criando:teste", None)
        .expect("sala em criação");
    store
        .matrix()
        .ensure_room(b.id, of_other_org.id, "!outra:teste", None)
        .expect("sala da outra org");
    store
        .matrix()
        .mark_room_ready(b.id, of_other_org.id)
        .expect("pronta da outra org");

    let requested: Vec<ConversationId> =
        vec![ready.id, creating.id, without_room.id, of_other_org.id];
    let before = store.reads_done();
    let rooms = store.matrix().rooms_ready(a.id, &requested).expect("salas");
    assert_eq!(
        store.reads_done() - before,
        1,
        "as salas de várias conversas saem numa leitura só"
    );
    assert_eq!(
        rooms.get(&ready.id).map(String::as_str),
        Some("!pronta:teste"),
        "sala ready aparece"
    );
    assert_eq!(rooms.len(), 1, "só a sala pronta da própria org aparece");
    assert!(
        !rooms.contains_key(&creating.id),
        "sala em creating não aparece"
    );
    assert!(
        !rooms.contains_key(&without_room.id),
        "conversa sem sala não aparece"
    );
    assert!(
        !rooms.contains_key(&of_other_org.id),
        "sala de conversa de outra org não aparece, mesmo com state ready"
    );

    let empty = store.matrix().rooms_ready(a.id, &[]).expect("lista vazia");
    assert!(empty.is_empty());
}
