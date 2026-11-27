use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_matrix::error::Classification;
use poph_matrix::users::{DEFAULT_CACHE_CAP, Ghost, GhostIds, GhostsError, VirtualUsers};
use poph_matrix::{ClientConfig, MatrixClient};
use poph_types::Secret;
use serde_json::{Value, json};

const CANARIO: &str = "CANARIO-MX10-nao-pode-vazar";

#[derive(Clone, Debug)]
struct FakeResponse {
    status: u16,
    body: Value,
}

impl FakeResponse {
    fn ok(body: Value) -> Self {
        Self { status: 200, body }
    }

    fn error(status: u16, body: Value) -> Self {
        Self { status, body }
    }
}

#[derive(Clone, Debug, Default)]
struct Record {
    method: String,
    path: String,
    query: String,
    body: Value,
    authorization_matches: bool,
}

#[derive(Default)]
struct FakeState {
    script: VecDeque<FakeResponse>,
    records: Vec<Record>,
    expected_token: String,
}

async fn attend(
    State(state): State<Arc<Mutex<FakeState>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    let response = {
        let mut guard = state.lock().expect("trava do Synapse falso");
        let authorization = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok());
        let record = Record {
            method: method.to_string(),
            path: uri.path().to_owned(),
            query: uri.query().unwrap_or_default().to_owned(),
            body: serde_json::from_str(&body).unwrap_or(Value::Null),
            authorization_matches: authorization
                .map(|value| value == format!("Bearer {}", guard.expected_token))
                .unwrap_or(false),
        };
        guard.records.push(record);
        guard.script.pop_front().unwrap_or_else(|| {
            panic!(
                "chegou requisição sem resposta no roteiro: {method} {}",
                uri.path()
            )
        })
    };
    let status = StatusCode::from_u16(response.status).expect("status válido");
    (
        status,
        [("content-type", "application/json")],
        response.body.to_string(),
    )
        .into_response()
}

struct Scenario {
    state: Arc<Mutex<FakeState>>,
    users: VirtualUsers,
}

impl Scenario {
    fn enqueue(&self, response: FakeResponse) {
        self.state.lock().expect("trava").script.push_back(response);
    }

    fn records(&self) -> Vec<Record> {
        self.state.lock().expect("trava").records.clone()
    }

    fn total(&self) -> usize {
        self.records().len()
    }

    fn last(&self) -> Record {
        self.records()
            .pop()
            .expect("deveria ter havido alguma requisição")
    }
}

async fn scenario_with(cap: usize) -> Scenario {
    let state = Arc::new(Mutex::new(FakeState {
        expected_token: CANARIO.to_owned(),
        ..Default::default()
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
    let config = ClientConfig::new(
        format!("http://{address}"),
        "matrix-dev.local",
        Secret::new(CANARIO.to_owned()),
    )
    .expect("configuração do cliente");
    let client = MatrixClient::new(config).expect("cliente");
    let ids = GhostIds::new("ph", "matrix-dev.local").expect("ids");
    Scenario {
        state,
        users: VirtualUsers::new(client, ids, cap),
    }
}

async fn scenario() -> Scenario {
    scenario_with(DEFAULT_CACHE_CAP).await
}

fn id_of(prefix: &str, body: &str) -> String {
    format!("@ph_{prefix}_{body}:matrix-dev.local")
}

fn creation_record(id: &str) -> FakeResponse {
    FakeResponse::ok(json!({ "user_id": id }))
}

fn record_already_exists() -> FakeResponse {
    FakeResponse::error(
        400,
        json!({ "errcode": "M_USER_IN_USE", "error": "User ID already taken." }),
    )
}

fn profile_with(name: &str) -> FakeResponse {
    FakeResponse::ok(json!({ "displayname": name }))
}

fn profile_without_name() -> FakeResponse {
    FakeResponse::ok(json!({}))
}

fn applied_name() -> FakeResponse {
    FakeResponse::ok(json!({}))
}

#[tokio::test]
async fn new_record_applies_name_and_second_call_does_not_talk_to_synapse() {
    let c = scenario().await;
    let id = id_of("wa", "5511999999999");
    c.enqueue(creation_record(&id));
    c.enqueue(applied_name());

    let ghost = c
        .users
        .ensure_client("+55 (11) 99999-9999", "Maria")
        .await
        .expect("fantasma pronto");
    assert_eq!(ghost.user_id(), id);

    let records = c.records();
    assert_eq!(records.len(), 2, "registro + nome");
    assert_eq!(records[0].method, "POST");
    assert!(records[0].path.ends_with("/register"), "{records:?}");
    assert!(records[0].query.contains("kind=user"), "{records:?}");
    assert_eq!(
        records[0].body["username"],
        Value::from("ph_wa_5511999999999")
    );
    assert_eq!(
        records[0].body["type"],
        Value::from("m.login.application_service")
    );
    assert_eq!(records[0].body["inhibit_login"], Value::Bool(true));
    assert!(records[0].authorization_matches, "token fora do cabeçalho");

    assert_eq!(records[1].method, "PUT");
    assert!(records[1].path.ends_with("/displayname"), "{records:?}");
    assert_eq!(
        records[1].body["displayname"],
        Value::from("Maria (WhatsApp)")
    );
    assert!(
        records[1].query.contains("user_id=%40ph_wa_5511999999999")
            || records[1].query.contains("user_id=@ph_wa_5511999999999"),
        "disfarce errado: {}",
        records[1].query
    );

    let again = c
        .users
        .ensure_client("5511999999999", "Maria")
        .await
        .expect("fantasma pronto");
    assert_eq!(again, ghost);
    assert_eq!(c.total(), 2, "o cache deveria ter evitado a segunda rodada");
    assert_eq!(c.users.cache(), (1, DEFAULT_CACHE_CAP));
}

#[tokio::test]
async fn existing_ghost_does_not_rewrite_same_name() {
    let c = scenario().await;
    let id = id_of("wa", "5521988887777");
    c.enqueue(record_already_exists());
    c.enqueue(profile_with("João (WhatsApp)"));

    let ghost = c
        .users
        .ensure_client("5521988887777", "João")
        .await
        .expect("fantasma pronto");
    assert_eq!(ghost.user_id(), id);

    let records = c.records();
    assert_eq!(records.len(), 2, "registro + leitura de perfil, sem PUT");
    assert_eq!(records[0].path.rsplit('/').next(), Some("register"));
    assert_eq!(records[1].method, "GET");
    assert!(
        records[1].path.contains(&id),
        "leu o perfil de outro id: {}",
        records[1].path
    );
}

#[tokio::test]
async fn changed_name_is_rewritten_and_new_name_applied_once() {
    let c = scenario().await;
    let id = id_of("wa", "5511977776666");
    c.enqueue(record_already_exists());
    c.enqueue(profile_with("Maria Antiga (WhatsApp)"));
    c.enqueue(applied_name());
    c.enqueue(applied_name());

    c.users
        .ensure_client("5511977776666", "Maria")
        .await
        .expect("primeira rodada");
    assert_eq!(c.total(), 3, "registro + perfil + nome");

    c.users
        .ensure_client("5511977776666", "Maria Nova")
        .await
        .expect("segunda rodada");
    let records = c.records();
    assert_eq!(records.len(), 4, "só o nome mudou, só um PUT");
    let last = &records[3];
    assert_eq!(last.method, "PUT");
    assert!(last.path.ends_with("/displayname"), "{last:?}");
    assert_eq!(
        last.body["displayname"],
        Value::from("Maria Nova (WhatsApp)")
    );
    assert!(last.path.contains(&id));

    c.users
        .ensure_client("5511977776666", "Maria Nova")
        .await
        .expect("terceira rodada");
    assert_eq!(c.total(), 4);
}

#[tokio::test]
async fn old_ghost_without_name_receives_name() {
    let c = scenario().await;
    let id = id_of("wa", "5511944443333");
    c.enqueue(record_already_exists());
    c.enqueue(profile_without_name());
    c.enqueue(applied_name());

    c.users
        .ensure_client("5511944443333", "  Cliente Novo ")
        .await
        .expect("fantasma pronto");
    let records = c.records();
    assert_eq!(records.len(), 3, "registro + perfil + nome");
    assert_eq!(records[2].method, "PUT");
    assert!(records[2].path.contains(&id));
    assert_eq!(
        records[2].body["displayname"],
        Value::from("Cliente Novo (WhatsApp)")
    );
}

#[tokio::test]
async fn agent_uses_agent_name_and_same_cache_applies() {
    let c = scenario().await;
    let id_agent = "01912345-6789-7abc-8def-0123456789ab";
    let id = id_of("ag", id_agent);
    c.enqueue(creation_record(&id));
    c.enqueue(applied_name());

    let ghost = c
        .users
        .ensure_agent(id_agent, "Suporte")
        .await
        .expect("fantasma do agente");
    assert_eq!(ghost.user_id(), id);
    let records = c.records();
    assert_eq!(
        records[0].body["username"],
        Value::from(format!("ph_ag_{id_agent}"))
    );
    assert_eq!(records[1].body["displayname"], Value::from("Suporte"));

    c.enqueue(applied_name());
    c.users
        .ensure_agent(id_agent, "")
        .await
        .expect("agente sem nome");
    assert_eq!(c.last().body["displayname"], Value::from(id_agent));
}

#[tokio::test]
async fn cache_respects_cap_in_real_life() {
    let c = scenario_with(2).await;
    let phones = ["5511900000001", "5511900000002", "5511900000003"];
    for phone in phones {
        let id = id_of("wa", phone);
        c.enqueue(creation_record(&id));
        c.enqueue(applied_name());
        c.users
            .ensure_client(phone, "Cliente")
            .await
            .expect("fantasma pronto");
    }
    assert_eq!(c.total(), 6);
    assert_eq!(c.users.cache(), (2, 2), "o cache passou do teto");

    c.enqueue(record_already_exists());
    c.enqueue(profile_with("Cliente (WhatsApp)"));
    c.users
        .ensure_client(phones[0], "Cliente")
        .await
        .expect("fantasma pronto");
    assert_eq!(c.total(), 8, "o fantasma expirado deveria ser reconferido");
    assert_eq!(c.users.cache().0, 2);

    c.users
        .ensure_client(phones[2], "Cliente")
        .await
        .expect("fantasma pronto");
    assert_eq!(
        c.total(),
        8,
        "o fantasma que ficou no cache foi reconsultado"
    );
}

#[tokio::test]
async fn id_diverging_from_synapse_is_error_without_entering_cache() {
    let c = scenario().await;
    c.enqueue(creation_record("@ph_wa_outro:matrix-dev.local"));

    let error = c
        .users
        .ensure_client("5511955554444", "Maria")
        .await
        .expect_err("id divergente deveria falhar");
    assert_eq!(error, GhostsError::DivergentUser);
    assert_eq!(c.total(), 1, "não pode ter tentado aplicar nome");
    assert_eq!(c.users.cache().0, 0);
}

#[tokio::test]
async fn transient_failure_propagates_without_entering_cache() {
    let c = scenario().await;
    c.enqueue(FakeResponse::error(503, json!({ "error": "indisponível" })));

    let error = c
        .users
        .ensure_client("5511933332222", "Maria")
        .await
        .expect_err("503 deveria falhar");
    match error {
        GhostsError::Call(ref call) => {
            assert_eq!(call.classification(), Classification::Transient);
            assert!(!call.to_string().contains(CANARIO), "vazou token: {call}");
        }
        other => panic!("esperado erro de chamada: {other:?}"),
    }
    assert_eq!(c.users.cache().0, 0);
}

#[test]
fn built_ghost_does_not_show_secret_in_debug() {
    let ids = GhostIds::new("ph", "matrix-dev.local").expect("ids");
    let ghost: Ghost = ids.client("5511999999999").expect("cliente");
    let debug = format!("{ghost:?}");
    assert!(
        debug.contains("@ph_wa_5511999999999:matrix-dev.local"),
        "{debug}"
    );
    assert!(!debug.contains(CANARIO));
}
