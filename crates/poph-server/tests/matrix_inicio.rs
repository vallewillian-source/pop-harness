use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_matrix::config::{MatrixModule, MatrixSecrets};
use poph_matrix::rooms::METADATA_KIND;
use poph_server::matrix_beginning::StartupSettings;
use poph_server::{ServerState, matrix_beginning};
use poph_store::{
    ConversationStatus, Direction, FilterConversations, NewAgent, NewConversation, NewDepartment,
    NewMessage, NewOrg, Origin, OutboxState, Page, Role, Store,
};
use poph_types::{
    AgentId, ConversationId, DepartmentId, HarnessConfig, MatrixConfig, OrgId, Secret,
};
use serde_json::{Value, json};

const AS_TOKEN: &str = "CANARIO-AS-mx20-nao-pode-vazar";
const HS_TOKEN: &str = "CANARIO-HS-mx20-nao-pode-vazar";
const SERVIDOR: &str = "matrix-dev.local";
const PREFIX: &str = "ph";

#[derive(Default, Clone)]
struct FakeRoom {
    name: Option<String>,
    tipo: Option<String>,
    members: Vec<String>,
    guests: Vec<String>,
    state: BTreeMap<(String, String), Value>,
    messages: Vec<(String, String, Value)>,
}

#[derive(Default)]
struct FakeState {
    users: BTreeMap<String, Option<String>>,
    children_written: u32,
    rooms: BTreeMap<String, FakeRoom>,
    aliases: BTreeMap<String, String>,
    next_room: u32,
    fail_send: bool,
    versions: u32,
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
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                saida.push(byte);
                i += 3;
                continue;
            }
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

async fn attend(
    State(state): State<Arc<Mutex<FakeState>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let mut guard = state.lock().expect("trava do Synapse falso");

    let authorization = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if authorization != format!("Bearer {AS_TOKEN}") {
        return respond(
            401,
            json!({ "errcode": "M_UNKNOWN_TOKEN", "error": "Token inválido" }),
        );
    }

    if uri.path() == "/_matrix/client/versions" {
        guard.versions += 1;
        return respond(200, json!({ "versions": ["v1.11"] }));
    }

    let path = uri.path().trim_start_matches("/_matrix/client/v3");
    let parts: Vec<String> = path
        .split('/')
        .filter(|piece| !piece.is_empty())
        .map(decode)
        .collect();
    let raw_query = uri.query().unwrap_or_default().to_owned();
    let who = query(&raw_query, "user_id").unwrap_or_default();

    match (method.as_str(), parts.as_slice()) {
        ("POST", [registro]) if registro == "register" => {
            let localpart = body["username"].as_str().unwrap_or_default();
            let id = format!("@{localpart}:{SERVIDOR}");
            if guard.users.contains_key(&id) {
                return respond(
                    400,
                    json!({ "errcode": "M_USER_IN_USE", "error": "User ID already taken." }),
                );
            }
            guard.users.insert(id.clone(), None);
            respond(200, json!({ "user_id": id }))
        }
        ("GET", [profile, id]) if profile == "profile" => match guard.users.get(id) {
            Some(name) => {
                let mut reply = json!({});
                if let Some(name) = name {
                    reply["displayname"] = Value::String(name.clone());
                }
                respond(200, reply)
            }
            None => respond(
                404,
                json!({ "errcode": "M_NOT_FOUND", "error": "Usuário não existe" }),
            ),
        },
        ("PUT", [profile, id, campo]) if profile == "profile" && campo == "displayname" => {
            guard
                .users
                .insert(id.clone(), body["displayname"].as_str().map(str::to_owned));
            respond(200, json!({}))
        }
        ("POST", [creation]) if creation == "createRoom" => {
            let alias = body["room_alias_name"]
                .as_str()
                .map(|localpart| format!("#{localpart}:{SERVIDOR}"));
            if let Some(alias) = &alias
                && guard.aliases.contains_key(alias)
            {
                return respond(
                    400,
                    json!({ "errcode": "M_ROOM_IN_USE", "error": "Room alias already taken." }),
                );
            }
            guard.next_room += 1;
            let room = format!("!{}:{SERVIDOR}", guard.next_room);
            let mut members = Vec::new();
            if !who.is_empty() {
                members.push(who.clone());
            }
            guard.rooms.insert(
                room.clone(),
                FakeRoom {
                    name: body["name"].as_str().map(str::to_owned),
                    tipo: body["creation_content"]["type"].as_str().map(str::to_owned),
                    members,
                    ..FakeRoom::default()
                },
            );
            if let Some(alias) = alias {
                guard.aliases.insert(alias, room.clone());
            }
            respond(200, json!({ "room_id": room }))
        }
        ("GET", [diretorio, room, alias]) if diretorio == "directory" && room == "room" => {
            match guard.aliases.get(alias) {
                Some(room) => respond(200, json!({ "room_id": room })),
                None => respond(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Alias não encontrado" }),
                ),
            }
        }
        ("POST", [rooms, room, invite]) if rooms == "rooms" && invite == "invite" => {
            match guard.rooms.get_mut(room) {
                Some(target) => {
                    let invited = body["user_id"].as_str().unwrap_or_default().to_owned();
                    if !target.guests.contains(&invited) {
                        target.guests.push(invited);
                    }
                    respond(200, json!({}))
                }
                None => respond(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("POST", [enter, target]) if enter == "join" => {
            let room = if target.starts_with('#') {
                guard.aliases.get(target).cloned()
            } else {
                Some(target.clone())
            };
            match room.and_then(|room| guard.rooms.get_mut(&room).map(|target| (room, target))) {
                Some((room, target)) => {
                    if !target.members.contains(&who) {
                        target.members.push(who.clone());
                    }
                    respond(200, json!({ "room_id": room }))
                }
                None => respond(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("GET", [rooms, room, members]) if rooms == "rooms" && members == "joined_members" => {
            match guard.rooms.get(room) {
                Some(target) => {
                    let together: BTreeMap<String, Value> = target
                        .members
                        .iter()
                        .map(|id| (id.clone(), json!({})))
                        .collect();
                    respond(200, json!({ "joined": together }))
                }
                None => respond(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("PUT", [rooms, room, state, tipo, chave @ ..]) if rooms == "rooms" && state == "state" => {
            let chave = chave.join("/");
            if tipo == "m.space.child" && !body.as_object().is_none_or(|body| body.is_empty()) {
                guard.children_written += 1;
            }
            match guard.rooms.get_mut(room) {
                Some(target) => {
                    target.state.insert((tipo.clone(), chave), body);
                    respond(200, json!({ "event_id": "$estado" }))
                }
                None => respond(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("GET", [rooms, room, state]) if rooms == "rooms" && state == "state" => {
            match guard.rooms.get(room) {
                Some(target) => {
                    let events: Vec<Value> = target
                    .state
                    .iter()
                    .map(|((tipo, chave), body)| json!({ "type": tipo, "state_key": chave, "content": body }))
                    .collect();
                    respond(200, Value::Array(events))
                }
                None => respond(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("GET", [rooms, room, state, tipo, chave @ ..]) if rooms == "rooms" && state == "state" => {
            let chave = chave.join("/");
            match guard.rooms.get(room) {
                Some(target) => match target.state.get(&(tipo.clone(), chave)) {
                    Some(body) => respond(200, body.clone()),
                    None => respond(
                        404,
                        json!({ "errcode": "M_NOT_FOUND", "error": "Evento não existe" }),
                    ),
                },
                None => respond(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("PUT", [rooms, room, send, tipo, txn]) if rooms == "rooms" && send == "send" => {
            if guard.fail_send {
                return respond(
                    503,
                    json!({ "errcode": "M_UNKNOWN", "error": "indisponível" }),
                );
            }
            match guard.rooms.get_mut(room) {
                Some(target) => {
                    if !target.messages.iter().any(|(ja, _, _)| ja == txn) {
                        target.messages.push((txn.clone(), tipo.clone(), body));
                    }
                    respond(200, json!({ "event_id": format!("$msg_{txn}") }))
                }
                None => respond(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        _ => respond(
            404,
            json!({ "errcode": "M_UNRECOGNIZED", "error": "Rota não existe no Synapse falso" }),
        ),
    }
}

struct Synapse {
    endereco: String,
    state: Arc<Mutex<FakeState>>,
    desligar: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Synapse {
    async fn boot() -> Self {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let app = Router::new()
            .fallback(attend)
            .with_state(Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = format!("http://{}", listener.local_addr().expect("endereço"));
        let (desligar, signal) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = signal.await;
                })
                .await;
        });
        Self {
            endereco,
            state,
            desligar: Some(desligar),
        }
    }

    fn rooms(&self) -> Vec<(String, FakeRoom)> {
        self.state
            .lock()
            .expect("trava")
            .rooms
            .iter()
            .map(|(id, room)| (id.clone(), room.clone()))
            .collect()
    }

    fn room_state(&self, room: &str, tipo: &str, chave: &str) -> Option<Value> {
        self.state
            .lock()
            .expect("trava")
            .rooms
            .get(room)
            .and_then(|room| {
                room.state
                    .get(&(tipo.to_owned(), chave.to_owned()))
                    .cloned()
            })
    }

    fn messages(&self, room: &str) -> Vec<String> {
        self.state
            .lock()
            .expect("trava")
            .rooms
            .get(room)
            .map(|room| {
                room.messages
                    .iter()
                    .map(|(txn, _, _)| txn.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn message_bodies(&self) -> Vec<Value> {
        self.state
            .lock()
            .expect("trava")
            .rooms
            .values()
            .flat_map(|room| room.messages.iter().map(|(_, _, body)| body.clone()))
            .collect()
    }

    fn total_messages(&self) -> usize {
        self.state
            .lock()
            .expect("trava")
            .rooms
            .values()
            .map(|room| room.messages.len())
            .sum()
    }

    fn versions(&self) -> u32 {
        self.state.lock().expect("trava").versions
    }

    fn children_written(&self) -> u32 {
        self.state.lock().expect("trava").children_written
    }

    fn fail_send(&self, fail: bool) {
        self.state.lock().expect("trava").fail_send = fail;
    }
}

impl Drop for Synapse {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
    }
}

struct Scenario {
    dir: PathBuf,
    store: Arc<Store>,
    config: HarnessConfig,
    clock: Arc<AtomicI64>,
}

impl Scenario {
    fn new(label: &str, homeserver_url: String) -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "poph-matrix-inicio-{label}-{}-{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = dir.join("dados");
        config.matrix = Some(MatrixConfig {
            enabled: true,
            homeserver_url,
            server_name: SERVIDOR.to_owned(),
            prefix: PREFIX.to_owned(),
            concurrency: 4,
            interval_ms: 50,
            ..MatrixConfig::default()
        });
        config.validar().expect("configuração válida");

        let clock = Arc::new(AtomicI64::new(1_000_000));
        let own = Arc::clone(&clock);
        let store = Arc::new(
            Store::open(&config)
                .expect("abre o banco")
                .with_clock(Arc::new(move || own.fetch_add(10, Ordering::SeqCst))),
        );
        Self {
            dir,
            store,
            config,
            clock,
        }
    }

    fn state(&self) -> ServerState {
        ServerState::new(Arc::clone(&self.store), self.config.clone()).expect("estado do servidor")
    }

    fn module(&self) -> MatrixModule {
        MatrixModule {
            config: self.config.matrix.clone().expect("seção [matrix]"),
            secrets: MatrixSecrets {
                as_token: Secret::new(AS_TOKEN.to_owned()),
                hs_token: Secret::new(HS_TOKEN.to_owned()),
            },
        }
    }

    fn org(&self, slug: &str) -> OrgId {
        self.store
            .orgs()
            .create(NewOrg {
                name: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id
    }

    fn agent(&self, org: OrgId) -> AgentId {
        self.store
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

    fn department(&self, org: OrgId, name: &str) -> DepartmentId {
        self.store
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

    fn agent_with_department(&self, org: OrgId, department: DepartmentId) -> AgentId {
        self.store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Bia".to_owned(),
                    slug: "bia".to_owned(),
                    default_department: Some(department),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id
    }

    fn conversation(&self, org: OrgId, agent: AgentId, peer: &str) -> ConversationId {
        self.store
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

    fn message(&self, org: OrgId, conversation: ConversationId, text: &str) -> String {
        self.clock.fetch_add(10, Ordering::SeqCst);
        self.store
            .messages()
            .write(
                org,
                NewMessage {
                    conversation_id: conversation,
                    direction: Direction::Outbound,
                    role: Role::Assistant,
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
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

async fn wait_until(descricao: &str, mut condition: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("tempo esgotado esperando: {descricao}");
}

async fn serve(state: ServerState) -> (String, tokio::sync::oneshot::Sender<()>) {
    let app = poph_server::router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = format!("http://{}", listener.local_addr().expect("endereço"));
    let (desligar, signal) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = signal.await;
            })
            .await;
    });
    (endereco, desligar)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("cliente HTTP")
}

async fn search(url: &str, token: Option<&str>) -> (StatusCode, String) {
    let mut request = client().get(url);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let reply = request.send().await.expect("pedido HTTP");
    let status = reply.status();
    (status, reply.text().await.unwrap_or_default())
}

#[tokio::test]
async fn module_off_starts_no_tasks_nor_metrics() {
    let scenario = Scenario::new("desligado", "http://127.0.0.1:9".to_owned());
    for section in [None, Some(MatrixConfig::default())] {
        let mut config = scenario.config.clone();
        config.matrix = section;
        let mut state = ServerState::new(Arc::clone(&scenario.store), config).expect("estado");

        let running = matrix_beginning::start(&mut state);
        assert!(
            running.is_none(),
            "módulo desligado não pode render tarefa do módulo"
        );
        assert!(
            state.matrix_as.is_none(),
            "sem receptor injetado, as rotas do Synapse não existem"
        );
        assert!(
            state.matrix_connected().is_none(),
            "módulo desligado não tem estado de conexão para a API ler"
        );
    }
}

#[tokio::test]
async fn module_off_answers_404_on_application_service() {
    let scenario = Scenario::new("desligado-http", "http://127.0.0.1:9".to_owned());
    let mut config = scenario.config.clone();
    config.matrix = None;
    let state = ServerState::new(Arc::clone(&scenario.store), config).expect("estado");
    let (base, desligar) = serve(state).await;

    let target = format!("{base}/_matrix/app/v1/users/@ph_wa_5511999999999:{SERVIDOR}");
    let (status, _) = search(&target, Some(HS_TOKEN)).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "módulo desligado: nenhuma rota do Application Service"
    );

    let (status, body) = search(&format!("{base}/metrics"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !body.contains("poph_matrix_"),
        "módulo desligado não pode publicar métrica do módulo"
    );
    let _ = desligar.send(());
}

#[tokio::test]
async fn synapse_down_at_boot_does_not_block_serve() {
    let scenario = Scenario::new("fora", "http://127.0.0.1:9".to_owned());
    let mut state = scenario.state();
    let running = matrix_beginning::start_with(&mut state, scenario.module())
        .expect("módulo ligado sobe mesmo com o Synapse fora");
    assert!(state.matrix_as.is_some(), "receptor injetado");

    let (base, desligar) = serve(state.clone()).await;
    let (status, _) = search(&format!("{base}/healthz"), None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "o servidor atende com o Synapse fora"
    );

    let target = format!("{base}/_matrix/app/v1/users/@ph_wa_5511999999999:{SERVIDOR}");
    let (status, _) = search(&target, Some(HS_TOKEN)).await;
    assert_eq!(status, StatusCode::OK);

    assert!(
        !running.connected(),
        "sem Synapse do outro lado, o módulo está degradado"
    );
    assert_eq!(
        state.matrix_connected(),
        Some(false),
        "o estado do módulo diz 'degradado' para a API administrativa"
    );
    running
        .with_shutdown_timeout(Duration::from_secs(2))
        .stop()
        .await;
    let _ = desligar.send(());
}

#[tokio::test]
async fn probe_marks_connected_and_back_to_degraded() {
    let synapse = Synapse::boot().await;
    let scenario = Scenario::new("sonda", synapse.endereco.clone());
    let mut state = scenario.state();
    let settings = StartupSettings {
        probe_period: Duration::from_millis(50),
        probe_delay_cap: Duration::from_millis(200),
        shutdown_timeout: Duration::from_secs(2),
    };
    let running = matrix_beginning::start_with_settings(&mut state, scenario.module(), settings)
        .expect("módulo ligado");

    wait_until("a sonda marcar conectado", || running.connected()).await;
    assert!(synapse.versions() >= 1, "a sonda fala com o Synapse");
    assert_eq!(state.matrix_connected(), Some(true));

    drop(synapse);
    wait_until("a sonda marcar degradado depois da queda", || {
        !running.connected()
    })
    .await;

    running
        .with_shutdown_timeout(Duration::from_secs(2))
        .stop()
        .await;
}

struct Scene {
    scenario: Scenario,
    org: OrgId,
    conversation: ConversationId,
    first: String,
}

fn scene(label: &str, endereco: String) -> Scene {
    let scenario = Scenario::new(label, endereco);
    let org = scenario.org("a");
    let agent = scenario.agent(org);
    let conversation = scenario.conversation(org, agent, "5511999999999");
    scenario
        .store
        .conversations()
        .set_customer_name(org, conversation, "Ana Souza")
        .expect("grava o nome do cliente");
    scenario.store.matrix().enable(org).expect("liga a org");
    let first = scenario.message(org, conversation, "primeira resposta do agente");
    Scene {
        scenario,
        org,
        conversation,
        first,
    }
}

#[tokio::test]
async fn room_created_uses_customer_name_and_mirrors_message() {
    let synapse = Synapse::boot().await;
    let scene = scene("nome", synapse.endereco.clone());
    let mut state = scene.scenario.state();
    let running =
        matrix_beginning::start_with(&mut state, scene.scenario.module()).expect("módulo ligado");

    wait_until("o Synapse receber a mensagem", || {
        synapse.total_messages() >= 1
    })
    .await;

    let (room, fake) = synapse.rooms().pop().expect("a sala foi criada");
    assert_eq!(
        fake.name.as_deref(),
        Some("Ana Souza"),
        "a sala nasce com o nome do cliente, não com o telefone"
    );
    assert_eq!(
        fake.tipo.as_deref(),
        Some(poph_matrix::rooms::ROOM_KIND),
        "a sala nasce com o tipo de atendimento que o Element reconhece"
    );
    assert!(
        synapse
            .messages(&room)
            .contains(&format!("poph-msg-{}", scene.first)),
        "a mensagem do Harness chegou à sala"
    );
    let metadata = synapse
        .room_state(&room, METADATA_KIND, "")
        .expect("metadata escrito");
    assert_eq!(metadata["client_name"], json!("Ana Souza"));
    assert_eq!(
        metadata["conversation_id"],
        json!(scene.conversation.to_string())
    );

    let recorded = scene
        .scenario
        .store
        .matrix()
        .room(scene.org, scene.conversation)
        .expect("lê a sala")
        .expect("linha da sala");
    assert_eq!(
        recorded.state,
        poph_store::StateRoom::Ready,
        "a linha fica pronta depois da criação"
    );
    assert_eq!(recorded.room_id, room);

    running
        .with_shutdown_timeout(Duration::from_secs(2))
        .stop()
        .await;
}

#[tokio::test]
async fn room_created_after_state_pass_joins_department_space() {
    let synapse = Synapse::boot().await;
    let scenario = Scenario::new("espaco", synapse.endereco.clone());
    let org = scenario.org("a");
    let department = scenario.department(org, "Comercial");
    let agent = scenario.agent_with_department(org, department);
    let conversation = scenario.conversation(org, agent, "5511988887777");
    scenario.store.matrix().enable(org).expect("liga a org");

    let mut state = scenario.state();
    let running =
        matrix_beginning::start_with(&mut state, scenario.module()).expect("módulo ligado");

    wait_until("o laço de estado passar pela conversa sem sala", || {
        let cursors = scenario.store.matrix().read_cursors(org).expect("cursores");
        cursors.conversations.id == conversation.to_string()
    })
    .await;
    assert_eq!(
        scenario
            .store
            .matrix()
            .room(org, conversation)
            .expect("lê a sala"),
        None,
        "o laço de estado não cria sala: quem cria é o espelho"
    );

    let first = scenario.message(org, conversation, "primeira resposta do agente");
    wait_until("o Synapse receber a mensagem", || {
        synapse.total_messages() >= 1
    })
    .await;

    let (room, _) = synapse
        .rooms()
        .into_iter()
        .find(|(id, _)| !synapse.messages(id).is_empty())
        .expect("a sala da conversa foi criada");
    assert!(
        synapse
            .messages(&room)
            .contains(&format!("poph-msg-{first}"))
    );

    let space = scenario
        .store
        .matrix()
        .space(org, department)
        .expect("lê o espaço")
        .expect("o espaço do departamento foi criado");
    let child = synapse
        .room_state(&space.room_id, "m.space.child", &room)
        .expect("m.space.child no espaço do departamento");
    assert!(
        child["via"].is_array(),
        "o filho leva `via`, que é o que o Element usa para desenhar a sala sob o espaço: {child}"
    );
    let recorded = scenario
        .store
        .matrix()
        .room(org, conversation)
        .expect("lê a sala")
        .expect("linha da sala");
    assert_eq!(
        recorded.space.as_deref(),
        Some(space.room_id.as_str()),
        "o vínculo da sala com o espaço fica gravado"
    );
    let writes = synapse.children_written();
    assert_eq!(writes, 1, "o `m.space.child` foi escrito uma vez");

    let second = scenario.message(org, conversation, "segunda resposta do agente");
    wait_until("a segunda mensagem ser espelhada", || {
        synapse
            .messages(&room)
            .contains(&format!("poph-msg-{second}"))
    })
    .await;
    assert_eq!(
        synapse.children_written(),
        writes,
        "sala já posicionada não escreve `m.space.child` de novo"
    );

    running
        .with_shutdown_timeout(Duration::from_secs(2))
        .stop()
        .await;
}

#[tokio::test]
async fn turning_org_off_stops_its_mirror_without_restart() {
    let synapse = Synapse::boot().await;
    let scene = scene("org", synapse.endereco.clone());
    let mut state = scene.scenario.state();
    let running =
        matrix_beginning::start_with(&mut state, scene.scenario.module()).expect("módulo ligado");

    wait_until("a primeira mensagem ser espelhada", || {
        synapse.total_messages() >= 1
    })
    .await;
    let before = synapse.total_messages();

    scene
        .scenario
        .store
        .matrix()
        .disable(scene.org)
        .expect("desliga");
    scene.scenario.message(
        scene.org,
        scene.conversation,
        "mensagem com a org desligada",
    );
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        synapse.total_messages(),
        before,
        "org desligada não espelha mais nada"
    );

    scene
        .scenario
        .store
        .matrix()
        .enable(scene.org)
        .expect("religa");
    scene
        .scenario
        .message(scene.org, scene.conversation, "mensagem depois de religar");
    wait_until("a mensagem nova sair depois de religar", || {
        synapse.total_messages() > before
    })
    .await;

    running
        .with_shutdown_timeout(Duration::from_secs(2))
        .stop()
        .await;
}

#[tokio::test]
async fn ordered_shutdown_keeps_pending_message() {
    let synapse = Synapse::boot().await;
    let scene = scene("parada", synapse.endereco.clone());
    synapse.fail_send(true);

    let mut state = scene.scenario.state();
    let running =
        matrix_beginning::start_with(&mut state, scene.scenario.module()).expect("módulo ligado");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        synapse.total_messages(),
        0,
        "nada entregue enquanto o envio falha"
    );
    let cursor = scene
        .scenario
        .store
        .matrix()
        .read_cursors(scene.org)
        .expect("cursor")
        .messages;
    assert_eq!(cursor, 0, "o cursor não anda sem sucesso");

    running
        .with_shutdown_timeout(Duration::from_secs(5))
        .stop()
        .await;

    synapse.fail_send(false);
    let mut state = scene.scenario.state();
    let running =
        matrix_beginning::start_with(&mut state, scene.scenario.module()).expect("módulo ligado");
    wait_until("a mensagem pendente sair na subida seguinte", || {
        synapse.total_messages() >= 1
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(synapse.total_messages(), 1, "uma entrega só, sem duplicar");
    running
        .with_shutdown_timeout(Duration::from_secs(2))
        .stop()
        .await;
}

#[tokio::test]
async fn attendant_reply_is_wired_in_server() {
    let synapse = Synapse::boot().await;
    let scene = scene("resposta", synapse.endereco.clone());
    let mut state = scene.scenario.state();
    let running =
        matrix_beginning::start_with(&mut state, scene.scenario.module()).expect("módulo ligado");
    wait_until("a sala da conversa existir", || !synapse.rooms().is_empty()).await;
    let (room, _) = synapse.rooms().pop().expect("sala criada");

    let (base, desligar) = serve(state).await;
    let body = json!({
        "events": [{
            "type": "m.room.message",
            "room_id": room,
            "sender": "@ana:matrix-dev.local",
            "event_id": "$humano-1",
            "content": { "msgtype": "m.text", "body": "oi, aqui é a Ana do atendimento" }
        }]
    })
    .to_string();
    let reply = client()
        .put(format!("{base}/_matrix/app/v1/transactions/9"))
        .bearer_auth(HS_TOKEN)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("pedido HTTP");
    assert_eq!(reply.status(), StatusCode::OK);

    wait_until("a resposta entrar na fila de saída", || {
        scene
            .scenario
            .store
            .outbox()
            .count_by_state(scene.org, OutboxState::Pending)
            .map(|total| total >= 1)
            .unwrap_or(false)
    })
    .await;
    let conversation = scene
        .scenario
        .store
        .conversations()
        .get(scene.org, scene.conversation)
        .expect("lê a conversa");
    assert_eq!(
        conversation.status,
        ConversationStatus::Transferred,
        "a resposta do atendente pausa o agente"
    );

    let _ = desligar.send(());
    running
        .with_shutdown_timeout(Duration::from_secs(2))
        .stop()
        .await;
}

#[tokio::test]
async fn attendant_file_is_wired_in_server() {
    let synapse = Synapse::boot().await;
    let scene = scene("arquivo", synapse.endereco.clone());
    let mut state = scene.scenario.state();
    let running =
        matrix_beginning::start_with(&mut state, scene.scenario.module()).expect("módulo ligado");
    wait_until("a sala da conversa existir", || !synapse.rooms().is_empty()).await;
    let (room, _) = synapse.rooms().pop().expect("sala criada");

    let (base, desligar) = serve(state).await;
    let body = json!({
        "events": [{
            "type": "m.room.message",
            "room_id": room,
            "sender": "@ana:matrix-dev.local",
            "event_id": "$midia-1",
            "content": { "msgtype": "m.image", "body": "foto.png" }
        }]
    })
    .to_string();
    let reply = client()
        .put(format!("{base}/_matrix/app/v1/transactions/10"))
        .bearer_auth(HS_TOKEN)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("pedido HTTP");
    assert_eq!(reply.status(), StatusCode::OK);

    wait_until("o aviso da mídia aparecer na sala", || {
        synapse
            .message_bodies()
            .iter()
            .any(|body| body["msgtype"] == json!("m.notice"))
    })
    .await;
    let notice = synapse
        .message_bodies()
        .into_iter()
        .find(|body| body["msgtype"] == json!("m.notice"))
        .expect("o aviso está na sala");
    assert_eq!(
        notice["body"],
        json!("Não consegui enviar o arquivo ao cliente: o evento não trouxe o arquivo"),
        "a pasta de blobs do servidor está ligada no receptor"
    );

    let _ = desligar.send(());
    running
        .with_shutdown_timeout(Duration::from_secs(2))
        .stop()
        .await;
}

fn created_room_metadata(synapse: &Synapse) -> Option<(String, Value)> {
    let (room, _) = synapse.rooms().into_iter().next()?;
    let metadata = synapse.room_state(&room, METADATA_KIND, "")?;
    Some((room, metadata))
}

#[tokio::test]
async fn metadata_carries_external_peer_without_spaces_on_create_and_close() {
    let synapse = Synapse::boot().await;
    let scenario = Scenario::new("peer-espacos", synapse.endereco.clone());
    let org = scenario.org("a");
    let agent = scenario.agent(org);
    let conversation = scenario.conversation(org, agent, "  5511999999999 ");
    scenario.store.matrix().enable(org).expect("liga a org");
    scenario.message(org, conversation, "primeira resposta do agente");

    let mut state = scenario.state();
    let running =
        matrix_beginning::start_with(&mut state, scenario.module()).expect("módulo ligado");

    wait_until("a sala criada com metadata", || {
        created_room_metadata(&synapse).is_some()
    })
    .await;
    let (room, created) = created_room_metadata(&synapse).expect("sala e metadata");
    assert_eq!(
        created["client_identifier"],
        json!("5511999999999"),
        "na criação, o identificador sai sem espaços nas pontas"
    );

    let current = scenario
        .store
        .conversations()
        .get(org, conversation)
        .expect("conversa");
    scenario
        .store
        .conversations()
        .close_audited(org, conversation, current.version, "teste".to_owned())
        .expect("fecha a conversa");
    wait_until("o metadata reescrito no fechamento", || {
        synapse
            .room_state(&room, METADATA_KIND, "")
            .is_some_and(|metadata| metadata["status"] == json!("closed"))
    })
    .await;
    let closed = synapse
        .room_state(&room, METADATA_KIND, "")
        .expect("metadata do fechamento");
    assert_eq!(
        closed["client_identifier"],
        json!("5511999999999"),
        "no fechamento, o identificador continua o mesmo"
    );

    running
        .with_shutdown_timeout(Duration::from_secs(2))
        .stop()
        .await;
}

#[tokio::test]
async fn conversation_without_peer_uses_conversation_id_in_metadata() {
    let synapse = Synapse::boot().await;
    let scenario = Scenario::new("sem-peer", synapse.endereco.clone());
    let org = scenario.org("a");
    let agent = scenario.agent(org);
    let conversation = scenario
        .store
        .conversations()
        .create(
            org,
            NewConversation {
                agent_id: agent,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: None,
                target_department: None,
            },
        )
        .expect("cria a conversa sem cliente")
        .id;
    scenario.store.matrix().enable(org).expect("liga a org");
    scenario.message(org, conversation, "primeira resposta do agente");

    let mut state = scenario.state();
    let running =
        matrix_beginning::start_with(&mut state, scenario.module()).expect("módulo ligado");

    wait_until("a sala criada com metadata", || {
        created_room_metadata(&synapse).is_some()
    })
    .await;
    let (_, created) = created_room_metadata(&synapse).expect("sala e metadata");
    assert_eq!(
        created["client_identifier"],
        json!(conversation.to_string()),
        "sem external_peer, o identificador é o id da conversa (a listagem usa o mesmo valor)"
    );

    running
        .with_shutdown_timeout(Duration::from_secs(2))
        .stop()
        .await;
}

#[tokio::test]
async fn listing_by_customer_finds_only_metadata_identifier_conversation() {
    let synapse = Synapse::boot().await;
    let scenario = Scenario::new("listagem", synapse.endereco.clone());
    let org_a = scenario.org("a");
    let agent_a = scenario.agent(org_a);
    let conversation_a = scenario.conversation(org_a, agent_a, "5511888888888");
    let org_b = scenario.org("b");
    let agent_b = scenario.agent(org_b);
    let conversation_b = scenario.conversation(org_b, agent_b, "5511888888888");
    scenario.store.matrix().enable(org_a).expect("liga a org");
    scenario.message(org_a, conversation_a, "primeira resposta do agente");

    let mut state = scenario.state();
    let running =
        matrix_beginning::start_with(&mut state, scenario.module()).expect("módulo ligado");

    wait_until("a sala criada com metadata", || {
        created_room_metadata(&synapse).is_some()
    })
    .await;
    let (_, created) = created_room_metadata(&synapse).expect("sala e metadata");
    let identifier = created["client_identifier"]
        .as_str()
        .expect("identificador em texto")
        .to_owned();
    assert_eq!(identifier, "5511888888888");

    let filter = FilterConversations {
        status: None,
        external_peer: Some(identifier),
        ..Default::default()
    };
    let of_a: Vec<String> = scenario
        .store
        .conversations()
        .list_filter(org_a, &filter, Page::default())
        .expect("lista a org A")
        .iter()
        .map(|conversation| conversation.id.to_string())
        .collect();
    assert_eq!(
        of_a,
        vec![conversation_a.to_string()],
        "a listagem da org A acha exatamente a conversa da sala"
    );
    let of_b: Vec<String> = scenario
        .store
        .conversations()
        .list_filter(org_b, &filter, Page::default())
        .expect("lista a org B")
        .iter()
        .map(|conversation| conversation.id.to_string())
        .collect();
    assert_eq!(
        of_b,
        vec![conversation_b.to_string()],
        "a listagem da org B acha só a conversa dela, com o mesmo telefone"
    );

    running
        .with_shutdown_timeout(Duration::from_secs(2))
        .stop()
        .await;
}
