use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_matrix::client::{ClientConfig, CreateRoom, MatrixClient};
use poph_matrix::error::{CallError, Classification};
use poph_types::Secret;
use serde_json::{Value, json};

const CANARIO: &str = "CANARIO-MATRIX-MX09-nao-pode-vazar";

#[derive(Clone, Debug)]
struct FakeResponse {
    status: u16,
    body: String,
    delay: Duration,
}

impl FakeResponse {
    fn ok(body: Value) -> Self {
        Self {
            status: 200,
            body: body.to_string(),
            delay: Duration::ZERO,
        }
    }

    fn error(status: u16, body: Value) -> Self {
        Self {
            status,
            body: body.to_string(),
            delay: Duration::ZERO,
        }
    }

    fn raw(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            body: body.into(),
            delay: Duration::ZERO,
        }
    }

    fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

#[derive(Clone, Debug, Default)]
struct Record {
    method: String,
    path: String,
    query: Option<String>,
    authorization_present: bool,
    authorization_matches: bool,
    body: String,
}

#[derive(Default)]
struct FakeState {
    script: VecDeque<FakeResponse>,
    default: Option<FakeResponse>,
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
            query: uri.query().map(str::to_owned),
            authorization_present: authorization.is_some(),
            authorization_matches: authorization
                .map(|v| v == format!("Bearer {}", guard.expected_token))
                .unwrap_or(false),
            body,
        };
        guard.records.push(record);
        guard
            .script
            .pop_front()
            .or_else(|| guard.default.clone())
            .unwrap_or_else(|| FakeResponse::ok(json!({})))
    };

    if !response.delay.is_zero() {
        tokio::time::sleep(response.delay).await;
    }
    let status = StatusCode::from_u16(response.status).expect("status válido");
    (
        status,
        [("content-type", "application/json")],
        response.body,
    )
        .into_response()
}

struct Scenario {
    state: Arc<Mutex<FakeState>>,
    client: MatrixClient,
}

impl Scenario {
    fn enqueue(&self, response: FakeResponse) {
        self.state.lock().expect("trava").script.push_back(response);
    }

    fn records(&self) -> Vec<Record> {
        self.state.lock().expect("trava").records.clone()
    }

    fn record(&self, index: usize) -> Record {
        self.records()
            .get(index)
            .cloned()
            .unwrap_or_else(|| panic!("não houve a requisição {index}"))
    }
}

async fn scenario_with(token: &str, timeout: Duration) -> Scenario {
    let state = Arc::new(Mutex::new(FakeState {
        expected_token: token.to_owned(),
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
    let base = format!("http://{address}");
    let config = ClientConfig::new(base, "matrix-dev.local", Secret::new(token.to_owned()))
        .expect("configuração do cliente")
        .with_timeout(timeout);
    let client = MatrixClient::new(config).expect("cliente");
    Scenario { state, client }
}

async fn scenario() -> Scenario {
    scenario_with(CANARIO, Duration::from_secs(5)).await
}

fn query(record: &Record) -> String {
    record.query.clone().unwrap_or_default()
}

#[tokio::test]
async fn create_room_masks_user_and_sends_token_only_in_header() {
    let c = scenario().await;
    c.enqueue(FakeResponse::ok(
        json!({"room_id": "!sala1:matrix-dev.local"}),
    ));

    let room = c
        .client
        .create_room(
            "@ph_as_bot:matrix-dev.local",
            &CreateRoom {
                room_alias_name: Some("ph_c_1".to_owned()),
                name: Some("Cliente Teste".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("sala criada");
    assert_eq!(room, "!sala1:matrix-dev.local");

    let r = c.record(0);
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/_matrix/client/v3/createRoom");
    assert!(query(&r).contains("user_id="), "sem disfarce: {r:?}");
    assert!(query(&r).contains("ph_as_bot"), "usuário errado: {r:?}");
    assert!(
        r.authorization_present && r.authorization_matches,
        "token não foi no cabeçalho"
    );
    for field in [&r.path, &query(&r), &r.body] {
        assert!(
            !field.contains(CANARIO),
            "token vazou na requisição: {field}"
        );
    }
    let body: Value = serde_json::from_str(&r.body).expect("corpo JSON");
    assert_eq!(body["room_alias_name"], "ph_c_1");
    assert_eq!(body["name"], "Cliente Teste");
}

#[tokio::test]
async fn invite_enter_and_list_members() {
    let c = scenario().await;

    c.enqueue(FakeResponse::ok(json!({})));
    c.client
        .invite(
            "@ph_as_bot:matrix-dev.local",
            "!s:matrix-dev.local",
            "@ph_wa_1:matrix-dev.local",
        )
        .await
        .expect("convite");
    let invitation = c.record(0);
    assert_eq!(invitation.method, "POST");
    assert_eq!(
        invitation.path,
        "/_matrix/client/v3/rooms/!s:matrix-dev.local/invite"
    );
    assert!(query(&invitation).contains("user_id="));
    let body: Value = serde_json::from_str(&invitation.body).expect("corpo JSON");
    assert_eq!(body["user_id"], "@ph_wa_1:matrix-dev.local");

    c.enqueue(FakeResponse::ok(json!({"room_id": "!s:matrix-dev.local"})));
    let entered = c
        .client
        .enter("@ph_wa_1:matrix-dev.local", "!s:matrix-dev.local")
        .await
        .expect("entrada");
    assert_eq!(entered, "!s:matrix-dev.local");
    let entry = c.record(1);
    assert_eq!(entry.path, "/_matrix/client/v3/join/!s:matrix-dev.local");
    assert!(
        query(&entry).contains("ph_wa_1"),
        "entrou como quem? {entry:?}"
    );

    c.enqueue(FakeResponse::ok(json!({
        "joined": {
            "@ph_as_bot:matrix-dev.local": {"display_name": "Bot"},
            "@ph_wa_1:matrix-dev.local": {"display_name": "Cliente (WhatsApp)"}
        }
    })));
    let members = c
        .client
        .members("@ph_as_bot:matrix-dev.local", "!s:matrix-dev.local")
        .await
        .expect("membros");
    assert_eq!(members.len(), 2);
    assert_eq!(
        members["@ph_wa_1:matrix-dev.local"].display_name.as_deref(),
        Some("Cliente (WhatsApp)")
    );
    assert_eq!(
        c.record(2).path,
        "/_matrix/client/v3/rooms/!s:matrix-dev.local/joined_members"
    );
}

#[tokio::test]
async fn event_carries_txn_id_in_path_and_returns_event_id() {
    let c = scenario().await;
    c.enqueue(FakeResponse::ok(json!({"event_id": "$evento1"})));

    let event = c
        .client
        .send_event(
            "@ph_wa_1:matrix-dev.local",
            "!s:matrix-dev.local",
            "m.room.message",
            "txn-1",
            &json!({"msgtype": "m.text", "body": "olá"}),
        )
        .await
        .expect("envio");
    assert_eq!(event, "$evento1");
    let r = c.record(0);
    assert_eq!(r.method, "PUT");
    assert_eq!(
        r.path,
        "/_matrix/client/v3/rooms/!s:matrix-dev.local/send/m.room.message/txn-1"
    );
    let body: Value = serde_json::from_str(&r.body).expect("corpo JSON");
    assert_eq!(body["body"], "olá");
}

#[tokio::test]
async fn state_writes_with_key_and_reads_back() {
    let c = scenario().await;

    c.enqueue(FakeResponse::ok(json!({"event_id": "$estado1"})));
    let event = c
        .client
        .send_state(
            "@ph_as_bot:matrix-dev.local",
            "!s:matrix-dev.local",
            "ph.atendimento.metadata",
            None,
            &json!({"status": "active"}),
        )
        .await
        .expect("estado escrito");
    assert_eq!(event, "$estado1");
    assert_eq!(
        c.record(0).path,
        "/_matrix/client/v3/rooms/!s:matrix-dev.local/state/ph.atendimento.metadata"
    );

    c.enqueue(FakeResponse::ok(json!({"status": "active"})));
    let value = c
        .client
        .read_state(
            "@ph_as_bot:matrix-dev.local",
            "!s:matrix-dev.local",
            "ph.atendimento.metadata",
            Some(""),
        )
        .await
        .expect("estado lido");
    assert_eq!(value["status"], "active");
    assert_eq!(c.record(1).method, "GET");
}

#[tokio::test]
async fn child_space_uses_room_as_key_and_server_name_as_via() {
    let c = scenario().await;
    c.enqueue(FakeResponse::ok(json!({"event_id": "$filho"})));

    c.client
        .space_child(
            "@ph_as_bot:matrix-dev.local",
            "!espaco:matrix-dev.local",
            "!sala:matrix-dev.local",
            Some("10"),
        )
        .await
        .expect("m.space.child");

    let r = c.record(0);
    assert_eq!(
        r.path,
        "/_matrix/client/v3/rooms/!espaco:matrix-dev.local/state/m.space.child/!sala:matrix-dev.local"
    );
    let body: Value = serde_json::from_str(&r.body).expect("corpo JSON");
    assert_eq!(body["via"], json!(["matrix-dev.local"]));
    assert_eq!(body["order"], "10");
}

#[tokio::test]
async fn profile_and_display_name() {
    let c = scenario().await;

    c.enqueue(FakeResponse::ok(
        json!({"displayname": "Cliente (WhatsApp)"}),
    ));
    let profile = c
        .client
        .profile("@ph_wa_1:matrix-dev.local")
        .await
        .expect("perfil");
    assert_eq!(profile.displayname.as_deref(), Some("Cliente (WhatsApp)"));
    let read = c.record(0);
    assert_eq!(read.method, "GET");
    assert_eq!(
        read.path,
        "/_matrix/client/v3/profile/@ph_wa_1:matrix-dev.local"
    );
    assert!(read.query.is_none(), "leitura não precisa de disfarce");

    c.enqueue(FakeResponse::ok(json!({})));
    c.client
        .set_display_name(
            "@ph_wa_1:matrix-dev.local",
            "@ph_wa_1:matrix-dev.local",
            "Cliente (WhatsApp)",
        )
        .await
        .expect("nome definido");
    let writing = c.record(1);
    assert_eq!(writing.method, "PUT");
    assert_eq!(
        writing.path,
        "/_matrix/client/v3/profile/@ph_wa_1:matrix-dev.local/displayname"
    );
    assert!(query(&writing).contains("ph_wa_1"));
    let body: Value = serde_json::from_str(&writing.body).expect("corpo JSON");
    assert_eq!(body["displayname"], "Cliente (WhatsApp)");
}

#[tokio::test]
async fn virtual_user_registration_uses_m_login_application_service() {
    let c = scenario().await;
    c.enqueue(FakeResponse::ok(
        json!({"user_id": "@ph_wa_1:matrix-dev.local"}),
    ));

    let user = c.client.register_user("ph_wa_1").await.expect("registro");
    assert_eq!(user, "@ph_wa_1:matrix-dev.local");

    let r = c.record(0);
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/_matrix/client/v3/register");
    assert!(query(&r).contains("kind=user"), "sem kind=user: {r:?}");
    assert!(r.query.as_deref().unwrap_or_default().contains("kind=user"));
    let body: Value = serde_json::from_str(&r.body).expect("corpo JSON");
    assert_eq!(body["type"], "m.login.application_service");
    assert_eq!(body["username"], "ph_wa_1");
    assert_eq!(body["inhibit_login"], true);
}

#[tokio::test]
async fn respects_limit_429_once_and_measures_wait() {
    let c = scenario().await;
    c.enqueue(FakeResponse::error(
        429,
        json!({
            "errcode": "M_LIMIT_EXCEEDED",
            "error": "Too Many Requests",
            "retry_after_ms": 150
        }),
    ));
    c.enqueue(FakeResponse::ok(
        json!({"room_id": "!sala:matrix-dev.local"}),
    ));

    let beginning = Instant::now();
    let room = c
        .client
        .create_room("@ph_as_bot:matrix-dev.local", &CreateRoom::default())
        .await
        .expect("segunda tentativa deu certo");
    let elapsed = beginning.elapsed();

    assert_eq!(room, "!sala:matrix-dev.local");
    assert!(
        elapsed >= Duration::from_millis(140),
        "não esperou o retry_after_ms: {elapsed:?}"
    );
    assert_eq!(c.records().len(), 2, "deveria ter repetido uma vez");
}

#[tokio::test]
async fn limit_429_without_declared_wait_does_not_retry() {
    let c = scenario().await;
    c.enqueue(FakeResponse::error(
        429,
        json!({"errcode": "M_LIMIT_EXCEEDED", "error": "Too Many Requests"}),
    ));

    let error = c
        .client
        .create_room("@ph_as_bot:matrix-dev.local", &CreateRoom::default())
        .await
        .expect_err("429 sem espera volta para o laço");
    assert!(error.transient());
    assert_eq!(error.retry_after_ms(), None);
    assert_eq!(c.records().len(), 1);
}

#[tokio::test]
async fn limit_429_with_long_wait_does_not_block_client() {
    let c = scenario().await;
    c.enqueue(FakeResponse::error(
        429,
        json!({
            "errcode": "M_LIMIT_EXCEEDED",
            "error": "Too Many Requests",
            "retry_after_ms": 120_000
        }),
    ));

    let beginning = Instant::now();
    let error = c
        .client
        .create_room("@ph_as_bot:matrix-dev.local", &CreateRoom::default())
        .await
        .expect_err("espera longa fica para o laço");
    assert!(
        beginning.elapsed() < Duration::from_secs(2),
        "o cliente esperou demais"
    );
    assert_eq!(error.retry_after_ms(), Some(120_000));
    assert_eq!(c.records().len(), 1);
}

#[tokio::test]
async fn error_5xx_is_transient() {
    let c = scenario().await;
    c.enqueue(FakeResponse::error(
        503,
        json!({"errcode": "M_UNKNOWN", "error": "indisponível"}),
    ));

    let error = c
        .client
        .create_room("@ph_as_bot:matrix-dev.local", &CreateRoom::default())
        .await
        .expect_err("503 é falha");
    match &error {
        CallError::Response { status, .. } => assert_eq!(*status, 503),
        other => panic!("esperado erro de resposta, veio {other:?}"),
    }
    assert!(error.transient());
    assert_eq!(c.records().len(), 1, "5xx não repete dentro do cliente");
}

#[tokio::test]
async fn error_4xx_is_definitive_and_leaks_neither_token_nor_whole_body() {
    let c = scenario().await;
    let message = "You are not allowed to invite this user ".repeat(20);
    c.enqueue(FakeResponse::error(
        403,
        json!({"errcode": "M_FORBIDDEN", "error": message}),
    ));

    let error = c
        .client
        .invite(
            "@ph_as_bot:matrix-dev.local",
            "!s:matrix-dev.local",
            "@x:dev",
        )
        .await
        .expect_err("403 é definitivo");
    assert_eq!(error.classification(), Classification::Definitive);
    match &error {
        CallError::Response {
            status,
            errcode,
            message,
            ..
        } => {
            assert_eq!(*status, 403);
            assert_eq!(errcode.as_deref(), Some("M_FORBIDDEN"));
            assert!(
                message.chars().count() <= 161,
                "mensagem sem recorte: {message}"
            );
        }
        other => panic!("esperado erro de resposta, veio {other:?}"),
    }
    assert!(!error.to_string().contains(CANARIO));
}

#[tokio::test]
async fn deadline_returns_transient_error() {
    let c = scenario_with(CANARIO, Duration::from_millis(100)).await;
    c.enqueue(
        FakeResponse::ok(json!({"room_id": "!s:dev"})).with_delay(Duration::from_millis(600)),
    );

    let error = c
        .client
        .create_room("@ph_as_bot:matrix-dev.local", &CreateRoom::default())
        .await
        .expect_err("deveria estourar o prazo");
    match error {
        CallError::Deadline { ms, .. } => assert_eq!(ms, 100),
        other => panic!("esperado prazo, veio {other:?}"),
    }
    assert!(error.transient());
}

#[tokio::test]
async fn network_failure_is_transient() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let address = listener.local_addr().expect("endereço");
    drop(listener);

    let config = ClientConfig::new(
        format!("http://{address}"),
        "matrix-dev.local",
        Secret::new(CANARIO.to_owned()),
    )
    .expect("configuração")
    .with_timeout(Duration::from_secs(2));
    let client = MatrixClient::new(config).expect("cliente");
    let error = client
        .create_room("@ph_as_bot:matrix-dev.local", &CreateRoom::default())
        .await
        .expect_err("sem servidor não há resposta");
    assert!(matches!(error, CallError::Network { .. }), "{error:?}");
    assert!(error.transient());
    assert!(!error.to_string().contains(CANARIO));
}

#[tokio::test]
async fn malformed_response_is_definitive() {
    let c = scenario().await;
    c.enqueue(FakeResponse::raw(200, "isto não é JSON"));
    let error = c
        .client
        .create_room("@ph_as_bot:matrix-dev.local", &CreateRoom::default())
        .await
        .expect_err("corpo que não é JSON");
    match error {
        CallError::InvalidResponse { reason, .. } => {
            assert!(reason.contains("JSON"), "{reason}")
        }
        other => panic!("esperado resposta inválida, veio {other:?}"),
    }

    c.enqueue(FakeResponse::ok(json!({"nada": true})));
    let error = c
        .client
        .create_room("@ph_as_bot:matrix-dev.local", &CreateRoom::default())
        .await
        .expect_err("sem room_id");
    match error {
        CallError::InvalidResponse { reason, .. } => {
            assert!(reason.contains("room_id"), "{reason}")
        }
        other => panic!("esperado resposta inválida, veio {other:?}"),
    }
}

#[tokio::test]
async fn canary_token_never_appears_in_debug_display_log_or_error() {
    let c = scenario().await;

    let debug_client = format!("{:?}", c.client);
    assert!(
        debug_client.contains("[oculto]"),
        "Debug sem máscara: {debug_client}"
    );
    assert!(
        !debug_client.contains(CANARIO),
        "vazou no Debug: {debug_client}"
    );

    c.enqueue(FakeResponse::error(
        401,
        json!({"errcode": "M_UNKNOWN_TOKEN", "error": "Invalid access token"}),
    ));
    let error = c
        .client
        .create_room("@ph_as_bot:matrix-dev.local", &CreateRoom::default())
        .await
        .expect_err("401 é erro");
    assert!(
        !format!("{error:?}").contains(CANARIO),
        "vazou no Debug do erro"
    );
    assert!(
        !error.to_string().contains(CANARIO),
        "vazou no Display do erro"
    );

    let captured = Arc::new(Mutex::new(Vec::<u8>::new()));
    let assinante = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer({
            let captured = Arc::clone(&captured);
            move || Writer(Arc::clone(&captured))
        })
        .finish();
    tracing::subscriber::with_default(assinante, || {
        tracing::info!(client = ?c.client, error = %error, "chamada ao Synapse falhou");
    });
    let log = String::from_utf8(captured.lock().expect("trava").clone()).expect("utf-8");
    assert!(
        log.contains("chamada ao Synapse falhou"),
        "log não saiu: {log}"
    );
    assert!(!log.contains(CANARIO), "vazou no log: {log}");
}

struct Writer(Arc<Mutex<Vec<u8>>>);

impl Write for Writer {
    fn write(&mut self, givens: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("trava").extend_from_slice(givens);
        Ok(givens.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
