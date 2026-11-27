use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use poph_matrix::error::CallError;
use poph_matrix::rooms::RoomMetadata;
use poph_matrix::spaces::{Position, RoomForSpace, SpaceError};
use poph_server::matrix_state::{
    ConversationChanged, ConversationState, ConversationStates, OrgState, PAGE, RoomState,
    StateError, StatePort, StateStorePort, StateTarget, StoredCursor,
};
use poph_store::{ConversationStatus, NewAgent, NewConversation, NewDepartment, NewOrg, Store};
use poph_types::{AgentId, ConversationId, DepartmentId, MatrixConfig, OrgId};
use serde_json::Value;

struct TestClock {
    now: AtomicI64,
    congelado: std::sync::atomic::AtomicBool,
}

impl TestClock {
    fn now(&self) -> i64 {
        if self.congelado.load(Ordering::SeqCst) {
            return self.now.load(Ordering::SeqCst);
        }
        self.now.fetch_add(1, Ordering::SeqCst)
    }
}

struct Scenario {
    folder: PathBuf,
    store: Option<Arc<Store>>,
    clock: Arc<TestClock>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder =
            std::env::temp_dir().join(format!("poph-estado-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("cria a pasta do teste");
        let clock = Arc::new(TestClock {
            now: AtomicI64::new(1_000_000),
            congelado: std::sync::atomic::AtomicBool::new(false),
        });
        let pulso = Arc::clone(&clock);
        let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
            .expect("abre o banco")
            .with_clock(Arc::new(move || pulso.now()));
        Self {
            folder,
            store: Some(Arc::new(store)),
            clock,
        }
    }

    fn store(&self) -> Arc<Store> {
        Arc::clone(self.store.as_ref().expect("store viva"))
    }

    fn congelar(&self, em: i64) {
        self.clock.now.store(em, Ordering::SeqCst);
        self.clock.congelado.store(true, Ordering::SeqCst);
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
        concurrency: 4,
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

fn create_department(store: &Store, org: OrgId, name: &str) -> DepartmentId {
    store
        .departments()
        .create(
            org,
            NewDepartment {
                name: name.to_owned(),
                description: String::new(),
            },
        )
        .expect("cria o departamento")
        .id
}

fn create_agent(
    store: &Store,
    org: OrgId,
    slug: &str,
    default_department: Option<DepartmentId>,
) -> AgentId {
    store
        .agents()
        .create(
            org,
            NewAgent {
                name: "Bia".to_owned(),
                slug: slug.to_owned(),
                default_department,
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

fn room_ready(store: &Store, org: OrgId, conversation: ConversationId) {
    store
        .matrix()
        .ensure_room(org, conversation, &room_for(conversation), None)
        .expect("grava a sala");
    store
        .matrix()
        .mark_room_ready(org, conversation)
        .expect("marca pronta");
}

fn room_being_created(store: &Store, org: OrgId, conversation: ConversationId) {
    store
        .matrix()
        .ensure_room(org, conversation, &room_for(conversation), None)
        .expect("grava a sala");
}

fn room_for(conversation: ConversationId) -> String {
    format!("!{conversation}:matrix-dev.local")
}

fn set_status(store: &Store, org: OrgId, conversation: ConversationId, status: ConversationStatus) {
    let current = store
        .conversations()
        .get(org, conversation)
        .expect("conversa existe");
    match status {
        ConversationStatus::Open => {
            store
                .conversations()
                .reopen_audited(org, conversation, current.version, "teste".to_owned())
                .expect("reabre");
        }
        ConversationStatus::Closed => {
            store
                .conversations()
                .close_audited(org, conversation, current.version, "teste".to_owned())
                .expect("fecha");
        }
        ConversationStatus::Transferred => {
            store
                .conversations()
                .transfer(org, conversation, None, None, "{}".to_owned())
                .expect("transfere");
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Notice {
    room: String,
    txn: String,
    agent: AgentId,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WrittenMetadata {
    room: String,
    body: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Renomeacao {
    room: String,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenamedGhost {
    client: String,
    name: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    FailureOnFirst,
    FailureOnWrite(usize),
    Refusal,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Espaco {
    AlreadyInPlace,
    Moveu,
}

struct Fake {
    notices: Mutex<Vec<Notice>>,
    metadatas: Mutex<Vec<WrittenMetadata>>,
    renames: Mutex<Vec<Renomeacao>>,
    ghosts: Mutex<Vec<RenamedGhost>>,
    positions: Mutex<Vec<RoomForSpace>>,
    mode: Mutex<Mode>,
    space: Mutex<Espaco>,
    writes: AtomicUsize,
}

impl Fake {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            notices: Mutex::new(Vec::new()),
            metadatas: Mutex::new(Vec::new()),
            renames: Mutex::new(Vec::new()),
            ghosts: Mutex::new(Vec::new()),
            positions: Mutex::new(Vec::new()),
            mode: Mutex::new(Mode::Normal),
            space: Mutex::new(Espaco::AlreadyInPlace),
            writes: AtomicUsize::new(0),
        })
    }

    fn set_mode(&self, mode: Mode) {
        *self.mode.lock().unwrap() = mode;
    }

    fn set_space(&self, space: Espaco) {
        *self.space.lock().unwrap() = space;
    }

    fn notices(&self) -> Vec<Notice> {
        self.notices.lock().unwrap().clone()
    }

    fn metadatas(&self) -> Vec<WrittenMetadata> {
        self.metadatas.lock().unwrap().clone()
    }

    fn renames(&self) -> Vec<Renomeacao> {
        self.renames.lock().unwrap().clone()
    }

    fn ghosts(&self) -> Vec<RenamedGhost> {
        self.ghosts.lock().unwrap().clone()
    }

    fn positions(&self) -> Vec<RoomForSpace> {
        self.positions.lock().unwrap().clone()
    }

    fn writes(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }

    fn fail(&self) -> Option<CallError> {
        let mode = *self.mode.lock().unwrap();
        let write_number = self.writes.fetch_add(1, Ordering::SeqCst) + 1;
        match mode {
            Mode::Normal => None,
            Mode::FailureOnWrite(target) if write_number != target => None,
            Mode::FailureOnWrite(_) | Mode::FailureOnFirst => {
                *self.mode.lock().unwrap() = Mode::Normal;
                Some(CallError::Response {
                    operation: "send",
                    status: 503,
                    errcode: Some("M_UNKNOWN".to_owned()),
                    message: "indisponível".to_owned(),
                    retry_after_ms: None,
                })
            }
            Mode::Refusal => Some(CallError::Response {
                operation: "send",
                status: 403,
                errcode: Some("M_FORBIDDEN".to_owned()),
                message: "recusado".to_owned(),
                retry_after_ms: None,
            }),
        }
    }
}

struct Signal(Arc<Fake>);

impl StateTarget for Signal {
    async fn notify(
        &self,
        room: &str,
        agent: AgentId,
        txn: &str,
        text: &str,
    ) -> Result<(), CallError> {
        if let Some(error) = self.0.fail() {
            return Err(error);
        }
        self.0.notices.lock().unwrap().push(Notice {
            room: room.to_owned(),
            txn: txn.to_owned(),
            agent,
            text: text.to_owned(),
        });
        Ok(())
    }

    async fn write_metadata(
        &self,
        room: &str,
        _agent: AgentId,
        metadata: &RoomMetadata,
    ) -> Result<(), CallError> {
        if let Some(error) = self.0.fail() {
            return Err(error);
        }
        let body = serde_json::to_value(metadata).expect("metadata serializa");
        self.0.metadatas.lock().unwrap().push(WrittenMetadata {
            room: room.to_owned(),
            body,
        });
        Ok(())
    }

    async fn rename(&self, room: &str, _agent: AgentId, name: &str) -> Result<(), CallError> {
        if let Some(error) = self.0.fail() {
            return Err(error);
        }
        self.0.renames.lock().unwrap().push(Renomeacao {
            room: room.to_owned(),
            name: name.to_owned(),
        });
        Ok(())
    }

    async fn rename_ghost(&self, client: &str, name: &str) -> Result<(), CallError> {
        if let Some(error) = self.0.fail() {
            return Err(error);
        }
        self.0.ghosts.lock().unwrap().push(RenamedGhost {
            client: client.to_owned(),
            name: name.to_owned(),
        });
        Ok(())
    }

    async fn place(&self, dados: RoomForSpace) -> Result<Position, SpaceError> {
        self.0.positions.lock().unwrap().push(dados.clone());
        match *self.0.space.lock().unwrap() {
            Espaco::AlreadyInPlace => Ok(Position::AlreadyPositioned {
                space: "!espaco:matrix-dev.local".to_owned(),
            }),
            Espaco::Moveu => Ok(Position::Positioned {
                space: "!espaco:matrix-dev.local".to_owned(),
                moved: true,
                guests: 0,
                failed_invitations: 0,
            }),
        }
    }
}

fn state_loop(
    store: Arc<Store>,
    destination: Arc<Fake>,
) -> ConversationStates<Signal, StateStorePort> {
    ConversationStates::new(StateStorePort::fresh(store), Signal(destination), &config())
        .expect("monta o laço")
}

struct World {
    agent: AgentId,
    conversation: ConversationId,
}

fn world(store: &Store, org: OrgId) -> World {
    store.matrix().enable(org).expect("liga o módulo");
    let agent = create_agent(store, org, "bia", None);
    let conversation = create_conversation(store, org, agent, "5511999990000");
    room_ready(store, org, conversation);
    World {
        agent,
        conversation,
    }
}

#[tokio::test]
async fn transfer_notices_and_rewrites_metadata() {
    let scenario = Scenario::new("transferencia");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let world = world(&store, org);
    let destination = Fake::new();
    let state = state_loop(Arc::clone(&store), Arc::clone(&destination));

    set_status(
        &store,
        org,
        world.conversation,
        ConversationStatus::Transferred,
    );
    let report = state.cycle().await.expect("passagem");

    assert_eq!(report.notified, 1, "{report:?}");
    let notices = destination.notices();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].room, room_for(world.conversation));
    assert_eq!(notices[0].agent, world.agent);
    assert!(notices[0].text.contains("Atendimento humano assumiu"));
    let metadatas = destination.metadatas();
    assert_eq!(metadatas.len(), 1);
    assert_eq!(metadatas[0].room, room_for(world.conversation));
    assert_eq!(
        metadatas[0].body["conversation_id"],
        world.conversation.to_string()
    );
    assert_eq!(metadatas[0].body["status"], "active");
    let room = store
        .matrix()
        .room(org, world.conversation)
        .expect("lê a sala")
        .expect("sala existe");
    assert_eq!(room.last_status, Some(ConversationStatus::Transferred));
}

#[tokio::test]
async fn reopening_notices_agent_returned() {
    let scenario = Scenario::new("reabertura");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let world = world(&store, org);
    let destination = Fake::new();
    let state = state_loop(Arc::clone(&store), Arc::clone(&destination));

    set_status(
        &store,
        org,
        world.conversation,
        ConversationStatus::Transferred,
    );
    state.cycle().await.expect("primeira passagem");
    assert_eq!(destination.notices().len(), 1);

    set_status(&store, org, world.conversation, ConversationStatus::Open);
    let report = state.cycle().await.expect("segunda passagem");
    assert_eq!(report.notified, 1, "{report:?}");
    let notices = destination.notices();
    assert_eq!(notices.len(), 2);
    assert!(notices[1].text.contains("retomou o atendimento"));
    assert_ne!(
        notices[0].txn, notices[1].txn,
        "cada transição tem sua chave"
    );
    assert_eq!(destination.metadatas().len(), 2);
}

#[tokio::test]
async fn closing_notices_and_marks_metadata_closed() {
    let scenario = Scenario::new("encerramento");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let world = world(&store, org);
    let destination = Fake::new();
    let state = state_loop(Arc::clone(&store), Arc::clone(&destination));

    set_status(
        &store,
        org,
        world.conversation,
        ConversationStatus::Transferred,
    );
    state.cycle().await.expect("transferência");
    set_status(&store, org, world.conversation, ConversationStatus::Closed);
    let report = state.cycle().await.expect("encerramento");

    assert_eq!(report.notified, 1, "{report:?}");
    let notices = destination.notices();
    assert_eq!(notices.len(), 2);
    assert!(notices[1].text.contains("Conversa encerrada"));
    let metadatas = destination.metadatas();
    assert_eq!(metadatas.len(), 2);
    assert_eq!(metadatas[1].body["status"], "closed");
}

#[tokio::test]
async fn same_situation_writes_nothing() {
    let scenario = Scenario::new("mesma-situacao");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let world = world(&store, org);
    let destination = Fake::new();
    let state = state_loop(Arc::clone(&store), Arc::clone(&destination));

    set_status(
        &store,
        org,
        world.conversation,
        ConversationStatus::Transferred,
    );
    state.cycle().await.expect("primeira passagem");
    let writes = destination.writes();
    assert_eq!(writes, 2, "um aviso e um metadata na transição");

    store
        .conversations()
        .touch(org, world.conversation, 3_000_000)
        .expect("toca a conversa");
    let report = state.cycle().await.expect("segunda passagem");
    assert_eq!(report.checked, 1, "{report:?}");
    assert_eq!(report.without_change, 1, "{report:?}");
    assert_eq!(report.notified, 0, "{report:?}");
    assert_eq!(destination.writes(), writes, "nenhuma escrita nova");
    assert_eq!(destination.notices().len(), 1);
    assert_eq!(destination.metadatas().len(), 1);
}

#[tokio::test]
async fn conversation_of_disabled_org_makes_no_call() {
    let scenario = Scenario::new("org-desligada");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let agent = create_agent(&store, org, "bia", None);
    let conversation = create_conversation(&store, org, agent, "5511999990000");
    room_ready(&store, org, conversation);
    set_status(&store, org, conversation, ConversationStatus::Transferred);
    let destination = Fake::new();
    let state = state_loop(Arc::clone(&store), Arc::clone(&destination));

    let report = state.cycle().await.expect("passagem");
    assert_eq!(report.checked, 0, "{report:?}");
    assert_eq!(destination.writes(), 0);
}

#[tokio::test]
async fn conversation_without_room_does_not_notice_and_cursor_moves() {
    let scenario = Scenario::new("sem-sala");
    let store = scenario.store();
    let org = create_org(&store, "a");
    store.matrix().enable(org).expect("liga");
    let agent = create_agent(&store, org, "bia", None);
    let conversation = create_conversation(&store, org, agent, "5511999990000");
    set_status(&store, org, conversation, ConversationStatus::Transferred);
    let destination = Fake::new();
    let state = state_loop(Arc::clone(&store), Arc::clone(&destination));

    let report = state.cycle().await.expect("passagem");
    assert_eq!(report.without_room, 1, "{report:?}");
    assert_eq!(destination.writes(), 0);
    let cursors = store.matrix().read_cursors(org).expect("cursores");
    assert!(cursors.conversations.updated_at > 0);
}

#[tokio::test]
async fn room_being_created_waits_for_next_pass() {
    let scenario = Scenario::new("em-criacao");
    let store = scenario.store();
    let org = create_org(&store, "a");
    store.matrix().enable(org).expect("liga");
    let agent = create_agent(&store, org, "bia", None);
    let conversation = create_conversation(&store, org, agent, "5511999990000");
    room_being_created(&store, org, conversation);
    set_status(&store, org, conversation, ConversationStatus::Transferred);
    let destination = Fake::new();
    let state = state_loop(Arc::clone(&store), Arc::clone(&destination));

    let report = state.cycle().await.expect("passagem");
    assert_eq!(report.deferred, 1, "{report:?}");
    assert_eq!(
        destination.writes(),
        0,
        "nada no Synapse antes da sala pronta"
    );
    let cursors = store.matrix().read_cursors(org).expect("cursores");
    assert_eq!(cursors.conversations.updated_at, 0, "cursor não andou");

    store
        .matrix()
        .mark_room_ready(org, conversation)
        .expect("marca pronta");
    let report = state.cycle().await.expect("segunda passagem");
    assert_eq!(report.notified, 1, "{report:?}");
    assert_eq!(destination.notices().len(), 1);
}

#[tokio::test]
async fn transient_failure_does_not_advance_cursor_and_next_pass_applies() {
    let scenario = Scenario::new("falha-transitoria");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let world = world(&store, org);
    let destination = Fake::new();
    destination.set_mode(Mode::FailureOnFirst);
    let state = state_loop(Arc::clone(&store), Arc::clone(&destination));

    set_status(
        &store,
        org,
        world.conversation,
        ConversationStatus::Transferred,
    );
    let report = state.cycle().await.expect("passagem com falha");
    assert_eq!(report.deferred, 1, "{report:?}");
    assert_eq!(destination.notices().len(), 0);
    let cursors = store.matrix().read_cursors(org).expect("cursores");
    assert_eq!(
        cursors.conversations.updated_at, 0,
        "o cursor não pode consumir a conversa em silêncio"
    );
    let room = store
        .matrix()
        .room(org, world.conversation)
        .expect("lê")
        .expect("existe");
    assert_eq!(room.last_status, None, "nada espelhado");

    let report = state.cycle().await.expect("passagem seguinte");
    assert_eq!(report.notified, 1, "{report:?}");
    let notices = destination.notices();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].txn.starts_with("ph_est_"));
}

#[tokio::test]
async fn definitive_error_skips_conversation_and_does_not_block_queue() {
    let scenario = Scenario::new("definitivo");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let world = world(&store, org);
    let destination = Fake::new();
    destination.set_mode(Mode::Refusal);
    let state = state_loop(Arc::clone(&store), Arc::clone(&destination));

    set_status(
        &store,
        org,
        world.conversation,
        ConversationStatus::Transferred,
    );
    let report = state.cycle().await.expect("passagem");
    assert_eq!(report.skipped, 1, "{report:?}");
    assert_eq!(report.deferred, 0);
    assert!(destination.notices().is_empty(), "nada foi escrito na sala");
    let cursors = store.matrix().read_cursors(org).expect("cursores");
    assert!(
        cursors.conversations.updated_at > 0,
        "recusa definitiva não pode travar o cursor"
    );
    destination.set_mode(Mode::Normal);
    set_status(&store, org, world.conversation, ConversationStatus::Closed);
    let report = state.cycle().await.expect("segunda passagem");
    assert_eq!(report.notified, 1, "{report:?}");
    assert_eq!(destination.notices().len(), 1);
    assert!(destination.notices()[0].text.contains("Conversa encerrada"));
}

#[tokio::test]
async fn two_conversations_in_same_millisecond_are_not_lost() {
    let scenario = Scenario::new("mesmo-instante");
    let store = scenario.store();
    let org = create_org(&store, "a");
    store.matrix().enable(org).expect("liga");
    let agent = create_agent(&store, org, "bia", None);
    let first_conversation = create_conversation(&store, org, agent, "5511999990001");
    let second_conversation = create_conversation(&store, org, agent, "5511999990002");
    room_ready(&store, org, first_conversation);
    room_ready(&store, org, second_conversation);

    scenario.congelar(2_000_000);
    set_status(
        &store,
        org,
        first_conversation,
        ConversationStatus::Transferred,
    );
    set_status(
        &store,
        org,
        second_conversation,
        ConversationStatus::Transferred,
    );
    let updated_values: Vec<i64> = [first_conversation, second_conversation]
        .iter()
        .map(|conversation| {
            store
                .conversations()
                .get(org, *conversation)
                .expect("existe")
                .updated_at
        })
        .collect();
    assert_eq!(
        updated_values[0], updated_values[1],
        "mesmo instante de propósito"
    );

    let destination = Fake::new();
    let state = state_loop(Arc::clone(&store), Arc::clone(&destination));
    let report = state.cycle().await.expect("passagem");
    assert_eq!(report.notified, 2, "{report:?}");
    assert_eq!(
        destination.notices().len(),
        2,
        "nenhuma conversa ficou de fora"
    );

    let report = state.cycle().await.expect("repetição");
    assert_eq!(report.notified, 0, "nada novo na repetição: {report:?}");
}

#[tokio::test]
async fn pages_cover_all_changed_conversations() {
    let scenario = Scenario::new("paginas");
    let store = scenario.store();
    let org = create_org(&store, "a");
    store.matrix().enable(org).expect("liga");
    let agent = create_agent(&store, org, "bia", None);
    let mut conversas = Vec::new();
    for n in 0..5 {
        let conversation = create_conversation(&store, org, agent, &format!("55119999900{n:02}"));
        room_ready(&store, org, conversation);
        set_status(&store, org, conversation, ConversationStatus::Transferred);
        conversas.push(conversation);
    }
    let destination = Fake::new();
    let state = ConversationStates::new(
        StateStorePort::fresh(Arc::clone(&store)),
        Signal(Arc::clone(&destination)),
        &config(),
    )
    .expect("monta")
    .with_page(2);

    let report = state.cycle().await.expect("passagem");
    assert_eq!(report.notified, 5, "{report:?}");
    assert_eq!(destination.notices().len(), 5);
    assert!(report.pages >= 3, "{report:?}");
    const { assert!(PAGE > 2) };
}

#[tokio::test]
async fn department_change_moves_room_without_duplicate_notice() {
    let scenario = Scenario::new("departamento");
    let store = scenario.store();
    let org = create_org(&store, "a");
    store.matrix().enable(org).expect("liga");
    let support_department = create_department(&store, org, "Suporte");
    let commercial = create_department(&store, org, "Comercial");
    let agent = create_agent(&store, org, "bia", Some(support_department));
    let conversation = create_conversation(&store, org, agent, "5511999990000");
    room_ready(&store, org, conversation);
    store
        .matrix()
        .set_space_of_room(org, conversation, Some("!espaco-suporte:matrix-dev.local"))
        .expect("grava o espaço");

    let destination = Fake::new();
    destination.set_space(Espaco::Moveu);
    let state = state_loop(Arc::clone(&store), Arc::clone(&destination));

    store
        .conversations()
        .transfer(org, conversation, Some(commercial), None, "{}".to_owned())
        .expect("transfere");
    let report = state.cycle().await.expect("passagem");
    assert_eq!(report.notified, 1, "{report:?}");
    assert_eq!(report.positioned, 1, "{report:?}");
    assert_eq!(destination.notices().len(), 1, "um aviso só");
    let positions = destination.positions();
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].conversation, conversation);
    assert_eq!(
        positions[0]
            .department
            .as_ref()
            .map(|department| department.id),
        Some(commercial)
    );
    let metadatas = destination.metadatas();
    assert_eq!(metadatas.len(), 1);
    assert_eq!(metadatas[0].body["department_id"], commercial.to_string());

    let report = state.cycle().await.expect("repetição");
    assert_eq!(report.notified, 0, "{report:?}");
    assert_eq!(report.positioned, 0, "{report:?}");
    assert_eq!(destination.notices().len(), 1);
}

#[tokio::test]
async fn conversation_without_department_does_not_touch_space() {
    let scenario = Scenario::new("sem-departamento");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let world = world(&store, org);
    let destination = Fake::new();
    let state = state_loop(Arc::clone(&store), Arc::clone(&destination));

    set_status(
        &store,
        org,
        world.conversation,
        ConversationStatus::Transferred,
    );
    state.cycle().await.expect("passagem");
    assert!(
        destination.positions().is_empty(),
        "sem departamento, sem espaço"
    );
    assert_eq!(destination.notices().len(), 1);
}

#[tokio::test]
async fn space_failure_does_not_undo_mirrored_state() {
    let scenario = Scenario::new("espaco-falha");
    let store = scenario.store();
    let org = create_org(&store, "a");
    store.matrix().enable(org).expect("liga");
    let support_department = create_department(&store, org, "Suporte");
    let agent = create_agent(&store, org, "bia", Some(support_department));
    let conversation = create_conversation(&store, org, agent, "5511999990000");
    room_ready(&store, org, conversation);
    set_status(&store, org, conversation, ConversationStatus::Transferred);

    let destination = Fake::new();
    let state = state_loop(Arc::clone(&store), Arc::clone(&destination));
    let report = state.cycle().await.expect("passagem");

    assert_eq!(report.notified, 1, "{report:?}");
    assert_eq!(report.positioned, 0);
    assert_eq!(destination.notices().len(), 1);
    let room = store
        .matrix()
        .room(org, conversation)
        .expect("lê")
        .expect("existe");
    assert_eq!(room.last_status, Some(ConversationStatus::Transferred));
}

struct LyingPort {
    real: StateStorePort,
    reads: AtomicUsize,
    mentira: Mutex<Option<ConversationStatus>>,
}

impl LyingPort {
    fn fresh(store: Arc<Store>) -> Self {
        Self {
            real: StateStorePort::fresh(store),
            reads: AtomicUsize::new(0),
            mentira: Mutex::new(None),
        }
    }

    fn lie_on_second(&self, status: ConversationStatus) {
        *self.mentira.lock().unwrap() = Some(status);
    }
}

impl StatePort for LyingPort {
    async fn orgs(&self) -> Result<Vec<OrgState>, poph_store::error::StoreError> {
        self.real.orgs().await
    }

    async fn cursors(&self, org: OrgId) -> Result<StoredCursor, poph_store::error::StoreError> {
        self.real.cursors(org).await
    }

    async fn changed_items(
        &self,
        after: Option<(i64, ConversationId)>,
        limit: u32,
    ) -> Result<Vec<ConversationChanged>, poph_store::error::StoreError> {
        self.real.changed_items(after, limit).await
    }

    async fn conversation(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Result<Option<ConversationState>, poph_store::error::StoreError> {
        let read_number = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
        let real = self.real.conversation(org, conversation).await?;
        if read_number != 2 {
            return Ok(real);
        }
        let mentira = *self.mentira.lock().unwrap();
        Ok(real.map(|mut vista| {
            if let Some(status) = mentira {
                vista.status = status;
            }
            vista
        }))
    }

    async fn room(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Result<Option<RoomState>, poph_store::error::StoreError> {
        self.real.room(org, conversation).await
    }

    async fn define_last_status(
        &self,
        org: OrgId,
        conversation: ConversationId,
        status: ConversationStatus,
    ) -> Result<(), poph_store::error::StoreError> {
        self.real
            .define_last_status(org, conversation, status)
            .await
    }

    async fn define_name_mirrored(
        &self,
        org: OrgId,
        conversation: ConversationId,
        name: &str,
    ) -> Result<(), poph_store::error::StoreError> {
        self.real
            .define_name_mirrored(org, conversation, name)
            .await
    }

    async fn advance_conversations(
        &self,
        org: OrgId,
        updated_at: i64,
        id: &str,
    ) -> Result<(), poph_store::error::StoreError> {
        self.real.advance_conversations(org, updated_at, id).await
    }
}

#[tokio::test]
async fn conversation_closed_mid_pass_receives_no_reopening_metadata() {
    let scenario = Scenario::new("corrida");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let world = world(&store, org);
    store
        .matrix()
        .define_last_status(org, world.conversation, ConversationStatus::Transferred)
        .expect("grava a situação anterior");
    let destination = Fake::new();
    let porta = LyingPort::fresh(Arc::clone(&store));
    porta.lie_on_second(ConversationStatus::Closed);
    let state =
        ConversationStates::new(porta, Signal(Arc::clone(&destination)), &config()).expect("monta");

    let report = state.cycle().await.expect("passagem");
    assert_eq!(report.deferred, 1, "{report:?}");
    assert!(
        destination.metadatas().is_empty(),
        "nenhum metadata atrasado: a conversa fechou no meio"
    );
    assert!(destination.notices().is_empty(), "nenhum aviso atrasado");

    set_status(&store, org, world.conversation, ConversationStatus::Closed);
    let report = state.cycle().await.expect("segunda passagem");
    assert_eq!(report.notified, 1, "{report:?}");
    let metadatas = destination.metadatas();
    assert_eq!(metadatas.len(), 1);
    assert_eq!(metadatas[0].body["status"], "closed");
    assert_eq!(destination.notices().len(), 1);
    assert!(destination.notices()[0].text.contains("Conversa encerrada"));
}

#[tokio::test]
async fn store_port_reads_what_database_wrote() {
    let scenario = Scenario::new("porta");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let world = world(&store, org);
    let porta = StateStorePort::fresh(Arc::clone(&store));

    let orgs = porta.orgs().await.expect("orgs");
    assert_eq!(orgs.len(), 1);
    assert!(orgs[0].enabled);

    let cursors = porta.cursors(org).await.expect("cursores");
    assert_eq!(cursors.updated_at, 0);
    assert_eq!(cursors.id, None);

    let changed_rows = porta.changed_items(None, 10).await.expect("alteradas");
    assert_eq!(changed_rows.len(), 1);
    assert_eq!(changed_rows[0].conversation, world.conversation);

    let conversation = porta
        .conversation(org, world.conversation)
        .await
        .expect("conversa")
        .expect("existe");
    assert_eq!(conversation.status, ConversationStatus::Open);
    assert_eq!(conversation.agent, world.agent);
    assert_eq!(conversation.channel, "whatsapp");
    assert_eq!(conversation.client, "5511999990000");
    assert_eq!(conversation.agent_name.as_deref(), Some("Bia"));

    let room = porta
        .room(org, world.conversation)
        .await
        .expect("sala")
        .expect("existe");
    assert!(room.ready);
    assert_eq!(room.last_status, None);

    porta
        .define_last_status(org, world.conversation, ConversationStatus::Transferred)
        .await
        .expect("grava situação");
    let room = porta
        .room(org, world.conversation)
        .await
        .expect("sala")
        .expect("existe");
    assert_eq!(room.last_status, Some(ConversationStatus::Transferred));

    porta
        .advance_conversations(org, 5_000, &world.conversation.to_string())
        .await
        .expect("avança");
    let cursors = porta.cursors(org).await.expect("cursores");
    assert_eq!(cursors.updated_at, 5_000);
    assert_eq!(cursors.id, Some(world.conversation));
}

#[tokio::test]
async fn conversation_deleted_between_page_and_read_does_not_break_pass() {
    let scenario = Scenario::new("apagada");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let world = world(&store, org);
    let destination = Fake::new();

    let porta = DeletingPort {
        real: StateStorePort::fresh(Arc::clone(&store)),
    };
    let state =
        ConversationStates::new(porta, Signal(Arc::clone(&destination)), &config()).expect("monta");
    let report = state.cycle().await.expect("passagem");
    assert_eq!(report.checked, 1, "{report:?}");
    assert_eq!(report.without_change, 1, "{report:?}");
    assert_eq!(report.deferred, 0, "{report:?}");
    assert_eq!(destination.writes(), 0);
    let cursors = store.matrix().read_cursors(org).expect("cursores");
    assert!(
        cursors.conversations.updated_at >= INVENTED_INSTANT,
        "conversa que sumiu não prende o cursor"
    );
    let _ = world;
}

const INVENTED_INSTANT: i64 = 9_000_000;

struct DeletingPort {
    real: StateStorePort,
}

impl StatePort for DeletingPort {
    async fn orgs(&self) -> Result<Vec<OrgState>, poph_store::error::StoreError> {
        self.real.orgs().await
    }

    async fn cursors(&self, org: OrgId) -> Result<StoredCursor, poph_store::error::StoreError> {
        self.real.cursors(org).await
    }

    async fn changed_items(
        &self,
        after: Option<(i64, ConversationId)>,
        limit: u32,
    ) -> Result<Vec<ConversationChanged>, poph_store::error::StoreError> {
        let real_items = self.real.changed_items(after, limit).await?;
        let mut items = real_items;
        let org = items.first().map(|changed_row| changed_row.org);
        if let Some(org) = org {
            items.push(ConversationChanged {
                org,
                conversation: ConversationId::generate(),
                updated_at: INVENTED_INSTANT,
            });
        }
        Ok(items)
    }

    async fn conversation(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Result<Option<ConversationState>, poph_store::error::StoreError> {
        self.real.conversation(org, conversation).await
    }

    async fn room(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Result<Option<RoomState>, poph_store::error::StoreError> {
        self.real.room(org, conversation).await
    }

    async fn define_last_status(
        &self,
        org: OrgId,
        conversation: ConversationId,
        status: ConversationStatus,
    ) -> Result<(), poph_store::error::StoreError> {
        self.real
            .define_last_status(org, conversation, status)
            .await
    }

    async fn define_name_mirrored(
        &self,
        org: OrgId,
        conversation: ConversationId,
        name: &str,
    ) -> Result<(), poph_store::error::StoreError> {
        self.real
            .define_name_mirrored(org, conversation, name)
            .await
    }

    async fn advance_conversations(
        &self,
        org: OrgId,
        updated_at: i64,
        id: &str,
    ) -> Result<(), poph_store::error::StoreError> {
        self.real.advance_conversations(org, updated_at, id).await
    }
}

#[tokio::test]
async fn response_and_metadata_carry_no_customer_data() {
    let scenario = Scenario::new("sem-dado");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let world = world(&store, org);
    let destination = Fake::new();
    let state = state_loop(Arc::clone(&store), Arc::clone(&destination));

    set_status(
        &store,
        org,
        world.conversation,
        ConversationStatus::Transferred,
    );
    state.cycle().await.expect("passagem");

    for notice in destination.notices() {
        assert!(
            !notice.text.contains("5511999990000"),
            "o aviso não pode ter o telefone: {}",
            notice.text
        );
    }
    for metadata in destination.metadatas() {
        let text = metadata.body.to_string();
        assert!(text.contains("conversation_id"));
        assert!(!text.contains("\"client_name\""), "{text}");
    }
}

fn set_name(store: &Store, org: OrgId, conversation: ConversationId, name: &str) {
    let changed_name = store
        .conversations()
        .set_customer_name(org, conversation, name)
        .expect("grava o nome do cliente");
    assert!(
        changed_name,
        "o nome precisa ter mudado para o cursor enxergar"
    );
}

fn mirrored_name(store: &Store, org: OrgId, conversation: ConversationId) -> Option<String> {
    store
        .matrix()
        .room(org, conversation)
        .expect("lê a sala")
        .expect("sala existe")
        .name_client_mirrored
}

fn touch_conversation(
    scenario: &Scenario,
    store: &Store,
    org: OrgId,
    conversation: ConversationId,
) {
    let instant = scenario.clock.now.fetch_add(60_000, Ordering::SeqCst) + 60_000;
    store
        .conversations()
        .touch(org, conversation, instant)
        .expect("toca a conversa");
}

#[tokio::test]
async fn late_name_renames_room_metadata_and_ghost_once_each() {
    let scenario = Scenario::new("nome-depois");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let world = world(&store, org);
    let destination = Fake::new();
    let state = state_loop(Arc::clone(&store), Arc::clone(&destination));
    assert_eq!(mirrored_name(&store, org, world.conversation), None);

    set_name(&store, org, world.conversation, "Ana Souza");
    let report = state.cycle().await.expect("passagem do nome");

    assert_eq!(report.renamed, 1, "{report:?}");
    let renames = destination.renames();
    assert_eq!(renames.len(), 1, "uma renomeação de sala");
    assert_eq!(renames[0].room, room_for(world.conversation));
    assert_eq!(renames[0].name, "Ana Souza");

    let metadatas = destination.metadatas();
    assert_eq!(metadatas.len(), 1, "um metadata com o nome");
    assert_eq!(metadatas[0].body["client_name"], "Ana Souza");
    assert_eq!(metadatas[0].body["status"], "active");

    let ghosts = destination.ghosts();
    assert_eq!(ghosts.len(), 1, "um nome de fantasma");
    assert_eq!(ghosts[0].client, "5511999990000");
    assert_eq!(ghosts[0].name, "Ana Souza");

    assert!(destination.notices().is_empty(), "trocar nome não é aviso");
    assert_eq!(
        mirrored_name(&store, org, world.conversation).as_deref(),
        Some("Ana Souza"),
        "o nome só é dado como espelhado no fim"
    );

    let writes = destination.writes();
    touch_conversation(&scenario, &store, org, world.conversation);
    let report = state.cycle().await.expect("passagem repetida");
    assert_eq!(report.renamed, 0, "{report:?}");
    assert_eq!(destination.writes(), writes, "repetir não escreve nada");
    assert_eq!(destination.renames().len(), 1);
    assert_eq!(destination.metadatas().len(), 1);
    assert_eq!(destination.ghosts().len(), 1);

    set_name(&store, org, world.conversation, "Ana Maria");
    let report = state.cycle().await.expect("passagem do nome novo");
    assert_eq!(report.renamed, 1, "{report:?}");
    assert_eq!(destination.renames().len(), 2);
    assert_eq!(destination.renames()[1].name, "Ana Maria");
    assert_eq!(destination.metadatas()[1].body["client_name"], "Ana Maria");
    assert_eq!(destination.ghosts()[1].name, "Ana Maria");
    assert_eq!(
        mirrored_name(&store, org, world.conversation).as_deref(),
        Some("Ana Maria")
    );
}

#[tokio::test]
async fn failure_between_name_and_metadata_resumes_without_duplicate_room_or_notice() {
    let scenario = Scenario::new("queda-no-nome");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let world = world(&store, org);
    let destination = Fake::new();
    let state = state_loop(Arc::clone(&store), Arc::clone(&destination));

    set_name(&store, org, world.conversation, "Ana Souza");
    *destination.mode.lock().unwrap() = Mode::FailureOnWrite(2);
    let report = state.cycle().await.expect("passagem que cai");

    assert_eq!(report.deferred, 1, "{report:?}");
    assert_eq!(destination.renames().len(), 1, "a sala foi renomeada");
    assert!(destination.metadatas().is_empty(), "o metadata não saiu");
    assert!(
        destination.ghosts().is_empty(),
        "o fantasma vem depois do metadata"
    );
    assert_eq!(
        mirrored_name(&store, org, world.conversation),
        None,
        "queda no meio não dá o nome como espelhado"
    );

    let report = state.cycle().await.expect("passagem da retomada");
    assert_eq!(report.renamed, 1, "{report:?}");
    assert_eq!(destination.renames().len(), 2, "a renomeação se repete");
    assert_eq!(destination.renames()[1].name, "Ana Souza");
    assert_eq!(destination.metadatas().len(), 1, "um metadata só");
    assert_eq!(destination.ghosts().len(), 1);
    assert!(
        destination.notices().is_empty(),
        "nenhum aviso de atendimento"
    );
    assert_eq!(
        store
            .matrix()
            .room(org, world.conversation)
            .expect("lê a sala")
            .expect("sala existe")
            .room_id,
        room_for(world.conversation)
    );
    assert_eq!(
        mirrored_name(&store, org, world.conversation).as_deref(),
        Some("Ana Souza")
    );
}

#[tokio::test]
async fn late_name_does_not_return_closed_conversation_to_active_in_metadata() {
    let scenario = Scenario::new("nome-atrasado");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let world = world(&store, org);
    let destination = Fake::new();

    set_status(&store, org, world.conversation, ConversationStatus::Closed);
    set_name(&store, org, world.conversation, "Ana Souza");

    let atrasada = DelayedPort::fresh(Arc::clone(&store));
    let state = ConversationStates::new(atrasada, Signal(Arc::clone(&destination)), &config())
        .expect("monta o laço");

    let report = state.cycle().await.expect("passagem do nome");
    assert_eq!(report.renamed, 1, "{report:?}");
    let metadatas = destination.metadatas();
    assert_eq!(metadatas.len(), 1);
    assert_eq!(
        metadatas[0].body["status"], "closed",
        "o metadata do nome sai com o estado atual da conversa"
    );
    assert_eq!(metadatas[0].body["client_name"], "Ana Souza");
    assert!(
        destination.notices().is_empty(),
        "renomear não é aviso e a leitura atrasada não gerou aviso"
    );
    assert_eq!(
        store
            .conversations()
            .get(org, world.conversation)
            .expect("lê a conversa")
            .status,
        ConversationStatus::Closed
    );

    assert_eq!(
        store
            .matrix()
            .room(org, world.conversation)
            .expect("lê a sala")
            .expect("sala existe")
            .last_status,
        None,
        "renomear não espelha situação"
    );
}

#[tokio::test]
async fn conversation_without_new_name_and_disabled_org_do_not_talk_to_synapse() {
    let scenario = Scenario::new("sem-nome-novo");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let world = world(&store, org);
    let destination = Fake::new();
    let state = state_loop(Arc::clone(&store), Arc::clone(&destination));

    touch_conversation(&scenario, &store, org, world.conversation);
    let report = state.cycle().await.expect("passagem sem nome");
    assert_eq!(report.renamed, 0, "{report:?}");
    assert!(destination.renames().is_empty());
    assert!(destination.metadatas().is_empty());
    assert!(destination.ghosts().is_empty());

    set_name(&store, org, world.conversation, "Ana Souza");
    state.cycle().await.expect("passagem do nome");
    let writes = destination.writes();
    touch_conversation(&scenario, &store, org, world.conversation);
    state.cycle().await.expect("passagem repetida");
    assert_eq!(destination.writes(), writes);

    store.matrix().disable(org).expect("desliga a org");
    set_name(&store, org, world.conversation, "Ana Maria");
    let writes = destination.writes();
    let report = state.cycle().await.expect("passagem com a org desligada");
    assert_eq!(report.renamed, 0, "{report:?}");
    assert_eq!(destination.writes(), writes, "org desligada não escreve");
    assert_eq!(
        mirrored_name(&store, org, world.conversation).as_deref(),
        Some("Ana Souza"),
        "o nome espelhado fica como estava"
    );
}

struct DelayedPort {
    real: StateStorePort,
    atrasadas: Mutex<std::collections::BTreeSet<String>>,
}

impl DelayedPort {
    fn fresh(store: Arc<Store>) -> Self {
        Self {
            real: StateStorePort::fresh(store),
            atrasadas: Mutex::new(std::collections::BTreeSet::new()),
        }
    }
}

impl StatePort for DelayedPort {
    async fn orgs(&self) -> Result<Vec<OrgState>, poph_store::error::StoreError> {
        self.real.orgs().await
    }

    async fn cursors(&self, org: OrgId) -> Result<StoredCursor, poph_store::error::StoreError> {
        self.real.cursors(org).await
    }

    async fn changed_items(
        &self,
        after: Option<(i64, ConversationId)>,
        limit: u32,
    ) -> Result<Vec<ConversationChanged>, poph_store::error::StoreError> {
        self.real.changed_items(after, limit).await
    }

    async fn conversation(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Result<Option<ConversationState>, poph_store::error::StoreError> {
        let first = self
            .atrasadas
            .lock()
            .unwrap()
            .insert(conversation.to_string());
        let real = self.real.conversation(org, conversation).await?;
        if !first {
            return Ok(real);
        }
        Ok(real.map(|mut vista| {
            vista.status = ConversationStatus::Open;
            vista
        }))
    }

    async fn room(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Result<Option<RoomState>, poph_store::error::StoreError> {
        self.real.room(org, conversation).await
    }

    async fn define_last_status(
        &self,
        org: OrgId,
        conversation: ConversationId,
        status: ConversationStatus,
    ) -> Result<(), poph_store::error::StoreError> {
        self.real
            .define_last_status(org, conversation, status)
            .await
    }

    async fn define_name_mirrored(
        &self,
        org: OrgId,
        conversation: ConversationId,
        name: &str,
    ) -> Result<(), poph_store::error::StoreError> {
        self.real
            .define_name_mirrored(org, conversation, name)
            .await
    }

    async fn advance_conversations(
        &self,
        org: OrgId,
        updated_at: i64,
        id: &str,
    ) -> Result<(), poph_store::error::StoreError> {
        self.real.advance_conversations(org, updated_at, id).await
    }
}

#[test]
fn report_error_has_short_code() {
    let error = StateError::Armazenamento(poph_store::error::StoreError::WriterClosed);
    assert_eq!(error.code(), "armazenamento_indisponivel");
}
