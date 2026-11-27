use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::{
    AsReceiver, CAP_BODY, DeliveryError, DeliveryFuture, EventDelivery, ServerState,
};
use poph_store::Store;
use poph_types::config::GlobalTokenConfig;
use poph_types::{HarnessConfig, MatrixConfig, Sensitive};
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-mx16";

const HS_TOKEN: &str = "CANARIO-HS-TOKEN-mx16-9d2b-TAO-SECRETO";

const PREFIX: &str = "ph";
const SERVER_NAME: &str = "matrix-dev.local";

fn hash_argon2(value: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

#[derive(Debug, Clone)]
struct Call {
    transaction: String,
    events: Vec<Value>,
}

#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<Call>>,
    fail: AtomicBool,
}

impl Recorder {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("trava").clone()
    }
}

impl EventDelivery for Recorder {
    fn deliver(&self, transaction: &str, events: Vec<Value>) -> DeliveryFuture<'_> {
        if self.fail.load(Ordering::SeqCst) {
            return Box::pin(async { Err(DeliveryError::new("banco indisponível")) });
        }
        self.calls.lock().expect("trava").push(Call {
            transaction: transaction.to_owned(),
            events,
        });
        Box::pin(async { Ok(()) })
    }
}

struct Scenario {
    base: String,
    client: Client,
    folder: PathBuf,
    delivery: Arc<Recorder>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Scenario {
    async fn enabled(label: &str) -> Self {
        let delivery = Arc::new(Recorder::default());
        let receiver = AsReceiver::new(
            PREFIX,
            SERVER_NAME,
            Sensitive::new(HS_TOKEN.to_owned()),
            Arc::clone(&delivery) as Arc<dyn EventDelivery>,
        );
        Self::build(label, true, Some(receiver), delivery).await
    }

    async fn disabled(label: &str) -> Self {
        Self::build(label, false, None, Arc::new(Recorder::default())).await
    }

    async fn build(
        label: &str,
        matrix_enabled: bool,
        receiver: Option<AsReceiver>,
        delivery: Arc<Recorder>,
    ) -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let folder = std::env::temp_dir().join(format!(
            "poph-matrix-as-{label}-{}-{stamp}",
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
            enabled: matrix_enabled,
            homeserver_url: "http://127.0.0.1:18008".to_owned(),
            server_name: SERVER_NAME.to_owned(),
            prefix: PREFIX.to_owned(),
            ..Default::default()
        });
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let mut state = ServerState::new(store, config).expect("estado do servidor");
        if let Some(receiver) = receiver {
            state = state.with_matrix_as(Arc::new(receiver));
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, signal) = oneshot::channel();
        tokio::spawn(async move {
            let _ = poph_server::serve_at(listener, state, async {
                let _ = signal.await;
            })
            .await;
        });

        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("cliente HTTP");
        let scenario = Self {
            base: format!("http://{endereco}"),
            client,
            folder,
            delivery,
            desligar: Some(desligar),
        };
        scenario.wait_until_up().await;
        scenario
    }

    async fn wait_until_up(&self) {
        let started_at = Instant::now();
        loop {
            match self
                .client
                .get(format!("{}/healthz", self.base))
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => return,
                _ if started_at.elapsed() < Duration::from_secs(10) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                other => panic!("servidor não subiu: {other:?}"),
            }
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
    ) -> (StatusCode, String) {
        let mut request_builder = self.client.request(method, self.url(path));
        if let Some(token) = token {
            request_builder = request_builder.bearer_auth(token);
        }
        let response = request_builder.send().await.expect("pedido HTTP");
        let status = response.status();
        (status, response.text().await.unwrap_or_default())
    }

    async fn transaction(
        &self,
        path: &str,
        token: Option<&str>,
        body: String,
    ) -> (StatusCode, String) {
        let mut request_builder = self
            .client
            .put(self.url(path))
            .header("content-type", "application/json")
            .body(body);
        if let Some(token) = token {
            request_builder = request_builder.bearer_auth(token);
        }
        let response = request_builder.send().await.expect("pedido HTTP");
        let status = response.status();
        (status, response.text().await.unwrap_or_default())
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        let _ = fs::remove_dir_all(&self.folder);
    }
}

fn transaction_body(event: &str) -> String {
    json!({
        "events": [
            {
                "type": "m.room.message",
                "room_id": "!sala:matrix-dev.local",
                "sender": "@atendente:matrix-dev.local",
                "event_id": event,
                "content": { "msgtype": "m.text", "body": "oi" }
            }
        ]
    })
    .to_string()
}

const ROUTE: &str = "/_matrix/app/v1/transactions/1";

#[tokio::test]
async fn valid_transaction_answers_200_after_delivering_events() {
    let c = Scenario::enabled("valida").await;
    let (status, body) = c
        .transaction(ROUTE, Some(HS_TOKEN), transaction_body("$a"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.trim(), "{}", "o Synapse espera corpo vazio de objeto");

    let calls = c.delivery.calls();
    assert_eq!(calls.len(), 1, "a entrega tinha de ser chamada uma vez");
    assert_eq!(calls[0].transaction, "1");
    assert_eq!(calls[0].events.len(), 1);
    assert_eq!(calls[0].events[0]["event_id"], json!("$a"));
}

#[tokio::test]
async fn variants_without_v1_answer_like_the_canonical() {
    let c = Scenario::enabled("variantes").await;
    for (index, path) in ["/_matrix/app/transactions/v1menos", "/transactions/raiz"]
        .into_iter()
        .enumerate()
    {
        let (status, _) = c
            .transaction(
                path,
                Some(HS_TOKEN),
                transaction_body(&format!("$v{index}")),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "rota `{path}`");
    }
    assert_eq!(c.delivery.calls().len(), 2);
}

#[tokio::test]
async fn routes_with_matrix_prefix_answer_like_those_without() {
    let c = Scenario::enabled("prefixo").await;

    let (status, body) = c
        .transaction(
            "/matrix/_matrix/app/v1/transactions/9",
            Some(HS_TOKEN),
            transaction_body("$p1"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "transação pela rota com prefixo");
    assert_eq!(body.trim(), "{}");
    assert_eq!(c.delivery.calls().len(), 1);
    assert_eq!(c.delivery.calls()[0].transaction, "9");

    let (status, _) = c
        .transaction(
            "/_matrix/app/v1/transactions/9",
            Some(HS_TOKEN),
            transaction_body("$p1"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        c.delivery.calls().len(),
        1,
        "o `txnId` já processado não é entregue de novo por outro caminho"
    );

    let (status, _) = c
        .transaction(
            "/matrix/_matrix/app/transactions/10",
            Some(HS_TOKEN),
            transaction_body("$p2"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "rota com prefixo e sem /v1");
    assert_eq!(c.delivery.calls().len(), 2);

    for path in [
        "/matrix/_matrix/app/v1/transactions/11",
        "/_matrix/app/v1/transactions/11",
    ] {
        let (status, body) = c.transaction(path, None, transaction_body("$p3")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "rota `{path}`");
        assert!(!body.contains(HS_TOKEN), "vazou token: {body}");
    }

    let (status, _) = c
        .request(Method::POST, "/matrix/_matrix/app/v1/ping", Some(HS_TOKEN))
        .await;
    assert_eq!(status, StatusCode::OK, "ping com prefixo");
    let (status, _) = c
        .request(
            Method::GET,
            "/matrix/_matrix/app/v1/users/@ph_wa_5511999999999:matrix-dev.local",
            Some(HS_TOKEN),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "consulta de usuário com prefixo");
}

#[tokio::test]
async fn ping_users_and_alias_answer_by_namespace() {
    let c = Scenario::enabled("consultas").await;

    let (status, body) = c
        .request(Method::POST, "/_matrix/app/v1/ping", Some(HS_TOKEN))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.trim(), "{}");

    let (status, _) = c
        .request(
            Method::GET,
            "/_matrix/app/v1/users/@ph_wa_5511999999999:matrix-dev.local",
            Some(HS_TOKEN),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "fantasma do prefixo é da instância");

    let (status, body) = c
        .request(
            Method::GET,
            "/_matrix/app/v1/users/@wa_5511999999999:matrix-dev.local",
            Some(HS_TOKEN),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "prefixo de outro registro");
    assert!(
        body.contains("M_NOT_FOUND"),
        "o 404 leva o errcode do Matrix: {body}"
    );

    let (status, _) = c
        .request(
            Method::GET,
            "/_matrix/app/v1/rooms/%23ph_c_0123456789abcdef:matrix-dev.local",
            Some(HS_TOKEN),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "alias do módulo");

    let (status, _) = c
        .request(
            Method::GET,
            "/_matrix/app/v1/rooms/%23dep_vendas:matrix-dev.local",
            Some(HS_TOKEN),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "alias #dep_ não é exclusivo");
}

#[tokio::test]
async fn no_token_gives_401_and_wrong_token_gives_403_without_processing() {
    let c = Scenario::enabled("auth").await;

    let (status, body) = c.transaction(ROUTE, None, transaction_body("$a")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!body.contains(HS_TOKEN), "vazou token na resposta: {body}");

    let (status, body) = c
        .transaction(ROUTE, Some("token-errado"), transaction_body("$a"))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!body.contains(HS_TOKEN), "vazou token na resposta: {body}");

    let (status, _) = c
        .request(
            Method::GET,
            "/_matrix/app/v1/users/@ph_wa_1:matrix-dev.local",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    assert!(c.delivery.calls().is_empty(), "nada podia ser processado");
}

#[tokio::test]
async fn canary_token_does_not_appear_in_response() {
    let c = Scenario::enabled("canario").await;
    let mut bodies = Vec::new();
    for (method, path, token) in [
        (Method::PUT, ROUTE, Some(HS_TOKEN)),
        (Method::PUT, ROUTE, Some("errado")),
        (Method::PUT, ROUTE, None),
        (
            Method::GET,
            "/_matrix/app/v1/users/@ph_wa_1:matrix-dev.local",
            Some("errado"),
        ),
        (Method::POST, "/_matrix/app/v1/ping", Some("errado")),
    ] {
        let (_, body) = match method {
            Method::PUT => c.transaction(path, token, transaction_body("$a")).await,
            _ => c.request(method, path, token).await,
        };
        bodies.push(body);
    }
    for body in bodies {
        assert!(!body.contains(HS_TOKEN), "o hs_token vazou: {body}");
    }
}

fn valid_body_over_cap() -> String {
    let enchimento = "g".repeat(CAP_BODY);
    json!({
        "events": [
            {
                "type": "m.room.message",
                "room_id": "!sala:matrix-dev.local",
                "sender": "@atendente:matrix-dev.local",
                "event_id": "$grande",
                "content": { "msgtype": "m.text", "body": enchimento }
            }
        ]
    })
    .to_string()
}

#[tokio::test]
async fn huge_body_answers_200_without_processing() {
    let c = Scenario::enabled("gigante").await;
    let enorme = valid_body_over_cap();
    assert!(
        enorme.len() > CAP_BODY,
        "o corpo do teste precisa passar do teto"
    );
    let (status, _) = c.transaction(ROUTE, Some(HS_TOKEN), enorme).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "corpo grande não pode dar 4xx: o Synapse marcaria o AS como fora do ar"
    );
    assert!(c.delivery.calls().is_empty(), "nada podia ser processado");
}

#[tokio::test]
async fn large_body_without_content_length_answers_200_without_processing() {
    let c = Scenario::enabled("gigante-stream").await;
    let body = valid_body_over_cap().into_bytes();
    let middle = body.len() / 2;
    let chunks = vec![
        Ok::<_, std::io::Error>(axum::body::Bytes::from(body[..middle].to_vec())),
        Ok(axum::body::Bytes::from(body[middle..].to_vec())),
    ];
    let response = c
        .client
        .put(c.url(ROUTE))
        .bearer_auth(HS_TOKEN)
        .body(reqwest::Body::wrap_stream(futures::stream::iter(chunks)))
        .send()
        .await
        .expect("pedido HTTP");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(c.delivery.calls().is_empty(), "nada podia ser processado");
}

#[tokio::test]
async fn invalid_json_answers_200_without_processing() {
    let c = Scenario::enabled("json").await;
    for body in ["isto não é json".to_owned(), "{}".to_owned(), String::new()] {
        let (status, _) = c.transaction(ROUTE, Some(HS_TOKEN), body).await;
        assert_eq!(status, StatusCode::OK);
    }
    assert!(c.delivery.calls().is_empty());
}

#[tokio::test]
async fn retransmission_of_same_transaction_processes_once() {
    let c = Scenario::enabled("repetida").await;
    for _ in 0..3 {
        let (status, _) = c
            .transaction(ROUTE, Some(HS_TOKEN), transaction_body("$a"))
            .await;
        assert_eq!(status, StatusCode::OK);
    }
    let calls = c.delivery.calls();
    assert_eq!(
        calls.len(),
        1,
        "o Synapse reenviou a mesma transação: tinha de processar uma vez"
    );

    let (status, _) = c
        .transaction(
            "/_matrix/app/v1/transactions/2",
            Some(HS_TOKEN),
            transaction_body("$b"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(c.delivery.calls().len(), 2);
}

#[tokio::test]
async fn delivery_failure_gives_5xx_and_resend_processes() {
    let c = Scenario::enabled("falha").await;

    c.delivery.fail.store(true, Ordering::SeqCst);
    let (status, body) = c
        .transaction(ROUTE, Some(HS_TOKEN), transaction_body("$a"))
        .await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "erro transitório nosso tem de ser 5xx para o Synapse reenviar"
    );
    assert!(
        status.is_server_error(),
        "um 4xx aqui bloquearia o AS por minutos"
    );
    assert!(!body.contains(HS_TOKEN));

    c.delivery.fail.store(false, Ordering::SeqCst);
    let (status, _) = c
        .transaction(ROUTE, Some(HS_TOKEN), transaction_body("$a"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        c.delivery.calls().len(),
        1,
        "o reenvio da transação que falhou tem de ser processado"
    );
}

#[tokio::test]
async fn disabled_module_gives_404_on_four_routes() {
    let c = Scenario::disabled("desligado").await;

    let routes = [
        (Method::PUT, "/_matrix/app/v1/transactions/1"),
        (
            Method::GET,
            "/_matrix/app/v1/users/@ph_wa_1:matrix-dev.local",
        ),
        (
            Method::GET,
            "/_matrix/app/v1/rooms/%23ph_c_1:matrix-dev.local",
        ),
        (Method::POST, "/_matrix/app/v1/ping"),
        (Method::PUT, "/_matrix/app/transactions/1"),
        (Method::PUT, "/transactions/raiz"),
    ];
    for (method, path) in routes {
        let (status, body) = match method {
            Method::PUT => {
                c.transaction(path, Some(HS_TOKEN), transaction_body("$a"))
                    .await
            }
            _ => c.request(method.clone(), path, Some(HS_TOKEN)).await,
        };
        assert_eq!(status, StatusCode::NOT_FOUND, "rota `{path}`");
        if method == Method::GET {
            assert!(
                body.contains("not_found"),
                "o GET do módulo desligado não pode cair na página do Console: {body}"
            );
        }
    }

    let (status, body) = c
        .request(Method::GET, "/_matrix/inexistente", Some(HS_TOKEN))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!body.contains("<html"), "veio a página do Console: {body}");

    assert!(c.delivery.calls().is_empty());
}
