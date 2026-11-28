use std::collections::BTreeMap;
use std::fs;
use std::future::Future;
use std::net::{SocketAddr, TcpListener as StdListener};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use poph_agent::actors::{ActorOptions, Actors};
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::entry_output::{FlowOptions, Fluxo};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsFlow, MetricsMemory, MetricsTurn};
use poph_agent::turno::{TurnEngine, TurnOptions, TurnProcessor};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, LlmProvider, Script, Usage};
use poph_evolution::webhook::SECRET_HEADER;
use poph_evolution::{ATTACHMENT_CAP, ConfigEvolution, EvolutionClient};
use poph_matrix::client::{ClientConfig, MatrixClient};
use poph_matrix::config::{MatrixModule, MatrixSecrets};
use poph_matrix::spaces::{DepartmentSpace, InviteLimits, Position, RoomForSpace, Spaces};
use poph_matrix::users::{Ghost, GhostIds};
use poph_server::ServerState;
use poph_server::evolution::EvolutionChannel;
use poph_server::matrix_beginning::{self, RunningMatrix, SpaceStorePort, StartupSettings};
use poph_server::{turn_input, whatsapp_output};
use poph_store::{
    Direction, NewAgent, NewConversation, NewDepartment, NewMessage, NewOrg, Origin, OutboxState,
    Page, Role, StateRoom, Store,
};
use poph_types::{
    AgentId, ConversationId, DepartmentId, HarnessConfig, IntegrationId, MatrixConfig, OrgId,
    Secret,
};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const AS_TOKEN: &str = "CANARIO-AS-mx29-nao-pode-vazar";
const HS_TOKEN: &str = "CANARIO-HS-mx29-nao-pode-vazar";
const SERVIDOR: &str = "matrix-dev.local";
const PREFIX: &str = "ph";
const WEBHOOK_SECRET: &str = "segredo-webhook-mx29";
const EVO_KEY: &str = "chave-evo-mx29";
const INSTANCIA: &str = "loja-mx29";
const PHONE: &str = "5511999990029";
const JID: &str = "5511999990029@s.whatsapp.net";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn free_port() -> SocketAddr {
    let listener = StdListener::bind("127.0.0.1:0").expect("porta livre");
    listener.local_addr().expect("endereço")
}

fn bind_port(endereco: SocketAddr) -> StdListener {
    for _ in 0..80 {
        if let Ok(listener) = StdListener::bind(endereco) {
            return listener;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("não consegui religar a porta {endereco}");
}

async fn wait_until(label: &str, mut condition: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(40) {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("tempo esgotado esperando: {label}");
}

fn marker(i: usize) -> String {
    format!("[m{i:02}]")
}

fn check_order(texts: &[String], expected: &[String]) -> Result<(), String> {
    let mut positions = Vec::with_capacity(expected.len());
    for mark in expected {
        let findings: Vec<usize> = texts
            .iter()
            .enumerate()
            .filter(|(_, text)| text.contains(mark.as_str()))
            .map(|(position, _)| position)
            .collect();
        match findings.as_slice() {
            [single] => positions.push(*single),
            _ => {
                return Err(format!(
                    "{mark} aparece {} vezes na sala (esperado 1)",
                    findings.len()
                ));
            }
        }
    }
    if positions.windows(2).all(|pair| pair[0] < pair[1]) {
        Ok(())
    } else {
        Err("as mensagens da sala não estão na ordem do banco".to_owned())
    }
}

fn statistics(values: &[f64]) -> (f64, f64, f64, f64) {
    let mut ordenados = values.to_vec();
    ordenados.sort_by(|a, b| a.partial_cmp(b).expect("números comparáveis"));
    let n = ordenados.len();
    let median = if n % 2 == 1 {
        ordenados[n / 2]
    } else {
        (ordenados[n / 2 - 1] + ordenados[n / 2]) / 2.0
    };
    let p95_index = ((n as f64 * 0.95).ceil() as usize).clamp(1, n) - 1;
    let asset = ordenados.iter().sum::<f64>() / n as f64;
    (median, ordenados[p95_index], asset, ordenados[n - 1])
}

struct Hospedeiro {
    stop: Option<oneshot::Sender<()>>,
    fio: Option<std::thread::JoinHandle<()>>,
}

impl Hospedeiro {
    fn boot<F, Fut>(name: &'static str, principal: F) -> Self
    where
        F: FnOnce(oneshot::Receiver<()>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let (stop, recebe) = oneshot::channel();
        let fio = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("runtime do processo simulado");
                rt.block_on(principal(recebe));
            })
            .expect("thread do processo simulado");
        Self {
            stop: Some(stop),
            fio: Some(fio),
        }
    }

    fn crash(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(fio) = self.fio.take() {
            let _ = fio.join();
        }
    }
}

impl Drop for Hospedeiro {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Default, Clone)]
struct FakeRoom {
    members: Vec<String>,
    guests: Vec<String>,
    state: BTreeMap<(String, String), Value>,
    messages: Vec<(String, String, Value)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    CreateRoom,
    Invite,
    Send,
    Profile,
}

#[derive(Clone)]
struct Hold {
    target: Target,
    text: Option<String>,
    ocorrencia: u32,
    seen: u32,
}

impl Hold {
    fn new(target: Target, ocorrencia: u32, text: Option<&str>) -> Self {
        Self {
            target,
            text: text.map(str::to_owned),
            ocorrencia,
            seen: 0,
        }
    }
}

#[derive(Default)]
struct FakeState {
    users: BTreeMap<String, Option<String>>,
    rooms: BTreeMap<String, FakeRoom>,
    aliases: BTreeMap<String, String>,
    next_room: u32,
    delay: Duration,
    hold: Option<Hold>,
    chegou: bool,
    requests: u32,
    respondidos: u32,
    fail_invite: Option<u32>,
    invites_seen: u32,
    reject_text: Option<String>,
    raw_sends: Vec<(String, String)>,
}

impl FakeState {
    fn hold_if(&mut self, target: Target, detail: &str) -> bool {
        let Some(hold) = self.hold.as_mut() else {
            return false;
        };
        if hold.target != target {
            return false;
        }
        if let Some(text) = &hold.text
            && !detail.contains(text.as_str())
        {
            return false;
        }
        hold.seen += 1;
        if hold.seen == hold.ocorrencia {
            self.chegou = true;
            return true;
        }
        false
    }
}

fn respond(status: u16, body: Value) -> Response {
    (
        StatusCode::from_u16(status).expect("status válido"),
        [("content-type", "application/json")],
        body.to_string(),
    )
        .into_response()
}

fn decode(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut saida = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
        {
            saida.push(byte);
            i += 3;
            continue;
        }
        saida.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&saida).into_owned()
}

fn query(raw: &str, name: &str) -> Option<String> {
    raw.split('&').find_map(|pair| {
        let (chave, value) = pair.split_once('=')?;
        (chave == name).then(|| decode(value))
    })
}

enum Decision {
    Responds(Response),
    Holds,
}

fn execute(
    g: &mut FakeState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &Value,
) -> Decision {
    let authorization = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if authorization != format!("Bearer {AS_TOKEN}") {
        return Decision::Responds(respond(
            401,
            json!({ "errcode": "M_UNKNOWN_TOKEN", "error": "Token inválido" }),
        ));
    }
    if uri.path() == "/_matrix/client/versions" {
        return Decision::Responds(respond(200, json!({ "versions": ["v1.11"] })));
    }

    let path = uri.path().trim_start_matches("/_matrix/client/v3");
    let parts: Vec<String> = path
        .split('/')
        .filter(|piece| !piece.is_empty())
        .map(decode)
        .collect();
    let who = query(uri.query().unwrap_or_default(), "user_id").unwrap_or_default();
    let missing = || {
        respond(
            404,
            json!({ "errcode": "M_NOT_FOUND", "error": "Não existe no Synapse falso" }),
        )
    };

    let reply = match (method.as_str(), parts.as_slice()) {
        ("POST", [registro]) if registro == "register" => {
            let localpart = body["username"].as_str().unwrap_or_default();
            let id = format!("@{localpart}:{SERVIDOR}");
            if g.users.contains_key(&id) {
                respond(
                    400,
                    json!({ "errcode": "M_USER_IN_USE", "error": "User ID already taken." }),
                )
            } else {
                g.users.insert(id.clone(), None);
                respond(200, json!({ "user_id": id }))
            }
        }
        ("GET", [profile, id]) if profile == "profile" => {
            if g.hold_if(Target::Profile, id) {
                return Decision::Holds;
            }
            match g.users.get(id) {
                Some(name) => {
                    let mut body = json!({});
                    if let Some(name) = name {
                        body["displayname"] = Value::String(name.clone());
                    }
                    respond(200, body)
                }
                None => missing(),
            }
        }
        ("PUT", [profile, id, campo]) if profile == "profile" && campo == "displayname" => {
            g.users
                .insert(id.clone(), body["displayname"].as_str().map(str::to_owned));
            respond(200, json!({}))
        }
        ("POST", [creation]) if creation == "createRoom" => {
            let alias = body["room_alias_name"]
                .as_str()
                .map(|localpart| format!("#{localpart}:{SERVIDOR}"));
            if let Some(alias) = &alias
                && g.aliases.contains_key(alias)
            {
                return Decision::Responds(respond(
                    400,
                    json!({ "errcode": "M_ROOM_IN_USE", "error": "Room alias already taken." }),
                ));
            }
            g.next_room += 1;
            let room = format!("!{}:{SERVIDOR}", g.next_room);
            let members = if who.is_empty() {
                Vec::new()
            } else {
                vec![who.clone()]
            };
            g.rooms.insert(
                room.clone(),
                FakeRoom {
                    members,
                    ..FakeRoom::default()
                },
            );
            if let Some(alias) = alias {
                g.aliases.insert(alias.clone(), room.clone());
                if g.hold_if(Target::CreateRoom, &alias) {
                    return Decision::Holds;
                }
            }
            respond(200, json!({ "room_id": room }))
        }
        ("GET", [diretorio, room, alias]) if diretorio == "directory" && room == "room" => {
            match g.aliases.get(alias) {
                Some(room) => respond(200, json!({ "room_id": room })),
                None => missing(),
            }
        }
        ("POST", [rooms, room, invite]) if rooms == "rooms" && invite == "invite" => {
            g.invites_seen += 1;
            if g.fail_invite == Some(g.invites_seen) {
                g.fail_invite = None;
                return Decision::Responds(respond(
                    503,
                    json!({ "errcode": "M_UNKNOWN", "error": "indisponível" }),
                ));
            }
            match g.rooms.get_mut(room) {
                Some(target) => {
                    let invited = body["user_id"].as_str().unwrap_or_default().to_owned();
                    if !target.guests.contains(&invited) {
                        target.guests.push(invited.clone());
                    }
                    if g.hold_if(Target::Invite, &invited) {
                        return Decision::Holds;
                    }
                    respond(200, json!({}))
                }
                None => missing(),
            }
        }
        ("POST", [enter, target]) if enter == "join" => {
            let room = if target.starts_with('#') {
                g.aliases.get(target).cloned()
            } else {
                Some(target.clone())
            };
            match room.and_then(|room| g.rooms.get_mut(&room).map(|target| (room, target))) {
                Some((room, target)) => {
                    if !target.members.contains(&who) {
                        target.members.push(who.clone());
                    }
                    respond(200, json!({ "room_id": room }))
                }
                None => missing(),
            }
        }
        ("GET", [rooms, room, members]) if rooms == "rooms" && members == "joined_members" => {
            match g.rooms.get(room) {
                Some(target) => {
                    let together: BTreeMap<String, Value> = target
                        .members
                        .iter()
                        .map(|id| (id.clone(), json!({})))
                        .collect();
                    respond(200, json!({ "joined": together }))
                }
                None => missing(),
            }
        }
        ("PUT", [rooms, room, state, tipo, chave @ ..]) if rooms == "rooms" && state == "state" => {
            let chave = chave.join("/");
            match g.rooms.get_mut(room) {
                Some(target) => {
                    target.state.insert((tipo.clone(), chave), body.clone());
                    respond(200, json!({ "event_id": "$estado" }))
                }
                None => missing(),
            }
        }
        ("GET", [rooms, room, state]) if rooms == "rooms" && state == "state" => {
            match g.rooms.get(room) {
                Some(target) => {
                    let events: Vec<Value> = target
                    .state
                    .iter()
                    .map(|((tipo, chave), body)| json!({ "type": tipo, "state_key": chave, "content": body }))
                    .collect();
                    respond(200, Value::Array(events))
                }
                None => missing(),
            }
        }
        ("GET", [rooms, room, state, tipo, chave @ ..]) if rooms == "rooms" && state == "state" => {
            let chave = chave.join("/");
            match g
                .rooms
                .get(room)
                .and_then(|target| target.state.get(&(tipo.clone(), chave)))
            {
                Some(body) => respond(200, body.clone()),
                None => missing(),
            }
        }
        ("PUT", [rooms, room, send, tipo, txn]) if rooms == "rooms" && send == "send" => {
            let text = body["body"]
                .as_str()
                .map_or_else(|| body.to_string(), str::to_owned);
            g.raw_sends.push((txn.clone(), text.clone()));
            if g.reject_text
                .as_deref()
                .is_some_and(|mark| text.contains(mark))
            {
                return Decision::Responds(respond(
                    400,
                    json!({ "errcode": "M_BAD_JSON", "error": "Corpo recusado" }),
                ));
            }
            match g.rooms.get_mut(room) {
                Some(target) => {
                    if !target.messages.iter().any(|(ja, _, _)| ja == txn) {
                        target
                            .messages
                            .push((txn.clone(), tipo.clone(), body.clone()));
                    }
                    if g.hold_if(Target::Send, &text) {
                        return Decision::Holds;
                    }
                    respond(200, json!({ "event_id": format!("$msg_{txn}") }))
                }
                None => missing(),
            }
        }
        _ => respond(
            404,
            json!({ "errcode": "M_UNRECOGNIZED", "error": "Rota não existe no Synapse falso" }),
        ),
    };
    Decision::Responds(reply)
}

async fn attend(
    State(state): State<Arc<Mutex<FakeState>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let probe = uri.path() == "/_matrix/client/versions";
    let delay = {
        let mut g = lock(&state);
        if !probe {
            g.requests += 1;
        }
        g.delay
    };
    if !delay.is_zero() && !probe {
        tokio::time::sleep(delay).await;
    }
    let decisao = {
        let mut g = lock(&state);
        execute(&mut g, &method, &uri, &headers, &body)
    };
    match decisao {
        Decision::Holds => {
            std::future::pending::<()>().await;
            unreachable!("a chamada segurada nunca tem resposta")
        }
        Decision::Responds(reply) => {
            if !probe {
                lock(&state).respondidos += 1;
            }
            reply
        }
    }
}

async fn principal_synapse(
    listener: StdListener,
    state: Arc<Mutex<FakeState>>,
    recebe: oneshot::Receiver<()>,
) {
    listener.set_nonblocking(true).expect("modo não bloqueante");
    let listener = tokio::net::TcpListener::from_std(listener).expect("ouvinte do Synapse falso");
    let app = Router::new().fallback(attend).with_state(state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let _ = recebe.await;
}

struct Synapse {
    endereco: String,
    addr: SocketAddr,
    state: Arc<Mutex<FakeState>>,
    process: Option<Hospedeiro>,
}

impl Synapse {
    fn em(addr: SocketAddr) -> Self {
        Self {
            endereco: format!("http://{addr}"),
            addr,
            state: Arc::new(Mutex::new(FakeState::default())),
            process: None,
        }
    }

    fn no_ar() -> Self {
        let mut synapse = Self::em(free_port());
        synapse.boot();
        synapse
    }

    fn boot(&mut self) {
        if self.process.is_some() {
            return;
        }
        let listener = bind_port(self.addr);
        let state = Arc::clone(&self.state);
        self.process = Some(Hospedeiro::boot("synapse-falso", move |recebe| {
            principal_synapse(listener, state, recebe)
        }));
    }

    fn crash(&mut self) {
        if let Some(mut process) = self.process.take() {
            process.shutdown();
        }
    }

    fn configure<R>(&self, f: impl FnOnce(&mut FakeState) -> R) -> R {
        f(&mut lock(&self.state))
    }

    fn look<R>(&self, f: impl FnOnce(&FakeState) -> R) -> R {
        f(&lock(&self.state))
    }

    fn seed_room(&self, id: &str, members: &[&str], state: Vec<((&str, &str), Value)>) {
        self.configure(|g| {
            let mut room = FakeRoom {
                members: members.iter().map(|m| (*m).to_owned()).collect(),
                ..FakeRoom::default()
            };
            for ((tipo, chave), value) in state {
                room.state
                    .insert((tipo.to_owned(), chave.to_owned()), value);
            }
            g.rooms.insert(id.to_owned(), room);
        });
    }
}

impl Drop for Synapse {
    fn drop(&mut self) {
        self.crash();
    }
}

fn single_room(sinapse: &Synapse) -> Option<String> {
    sinapse.look(|g| {
        if g.rooms.len() == 1 {
            g.rooms.keys().next().cloned()
        } else {
            None
        }
    })
}

fn room_texts(sinapse: &Synapse, room: &str) -> Vec<String> {
    sinapse.look(|g| {
        g.rooms
            .get(room)
            .map(|target| {
                target
                    .messages
                    .iter()
                    .map(|(_, _, body)| body.to_string())
                    .collect()
            })
            .unwrap_or_default()
    })
}

struct Base {
    dir: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agent: AgentId,
    conversation: ConversationId,
}

impl Base {
    fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("poph-mx29-{label}-{}-{stamp}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("pasta do teste");
        let store =
            Arc::new(Store::open_at(&dir.join("harness.db"), &dir.join("rag.db")).expect("banco"));
        let org = store
            .orgs()
            .create(NewOrg {
                name: format!("Org {label}"),
                slug: format!("org-{label}"),
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
                    grouping_window_ms: 0,
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        let conversation = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agent,
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some(PHONE.to_owned()),
                    target_department: None,
                },
            )
            .expect("cria a conversa")
            .id;
        Self {
            dir,
            store,
            org,
            agent,
            conversation,
        }
    }

    fn db(&self) -> PathBuf {
        self.dir.join("harness.db")
    }

    fn rag(&self) -> PathBuf {
        self.dir.join("rag.db")
    }

    fn ligar(&self) {
        self.store.matrix().enable(self.org).expect("liga o módulo");
    }

    fn record(&self, direction: Direction, role: Role, text: &str) -> String {
        self.store
            .messages()
            .write(
                self.org,
                NewMessage {
                    conversation_id: self.conversation,
                    direction,
                    role,
                    origin: Origin::Agente,
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

    fn client(&self, text: &str) -> String {
        self.record(Direction::Inbound, Role::User, text)
    }

    fn agent_speaks(&self, text: &str) -> String {
        self.record(Direction::Outbound, Role::Assistant, text)
    }
}

impl Drop for Base {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn config_matrix(url: &str, interval_ms: u64) -> MatrixConfig {
    MatrixConfig {
        enabled: true,
        homeserver_url: url.to_owned(),
        server_name: SERVIDOR.to_owned(),
        prefix: PREFIX.to_owned(),
        concurrency: 4,
        interval_ms,
        ..MatrixConfig::default()
    }
}

fn module_of(config: &MatrixConfig) -> MatrixModule {
    MatrixModule {
        config: config.clone(),
        secrets: MatrixSecrets {
            as_token: Secret::new(AS_TOKEN.to_owned()),
            hs_token: Secret::new(HS_TOKEN.to_owned()),
        },
    }
}

fn test_settings() -> StartupSettings {
    StartupSettings {
        probe_period: Duration::from_millis(100),
        probe_delay_cap: Duration::from_millis(200),
        shutdown_timeout: Duration::from_secs(1),
    }
}

fn matrix_customer(url: &str) -> MatrixClient {
    MatrixClient::new(
        ClientConfig::new(url, SERVIDOR, Secret::new(AS_TOKEN.to_owned()))
            .expect("config do cliente"),
    )
    .expect("cliente Matrix")
}

fn module_process(base: &Base, config: MatrixConfig) -> Hospedeiro {
    let db = base.db();
    let rag = base.rag();
    Hospedeiro::boot("harness-modulo", move |recebe| async move {
        let store = Arc::new(Store::open_at(&db, &rag).expect("abre o banco no processo"));
        let harness = HarnessConfig {
            matrix: Some(config.clone()),
            ..HarnessConfig::default()
        };
        let mut state = ServerState::new(store, harness).expect("estado do servidor");
        let _running =
            matrix_beginning::start_with_settings(&mut state, module_of(&config), test_settings());
        let _ = recebe.await;
    })
}

fn receiver_process(base: &Base, config: MatrixConfig, listener: StdListener) -> Hospedeiro {
    let db = base.db();
    let rag = base.rag();
    Hospedeiro::boot("harness-receptor", move |recebe| async move {
        listener.set_nonblocking(true).expect("modo não bloqueante");
        let listener = tokio::net::TcpListener::from_std(listener).expect("ouvinte do Harness");
        let store = Arc::new(Store::open_at(&db, &rag).expect("abre o banco no processo"));
        let harness = HarnessConfig {
            matrix: Some(config.clone()),
            ..HarnessConfig::default()
        };
        let mut state = ServerState::new(store, harness).expect("estado do servidor");
        let _running =
            matrix_beginning::start_with_settings(&mut state, module_of(&config), test_settings());
        let app = poph_server::router(state);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let _ = recebe.await;
    })
}

#[derive(Clone, Default)]
struct FakeEvolution {
    arrivals: Arc<Mutex<Vec<Instant>>>,
}

async fn send_fake(
    State(evo): State<FakeEvolution>,
    Json(_body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    lock(&evo.arrivals).push(Instant::now());
    let n = lock(&evo.arrivals).len();
    (
        StatusCode::OK,
        Json(json!({ "key": { "id": format!("MSG-SAIDA-{n}") } })),
    )
}

async fn boot_evolution(evo: FakeEvolution) -> String {
    let router = Router::new()
        .route("/message/sendText/{instance}", post(send_fake))
        .route("/message/sendMedia/{instance}", post(send_fake))
        .with_state(evo);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta da Evolution falsa");
    let endereco = listener.local_addr().expect("endereço");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    format!("http://{endereco}")
}

fn agent_reply(i: usize) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(format!("resposta do agente {}", marker(i))),
        LlmEvent::Usage(Usage {
            input: 120,
            saida: 30,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

struct Servidor {
    llm: Arc<FakeProvider>,
    base_url: String,
    http: reqwest::Client,
    org: OrgId,
    evo: FakeEvolution,
    desligar: Option<oneshot::Sender<()>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    module: Option<RunningMatrix>,
}

impl Servidor {
    async fn webhook(&self, id: &str, text: &str) -> StatusCode {
        let body = json!({
            "event": "messages.upsert",
            "instance": INSTANCIA,
            "apikey": EVO_KEY,
            "destination": "http://harness.teste/webhook/evolution/x/loja",
            "data": {
                "key": {"id": id, "remoteJid": JID, "fromMe": false},
                "pushName": "Cliente Teste",
                "messageType": "conversation",
                "message": {"conversation": text},
                "messageTimestamp": 1795177000
            }
        });
        let url = format!(
            "{}/webhook/evolution/{}/{INSTANCIA}",
            self.base_url, self.org
        );
        self.http
            .post(url)
            .header(SECRET_HEADER, WEBHOOK_SECRET)
            .json(&body)
            .send()
            .await
            .expect("entrega o webhook")
            .status()
    }

    fn llm_calls(&self) -> usize {
        self.llm.received().len()
    }

    fn arrivals(&self) -> Vec<Instant> {
        lock(&self.evo.arrivals).clone()
    }

    async fn shutdown(mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        for task in self.tasks.drain(..) {
            task.abort();
        }
        if let Some(module) = self.module.take() {
            module.stop().await;
        }
    }
}

async fn boot_server(
    base: &Base,
    config: Option<MatrixConfig>,
    settings: StartupSettings,
    replies: usize,
) -> Servidor {
    let store = Arc::new(Store::open_at(&base.db(), &base.rag()).expect("banco do servidor"));
    let evo = FakeEvolution::default();
    let url_evo = boot_evolution(evo.clone()).await;
    let integracao: IntegrationId = store
        .create_integration(
            base.org,
            base.agent,
            "whatsapp".to_owned(),
            "active".to_owned(),
        )
        .expect("cria a integração")
        .id
        .parse()
        .expect("id da integração");
    let evo_customer = EvolutionClient::new(
        ConfigEvolution::new(url_evo, INSTANCIA, Secret::new(EVO_KEY.to_owned()))
            .expect("config da Evolution falsa"),
    )
    .expect("cliente da Evolution falsa");
    let channel = EvolutionChannel {
        org: base.org,
        instance: INSTANCIA.to_owned(),
        integration: integracao,
        secret: Some(Secret::new(WEBHOOK_SECRET.to_owned())),
        client: Some(evo_customer),
        attachment_limit: ATTACHMENT_CAP,
    };
    store
        .instructions()
        .save(
            base.org,
            base.agent,
            0,
            "{}",
            "# Atendente\n\nVocê atende a loja.",
            true,
        )
        .expect("grava as instruções");

    let harness = HarnessConfig {
        matrix: config.clone(),
        ..HarnessConfig::default()
    };
    let state = ServerState::new(Arc::clone(&store), harness)
        .expect("estado do servidor")
        .with_evolution_channels(vec![channel]);

    let scripts: Vec<Script> = (0..replies).map(agent_reply).collect();
    let fake = Arc::new(FakeProvider::with_scripts(scripts));
    let provedor: Arc<dyn LlmProvider> = fake.clone();
    let registro = &state.registry;
    let hot_config = ConfigHot::new(
        Arc::clone(&store),
        HotConfigOptions::default(),
        MetricsMemory::own(),
    );
    let limiter = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
    let engine = TurnEngine::new(
        Arc::clone(&store),
        hot_config,
        limiter,
        provedor,
        TurnOptions {
            default_window: Duration::from_millis(1),
            ..TurnOptions::default()
        },
        MetricsTurn::register(registro).expect("métricas do turno"),
    );
    let processor = Arc::new(TurnProcessor::new(Arc::new(engine)));
    let actors = Actors::new(
        processor.clone(),
        ActorOptions::default(),
        MetricsMemory::own(),
    );
    let fluxo = Fluxo::with_metrics(
        Arc::clone(&store),
        actors,
        FlowOptions::default(),
        MetricsFlow::register(registro).expect("métricas da fila"),
    );
    processor.attach_io(fluxo.clone());
    let mut state = state.with_turn(fluxo, Duration::from_millis(20));

    let module = match config {
        Some(config) => {
            matrix_beginning::start_with_settings(&mut state, module_of(&config), settings)
        }
        None => None,
    };
    let mut tasks = Vec::new();
    if let Some(saida) = whatsapp_output::start(&state) {
        tasks.push(saida);
    }
    if let Some(turno) = turn_input::start(&state) {
        tasks.push(turno);
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta do servidor");
    let endereco = listener.local_addr().expect("endereço");
    let (desligar, signal) = oneshot::channel::<()>();
    let app = poph_server::router(state);
    tasks.push(tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = signal.await;
            })
            .await;
    }));

    Servidor {
        llm: fake,
        base_url: format!("http://{endereco}"),
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("cliente HTTP"),
        org: base.org,
        evo,
        desligar: Some(desligar),
        tasks,
        module,
    }
}

async fn wait_send(servidor: &Servidor, i: usize) -> Instant {
    let start = Instant::now();
    loop {
        if let Some(arrival) = servidor.arrivals().get(i) {
            return *arrival;
        }
        if start.elapsed() > Duration::from_secs(30) {
            panic!("o envio {} não chegou à Evolution falsa", i + 1);
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

#[tokio::test]
async fn customer_gets_reply_with_synapse_down_and_module_on() {
    let base = Base::new("fora");
    base.ligar();
    let closed_url = format!("http://{}", free_port());
    let config = config_matrix(&closed_url, 50);
    let servidor = boot_server(&base, Some(config), test_settings(), 8).await;

    for i in 0..8 {
        let status = servidor
            .webhook(&format!("MSG-FORA-{i}"), &format!("pergunta {}", marker(i)))
            .await;
        assert_eq!(status, StatusCode::OK);
        let _ = wait_send(&servidor, i).await;
    }
    assert_eq!(
        servidor.arrivals().len(),
        8,
        "com o Synapse fora, as 8 respostas saem para o cliente"
    );
    servidor.shutdown().await;
}

#[tokio::test]
async fn reply_leaves_before_slow_synapse_answers() {
    let synapse = Synapse::no_ar();
    synapse.configure(|g| g.delay = Duration::from_secs(5));
    let base = Base::new("lento");
    base.ligar();
    let config = config_matrix(&synapse.endereco, 50);
    let servidor = boot_server(&base, Some(config), test_settings(), 2).await;

    assert_eq!(
        servidor.webhook("MSG-LENTO-1", "primeira").await,
        StatusCode::OK
    );
    let _ = wait_send(&servidor, 0).await;
    wait_until("o módulo chamar o Synapse lento", || {
        synapse.look(|g| g.requests >= 1)
    })
    .await;

    assert_eq!(
        servidor.webhook("MSG-LENTO-2", "segunda").await,
        StatusCode::OK
    );
    let _ = wait_send(&servidor, 1).await;
    let respondidos = synapse.look(|g| g.respondidos);
    assert_eq!(
        respondidos, 0,
        "a segunda resposta saiu antes de qualquer chamada lenta ao Synapse terminar"
    );
    servidor.shutdown().await;
}

#[tokio::test]
async fn synapse_return_drains_queue_in_order_without_duplicates() {
    let mut synapse = Synapse::em(free_port());
    let base = Base::new("volta");
    base.ligar();
    let process = module_process(&base, config_matrix(&synapse.endereco, 50));

    let mut expected = Vec::new();
    for i in 0..5 {
        base.client(&format!("cliente {}", marker(2 * i)));
        base.agent_speaks(&format!("agente {}", marker(2 * i + 1)));
        expected.push(marker(2 * i));
        expected.push(marker(2 * i + 1));
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        base.store
            .matrix()
            .read_cursors(base.org)
            .expect("cursores")
            .messages,
        0,
        "com o Synapse fora, o cursor não anda"
    );

    synapse.boot();
    wait_until("a sala receber as mensagens", || {
        single_room(&synapse)
            .is_some_and(|room| check_order(&room_texts(&synapse, &room), &expected).is_ok())
    })
    .await;

    let room = single_room(&synapse).expect("uma sala só");
    let texts = room_texts(&synapse, &room);
    check_order(&texts, &expected).expect("ordem e uma vez só");
    let raw = synapse.look(|g| g.raw_sends.len());
    assert!(
        raw >= expected.len(),
        "cada mensagem foi tentada pelo menos uma vez"
    );
    process.crash();
    drop(synapse);
}

#[tokio::test]
async fn poison_message_is_isolated_and_queue_continues() {
    let synapse = Synapse::no_ar();
    synapse.configure(|g| g.reject_text = Some("VENENO".to_owned()));
    let base = Base::new("veneno");
    base.ligar();

    let mut expected = Vec::new();
    base.client(&format!("primeira {}", marker(0)));
    expected.push(marker(0));
    base.client(&format!("VENENO {}", marker(1)));
    base.client(&format!("depois {}", marker(2)));
    expected.push(marker(2));
    base.agent_speaks(&format!("resposta {}", marker(3)));
    expected.push(marker(3));

    let process = module_process(&base, config_matrix(&synapse.endereco, 50));
    wait_until(
        "as mensagens boas chegarem e a venenosa for isolada",
        || {
            let failures = base.store.matrix().count_failures(base.org).unwrap_or(0);
            if failures != 1 {
                return false;
            }
            single_room(&synapse)
                .is_some_and(|room| check_order(&room_texts(&synapse, &room), &expected).is_ok())
        },
    )
    .await;

    let room = single_room(&synapse).expect("uma sala");
    let texts = room_texts(&synapse, &room);
    assert!(
        texts.iter().all(|text| !text.contains("VENENO")),
        "a mensagem venenosa não entrou na sala"
    );
    check_order(&texts, &expected).expect("as boas saem na ordem");

    let failures = base
        .store
        .matrix()
        .list_failures(base.org, 10)
        .expect("falhas");
    assert_eq!(failures.len(), 1, "uma mensagem isolada");
    let reason = failures[0].motivo.clone();
    assert!(
        !reason.contains("VENENO") && !reason.contains("depois"),
        "o motivo isolado é código curto, sem texto de cliente"
    );
    process.crash();
    drop(synapse);
}

#[tokio::test]
async fn crash_during_room_creation_does_not_duplicate_room() {
    let synapse = Synapse::no_ar();
    let base = Base::new("c1");
    base.ligar();
    let config = config_matrix(&synapse.endereco, 50);
    let expected: Vec<String> = (0..3).map(marker).collect();
    base.client(&format!("primeira {}", marker(0)));
    base.agent_speaks(&format!("resposta {}", marker(1)));
    base.client(&format!("segunda {}", marker(2)));

    synapse.configure(|g| g.hold = Some(Hold::new(Target::CreateRoom, 1, None)));
    let first = module_process(&base, config.clone());
    wait_until("o módulo pedir a criação da sala", || {
        synapse.look(|g| g.chegou)
    })
    .await;
    first.crash();

    assert_eq!(
        synapse.look(|g| g.rooms.len()),
        1,
        "o Synapse criou uma sala"
    );
    let line = base
        .store
        .matrix()
        .room(base.org, base.conversation)
        .expect("linha");
    assert!(
        line.is_none_or(|room| !matches!(room.state, StateRoom::Ready)),
        "a linha ainda não estava pronta na queda"
    );

    synapse.configure(|g| g.hold = None);
    let second = module_process(&base, config);
    wait_until("as três mensagens na sala", || {
        single_room(&synapse)
            .is_some_and(|room| check_order(&room_texts(&synapse, &room), &expected).is_ok())
    })
    .await;
    second.crash();

    let rooms = synapse.look(|g| g.rooms.len());
    assert_eq!(rooms, 1, "uma sala só depois da retomada");
    let aliases = synapse.look(|g| g.aliases.len());
    assert_eq!(aliases, 1, "um alias só");
    let room = single_room(&synapse).expect("sala");
    let recorded = base
        .store
        .matrix()
        .room(base.org, base.conversation)
        .expect("linha")
        .expect("a conversa tem sala");
    assert_eq!(
        recorded.room_id, room,
        "a linha aponta para a sala do Synapse"
    );
    assert!(
        matches!(recorded.state, StateRoom::Ready),
        "a linha termina pronta"
    );
    drop(synapse);
}

#[tokio::test]
async fn crash_during_mirror_send_does_not_duplicate() {
    let synapse = Synapse::no_ar();
    let base = Base::new("c3");
    base.ligar();
    let config = config_matrix(&synapse.endereco, 50);
    base.client(&format!("inicial {}", marker(0)));
    base.agent_speaks(&format!("resposta {}", marker(1)));

    let first = module_process(&base, config.clone());
    wait_until("duas mensagens iniciais na sala", || {
        single_room(&synapse).is_some_and(|room| room_texts(&synapse, &room).len() == 2)
    })
    .await;
    first.crash();

    base.client(&format!("nova {}", marker(2)));
    synapse.configure(|g| g.hold = Some(Hold::new(Target::Send, 1, Some("[m02]"))));
    let second = module_process(&base, config.clone());
    wait_until("o envio da mensagem nova chegar ao Synapse", || {
        synapse.look(|g| g.chegou)
    })
    .await;
    second.crash();

    synapse.configure(|g| g.hold = None);
    let third = module_process(&base, config);
    let expected: Vec<String> = (0..3).map(marker).collect();
    wait_until("a mensagem nova reenviada e na sala", || {
        let resends = synapse.look(|g| {
            g.raw_sends
                .iter()
                .filter(|(_, text)| text.contains("[m02]"))
                .count()
        });
        resends >= 2
            && single_room(&synapse)
                .is_some_and(|room| check_order(&room_texts(&synapse, &room), &expected).is_ok())
    })
    .await;
    third.crash();

    let tentativas = synapse.look(|g| {
        g.raw_sends
            .iter()
            .filter(|(_, text)| text.contains("[m02]"))
            .count()
    });
    assert!(
        tentativas >= 2,
        "a mensagem foi reenviada depois da queda ({tentativas} tentativas)"
    );
    let room = single_room(&synapse).expect("sala");
    let texts = room_texts(&synapse, &room);
    check_order(&texts, &expected).expect("uma vez só, na ordem");
    drop(synapse);
}

#[tokio::test]
async fn crash_mid_invite_batch_finishes_without_loss() {
    let synapse = Synapse::no_ar();
    let config = config_matrix(&synapse.endereco, 50);
    let base = Base::new("c2");
    base.ligar();
    let dados = build_space(&base, &synapse, &config);

    synapse.configure(|g| g.hold = Some(Hold::new(Target::Invite, 3, None)));
    let of_process = dados.clone();
    let db = base.db();
    let rag = base.rag();
    let first = Hospedeiro::boot("posicionar", move |recebe| async move {
        let store = Arc::new(Store::open_at(&db, &rag).expect("banco do processo"));
        tokio::select! {
            _ = position_with(&of_process, store, batch_limits()) => {}
            _ = recebe => {}
        }
    });
    wait_until("o terceiro convite chegar ao Synapse", || {
        synapse.look(|g| g.chegou)
    })
    .await;
    first.crash();

    synapse.configure(|g| g.hold = None);
    let resultado = position_with(&dados, Arc::clone(&base.store), batch_limits())
        .await
        .expect("passagem seguinte");
    assert!(
        matches!(resultado, Position::Positioned { .. }),
        "a passagem seguinte põe a sala no espaço: {resultado:?}"
    );
    let guests = room_invitees(&synapse, &dados.room);
    for human in &dados.humans {
        assert!(
            guests.contains(human),
            "{human} ficou sem convite depois da queda"
        );
    }
    assert_eq!(
        guests.len(),
        dados.humans.len(),
        "nenhum convite fora do espaço"
    );
    drop(synapse);
}

#[tokio::test]
async fn crash_before_reply_record_writes_once() {
    let synapse = Synapse::no_ar();
    let base = Base::new("c4a");
    base.ligar();
    let config = config_matrix(&synapse.endereco, 50);
    let (_conversation, room) = room_ready_for_reply(&base);
    let porta = free_port();
    let listener = bind_port(porta);
    let body = reply_transaction(&room, "$evento-c4a", "olá, o pedido saiu hoje");

    synapse.configure(|g| g.hold = Some(Hold::new(Target::Profile, 1, None)));
    let first = receiver_process(&base, config.clone(), listener);
    let send = tokio::spawn(send_transaction(porta, "TXN-C4A".to_owned(), body.clone()));
    wait_until("o módulo ler o perfil do atendente", || {
        synapse.look(|g| g.chegou)
    })
    .await;
    first.crash();
    let _ = send.await;

    assert_eq!(
        pending_queue(&base),
        0,
        "antes do registro, nada entrou na fila"
    );

    synapse.configure(|g| g.hold = None);
    let second = receiver_process(&base, config, bind_port(porta));
    let (status, _) = send_transaction(porta, "TXN-C4A".to_owned(), body).await;
    assert_eq!(status, StatusCode::OK, "a retransmissão é aceita");
    second.crash();

    assert_eq!(pending_queue(&base), 1, "uma resposta na fila");
    assert_eq!(human_replies(&base), 1, "uma mensagem do atendente gravada");
    drop(synapse);
}

#[tokio::test]
async fn retransmission_after_crash_does_not_make_second_reply() {
    let synapse = Synapse::no_ar();
    let base = Base::new("c4b");
    base.ligar();
    let config = config_matrix(&synapse.endereco, 50);
    let (_conversation, room) = room_ready_for_reply(&base);
    let porta = free_port();
    let body = reply_transaction(&room, "$evento-c4b", "pedido enviado");

    let first = receiver_process(&base, config.clone(), bind_port(porta));
    let (status, _) = send_transaction(porta, "TXN-C4B".to_owned(), body.clone()).await;
    assert_eq!(status, StatusCode::OK);
    first.crash();

    let second = receiver_process(&base, config, bind_port(porta));
    let (status, _) = send_transaction(porta, "TXN-C4B".to_owned(), body).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a retransmissão é aceita depois da queda"
    );
    second.crash();

    assert_eq!(
        pending_queue(&base),
        1,
        "a retransmissão não cria segunda entrada na fila"
    );
    assert_eq!(human_replies(&base), 1, "uma mensagem do atendente gravada");
    drop(synapse);
}

#[tokio::test]
async fn failed_invite_in_batch_is_redone_next_pass() {
    let synapse = Synapse::no_ar();
    let config = config_matrix(&synapse.endereco, 50);
    let base = Base::new("convite-falho");
    base.ligar();
    let dados = build_space(&base, &synapse, &config);
    synapse.configure(|g| g.fail_invite = Some(2));

    let first = position_with(&dados, Arc::clone(&base.store), batch_limits())
        .await
        .expect("primeira passagem");
    assert!(
        matches!(
            first,
            Position::Positioned {
                failed_invitations: 1,
                ..
            }
        ),
        "a primeira passagem registra um convite falho"
    );
    let _ = position_with(&dados, Arc::clone(&base.store), batch_limits())
        .await
        .expect("segunda passagem");

    let guests = room_invitees(&synapse, &dados.room);
    for human in &dados.humans {
        assert!(
            guests.contains(human),
            "{human} não foi convidado: a passagem seguinte não refez o convite falho"
        );
    }
    drop(synapse);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    NoModule,
    SynapseDown,
    SynapseLento,
    SynapseNoAr,
}

impl Mode {
    const ALL: [Mode; 4] = [
        Mode::NoModule,
        Mode::SynapseDown,
        Mode::SynapseLento,
        Mode::SynapseNoAr,
    ];

    fn label(self) -> &'static str {
        match self {
            Mode::NoModule => "sem_modulo",
            Mode::SynapseDown => "modulo_synapse_fora",
            Mode::SynapseLento => "modulo_synapse_lento_5s",
            Mode::SynapseNoAr => "modulo_synapse_no_ar",
        }
    }
}

fn recorded_replies(store: &Store, org: OrgId) -> usize {
    let Ok(conversas) = store.conversations().list(org, None, Page::with_limit(50)) else {
        return 0;
    };
    conversas
        .iter()
        .map(|conversation| {
            store
                .messages()
                .list(org, conversation.id, Page::with_limit(1000))
                .map(|messages| {
                    messages
                        .iter()
                        .filter(|message| message.direction == Direction::Outbound)
                        .count()
                })
                .unwrap_or(0)
        })
        .sum()
}

struct Amostra {
    recorded_ms: f64,
    send_ms: f64,
}

async fn measure_round(mode: Mode, round: usize, samples: usize) -> Vec<Amostra> {
    let base = Base::new(&format!("med-{}-{round}", mode.label()));
    let mut synapse: Option<Synapse> = None;
    let config = match mode {
        Mode::NoModule => None,
        _ => {
            base.ligar();
            let mut sinapse = Synapse::em(free_port());
            match mode {
                Mode::SynapseDown => {}
                Mode::SynapseLento => {
                    sinapse.boot();
                    sinapse.configure(|g| g.delay = Duration::from_secs(5));
                }
                _ => sinapse.boot(),
            }
            let config = config_matrix(&sinapse.endereco, 500);
            synapse = Some(sinapse);
            Some(config)
        }
    };
    let servidor = boot_server(&base, config, StartupSettings::defaults(), samples * 4).await;
    let mut resultado = Vec::with_capacity(samples);
    for i in 0..samples {
        let before = Instant::now();
        let status = servidor
            .webhook(&format!("MSG-MED-{round}-{i}"), &format!("pergunta {i}"))
            .await;
        assert_eq!(status, StatusCode::OK, "webhook aceito");
        let wait_start = Instant::now();
        while recorded_replies(&base.store, base.org) < i + 1 {
            assert!(
                wait_start.elapsed() < Duration::from_secs(30),
                "a resposta {} não foi gravada",
                i + 1
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let recorded = Instant::now();
        let arrival = wait_send(&servidor, i).await;
        resultado.push(Amostra {
            recorded_ms: recorded.duration_since(before).as_secs_f64() * 1000.0,
            send_ms: arrival.duration_since(before).as_secs_f64() * 1000.0,
        });
    }
    let calls = servidor.llm_calls();
    println!(
        "{} rodada {round}: {calls} chamadas ao modelo falso para {samples} mensagens",
        mode.label()
    );
    servidor.shutdown().await;
    drop(synapse);
    resultado
}

#[tokio::test]
#[ignore = "medição: rodar por scripts/measure-resilience.sh"]
async fn measure_customer_first() {
    let rounds: usize = std::env::var("MX29_ROUNDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3);
    let samples: usize = std::env::var("MX29_SAMPLES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20);
    let saida = std::env::var("MX29_CSV")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("mx29-medicao.csv"));

    let mut csv = String::from("modo,rodada,amostra,gravada_ms,envio_ms\n");
    let mut summary = Vec::new();
    for mode in Mode::ALL {
        let mut recorded = Vec::new();
        let mut sends = Vec::new();
        for round in 1..=rounds {
            let round_samples = measure_round(mode, round, samples).await;
            for (i, amostra) in round_samples.iter().enumerate() {
                csv.push_str(&format!(
                    "{},{round},{i},{:.3},{:.3}\n",
                    mode.label(),
                    amostra.recorded_ms,
                    amostra.send_ms
                ));
            }
            let g: Vec<f64> = round_samples.iter().map(|a| a.recorded_ms).collect();
            let e: Vec<f64> = round_samples.iter().map(|a| a.send_ms).collect();
            let (mg, pg, _, _) = statistics(&g);
            let (me, pe, _, _) = statistics(&e);
            println!(
                "{} rodada {round}: gravada mediana {mg:.1} ms p95 {pg:.1} ms | envio mediana {me:.1} ms p95 {pe:.1} ms",
                mode.label()
            );
            recorded.extend(g);
            sends.extend(e);
        }
        let (mg, pg, _, xg) = statistics(&recorded);
        let (me, pe, _, xe) = statistics(&sends);
        summary.push(format!(
            "{}: n={} | gravada mediana={mg:.1} p95={pg:.1} max={xg:.1} | envio mediana={me:.1} p95={pe:.1} max={xe:.1} (ms)",
            mode.label(),
            recorded.len()
        ));
    }
    fs::write(&saida, csv).expect("grava o CSV da medição");
    println!("--- resumo ---");
    for line in &summary {
        println!("{line}");
    }
    println!("CSV: {}", saida.display());
}

#[derive(Clone)]
struct SpaceData {
    org: OrgId,
    conversation: ConversationId,
    agent: AgentId,
    room: String,
    space: String,
    department: DepartmentId,
    department_name: String,
    humans: Vec<String>,
    url: String,
    config: MatrixConfig,
}

fn batch_limits() -> InviteLimits {
    InviteLimits::with(2, 1, Duration::ZERO, 100)
}

fn build_space(base: &Base, synapse: &Synapse, config: &MatrixConfig) -> SpaceData {
    let ids = GhostIds::of_config(config).expect("ids");
    let bot = ids.bot().user_id().to_owned();
    let department = base
        .store
        .departments()
        .create(
            base.org,
            NewDepartment {
                name: "Vendas".to_owned(),
                description: String::new(),
            },
        )
        .expect("cria o departamento")
        .id;
    let humans: Vec<String> = (0..5).map(|i| format!("@humano{i}:{SERVIDOR}")).collect();
    let space = format!("!espaco-mx29:{SERVIDOR}");
    let room = format!("!sala-mx29:{SERVIDOR}");
    let mut in_space: Vec<&str> = vec![bot.as_str()];
    in_space.extend(humans.iter().map(String::as_str));
    synapse.seed_room(
        &space,
        &in_space,
        vec![(
            ("m.room.power_levels", ""),
            json!({ "users": { bot.clone(): 100 } }),
        )],
    );
    synapse.seed_room(&room, &[], Vec::new());
    base.store
        .matrix()
        .ensure_room(base.org, base.conversation, &room, None)
        .expect("linha da sala");
    base.store
        .matrix()
        .mark_room_ready(base.org, base.conversation)
        .expect("sala pronta");
    SpaceData {
        org: base.org,
        conversation: base.conversation,
        agent: base.agent,
        room,
        space,
        department,
        department_name: "Vendas".to_owned(),
        humans,
        url: synapse.endereco.clone(),
        config: config.clone(),
    }
}

async fn position_with(
    dados: &SpaceData,
    store: Arc<Store>,
    limites: InviteLimits,
) -> Result<Position, poph_matrix::spaces::SpaceError> {
    let ids = GhostIds::of_config(&dados.config).expect("ids");
    let agent: Ghost = ids
        .agent(&dados.agent.to_string())
        .expect("fantasma do agente");
    let spaces =
        Spaces::with(matrix_customer(&dados.url), ids, SpaceStorePort(store)).with_limits(limites);
    let room = RoomForSpace {
        org: dados.org,
        conversation: dados.conversation,
        room: dados.room.clone(),
        agent,
        department: Some(DepartmentSpace {
            id: dados.department,
            name: dados.department_name.clone(),
        }),
        existing: Some(dados.space.clone()),
    };
    spaces.place(&room).await
}

fn room_invitees(synapse: &Synapse, room: &str) -> Vec<String> {
    synapse.look(|g| {
        g.rooms
            .get(room)
            .map(|target| target.guests.clone())
            .unwrap_or_default()
    })
}

fn room_ready_for_reply(base: &Base) -> (ConversationId, String) {
    let room = format!("!resposta-mx29:{SERVIDOR}");
    base.store
        .matrix()
        .ensure_room(base.org, base.conversation, &room, None)
        .expect("linha da sala");
    base.store
        .matrix()
        .mark_room_ready(base.org, base.conversation)
        .expect("sala pronta");
    (base.conversation, room)
}

fn reply_transaction(room: &str, event: &str, text: &str) -> Value {
    json!({
        "events": [{
            "type": "m.room.message",
            "room_id": room,
            "event_id": event,
            "sender": format!("@ana:{SERVIDOR}"),
            "content": { "msgtype": "m.text", "body": text }
        }]
    })
}

async fn send_transaction(endereco: SocketAddr, txn: String, body: Value) -> (StatusCode, Value) {
    let url = format!("http://{endereco}/_matrix/app/v1/transactions/{txn}");
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("cliente HTTP");
    match http
        .put(url)
        .header(AUTHORIZATION, format!("Bearer {HS_TOKEN}"))
        .json(&body)
        .send()
        .await
    {
        Ok(reply) => {
            let status = reply.status();
            let value = reply.json::<Value>().await.unwrap_or(Value::Null);
            (status, value)
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Value::Null),
    }
}

fn pending_queue(base: &Base) -> u64 {
    base.store
        .outbox()
        .count_by_state(base.org, OutboxState::Pending)
        .expect("conta a fila")
}

fn human_replies(base: &Base) -> usize {
    base.store
        .messages()
        .list(base.org, base.conversation, Page::with_limit(100))
        .expect("mensagens")
        .into_iter()
        .filter(|message| message.origin == Origin::Human)
        .count()
}
