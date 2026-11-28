use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poph_matrix::error::CallError;
use poph_matrix::format::{MessageOrigin, Outbound};
use poph_matrix::rooms::{ConversationToRoom, RoomError};
use poph_matrix::users::Ghost;
use poph_matrix::users::GhostIds;
use poph_server::matrix_mirror::{
    AssetSend, DEFINITIVE_ATTEMPTS, MAX_DELAY, MAX_PAGES_PER_CYCLE, MatrixMirror, MatrixTarget,
    PAGE, PublishedAsset, RoomNotice,
};
use poph_store::{
    Direction, NewAgent, NewConversation, NewDeviceMessage, NewMessage, NewOrg, Origin, Role, Store,
};
use poph_types::{AgentId, ConversationId, IntegrationId, MatrixConfig, MessageId, OrgId};
use rusqlite::Connection;

struct Scenario {
    folder: PathBuf,
    store: Option<Arc<Store>>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder =
            std::env::temp_dir().join(format!("poph-espelho-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("cria a pasta do teste");
        let clock = Arc::new(AtomicI64::new(1_000));
        let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
            .expect("abre o banco")
            .with_clock(Arc::new(move || clock.fetch_add(1, Ordering::SeqCst)));
        Self {
            folder,
            store: Some(Arc::new(store)),
        }
    }

    fn store(&self) -> Arc<Store> {
        Arc::clone(self.store.as_ref().expect("store viva"))
    }

    fn connection(&self) -> Connection {
        Connection::open(self.store().harness_path()).expect("abre o banco para conferir")
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.folder);
    }
}

fn config() -> MatrixConfig {
    MatrixConfig {
        enabled: true,
        homeserver_url: "http://127.0.0.1:1".to_owned(),
        server_name: "matrix-dev.local".to_owned(),
        prefix: "ph".to_owned(),
        concurrency: 8,
        interval_ms: 500,
        ..MatrixConfig::default()
    }
}

fn create_org(store: &Store, slug: &str) -> OrgId {
    store
        .orgs()
        .create(NewOrg {
            name: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id
}

fn create_agent(store: &Store, org: OrgId) -> AgentId {
    store
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
        .id
}

fn create_conversation(store: &Store, org: OrgId, agent: AgentId, peer: &str) -> ConversationId {
    store
        .conversations()
        .create(
            org,
            NewConversation {
                agent_id: agent,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: Some(peer.to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa")
        .id
}

fn client_message(store: &Store, org: OrgId, conversation: ConversationId, text: &str) -> String {
    write_message(
        store,
        org,
        conversation,
        Direction::Inbound,
        Role::User,
        Origin::Agente,
        text,
    )
}

fn agent_message(store: &Store, org: OrgId, conversation: ConversationId, text: &str) -> String {
    write_message(
        store,
        org,
        conversation,
        Direction::Outbound,
        Role::Assistant,
        Origin::Agente,
        text,
    )
}

fn write_message(
    store: &Store,
    org: OrgId,
    conversation: ConversationId,
    direction: Direction,
    role: Role,
    origin: Origin,
    text: &str,
) -> String {
    store
        .messages()
        .write(
            org,
            NewMessage {
                conversation_id: conversation,
                direction,
                role,
                origin,
                author_name: None,
                texto: text.to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
            },
        )
        .expect("grava a mensagem")
        .id
        .to_string()
}

fn txn_for(id: &str) -> String {
    format!("poph-msg-{id}")
}

#[derive(Clone, Copy)]
enum Mode {
    Normal,
    Down,
    Venenoso(&'static str),
    FailureOnCall(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Created {
    room: String,
    txn: String,
    body: String,
    sender: String,
}

struct Fake {
    mode: Mutex<Mode>,
    created_events: Mutex<Vec<Created>>,
    tentativas: Mutex<HashMap<String, u32>>,
    room_calls: AtomicUsize,
    send_calls: AtomicUsize,
    device_calls: AtomicUsize,
    members: Mutex<std::collections::HashSet<(String, String)>>,
    device_failure: Mutex<bool>,
}

impl Fake {
    fn new(mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            mode: Mutex::new(mode),
            created_events: Mutex::new(Vec::new()),
            tentativas: Mutex::new(HashMap::new()),
            room_calls: AtomicUsize::new(0),
            send_calls: AtomicUsize::new(0),
            device_calls: AtomicUsize::new(0),
            members: Mutex::new(std::collections::HashSet::new()),
            device_failure: Mutex::new(false),
        })
    }

    fn set_device_failure(&self, failure: bool) {
        *self.device_failure.lock().unwrap() = failure;
    }

    fn created_by(&self, sender: &str) -> Vec<Created> {
        self.created_events()
            .into_iter()
            .filter(|created_event| created_event.sender == sender)
            .collect()
    }

    fn set_mode(&self, mode: Mode) {
        *self.mode.lock().unwrap() = mode;
    }

    fn created_events(&self) -> Vec<Created> {
        self.created_events.lock().unwrap().clone()
    }

    fn room_txns(&self, room: &str) -> Vec<String> {
        self.created_events()
            .into_iter()
            .filter(|created_event| created_event.room == room)
            .map(|created_event| created_event.txn)
            .collect()
    }

    fn attempts_for(&self, txn: &str) -> u32 {
        *self.tentativas.lock().unwrap().get(txn).unwrap_or(&0)
    }
}

fn transient() -> CallError {
    CallError::Response {
        operation: "send",
        status: 503,
        errcode: Some("M_UNKNOWN".to_owned()),
        message: "indisponível".to_owned(),
        retry_after_ms: None,
    }
}

fn refused_error() -> CallError {
    CallError::Response {
        operation: "send",
        status: 403,
        errcode: Some("M_FORBIDDEN".to_owned()),
        message: "recusado".to_owned(),
        retry_after_ms: None,
    }
}

struct Signal(Arc<Fake>);

impl MatrixTarget for Signal {
    fn ensure_room(
        &self,
        dados: &ConversationToRoom,
    ) -> impl std::future::Future<Output = Result<String, RoomError>> + Send {
        let room = format!("!{}:matrix-dev.local", dados.conversation);
        self.0.room_calls.fetch_add(1, Ordering::SeqCst);
        async move { Ok(room) }
    }

    fn ensure_device(
        &self,
        _data: &ConversationToRoom,
        room: &str,
        device: &Ghost,
    ) -> impl std::future::Future<Output = Result<(), RoomError>> + Send {
        let fake = &self.0;
        fake.device_calls.fetch_add(1, Ordering::SeqCst);
        let fail = *fake.device_failure.lock().unwrap();
        if !fail {
            fake.members
                .lock()
                .unwrap()
                .insert((room.to_owned(), device.user_id().to_owned()));
        }
        async move {
            if fail {
                Err(RoomError::Call(transient()))
            } else {
                Ok(())
            }
        }
    }

    fn send(
        &self,
        room: &str,
        send: &Outbound,
    ) -> impl std::future::Future<Output = Result<(), CallError>> + Send {
        let fake = &self.0;
        let n = fake.send_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let mode = *fake.mode.lock().unwrap();
        let room = room.to_owned();
        let txn = send.txn_id.clone();
        let body = send.body.clone();
        let sender = send.sender.user_id().to_owned();
        *fake
            .tentativas
            .lock()
            .unwrap()
            .entry(txn.clone())
            .or_insert(0) += 1;
        let outside_room = send.origin == MessageOrigin::Device
            && !fake
                .members
                .lock()
                .unwrap()
                .contains(&(room.clone(), sender.clone()));
        let resultado = match mode {
            _ if outside_room => Err(refused_error()),
            Mode::Down => Err(transient()),
            Mode::Venenoso(text) if body.contains(text) => Err(refused_error()),
            Mode::FailureOnCall(k) if n == k => {
                *fake.mode.lock().unwrap() = Mode::Normal;
                Err(transient())
            }
            _ => Ok(()),
        };
        let mut created_events = fake.created_events.lock().unwrap();
        if resultado.is_ok()
            && !created_events
                .iter()
                .any(|c| c.room == room && c.txn == txn)
        {
            created_events.push(Created {
                room,
                txn,
                body,
                sender,
            });
        }
        drop(created_events);
        async move { resultado }
    }

    fn send_asset(
        &self,
        room: &str,
        send: &AssetSend,
    ) -> impl std::future::Future<Output = Result<PublishedAsset, CallError>> + Send {
        let fake = &self.0;
        let n = fake.send_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let mode = *fake.mode.lock().unwrap();
        let room = room.to_owned();
        let txn = send.asset.txn_id.clone();
        let body = send.asset.name.clone();
        let sender = send.sender.user_id().to_owned();
        *fake
            .tentativas
            .lock()
            .unwrap()
            .entry(txn.clone())
            .or_insert(0) += 1;
        let resultado = match mode {
            Mode::Down => Err(transient()),
            Mode::Venenoso(text) if body.contains(text) => Err(refused_error()),
            Mode::FailureOnCall(k) if n == k => {
                *fake.mode.lock().unwrap() = Mode::Normal;
                Err(transient())
            }
            _ => Ok(()),
        };
        let mut created_events = fake.created_events.lock().unwrap();
        let new = !created_events
            .iter()
            .any(|c| c.room == room && c.txn == txn);
        if resultado.is_ok() && new {
            created_events.push(Created {
                room: room.clone(),
                txn: txn.clone(),
                body,
                sender,
            });
        }
        drop(created_events);
        let published = PublishedAsset {
            event: format!("$midia-{txn}:matrix-dev.local"),
            mxc: format!("mxc://matrix-dev.local/{txn}"),
        };
        async move { resultado.map(|()| published) }
    }

    fn send_notice(
        &self,
        room: &str,
        notice: &RoomNotice,
    ) -> impl std::future::Future<Output = Result<(), CallError>> + Send {
        let fake = &self.0;
        let room = room.to_owned();
        let txn = notice.txn_id.clone();
        let body = poph_matrix::asset::notice_body(notice.reason);
        let sender = notice.sender.user_id().to_owned();
        let mut created_events = fake.created_events.lock().unwrap();
        if !created_events
            .iter()
            .any(|c| c.room == room && c.txn == txn)
        {
            created_events.push(Created {
                room,
                txn,
                body,
                sender,
            });
        }
        drop(created_events);
        async move { Ok(()) }
    }
}

fn mirror(store: Arc<Store>, destination: Arc<Fake>) -> MatrixMirror<Signal> {
    MatrixMirror::new(store, Signal(destination), &config()).expect("monta o espelho")
}

fn room_for(conversation: ConversationId) -> String {
    format!("!{conversation}:matrix-dev.local")
}

#[tokio::test]
async fn skips_site_and_popbot_but_mirrors_email() {
    let scenario = Scenario::new("adapter-channels");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let agent = create_agent(&store, org);
    store.matrix().enable(org).expect("liga");

    let (site, _) = store
        .site_chat()
        .create(org, agent, "site-thread".to_owned())
        .expect("cria conversa do site");
    let (popbot, _) = store
        .adapter_chat()
        .create_or_resume(
            org,
            agent,
            "popbot".to_owned(),
            "bot-thread".to_owned(),
            false,
        )
        .expect("cria conversa do POP Bot");
    let (email, _) = store
        .adapter_chat()
        .create_or_resume(
            org,
            agent,
            "email".to_owned(),
            "mail-thread".to_owned(),
            true,
        )
        .expect("cria conversa de e-mail");

    client_message(&store, org, site, "Mensagem do site");
    client_message(&store, org, popbot, "Mensagem do POP Bot");
    let email_message = client_message(&store, org, email, "Mensagem de e-mail");

    let destination = Fake::new(Mode::Normal);
    let report = mirror(store, Arc::clone(&destination))
        .cycle()
        .await
        .expect("ciclo");

    assert_eq!(report.sent, 1);
    assert_eq!(destination.room_calls.load(Ordering::SeqCst), 1);
    assert!(destination.room_txns(&room_for(site)).is_empty());
    assert!(destination.room_txns(&room_for(popbot)).is_empty());
    assert_eq!(
        destination.room_txns(&room_for(email)),
        [txn_for(&email_message)]
    );
}

#[tokio::test]
async fn order_inside_room_and_parallel_between_rooms() {
    let scenario = Scenario::new("ordem");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let agent = create_agent(&store, org);
    let first_conversation = create_conversation(&store, org, agent, "+5511111");
    let bruno = create_conversation(&store, org, agent, "+5522222");
    store.matrix().enable(org).expect("liga");

    let mut expected_a = Vec::new();
    let mut expected_b = Vec::new();
    for i in 0..6 {
        if i % 2 == 0 {
            expected_a.push(txn_for(&client_message(
                &store,
                org,
                first_conversation,
                &format!("ana {i}"),
            )));
        } else {
            expected_b.push(txn_for(&client_message(
                &store,
                org,
                bruno,
                &format!("bruno {i}"),
            )));
        }
    }

    let destination = Fake::new(Mode::Normal);
    let report = mirror(Arc::clone(&store), Arc::clone(&destination))
        .cycle()
        .await
        .expect("ciclo");

    assert_eq!(report.sent, 6);
    assert_eq!(
        destination.room_txns(&room_for(first_conversation)),
        expected_a
    );
    assert_eq!(destination.room_txns(&room_for(bruno)), expected_b);
}

#[tokio::test]
async fn mid_failure_skips_nothing_and_duplicates_nothing() {
    let scenario = Scenario::new("queda");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let agent = create_agent(&store, org);
    let first_conversation = create_conversation(&store, org, agent, "+5511111");
    let bruno = create_conversation(&store, org, agent, "+5522222");
    store.matrix().enable(org).expect("liga");

    let mut all_txns = Vec::new();
    for i in 0..6 {
        let conversation = if i % 2 == 0 {
            first_conversation
        } else {
            bruno
        };
        all_txns.push(txn_for(&client_message(
            &store,
            org,
            conversation,
            &format!("msg {i}"),
        )));
    }

    let destination = Fake::new(Mode::FailureOnCall(3));
    let mirror = mirror(Arc::clone(&store), Arc::clone(&destination));

    let first = mirror.cycle().await.expect("ciclo 1");
    assert!(first.pending >= 1, "a falha no meio deixa pendência");
    let cursor = store.matrix().read_cursors(org).expect("cursor").messages;
    assert!(cursor < 6, "cursor não pode andar além da falha: {cursor}");

    mirror.cycle().await.expect("ciclo 2");
    mirror.cycle().await.expect("ciclo 3");

    let created_events: Vec<String> = destination
        .created_events()
        .into_iter()
        .map(|c| c.txn)
        .collect();
    assert_eq!(created_events.len(), 6, "todas entregues uma vez");
    for txn in &all_txns {
        assert!(created_events.contains(txn), "nenhuma linha pulada: {txn}");
    }
    let unique: std::collections::HashSet<_> = created_events.iter().collect();
    assert_eq!(unique.len(), 6, "nenhum evento duplicado");
    let repeated = all_txns
        .iter()
        .filter(|txn| destination.attempts_for(txn) >= 2)
        .count();
    assert!(
        repeated >= 1,
        "a mensagem da queda foi reenviada com o mesmo txnId"
    );
}

#[tokio::test]
async fn poison_message_does_not_block_queue() {
    let scenario = Scenario::new("venenosa");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "+5511111");
    store.matrix().enable(org).expect("liga");

    let before = txn_for(&client_message(&store, org, conversation, "primeira"));
    let poison = txn_for(&client_message(&store, org, conversation, "VENENO aqui"));
    let after = txn_for(&client_message(&store, org, conversation, "terceira"));
    let last = txn_for(&client_message(&store, org, conversation, "quarta"));

    let destination = Fake::new(Mode::Venenoso("VENENO"));
    let mirror = mirror(Arc::clone(&store), Arc::clone(&destination));

    for _ in 0..(DEFINITIVE_ATTEMPTS - 1) {
        mirror.cycle().await.expect("ciclo");
    }
    assert!(
        !destination
            .room_txns(&room_for(conversation))
            .contains(&after)
    );

    mirror.cycle().await.expect("ciclo limite");
    mirror.cycle().await.expect("ciclo seguinte");

    let sent = destination.room_txns(&room_for(conversation));
    assert!(sent.contains(&before));
    assert!(sent.contains(&after));
    assert!(sent.contains(&last));
    assert!(!sent.contains(&poison));

    let failures = store.matrix().list_failures(org, 10).expect("falhas");
    assert_eq!(failures.len(), 1, "a venenosa foi isolada uma vez");
    assert_eq!(failures[0].motivo, "M_FORBIDDEN");
    assert_eq!(failures[0].attempts, DEFINITIVE_ATTEMPTS);
}

#[tokio::test]
async fn offline_synapse_accumulates_without_advancing_then_drains() {
    let scenario = Scenario::new("fora");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "+5511111");
    store.matrix().enable(org).expect("liga");

    let mut all_txns = Vec::new();
    for i in 0..5 {
        all_txns.push(txn_for(&client_message(
            &store,
            org,
            conversation,
            &format!("m{i}"),
        )));
    }

    let destination = Fake::new(Mode::Down);
    let mirror = mirror(Arc::clone(&store), Arc::clone(&destination));
    for _ in 0..3 {
        let report = mirror.cycle().await.expect("ciclo");
        assert_eq!(report.sent, 0);
        assert!(report.pending >= 1);
    }
    assert_eq!(
        store.matrix().read_cursors(org).expect("cursor").messages,
        0
    );
    assert!(
        mirror.delay_after(3) > mirror.delay_after(1),
        "o atraso cresce"
    );

    destination.set_mode(Mode::Normal);
    mirror.cycle().await.expect("ciclo de volta");
    let created_events: Vec<String> = destination
        .created_events()
        .into_iter()
        .map(|c| c.txn)
        .collect();
    assert_eq!(created_events, all_txns, "tudo entregue, na ordem, uma vez");
}

#[tokio::test]
async fn disabled_org_makes_no_call() {
    let scenario = Scenario::new("desligada");
    let store = scenario.store();
    let enabled_org = create_org(&store, "ligada");
    let disabled_org = create_org(&store, "desligada");
    let agent_enabled = create_agent(&store, enabled_org);
    let agent_disabled = create_agent(&store, disabled_org);
    let conv_l = create_conversation(&store, enabled_org, agent_enabled, "+5511111");
    let conv_d = create_conversation(&store, disabled_org, agent_disabled, "+5522222");
    store.matrix().enable(enabled_org).expect("liga");

    client_message(&store, enabled_org, conv_l, "da ligada");
    client_message(&store, disabled_org, conv_d, "da desligada");
    client_message(&store, disabled_org, conv_d, "de novo na desligada");

    let destination = Fake::new(Mode::Normal);
    let report = mirror(Arc::clone(&store), Arc::clone(&destination))
        .cycle()
        .await
        .expect("ciclo");

    assert_eq!(report.sent, 1);
    assert_eq!(report.from_disabled_org, 2);
    assert_eq!(destination.room_calls.load(Ordering::SeqCst), 1);
    assert!(
        destination
            .created_events()
            .iter()
            .all(|c| c.room == room_for(conv_l))
    );
}

#[tokio::test]
async fn message_before_cutoff_is_not_mirrored() {
    let scenario = Scenario::new("corte");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "+5511111");

    client_message(&store, org, conversation, "antes de ligar");
    store.matrix().enable(org).expect("liga");
    let after = txn_for(&client_message(
        &store,
        org,
        conversation,
        "depois de ligar",
    ));

    let destination = Fake::new(Mode::Normal);
    let report = mirror(Arc::clone(&store), Arc::clone(&destination))
        .cycle()
        .await
        .expect("ciclo");

    assert_eq!(report.before_cutoff, 1);
    assert_eq!(destination.room_txns(&room_for(conversation)), vec![after]);
}

#[tokio::test]
async fn human_response_and_tool_become_no_event() {
    let scenario = Scenario::new("nao-espelha");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "+5511111");
    store.matrix().enable(org).expect("liga");

    write_message(
        &store,
        org,
        conversation,
        Direction::Outbound,
        Role::Assistant,
        Origin::Human,
        "vindo do Element",
    );
    write_message(
        &store,
        org,
        conversation,
        Direction::Outbound,
        Role::System,
        Origin::Agente,
        "instrução",
    );
    client_message(&store, org, conversation, "   ");
    let normal = txn_for(&agent_message(
        &store,
        org,
        conversation,
        "resposta do agente",
    ));

    let destination = Fake::new(Mode::Normal);
    let report = mirror(Arc::clone(&store), Arc::clone(&destination))
        .cycle()
        .await
        .expect("ciclo");

    assert_eq!(report.sent, 1);
    assert_eq!(destination.room_txns(&room_for(conversation)), vec![normal]);
}

#[tokio::test]
async fn reused_rowid_does_not_lose_new_message() {
    let scenario = Scenario::new("rowid");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "+5511111");
    store.matrix().enable(org).expect("liga");

    client_message(&store, org, conversation, "um");
    client_message(&store, org, conversation, "dois");
    let last = client_message(&store, org, conversation, "tres");

    let destination = Fake::new(Mode::Normal);
    let mirror = mirror(Arc::clone(&store), Arc::clone(&destination));
    mirror.cycle().await.expect("ciclo 1");
    assert_eq!(destination.created_events().len(), 3);

    let old_rowid: i64 = scenario
        .connection()
        .query_row(
            "SELECT rowid FROM messages WHERE id = ?1",
            [last.as_str()],
            |row| row.get(0),
        )
        .expect("rowid");
    scenario
        .connection()
        .execute("DELETE FROM messages WHERE id = ?1", [last.as_str()])
        .expect("apaga");
    let fresh = client_message(&store, org, conversation, "quatro, com rowid antigo");
    let new_rowid: i64 = scenario
        .connection()
        .query_row(
            "SELECT rowid FROM messages WHERE id = ?1",
            [fresh.as_str()],
            |row| row.get(0),
        )
        .expect("rowid novo");
    assert_eq!(new_rowid, old_rowid, "o rowid foi reaproveitado");

    mirror.cycle().await.expect("ciclo 2");
    assert!(
        destination
            .room_txns(&room_for(conversation))
            .contains(&txn_for(&fresh)),
        "a mensagem com rowid reaproveitado sai"
    );
}

#[tokio::test]
async fn cursor_ahead_of_max_rowid_returns_to_safe_point() {
    let scenario = Scenario::new("recuo");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "+5511111");
    store.matrix().enable(org).expect("liga");
    client_message(&store, org, conversation, "um");
    client_message(&store, org, conversation, "dois");

    store
        .matrix()
        .advance_messages_with_id(org, 99, "00000000-0000-7000-8000-000000000000")
        .expect("cursor");

    let destination = Fake::new(Mode::Normal);
    let mirror = mirror(Arc::clone(&store), Arc::clone(&destination));
    let report = mirror.cycle().await.expect("ciclo");
    assert_eq!(report.rewinds, 1);
    assert!(store.matrix().read_cursors(org).expect("cursor").messages <= 2);

    let fresh = txn_for(&client_message(&store, org, conversation, "tres"));
    mirror.cycle().await.expect("ciclo 2");
    assert!(
        destination
            .room_txns(&room_for(conversation))
            .contains(&fresh)
    );
}

#[tokio::test]
async fn cycle_memory_is_limited_to_one_page() {
    let scenario = Scenario::new("memoria");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "+5511111");
    store.matrix().enable(org).expect("liga");

    let total = PAGE as usize * 3 + 17;
    for i in 0..total {
        client_message(&store, org, conversation, &format!("m{i}"));
    }

    let destination = Fake::new(Mode::Normal);
    let report = mirror(Arc::clone(&store), Arc::clone(&destination))
        .cycle()
        .await
        .expect("ciclo");
    assert!(
        report.largest_page <= PAGE,
        "nunca mais que uma página em memória"
    );
    assert_eq!(
        report.sent as usize,
        total.min(PAGE as usize * MAX_PAGES_PER_CYCLE as usize)
    );
}

#[tokio::test]
async fn delay_grows_with_failures_and_stops_at_cap() {
    let scenario = Scenario::new("atraso");
    let mirror = mirror(scenario.store(), Fake::new(Mode::Normal));
    assert_eq!(mirror.delay_after(0), Duration::from_millis(500));
    assert_eq!(mirror.delay_after(1), Duration::from_millis(1000));
    assert_eq!(mirror.delay_after(2), Duration::from_millis(2000));
    assert_eq!(mirror.delay_after(40), MAX_DELAY);
}

#[tokio::test]
async fn stop_ends_loop() {
    let scenario = Scenario::new("parada");
    let store = scenario.store();
    let mirror = Arc::new(mirror(Arc::clone(&store), Fake::new(Mode::Normal)));
    let task = tokio::spawn(Arc::clone(&mirror).run());
    tokio::time::sleep(Duration::from_millis(50)).await;
    mirror.request_stop();
    let resultado = tokio::time::timeout(Duration::from_secs(5), task).await;
    assert!(resultado.is_ok(), "o laço sai depois do pedido de parada");
}

#[tokio::test]
#[ignore = "grava 50 mil mensagens; rodar com --ignored"]
async fn memory_with_50_thousand_pending() {
    let scenario = Scenario::new("cinquenta-mil");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "+5511111");
    store.matrix().enable(org).expect("liga");
    let started_at = std::time::Instant::now();
    for i in 0..50_000 {
        client_message(&store, org, conversation, &format!("m{i}"));
    }
    eprintln!("gravação de 50 mil: {:?}", started_at.elapsed());

    let destination = Fake::new(Mode::Down);
    let started_at = std::time::Instant::now();
    let report = mirror(Arc::clone(&store), Arc::clone(&destination))
        .cycle()
        .await
        .expect("ciclo");
    eprintln!(
        "ciclo com 50 mil pendentes: {:?}, {report:?}",
        started_at.elapsed()
    );
    assert_eq!(report.sent, 0);
    assert!(report.largest_page <= PAGE);
    assert_eq!(report.pages, 1, "para na primeira página com falha");
    assert_eq!(
        store.matrix().read_cursors(org).expect("cursor").messages,
        0
    );
}

const DEVICE_PEER: &str = "5511999990000";

fn org_with_device(store: &Store) -> (OrgId, AgentId, IntegrationId) {
    let org = create_org(store, "ap");
    let agent = create_agent(store, org);
    let integracao: IntegrationId = store
        .create_integration(org, agent, "whatsapp".to_owned(), "active".to_owned())
        .expect("cria a integração")
        .id
        .parse()
        .expect("id da integração");
    store.matrix().enable(org).expect("liga");
    (org, agent, integracao)
}

fn receive_device_message(
    store: &Store,
    org: OrgId,
    agent: AgentId,
    integracao: IntegrationId,
    id_externo: &str,
    text: &str,
) -> (ConversationId, MessageId) {
    let received = store
        .inbox()
        .receive_from_device(
            org,
            NewDeviceMessage {
                integration_id: integracao,
                agent_id: agent,
                canal: "whatsapp".to_owned(),
                peer: DEVICE_PEER.to_owned(),
                external_id: id_externo.to_owned(),
                texto: text.to_owned(),
            },
        )
        .expect("recebe a mensagem do aparelho");
    (received.conversation, received.mensagem.id)
}

fn device_ghost(integracao: IntegrationId) -> String {
    GhostIds::new("ph", "matrix-dev.local")
        .expect("ids")
        .device(&integracao.to_string())
        .expect("id do aparelho")
        .user_id()
        .to_owned()
}

fn count_outbox(scenario: &Scenario) -> i64 {
    scenario
        .connection()
        .query_row("SELECT COUNT(*) FROM outbox", [], |row| row.get(0))
        .expect("conta a fila de saída")
}

#[tokio::test]
async fn device_message_enters_room_through_device_user() {
    let scenario = Scenario::new("aparelho-sala");
    let store = scenario.store();
    let (org, agent, integracao) = org_with_device(&store);
    let (conversation, message) = receive_device_message(
        &store,
        org,
        agent,
        integracao,
        "AP-1",
        "Já enviei, obrigado",
    );
    store
        .conversations()
        .set_customer_name(org, conversation, "Maria")
        .expect("grava o nome do cliente");
    client_message(&store, org, conversation, "oi, tudo bem?");

    let destination = Fake::new(Mode::Normal);
    let outbox_before = count_outbox(&scenario);
    let report = mirror(Arc::clone(&store), Arc::clone(&destination))
        .cycle()
        .await
        .expect("ciclo");

    assert_eq!(report.sent, 2, "a do cliente e a do aparelho");
    let of_device = destination.created_by(&device_ghost(integracao));
    assert_eq!(of_device.len(), 1, "o usuário do aparelho fala uma vez");
    assert_eq!(of_device[0].txn, txn_for(&message.to_string()));
    assert_eq!(of_device[0].body, "Já enviei, obrigado");
    assert_eq!(of_device[0].room, room_for(conversation));
    assert_eq!(
        destination
            .created_by("@ph_wa_5511999990000:matrix-dev.local")
            .len(),
        1,
        "a fala do cliente continua saindo pelo usuário do cliente"
    );
    assert_eq!(
        destination.device_calls.load(Ordering::SeqCst),
        1,
        "o aparelho entrou na sala uma vez"
    );
    assert_eq!(
        count_outbox(&scenario),
        outbox_before,
        "o espelho não cria linha na fila de saída: a mensagem já saiu pelo celular"
    );
    let name = store
        .conversations()
        .get(org, conversation)
        .expect("lê a conversa")
        .customer_name;
    assert_eq!(name.as_deref(), Some("Maria"), "o nome do cliente não muda");
}

#[tokio::test]
async fn device_enters_once_per_pass_and_repetition_does_not_duplicate() {
    let scenario = Scenario::new("aparelho-repeticao");
    let store = scenario.store();
    let (org, agent, integracao) = org_with_device(&store);
    let (conversation, first_message) =
        receive_device_message(&store, org, agent, integracao, "AP-1", "primeira");
    let (_, second_message) =
        receive_device_message(&store, org, agent, integracao, "AP-2", "segunda");
    assert_eq!(
        store
            .conversations()
            .get(org, conversation)
            .expect("conversa")
            .id,
        conversation,
        "as duas mensagens caem na mesma conversa"
    );

    let destination = Fake::new(Mode::Normal);
    let mirror = mirror(Arc::clone(&store), Arc::clone(&destination));
    let first_cycle = mirror.cycle().await.expect("ciclo 1");
    assert_eq!(first_cycle.sent, 2);
    assert_eq!(
        destination.device_calls.load(Ordering::SeqCst),
        1,
        "um grupo da sala, um pedido de entrada"
    );

    store
        .matrix()
        .rewind_cursor_messages(org, 0, "")
        .expect("recua o cursor");
    mirror.cycle().await.expect("ciclo 2");

    let of_device = destination.created_by(&device_ghost(integracao));
    assert_eq!(of_device.len(), 2, "cada mensagem aparece uma vez");
    for id in [first_message, second_message] {
        assert!(
            destination.attempts_for(&txn_for(&id.to_string())) >= 2,
            "a mensagem foi reenviada com o mesmo txnId"
        );
    }
}

#[tokio::test]
async fn device_outside_room_defers_message_without_losing_it() {
    let scenario = Scenario::new("aparelho-adia");
    let store = scenario.store();
    let (org, agent, integracao) = org_with_device(&store);
    let (_, message) =
        receive_device_message(&store, org, agent, integracao, "AP-1", "oi do celular");

    let destination = Fake::new(Mode::Normal);
    destination.set_device_failure(true);
    let mirror = mirror(Arc::clone(&store), Arc::clone(&destination));
    let first = mirror.cycle().await.expect("ciclo 1");
    assert_eq!(first.sent, 0, "sem entrar na sala, nada é enviado");
    assert!(first.pending >= 1, "a mensagem fica pendente");
    assert!(destination.created_events().is_empty());

    destination.set_device_failure(false);
    let second = mirror.cycle().await.expect("ciclo 2");
    assert_eq!(second.sent, 1);
    let of_device = destination.created_by(&device_ghost(integracao));
    assert_eq!(of_device.len(), 1);
    assert_eq!(of_device[0].txn, txn_for(&message.to_string()));
}
