use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_matrix::error::Classification;
use poph_matrix::rooms::{
    CREATION_POWER, ConversationToRoom, METADATA_KIND, NOTICE_WITHOUT_HISTORY, PortError,
    Provisioner, ROOM_KIND, RoomStored, RoomsPort, StoredState,
};
use poph_matrix::{ClientConfig, MatrixClient};
use poph_types::{AgentId, ConversationId, DepartmentId, MatrixConfig, OrgId, Secret};
use serde_json::{Value, json};

const CANARIO: &str = "CANARIO-MX11-nao-pode-vazar";

const SERVER: &str = "matrix-dev.local";

#[derive(Clone, Debug)]
struct Request {
    method: String,
    path: String,
    body: Value,
}

#[derive(Default, Clone)]
struct FakeRoom {
    kind: Option<String>,
    name: Option<String>,
    topico: Option<String>,
    criador: String,
    members: BTreeSet<String>,
    guests: BTreeSet<String>,
    state: BTreeMap<(String, String), Value>,
    messages: Vec<(String, String, Value)>,
    power: Option<Value>,
}

#[derive(Default)]
struct FakeState {
    expected_token: String,
    users: BTreeMap<String, Option<String>>,
    rooms: BTreeMap<String, FakeRoom>,
    aliases: BTreeMap<String, String>,
    next_room: u32,
    http_requests: Vec<Request>,
    creations: u32,
    fail_creation: Option<(u16, Value)>,
}

impl FakeState {
    fn records(&self, suffix: &str) -> Vec<&Request> {
        self.http_requests
            .iter()
            .filter(|http_request| http_request.path.ends_with(suffix))
            .collect()
    }

    fn count(&self, suffix: &str) -> usize {
        self.records(suffix).len()
    }
}

fn reply(status: u16, body: Value) -> Response {
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

async fn attend(
    State(state): State<Arc<Mutex<FakeState>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let mut guard = state.lock().expect("trava do Synapse falso");

    guard.http_requests.push(Request {
        method: method.to_string(),
        path: uri.path().to_owned(),
        body: body.clone(),
    });

    let authorization = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if authorization != format!("Bearer {}", guard.expected_token) {
        return reply(
            401,
            json!({ "errcode": "M_UNKNOWN_TOKEN", "error": "Token inválido" }),
        );
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
        ("POST", [record]) if record == "register" => {
            let localpart = body["username"].as_str().unwrap_or_default();
            let id = format!("@{localpart}:{SERVER}");
            if guard.users.contains_key(&id) {
                return reply(
                    400,
                    json!({ "errcode": "M_USER_IN_USE", "error": "User ID already taken." }),
                );
            }
            guard.users.insert(id.clone(), None);
            reply(200, json!({ "user_id": id }))
        }
        ("GET", [profile, id]) if profile == "profile" => match guard.users.get(id) {
            Some(name) => {
                let mut response = json!({});
                if let Some(name) = name {
                    response["displayname"] = Value::String(name.clone());
                }
                reply(200, response)
            }
            None => reply(
                404,
                json!({ "errcode": "M_NOT_FOUND", "error": "Usuário não existe" }),
            ),
        },
        ("PUT", [profile, id, field]) if profile == "profile" && field == "displayname" => {
            guard
                .users
                .insert(id.clone(), body["displayname"].as_str().map(str::to_owned));
            reply(200, json!({}))
        }
        ("POST", [creation]) if creation == "createRoom" => {
            if let Some((status, body)) = guard.fail_creation.take() {
                return reply(status, body);
            }
            if let Some(users) = body["power_level_content_override"]["users"].as_object() {
                let power = users.get(&who).and_then(Value::as_i64).unwrap_or(0);
                if power < 100 {
                    return reply(
                        403,
                        json!({
                            "errcode": "M_FORBIDDEN",
                            "error": format!(
                                "You don't have permission to post that to the room. \
                                 user_level ({power}) < send_level (100)"
                            ),
                        }),
                    );
                }
            }
            let alias = body["room_alias_name"]
                .as_str()
                .map(|localpart| format!("#{localpart}:{SERVER}"));
            if let Some(alias) = &alias
                && guard.aliases.contains_key(alias)
            {
                return reply(
                    400,
                    json!({ "errcode": "M_ROOM_IN_USE", "error": "Room alias already taken." }),
                );
            }
            guard.next_room += 1;
            let room = format!("!{}:{SERVER}", guard.next_room);
            let mut members = BTreeSet::new();
            if !who.is_empty() {
                members.insert(who.clone());
            }
            let mut room_state = BTreeMap::new();
            if let Some(name) = body["name"].as_str() {
                room_state.insert(
                    ("m.room.name".to_owned(), String::new()),
                    json!({ "name": name }),
                );
            }
            if !body["power_level_content_override"].is_null() {
                room_state.insert(
                    ("m.room.power_levels".to_owned(), String::new()),
                    body["power_level_content_override"].clone(),
                );
            }
            guard.rooms.insert(
                room.clone(),
                FakeRoom {
                    kind: body["creation_content"]["type"].as_str().map(str::to_owned),
                    name: body["name"].as_str().map(str::to_owned),
                    topico: body["topic"].as_str().map(str::to_owned),
                    criador: who.clone(),
                    members,
                    state: room_state,
                    power: Some(body["power_level_content_override"].clone()),
                    ..FakeRoom::default()
                },
            );
            if let Some(alias) = alias {
                guard.aliases.insert(alias, room.clone());
            }
            guard.creations += 1;
            reply(200, json!({ "room_id": room }))
        }
        ("GET", [directory, room, alias]) if directory == "directory" && room == "room" => {
            match guard.aliases.get(alias) {
                Some(room) => reply(200, json!({ "room_id": room })),
                None => reply(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Alias não encontrado" }),
                ),
            }
        }
        ("POST", [rooms, room, invitation]) if rooms == "rooms" && invitation == "invite" => {
            match guard.rooms.get_mut(room) {
                Some(target) => {
                    target
                        .guests
                        .insert(body["user_id"].as_str().unwrap_or_default().to_owned());
                    reply(200, json!({}))
                }
                None => reply(
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
                    target.members.insert(who.clone());
                    reply(200, json!({ "room_id": room }))
                }
                None => reply(
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
                    reply(200, json!({ "joined": together }))
                }
                None => reply(
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
                    reply(200, json!({ "event_id": "$estado" }))
                }
                None => reply(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("GET", [rooms, room, state, kind, key @ ..]) if rooms == "rooms" && state == "state" => {
            let key = key.join("/");
            match guard.rooms.get(room) {
                Some(target) => match target.state.get(&(kind.clone(), key)) {
                    Some(body) => reply(200, body.clone()),
                    None => reply(
                        404,
                        json!({ "errcode": "M_NOT_FOUND", "error": "Evento de estado não existe" }),
                    ),
                },
                None => reply(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("PUT", [rooms, room, send, kind, txn]) if rooms == "rooms" && send == "send" => {
            match guard.rooms.get_mut(room) {
                Some(target) => {
                    if !target.messages.iter().any(|(ja, _, _)| ja == txn) {
                        target.messages.push((txn.clone(), kind.clone(), body));
                    }
                    reply(200, json!({ "event_id": format!("$msg_{txn}") }))
                }
                None => reply(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        _ => reply(
            404,
            json!({ "errcode": "M_UNRECOGNIZED", "error": "Rota não existe no Synapse falso" }),
        ),
    }
}

#[derive(Default)]
struct PortState {
    rooms: BTreeMap<(OrgId, ConversationId), RoomStored>,
    history: bool,
    write_failures: u32,
    recordings: usize,
    ready_marks: usize,
    mirrored_names: BTreeMap<(OrgId, ConversationId), Option<String>>,
}

#[derive(Clone, Default)]
struct FakePort(Arc<Mutex<PortState>>);

impl FakePort {
    fn history(&self, value: bool) {
        self.0.lock().expect("trava").history = value;
    }

    fn fall_back_on_next_write(&self, times: u32) {
        self.0.lock().expect("trava").write_failures = times;
    }

    fn room_of(&self, org: OrgId, conversation: ConversationId) -> Option<RoomStored> {
        self.0
            .lock()
            .expect("trava")
            .rooms
            .get(&(org, conversation))
            .cloned()
    }

    fn return_to_creating(&self, org: OrgId, conversation: ConversationId) {
        if let Some(room) = self
            .0
            .lock()
            .expect("trava")
            .rooms
            .get_mut(&(org, conversation))
        {
            room.state = StoredState::Creating;
        }
    }

    fn ready_marks(&self) -> usize {
        self.0.lock().expect("trava").ready_marks
    }

    fn mirrored_name(&self, org: OrgId, conversation: ConversationId) -> Option<Option<String>> {
        self.0
            .lock()
            .expect("trava")
            .mirrored_names
            .get(&(org, conversation))
            .cloned()
    }
}

impl RoomsPort for FakePort {
    async fn room(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Result<Option<RoomStored>, PortError> {
        Ok(self.room_of(org, conversation))
    }

    async fn ensure_room(
        &self,
        org: OrgId,
        conversation: ConversationId,
        room_id: &str,
    ) -> Result<RoomStored, PortError> {
        let mut guard = self.0.lock().expect("trava");
        if guard.write_failures > 0 {
            guard.write_failures -= 1;
            return Err(PortError::fresh("o banco caiu antes de gravar a sala"));
        }
        guard.recordings += 1;
        let room = guard
            .rooms
            .entry((org, conversation))
            .or_insert_with(|| RoomStored {
                room_id: room_id.to_owned(),
                state: StoredState::Creating,
            });
        Ok(room.clone())
    }

    async fn mark_ready(
        &self,
        org: OrgId,
        conversation: ConversationId,
        mirrored_name: Option<&str>,
    ) -> Result<RoomStored, PortError> {
        let mut guard = self.0.lock().expect("trava");
        guard.ready_marks += 1;
        let name = mirrored_name.map(str::to_owned);
        let room = match guard.rooms.get_mut(&(org, conversation)) {
            Some(room) => {
                room.state = StoredState::Ready;
                room.clone()
            }
            None => return Err(PortError::fresh("sala ausente na porta")),
        };
        guard.mirrored_names.insert((org, conversation), name);
        Ok(room)
    }

    async fn had_messages_before(
        &self,
        _org: OrgId,
        _conversation: ConversationId,
        _cutoff: i64,
    ) -> Result<bool, PortError> {
        Ok(self.0.lock().expect("trava").history)
    }
}

struct Scenario {
    state: Arc<Mutex<FakeState>>,
    port: FakePort,
    provisioner: Provisioner<FakePort>,
    givens: ConversationToRoom,
    address: String,
}

impl Scenario {
    fn with_state(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().expect("trava do Synapse falso")
    }

    fn restart(&self) -> Provisioner<FakePort> {
        self.with_power(50)
    }

    fn with_power(&self, power: u8) -> Provisioner<FakePort> {
        Provisioner::with(
            self.client_provisioner(),
            self.user_provisioner(),
            self.port.clone(),
            power,
        )
    }

    fn client_provisioner(&self) -> MatrixClient {
        MatrixClient::new(
            ClientConfig::new(
                self.address.clone(),
                SERVER,
                Secret::new(CANARIO.to_owned()),
            )
            .expect("configuração do cliente"),
        )
        .expect("cliente")
    }

    fn user_provisioner(&self) -> poph_matrix::users::VirtualUsers {
        poph_matrix::users::VirtualUsers::new(
            self.client_provisioner(),
            poph_matrix::users::GhostIds::new("ph", SERVER).expect("ids"),
            64,
        )
    }
}

impl Scenario {
    fn single_room(&self) -> (String, FakeRoom) {
        let guard = self.with_state();
        assert_eq!(guard.rooms.len(), 1, "esperava uma sala só");
        let (id, room) = guard.rooms.iter().next().expect("sala");
        (id.clone(), room.clone())
    }
}

async fn scenario() -> Scenario {
    let state = Arc::new(Mutex::new(FakeState {
        expected_token: CANARIO.to_owned(),
        ..FakeState::default()
    }));
    let app = Router::new()
        .fallback(attend)
        .with_state(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let address = listener.local_addr().expect("endereço local");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("Synapse falso no ar");
    });

    let config = MatrixConfig {
        enabled: true,
        homeserver_url: format!("http://{address}"),
        server_name: SERVER.to_owned(),
        prefix: "ph".to_owned(),
        agent_power: 50,
        ..MatrixConfig::default()
    };
    let client = MatrixClient::new(
        ClientConfig::from_parts(
            &config,
            &poph_matrix::MatrixSecrets {
                as_token: Secret::new(CANARIO.to_owned()),
                hs_token: Secret::new(CANARIO.to_owned()),
            },
        )
        .expect("configuração do cliente"),
    )
    .expect("cliente");
    let port = FakePort::default();
    let provisioner = Provisioner::of_config(client, port.clone(), &config).expect("provisionador");

    let givens = ConversationToRoom {
        org: OrgId::generate(),
        conversation: ConversationId::generate(),
        client: "+55 (11) 99999-9999".to_owned(),
        client_name: Some("Maria".to_owned()),
        agent: AgentId::generate(),
        agent_name: Some("Agente de Teste".to_owned()),
        channel: "whatsapp".to_owned(),
        department: Some(DepartmentId::generate()),
        closed: false,
        backfill_cutoff: None,
    };

    Scenario {
        state,
        port,
        provisioner,
        givens,
        address: format!("http://{address}"),
    }
}

#[tokio::test]
async fn provisioning_creates_room_with_frontend_kind_name_and_metadata() {
    let c = scenario().await;

    let ready = c
        .provisioner
        .provision(&c.givens)
        .await
        .expect("sala pronta");

    let (room_id, room) = c.single_room();
    assert_eq!(ready.room_id, room_id);
    assert!(!ready.was_already_ready);

    assert_eq!(room.kind.as_deref(), Some(ROOM_KIND));
    assert_eq!(room.name.as_deref(), Some("Maria"));
    assert!(
        room.topico
            .as_deref()
            .unwrap_or_default()
            .contains("whatsapp"),
        "{:?}",
        room.topico
    );

    let agent = format!("@ph_ag_{}:{SERVER}", c.givens.agent);
    assert_eq!(
        room.power.as_ref().expect("poder")["users"][&agent],
        json!(CREATION_POWER),
        "poder do agente na criação"
    );
    assert_eq!(
        room.state[&("m.room.power_levels".to_owned(), String::new())]["users"][&agent],
        json!(50),
        "poder final do agente na sala"
    );

    let client = format!("@ph_wa_5511999999999:{SERVER}");
    assert!(room.members.contains(&client), "{:?}", room.members);
    assert!(room.members.contains(&agent), "{:?}", room.members);

    let metadata = room
        .state
        .get(&(METADATA_KIND.to_owned(), String::new()))
        .expect("evento de metadata");
    assert_eq!(
        metadata["conversation_id"],
        json!(c.givens.conversation.to_string())
    );
    assert_eq!(metadata["agent_id"], json!(c.givens.agent.to_string()));
    assert_eq!(metadata["integration_type"], json!("whatsapp"));
    assert_eq!(metadata["channel"], json!("whatsapp"));
    assert_eq!(metadata["client_identifier"], json!("+55 (11) 99999-9999"));
    assert_eq!(metadata["client_name"], json!("Maria"));
    assert_eq!(
        metadata["department_id"],
        json!(c.givens.department.expect("departamento").to_string())
    );
    assert_eq!(metadata["status"], json!("active"));
    let mut keies: Vec<&String> = metadata.as_object().expect("objeto").keys().collect();
    keies.sort();
    assert_eq!(
        keies,
        vec![
            "agent_id",
            "channel",
            "client_identifier",
            "client_name",
            "conversation_id",
            "department_id",
            "integration_type",
            "status",
        ],
        "campo a mais ou a menos no metadata"
    );

    let alias_expected = c.provisioner.alias(c.givens.conversation);
    assert!(alias_expected.starts_with("#ph_c_"), "{alias_expected}");
    let guard = c.with_state();
    assert_eq!(
        guard.aliases.get(&alias_expected).cloned(),
        Some(room_id.clone())
    );

    let creation = guard
        .http_requests
        .iter()
        .find(|http_request| http_request.path.ends_with("/createRoom"))
        .expect("createRoom");
    assert_eq!(
        creation.body["room_alias_name"],
        json!(format!(
            "ph_c_{}",
            c.givens.conversation.to_string().replace('-', "")
        ))
    );
    assert_eq!(creation.body["creation_content"]["type"], json!(ROOM_KIND));
    assert_eq!(creation.body["preset"], json!("private_chat"));
    assert_eq!(
        creation.body["power_level_content_override"]["users"][&agent],
        json!(100)
    );
    assert_eq!(
        room.criador, agent,
        "quem cria a sala é o fantasma do agente"
    );

    for http_request in &guard.http_requests {
        assert!(
            !http_request.body.to_string().contains(CANARIO),
            "token no corpo de {}",
            http_request.path
        );
        assert!(
            !http_request.path.contains(CANARIO),
            "token no caminho {}",
            http_request.path
        );
        assert!(!http_request.method.is_empty());
    }
    drop(guard);

    let stored = c
        .port
        .room_of(c.givens.org, c.givens.conversation)
        .expect("linha da sala");
    assert_eq!(stored.state, StoredState::Ready);
    assert_eq!(stored.room_id, room_id);

    assert_eq!(
        c.port.mirrored_name(c.givens.org, c.givens.conversation),
        Some(Some("Maria".to_owned())),
        "nome do cliente espelhado na criação"
    );
}

#[tokio::test]
async fn room_created_without_client_name_keeps_mirrored_name_empty() {
    let mut c = scenario().await;
    c.givens.client_name = None;

    c.provisioner
        .provision(&c.givens)
        .await
        .expect("sala pronta");

    assert_eq!(
        c.port.mirrored_name(c.givens.org, c.givens.conversation),
        Some(None),
        "sem nome do cliente, o nome espelhado é vazio"
    );
    let (_, room) = c.single_room();
    assert_eq!(room.name.as_deref(), Some("+55 (11) 99999-9999"));
}

#[tokio::test]
async fn crash_between_create_and_store_leaves_single_room_and_resume_finds_by_alias() {
    let c = scenario().await;
    c.port.fall_back_on_next_write(1);

    let error = c
        .provisioner
        .provision(&c.givens)
        .await
        .expect_err("o banco caiu antes de gravar");
    assert_eq!(
        error.classification(),
        Classification::Transient,
        "banco indisponível passa: {error}"
    );

    let (created_room_id, _) = c.single_room();
    assert!(
        c.port
            .room_of(c.givens.org, c.givens.conversation)
            .is_none()
    );

    let restart = c.restart();
    let ready = restart
        .provision(&c.givens)
        .await
        .expect("a retomada completa a sala");

    assert_eq!(ready.room_id, created_room_id, "a sala é a mesma");
    let guard = c.with_state();
    assert_eq!(guard.rooms.len(), 1, "nenhuma sala duplicada");
    assert_eq!(guard.creations, 1, "o Synapse criou uma sala só");
    assert!(
        guard.count("/createRoom") >= 2,
        "a retomada tentou criar e caiu no alias em uso"
    );
    drop(guard);

    let stored = c
        .port
        .room_of(c.givens.org, c.givens.conversation)
        .expect("linha gravada na retomada");
    assert_eq!(stored.state, StoredState::Ready);
    assert_eq!(stored.room_id, created_room_id);
}

#[tokio::test]
async fn row_in_creating_finishes_creation_without_creating_another_room() {
    let c = scenario().await;
    c.port.fall_back_on_next_write(1);
    let _ = c.provisioner.provision(&c.givens).await;
    let (room_id, _) = c.single_room();
    c.port
        .ensure_room(c.givens.org, c.givens.conversation, &room_id)
        .await
        .expect("grava a linha em creating");
    assert_eq!(
        c.port
            .room_of(c.givens.org, c.givens.conversation)
            .expect("linha")
            .state,
        StoredState::Creating
    );

    let ready = c
        .restart()
        .provision(&c.givens)
        .await
        .expect("termina a criação");

    assert_eq!(ready.room_id, room_id);
    let guard = c.with_state();
    assert_eq!(guard.rooms.len(), 1);
    assert_eq!(
        guard.creations, 1,
        "não cria sala nova ao terminar a criação"
    );
    let room = guard.rooms.get(&room_id).expect("sala");
    assert!(
        room.state
            .contains_key(&(METADATA_KIND.to_owned(), String::new())),
        "a retomada escreve o metadata que faltava"
    );
    drop(guard);
    assert_eq!(
        c.port
            .room_of(c.givens.org, c.givens.conversation)
            .expect("linha")
            .state,
        StoredState::Ready
    );
}

#[tokio::test]
async fn different_conversations_never_share_room() {
    let c = scenario().await;
    let other_one = ConversationToRoom {
        conversation: ConversationId::generate(),
        client: "+55 (21) 98888-8888".to_owned(),
        client_name: Some("João".to_owned()),
        ..c.givens.clone()
    };

    let first = c
        .provisioner
        .provision(&c.givens)
        .await
        .expect("primeira sala");
    let second = c
        .provisioner
        .provision(&other_one)
        .await
        .expect("segunda sala");

    assert_ne!(first.room_id, second.room_id);
    let guard = c.with_state();
    assert_eq!(guard.rooms.len(), 2);
    assert_eq!(guard.aliases.len(), 2);
    assert_ne!(
        c.provisioner.alias(c.givens.conversation),
        c.provisioner.alias(other_one.conversation)
    );
    let names: BTreeSet<String> = guard
        .rooms
        .values()
        .filter_map(|room| room.name.clone())
        .collect();
    assert_eq!(
        names,
        BTreeSet::from(["Maria".to_owned(), "João".to_owned()])
    );
}

#[tokio::test]
async fn two_concurrent_calls_create_single_room() {
    let c = scenario().await;

    let (a, other_one) = tokio::join!(
        c.provisioner.provision(&c.givens),
        c.provisioner.provision(&c.givens)
    );
    let a = a.expect("primeira chamada");
    let other_one = other_one.expect("segunda chamada");

    assert_eq!(a.room_id, other_one.room_id, "as duas caem na mesma sala");
    let guard = c.with_state();
    assert_eq!(guard.rooms.len(), 1, "uma sala só no Synapse");
    assert_eq!(guard.creations, 1, "o alias impediu a segunda criação");
    assert!(
        guard.count("/createRoom") >= 2,
        "a segunda chamada tentou criar e resolveu pelo alias"
    );
}

#[tokio::test]
async fn ready_room_does_not_talk_to_synapse() {
    let c = scenario().await;
    c.provisioner
        .provision(&c.givens)
        .await
        .expect("primeira passagem");
    let before = c.with_state().http_requests.len();

    let ready = c
        .provisioner
        .provision(&c.givens)
        .await
        .expect("segunda passagem");
    assert!(ready.was_already_ready);
    assert_eq!(
        c.with_state().http_requests.len(),
        before,
        "linha pronta não gera requisição"
    );
}

#[tokio::test]
async fn client_already_in_room_gets_no_invite_again() {
    let c = scenario().await;
    c.provisioner
        .provision(&c.givens)
        .await
        .expect("primeira passagem");
    assert_eq!(c.with_state().count("/invite"), 1);

    c.port
        .return_to_creating(c.givens.org, c.givens.conversation);
    c.provisioner.provision(&c.givens).await.expect("retomada");

    assert_eq!(
        c.with_state().count("/invite"),
        1,
        "o cliente já estava na sala: nenhum convite novo"
    );
    assert_eq!(c.port.ready_marks(), 2, "a retomada marca pronta de novo");
}

#[tokio::test]
async fn history_notice_only_when_message_existed_before_cutoff() {
    let c = scenario().await;
    let givens = ConversationToRoom {
        backfill_cutoff: Some(1_000),
        ..c.givens.clone()
    };
    let ready = c.provisioner.provision(&givens).await.expect("sala");
    assert!(!ready.notice_sent);
    let (_, room) = c.single_room();
    assert!(room.messages.is_empty(), "sem histórico: nenhum aviso");

    let with_history = scenario().await;
    with_history.port.history(true);
    let givens = ConversationToRoom {
        backfill_cutoff: Some(1_000),
        ..with_history.givens.clone()
    };
    let ready = with_history
        .provisioner
        .provision(&givens)
        .await
        .expect("sala");
    assert!(ready.notice_sent);
    let (_, room) = with_history.single_room();
    assert_eq!(room.messages.len(), 1, "um aviso");
    let (txn, kind, body) = &room.messages[0];
    assert_eq!(kind, "m.room.message");
    assert_eq!(body["msgtype"], json!("m.notice"));
    assert_eq!(body["body"], json!(NOTICE_WITHOUT_HISTORY));
    assert!(txn.contains("hist"), "{txn}");

    let without_cutoff = scenario().await;
    without_cutoff.port.history(true);
    let ready = without_cutoff
        .provisioner
        .provision(&ConversationToRoom {
            backfill_cutoff: None,
            ..without_cutoff.givens.clone()
        })
        .await
        .expect("sala");
    assert!(!ready.notice_sent);
    let (_, room) = without_cutoff.single_room();
    assert!(room.messages.is_empty(), "sem corte: nenhum aviso");
}

#[tokio::test]
async fn transient_synapse_failure_does_not_store_room() {
    let c = scenario().await;
    c.with_state().fail_creation = Some((
        503,
        json!({ "errcode": "M_UNKNOWN", "error": "Synapse indisponível" }),
    ));

    let error = c
        .provisioner
        .provision(&c.givens)
        .await
        .expect_err("o Synapse não respondeu");
    assert_eq!(error.classification(), Classification::Transient, "{error}");
    assert!(
        c.port
            .room_of(c.givens.org, c.givens.conversation)
            .is_none(),
        "nada gravado quando a sala não nasceu"
    );
    assert_eq!(c.with_state().rooms.len(), 0);
}

#[tokio::test]
async fn channel_unknown_to_frontend_goes_without_integration_type() {
    let c = scenario().await;
    let givens = ConversationToRoom {
        channel: "console".to_owned(),
        closed: true,
        client_name: None,
        client: "5511999999999".to_owned(),
        department: None,
        ..c.givens.clone()
    };
    c.provisioner.provision(&givens).await.expect("sala");

    let (_, room) = c.single_room();
    let metadata = room
        .state
        .get(&(METADATA_KIND.to_owned(), String::new()))
        .expect("metadata");
    assert!(metadata.get("integration_type").is_none(), "{metadata}");
    assert_eq!(metadata["channel"], json!("console"));
    assert_eq!(metadata["status"], json!("closed"));
    assert!(metadata.get("department_id").is_none(), "{metadata}");
    assert!(
        metadata.get("client_name").is_none(),
        "sem nome não vai campo vazio: {metadata}"
    );
    assert_eq!(room.name.as_deref(), Some("5511999999999"));
}

#[tokio::test]
async fn room_is_born_with_creation_power_and_drops_to_configured_power() {
    let c = scenario().await;
    let agent = format!("@ph_ag_{}:{SERVER}", c.givens.agent);

    c.with_power(10)
        .provision(&c.givens)
        .await
        .expect("sala criada mesmo com poder baixo");
    {
        let guard = c.with_state();
        let creation = guard
            .http_requests
            .iter()
            .find(|http_request| http_request.path.ends_with("/createRoom"))
            .expect("createRoom");
        assert_eq!(
            creation.body["power_level_content_override"]["users"][&agent],
            json!(CREATION_POWER),
            "a criação precisa do poder que o Synapse exige para escrever os próprios poderes"
        );
        let (_, room) = guard.rooms.iter().next().expect("sala");
        let powers = room
            .state
            .get(&("m.room.power_levels".to_owned(), String::new()))
            .expect("poderes da sala");
        assert_eq!(
            powers["users"][&agent],
            json!(50),
            "o piso de poder é o mínimo que mantém o módulo capaz de atualizar a sala"
        );
    }

    let other = scenario().await;
    let other_agent = format!("@ph_ag_{}:{SERVER}", other.givens.agent);
    other
        .with_power(80)
        .provision(&other.givens)
        .await
        .expect("sala com poder 80");
    let guard = other.with_state();
    let creation = guard
        .http_requests
        .iter()
        .find(|http_request| http_request.path.ends_with("/createRoom"))
        .expect("createRoom");
    assert_eq!(
        creation.body["power_level_content_override"]["users"][&other_agent],
        json!(CREATION_POWER)
    );
    drop(guard);
    let (_, room) = other.single_room();
    assert_eq!(
        room.state[&("m.room.power_levels".to_owned(), String::new())]["users"][&other_agent],
        json!(80),
        "o poder configurado vale como poder final"
    );
}
