use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_matrix::client::{ClientConfig, CreateRoom, MatrixClient};
use poph_matrix::rooms::PortError;
use poph_matrix::spaces::{
    CHILD_KIND, DEPARTMENT_KIND, DepartmentSpace, ENTRY_RULES_KIND, InviteLimits, PARENT_KIND,
    Position, RoomForSpace, SpaceError, SpaceOrigin, Spaces, SpacesPort, StoredSpace,
    department_alias, resolve_department,
};
use poph_matrix::users::{Ghost, GhostIds};
use poph_types::{AgentId, ConversationId, DepartmentId, MatrixConfig, OrgId, Secret};
use serde_json::{Value, json};

const CANARIO: &str = "CANARIO-MX12-nao-pode-vazar";
const SERVER: &str = "matrix-dev.local";
const ATTENDANT: &str = "@ana:matrix-dev.local";

#[derive(Default, Clone)]
struct FakeRoom {
    kind: Option<String>,
    name: Option<String>,
    topico: Option<String>,
    criador: String,
    members: BTreeSet<String>,
    guests: BTreeSet<String>,
    state: BTreeMap<(String, String), Value>,
    power: Option<Value>,
}

#[derive(Default)]
struct FakeState {
    expected_token: String,
    rooms: BTreeMap<String, FakeRoom>,
    aliases: BTreeMap<String, String>,
    next_room: u32,
    http_requests: Vec<String>,
    creations: u32,
    fail_next_creation: bool,
    fail_invitations: usize,
}

impl FakeState {
    fn count(&self, suffix: &str) -> usize {
        self.http_requests
            .iter()
            .filter(|path| path.ends_with(suffix))
            .count()
    }

    fn alias_room(&self, alias: &str) -> Option<&String> {
        self.aliases.get(alias)
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

fn initial_powers(criador: &str, sobreposto: Option<&Value>) -> Value {
    let mut base = json!({
        "users": { criador: 100 },
        "users_default": 0,
        "events_default": 0,
        "state_default": 50,
        "ban": 50,
        "kick": 50,
        "redact": 50,
        "invite": 0,
    });
    if let Some(sobreposto) = sobreposto.filter(|value| value.is_object()) {
        for (key, value) in sobreposto.as_object().expect("objeto") {
            base[key.clone()] = value.clone();
        }
    }
    base
}

fn room_powers(room: &FakeRoom) -> Value {
    match room
        .state
        .get(&("m.room.power_levels".to_owned(), String::new()))
    {
        Some(event) => event.clone(),
        None => initial_powers(&room.criador, room.power.as_ref()),
    }
}

fn user_power(powers: &Value, user: &str) -> i64 {
    powers
        .get("users")
        .and_then(|users| users.get(user))
        .and_then(Value::as_i64)
        .or_else(|| powers.get("users_default").and_then(Value::as_i64))
        .unwrap_or(0)
}

fn state_level(powers: &Value, kind: &str) -> i64 {
    powers
        .get("events")
        .and_then(|events| events.get(kind))
        .and_then(Value::as_i64)
        .or_else(|| powers.get("state_default").and_then(Value::as_i64))
        .unwrap_or(50)
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

    guard.http_requests.push(uri.path().to_owned());

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

    let path = uri
        .path()
        .trim_start_matches("/_matrix/client/v3")
        .trim_start_matches("/_matrix/client/v1");
    let parts: Vec<String> = path
        .split('/')
        .filter(|piece| !piece.is_empty())
        .map(decode)
        .collect();
    let raw_query = uri.query().unwrap_or_default().to_owned();
    let who = query(&raw_query, "user_id").unwrap_or_default();

    match (method.as_str(), parts.as_slice()) {
        ("POST", [creation]) if creation == "createRoom" => {
            if guard.fail_next_creation {
                guard.fail_next_creation = false;
                return reply(
                    503,
                    json!({ "errcode": "M_UNKNOWN", "error": "Synapse fora do ar" }),
                );
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
            let mut state = BTreeMap::new();
            state.insert(
                ("m.room.power_levels".to_owned(), String::new()),
                initial_powers(&who, body.get("power_level_content_override")),
            );
            guard.rooms.insert(
                room.clone(),
                FakeRoom {
                    kind: body["creation_content"]["type"].as_str().map(str::to_owned),
                    name: body["name"].as_str().map(str::to_owned),
                    topico: body["topic"].as_str().map(str::to_owned),
                    criador: who.clone(),
                    members,
                    power: body.get("power_level_content_override").cloned(),
                    state,
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
            if guard.fail_invitations > 0 {
                guard.fail_invitations -= 1;
                return reply(
                    503,
                    json!({ "errcode": "M_UNKNOWN", "error": "indisponível no momento" }),
                );
            }
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
        ("GET", [rooms, room, hierarquia])
            if rooms == "rooms" && hierarquia == "hierarchy" && path.starts_with("/rooms") =>
        {
            match guard.rooms.get(room) {
                Some(target) => {
                    let children: Vec<Value> = target
                        .state
                        .iter()
                        .filter(|((kind, _), body)| {
                            kind == CHILD_KIND && body.as_object().is_some_and(|m| !m.is_empty())
                        })
                        .map(|((_, child_room), _)| json!({ "room_id": child_room }))
                        .collect();
                    reply(200, json!({ "rooms": children }))
                }
                None => reply(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("GET", [rooms, room, state]) if rooms == "rooms" && state == "state" => {
            match guard.rooms.get(room) {
                Some(target) if target.members.contains(&who) => {
                    let events: Vec<Value> = target
                        .state
                        .iter()
                        .map(|((kind, key), body)| {
                            json!({ "type": kind, "state_key": key, "content": body })
                        })
                        .collect();
                    reply(200, Value::Array(events))
                }
                Some(_) => reply(
                    403,
                    json!({ "errcode": "M_FORBIDDEN", "error": "User not in room" }),
                ),
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
                        json!({ "errcode": "M_NOT_FOUND", "error": "Evento de estado ausente" }),
                    ),
                },
                None => reply(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("PUT", [rooms, room, state, kind, key @ ..]) if rooms == "rooms" && state == "state" => {
            let key = key.join("/");
            let Some(target) = guard.rooms.get_mut(room) else {
                return reply(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                );
            };
            if !target.members.contains(&who) {
                return reply(
                    403,
                    json!({ "errcode": "M_FORBIDDEN", "error": "User not in room" }),
                );
            }
            let powers = room_powers(target);
            let power = user_power(&powers, &who);
            let necessario = state_level(&powers, kind);
            if power < necessario {
                return reply(
                    403,
                    json!({ "errcode": "M_FORBIDDEN", "error": "You don't have permission" }),
                );
            }
            target.state.insert((kind.clone(), key), body);
            reply(200, json!({ "event_id": "$estado" }))
        }
        _ => reply(
            404,
            json!({ "errcode": "M_UNRECOGNIZED", "error": "Rota não existe no Synapse falso" }),
        ),
    }
}

#[derive(Default)]
struct PortState {
    spaces: BTreeMap<(OrgId, DepartmentId), StoredSpace>,
    rooms_space: BTreeMap<(OrgId, ConversationId), String>,
}

#[derive(Clone, Default)]
struct FakePort(Arc<Mutex<PortState>>);

impl FakePort {
    fn space_of(&self, org: OrgId, department: DepartmentId) -> Option<StoredSpace> {
        self.0
            .lock()
            .expect("trava")
            .spaces
            .get(&(org, department))
            .cloned()
    }

    fn room_space(&self, org: OrgId, conversation: ConversationId) -> Option<String> {
        self.0
            .lock()
            .expect("trava")
            .rooms_space
            .get(&(org, conversation))
            .cloned()
    }
}

impl SpacesPort for FakePort {
    async fn space(
        &self,
        org: OrgId,
        department: DepartmentId,
    ) -> Result<Option<StoredSpace>, PortError> {
        Ok(self.space_of(org, department))
    }

    async fn ensure_space(
        &self,
        org: OrgId,
        department: DepartmentId,
        room_id: &str,
        alias: &str,
        origin: SpaceOrigin,
    ) -> Result<StoredSpace, PortError> {
        let mut guard = self.0.lock().expect("trava");
        let stored = guard
            .spaces
            .entry((org, department))
            .or_insert_with(|| StoredSpace {
                room_id: room_id.to_owned(),
                alias: alias.to_owned(),
                origin,
            });
        Ok(stored.clone())
    }

    async fn room_space(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Result<Option<String>, PortError> {
        Ok(FakePort::room_space(self, org, conversation))
    }

    async fn set_room_space(
        &self,
        org: OrgId,
        conversation: ConversationId,
        space: Option<&str>,
    ) -> Result<(), PortError> {
        let mut guard = self.0.lock().expect("trava");
        match space {
            Some(space) => {
                guard
                    .rooms_space
                    .insert((org, conversation), space.to_owned());
            }
            None => {
                guard.rooms_space.remove(&(org, conversation));
            }
        }
        Ok(())
    }
}

struct Scenario {
    state: Arc<Mutex<FakeState>>,
    port: FakePort,
    spaces: Spaces<FakePort>,
    address: String,
    agent: Ghost,
    bot: String,
    fixed_org: OrgId,
    fixed_conversation: ConversationId,
}

impl Scenario {
    fn with_state(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().expect("trava do Synapse falso")
    }

    fn client(&self) -> MatrixClient {
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

    async fn conversation_room(&self) -> String {
        self.client()
            .create_room(
                self.agent.user_id(),
                &CreateRoom {
                    name: Some("Maria".to_owned()),
                    ..CreateRoom::default()
                },
            )
            .await
            .expect("createRoom da sala")
    }

    async fn operator_space(&self, bot_inside: bool) -> String {
        let client = self.client();
        let space = client
            .create_room(
                ATTENDANT,
                &CreateRoom {
                    name: Some("Espaço do operador".to_owned()),
                    creation_content: Some(json!({ "type": "m.space" })),
                    ..CreateRoom::default()
                },
            )
            .await
            .expect("createRoom do espaço do operador");
        if bot_inside {
            client
                .invite(ATTENDANT, &space, &self.bot)
                .await
                .expect("convite do bot");
            client.enter(&self.bot, &space).await.expect("join do bot");
        }
        space
    }

    async fn give_bot_power(&self, space: &str, power: i64) {
        self.client()
            .send_state(
                ATTENDANT,
                space,
                "m.room.power_levels",
                None,
                &json!({
                    "users": { ATTENDANT: 100, self.bot.clone(): power },
                    "users_default": 0,
                    "state_default": 50,
                }),
            )
            .await
            .expect("power levels do espaço");
    }

    async fn person_in_space(&self, person: &str, space: &str) {
        let client = self.client();
        client
            .invite(ATTENDANT, space, person)
            .await
            .expect("convite da pessoa");
        client.enter(person, space).await.expect("join da pessoa");
    }

    fn with_room(&self, room: &str) -> RoomForSpace {
        RoomForSpace {
            org: self.org(),
            conversation: self.conversation(),
            room: room.to_owned(),
            agent: self.agent.clone(),
            department: Some(self.department("Comercial")),
            existing: None,
        }
    }

    fn org(&self) -> OrgId {
        self.fixed_org
    }

    fn conversation(&self) -> ConversationId {
        self.fixed_conversation
    }

    fn department(&self, name: &str) -> DepartmentSpace {
        DepartmentSpace {
            id: DepartmentId::generate(),
            name: name.to_owned(),
        }
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
    let ids = GhostIds::new("ph", SERVER).expect("ids");
    let agent = ids.agent(&AgentId::generate().to_string()).expect("agente");
    let bot = ids.bot().user_id().to_owned();
    let port = FakePort::default();
    let spaces = Spaces::with(client, ids, port.clone()).with_limits(InviteLimits::with(
        2,
        2,
        Duration::ZERO,
        4,
    ));

    Scenario {
        state,
        port,
        spaces,
        address: format!("http://{address}"),
        agent,
        bot,
        fixed_org: OrgId::generate(),
        fixed_conversation: ConversationId::generate(),
    }
}

#[tokio::test]
async fn creates_space_when_missing_and_repeat_does_not_create_second() {
    let scenario = scenario().await;
    let room = scenario.conversation_room().await;
    let mut givens = scenario.with_room(&room);
    let department = givens.department.clone().expect("departamento");
    let alias = department_alias(SERVER, department.id);

    let first = scenario.spaces.place(&givens).await.expect("posicionar");
    let space = match first {
        Position::Positioned { space, moved, .. } => {
            assert!(!moved, "a sala não veio de outro espaço");
            space
        }
        other_one => panic!("esperava Posicionada, veio {other_one:?}"),
    };

    {
        let guard = scenario.with_state();
        assert_eq!(guard.creations, 2, "a sala da conversa e o espaço");
        assert_eq!(
            guard.count("/createRoom"),
            2,
            "dois createRoom no total; o espaço vem pelo alias"
        );
        let no_synapse = guard.rooms.get(&space).expect("espaço no Synapse");
        assert_eq!(no_synapse.kind.as_deref(), Some("m.space"));
        assert_eq!(no_synapse.name.as_deref(), Some("Comercial"));
        assert_eq!(
            no_synapse.topico.as_deref(),
            Some("Departamento · Comercial")
        );
        assert_eq!(
            guard.alias_room(&alias).map(String::as_str),
            Some(space.as_str()),
            "o espaço nasceu com o alias do departamento"
        );
        assert_eq!(
            no_synapse
                .state
                .get(&(DEPARTMENT_KIND.to_owned(), String::new()))
                .map(|body| body["department_id"].clone()),
            Some(json!(department.id.to_string())),
            "o espaço diz qual departamento ele é"
        );
        let child = no_synapse
            .state
            .get(&(CHILD_KIND.to_owned(), room.clone()))
            .expect("m.space.child do espaço");
        assert_eq!(child["via"], json!([SERVER]));

        let of_room = guard.rooms.get(&room).expect("sala");
        let parent = of_room
            .state
            .get(&(PARENT_KIND.to_owned(), space.clone()))
            .expect("m.space.parent da sala");
        assert_eq!(parent["via"], json!([SERVER]));
        assert_eq!(parent["canonical"], json!(true));
        let rules = of_room
            .state
            .get(&(ENTRY_RULES_KIND.to_owned(), String::new()))
            .expect("join_rules");
        assert_eq!(rules["join_rule"], json!("restricted"));
        assert_eq!(rules["allow"][0]["type"], json!("m.room_membership"));
        assert_eq!(rules["allow"][0]["room_id"], json!(space));
    }

    let before = scenario.with_state().http_requests.len();
    let second = scenario.spaces.place(&givens).await.expect("repetir");
    assert_eq!(
        second,
        Position::AlreadyPositioned {
            space: space.clone()
        }
    );
    assert_eq!(
        scenario.with_state().http_requests.len(),
        before,
        "sala já posicionada não fala com o Synapse"
    );
    assert_eq!(scenario.with_state().creations, 2, "nenhum segundo espaço");
    assert_eq!(
        scenario
            .port
            .space_of(givens.org, department.id)
            .expect("gravado")
            .origin,
        SpaceOrigin::Automatic
    );

    givens.department = Some(DepartmentSpace {
        id: department.id,
        name: "Comercial e Vendas".to_owned(),
    });
    let third = scenario.spaces.place(&givens).await.expect("renomeado");
    assert_eq!(
        third,
        Position::AlreadyPositioned {
            space: space.clone()
        }
    );
    assert_eq!(scenario.with_state().creations, 2);
}

#[tokio::test]
async fn links_space_that_already_exists_in_element() {
    let scenario = scenario().await;
    let room = scenario.conversation_room().await;
    let existing = scenario.operator_space(true).await;
    scenario.give_bot_power(&existing, 50).await;
    let mut givens = scenario.with_room(&room);
    let department = givens.department.clone().expect("departamento");
    givens.existing = Some(existing.clone());

    let position = scenario.spaces.place(&givens).await.expect("ligar");
    assert!(
        matches!(&position, Position::Positioned { space, .. } if *space == existing),
        "esperava a sala no espaço do operador, veio {position:?}"
    );

    {
        let guard = scenario.with_state();
        assert_eq!(guard.creations, 2, "só a sala e o espaço do operador");
        assert!(
            guard
                .rooms
                .get(&existing)
                .expect("espaço")
                .state
                .contains_key(&(CHILD_KIND.to_owned(), room.clone())),
            "o filho foi escrito no espaço existente"
        );
        assert!(
            !guard
                .aliases
                .contains_key(&department_alias(SERVER, department.id)),
            "ligar a um espaço existente não cria alias novo"
        );
    }

    let stored = scenario
        .port
        .space_of(givens.org, department.id)
        .expect("gravado");
    assert_eq!(stored.room_id, existing);
    assert_eq!(stored.origin, SpaceOrigin::Linked);

    let before = scenario.with_state().http_requests.len();
    let repeated = scenario.spaces.place(&givens).await.expect("repetir");
    assert_eq!(
        repeated,
        Position::AlreadyPositioned {
            space: existing.clone()
        }
    );
    assert_eq!(scenario.with_state().http_requests.len(), before);
}

#[tokio::test]
async fn linking_space_without_bot_inside_gives_clear_error_and_stores_nothing() {
    let scenario = scenario().await;
    let room = scenario.conversation_room().await;
    let existing = scenario.operator_space(false).await;
    let mut givens = scenario.with_room(&room);
    let department = givens.department.clone().expect("departamento");
    givens.existing = Some(existing);

    let error = scenario
        .spaces
        .place(&givens)
        .await
        .expect_err("sem o bot no espaço tem que falhar");
    match &error {
        SpaceError::WithoutPermission { what_is_missing } => {
            assert!(
                what_is_missing.contains(&scenario.bot),
                "o erro diz qual usuário convidar: {what_is_missing}"
            );
            assert!(
                what_is_missing.contains("convide") && what_is_missing.contains("Element"),
                "o erro diz o que fazer: {what_is_missing}"
            );
            assert!(
                what_is_missing.contains("não é membro"),
                "o erro diz que falta o convite, não o poder: {what_is_missing}"
            );
        }
        other => panic!("esperava SemPermissao, veio {other:?}"),
    }
    assert_eq!(
        error.classification(),
        poph_matrix::error::Classification::Transient,
        "falta de permissão volta a ser tentada depois da correção do operador"
    );
    assert!(
        scenario.port.space_of(givens.org, department.id).is_none(),
        "espaço sem permissão não é gravado"
    );
    assert!(
        scenario
            .port
            .room_space(givens.org, givens.conversation)
            .is_none(),
        "a sala não é dada como posicionada"
    );
}

#[tokio::test]
async fn linking_space_where_bot_lacks_power_gives_clear_error() {
    let scenario = scenario().await;
    let room = scenario.conversation_room().await;
    let existing = scenario.operator_space(true).await;
    scenario
        .client()
        .send_state(
            ATTENDANT,
            &existing,
            "m.room.power_levels",
            None,
            &json!({
                "users": { ATTENDANT: 100 },
                "users_default": 0,
                "state_default": 50,
            }),
        )
        .await
        .expect("power levels do espaço");
    let mut givens = scenario.with_room(&room);
    givens.existing = Some(existing);

    let error = scenario
        .spaces
        .place(&givens)
        .await
        .expect_err("sem poder tem que falhar");
    match &error {
        SpaceError::WithoutPermission { what_is_missing } => {
            assert!(
                what_is_missing.contains("poder") && what_is_missing.contains("50"),
                "o erro explica o poder que falta: {what_is_missing}"
            );
        }
        other => panic!("esperava SemPermissao, veio {other:?}"),
    }
}

#[tokio::test]
async fn moves_room_when_department_changes() {
    let scenario = scenario().await;
    let room = scenario.conversation_room().await;
    let mut givens = scenario.with_room(&room);
    let commercial = scenario
        .spaces
        .place(&givens)
        .await
        .expect("no espaço do comercial");
    let Position::Positioned { space: old, .. } = commercial else {
        panic!("esperava Posicionada");
    };

    givens.department = Some(scenario.department("Suporte"));
    let fresh = scenario
        .spaces
        .place(&givens)
        .await
        .expect("mover de espaço");
    let Position::Positioned {
        space: new, moved, ..
    } = fresh
    else {
        panic!("esperava Posicionada");
    };
    assert!(moved, "a sala veio de outro espaço");
    assert_ne!(new, old);

    let guard = scenario.with_state();
    let in_old = guard
        .rooms
        .get(&old)
        .expect("espaço antigo")
        .state
        .get(&(CHILD_KIND.to_owned(), room.clone()))
        .expect("o filho continua escrito, mas vazio");
    assert_eq!(
        in_old,
        &json!({}),
        "conteúdo vazio é como o Matrix tira o filho do espaço"
    );
    assert!(
        guard
            .rooms
            .get(&new)
            .expect("espaço novo")
            .state
            .get(&(CHILD_KIND.to_owned(), room.clone()))
            .is_some_and(|body| body["via"] == json!([SERVER])),
        "a sala entrou no espaço novo"
    );
    let of_room = guard.rooms.get(&room).expect("sala");
    assert_eq!(
        of_room
            .state
            .get(&(PARENT_KIND.to_owned(), old.clone()))
            .cloned(),
        Some(json!({})),
        "o pai antigo foi removido"
    );
    assert_eq!(
        of_room
            .state
            .get(&(PARENT_KIND.to_owned(), new.clone()))
            .map(|body| body["canonical"].clone()),
        Some(json!(true)),
        "o pai novo aponta para o espaço novo"
    );
    drop(guard);
    assert_eq!(
        scenario.port.room_space(givens.org, givens.conversation),
        Some(new)
    );
}

#[tokio::test]
async fn conversation_without_department_does_not_fail_and_does_not_talk_to_synapse() {
    let scenario = scenario().await;
    let room = scenario.conversation_room().await;
    let before = scenario.with_state().http_requests.len();

    let mut givens = scenario.with_room(&room);
    givens.department = None;
    let position = scenario.spaces.place(&givens).await.expect("sem dep");
    assert_eq!(position, Position::WithoutDepartment);
    assert_eq!(
        scenario.with_state().http_requests.len(),
        before,
        "sem departamento, nenhuma chamada de espaço"
    );
    assert!(
        scenario
            .port
            .room_space(givens.org, givens.conversation)
            .is_none()
    );

    let default = DepartmentId::generate();
    assert_eq!(resolve_department(None, Some(default)), Some(default));
    assert_eq!(resolve_department(None, None), None);
}

#[tokio::test]
async fn room_left_without_department_leaves_space() {
    let scenario = scenario().await;
    let room = scenario.conversation_room().await;
    let mut givens = scenario.with_room(&room);
    let Position::Positioned { space, .. } =
        scenario.spaces.place(&givens).await.expect("no espaço")
    else {
        panic!("esperava Posicionada");
    };

    givens.department = None;
    let position = scenario.spaces.place(&givens).await.expect("sem dep");
    assert_eq!(
        position,
        Position::Removed {
            space: space.clone()
        }
    );
    {
        let guard = scenario.with_state();
        assert_eq!(
            guard
                .rooms
                .get(&space)
                .expect("espaço")
                .state
                .get(&(CHILD_KIND.to_owned(), room.clone())),
            Some(&json!({})),
            "o filho foi removido do espaço"
        );
    }
    assert!(
        scenario
            .port
            .room_space(givens.org, givens.conversation)
            .is_none()
    );
}

#[tokio::test]
async fn invites_space_members_in_limited_batch() {
    let scenario = scenario().await;
    let room = scenario.conversation_room().await;
    let mut givens = scenario.with_room(&room);
    let department = givens.department.clone().expect("departamento");
    let existing = scenario.operator_space(true).await;
    scenario.give_bot_power(&existing, 50).await;
    givens.existing = Some(existing);

    let persons: Vec<String> = (0..6).map(|n| format!("@atendente{n}:{SERVER}")).collect();
    for person in &persons {
        scenario
            .person_in_space(person, givens.existing.as_deref().expect("espaço"))
            .await;
    }

    let position = scenario.spaces.place(&givens).await.expect("posicionar");
    let Position::Positioned {
        guests,
        failed_invitations,
        ..
    } = position
    else {
        panic!("esperava Posicionada");
    };
    assert_eq!(guests, 4, "só o teto da passagem é convidado");
    assert_eq!(failed_invitations, 0);

    {
        let guard = scenario.with_state();
        let guests_in_room = &guard.rooms.get(&room).expect("sala").guests;
        assert_eq!(guests_in_room.len(), 4);
        assert!(
            !guests_in_room.contains(&scenario.bot),
            "o bot não é convidado para a sala da conversa"
        );
        assert!(
            !guests_in_room.contains(scenario.agent.user_id()),
            "o agente já está na sala e não é convidado"
        );
    }
    assert!(scenario.port.space_of(givens.org, department.id).is_some());

    let guests_in_room: Vec<String> = scenario
        .with_state()
        .rooms
        .get(&room)
        .expect("sala")
        .guests
        .iter()
        .cloned()
        .collect();
    for person in guests_in_room {
        scenario
            .client()
            .enter(&person, &room)
            .await
            .expect("join de quem aceitou");
    }

    scenario.port.0.lock().expect("trava").rooms_space.clear();
    let position = scenario.spaces.place(&givens).await.expect("segunda");
    let Position::Positioned { guests, .. } = position else {
        panic!("esperava Posicionada");
    };
    assert_eq!(guests, 3, "só quem ainda estava de fora");
    assert_eq!(
        scenario
            .with_state()
            .rooms
            .get(&room)
            .expect("sala")
            .guests
            .len(),
        7,
        "todos os membros do espaço foram convidados ao todo"
    );
}

#[tokio::test]
async fn failed_invite_does_not_store_link_and_next_pass_completes() {
    let scenario = scenario().await;
    let room = scenario.conversation_room().await;
    let mut givens = scenario.with_room(&room);
    let department = givens.department.clone().expect("departamento");
    let existing = scenario.operator_space(true).await;
    scenario.give_bot_power(&existing, 50).await;
    givens.existing = Some(existing.clone());

    let persons: Vec<String> = (0..2).map(|n| format!("@atendente{n}:{SERVER}")).collect();
    for person in &persons {
        scenario.person_in_space(person, &existing).await;
    }

    scenario.with_state().fail_invitations = 1;
    let first = scenario
        .spaces
        .place(&givens)
        .await
        .expect("primeira passagem");
    let Position::Positioned {
        failed_invitations,
        moved,
        ..
    } = first
    else {
        panic!("esperava Posicionada");
    };
    assert_eq!(failed_invitations, 1, "um convite falhou");
    assert!(!moved, "a sala não é dada como movida");
    assert!(
        scenario
            .port
            .room_space(givens.org, givens.conversation)
            .is_none(),
        "convite pendente não pode gravar o vínculo: a passagem seguinte sairia por JaPosicionada"
    );

    let second = scenario
        .spaces
        .place(&givens)
        .await
        .expect("segunda passagem");
    let Position::Positioned {
        failed_invitations,
        space,
        ..
    } = second
    else {
        panic!("esperava Posicionada na retomada");
    };
    assert_eq!(
        failed_invitations, 0,
        "a retomada não deixa convite pendente"
    );
    let guests_in_room: Vec<String> = scenario
        .with_state()
        .rooms
        .get(&room)
        .expect("sala")
        .guests
        .iter()
        .cloned()
        .collect();
    for person in &persons {
        assert!(
            guests_in_room.contains(person),
            "{person} não foi convidado na passagem seguinte"
        );
    }
    assert_eq!(
        scenario
            .port
            .room_space(givens.org, givens.conversation)
            .as_deref(),
        Some(space.as_str()),
        "com os convites fechados, o vínculo é gravado"
    );
    assert!(scenario.port.space_of(givens.org, department.id).is_some());

    let before = scenario.with_state().http_requests.len();
    let third = scenario
        .spaces
        .place(&givens)
        .await
        .expect("terceira passagem");
    assert!(
        matches!(third, Position::AlreadyPositioned { .. }),
        "esperava JaPosicionada"
    );
    assert_eq!(
        scenario.with_state().http_requests.len(),
        before,
        "nada foi escrito no Synapse"
    );
}

#[tokio::test]
async fn room_appears_under_space_in_synapse_hierarchy() {
    let scenario = scenario().await;
    let room = scenario.conversation_room().await;
    let givens = scenario.with_room(&room);
    let Position::Positioned { space, .. } =
        scenario.spaces.place(&givens).await.expect("posicionar")
    else {
        panic!("esperava Posicionada");
    };

    let children = scenario
        .client()
        .hierarquia(scenario.agent.user_id(), &space)
        .await
        .expect("hierarchy");
    assert_eq!(children, vec![room], "a sala está sob o espaço");
}

#[tokio::test]
async fn transient_synapse_failure_does_not_store_link() {
    let scenario = scenario().await;
    let room = scenario.conversation_room().await;
    let givens = scenario.with_room(&room);
    scenario.with_state().fail_next_creation = true;

    let error = scenario
        .spaces
        .place(&givens)
        .await
        .expect_err("Synapse fora do ar");
    assert_eq!(
        error.classification(),
        poph_matrix::error::Classification::Transient
    );
    assert!(
        scenario
            .port
            .space_of(givens.org, givens.department.clone().expect("dep").id)
            .is_none()
    );
    assert!(
        scenario
            .port
            .room_space(givens.org, givens.conversation)
            .is_none()
    );

    let position = scenario.spaces.place(&givens).await.expect("retomada");
    assert!(matches!(position, Position::Positioned { .. }));
}

#[tokio::test]
async fn given_space_invalid_is_contract_error() {
    let scenario = scenario().await;
    let room = scenario.conversation_room().await;
    let mut givens = scenario.with_room(&room);
    givens.existing = Some("sala-sem-id-nem-alias".to_owned());
    let error = scenario
        .spaces
        .place(&givens)
        .await
        .expect_err("referência inválida");
    assert!(
        matches!(error, SpaceError::InvalidSpace { .. }),
        "{error:?}"
    );
}

fn requests_with(scenario: &Scenario, passage: &str) -> usize {
    scenario
        .with_state()
        .http_requests
        .iter()
        .filter(|path| path.contains(passage))
        .count()
}

#[tokio::test]
async fn transfer_with_empty_record_removes_room_from_old_space() {
    let scenario = scenario().await;
    let room = scenario.conversation_room().await;
    let mut givens = scenario.with_room(&room);
    givens.department = Some(scenario.department("Comercial"));
    let Position::Positioned { space: old, .. } = scenario
        .spaces
        .place(&givens)
        .await
        .expect("no espaço do comercial")
    else {
        panic!("esperava Posicionada");
    };

    scenario
        .port
        .set_room_space(givens.org, givens.conversation, None)
        .await
        .expect("registro vazio");

    givens.department = Some(scenario.department("Suporte"));
    let writes_in_old_before = requests_with(&scenario, &format!("{old}/state/{CHILD_KIND}/"));
    let fresh = scenario
        .spaces
        .place(&givens)
        .await
        .expect("transferência com registro vazio");
    let Position::Positioned {
        space: new,
        moved,
        failed_invitations: 0,
        ..
    } = fresh.clone()
    else {
        panic!("esperava Posicionada sem convite falho, veio {fresh:?}");
    };
    assert!(moved, "a sala veio do espaço antigo, mesmo sem o registro");
    assert_ne!(new, old);

    {
        let guard = scenario.with_state();
        assert_eq!(
            guard.rooms[&old]
                .state
                .get(&(CHILD_KIND.to_owned(), room.clone())),
            Some(&json!({})),
            "o m.space.child sai do espaço antigo"
        );
        let of_room = &guard.rooms[&room];
        assert_eq!(
            of_room.state.get(&(PARENT_KIND.to_owned(), old.clone())),
            Some(&json!({})),
            "o m.space.parent do espaço antigo sai da sala"
        );
        assert_eq!(
            of_room
                .state
                .get(&(PARENT_KIND.to_owned(), new.clone()))
                .map(|body| body["canonical"].clone()),
            Some(json!(true)),
            "o m.space.parent do espaço novo fica na sala"
        );
    }
    let client = scenario.client();
    let under_old = client
        .hierarquia(&scenario.bot, &old)
        .await
        .expect("hierarquia do antigo");
    assert!(
        !under_old.contains(&room),
        "a sala não aparece mais sob o espaço antigo"
    );
    let under_new = client
        .hierarquia(&scenario.bot, &new)
        .await
        .expect("hierarquia do novo");
    assert!(
        under_new.contains(&room),
        "a sala aparece sob o espaço novo"
    );
    assert_eq!(
        scenario.port.room_space(givens.org, givens.conversation),
        Some(new.clone())
    );

    let old_writes = requests_with(&scenario, &format!("{old}/state/{CHILD_KIND}/"));
    assert_eq!(
        old_writes,
        writes_in_old_before + 1,
        "a transferência tirou o m.space.child do espaço antigo com uma escrita só"
    );
    scenario
        .port
        .set_room_space(givens.org, givens.conversation, None)
        .await
        .expect("registro vazio de novo");
    let repeated = scenario
        .spaces
        .place(&givens)
        .await
        .expect("repetir com registro vazio");
    assert!(
        matches!(repeated, Position::Positioned { moved: false, .. }),
        "repetir não move a sala de novo, veio {repeated:?}"
    );
    assert_eq!(
        requests_with(&scenario, &format!("{old}/state/{CHILD_KIND}/")),
        old_writes,
        "o espaço antigo não recebe escrita de novo"
    );
}

#[tokio::test]
async fn transfer_with_empty_record_and_failed_invite_only_leaves_old_on_next_pass() {
    let scenario = scenario().await;
    let room = scenario.conversation_room().await;
    let mut givens = scenario.with_room(&room);
    givens.department = Some(scenario.department("Comercial"));
    let Position::Positioned { space: old, .. } = scenario
        .spaces
        .place(&givens)
        .await
        .expect("no espaço do comercial")
    else {
        panic!("esperava Posicionada");
    };
    scenario
        .port
        .set_room_space(givens.org, givens.conversation, None)
        .await
        .expect("registro vazio");

    let support = scenario.operator_space(true).await;
    scenario.give_bot_power(&support, 50).await;
    scenario
        .person_in_space("@atendente0:matrix-dev.local", &support)
        .await;
    givens.department = Some(scenario.department("Suporte"));
    givens.existing = Some(support.clone());

    scenario.with_state().fail_invitations = 1;
    let first = scenario
        .spaces
        .place(&givens)
        .await
        .expect("primeira passagem");
    let Position::Positioned {
        failed_invitations: 1,
        moved: false,
        ..
    } = first
    else {
        panic!("esperava um convite falho, veio {first:?}");
    };
    assert!(
        scenario
            .port
            .room_space(givens.org, givens.conversation)
            .is_none(),
        "convite pendente não grava o vínculo"
    );
    assert!(
        scenario.with_state().rooms[&old]
            .state
            .get(&(CHILD_KIND.to_owned(), room.clone()))
            .is_some_and(|body| body != &json!({})),
        "com convite pendente a sala ainda não saiu do espaço antigo"
    );

    scenario.with_state().fail_invitations = 0;
    let second = scenario
        .spaces
        .place(&givens)
        .await
        .expect("passagem seguinte");
    let Position::Positioned {
        space,
        moved: true,
        failed_invitations: 0,
        ..
    } = second
    else {
        panic!("esperava a sala movida na passagem seguinte, veio {second:?}");
    };
    assert_eq!(space, support);
    assert_eq!(
        scenario.port.room_space(givens.org, givens.conversation),
        Some(support.clone())
    );
    assert_eq!(
        scenario.with_state().rooms[&old]
            .state
            .get(&(CHILD_KIND.to_owned(), room.clone())),
        Some(&json!({})),
        "na passagem seguinte a sala sai do espaço antigo"
    );
}

#[tokio::test]
async fn transfer_with_filled_record_does_not_read_room_state() {
    let scenario = scenario().await;
    let room = scenario.conversation_room().await;
    let mut givens = scenario.with_room(&room);
    givens.department = Some(scenario.department("Comercial"));
    scenario
        .spaces
        .place(&givens)
        .await
        .expect("no espaço do comercial");

    givens.department = Some(scenario.department("Suporte"));
    let before = scenario.with_state().http_requests.len();
    let fresh = scenario
        .spaces
        .place(&givens)
        .await
        .expect("transferência com registro");
    assert!(
        matches!(fresh, Position::Positioned { moved: true, .. }),
        "veio {fresh:?}"
    );
    let after: Vec<String> = scenario.with_state().http_requests[before..].to_vec();
    assert!(
        !after.iter().any(|path| path.ends_with("/state")),
        "com o registro preenchido, a transferência não lê o estado inteiro da sala: {after:?}"
    );

    let before_repeated = scenario.with_state().http_requests.len();
    let repeated = scenario
        .spaces
        .place(&givens)
        .await
        .expect("passagem repetida");
    assert!(
        matches!(repeated, Position::AlreadyPositioned { .. }),
        "veio {repeated:?}"
    );
    assert_eq!(
        scenario.with_state().http_requests.len(),
        before_repeated,
        "a passagem repetida não fala com o Synapse"
    );
}
