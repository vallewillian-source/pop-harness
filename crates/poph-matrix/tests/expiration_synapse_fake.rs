use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_matrix::client::{ClientConfig, MatrixClient};
use poph_matrix::error::{AssetError, CallError, Classification};
use poph_matrix::expiration::{
    self, AssetToExpire, CATEGORY_FIELD, EXPIRED_BLOCK, NAME_FIELD, SENT_AT_FIELD,
};
use poph_matrix::users::{Ghost, GhostIds};
use poph_types::Secret;
use serde_json::{Value, json};

const AS_TOKEN: &str = "as-token-do-teste";
const ADMIN_TOKEN: &str = "CANARIO-ADMIN-mx46-7c1d-TAO-SECRETO";
const SERVER: &str = "matrix-dev.local";
const PREFIX: &str = "ph";
const ROOM: &str = "!sala-mx46:matrix-dev.local";
const EVENT: &str = "$midia-mx46:matrix-dev.local";
const ATTACHMENT: &str = "anx-mx46";
const MXC: &str = "mxc://matrix-dev.local/abc123";

#[derive(Debug, Clone)]
struct Writing {
    room: String,
    event: String,
    txn: String,
    as_token: Option<String>,
    body: Value,
}

#[derive(Clone, Default)]
struct Fake {
    redactions: Arc<Mutex<Vec<Writing>>>,
    deleted_items: Arc<Mutex<Vec<(String, String)>>>,
    authorizations: Arc<Mutex<Vec<String>>>,
    calls: Arc<AtomicUsize>,
    status_do_delete: Arc<Mutex<Option<u16>>>,
}

fn reply(status: u16, body: Value) -> Response {
    (
        StatusCode::from_u16(status).expect("status válido"),
        [("content-type", "application/json")],
        body.to_string(),
    )
        .into_response()
}

async fn attend(
    State(fake): State<Fake>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    fake.calls.fetch_add(1, Ordering::SeqCst);
    let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let query: HashMap<String, String> = uri
        .query()
        .map(|raw| {
            raw.split('&')
                .filter_map(|pair| pair.split_once('='))
                .map(|(key, value)| (key.to_owned(), decode(value)))
                .collect()
        })
        .unwrap_or_default();
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();

    if method == Method::DELETE && uri.path().starts_with("/_synapse/admin/v1/media/") {
        let parts: Vec<&str> = uri
            .path()
            .trim_start_matches("/_synapse/admin/v1/media/")
            .split('/')
            .collect();
        if parts.len() != 2 {
            return reply(400, json!({ "error": "caminho de mídia inválido" }));
        }
        let status = fake.status_do_delete.lock().expect("trava").to_owned();
        fake.authorizations
            .lock()
            .expect("trava")
            .push(authorization.clone());
        if let Some(status) = status {
            return match status {
                404 => reply(404, json!({ "error": "Unknown media" })),
                403 => reply(403, json!({ "error": "You are not a server admin" })),
                other => reply(other, json!({ "error": "indisponível" })),
            };
        }
        if authorization != format!("Bearer {ADMIN_TOKEN}") {
            return reply(403, json!({ "error": "You are not a server admin" }));
        }
        fake.deleted_items
            .lock()
            .expect("trava")
            .push((parts[0].to_owned(), parts[1].to_owned()));
        return reply(200, json!({ "deleted_media": [parts[1]] }));
    }

    if method == Method::PUT && uri.path().contains("/redact/") {
        let parts: Vec<&str> = uri
            .path()
            .trim_start_matches("/_matrix/client/v3/rooms/")
            .split('/')
            .collect();
        if parts.len() != 4 || parts[1] != "redact" {
            return reply(404, json!({ "errcode": "M_UNRECOGNIZED" }));
        }
        if authorization != format!("Bearer {AS_TOKEN}") {
            return reply(401, json!({ "errcode": "M_UNKNOWN_TOKEN" }));
        }
        let mut redactions = fake.redactions.lock().expect("trava");
        if !redactions
            .iter()
            .any(|r| r.room == parts[0] && r.txn == parts[3])
        {
            redactions.push(Writing {
                room: parts[0].to_owned(),
                event: parts[2].to_owned(),
                txn: parts[3].to_owned(),
                as_token: query.get("user_id").cloned(),
                body,
            });
        }
        let position = redactions.len();
        return reply(
            200,
            json!({ "event_id": format!("$redacao-{position}:matrix-dev.local") }),
        );
    }

    reply(
        404,
        json!({ "errcode": "M_UNRECOGNIZED", "error": "rota não existe no Synapse falso" }),
    )
}

struct Synapse {
    url: String,
    fake: Fake,
    disable: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Synapse {
    async fn raise() -> Self {
        let fake = Fake::default();
        let app = Router::new().fallback(attend).with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let url = format!("http://{}", listener.local_addr().expect("endereço"));
        let (disable, signal) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = signal.await;
                })
                .await;
        });
        Self {
            url,
            fake,
            disable: Some(disable),
        }
    }

    fn client(&self) -> MatrixClient {
        MatrixClient::new(
            ClientConfig::new(self.url.clone(), SERVER.to_owned(), secret(AS_TOKEN))
                .expect("config do cliente"),
        )
        .expect("cliente do Matrix")
    }

    fn token_admin(&self) -> Secret {
        secret(ADMIN_TOKEN)
    }

    fn redactions(&self) -> Vec<Writing> {
        self.fake.redactions.lock().expect("trava").clone()
    }

    fn deleted_items(&self) -> Vec<(String, String)> {
        self.fake.deleted_items.lock().expect("trava").clone()
    }

    fn authorizations(&self) -> Vec<String> {
        self.fake.authorizations.lock().expect("trava").clone()
    }

    fn calls(&self) -> usize {
        self.fake.calls.load(Ordering::SeqCst)
    }

    fn refuse_delete_with(&self, status: Option<u16>) {
        *self.fake.status_do_delete.lock().expect("trava") = status;
    }
}

impl Drop for Synapse {
    fn drop(&mut self) {
        if let Some(disable) = self.disable.take() {
            let _ = disable.send(());
        }
    }
}

fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
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

fn secret(value: &str) -> Secret {
    Secret::new(value.to_owned())
}

fn asset() -> AssetToExpire<'static> {
    AssetToExpire {
        id: ATTACHMENT,
        name: "laudo.pdf",
        mime: "application/pdf",
        category: "document",
        size_bytes: 2_048,
        sent_at: 1_797_900_000_000,
    }
}

fn agent_ghost() -> Ghost {
    GhostIds::new(PREFIX, SERVER)
        .expect("ids dos fantasmas")
        .agent("agt_1")
        .expect("fantasma do agente")
}

#[tokio::test]
async fn redaction_goes_as_agent_ghost_with_frontend_block() {
    let synapse = Synapse::raise().await;
    let client = synapse.client();
    let agent = agent_ghost();

    let redaction_event = expiration::redact(&client, &agent, ROOM, EVENT, &asset())
        .await
        .expect("redige");
    assert_eq!(redaction_event, "$redacao-1:matrix-dev.local");

    let redactions = synapse.redactions();
    assert_eq!(redactions.len(), 1, "uma redação: {redactions:?}");
    let redaction = &redactions[0];
    assert_eq!(redaction.room, ROOM);
    assert_eq!(redaction.event, EVENT);
    assert_eq!(redaction.txn, expiration::redaction_txn(ATTACHMENT));
    assert_eq!(
        redaction.as_token.as_deref(),
        Some(agent.user_id()),
        "quem redige é o fantasma do agente (moderador da sala)"
    );
    let block = &redaction.body[EXPIRED_BLOCK];
    assert_eq!(block[NAME_FIELD], json!("laudo.pdf"));
    assert_eq!(block[CATEGORY_FIELD], json!("document"));
    assert_eq!(block[SENT_AT_FIELD], json!("2026-12-22T00:40:00Z"));
    assert_eq!(block["mimetype"], json!("application/pdf"));
    assert_eq!(block["size"], json!(2_048));
}

#[tokio::test]
async fn repeating_redaction_creates_no_second_redaction_in_synapse() {
    let synapse = Synapse::raise().await;
    let client = synapse.client();
    let agent = agent_ghost();

    for _ in 0..3 {
        expiration::redact(&client, &agent, ROOM, EVENT, &asset())
            .await
            .expect("redige");
    }
    assert_eq!(
        synapse.redactions().len(),
        1,
        "o mesmo txnId não duplica a redação (é o que o Synapse faz de verdade)"
    );
}

#[tokio::test]
async fn refused_redaction_returns_classified_without_token_in_error() {
    let synapse = Synapse::raise().await;
    let agent = agent_ghost();
    let intruso = MatrixClient::new(
        ClientConfig::new(
            synapse.url.clone(),
            SERVER.to_owned(),
            secret("outro-token"),
        )
        .expect("config"),
    )
    .expect("cliente");

    let error = expiration::redact(&intruso, &agent, ROOM, EVENT, &asset())
        .await
        .expect_err("401 é falha");
    assert_eq!(error.classification(), Classification::Definitive);
    assert!(
        !error.to_string().contains("outro-token"),
        "o erro não leva token: {error}"
    );
    assert!(synapse.redactions().is_empty());
}

#[tokio::test]
async fn delete_uses_admin_api_with_admin_token() {
    let synapse = Synapse::raise().await;
    let client = synapse.client();

    let deleted = expiration::delete(&client, &synapse.token_admin(), MXC)
        .await
        .expect("apaga");
    assert!(deleted, "o arquivo existia");
    assert_eq!(
        synapse.deleted_items(),
        vec![(SERVER.to_owned(), "abc123".to_owned())]
    );
    for authorization in synapse.authorizations() {
        assert_eq!(
            authorization,
            format!("Bearer {ADMIN_TOKEN}"),
            "o apagamento usa o token de administrador, não o do Application Service"
        );
    }
}

#[tokio::test]
async fn attachment_that_no_longer_exists_counts_as_deleted() {
    let synapse = Synapse::raise().await;
    let client = synapse.client();
    synapse.refuse_delete_with(Some(404));

    let deleted = expiration::delete(&client, &synapse.token_admin(), MXC)
        .await
        .expect("404 não é erro");
    assert!(!deleted, "já não existia: nada foi apagado agora");
}

#[tokio::test]
async fn no_permission_is_definitive_failure_and_unavailable_is_transient() {
    let synapse = Synapse::raise().await;
    let client = synapse.client();

    for (status, expected) in [
        (403, Classification::Definitive),
        (503, Classification::Transient),
        (429, Classification::Transient),
    ] {
        synapse.refuse_delete_with(Some(status));
        let error = expiration::delete(&client, &synapse.token_admin(), MXC)
            .await
            .expect_err("status de erro não é sucesso");
        assert_eq!(error.classification(), expected, "status {status}: {error}");

        let code = expiration::error_code(&error);
        assert!(
            code.len() <= 64 && !code.contains("mxc://") && !code.contains(ADMIN_TOKEN),
            "código do erro curto e sem segredo: {code}"
        );
    }
}

#[tokio::test]
async fn invalid_attachment_address_becomes_no_call() {
    let synapse = Synapse::raise().await;
    let client = synapse.client();
    let before = synapse.calls();

    let error = expiration::delete(&client, &synapse.token_admin(), "https://outro/servidor")
        .await
        .expect_err("endereço que não é mxc:// é recusado");
    assert!(matches!(error, AssetError::Address(_)), "{error:?}");
    assert_eq!(
        synapse.calls(),
        before,
        "nada foi chamado: o endereço é recusado antes da rede"
    );
}

#[test]
fn admin_token_never_appears_in_debug_display_or_error() {
    let token = secret(ADMIN_TOKEN);
    assert_eq!(format!("{token:?}"), "[oculto]");
    assert_eq!(format!("{token}"), "[oculto]");

    let error = CallError::Configuration(
        "o token de administrador do Synapse tem caractere que não cabe em cabeçalho HTTP"
            .to_owned(),
    );
    assert!(!error.to_string().contains(ADMIN_TOKEN));
}
