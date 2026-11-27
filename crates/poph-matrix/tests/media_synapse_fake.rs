use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_matrix::asset::{self, ATTACHMENT_BLOCK, AttachmentForAsset, MSGTYPE_FILE, MSGTYPE_IMAGE};
use poph_matrix::client::{ClientConfig, MatrixClient};
use poph_types::Secret;
use serde_json::{Value, json};

const CANARIO: &str = "CANARIO-MX43-nao-pode-vazar";

const SERVER: &str = "matrix-dev.local";

#[derive(Clone, Debug)]
struct Upload {
    path: String,
    query: String,
    mime: String,
    bytes: Vec<u8>,
    authorization: String,
}

#[derive(Clone, Debug)]
struct Event {
    room: String,
    kind: String,
    txn: String,
    query: String,
    body: Value,
}

#[derive(Default)]
struct FakeState {
    uploads: Mutex<Vec<Upload>>,
    events: Mutex<BTreeMap<(String, String), Event>>,
    old_route_gone: Mutex<bool>,
    refuse_send: Mutex<bool>,
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

async fn attend(
    State(state): State<Arc<FakeState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let authorization = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if authorization != format!("Bearer {CANARIO}") {
        return reply(
            401,
            json!({ "errcode": "M_UNKNOWN_TOKEN", "error": "Token inválido" }),
        );
    }
    let query = uri.query().unwrap_or_default().to_owned();
    let mime = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let path = uri.path().to_owned();

    if method == Method::POST && path.ends_with("/upload") && path.contains("/media/") {
        if path.starts_with("/_matrix/media/v3") && *state.old_route_gone.lock().unwrap() {
            return reply(
                404,
                json!({ "errcode": "M_UNRECOGNIZED", "error": "Unrecognized request" }),
            );
        }
        let n = {
            let mut uploads = state.uploads.lock().unwrap();
            uploads.push(Upload {
                path: path.clone(),
                query: query.clone(),
                mime,
                bytes: body.to_vec(),
                authorization,
            });
            uploads.len()
        };
        return reply(
            200,
            json!({ "content_uri": format!("mxc://{SERVER}/midia{n}") }),
        );
    }

    let parts: Vec<String> = path
        .split('/')
        .filter(|piece| !piece.is_empty())
        .map(decode)
        .collect();
    if method == Method::PUT
        && let Some(position) = parts.iter().position(|part| part == "send")
        && parts.len() == position + 3
    {
        if *state.refuse_send.lock().unwrap() {
            return reply(
                403,
                json!({ "errcode": "M_FORBIDDEN", "error": "recusado" }),
            );
        }
        let room = parts[position - 1].clone();
        let kind = parts[position + 1].clone();
        let txn = parts[position + 2].clone();
        let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let mut events = state.events.lock().unwrap();
        let event = events
            .entry((room.clone(), txn.clone()))
            .or_insert_with(|| Event {
                room,
                kind,
                txn,
                query,
                body,
            })
            .clone();
        return reply(200, json!({ "event_id": format!("${}", event.txn) }));
    }

    reply(404, json!({ "errcode": "M_UNRECOGNIZED", "error": "?" }))
}

async fn raise(state: Arc<FakeState>) -> (String, tokio::task::JoinHandle<()>) {
    let app = Router::new().fallback(attend).with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta livre");
    let address = listener.local_addr().expect("endereço").to_string();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{address}"), task)
}

fn client(base: &str) -> MatrixClient {
    MatrixClient::new(
        ClientConfig::new(base, SERVER, Secret::new(CANARIO.to_owned())).expect("configuração"),
    )
    .expect("cliente")
}

fn ghost() -> poph_matrix::users::Ghost {
    poph_matrix::users::GhostIds::new("ph", SERVER)
        .expect("ids")
        .client("5521988887777")
        .expect("fantasma do cliente")
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nconteudo-de-imagem";
const PDF: &[u8] = b"%PDF-1.4\nlaudo\n%%EOF\n";

fn asset_of(name: &str, mime: &str, bytes: &[u8]) -> asset::Asset {
    let size = bytes.len() as u64;
    asset::decide(&AttachmentForAsset {
        id: &format!("anx-{name}"),
        name,
        mime,
        size_bytes: size,
        source: "whatsapp",
    })
    .expect("decide a mídia")
}

#[tokio::test]
async fn image_and_pdf_upload_through_client_ghost_and_become_event_with_block() {
    let state = Arc::new(FakeState::default());
    let (base, task) = raise(Arc::clone(&state)).await;
    let client = client(&base);
    let sender = ghost();
    let room = "!sala1:matrix-dev.local";

    let image = asset_of("foto.jpg", "image/jpeg", PNG);
    let event_image = asset::send(&client, &sender, room, &image, PNG)
        .await
        .expect("imagem sobe e vira evento");

    let pdf = asset_of("laudo.pdf", "application/pdf", PDF);
    let event_pdf = asset::send(&client, &sender, room, &pdf, PDF)
        .await
        .expect("pdf sobe e vira evento");
    assert_ne!(event_image, event_pdf, "dois eventos diferentes");

    let uploads = state.uploads.lock().unwrap().clone();
    assert_eq!(uploads.len(), 2);
    assert_eq!(uploads[0].path, "/_matrix/media/v3/upload");
    assert_eq!(
        uploads[0].query,
        "filename=foto.jpg&user_id=%40ph_wa_5521988887777%3Amatrix-dev.local"
    );
    assert_eq!(uploads[0].mime, "image/jpeg");
    assert_eq!(uploads[0].bytes, PNG);
    assert_eq!(uploads[1].mime, "application/pdf");
    assert_eq!(uploads[1].bytes, PDF);
    assert!(
        uploads[0].authorization.starts_with("Bearer "),
        "o token vai no `Authorization`"
    );

    let events = state.events.lock().unwrap().clone();
    let image_in_room = events
        .get(&(room.to_owned(), asset::asset_txn("anx-foto.jpg")))
        .expect("evento da imagem")
        .clone();
    assert_eq!(image_in_room.kind, "m.room.message");
    assert_eq!(image_in_room.room, room);
    assert!(
        image_in_room.query.contains("user_id="),
        "o evento sai com o disfarce do fantasma"
    );
    assert_eq!(image_in_room.body["msgtype"], json!(MSGTYPE_IMAGE));
    assert_eq!(image_in_room.body["body"], json!("foto.jpg"));
    assert_eq!(
        image_in_room.body["url"],
        json!("mxc://matrix-dev.local/midia1")
    );
    assert_eq!(
        image_in_room.body[ATTACHMENT_BLOCK],
        json!({ "id": "anx-foto.jpg", "source": "whatsapp" })
    );

    let pdf_in_room = events
        .get(&(room.to_owned(), asset::asset_txn("anx-laudo.pdf")))
        .expect("evento do pdf")
        .clone();
    assert_eq!(pdf_in_room.body["msgtype"], json!(MSGTYPE_FILE));
    assert_eq!(
        pdf_in_room.body["url"],
        json!("mxc://matrix-dev.local/midia2")
    );
    assert_eq!(
        pdf_in_room.body[ATTACHMENT_BLOCK]["id"],
        json!("anx-laudo.pdf")
    );

    assert!(
        !format!("{events:?}").contains(CANARIO),
        "token nunca no evento"
    );
    task.abort();
}

#[tokio::test]
async fn repeating_same_attachment_creates_no_second_event() {
    let state = Arc::new(FakeState::default());
    let (base, task) = raise(Arc::clone(&state)).await;
    let client = client(&base);
    let sender = ghost();
    let room = "!sala1:matrix-dev.local";
    let asset = asset_of("foto.png", "image/png", PNG);

    let first = asset::send(&client, &sender, room, &asset, PNG)
        .await
        .expect("primeira");
    let second = asset::send(&client, &sender, room, &asset, PNG)
        .await
        .expect("repetição");
    assert_eq!(
        first.event, second.event,
        "o mesmo `txnId` devolve o mesmo evento"
    );
    assert_ne!(first.mxc, second.mxc, "cada upload gera um `mxc` novo");
    assert_eq!(state.events.lock().unwrap().len(), 1);
    task.abort();
}

#[tokio::test]
async fn old_route_gone_falls_back_to_new_upload_route() {
    let state = Arc::new(FakeState::default());
    *state.old_route_gone.lock().unwrap() = true;
    let (base, task) = raise(Arc::clone(&state)).await;
    let client = client(&base);
    let asset = asset_of("laudo.pdf", "application/pdf", PDF);

    asset::send(&client, &ghost(), "!sala1:matrix-dev.local", &asset, PDF)
        .await
        .expect("a rota nova aceita");

    let uploads = state.uploads.lock().unwrap().clone();
    assert_eq!(uploads.len(), 1, "só a rota nova registrou o arquivo");
    assert_eq!(uploads[0].path, "/_matrix/client/v1/media/upload");
    task.abort();
}

#[tokio::test]
async fn definitive_event_refusal_returns_as_definitive_error() {
    let state = Arc::new(FakeState::default());
    *state.refuse_send.lock().unwrap() = true;
    let (base, task) = raise(Arc::clone(&state)).await;
    let client = client(&base);
    let asset = asset_of("foto.jpg", "image/jpeg", PNG);

    let error = asset::send(&client, &ghost(), "!sala1:matrix-dev.local", &asset, PNG)
        .await
        .expect_err("403 no envio");
    assert_eq!(
        error.classification(),
        poph_matrix::error::Classification::Definitive
    );
    let text = format!("{error}");
    assert!(!text.contains(CANARIO));
    assert!(!text.contains("conteudo-de-imagem"));
    task.abort();
}

#[tokio::test]
async fn wrong_token_does_not_upload_file() {
    let state = Arc::new(FakeState::default());
    let (base, task) = raise(Arc::clone(&state)).await;
    let client = MatrixClient::new(
        ClientConfig::new(&base, SERVER, Secret::new("outro-token".to_owned()))
            .expect("configuração"),
    )
    .expect("cliente");
    let asset = asset_of("foto.jpg", "image/jpeg", PNG);

    let error = asset::send(&client, &ghost(), "!sala1:matrix-dev.local", &asset, PNG)
        .await
        .expect_err("401");
    assert_eq!(
        error.classification(),
        poph_matrix::error::Classification::Definitive
    );
    assert!(state.uploads.lock().unwrap().is_empty());
    task.abort();
}
