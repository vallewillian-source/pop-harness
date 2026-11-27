use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use poph_matrix::AssetError;
use poph_matrix::client::{ClientConfig, MatrixClient};
use poph_matrix::error::Classification;
use poph_types::Secret;

const CANARIO: &str = "CANARIO-MATRIX-MX44-nao-pode-vazar";

const USER: &str = "@ph_ag_01912345-0000-7000-8000-000000000000:matrix-dev.local";

const HUMAN: &str = "@ana:matrix-dev.local";

const NAMESPACE: &str = "@ph_";

#[derive(Clone, Debug)]
enum FakeResponse {
    Content(Vec<u8>),
    Error(u16, &'static str),
    Redirecionar(&'static str),
    WithoutSize(Vec<u8>),
}

#[derive(Clone, Debug)]
struct Record {
    path: String,
    query: HashMap<String, String>,
    authorization_present: bool,
}

#[derive(Clone, Default)]
struct Fake {
    news: Arc<Mutex<Vec<FakeResponse>>>,
    olds: Arc<Mutex<Vec<FakeResponse>>>,
    records: Arc<Mutex<Vec<Record>>>,
}

fn query_of(uri: &Uri) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(query) = uri.query() else {
        return map;
    };
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(decode(key), decode(value));
    }
    map
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(value) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            output.push(value);
            i += 3;
            continue;
        }
        output.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

async fn attend(
    State(fake): State<Fake>,
    Path((_server, _id)): Path<(String, String)>,
    uri: Uri,
    headers: HeaderMap,
    fresh: bool,
) -> Response {
    let query = query_of(&uri);
    fake.records.lock().unwrap().push(Record {
        path: if fresh {
            "nova".to_owned()
        } else {
            "antiga".to_owned()
        },
        query: query.clone(),
        authorization_present: headers.contains_key("authorization"),
    });
    if let Some(user) = query.get("user_id")
        && !user.starts_with(NAMESPACE)
    {
        return (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({
                "errcode": "M_FORBIDDEN",
                "error": "Application service cannot masquerade as this user"
            })
            .to_string(),
        )
            .into_response();
    }
    let queue = if fresh {
        fake.news.lock().unwrap().pop()
    } else {
        fake.olds.lock().unwrap().pop()
    };
    match queue {
        Some(FakeResponse::Content(bytes)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/octet-stream")],
            bytes,
        )
            .into_response(),
        Some(FakeResponse::WithoutSize(bytes)) => {
            let metade = bytes.len() / 2;
            let parts: Vec<Result<axum::body::Bytes, std::io::Error>> = vec![
                Ok(axum::body::Bytes::from(bytes[..metade].to_vec())),
                Ok(axum::body::Bytes::from(bytes[metade..].to_vec())),
            ];
            Response::builder()
                .status(StatusCode::OK)
                .body(axum::body::Body::from_stream(futures::stream::iter(parts)))
                .expect("resposta em fluxo")
        }
        Some(FakeResponse::Error(status, errcode)) => (
            StatusCode::from_u16(status).expect("status válido"),
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({ "errcode": errcode, "error": "recusado" }).to_string(),
        )
            .into_response(),
        Some(FakeResponse::Redirecionar(destination)) => Response::builder()
            .status(StatusCode::TEMPORARY_REDIRECT)
            .header(header::LOCATION, destination)
            .body(axum::body::Body::empty())
            .expect("redirecionamento"),
        None => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({ "errcode": "M_NOT_FOUND", "error": "Not found" }).to_string(),
        )
            .into_response(),
    }
}

async fn new_attendant(
    state: State<Fake>,
    path: Path<(String, String)>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    attend(state, path, uri, headers, true).await
}

async fn old_attendant(
    state: State<Fake>,
    path: Path<(String, String)>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    attend(state, path, uri, headers, false).await
}

async fn scenario(news: Vec<FakeResponse>, olds: Vec<FakeResponse>) -> (MatrixClient, Fake) {
    let fake = Fake::default();
    *fake.news.lock().unwrap() = news.into_iter().rev().collect();
    *fake.olds.lock().unwrap() = olds.into_iter().rev().collect();
    let router = Router::new()
        .route(
            "/_matrix/client/v1/media/download/{server}/{id}",
            get(new_attendant),
        )
        .route(
            "/_matrix/media/v3/download/{server}/{id}",
            get(old_attendant),
        )
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let address = listener.local_addr().expect("endereço");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let client = MatrixClient::new(
        ClientConfig::new(
            format!("http://{address}"),
            "matrix-dev.local".to_owned(),
            Secret::new(CANARIO.to_owned()),
        )
        .expect("configuração do cliente")
        .with_test_timeout(),
    )
    .expect("cliente");
    (client, fake)
}

trait WithDeadline {
    fn with_test_timeout(self) -> ClientConfig;
}

impl WithDeadline for ClientConfig {
    fn with_test_timeout(mut self) -> ClientConfig {
        self.timeout = Duration::from_secs(5);
        self
    }
}

fn records(fake: &Fake) -> Vec<Record> {
    fake.records.lock().unwrap().clone()
}

#[tokio::test]
async fn downloads_via_new_route_masked_and_without_redirect() {
    let (client, fake) = scenario(
        vec![FakeResponse::Content(vec![0x89, b'P', b'N', b'G'])],
        vec![],
    )
    .await;

    let bytes = client
        .download_asset(USER, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect("baixa");
    assert_eq!(bytes, vec![0x89, b'P', b'N', b'G']);

    let records = records(&fake);
    assert_eq!(records.len(), 1, "uma chamada só: {records:?}");
    assert_eq!(records[0].path, "nova");
    assert!(records[0].authorization_present);
    assert_eq!(
        records[0].query.get("user_id").map(String::as_str),
        Some(USER),
        "disfarce de usuário na consulta"
    );
    assert_eq!(
        records[0].query.get("allow_redirect").map(String::as_str),
        Some("false")
    );
}

#[tokio::test]
async fn mask_outside_application_service_namespace_is_refused() {
    let (client, fake) = scenario(vec![FakeResponse::Content(b"conteudo".to_vec())], vec![]).await;

    let error = client
        .download_asset(HUMAN, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect_err("humano está fora do namespace do AS");
    assert!(
        !error.transient(),
        "recusa por namespace é definitiva: {error}"
    );
    assert!(
        error.to_string().contains("403"),
        "o erro diz o status: {error}"
    );
    let records = records(&fake);
    assert_eq!(
        records.len(),
        1,
        "uma tentativa só (não volta na rota antiga nem no reenvio): {records:?}"
    );
    assert_eq!(
        records[0].query.get("user_id").map(String::as_str),
        Some(HUMAN)
    );

    let (client, _fake) = scenario(vec![FakeResponse::Content(b"conteudo".to_vec())], vec![]).await;
    let bytes = client
        .download_asset(USER, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect("o fantasma do agente baixa");
    assert_eq!(bytes, b"conteudo");
}

#[tokio::test]
async fn missing_new_route_falls_back_to_old_and_content_returns() {
    let (client, fake) = scenario(
        vec![FakeResponse::Error(404, "M_NOT_FOUND")],
        vec![FakeResponse::Content(b"conteudo".to_vec())],
    )
    .await;

    let bytes = client
        .download_asset(USER, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect("baixa pela rota antiga");
    assert_eq!(bytes, b"conteudo");
    let paths: Vec<String> = records(&fake)
        .into_iter()
        .map(|record| record.path)
        .collect();
    assert_eq!(paths, vec!["nova".to_owned(), "antiga".to_owned()]);
}

#[tokio::test]
async fn missing_attachment_keeps_main_route_response() {
    let (client, _fake) = scenario(
        vec![FakeResponse::Error(404, "M_NOT_FOUND")],
        vec![FakeResponse::Error(404, "M_NOT_FOUND")],
    )
    .await;

    let error = client
        .download_asset(USER, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect_err("não existe");
    assert!(!error.transient(), "mídia apagada não volta: {error}");
    assert_eq!(error.classification(), Classification::Definitive);
}

#[tokio::test]
async fn declared_size_above_cap_refuses_without_reading_content() {
    let large = vec![0u8; 4096];
    let (client, _fake) = scenario(vec![FakeResponse::Content(large)], vec![]).await;

    let error = client
        .download_asset(USER, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect_err("passa do teto");
    match error {
        AssetError::Large { bytes, cap } => {
            assert_eq!(cap, 1024);
            assert!(bytes > cap, "o tamanho declarado é o do arquivo: {bytes}");
        }
        other => panic!("esperava recusa por tamanho, veio {other:?}"),
    }
}

#[tokio::test]
async fn without_declared_size_cap_applies_on_chunked_read() {
    let large = vec![7u8; 8192];
    let (client, _fake) = scenario(vec![FakeResponse::WithoutSize(large)], vec![]).await;

    let error = client
        .download_asset(USER, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect_err("passa do teto na leitura");
    assert!(
        matches!(error, AssetError::Large { cap: 1024, .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn redirect_is_not_followed() {
    let (client, fake) = scenario(
        vec![FakeResponse::Redirecionar(
            "http://outro-servidor.invalid/arquivo",
        )],
        vec![],
    )
    .await;

    let error = client
        .download_asset(USER, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect_err("redirecionamento é recusado");
    assert!(!error.transient(), "seguir não resolveria: {error}");
    assert!(
        error.to_string().contains("redirecionamento"),
        "o erro diz o que aconteceu: {error}"
    );
    assert_eq!(
        records(&fake).len(),
        1,
        "o endereço de destino não é visitado"
    );
}

#[tokio::test]
async fn error_5xx_returns_next_cycle_and_4xx_does_not() {
    let (client, _fake) = scenario(vec![FakeResponse::Error(503, "M_UNKNOWN")], vec![]).await;
    let error = client
        .download_asset(USER, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect_err("Synapse fora");
    assert!(error.transient(), "{error}");

    let (client, _fake) = scenario(vec![FakeResponse::Error(403, "M_FORBIDDEN")], vec![]).await;
    let error = client
        .download_asset(USER, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect_err("sem permissão");
    assert!(!error.transient(), "{error}");
}

#[tokio::test]
async fn empty_attachment_and_invalid_address_are_own_refusals() {
    let (client, _fake) = scenario(vec![FakeResponse::Content(Vec::new())], vec![]).await;
    assert_eq!(
        client
            .download_asset(USER, "mxc://matrix-dev.local/abc123", 1024)
            .await,
        Err(AssetError::Empty)
    );

    let (client, fake) = scenario(vec![], vec![]).await;
    assert!(matches!(
        client
            .download_asset(USER, "http://exemplo/arquivo", 1024)
            .await,
        Err(AssetError::Address(_))
    ));
    assert!(records(&fake).is_empty(), "nada foi pedido ao servidor");
}

#[tokio::test]
async fn no_download_error_carries_token_into_message() {
    let (client, _fake) = scenario(
        vec![FakeResponse::Error(500, "M_UNKNOWN")],
        vec![FakeResponse::Error(500, "M_UNKNOWN")],
    )
    .await;
    let error = client
        .download_asset(USER, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect_err("falhou");
    let text = format!("{error} {error:?}");
    assert!(!text.contains(CANARIO), "token no erro: {text}");
}
