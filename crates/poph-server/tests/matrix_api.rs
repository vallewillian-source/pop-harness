use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use axum::Router;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_matrix::config::MatrixModule;
use poph_matrix::config::MatrixSecrets;
use poph_matrix::spaces::Spaces;
use poph_server::ServerState;
use poph_server::matrix_beginning::{self, ModuleSpaces, SpaceStorePort, StartupSettings};
use poph_store::{
    Direction, NewAgent, NewConversation, NewDepartment, NewMessage, NewOrg, Origin, Role, Store,
};
use poph_types::config::GlobalTokenConfig;
use poph_types::{AgentId, DepartmentId, HarnessConfig, MatrixConfig, OrgId, Secret};
use rand_core::OsRng;
use reqwest::{Client, Method as HttpMethod};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const AS_TOKEN: &str = "CANARIO-AS-mx22-nao-pode-vazar";
const HS_TOKEN: &str = "CANARIO-HS-mx22-nao-pode-vazar";
const SERVIDOR: &str = "matrix-dev.local";
const PREFIX: &str = "ph";
const TOKEN_GLOBAL: &str = "pophg_teste-global-mx22";

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
    rooms: BTreeMap<String, FakeRoom>,
    aliases: BTreeMap<String, String>,
    next_room: u32,
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

fn urlencoding(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '@' => "%40".to_owned(),
            ':' => "%3A".to_owned(),
            other => other.to_string(),
        })
        .collect()
}

fn query(raw: &str, name: &str) -> Option<String> {
    raw.split('&').find_map(|pair| {
        let (chave, value) = pair.split_once('=')?;
        (chave == name).then(|| decode(value))
    })
}

async fn handle(
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
                    let joined: BTreeMap<String, Value> = target
                        .members
                        .iter()
                        .map(|id| (id.clone(), json!({})))
                        .collect();
                    respond(200, json!({ "joined": joined }))
                }
                None => respond(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("PUT", [rooms, room, state, tipo, chave @ ..]) if rooms == "rooms" && state == "state" => {
            let chave = chave.join("/");
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
    desligar: Option<oneshot::Sender<()>>,
}

impl Synapse {
    async fn start() -> Self {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let app = Router::new()
            .fallback(handle)
            .with_state(Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = format!("http://{}", listener.local_addr().expect("endereço"));
        let (desligar, signal) = oneshot::channel();
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

    async fn create_room(&self, as_user: &str) -> String {
        let client = Client::new();
        let response = client
            .post(format!(
                "{}/_matrix/client/v3/createRoom?user_id={}",
                self.endereco,
                urlencoding(as_user)
            ))
            .bearer_auth(AS_TOKEN)
            .json(&json!({"name": "Espaço do operador"}))
            .send()
            .await
            .expect("createRoom no Synapse falso");
        let body: Value = response.json().await.expect("corpo do createRoom");
        body["room_id"].as_str().expect("room_id").to_owned()
    }

    fn space_count(&self) -> usize {
        self.rooms()
            .iter()
            .filter(|(_, room)| room.tipo.as_deref() == Some("m.space"))
            .count()
    }
}

impl Drop for Synapse {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
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
    folder: PathBuf,
    store: Arc<Store>,
    config: HarnessConfig,
    clock: Arc<AtomicI64>,
}

impl Scenario {
    fn new(label: &str, homeserver_url: String) -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let folder = std::env::temp_dir().join(format!(
            "poph-matrix-api-{label}-{}-{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(folder.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = folder.join("dados");
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
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
        let own_clock = Arc::clone(&clock);
        let store = Arc::new(
            Store::open(&config)
                .expect("abre o banco")
                .with_clock(Arc::new(move || own_clock.fetch_add(10, Ordering::SeqCst))),
        );
        Self {
            folder,
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

    fn token(&self, org: OrgId, label: &str) -> String {
        let secret = format!("segredo-{label}");
        let created = self
            .store
            .tokens()
            .create(Some(org), label, &hash_argon2(&secret))
            .expect("cria o token");
        format!("poph_{}_{secret}", created.id)
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

    fn conversation(&self, org: OrgId, agent: AgentId) -> poph_types::ConversationId {
        self.store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agent,
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some("5511999999999".to_owned()),
                    target_department: None,
                },
            )
            .expect("cria a conversa")
            .id
    }

    fn message(&self, org: OrgId, conversation: poph_types::ConversationId, text: &str) -> u64 {
        let message = self
            .store
            .messages()
            .write(
                org,
                NewMessage {
                    conversation_id: conversation,
                    direction: Direction::Inbound,
                    role: Role::User,
                    origin: Origin::Agente,
                    author_name: None,
                    texto: text.to_owned(),
                    attachments_json: "[]".to_owned(),
                    external_id: None,
                    turn_id: None,
                },
            )
            .expect("grava a mensagem");
        self.store
            .messages()
            .since(0, 1000)
            .expect("lê as mensagens")
            .into_iter()
            .find(|read_message| read_message.id == message.id)
            .expect("mensagem recém-gravada")
            .rowid
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

    fn spaces(&self) -> ModuleSpaces {
        Spaces::of_config(
            poph_matrix::MatrixClient::from_parts(&self.module().config, &self.module().secrets)
                .expect("cliente do Matrix"),
            SpaceStorePort(Arc::clone(&self.store)),
            &self.module().config,
        )
        .expect("espaços")
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.folder);
    }
}

async fn serve(state: ServerState) -> (String, oneshot::Sender<()>) {
    let app = poph_server::router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = format!("http://{}", listener.local_addr().expect("endereço"));
    let (desligar, signal) = oneshot::channel();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = signal.await;
            })
            .await;
    });
    (endereco, desligar)
}

struct FakeResponse {
    status: StatusCode,
    json: Value,
    text: String,
    etag: Option<String>,
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
    let mut request_builder = client.request(method, format!("{base}{path}"));
    if let Some(token) = token {
        request_builder = request_builder.bearer_auth(token);
    }
    if let Some(value) = if_match {
        request_builder = request_builder.header("If-Match", value);
    }
    if let Some(body) = body {
        request_builder = request_builder.json(&body);
    }
    let response = request_builder.send().await.expect("pedido HTTP");
    let status = response.status();
    let etag = response
        .headers()
        .get("etag")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let text = response.text().await.unwrap_or_default();
    let json = serde_json::from_str(&text).unwrap_or(Value::Null);
    FakeResponse {
        status,
        json,
        text,
        etag,
    }
}

async fn wait_until(descricao: &str, mut condition: impl FnMut() -> bool) {
    let started_at = Instant::now();
    while started_at.elapsed() < Duration::from_secs(20) {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("tempo esgotado esperando: {descricao}");
}

#[tokio::test]
async fn org_state_reports_backlog_failures_and_etag() {
    let scenario = Scenario::new("estado", "http://127.0.0.1:9".to_owned());
    let org = scenario.org("estado");
    let token = scenario.token(org, "estado");
    let agent = scenario.agent(org);
    let conversation = scenario.conversation(org, agent);
    let first = scenario.message(org, conversation, "primeira");
    scenario.message(org, conversation, "segunda");
    scenario
        .store
        .matrix()
        .advance_messages(org, first)
        .expect("avança o cursor");
    let pending = scenario
        .store
        .messages()
        .since(first, 10)
        .expect("lê a leitura incremental")
        .first()
        .map(|message| message.created_at)
        .expect("mensagem pendente");
    scenario.clock.store(pending + 65_000, Ordering::SeqCst);
    scenario
        .store
        .matrix()
        .register_failure(
            org,
            poph_types::MessageId::generate(),
            "formato_invalido",
            1,
        )
        .expect("registra a falha");
    let other = scenario.org("outra");
    scenario
        .store
        .matrix()
        .register_failure(
            other,
            poph_types::MessageId::generate(),
            "formato_invalido",
            1,
        )
        .expect("falha da outra org");

    let state = scenario.state();
    let (base, desligar) = serve(state).await;
    let route = format!("/admin/v1/orgs/{org}/matrix");
    let response = request(&base, HttpMethod::GET, &route, Some(&token), None, None).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text);
    assert_eq!(response.json["org_id"], json!(org.to_string()));
    assert_eq!(response.json["enabled"], json!(false));
    assert_eq!(response.json["enabled_at"], Value::Null);
    assert_eq!(
        response.json["connected"],
        Value::Null,
        "sem módulo nesta instância, `connected` é nulo"
    );
    assert_eq!(response.json["backlog"]["messages"], json!(1));
    assert_eq!(
        response.json["backlog"]["oldest_seconds"],
        json!(65),
        "a idade é a diferença entre agora e a mensagem mais antiga pendente"
    );
    assert_eq!(response.json["pending_failures"], json!(1));
    assert_eq!(response.json["last_failure"], json!("formato_invalido"));
    let updated_at = response.json["updated_at"].as_i64().expect("updated_at");
    assert_eq!(
        response.etag.as_deref(),
        Some(format!("\"{updated_at}\"").as_str()),
        "o ETag é o updated_at da linha"
    );
    assert!(
        !response.text.contains("5511999999999"),
        "o estado não pode trazer telefone nem texto de cliente"
    );

    let _ = desligar.send(());
}

#[tokio::test]
async fn org_state_says_connected_when_probe_reaches_synapse() {
    let synapse = Synapse::start().await;
    let scenario = Scenario::new("conectado", synapse.endereco.clone());
    let org = scenario.org("conectado");
    let token = scenario.token(org, "conectado");

    let mut state = scenario.state();
    let running = matrix_beginning::start_with_settings(
        &mut state,
        scenario.module(),
        StartupSettings {
            probe_period: Duration::from_millis(20),
            probe_delay_cap: Duration::from_millis(50),
            shutdown_timeout: Duration::from_secs(2),
        },
    )
    .expect("módulo ligado");
    let (base, desligar) = serve(state.clone()).await;
    wait_until("sonda falar com o Synapse falso", || {
        state.matrix_connected() == Some(true)
    })
    .await;

    let route = format!("/admin/v1/orgs/{org}/matrix");
    let response = request(&base, HttpMethod::GET, &route, Some(&token), None, None).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text);
    assert_eq!(
        response.json["connected"],
        json!(true),
        "com o Synapse respondendo, a API diz conectado"
    );
    assert!(
        response.json["backlog"]["messages"].as_u64().is_some(),
        "o atraso é número, não texto"
    );

    running.stop().await;
    let _ = desligar.send(());
}

#[tokio::test]
async fn enabling_and_disabling_require_if_match_and_keep_cutoff() {
    let scenario = Scenario::new("liga", "http://127.0.0.1:9".to_owned());
    let org = scenario.org("liga");
    let token = scenario.token(org, "liga");
    let state = scenario.state();
    let (base, desligar) = serve(state).await;
    let route = format!("/admin/v1/orgs/{org}/matrix");

    let without = request(
        &base,
        HttpMethod::PUT,
        &route,
        Some(&token),
        Some(json!({"enabled": true})),
        None,
    )
    .await;
    assert_eq!(
        without.status,
        StatusCode::PRECONDITION_REQUIRED,
        "{}",
        without.text
    );
    assert_eq!(without.json["code"], json!("precondition_required"));

    let initial = request(&base, HttpMethod::GET, &route, Some(&token), None, None).await;
    assert_eq!(initial.status, StatusCode::OK);
    assert_eq!(initial.json["updated_at"], json!(0));

    let enabled = request(
        &base,
        HttpMethod::PUT,
        &route,
        Some(&token),
        Some(json!({"enabled": true})),
        Some("0"),
    )
    .await;
    assert_eq!(enabled.status, StatusCode::OK, "{}", enabled.text);
    assert_eq!(enabled.json["enabled"], json!(true));
    let cutoff = enabled.json["enabled_at"]
        .as_i64()
        .expect("corte do backfill");
    let etag = enabled.json["updated_at"].as_i64().expect("updated_at");
    assert!(etag > 0);

    let stale = request(
        &base,
        HttpMethod::PUT,
        &route,
        Some(&token),
        Some(json!({"enabled": false})),
        Some("0"),
    )
    .await;
    assert_eq!(
        stale.status,
        StatusCode::PRECONDITION_FAILED,
        "{}",
        stale.text
    );
    assert_eq!(stale.json["code"], json!("version_conflict"));
    assert!(
        scenario.store.matrix().state(org).expect("estado").linked,
        "o 412 não pode desligar a org"
    );

    let again = request(
        &base,
        HttpMethod::PUT,
        &route,
        Some(&token),
        Some(json!({"enabled": true})),
        Some(&etag.to_string()),
    )
    .await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    assert_eq!(
        again.json["enabled_at"],
        json!(cutoff),
        "religar não move o corte"
    );

    let current_etag = again.json["updated_at"].as_i64().expect("updated_at");
    let disabled = request(
        &base,
        HttpMethod::PUT,
        &route,
        Some(&token),
        Some(json!({"enabled": false})),
        Some(&current_etag.to_string()),
    )
    .await;
    assert_eq!(disabled.status, StatusCode::OK, "{}", disabled.text);
    assert_eq!(disabled.json["enabled"], json!(false));
    assert_eq!(
        disabled.json["enabled_at"],
        json!(cutoff),
        "o corte fica gravado"
    );
    assert!(
        !disabled.text.contains(TOKEN_GLOBAL),
        "token global não vaza"
    );
    assert!(!disabled.text.contains(&token), "token de org não vaza");

    let _ = desligar.send(());
}

#[tokio::test]
async fn token_of_other_org_and_missing_org() {
    let scenario = Scenario::new("isolamento", "http://127.0.0.1:9".to_owned());
    let org_a = scenario.org("a");
    let org_b = scenario.org("b");
    let token_a = scenario.token(org_a, "a");
    let state = scenario.state();
    let (base, desligar) = serve(state).await;

    let from_b = format!("/admin/v1/orgs/{org_b}/matrix");
    let without_token = request(&base, HttpMethod::GET, &from_b, None, None, None).await;
    assert_eq!(without_token.status, StatusCode::UNAUTHORIZED);

    let cruzado = request(&base, HttpMethod::GET, &from_b, Some(&token_a), None, None).await;
    assert_eq!(cruzado.status, StatusCode::FORBIDDEN, "{}", cruzado.text);

    let cruzado_put = request(
        &base,
        HttpMethod::PUT,
        &from_b,
        Some(&token_a),
        Some(json!({"enabled": true})),
        Some("0"),
    )
    .await;
    assert_eq!(cruzado_put.status, StatusCode::FORBIDDEN);
    assert!(
        !scenario.store.matrix().state(org_b).expect("estado").linked,
        "o ataque não liga o módulo na outra org"
    );

    let missing = poph_types::OrgId::generate();
    let ghost_route = format!("/admin/v1/orgs/{missing}/matrix");
    let missing_org = request(
        &base,
        HttpMethod::GET,
        &ghost_route,
        Some(TOKEN_GLOBAL),
        None,
        None,
    )
    .await;
    assert_eq!(
        missing_org.status,
        StatusCode::NOT_FOUND,
        "{}",
        missing_org.text
    );

    let ghost_space = format!("/admin/v1/orgs/{missing}/matrix/spaces");
    let without_org = request(
        &base,
        HttpMethod::GET,
        &ghost_space,
        Some(TOKEN_GLOBAL),
        None,
        None,
    )
    .await;
    assert_eq!(without_org.status, StatusCode::NOT_FOUND);

    let own_route = format!("/admin/v1/orgs/{org_a}/matrix");
    let owner = request(
        &base,
        HttpMethod::GET,
        &own_route,
        Some(&token_a),
        None,
        None,
    )
    .await;
    assert_eq!(owner.status, StatusCode::OK);

    let _ = desligar.send(());
}

#[tokio::test]
async fn department_space_is_created_once_enabled_and_disabled() {
    let synapse = Synapse::start().await;
    let scenario = Scenario::new("espacos", synapse.endereco.clone());
    let org = scenario.org("espacos");
    let token = scenario.token(org, "espacos");
    let commercial = scenario.department(org, "Comercial");
    let other = scenario.department(org, "Outro");

    let mut state = scenario.state();
    state.set_matrix_spaces(Arc::new(scenario.spaces()));
    let (base, desligar) = serve(state).await;
    let route = format!("/admin/v1/orgs/{org}/matrix/spaces");

    let created = request(
        &base,
        HttpMethod::PUT,
        &route,
        Some(&token),
        Some(json!({"department_id": commercial.to_string()})),
        None,
    )
    .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text);
    assert_eq!(created.json["department_id"], json!(commercial.to_string()));
    assert_eq!(created.json["department_name"], json!("Comercial"));
    assert_eq!(created.json["origin"], json!("auto"));
    let space = created.json["space_room_id"]
        .as_str()
        .expect("espaço")
        .to_owned();
    assert_eq!(synapse.space_count(), 1);
    let no_synapse = synapse
        .rooms()
        .into_iter()
        .find(|(id, _)| id == &space)
        .expect("o espaço existe no Synapse")
        .1;
    assert_eq!(
        no_synapse.name.as_deref(),
        Some("Comercial"),
        "o espaço criado leva o nome do departamento"
    );
    assert_eq!(no_synapse.tipo.as_deref(), Some("m.space"));

    let again = request(
        &base,
        HttpMethod::PUT,
        &route,
        Some(&token),
        Some(json!({"department_id": commercial.to_string()})),
        None,
    )
    .await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    assert_eq!(again.json["space_room_id"], json!(space));
    assert_eq!(synapse.space_count(), 1, "um espaço só");

    let do_operador = synapse.create_room("@operador:matrix-dev.local").await;
    let without_permission = request(
        &base,
        HttpMethod::PUT,
        &route,
        Some(&token),
        Some(json!({"department_id": other.to_string(), "space": do_operador})),
        None,
    )
    .await;
    assert_eq!(
        without_permission.status,
        StatusCode::CONFLICT,
        "{}",
        without_permission.text
    );
    assert_eq!(without_permission.json["code"], json!("conflict"));
    let message = without_permission.json["message"]
        .as_str()
        .unwrap_or_default();
    assert!(
        message.contains("convite") || message.contains("convite") || message.contains("poder"),
        "o erro diz o que o operador faz no Element: {message}"
    );
    assert!(
        scenario
            .store
            .matrix()
            .space(org, other)
            .expect("lê")
            .is_none(),
        "a falha de permissão não grava espaço"
    );

    let invalid = request(
        &base,
        HttpMethod::PUT,
        &route,
        Some(&token),
        Some(json!({"department_id": other.to_string(), "space": "nem-id-nem-alias"})),
        None,
    )
    .await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST, "{}", invalid.text);

    let list = request(&base, HttpMethod::GET, &route, Some(&token), None, None).await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text);
    let items = list.json["items"].as_array().expect("lista").clone();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["department_id"], json!(commercial.to_string()));
    assert_eq!(items[0]["department_name"], json!("Comercial"));

    let agent = scenario.agent(org);
    let conversation = scenario.conversation(org, agent);
    scenario
        .store
        .matrix()
        .ensure_room(org, conversation, "!sala:matrix-dev.local", Some(&space))
        .expect("sala no espaço");

    let turned_off = request(
        &base,
        HttpMethod::DELETE,
        &format!("{route}/{commercial}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(turned_off.status, StatusCode::OK, "{}", turned_off.text);
    assert_eq!(turned_off.json["removed"], json!(true));
    assert_eq!(turned_off.json["rooms_forgotten"], json!(1));
    assert_eq!(
        synapse.space_count(),
        1,
        "o espaço continua no Synapse (desligar não apaga sala)"
    );
    let after = request(&base, HttpMethod::GET, &route, Some(&token), None, None).await;
    assert_eq!(after.json["items"].as_array().expect("lista").len(), 0);
    assert_eq!(
        scenario
            .store
            .matrix()
            .room(org, conversation)
            .expect("sala")
            .expect("existe")
            .space,
        None,
        "a sala continua existindo, só sai do registro do módulo"
    );

    let other_org = scenario.org("outra-espacos");
    let other_org_department = scenario.department(other_org, "Dela");
    let foreign = request(
        &base,
        HttpMethod::DELETE,
        &format!("{route}/{other_org_department}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(foreign.status, StatusCode::NOT_FOUND, "{}", foreign.text);

    let _ = desligar.send(());
}

#[tokio::test]
async fn module_off_refuses_space_and_reports_null_connected() {
    let scenario = Scenario::new("sem-modulo", "http://127.0.0.1:9".to_owned());
    let org = scenario.org("sem-modulo");
    let token = scenario.token(org, "sem-modulo");
    let department = scenario.department(org, "Comercial");

    let mut config = scenario.config.clone();
    config.matrix = None;
    let state = ServerState::new(Arc::clone(&scenario.store), config).expect("estado sem módulo");
    let (base, desligar) = serve(state).await;

    let route = format!("/admin/v1/orgs/{org}/matrix");
    let response = request(&base, HttpMethod::GET, &route, Some(&token), None, None).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text);
    assert_eq!(
        response.json["connected"],
        Value::Null,
        "sem módulo nesta instância, `connected` é nulo"
    );

    let space = request(
        &base,
        HttpMethod::PUT,
        &format!("{route}/spaces"),
        Some(&token),
        Some(json!({"department_id": department.to_string()})),
        None,
    )
    .await;
    assert_eq!(space.status, StatusCode::CONFLICT, "{}", space.text);
    assert_eq!(space.json["code"], json!("matrix_desligado"));

    let _ = desligar.send(());
}
