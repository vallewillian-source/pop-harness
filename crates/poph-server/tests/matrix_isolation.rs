use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use axum::Router;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_evolution::webhook::SECRET_HEADER;
use poph_matrix::{ClientConfig, GhostIds, MatrixClient, Spaces, VirtualUsers};
use poph_server::evolution::EvolutionChannel;
use poph_server::matrix_beginning::SpaceStorePort;
use poph_server::matrix_state::{ConversationStates, StateStorePort, SynapseTarget};
use poph_server::{AsReceiver, AttendantResponse, HarnessResponsePort, ResponsePort, ServerState};
use poph_store::{
    ConversationStatus, HumanResponse, NewAgent, NewConversation, NewDepartment, NewHumanResponse,
    NewOrg, Origin, Page, Store,
};
use poph_types::config::GlobalTokenConfig;
use poph_types::{
    AgentId, ConversationId, DepartmentId, HarnessConfig, IntegrationId, MatrixConfig, OrgId,
    Secret,
};
use rand_core::OsRng;
use reqwest::{Client, Method as HttpMethod};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tracing_subscriber::fmt::MakeWriter;

const PREFIX: &str = "ph";
const SERVER: &str = "matrix-dev.local";
const INSTANCE: &str = "loja";

const AS_TOKEN: &str = "CANARIO-AS-mx28-3c1d-TAO-SECRETO";
const HS_TOKEN: &str = "CANARIO-HS-mx28-77a0-TAO-SECRETO";
const TOKEN_GLOBAL: &str = "pophg_teste-global-mx28";

const EVOLUTION_KEY: &str = "chave-evo-mx28-teste";
const WEBHOOK_SECRET: &str = "segredo-webhook-mx28";

const PHONE: &str = "5511999990028";
const JID: &str = "5511999990028@s.whatsapp.net";

const ATTENDANT: &str = "@ana:matrix-dev.local";
const ATTENDANT_NAME: &str = "Ana Atendente";

const ROOM_A: &str = "!sala-mx28-a:matrix-dev.local";
const ROOM_B: &str = "!sala-mx28-b:matrix-dev.local";

const ATTENDANT_TEXT: &str = "CANARIO-TEXTO-mx28-resposta-do-atendente";

const CUSTOMER_A_NAME: &str = "Cliente da Org A";

#[derive(Clone)]
struct LogCapture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogCapture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for LogCapture {
    type Writer = LogCapture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn captured_logs() -> Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let _ = tracing_subscriber::fmt()
            .with_writer(LogCapture(Arc::clone(&buffer)))
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .try_init();
        buffer
    })
    .clone()
}

fn logs_as_text() -> String {
    String::from_utf8_lossy(&captured_logs().lock().unwrap()).into_owned()
}

#[derive(Default, Clone)]
struct FakeRoom {
    name: Option<String>,
    kind: Option<String>,
    members: Vec<String>,
    guests: Vec<String>,
    state: BTreeMap<(String, String), Value>,
    messages: Vec<(String, String, Value)>,
}

#[derive(Default)]
struct FakeSynapseState {
    users: BTreeMap<String, Option<String>>,
    rooms: BTreeMap<String, FakeRoom>,
    aliases: BTreeMap<String, String>,
    next_room: u32,
    versions: u32,
    urls_with_token: Vec<String>,
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
    let mut output = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                output.push(byte);
                i += 3;
                continue;
            }
        }
        output.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn query(raw: &str, name: &str) -> Option<String> {
    raw.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| decode(value))
    })
}

async fn handle_request(
    State(state): State<Arc<Mutex<FakeSynapseState>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let mut guard = state.lock().expect("trava do Synapse falso");
    let url = uri.to_string();
    if url.contains(AS_TOKEN) || url.contains(HS_TOKEN) {
        guard.urls_with_token.push(url);
    }

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
    let sender = query(&raw_query, "user_id").unwrap_or_default();

    match (method.as_str(), parts.as_slice()) {
        ("POST", [record]) if record == "register" => {
            let localpart = body["username"].as_str().unwrap_or_default();
            let id = format!("@{localpart}:{SERVER}");
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
                let mut response = json!({});
                if let Some(name) = name {
                    response["displayname"] = Value::String(name.clone());
                }
                respond(200, response)
            }
            None => respond(
                404,
                json!({ "errcode": "M_NOT_FOUND", "error": "Usuário não existe" }),
            ),
        },
        ("PUT", [profile, id, field]) if profile == "profile" && field == "displayname" => {
            guard
                .users
                .insert(id.clone(), body["displayname"].as_str().map(str::to_owned));
            respond(200, json!({}))
        }
        ("POST", [creation]) if creation == "createRoom" => {
            let alias = body["room_alias_name"]
                .as_str()
                .map(|localpart| format!("#{localpart}:{SERVER}"));
            if let Some(alias) = &alias
                && guard.aliases.contains_key(alias)
            {
                return respond(
                    400,
                    json!({ "errcode": "M_ROOM_IN_USE", "error": "Room alias already taken." }),
                );
            }
            guard.next_room += 1;
            let room = format!("!criada-{}:{SERVER}", guard.next_room);
            let mut members = Vec::new();
            if !sender.is_empty() {
                members.push(sender.clone());
            }
            guard.rooms.insert(
                room.clone(),
                FakeRoom {
                    name: body["name"].as_str().map(str::to_owned),
                    kind: body["creation_content"]["type"].as_str().map(str::to_owned),
                    members,
                    ..FakeRoom::default()
                },
            );
            if let Some(alias) = alias {
                guard.aliases.insert(alias, room.clone());
            }
            respond(200, json!({ "room_id": room }))
        }
        ("GET", [data_directory, room, alias])
            if data_directory == "directory" && room == "room" =>
        {
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
                    let invitee = body["user_id"].as_str().unwrap_or_default().to_owned();
                    if !target.guests.contains(&invitee) {
                        target.guests.push(invitee);
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
                    if !target.members.contains(&sender) {
                        target.members.push(sender.clone());
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
                    let combined: BTreeMap<String, Value> = target
                        .members
                        .iter()
                        .map(|id| (id.clone(), json!({})))
                        .collect();
                    respond(200, json!({ "joined": combined }))
                }
                None => respond(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("PUT", [rooms, room, state, kind, key @ ..]) if rooms == "rooms" && state == "state" => {
            let key = key.join("/");
            match guard.rooms.get_mut(room) {
                Some(target) => {
                    target.state.insert((kind.clone(), key), body);
                    respond(200, json!({ "event_id": "$estado" }))
                }
                None => respond(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("GET", [rooms, room, state, kind, key @ ..]) if rooms == "rooms" && state == "state" => {
            let key = key.join("/");
            match guard.rooms.get(room) {
                Some(target) => match target.state.get(&(kind.clone(), key)) {
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
        ("PUT", [rooms, room, send, kind, txn]) if rooms == "rooms" && send == "send" => {
            match guard.rooms.get_mut(room) {
                Some(target) => {
                    if !target.messages.iter().any(|(already, _, _)| already == txn) {
                        target.messages.push((txn.clone(), kind.clone(), body));
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
    address: String,
    state: Arc<Mutex<FakeSynapseState>>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Synapse {
    async fn start(attendant_name: &str) -> Self {
        let state = Arc::new(Mutex::new(FakeSynapseState::default()));
        {
            let mut guard = state.lock().expect("trava");
            guard
                .users
                .insert(ATTENDANT.to_owned(), Some(attendant_name.to_owned()));
        }
        let app = Router::new()
            .fallback(handle_request)
            .with_state(Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let address = format!("http://{}", listener.local_addr().expect("endereço"));
        let (shutdown, signal) = oneshot::channel();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = signal.await;
                })
                .await;
        });
        Self {
            address,
            state,
            shutdown: Some(shutdown),
        }
    }

    fn register_room(&self, room: &str, inside: &[&str]) {
        let mut guard = self.state.lock().expect("trava");
        guard.rooms.insert(
            room.to_owned(),
            FakeRoom {
                members: inside.iter().map(|id| (*id).to_owned()).collect(),
                ..FakeRoom::default()
            },
        );
    }

    fn spaces(&self) -> Vec<(String, FakeRoom)> {
        self.state
            .lock()
            .expect("trava")
            .rooms
            .iter()
            .filter(|(_, room)| room.kind.as_deref() == Some("m.space"))
            .map(|(id, room)| (id.clone(), room.clone()))
            .collect()
    }

    fn room_state(&self, room: &str, kind: &str, key: &str) -> Option<Value> {
        self.state
            .lock()
            .expect("trava")
            .rooms
            .get(room)
            .and_then(|room| room.state.get(&(kind.to_owned(), key.to_owned())).cloned())
    }
}

impl Drop for Synapse {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

fn hash_argon2(value: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

struct Scenario {
    directory: PathBuf,
    store: Arc<Store>,
    config: HarnessConfig,
    org_a: OrgId,
    conversation_a: ConversationId,
    dept_a: DepartmentId,
    org_b: OrgId,
    conversation_b: ConversationId,
    dept_b: DepartmentId,
    token_a: String,
    token_b: String,
    synapse: Synapse,
}

impl Scenario {
    async fn new(label: &str) -> Self {
        captured_logs();
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let directory = std::env::temp_dir().join(format!(
            "poph-isolamento-matrix-{label}-{}-{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(directory.join("dados")).expect("pasta de dados");

        let synapse = Synapse::start(ATTENDANT_NAME).await;

        let mut config = HarnessConfig::default();
        config.data.directory = directory.join("dados");
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.matrix = Some(MatrixConfig {
            enabled: true,
            homeserver_url: synapse.address.clone(),
            server_name: SERVER.to_owned(),
            prefix: PREFIX.to_owned(),
            concurrency: 4,
            interval_ms: 50,
            ..MatrixConfig::default()
        });
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::open(&config).expect("abre o banco"));

        let a = Self::seed_org_with_conversation(&store, "org-a", "Sala A", "bia-a", ROOM_A);
        let b = Self::seed_org_with_conversation(&store, "org-b", "Sala B", "bia-b", ROOM_B);
        let dept_a = Self::department(&store, a.0, "Comercial");
        let dept_b = Self::department(&store, b.0, "Financeiro");

        synapse.register_room(ROOM_A, &["@ph_ag_agente-a:matrix-dev.local"]);
        synapse.register_room(ROOM_B, &["@ph_ag_agente-b:matrix-dev.local"]);

        let token_a = Self::token(&store, a.0, "mx28-a");
        let token_b = Self::token(&store, b.0, "mx28-b");

        Self {
            directory,
            store,
            config,
            org_a: a.0,
            conversation_a: a.1,
            dept_a,
            org_b: b.0,
            conversation_b: b.1,
            dept_b,
            token_a,
            token_b,
            synapse,
        }
    }

    fn seed_org_with_conversation(
        store: &Store,
        slug: &str,
        agent_name: &str,
        slug_agent: &str,
        room: &str,
    ) -> (OrgId, ConversationId, AgentId) {
        let org = store
            .orgs()
            .create(NewOrg {
                name: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agent = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: agent_name.to_owned(),
                    slug: slug_agent.to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        let integration: IntegrationId = store
            .create_integration(org, agent, "whatsapp".to_owned(), "active".to_owned())
            .expect("cria a integração")
            .id
            .parse()
            .expect("id da integração");
        let conversation = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agent,
                    integration_id: Some(integration),
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some(PHONE.to_owned()),
                    target_department: None,
                },
            )
            .expect("cria a conversa")
            .id;
        store.matrix().enable(org).expect("liga o módulo na org");
        store
            .matrix()
            .ensure_room(org, conversation, room, None)
            .expect("grava a sala");
        store
            .matrix()
            .mark_room_ready(org, conversation)
            .expect("sala pronta");
        (org, conversation, agent)
    }

    fn department(store: &Store, org: OrgId, name: &str) -> DepartmentId {
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

    fn token(store: &Store, org: OrgId, label: &str) -> String {
        let secret = format!("segredo-{label}");
        let created = store
            .tokens()
            .create(Some(org), label, &hash_argon2(&secret))
            .expect("cria o token");
        format!("poph_{}_{secret}", created.id)
    }

    fn state(&self) -> ServerState {
        ServerState::new(Arc::clone(&self.store), self.config.clone()).expect("estado do servidor")
    }

    fn matrix_client(&self) -> MatrixClient {
        let config = self.config.matrix.clone().expect("seção [matrix]");
        MatrixClient::new(
            ClientConfig::new(
                config.homeserver_url.clone(),
                config.server_name.clone(),
                Secret::new(AS_TOKEN.to_owned()),
            )
            .expect("config do cliente"),
        )
        .expect("cliente do Matrix")
    }

    fn ids(&self) -> GhostIds {
        GhostIds::new(PREFIX, SERVER).expect("ids dos fantasmas")
    }

    fn delivery(&self) -> Arc<AttendantResponse> {
        Arc::new(AttendantResponse::new(
            Arc::new(HarnessResponsePort::fresh(Arc::clone(&self.store))),
            Some(self.matrix_client()),
            self.ids(),
        ))
    }

    fn channel(&self, org: OrgId, conversation: ConversationId) -> EvolutionChannel {
        let integracao = self
            .store
            .conversations()
            .get(org, conversation)
            .expect("lê a conversa")
            .integration_id
            .expect("a conversa tem integração");
        EvolutionChannel {
            org,
            instance: INSTANCE.to_owned(),
            integration: integracao,
            secret: Some(Secret::new(WEBHOOK_SECRET.to_owned())),
            client: None,
            attachment_limit: 0,
        }
    }

    fn conversation(&self, org: OrgId) -> ConversationId {
        if org == self.org_a {
            self.conversation_a
        } else {
            self.conversation_b
        }
    }

    fn messages(&self, org: OrgId) -> Vec<poph_store::Mensagem> {
        self.store
            .messages()
            .list(org, self.conversation(org), Page::with_limit(50))
            .expect("lista as mensagens")
    }

    fn status(&self, org: OrgId) -> ConversationStatus {
        self.store
            .conversations()
            .get(org, self.conversation(org))
            .expect("lê a conversa")
            .status
    }

    fn client_name(&self, org: OrgId) -> Option<String> {
        self.store
            .conversations()
            .get(org, self.conversation(org))
            .expect("lê a conversa")
            .customer_name
    }

    fn mirrored_name(&self, org: OrgId) -> Option<String> {
        self.store
            .matrix()
            .room(org, self.conversation(org))
            .expect("lê a sala")
            .expect("a sala existe")
            .name_client_mirrored
    }

    fn pending(&self, org: OrgId) -> u64 {
        self.store
            .outbox()
            .count_by_state(org, poph_store::OutboxState::Pending)
            .expect("conta a fila de saída")
    }

    fn stored_spaces(&self, org: OrgId) -> usize {
        self.store
            .matrix()
            .list_spaces(org)
            .expect("lista os espaços")
            .len()
    }

    async fn run_state_pass(&self) {
        let config = self.config.matrix.clone().expect("seção [matrix]");
        let client = self.matrix_client();
        let ids = self.ids();
        let users = Arc::new(VirtualUsers::new(
            client.clone(),
            ids,
            poph_matrix::DEFAULT_CACHE_CAP,
        ));
        let spaces = Spaces::of_config(
            client.clone(),
            SpaceStorePort(Arc::clone(&self.store)),
            &config,
        )
        .expect("espaços");
        let destination = SynapseTarget::new(client, users, spaces);
        let loop_task = ConversationStates::new(
            StateStorePort::fresh(Arc::clone(&self.store)),
            destination,
            &config,
        )
        .expect("laço de estado");
        loop_task.cycle().await.expect("passagem do estado");
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

async fn serve(state: ServerState) -> (String, oneshot::Sender<()>) {
    let app = poph_server::router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let address = format!("http://{}", listener.local_addr().expect("endereço"));
    let (shutdown, signal) = oneshot::channel();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = signal.await;
            })
            .await;
    });
    (address, shutdown)
}

struct FakeResponse {
    status: StatusCode,
    json: Value,
    text: String,
}

async fn request(
    base: &str,
    method: HttpMethod,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
    if_match: Option<&str>,
) -> FakeResponse {
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("cliente HTTP");
    let mut request = client.request(method, format!("{base}{path}"));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    if let Some(value) = if_match {
        request = request.header("If-Match", value);
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.expect("pedido HTTP");
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let json = serde_json::from_str(&text).unwrap_or(Value::Null);
    FakeResponse { status, json, text }
}

async fn transaction(base: &str, txn: &str, events: Value) -> FakeResponse {
    request_with_header(
        base,
        HttpMethod::PUT,
        &format!("/_matrix/app/v1/transactions/{txn}"),
        Some(("authorization", &format!("Bearer {HS_TOKEN}"))),
        Some(json!({ "events": events })),
    )
    .await
}

async fn request_with_header(
    base: &str,
    method: HttpMethod,
    path: &str,
    header: Option<(&str, &str)>,
    body: Option<Value>,
) -> FakeResponse {
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("cliente HTTP");
    let mut request = client.request(method, format!("{base}{path}"));
    if let Some((name, value)) = header {
        request = request.header(name, value);
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.expect("pedido HTTP");
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let json = serde_json::from_str(&text).unwrap_or(Value::Null);
    FakeResponse { status, json, text }
}

async fn webhook(base: &str, org: OrgId, body: Value) -> FakeResponse {
    request_with_header(
        base,
        HttpMethod::POST,
        &format!("/webhook/evolution/{org}/{INSTANCE}"),
        Some((SECRET_HEADER, WEBHOOK_SECRET)),
        Some(body),
    )
    .await
}

#[tokio::test]
async fn room_event_with_foreign_ids_leaves_other_org_conversation_untouched() {
    let scenario = Scenario::new("as-cruzado").await;
    let state = scenario
        .state()
        .with_evolution_channels(vec![
            scenario.channel(scenario.org_a, scenario.conversation_a),
            scenario.channel(scenario.org_b, scenario.conversation_b),
        ])
        .with_matrix_as(Arc::new(AsReceiver::new(
            PREFIX,
            SERVER,
            Secret::new(HS_TOKEN.to_owned()),
            scenario.delivery(),
        )));
    let (base, shutdown) = serve(state).await;

    let body_do_event = json!({
        "type": "m.room.message",
        "room_id": ROOM_A,
        "sender": ATTENDANT,
        "event_id": "$evento-cruzado:matrix-dev.local",
        "origin_server_ts": 1_795_300_000_000i64,
        "content": {
            "msgtype": "m.text",
            "body": ATTENDANT_TEXT,
            "conversation_id": scenario.conversation_b.to_string(),
            "org_id": scenario.org_b.to_string(),
            "com.sofacoding.conversation_id": scenario.conversation_b.to_string()
        },
        "conversation_id": scenario.conversation_b.to_string(),
        "org_id": scenario.org_b.to_string()
    });
    let response = transaction(&base, "txn-cruzada", json!([body_do_event])).await;
    assert_eq!(
        response.status,
        StatusCode::OK,
        "erro de conteúdo nunca vira 4xx: {}",
        response.text
    );
    assert_eq!(response.json, json!({}));

    let messages_a = scenario.messages(scenario.org_a);
    assert_eq!(messages_a.len(), 1, "a sala de A grava a resposta em A");
    assert_eq!(messages_a[0].origin, Origin::Human);
    assert_eq!(messages_a[0].texto, ATTENDANT_TEXT);
    assert_eq!(
        messages_a[0].external_id.as_deref(),
        Some("$evento-cruzado:matrix-dev.local")
    );
    assert_eq!(
        scenario.pending(scenario.org_a),
        1,
        "a fila de A tem o item"
    );
    assert_eq!(
        scenario.status(scenario.org_a),
        ConversationStatus::Transferred
    );

    assert!(
        scenario.messages(scenario.org_b).is_empty(),
        "o id da conversa de B no corpo não pode gravar em B"
    );
    assert_eq!(
        scenario.pending(scenario.org_b),
        0,
        "a fila de B segue vazia"
    );
    assert_eq!(scenario.status(scenario.org_b), ConversationStatus::Open);

    let port = HarnessResponsePort::fresh(Arc::clone(&scenario.store));
    let cruzada = port.record_response(
        poph_server::matrix_response::RoomConversation {
            org: scenario.org_a,
            conversation: scenario.conversation_b,
        },
        NewHumanResponse {
            event_id: "$evento-cruzado-na-porta:matrix-dev.local",
            raw_text: ATTENDANT_TEXT,
            sent_text: ATTENDANT_TEXT,
            author_id: ATTENDANT,
            author_name: ATTENDANT_NAME,
        },
    );
    assert!(
        matches!(cruzada, Ok(HumanResponse::ConversationMissing)),
        "conversa de outra org é inexistente para esta org: {cruzada:?}"
    );
    assert!(
        scenario.messages(scenario.org_b).is_empty(),
        "a tentativa cruzada não grava em B"
    );

    let _ = shutdown.send(());
}

#[tokio::test]
async fn unknown_room_with_foreign_id_records_metric_without_write() {
    let scenario = Scenario::new("sala-desconhecida").await;
    let delivery = scenario.delivery();
    let state = scenario.state().with_matrix_as(Arc::new(AsReceiver::new(
        PREFIX,
        SERVER,
        Secret::new(HS_TOKEN.to_owned()),
        delivery.clone(),
    )));
    let (base, shutdown) = serve(state).await;

    let response = transaction(
        &base,
        "txn-desconhecida",
        json!([{
            "type": "m.room.message",
            "room_id": "!sala-que-ninguem-conhece:matrix-dev.local",
            "sender": ATTENDANT,
            "event_id": "$evento-orfao:matrix-dev.local",
            "origin_server_ts": 1_795_300_000_000i64,
            "content": {
                "msgtype": "m.text",
                "body": ATTENDANT_TEXT,
                "conversation_id": scenario.conversation_b.to_string()
            },
            "conversation_id": scenario.conversation_b.to_string()
        }]),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::OK,
        "sala desconhecida é 200 sem processar: {}",
        response.text
    );
    assert!(scenario.messages(scenario.org_a).is_empty());
    assert!(scenario.messages(scenario.org_b).is_empty());
    assert_eq!(scenario.pending(scenario.org_a), 0);
    assert_eq!(scenario.pending(scenario.org_b), 0);
    assert_eq!(
        delivery
            .counters()
            .unknown_rooms
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "o caso fica contado"
    );
    let logs = logs_as_text();
    assert!(
        logs.contains("nenhuma org ligada conhece"),
        "o caso fica no log; capturado até agora: {logs}"
    );

    let _ = shutdown.send(());
}

#[tokio::test]
async fn org_token_cannot_use_other_org_matrix_routes() {
    let scenario = Scenario::new("api-cruzada").await;
    scenario
        .store
        .matrix()
        .disable(scenario.org_b)
        .expect("desliga o módulo na org B");
    let mut state = scenario.state();
    let config = scenario.config.matrix.clone().expect("seção [matrix]");
    state.set_matrix_spaces(Arc::new(
        Spaces::of_config(
            scenario.matrix_client(),
            SpaceStorePort(Arc::clone(&scenario.store)),
            &config,
        )
        .expect("espaços"),
    ));
    let (base, shutdown) = serve(state).await;

    let of_b = format!("/admin/v1/orgs/{}/matrix", scenario.org_b);
    let spaces_of_b = format!("{of_b}/spaces");
    let department_of_b = format!("{spaces_of_b}/{}", scenario.dept_b);

    let negados = [
        request(
            &base,
            HttpMethod::GET,
            &of_b,
            Some(&scenario.token_a),
            None,
            None,
        )
        .await,
        request(
            &base,
            HttpMethod::PUT,
            &of_b,
            Some(&scenario.token_a),
            Some(json!({"enabled": true})),
            Some("0"),
        )
        .await,
        request(
            &base,
            HttpMethod::GET,
            &spaces_of_b,
            Some(&scenario.token_a),
            None,
            None,
        )
        .await,
        request(
            &base,
            HttpMethod::PUT,
            &spaces_of_b,
            Some(&scenario.token_a),
            Some(json!({"department_id": scenario.dept_b.to_string()})),
            None,
        )
        .await,
        request(
            &base,
            HttpMethod::DELETE,
            &department_of_b,
            Some(&scenario.token_a),
            None,
            None,
        )
        .await,
    ];
    for (position, response) in negados.iter().enumerate() {
        assert_eq!(
            response.status,
            StatusCode::FORBIDDEN,
            "rota {position} da org B aceitou o token da org A: {}",
            response.text
        );
        assert!(
            !response.text.contains(&scenario.token_a)
                && !response.text.contains(&scenario.token_b),
            "token não aparece na negativa"
        );
    }

    assert!(
        !scenario
            .store
            .matrix()
            .state(scenario.org_b)
            .expect("estado")
            .linked,
        "o ataque não liga o módulo na outra org"
    );
    assert_eq!(scenario.stored_spaces(scenario.org_b), 0);
    assert_eq!(scenario.stored_spaces(scenario.org_a), 0);
    assert_eq!(scenario.status(scenario.org_b), ConversationStatus::Open);

    let of_a = format!("/admin/v1/orgs/{}/matrix", scenario.org_a);
    let b_at_a = request(
        &base,
        HttpMethod::GET,
        &of_a,
        Some(&scenario.token_b),
        None,
        None,
    )
    .await;
    assert_eq!(b_at_a.status, StatusCode::FORBIDDEN);
    let owner = request(
        &base,
        HttpMethod::GET,
        &of_b,
        Some(&scenario.token_b),
        None,
        None,
    )
    .await;
    assert_eq!(owner.status, StatusCode::OK, "{}", owner.text);

    let without_token = request(&base, HttpMethod::GET, &of_b, None, None, None).await;
    assert_eq!(without_token.status, StatusCode::UNAUTHORIZED);

    let _ = shutdown.send(());
}

#[tokio::test]
async fn space_rejects_department_from_other_org() {
    let scenario = Scenario::new("espaco-cruzado").await;
    let mut state = scenario.state();
    let config = scenario.config.matrix.clone().expect("seção [matrix]");
    state.set_matrix_spaces(Arc::new(
        Spaces::of_config(
            scenario.matrix_client(),
            SpaceStorePort(Arc::clone(&scenario.store)),
            &config,
        )
        .expect("espaços"),
    ));
    let (base, shutdown) = serve(state).await;

    let route_a = format!("/admin/v1/orgs/{}/matrix/spaces", scenario.org_a);

    let cruzado = request(
        &base,
        HttpMethod::PUT,
        &route_a,
        Some(&scenario.token_a),
        Some(json!({"department_id": scenario.dept_b.to_string()})),
        None,
    )
    .await;
    assert_eq!(
        cruzado.status,
        StatusCode::NOT_FOUND,
        "departamento de outra org não existe nesta: {}",
        cruzado.text
    );
    assert_eq!(
        scenario.stored_spaces(scenario.org_a),
        0,
        "nada gravado na org do token"
    );
    assert_eq!(
        scenario.stored_spaces(scenario.org_b),
        0,
        "nada gravado na org dona do departamento"
    );
    assert!(
        scenario.synapse.spaces().is_empty(),
        "nenhum espaço criado no Synapse"
    );

    let enabled = request(
        &base,
        HttpMethod::PUT,
        &route_a,
        Some(&scenario.token_a),
        Some(json!({
            "department_id": scenario.dept_b.to_string(),
            "space": "#sala-do-operador:matrix-dev.local"
        })),
        None,
    )
    .await;
    assert_eq!(enabled.status, StatusCode::NOT_FOUND, "{}", enabled.text);

    let disabled = request(
        &base,
        HttpMethod::DELETE,
        &format!("{route_a}/{}", scenario.dept_b),
        Some(&scenario.token_a),
        None,
        None,
    )
    .await;
    assert_eq!(disabled.status, StatusCode::NOT_FOUND, "{}", disabled.text);
    assert!(
        scenario
            .store
            .departments()
            .get(scenario.org_b, scenario.dept_b)
            .is_ok(),
        "o departamento da org B continua existindo"
    );

    let own = request(
        &base,
        HttpMethod::PUT,
        &route_a,
        Some(&scenario.token_a),
        Some(json!({"department_id": scenario.dept_a.to_string()})),
        None,
    )
    .await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.text);
    assert_eq!(
        own.json["department_id"],
        json!(scenario.dept_a.to_string())
    );
    assert_eq!(scenario.stored_spaces(scenario.org_a), 1);
    assert_eq!(
        scenario.stored_spaces(scenario.org_b),
        0,
        "o espaço criado é de A, não de B"
    );
    let spaces = scenario.synapse.spaces();
    assert_eq!(spaces.len(), 1, "um espaço no Synapse, o da org A");
    assert_eq!(
        spaces[0].1.name.as_deref(),
        Some("Comercial"),
        "o espaço criado leva o nome do departamento da org A"
    );

    let _ = shutdown.send(());
}

#[tokio::test]
async fn webhook_customer_name_stays_in_its_org() {
    let scenario = Scenario::new("nome").await;
    let state = scenario.state().with_evolution_channels(vec![
        scenario.channel(scenario.org_a, scenario.conversation_a),
        scenario.channel(scenario.org_b, scenario.conversation_b),
    ]);
    let (base, shutdown) = serve(state).await;

    let received = webhook(
        &base,
        scenario.org_a,
        json!({
            "event": "messages.upsert",
            "instance": INSTANCE,
            "apikey": EVOLUTION_KEY,
            "data": {
                "key": { "id": "MSG-MX28-1", "remoteJid": JID, "fromMe": false },
                "pushName": CUSTOMER_A_NAME,
                "messageType": "conversation",
                "message": { "conversation": "bom dia" },
                "messageTimestamp": 1_795_300_000i64
            }
        }),
    )
    .await;
    assert_eq!(received.status, StatusCode::OK, "{}", received.text);

    assert_eq!(
        scenario.client_name(scenario.org_a).as_deref(),
        Some(CUSTOMER_A_NAME),
        "o nome do cliente entra na conversa da org A"
    );
    assert_eq!(
        scenario.client_name(scenario.org_b),
        None,
        "a conversa da org B não recebe o nome da org A"
    );
    assert_eq!(
        scenario.mirrored_name(scenario.org_b),
        None,
        "a sala da org B não recebe o nome da org A"
    );
    assert_eq!(
        scenario
            .store
            .conversations()
            .get(scenario.org_b, scenario.conversation_b)
            .expect("lê a conversa de B")
            .status,
        ConversationStatus::Open,
        "a mensagem do cliente de A não muda o estado da conversa de B"
    );

    scenario.run_state_pass().await;
    assert_eq!(
        scenario.synapse.room_state(ROOM_A, "m.room.name", ""),
        Some(json!({ "name": CUSTOMER_A_NAME })),
        "a sala de A recebe o nome do cliente dela"
    );
    assert_eq!(
        scenario.synapse.room_state(ROOM_B, "m.room.name", ""),
        None,
        "a sala de B não é renomeada pela mudança de A"
    );
    assert_eq!(
        scenario.mirrored_name(scenario.org_a).as_deref(),
        Some(CUSTOMER_A_NAME),
        "o nome espelhado fica na org A"
    );
    assert_eq!(
        scenario.mirrored_name(scenario.org_b),
        None,
        "o nome espelhado da org B continua vazio"
    );

    let _ = shutdown.send(());
}

fn files_of_directory(directory: &Path) -> Vec<(String, Vec<u8>)> {
    let mut found = Vec::new();
    let mut pilha = vec![directory.to_path_buf()];
    while let Some(data_directory) = pilha.pop() {
        let Ok(entries) = fs::read_dir(&data_directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pilha.push(path);
            } else if let Ok(bytes) = fs::read(&path) {
                found.push((path.display().to_string(), bytes));
            }
        }
    }
    found
}

#[tokio::test]
async fn canary_tokens_are_absent_from_responses_logs_files_and_queue() {
    let scenario = Scenario::new("canario").await;
    let mut state = scenario.state().with_evolution_channels(vec![
        scenario.channel(scenario.org_a, scenario.conversation_a),
        scenario.channel(scenario.org_b, scenario.conversation_b),
    ]);
    let config = scenario.config.matrix.clone().expect("seção [matrix]");
    state.set_matrix_spaces(Arc::new(
        Spaces::of_config(
            scenario.matrix_client(),
            SpaceStorePort(Arc::clone(&scenario.store)),
            &config,
        )
        .expect("espaços"),
    ));
    let state = state.with_matrix_as(Arc::new(AsReceiver::new(
        PREFIX,
        SERVER,
        Secret::new(HS_TOKEN.to_owned()),
        scenario.delivery(),
    )));
    let (base, shutdown) = serve(state).await;

    let of_a = format!("/admin/v1/orgs/{}/matrix", scenario.org_a);
    let of_b = format!("/admin/v1/orgs/{}/matrix", scenario.org_b);
    let mut responses: Vec<FakeResponse> = Vec::new();
    responses.push(
        request(
            &base,
            HttpMethod::GET,
            &of_a,
            Some(&scenario.token_a),
            None,
            None,
        )
        .await,
    );
    responses.push(
        request(
            &base,
            HttpMethod::PUT,
            &of_a,
            Some(&scenario.token_a),
            Some(json!({"enabled": true})),
            Some("999"),
        )
        .await,
    );
    responses.push(
        request(
            &base,
            HttpMethod::GET,
            &of_b,
            Some(&scenario.token_a),
            None,
            None,
        )
        .await,
    );
    responses.push(request(&base, HttpMethod::GET, &of_b, None, None, None).await);
    responses.push(
        transaction(
            &base,
            "txn-canario",
            json!([{
                "type": "m.room.message",
                "room_id": ROOM_A,
                "sender": ATTENDANT,
                "event_id": "$evento-canario:matrix-dev.local",
                "origin_server_ts": 1_795_300_000_000i64,
                "content": { "msgtype": "m.text", "body": ATTENDANT_TEXT }
            }]),
        )
        .await,
    );
    responses.push(
        request_with_header(
            &base,
            HttpMethod::PUT,
            "/_matrix/app/v1/transactions/txn-sem-token",
            Some(("authorization", "Bearer token-errado-de-proposito")),
            Some(json!({"events": []})),
        )
        .await,
    );
    responses.push(
        request_with_header(
            &base,
            HttpMethod::GET,
            &format!("/_matrix/app/v1/users/@{PREFIX}_wa_{PHONE}:{SERVER}"),
            Some(("authorization", &format!("Bearer {HS_TOKEN}"))),
            None,
        )
        .await,
    );
    responses.push(
        request_with_header(
            &base,
            HttpMethod::POST,
            &format!("/webhook/evolution/{}/{INSTANCE}", scenario.org_a),
            Some((SECRET_HEADER, "segredo-errado")),
            Some(json!({"event": "messages.upsert"})),
        )
        .await,
    );
    responses.push(
        webhook(
            &base,
            scenario.org_a,
            json!({
                "event": "messages.upsert",
                "instance": INSTANCE,
                "apikey": EVOLUTION_KEY,
                "data": {
                    "key": { "id": "MSG-MX28-CANARIO", "remoteJid": JID, "fromMe": false },
                    "pushName": "Cliente Canário",
                    "messageType": "conversation",
                    "message": { "conversation": "oi" },
                    "messageTimestamp": 1_795_300_000i64
                }
            }),
        )
        .await,
    );

    assert_eq!(responses[0].status, StatusCode::OK);
    assert_eq!(responses[2].status, StatusCode::FORBIDDEN);
    assert_eq!(responses[3].status, StatusCode::UNAUTHORIZED);
    assert_eq!(responses[4].status, StatusCode::OK);
    assert_eq!(responses[5].status, StatusCode::FORBIDDEN);
    assert_eq!(responses[6].status, StatusCode::OK);
    assert_eq!(responses[7].status, StatusCode::UNAUTHORIZED);

    let forbidden_values = [
        ("as_token", AS_TOKEN),
        ("hs_token", HS_TOKEN),
        ("token global", TOKEN_GLOBAL),
    ];

    let responses_together: String = responses
        .iter()
        .map(|response| response.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    for (label, value) in forbidden_values {
        assert!(
            !responses_together.contains(value),
            "o {label} apareceu numa resposta HTTP"
        );
    }

    let urls = scenario
        .synapse
        .state
        .lock()
        .unwrap()
        .urls_with_token
        .clone();
    assert!(
        urls.is_empty(),
        "token na URL de uma chamada ao Synapse: {urls:?}"
    );
    assert!(
        scenario.synapse.state.lock().unwrap().versions > 0
            || !scenario.synapse.state.lock().unwrap().rooms.is_empty(),
        "o Synapse falso recebeu chamadas (senão a varredura de URL não prova nada)"
    );

    assert!(
        scenario.pending(scenario.org_a) >= 1,
        "a resposta do atendente entrou na fila de A"
    );

    for org in [scenario.org_a, scenario.org_b] {
        for item in scenario
            .store
            .outbox()
            .list_pending(org, 50)
            .expect("lista a fila")
        {
            let text = format!(
                "{} {} {}",
                item.tipo,
                item.content_json,
                item.ultimo_erro.as_deref().unwrap_or("")
            );
            for (label, value) in forbidden_values {
                assert!(
                    !text.contains(value),
                    "o {label} apareceu num item da fila de saída"
                );
            }
        }
    }

    let logs = logs_as_text();
    for (label, value) in forbidden_values {
        assert!(!logs.contains(value), "o {label} apareceu no log do módulo");
    }
    assert!(
        logs.contains("matrix_as") || logs.contains("matrix_resposta"),
        "o log tem linha do módulo (senão a varredura não prova nada)"
    );

    let files = files_of_directory(&scenario.directory);
    assert!(
        !files.is_empty(),
        "a varredura de arquivo precisa ter arquivos para ler"
    );
    for (path, bytes) in files {
        let text = String::from_utf8_lossy(&bytes);
        for (label, value) in forbidden_values {
            assert!(
                !text.contains(value),
                "o {label} apareceu no arquivo {path}"
            );
        }
    }

    let _ = shutdown.send(());
}
