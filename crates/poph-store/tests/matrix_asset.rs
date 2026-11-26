use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use poph_store::migrations::migrate;
use poph_store::{
    CategoryAsset, Conversa, HARNESS_MIGRATIONS, NewAgent, NewAsset, NewConversation, NewOrg, Org,
    RoomAsset, Store, StoreError, file_version, harness_target_version,
};
use poph_types::{AttachmentId, DomainError};

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
        let folder = std::env::temp_dir().join(format!(
            "poph-matrix-midia-{label}-{}-{n}",
            std::process::id()
        ));
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

fn create_conversation(store: &Store, org: &Org) -> Conversa {
    let agent = store
        .agents()
        .create(
            org.id,
            NewAgent {
                name: "Agente".to_owned(),
                slug: "agente".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente");
    store
        .conversations()
        .create(
            org.id,
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

fn asset(conversation: &Conversa, evento: &str, sent_at: i64) -> NewAsset {
    NewAsset {
        id: AttachmentId::generate(),
        conversation: conversation.id,
        agent: "agt_teste".to_owned(),
        room: "!sala:matrix-dev.local".to_owned(),
        evento: evento.to_owned(),
        mxc: format!("mxc://matrix-dev.local/{evento}"),
        nome: "laudo.pdf".to_owned(),
        mime: "application/pdf".to_owned(),
        category: CategoryAsset::Document,
        size_bytes: 2_048,
        sent_at,
    }
}

fn outbound(assets: &[RoomAsset]) -> Vec<String> {
    assets.iter().map(|m| m.id.to_string()).collect()
}

#[test]
fn new_migration_enters_empty_and_old_database() {
    let new = Scenario::new("migracao-nova");
    assert_eq!(
        file_version(&new.folder.join("harness.db")).expect("versão"),
        Some(harness_target_version())
    );

    let old = Scenario::with_version("migracao-antiga", Some(HARNESS_MIGRATIONS.len() - 1));
    let org = create_org(old.store(), "antiga");
    let conversation = create_conversation(old.store(), &org);
    let one = asset(&conversation, "$e1:matrix-dev.local", 5_000);
    assert!(
        old.store()
            .matrix_asset()
            .register(org.id, &one)
            .expect("grava depois de migrate")
    );
}

#[test]
fn registering_same_asset_twice_does_not_duplicate_nor_move_deadline() {
    let scenario = Scenario::new("idempotente");
    let store = scenario.store();
    let org = create_org(store, "a");
    let conversation = create_conversation(store, &org);

    let first = asset(&conversation, "$e1:matrix-dev.local", 5_000);
    assert!(
        store
            .matrix_asset()
            .register(org.id, &first)
            .expect("grava")
    );

    scenario.clock.define(9_000);
    let mut second = first.clone();
    second.sent_at = 9_000;
    assert!(
        !store
            .matrix_asset()
            .register(org.id, &second)
            .expect("regrava")
    );

    let pending = store
        .matrix_asset()
        .pending(org.id, 10_000, 10_000, 50)
        .expect("lê a fila");
    assert_eq!(pending.len(), 1, "uma linha só: {pending:?}");
    assert_eq!(
        pending[0].sent_at, 5_000,
        "o prazo conta da primeira publicação"
    );
}

#[test]
fn refuses_asset_without_room_or_without_agent() {
    let scenario = Scenario::new("validacao");
    let store = scenario.store();
    let org = create_org(store, "a");
    let conversation = create_conversation(store, &org);
    let completa = asset(&conversation, "$e1:matrix-dev.local", 5_000);

    for (label, adjustment) in [
        (
            "sala",
            Box::new(|m: &mut NewAsset| m.room.clear()) as Box<dyn Fn(&mut NewAsset)>,
        ),
        (
            "evento",
            Box::new(|m: &mut NewAsset| m.evento = "  ".to_owned()),
        ),
        ("agente", Box::new(|m: &mut NewAsset| m.agent.clear())),
        ("mxc", Box::new(|m: &mut NewAsset| m.mxc.clear())),
    ] {
        let mut defeituosa = completa.clone();
        adjustment(&mut defeituosa);
        let error = store
            .matrix_asset()
            .register(org.id, &defeituosa)
            .expect_err("mídia incompleta deve ser recusada");
        match error {
            StoreError::Domain(DomainError::Validation { field: campo, .. }) => {
                assert_eq!(campo, label)
            }
            another => panic!("esperado erro de validação em {label}, veio {another:?}"),
        }
    }
}

#[test]
fn queue_returns_only_won_and_still_pending_step() {
    let scenario = Scenario::new("prazo");
    let store = scenario.store();
    let org = create_org(store, "a");
    let conversation = create_conversation(store, &org);

    let previous = asset(&conversation, "$velha:matrix-dev.local", 1_000);
    let fresh = asset(&conversation, "$nova:matrix-dev.local", 9_000);
    for one in [&previous, &fresh] {
        store
            .matrix_asset()
            .register(org.id, one)
            .expect("grava a mídia");
    }

    let expired = store
        .matrix_asset()
        .pending(org.id, 5_000, 10_000, 50)
        .expect("lê a fila");
    assert_eq!(outbound(&expired), vec![previous.id.to_string()]);

    store
        .matrix_asset()
        .mark_draft(org.id, previous.id, 10_000)
        .expect("redigida");
    assert_eq!(
        store
            .matrix_asset()
            .pending(org.id, 5_000, 10_000, 50)
            .expect("lê a fila")
            .len(),
        1,
        "ainda falta apagar o arquivo"
    );
    store
        .matrix_asset()
        .mark_deleted(org.id, previous.id, 10_000)
        .expect("apagada");
    assert!(
        store
            .matrix_asset()
            .pending(org.id, 5_000, 10_000, 50)
            .expect("lê a fila")
            .is_empty(),
        "mídia completa não volta"
    );
    assert_eq!(
        store.matrix_asset().org_pending(org.id).expect("conta"),
        1,
        "só a nova segue pendente"
    );
}

#[test]
fn org_does_not_see_asset_of_other_org() {
    let scenario = Scenario::new("isolamento");
    let store = scenario.store();
    let minha = create_org(store, "minha");
    let foreign = create_org(store, "alheia");
    let my_conversation = create_conversation(store, &minha);
    let conversation_foreign = create_conversation(store, &foreign);

    let from_mine = asset(&my_conversation, "$a:matrix-dev.local", 1_000);
    let from_foreign = asset(&conversation_foreign, "$b:matrix-dev.local", 1_000);
    store
        .matrix_asset()
        .register(minha.id, &from_mine)
        .expect("grava a minha");
    store
        .matrix_asset()
        .register(foreign.id, &from_foreign)
        .expect("grava a alheia");

    let expired = store
        .matrix_asset()
        .pending(minha.id, 5_000, 10_000, 50)
        .expect("lê a fila");
    assert_eq!(outbound(&expired), vec![from_mine.id.to_string()]);

    store
        .matrix_asset()
        .mark_deleted(foreign.id, from_mine.id, 10_000)
        .expect("marca na org errada");
    let still_pending = store
        .matrix_asset()
        .pending(minha.id, 5_000, 10_000, 50)
        .expect("lê a fila");
    assert!(
        still_pending[0].still_to_delete(),
        "a org errada não podia mexer nesta linha"
    );
}

#[test]
fn failed_wait_removes_item_from_queue_until_due_time() {
    let scenario = Scenario::new("espera");
    let store = scenario.store();
    let org = create_org(store, "a");
    let conversation = create_conversation(store, &org);
    let one = asset(&conversation, "$e1:matrix-dev.local", 1_000);
    store
        .matrix_asset()
        .register(org.id, &one)
        .expect("grava a mídia");

    let attempts = store
        .matrix_asset()
        .adiar(org.id, one.id, 61_000, "M_UNKNOWN", 1_000)
        .expect("adia");
    assert_eq!(attempts, 1);

    assert!(
        store
            .matrix_asset()
            .pending(org.id, 5_000, 60_000, 50)
            .expect("lê a fila")
            .is_empty(),
        "antes da hora marcada o item não volta"
    );
    let on_time = store
        .matrix_asset()
        .pending(org.id, 5_000, 61_000, 50)
        .expect("lê a fila");
    assert_eq!(on_time.len(), 1);
    assert_eq!(on_time[0].attempts, 1);
    assert_eq!(on_time[0].ultimo_erro.as_deref(), Some("M_UNKNOWN"));

    store
        .matrix_asset()
        .mark_draft(org.id, one.id, 62_000)
        .expect("redigida");
    let later = store
        .matrix_asset()
        .pending(org.id, 5_000, 62_000, 50)
        .expect("lê a fila");
    assert_eq!(later[0].attempts, 0);
    assert_eq!(later[0].next_attempt_at, 0);
    assert_eq!(later[0].ultimo_erro, None);
    assert!(later[0].still_to_delete());
}

#[test]
fn error_code_fits_in_column() {
    let scenario = Scenario::new("codigo");
    let store = scenario.store();
    let org = create_org(store, "a");
    let conversation = create_conversation(store, &org);
    let one = asset(&conversation, "$e1:matrix-dev.local", 1_000);
    store
        .matrix_asset()
        .register(org.id, &one)
        .expect("grava a mídia");

    let enorme = "X".repeat(200);
    store
        .matrix_asset()
        .adiar(org.id, one.id, 0, &enorme, 1_000)
        .expect("adia com código enorme");
    let row = store
        .matrix_asset()
        .pending(org.id, 5_000, 1_000, 50)
        .expect("lê a fila")
        .remove(0);
    assert_eq!(
        row.ultimo_erro.as_deref().map(str::len),
        Some(poph_store::LIMIT_ERROR_ASSET),
        "o código é recortado no limite da coluna"
    );
}

#[test]
fn deleting_org_takes_module_assets_with_it() {
    let scenario = Scenario::new("cascade");
    let store = scenario.store();
    let org = create_org(store, "a");
    let conversation = create_conversation(store, &org);
    let one = asset(&conversation, "$e1:matrix-dev.local", 1_000);
    store
        .matrix_asset()
        .register(org.id, &one)
        .expect("grava a mídia");

    store
        .orgs()
        .delete_forced(org.id)
        .expect("apaga a org com os agentes dela");
    assert_eq!(
        store
            .matrix_asset()
            .pending(org.id, 5_000, 10_000, 50)
            .expect("lê a fila")
            .len(),
        0,
        "as linhas da org foram junto"
    );
}
