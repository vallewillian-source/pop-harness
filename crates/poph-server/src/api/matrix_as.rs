use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use futures::StreamExt;
use poph_matrix::MatrixModule;
use poph_types::Secret;
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::state::ServerState;

pub const CAP_BODY: usize = 1024 * 1024;

pub const CAP_DEFAULT_TRANSACTION: usize = 4096;

pub const BASE_V1: &str = "/_matrix/app/v1";

pub const BASE_WITHOUT_VERSION: &str = "/_matrix/app";

pub const BASES_WITH_PREFIX: [&str; 2] = ["/matrix/_matrix/app/v1", "/matrix/_matrix/app"];

const ERRCODE_NOT_FOUND: &str = "M_NOT_FOUND";

pub type DeliveryFuture<'a> = Pin<Box<dyn Future<Output = Result<(), DeliveryError>> + Send + 'a>>;

pub trait EventDelivery: Send + Sync {
    fn deliver(&self, transaction: &str, events: Vec<Value>) -> DeliveryFuture<'_>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("não consegui processar a transação do Application Service: {reason}")]
pub struct DeliveryError {
    pub reason: String,
}

impl DeliveryError {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

pub struct AsReceiver {
    hs_token: Secret,
    prefix: String,
    server_name: String,
    delivery: Arc<dyn EventDelivery>,
    processed: Mutex<Processed>,
}

impl AsReceiver {
    pub fn new(
        prefix: impl Into<String>,
        server_name: impl Into<String>,
        hs_token: Secret,
        delivery: Arc<dyn EventDelivery>,
    ) -> Self {
        Self::with_cap(
            prefix,
            server_name,
            hs_token,
            delivery,
            CAP_DEFAULT_TRANSACTION,
        )
    }

    pub fn with_cap(
        prefix: impl Into<String>,
        server_name: impl Into<String>,
        hs_token: Secret,
        delivery: Arc<dyn EventDelivery>,
        cap_transaction: usize,
    ) -> Self {
        Self {
            hs_token,
            prefix: prefix.into(),
            server_name: server_name.into(),
            delivery,
            processed: Mutex::new(Processed::new(cap_transaction)),
        }
    }

    pub fn of_module(module: MatrixModule, delivery: Arc<dyn EventDelivery>) -> Self {
        Self::new(
            module.config.prefix.clone(),
            module.config.server_name.clone(),
            module.secrets.hs_token.clone(),
            delivery,
        )
    }

    pub fn transaction_cap(&self) -> usize {
        self.processed_entries()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .cap
    }

    fn authorize(&self, header: &HeaderMap) -> Result<(), ApiError> {
        let value = header
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok());
        let Some(value) = value else {
            return Err(ApiError::unauthorized());
        };
        let (scheme, presented) = value.split_once(' ').unwrap_or((value, ""));
        if !scheme.eq_ignore_ascii_case("bearer") || presented.trim().is_empty() {
            return Err(ApiError::unauthorized());
        }
        let expected = self.hs_token.expose().as_bytes();
        if !constant_time_equal(expected, presented.trim().as_bytes()) {
            return Err(ApiError::forbidden("token do Application Service inválido"));
        }
        Ok(())
    }

    fn already_processed(&self, transaction: &str) -> bool {
        self.processed_entries()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .contains(transaction)
    }

    fn mark_processed(&self, transaction: &str) {
        self.processed_entries()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .register(transaction);
    }

    fn processed_entries(&self) -> &Mutex<Processed> {
        &self.processed
    }

    fn module_user(&self, user: &str) -> bool {
        self.no_namespace(user, '@')
    }

    fn module_alias(&self, alias: &str) -> bool {
        self.no_namespace(alias, '#')
    }

    fn no_namespace(&self, id: &str, prefix: char) -> bool {
        let Some(remainder) = id.strip_prefix(prefix) else {
            return false;
        };
        let Some((localpart, domain)) = remainder.split_once(':') else {
            return false;
        };
        let mark = format!("{}_", self.prefix);
        domain == self.server_name && localpart.len() > mark.len() && localpart.starts_with(&mark)
    }
}

fn constant_time_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        difference |= x ^ y;
    }
    difference == 0
}

impl fmt::Debug for AsReceiver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AsReceiver")
            .field("prefixo", &self.prefix)
            .field("server_name", &self.server_name)
            .field("hs_token", &self.hs_token)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct Processed {
    cap: usize,
    order: VecDeque<String>,
    set: HashSet<String>,
}

impl Processed {
    fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            order: VecDeque::new(),
            set: HashSet::new(),
        }
    }

    fn contains(&self, transaction: &str) -> bool {
        self.set.contains(transaction)
    }

    fn register(&mut self, transaction: &str) {
        if self.set.contains(transaction) {
            return;
        }
        while self.order.len() >= self.cap {
            match self.order.pop_front() {
                Some(old) => {
                    self.set.remove(&old);
                }
                None => break,
            }
        }
        self.order.push_back(transaction.to_owned());
        self.set.insert(transaction.to_owned());
    }
}

pub fn routes() -> Router<ServerState> {
    let mut routes = Router::new();
    let base_paths = [BASE_V1, BASE_WITHOUT_VERSION, ""]
        .into_iter()
        .chain(BASES_WITH_PREFIX);
    for base in base_paths {
        routes = routes
            .route(&format!("{base}/transactions/{{txn_id}}"), put(transaction))
            .route(&format!("{base}/users/{{user_id}}"), get(user))
            .route(&format!("{base}/rooms/{{room_alias}}"), get(alias))
            .route(&format!("{base}/ping"), post(ping));
    }
    routes.layer(DefaultBodyLimit::disable())
}

async fn transaction(
    State(state): State<ServerState>,
    Path(txn_id): Path<String>,
    header: HeaderMap,
    body: Body,
) -> Response {
    handle_transaction(state.matrix_as.as_deref(), &txn_id, &header, body).await
}

async fn user(
    State(state): State<ServerState>,
    Path(user): Path<String>,
    header: HeaderMap,
) -> Response {
    handle_user(state.matrix_as.as_deref(), &user, &header)
}

async fn alias(
    State(state): State<ServerState>,
    Path(alias): Path<String>,
    header: HeaderMap,
) -> Response {
    handle_alias(state.matrix_as.as_deref(), &alias, &header)
}

async fn ping(State(state): State<ServerState>, header: HeaderMap) -> Response {
    handle_ping(state.matrix_as.as_deref(), &header)
}

pub(crate) async fn handle_transaction(
    receiver: Option<&AsReceiver>,
    txn_id: &str,
    header: &HeaderMap,
    body: Body,
) -> Response {
    let Some(receiver) = receiver else {
        return module_disabled();
    };
    if let Err(error) = receiver.authorize(header) {
        return error.into_response();
    }

    let Some(bytes) = read_body(body, CAP_BODY).await else {
        tracing::warn!(
            txn = %txn_id,
            byte_cap = CAP_BODY,
            "transação do Application Service acima do limite: descartada sem processar"
        );
        return accepted();
    };
    let Some(events) = transaction_events(&bytes) else {
        tracing::warn!(
            txn = %txn_id,
            bytes = bytes.len(),
            "transação do Application Service malformada: descartada sem processar"
        );
        return accepted();
    };
    if receiver.already_processed(txn_id) {
        tracing::debug!(
            txn = %txn_id,
            "transação do Application Service repetida: respondendo sem reprocessar"
        );
        return accepted();
    }

    let quantity = events.len();
    match receiver.delivery.deliver(txn_id, events).await {
        Ok(()) => {
            receiver.mark_processed(txn_id);
            tracing::debug!(
                txn = %txn_id,
                events = quantity,
                "transação do Application Service processada"
            );
            accepted()
        }
        Err(error) => {
            tracing::warn!(
                txn = %txn_id,
                reason = %error.reason,
                "transação do Application Service não foi processada: 5xx para o Synapse reenviar"
            );
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "matrix_indisponivel",
                "não consegui processar a transação agora; o Synapse vai reenviar",
            )
            .into_response()
        }
    }
}

pub(crate) fn handle_user(
    receiver: Option<&AsReceiver>,
    user: &str,
    header: &HeaderMap,
) -> Response {
    let Some(receiver) = receiver else {
        return module_disabled();
    };
    if let Err(error) = receiver.authorize(header) {
        return error.into_response();
    }
    if receiver.module_user(user) {
        accepted()
    } else {
        not_mine("usuário")
    }
}

pub(crate) fn handle_alias(
    receiver: Option<&AsReceiver>,
    alias: &str,
    header: &HeaderMap,
) -> Response {
    let Some(receiver) = receiver else {
        return module_disabled();
    };
    if let Err(error) = receiver.authorize(header) {
        return error.into_response();
    }
    if receiver.module_alias(alias) {
        accepted()
    } else {
        not_mine("alias de sala")
    }
}

pub(crate) fn handle_ping(receiver: Option<&AsReceiver>, header: &HeaderMap) -> Response {
    let Some(receiver) = receiver else {
        return module_disabled();
    };
    if let Err(error) = receiver.authorize(header) {
        return error.into_response();
    }
    accepted()
}

fn accepted() -> Response {
    Json(json!({})).into_response()
}

fn module_disabled() -> Response {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "not_found",
        "o módulo Matrix está desligado nesta instância",
    )
    .into_response()
}

fn not_mine(that: &str) -> Response {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "not_found",
        format!("este {that} não pertence a esta instância"),
    )
    .with_details(json!({ "errcode": ERRCODE_NOT_FOUND }))
    .into_response()
}

async fn read_body(body: Body, cap: usize) -> Option<axum::body::Bytes> {
    let mut stream = body.into_data_stream();
    let mut data: Vec<u8> = Vec::new();
    while let Some(part) = stream.next().await {
        let part = part.ok()?;
        if data.len() + part.len() > cap {
            return None;
        }
        data.extend_from_slice(&part);
    }
    Some(data.into())
}

fn transaction_events(bytes: &[u8]) -> Option<Vec<Value>> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    match value.get("events") {
        Some(Value::Array(events)) => Some(events.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use poph_types::Sensitive;

    use super::*;

    const CANARIO: &str = "CANARIO-HS-TOKEN-4c1f-TAO-SECRETO";

    #[derive(Default)]
    struct Annotator {
        call: Mutex<Vec<(String, Vec<Value>)>>,
        fail: AtomicBool,
    }

    impl Annotator {
        fn call(&self) -> usize {
            self.call.lock().expect("trava").len()
        }
    }

    impl EventDelivery for Annotator {
        fn deliver(&self, transaction: &str, events: Vec<Value>) -> DeliveryFuture<'_> {
            if self.fail.load(Ordering::SeqCst) {
                return Box::pin(async { Err(DeliveryError::new("banco indisponível")) });
            }
            self.call
                .lock()
                .expect("trava")
                .push((transaction.to_owned(), events));
            Box::pin(async { Ok(()) })
        }
    }

    fn receiver(delivery: Arc<Annotator>, cap: usize) -> AsReceiver {
        AsReceiver::with_cap(
            "ph",
            "matrix-dev.local",
            Sensitive::new(CANARIO.to_owned()),
            delivery,
            cap,
        )
    }

    fn bearer_header(token: &str) -> HeaderMap {
        let mut header = HeaderMap::new();
        header.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().expect("cabeçalho"),
        );
        header
    }

    fn transaction_with(event: &str) -> Body {
        Body::from(
            json!({ "events": [{ "type": "m.room.message", "event_id": event }] }).to_string(),
        )
    }

    fn status(response: &Response) -> StatusCode {
        response.status()
    }

    static LOG_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn serial_logs() -> tokio::sync::MutexGuard<'static, ()> {
        LOG_SERIAL.lock().await
    }

    fn capture_logs(captured: Arc<Mutex<Vec<u8>>>) -> tracing::subscriber::DefaultGuard {
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || Writer(Arc::clone(&captured)))
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        tracing::callsite::rebuild_interest_cache();
        guard
    }

    #[test]
    fn memory_respects_cap_and_forgets_oldest() {
        let mut memory = Processed::new(2);
        memory.register("1");
        memory.register("2");
        assert!(memory.contains("1") && memory.contains("2"));
        memory.register("3");
        assert!(!memory.contains("1"), "a mais antiga devia ter saído");
        assert!(memory.contains("2") && memory.contains("3"));
        assert_eq!(memory.order.len(), 2, "a memória cresceu além do teto");
        memory.register("3");
        assert_eq!(memory.order.len(), 2);
    }

    #[test]
    fn cap_zero_means_one() {
        assert_eq!(Processed::new(0).cap, 1);
    }

    #[test]
    fn user_and_alias_namespace() {
        let annotator = Arc::new(Annotator::default());
        let receiver = receiver(Arc::clone(&annotator), 8);
        assert!(receiver.module_user("@ph_wa_5511999999999:matrix-dev.local"));
        assert!(receiver.module_user("@ph_ag_agt_1:matrix-dev.local"));
        assert!(receiver.module_alias("#ph_c_0123456789abcdef:matrix-dev.local"));
        for outside in [
            "@wa_5511999999999:matrix-dev.local",
            "@ph_wa_1:outro.servidor",
            "@ph_wa_1",
            "@ph_:matrix-dev.local",
            "ph_wa_1:matrix-dev.local",
        ] {
            assert!(
                !receiver.module_user(outside),
                "`{outside}` não podia ser do módulo"
            );
        }
        assert!(!receiver.module_alias("@ph_c_1:matrix-dev.local"));
    }

    #[tokio::test]
    async fn missing_token_is_401_wrong_token_is_403() {
        let _serial = serial_logs().await;
        let annotator = Arc::new(Annotator::default());
        let receiver = receiver(Arc::clone(&annotator), 8);

        let without_header = handle_transaction(
            Some(&receiver),
            "1",
            &HeaderMap::new(),
            transaction_with("$a"),
        )
        .await;
        assert_eq!(status(&without_header), StatusCode::UNAUTHORIZED);

        let mut bearer_only = HeaderMap::new();
        bearer_only.insert(header::AUTHORIZATION, "Bearer ".parse().expect("cabeçalho"));
        let empty =
            handle_transaction(Some(&receiver), "1", &bearer_only, transaction_with("$a")).await;
        assert_eq!(status(&empty), StatusCode::UNAUTHORIZED);

        for incorrect in [format!("{CANARIO}x"), "curto".to_owned()] {
            let response = handle_transaction(
                Some(&receiver),
                "1",
                &bearer_header(&incorrect),
                transaction_with("$a"),
            )
            .await;
            assert_eq!(
                status(&response),
                StatusCode::FORBIDDEN,
                "token `{incorrect}`"
            );
        }
        assert_eq!(annotator.call(), 0, "nada podia ter sido processado");
    }

    #[tokio::test]
    async fn valid_transaction_runs_once_and_replay_does_not_repeat() {
        let _serial = serial_logs().await;
        let annotator = Arc::new(Annotator::default());
        let receiver = receiver(Arc::clone(&annotator), 8);
        let header = bearer_header(CANARIO);

        let first =
            handle_transaction(Some(&receiver), "42", &header, transaction_with("$a")).await;
        assert_eq!(status(&first), StatusCode::OK);
        assert_eq!(annotator.call(), 1);

        let second =
            handle_transaction(Some(&receiver), "42", &header, transaction_with("$a")).await;
        assert_eq!(status(&second), StatusCode::OK);
        assert_eq!(annotator.call(), 1, "a repetição não podia reprocessar");

        let other =
            handle_transaction(Some(&receiver), "43", &header, transaction_with("$a")).await;
        assert_eq!(status(&other), StatusCode::OK);
        assert_eq!(annotator.call(), 2);
    }

    #[tokio::test]
    async fn delivery_5xx_failure_does_not_mark_the_transaction() {
        let _serial = serial_logs().await;
        let annotator = Arc::new(Annotator::default());
        let receiver = receiver(Arc::clone(&annotator), 8);
        let header = bearer_header(CANARIO);

        annotator.fail.store(true, Ordering::SeqCst);
        let fail = handle_transaction(Some(&receiver), "7", &header, transaction_with("$a")).await;
        assert_eq!(status(&fail), StatusCode::SERVICE_UNAVAILABLE);

        annotator.fail.store(false, Ordering::SeqCst);
        let replayed =
            handle_transaction(Some(&receiver), "7", &header, transaction_with("$a")).await;
        assert_eq!(status(&replayed), StatusCode::OK);
        assert_eq!(annotator.call(), 1);
    }

    #[tokio::test]
    async fn malformed_or_large_body_answers_200_without_processing() {
        let _serial = serial_logs().await;
        let annotator = Arc::new(Annotator::default());
        let receiver = receiver(Arc::clone(&annotator), 8);
        let header = bearer_header(CANARIO);

        for body in [
            Body::from("isto não é json"),
            Body::from("{ \"events\": \"não é lista\" }"),
            Body::from("{}"),
            Body::from(""),
        ] {
            let response = handle_transaction(Some(&receiver), "9", &header, body).await;
            assert_eq!(status(&response), StatusCode::OK, "corpo malformado");
        }

        let large = Body::from(vec![b'x'; CAP_BODY + 1]);
        let response = handle_transaction(Some(&receiver), "10", &header, large).await;
        assert_eq!(status(&response), StatusCode::OK, "corpo acima do teto");
        assert_eq!(annotator.call(), 0, "nada podia ter sido processado");
    }

    #[tokio::test]
    async fn disabled_module_is_404_on_the_four_routes() {
        let header = bearer_header(CANARIO);
        let transaction = handle_transaction(None, "1", &header, transaction_with("$a")).await;
        assert_eq!(status(&transaction), StatusCode::NOT_FOUND);
        let user = handle_user(None, "@ph_wa_1:matrix-dev.local", &header);
        assert_eq!(status(&user), StatusCode::NOT_FOUND);
        let alias = handle_alias(None, "#ph_c_1:matrix-dev.local", &header);
        assert_eq!(status(&alias), StatusCode::NOT_FOUND);
        let ping = handle_ping(None, &header);
        assert_eq!(status(&ping), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn canary_value_never_appears_in_response_or_log() {
        let _serial = serial_logs().await;
        let annotator = Arc::new(Annotator::default());
        let receiver = receiver(Arc::clone(&annotator), 8);

        let captured = Arc::new(Mutex::new(Vec::<u8>::new()));
        let _guard = capture_logs(Arc::clone(&captured));

        let mut bodies = Vec::new();
        for (header, body) in [
            (bearer_header(CANARIO), transaction_with("$a")),
            (bearer_header("outro-token"), transaction_with("$a")),
            (HeaderMap::new(), transaction_with("$a")),
            (bearer_header(CANARIO), Body::from("lixo")),
        ] {
            let response = handle_transaction(Some(&receiver), "77", &header, body).await;
            let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("corpo da resposta");
            bodies.push(String::from_utf8_lossy(&bytes).to_string());
        }

        let log = String::from_utf8(captured.lock().expect("trava").clone()).expect("utf-8");
        assert!(!log.trim().is_empty(), "o teste precisa de log para provar");
        for body in &bodies {
            assert!(!body.contains(CANARIO), "vazou na resposta: {body}");
        }
        assert!(!log.contains(CANARIO), "vazou no log: {log}");
        assert!(!format!("{receiver:?}").contains(CANARIO));
    }

    struct Writer(Arc<Mutex<Vec<u8>>>);

    impl Write for Writer {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("trava").extend_from_slice(data);
            Ok(data.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
