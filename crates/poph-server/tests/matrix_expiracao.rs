use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_matrix::client::{ClientConfig, MatrixClient};
use poph_matrix::config::VAR_ADMIN_TOKEN;
use poph_matrix::expiration::{CATEGORY_FIELD, EXPIRED_BLOCK, NAME_FIELD, SENT_AT_FIELD};
use poph_server::matrix_beginning::module_expiry;
use poph_server::matrix_expiry::{AssetExpiry, ExpiryReport};
use poph_server::matrix_metrics::MetricsMatrix;
use poph_store::{
    CategoryAsset, Conversa, NewAgent, NewAsset, NewConversation, NewOrg, Org, Store,
};
use poph_types::{AttachmentId, MatrixConfig, Secret};
use serde_json::{Value, json};
use tracing_subscriber::fmt::MakeWriter;

const PREFIX: &str = "ph";
const SERVIDOR: &str = "matrix-dev.local";
const ROOM: &str = "!sala-mx46:matrix-dev.local";
const AGENTE: &str = "agt_1";
const AS_TOKEN: &str = "as-token-mx46";
const ADMIN_TOKEN: &str = "CANARIO-ADMIN-mx46-c0f1-TAO-SECRETO";

const DAY: i64 = 86_400_000;

#[derive(Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn captured_logs() -> Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let _ = tracing_subscriber::fmt()
            .with_writer(Capture(Arc::clone(&buffer)))
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .try_init();
        buffer
    })
    .clone()
}

fn log_text() -> String {
    let logs = captured_logs();
    let guard = logs.lock().unwrap();
    String::from_utf8_lossy(&guard).into_owned()
}

#[derive(Debug, Clone)]
struct Redaction {
    room: String,
    event: String,
    txn: String,
    as_user: Option<String>,
    body: Value,
}

#[derive(Clone, Default)]
struct Fake {
    redactions: Arc<Mutex<Vec<Redaction>>>,
    deleted: Arc<Mutex<Vec<(String, String)>>>,
    authorizations: Arc<Mutex<Vec<String>>>,
    calls: Arc<AtomicUsize>,
    redaction_failure: Arc<Mutex<Option<u16>>>,
    deletion_failure: Arc<Mutex<Option<u16>>>,
}

fn respond(status: u16, body: Value) -> Response {
    (
        StatusCode::from_u16(status).expect("status válido"),
        [("content-type", "application/json")],
        body.to_string(),
    )
        .into_response()
}

fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
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

async fn handle(
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
                .map(|(chave, value)| (chave.to_owned(), decode(value)))
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
        fake.authorizations
            .lock()
            .unwrap()
            .push(authorization.clone());
        if authorization != format!("Bearer {ADMIN_TOKEN}") {
            return respond(403, json!({ "error": "You are not a server admin" }));
        }
        if let Some(status) = fake.deletion_failure.lock().unwrap().take() {
            return respond(status, json!({ "error": "indisponível" }));
        }
        if parts.len() != 2 {
            return respond(400, json!({ "error": "caminho de mídia inválido" }));
        }
        fake.deleted
            .lock()
            .unwrap()
            .push((parts[0].to_owned(), parts[1].to_owned()));
        return respond(200, json!({ "deleted_media": [parts[1]] }));
    }

    if method == Method::PUT && uri.path().contains("/redact/") {
        if authorization != format!("Bearer {AS_TOKEN}") {
            return respond(401, json!({ "errcode": "M_UNKNOWN_TOKEN" }));
        }
        if let Some(status) = fake.redaction_failure.lock().unwrap().take() {
            return respond(
                status,
                json!({ "errcode": "M_UNKNOWN", "error": "indisponível" }),
            );
        }
        let parts: Vec<&str> = uri
            .path()
            .trim_start_matches("/_matrix/client/v3/rooms/")
            .split('/')
            .collect();
        if parts.len() != 4 || parts[1] != "redact" {
            return respond(404, json!({ "errcode": "M_UNRECOGNIZED" }));
        }
        let mut redactions = fake.redactions.lock().unwrap();
        let new = !redactions
            .iter()
            .any(|r| r.room == parts[0] && r.txn == parts[3]);
        if new {
            redactions.push(Redaction {
                room: parts[0].to_owned(),
                event: parts[2].to_owned(),
                txn: parts[3].to_owned(),
                as_user: query.get("user_id").cloned(),
                body,
            });
        }
        let position = redactions.len();
        return respond(
            200,
            json!({ "event_id": format!("$redacao-{position}:matrix-dev.local") }),
        );
    }

    respond(
        404,
        json!({ "errcode": "M_UNRECOGNIZED", "error": "rota não existe no Synapse falso" }),
    )
}

struct Synapse {
    url: String,
    fake: Fake,
    desligar: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Synapse {
    async fn start() -> Self {
        let fake = Fake::default();
        let app = Router::new().fallback(handle).with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let url = format!("http://{}", listener.local_addr().expect("endereço"));
        let (desligar, signal) = tokio::sync::oneshot::channel();
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
            desligar: Some(desligar),
        }
    }

    fn client(&self) -> MatrixClient {
        MatrixClient::new(
            ClientConfig::new(
                self.url.clone(),
                SERVIDOR.to_owned(),
                Secret::new(AS_TOKEN.to_owned()),
            )
            .expect("config do cliente"),
        )
        .expect("cliente do Matrix")
    }

    fn redactions(&self) -> Vec<Redaction> {
        self.fake.redactions.lock().unwrap().clone()
    }

    fn deleted(&self) -> Vec<(String, String)> {
        self.fake.deleted.lock().unwrap().clone()
    }

    fn authorizations(&self) -> Vec<String> {
        self.fake.authorizations.lock().unwrap().clone()
    }

    fn calls(&self) -> usize {
        self.fake.calls.load(Ordering::SeqCst)
    }

    fn refuse_redaction_with(&self, status: u16) {
        *self.fake.redaction_failure.lock().unwrap() = Some(status);
    }

    fn refuse_deletion_with(&self, status: u16) {
        *self.fake.deletion_failure.lock().unwrap() = Some(status);
    }
}

impl Drop for Synapse {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
    }
}

struct Scenario {
    folder: PathBuf,
    clock: Arc<AtomicI64>,
    store: Option<Arc<Store>>,
    org: Org,
    conversation: Conversa,
    synapse: Synapse,
}

impl Scenario {
    async fn new(label: &str, now: i64) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder =
            std::env::temp_dir().join(format!("poph-expiracao-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("pasta do teste");
        let clock = Arc::new(AtomicI64::new(now));
        let store_now = Arc::clone(&clock);
        let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
            .expect("abre o banco")
            .with_clock(Arc::new(move || store_now.load(Ordering::SeqCst)));
        let store = Arc::new(store);

        let org = store
            .orgs()
            .create(NewOrg {
                name: "Loja MX-46".to_owned(),
                slug: "loja-mx46".to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org");
        let agent = store
            .agents()
            .create(
                org.id,
                NewAgent {
                    name: "Atendente".to_owned(),
                    slug: "atendente".to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente");
        let conversation = store
            .conversations()
            .create(
                org.id,
                NewConversation {
                    agent_id: agent.id,
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some("5511999990000".to_owned()),
                    target_department: None,
                },
            )
            .expect("cria a conversa");
        store.matrix().enable(org.id).expect("liga o módulo");
        store
            .matrix()
            .ensure_room(org.id, conversation.id, ROOM, None)
            .expect("grava a sala");
        store
            .matrix()
            .mark_room_ready(org.id, conversation.id)
            .expect("sala pronta");

        captured_logs();
        Self {
            folder,
            clock,
            store: Some(store),
            org,
            conversation,
            synapse: Synapse::start().await,
        }
    }

    fn store(&self) -> &Arc<Store> {
        self.store.as_ref().expect("store viva")
    }

    fn now(&self) -> i64 {
        self.clock.load(Ordering::SeqCst)
    }

    fn advance(&self, ms: i64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }

    fn publicar(&self, name: &str, sent_at: i64) -> AttachmentId {
        let id = AttachmentId::generate();
        let category = if name.ends_with(".jpg") {
            CategoryAsset::Image
        } else {
            CategoryAsset::Document
        };
        let mime = if category == CategoryAsset::Image {
            "image/jpeg"
        } else {
            "application/pdf"
        };
        let registro = NewAsset {
            id,
            conversation: self.conversation.id,
            agent: AGENTE.to_owned(),
            room: ROOM.to_owned(),
            evento: format!("$midia-{id}:matrix-dev.local"),
            mxc: format!("mxc://{SERVIDOR}/media-{id}"),
            nome: name.to_owned(),
            mime: mime.to_owned(),
            category,
            size_bytes: 2_048,
            sent_at,
        };
        assert!(
            self.store()
                .matrix_asset()
                .register(self.org.id, &registro)
                .expect("grava a mídia"),
            "a primeira gravação é nova"
        );
        id
    }

    fn config(&self) -> MatrixConfig {
        MatrixConfig {
            enabled: true,
            homeserver_url: self.synapse.url.clone(),
            server_name: SERVIDOR.to_owned(),
            prefix: PREFIX.to_owned(),
            attachment_retention_days: 4,
            ..MatrixConfig::default()
        }
    }

    fn service(&self) -> AssetExpiry {
        let clock = Arc::clone(&self.clock);
        AssetExpiry::new(
            Arc::clone(self.store()),
            self.synapse.client(),
            Secret::new(ADMIN_TOKEN.to_owned()),
            &self.config(),
        )
        .expect("monta a expiração")
        .with_clock(Arc::new(move || clock.load(Ordering::SeqCst)))
    }

    fn pending_count(&self) -> u32 {
        self.store()
            .matrix_asset()
            .org_pending(self.org.id)
            .expect("conta as pendentes")
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.folder);
    }
}

#[tokio::test]
async fn nothing_happens_before_deadline() {
    let scenario = Scenario::new("antes", 1_000 * DAY).await;
    let id = scenario.publicar("laudo.pdf", scenario.now() - 3 * DAY);

    let report = scenario.service().pass().await;
    assert_eq!(report, ExpiryReport::default(), "{report:?}");
    assert_eq!(
        scenario.synapse.calls(),
        0,
        "antes do prazo não há chamada nenhuma"
    );
    assert!(scenario.synapse.redactions().is_empty());
    assert!(scenario.synapse.deleted().is_empty());
    assert_eq!(scenario.pending_count(), 1, "a linha continua pendente");
    let rows = scenario
        .store()
        .matrix_asset()
        .pending(scenario.org.id, scenario.now(), scenario.now(), 10)
        .expect("lê a fila");
    assert_eq!(rows[0].id, id);
    assert!(rows[0].still_to_draft() && rows[0].still_to_delete());
}

#[tokio::test]
async fn after_deadline_redacts_and_deletes_once() {
    let scenario = Scenario::new("depois", 1_000 * DAY).await;
    let sent_at = scenario.now() - 5 * DAY;
    let id = scenario.publicar("laudo.pdf", sent_at);
    let service = scenario.service();

    let report = service.pass().await;
    assert_eq!(report.expired, 1);
    assert_eq!(report.redacted, 1);
    assert_eq!(report.deleted, 1);
    assert_eq!(report.deferred, 0);

    let redactions = scenario.synapse.redactions();
    assert_eq!(redactions.len(), 1, "{redactions:?}");
    assert_eq!(redactions[0].room, ROOM);
    assert_eq!(redactions[0].event, format!("$midia-{id}:matrix-dev.local"));
    assert_eq!(
        redactions[0].as_user.as_deref(),
        Some("@ph_ag_agt_1:matrix-dev.local"),
        "quem redige é o fantasma do agente (moderador da sala)"
    );
    let block = &redactions[0].body[EXPIRED_BLOCK];
    assert_eq!(block[NAME_FIELD], json!("laudo.pdf"));
    assert_eq!(block[CATEGORY_FIELD], json!("document"));
    assert_eq!(block[SENT_AT_FIELD], json!("1972-09-22T00:00:00Z"));

    assert_eq!(
        scenario.synapse.deleted(),
        vec![(SERVIDOR.to_owned(), format!("media-{id}"))]
    );
    for authorization in scenario.synapse.authorizations() {
        assert_eq!(authorization, format!("Bearer {ADMIN_TOKEN}"));
    }

    let calls = scenario.synapse.calls();
    let second_pass = service.pass().await;
    assert_eq!(second_pass, ExpiryReport::default(), "{second_pass:?}");
    assert_eq!(
        scenario.synapse.calls(),
        calls,
        "mídia completa não volta ao Synapse"
    );
    assert_eq!(scenario.pending_count(), 0);
}

#[tokio::test]
async fn failed_redaction_does_not_block_deletion_and_item_returns_later() {
    let scenario = Scenario::new("redacao-falha", 1_000 * DAY).await;
    scenario.publicar("foto.jpg", scenario.now() - 5 * DAY);
    let service = scenario.service();
    scenario.synapse.refuse_redaction_with(500);

    let report = service.pass().await;
    assert_eq!(report.redacted, 0, "{report:?}");
    assert_eq!(report.deleted, 1, "a retenção vale mais do que a redação");
    assert_eq!(report.deferred, 1);
    assert!(scenario.synapse.redactions().is_empty());
    assert_eq!(scenario.synapse.deleted().len(), 1);
    assert_eq!(scenario.pending_count(), 1, "falta só redigir");

    let calls = scenario.synapse.calls();
    let second_pass = service.pass().await;
    assert_eq!(second_pass.expired, 0, "{second_pass:?}");
    assert_eq!(scenario.synapse.calls(), calls);

    scenario.advance(61_000);
    let third_pass = service.pass().await;
    assert_eq!(third_pass.redacted, 1, "{third_pass:?}");
    assert_eq!(third_pass.deleted, 0, "o arquivo já tinha sido apagado");
    assert_eq!(scenario.synapse.redactions().len(), 1);
    assert_eq!(scenario.pending_count(), 0);
}

#[tokio::test]
async fn deletion_failure_does_not_spin_and_wait_grows() {
    let scenario = Scenario::new("espera", 1_000 * DAY).await;
    scenario.publicar("laudo.pdf", scenario.now() - 5 * DAY);
    let service = scenario.service();
    scenario.synapse.refuse_deletion_with(503);

    let first_pass = service.pass().await;
    assert_eq!(first_pass.redacted, 1, "{first_pass:?}");
    assert_eq!(first_pass.deleted, 0);
    assert_eq!(first_pass.deferred, 1);
    assert_eq!(scenario.synapse.deleted().len(), 0);

    let after_first = scenario.synapse.calls();
    for _ in 0..3 {
        let report = service.pass().await;
        assert_eq!(report.expired, 0, "{report:?}");
    }
    assert_eq!(
        scenario.synapse.calls(),
        after_first,
        "o item adiado não é chamado de novo antes da hora"
    );

    scenario.advance(61_000);
    scenario.synapse.refuse_deletion_with(503);
    let second_pass = service.pass().await;
    assert_eq!(second_pass.expired, 1, "{second_pass:?}");
    assert_eq!(second_pass.deferred, 1);

    scenario.advance(61_000);
    let cedo = service.pass().await;
    assert_eq!(cedo.expired, 0, "a espera dobrou: {cedo:?}");
    scenario.advance(60_000);
    let on_time = service.pass().await;
    assert_eq!(
        on_time.expired, 1,
        "na hora marcada o item volta: {on_time:?}"
    );
}

#[tokio::test]
async fn org_with_module_off_is_untouched() {
    let scenario = Scenario::new("desligada", 1_000 * DAY).await;
    scenario.publicar("laudo.pdf", scenario.now() - 30 * DAY);
    scenario
        .store()
        .matrix()
        .disable(scenario.org.id)
        .expect("desliga o módulo na org");

    let report = scenario.service().pass().await;
    assert_eq!(report, ExpiryReport::default(), "{report:?}");
    assert_eq!(scenario.synapse.calls(), 0);
    assert_eq!(scenario.pending_count(), 1, "a linha fica como estava");
}

#[tokio::test]
async fn admin_token_does_not_appear_in_log() {
    let scenario = Scenario::new("canario", 1_000 * DAY).await;
    scenario.publicar("laudo.pdf", scenario.now() - 5 * DAY);
    let service = scenario.service();
    scenario.synapse.refuse_redaction_with(403);
    scenario.synapse.refuse_deletion_with(403);

    let report = service.pass().await;
    assert_eq!(report.deferred, 1, "{report:?}");

    assert!(!format!("{service:?}").contains(ADMIN_TOKEN));

    let logs = log_text();
    assert!(
        logs.contains("matrix_expiracao"),
        "o log da rotina saiu: {logs}"
    );
    assert!(
        !logs.contains(ADMIN_TOKEN),
        "o token de administração vazou no log"
    );
    assert!(
        !logs.contains("Bearer"),
        "nenhum cabeçalho de autorização no log"
    );
}

#[tokio::test]
async fn without_token_routine_is_off_with_notice_and_no_call() {
    let scenario = Scenario::new("sem-token", 1_000 * DAY).await;
    scenario.publicar("laudo.pdf", scenario.now() - 5 * DAY);

    let without_token = module_expiry(
        Arc::clone(scenario.store()),
        scenario.synapse.client(),
        &scenario.config(),
        None,
        None,
    );
    assert!(without_token.is_none(), "sem token a rotina não existe");

    let logs = log_text();
    assert!(
        logs.contains("expiração da mídia desligada"),
        "o log avisa que a rotina está desligada: {logs}"
    );
    assert!(
        logs.contains(VAR_ADMIN_TOKEN),
        "o aviso diz qual variável falta: {logs}"
    );
    assert_eq!(
        scenario.synapse.calls(),
        0,
        "sem token não há chamada ao Synapse"
    );

    let with_token = module_expiry(
        Arc::clone(scenario.store()),
        scenario.synapse.client(),
        &scenario.config(),
        None,
        Some(Secret::new(ADMIN_TOKEN.to_owned())),
    )
    .expect("com token a rotina é montada");
    let clock = Arc::clone(&scenario.clock);
    let with_token = with_token.with_clock(Arc::new(move || clock.load(Ordering::SeqCst)));
    let report = with_token.pass().await;
    assert_eq!(report.redacted, 1, "{report:?}");
    assert_eq!(report.deleted, 1);
}

#[tokio::test]
async fn module_metrics_count_per_entry() {
    use poph_types::Registry;

    let scenario = Scenario::new("metricas", 1_000 * DAY).await;
    scenario.publicar("laudo.pdf", scenario.now() - 5 * DAY);
    scenario.publicar("foto.jpg", scenario.now() - 6 * DAY);
    scenario.publicar("novo.pdf", scenario.now() - DAY);

    let registro = Registry::new();
    let metrics = Arc::new(MetricsMatrix::register(&registro).expect("registra"));
    let clock = Arc::clone(&scenario.clock);
    let service = scenario
        .service()
        .with_metrics(Arc::clone(&metrics))
        .with_clock(Arc::new(move || clock.load(Ordering::SeqCst)));

    let report = service.pass().await;
    assert_eq!(report.redacted, 2, "{report:?}");
    assert_eq!(report.deleted, 2);

    let text = registro.render();
    assert!(
        text.contains("poph_matrix_midias_redigidas_total 2\n"),
        "{text}"
    );
    assert!(
        text.contains("poph_matrix_midias_apagadas_total 2\n"),
        "{text}"
    );
    assert!(
        text.contains("poph_matrix_midias_pendentes_expiracao 1\n"),
        "{text}"
    );
    assert!(
        text.contains("poph_matrix_midias_expiracao_adiadas_total 0\n"),
        "{text}"
    );
}

#[tokio::test]
async fn configured_deadline_drives_cutoff() {
    let scenario = Scenario::new("prazo", 1_000 * DAY).await;
    scenario.publicar("laudo.pdf", scenario.now() - 2 * DAY);
    let clock = Arc::clone(&scenario.clock);

    let mut config = scenario.config();
    config.attachment_retention_days = 1;
    let service = AssetExpiry::new(
        Arc::clone(scenario.store()),
        scenario.synapse.client(),
        Secret::new(ADMIN_TOKEN.to_owned()),
        &config,
    )
    .expect("monta")
    .with_clock(Arc::new(move || clock.load(Ordering::SeqCst)));

    let report = service.pass().await;
    assert_eq!(report.redacted, 1, "{report:?}");

    let other = Scenario::new("prazo-padrao", 1_000 * DAY).await;
    let id = other.publicar("laudo2.pdf", other.now() - 2 * DAY);
    let clock = Arc::clone(&other.clock);
    let default_service = AssetExpiry::new(
        Arc::clone(other.store()),
        other.synapse.client(),
        Secret::new(ADMIN_TOKEN.to_owned()),
        &other.config(),
    )
    .expect("monta")
    .with_clock(Arc::new(move || clock.load(Ordering::SeqCst)));
    assert_eq!(default_service.pass().await.redacted, 0);
    let pending_row = other
        .store()
        .matrix_asset()
        .pending(other.org.id, other.now(), other.now(), 10)
        .expect("lê a fila");
    assert_eq!(pending_row[0].id, id);
}

#[tokio::test]
async fn background_loop_stops_on_request() {
    let scenario = Scenario::new("parada", 1_000 * DAY).await;
    let service = Arc::new(scenario.service().with_period(Duration::from_millis(5)));
    let task = tokio::spawn(Arc::clone(&service).run());
    tokio::time::sleep(Duration::from_millis(20)).await;
    service.ask_stop();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("o laço termina no desligamento")
        .expect("a tarefa não estoura");
}
