use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poph_matrix::error::CallError;
use poph_matrix::format::Outbound;
use poph_matrix::rooms::{ConversationToRoom, RoomError};
use poph_matrix::users::GhostIds;
use poph_server::matrix_metrics::MetricsMatrix;
use poph_server::matrix_mirror::{MatrixMirror, MatrixTarget, PublishedAsset};
use poph_server::{AttendantResponse, EventDelivery, HarnessResponsePort, ServerState};
use poph_store::{Direction, NewAgent, NewConversation, NewMessage, NewOrg, Origin, Role, Store};
use poph_types::{AgentId, ConversationId, HarnessConfig, MatrixConfig, OrgId, Registry};
use serde_json::{Value, json};

const SERVIDOR: &str = "matrix-dev.local";

struct Scenario {
    dir: PathBuf,
    store: Option<Arc<Store>>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("poph-metricas-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("cria a pasta do teste");
        let clock = Arc::new(AtomicI64::new(1_000));
        let store = Store::open_at(&dir.join("harness.db"), &dir.join("rag.db"))
            .expect("abre o banco")
            .with_clock(Arc::new(move || clock.fetch_add(1, Ordering::SeqCst)));
        Self {
            dir,
            store: Some(Arc::new(store)),
        }
    }

    fn store(&self) -> Arc<Store> {
        Arc::clone(self.store.as_ref().expect("store viva"))
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn config() -> MatrixConfig {
    MatrixConfig {
        enabled: true,
        homeserver_url: "http://127.0.0.1:1".to_owned(),
        server_name: SERVIDOR.to_owned(),
        prefix: "ph".to_owned(),
        concurrency: 8,
        interval_ms: 500,
        ..MatrixConfig::default()
    }
}

fn create_org(store: &Store, slug: &str) -> OrgId {
    store
        .orgs()
        .create(NewOrg {
            name: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id
}

fn create_agent(store: &Store, org: OrgId) -> AgentId {
    store
        .agents()
        .create(
            org,
            NewAgent {
                name: "Bia".to_owned(),
                slug: "bia".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente")
        .id
}

fn create_conversation(store: &Store, org: OrgId, agent: AgentId, peer: &str) -> ConversationId {
    store
        .conversations()
        .create(
            org,
            NewConversation {
                agent_id: agent,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: Some(peer.to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa")
        .id
}

fn customer_msg(store: &Store, org: OrgId, conversation: ConversationId, text: &str) -> String {
    store
        .messages()
        .write(
            org,
            NewMessage {
                conversation_id: conversation,
                direction: Direction::Inbound,
                role: Role::User,
                origin: Origin::Agente,
                author_name: None,
                texto: text.to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
            },
        )
        .expect("grava a mensagem")
        .id
        .to_string()
}

fn new_metrics() -> Arc<MetricsMatrix> {
    Arc::new(MetricsMatrix::register(&Registry::new()).expect("registra as métricas"))
}

#[derive(Clone, Copy)]
enum Mode {
    Normal,
    Outside,
    Limit,
    Venenoso(&'static str),
}

struct Fake {
    mode: Mutex<Mode>,
    error_message: String,
    sends: AtomicUsize,
}

impl Fake {
    fn new(mode: Mode, error_message: &str) -> Arc<Self> {
        Arc::new(Self {
            mode: Mutex::new(mode),
            error_message: error_message.to_owned(),
            sends: AtomicUsize::new(0),
        })
    }

    fn set_mode(&self, mode: Mode) {
        *self.mode.lock().unwrap() = mode;
    }

    fn error(&self, status: u16, errcode: &str) -> CallError {
        CallError::Response {
            operation: "send",
            status,
            errcode: Some(errcode.to_owned()),
            message: self.error_message.clone(),
            retry_after_ms: None,
        }
    }
}

struct Signal(Arc<Fake>);

impl MatrixTarget for Signal {
    fn ensure_room(
        &self,
        dados: &ConversationToRoom,
    ) -> impl std::future::Future<Output = Result<String, RoomError>> + Send {
        let room = format!("!{}:{SERVIDOR}", dados.conversation);
        async move { Ok(room) }
    }

    async fn ensure_device(
        &self,
        _data: &ConversationToRoom,
        _room: &str,
        _device: &poph_matrix::users::Ghost,
    ) -> Result<(), RoomError> {
        Ok(())
    }

    fn send(
        &self,
        _room: &str,
        send: &Outbound,
    ) -> impl std::future::Future<Output = Result<(), CallError>> + Send {
        let fake = &self.0;
        fake.sends.fetch_add(1, Ordering::SeqCst);
        let mode = *fake.mode.lock().unwrap();
        let resultado = match mode {
            Mode::Normal => Ok(()),
            Mode::Outside => Err(fake.error(503, "M_UNKNOWN")),
            Mode::Limit => Err(fake.error(429, "M_LIMIT_EXCEEDED")),
            Mode::Venenoso(text) if send.body.contains(text) => Err(fake.error(403, "M_FORBIDDEN")),
            Mode::Venenoso(_) => Ok(()),
        };
        async move { resultado }
    }

    async fn send_asset(
        &self,
        _room: &str,
        _send: &poph_server::matrix_mirror::AssetSend,
    ) -> Result<PublishedAsset, CallError> {
        Err(CallError::Configuration(
            "sem mídia neste destino".to_owned(),
        ))
    }

    async fn send_notice(
        &self,
        _room: &str,
        _notice: &poph_server::matrix_mirror::RoomNotice,
    ) -> Result<(), CallError> {
        Ok(())
    }
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl Write for Capture {
    fn write(&mut self, dados: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(dados);
        Ok(dados.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn capture_logs() -> (Capture, tracing::subscriber::DefaultGuard) {
    let capture = Capture::default();
    let destination = capture.clone();
    let assinante = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .without_time()
        .with_writer(move || destination.clone())
        .finish();
    let guard = tracing::subscriber::set_default(assinante);
    (capture, guard)
}

#[tokio::test]
async fn cursor_lag_rises_with_synapse_down_and_returns_to_zero() {
    let scenario = Scenario::new("atraso");
    let store = scenario.store();
    let org = create_org(&store, "atraso");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "5521977770001");
    store.matrix().enable(org).expect("liga o módulo");
    customer_msg(&store, org, conversation, "primeira");
    customer_msg(&store, org, conversation, "segunda");

    let fake = Fake::new(Mode::Outside, "indisponível");
    let metrics = new_metrics();
    let now = Arc::new(AtomicI64::new(31_000));
    let clock = Arc::clone(&now);
    let mirror = MatrixMirror::new(Arc::clone(&store), Signal(Arc::clone(&fake)), &config())
        .expect("monta o espelho")
        .with_metrics(Arc::clone(&metrics))
        .with_clock(Arc::new(move || clock.load(Ordering::SeqCst)));

    mirror.cycle().await.expect("ciclo com o Synapse fora");
    let initial = metrics.cursor_age_seconds.value();
    assert!(
        (29..=31).contains(&initial),
        "com o Synapse fora, a primeira mensagem tem ~30 s de atraso; veio {initial}"
    );

    now.store(91_000, Ordering::SeqCst);
    mirror
        .cycle()
        .await
        .expect("ciclo com o Synapse ainda fora");
    let after = metrics.cursor_age_seconds.value();
    assert!(
        after > initial && (89..=91).contains(&after),
        "o atraso tem de subir enquanto o Synapse está fora; veio {after} (antes {initial})"
    );

    fake.set_mode(Mode::Normal);
    mirror.cycle().await.expect("ciclo com o Synapse de volta");
    assert_eq!(
        metrics.cursor_age_seconds.value(),
        0,
        "com o Synapse de volta e tudo espelhado, o atraso volta a zero"
    );
    assert_eq!(metrics.mirrored_messages.value(), 2);
}

#[tokio::test]
async fn mirror_counters_count_attempts_errors_and_429() {
    let scenario = Scenario::new("contadores");
    let store = scenario.store();
    let org = create_org(&store, "contadores");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "5521977770002");
    store.matrix().enable(org).expect("liga o módulo");
    customer_msg(&store, org, conversation, "um");
    customer_msg(&store, org, conversation, "dois");

    let fake = Fake::new(Mode::Limit, "limite");
    let metrics = new_metrics();
    let mirror = MatrixMirror::new(Arc::clone(&store), Signal(Arc::clone(&fake)), &config())
        .expect("monta o espelho")
        .with_metrics(Arc::clone(&metrics));

    mirror.cycle().await.expect("ciclo com limite de taxa");
    assert_eq!(metrics.attempts.value(), 2);
    assert_eq!(metrics.responses_429.value(), 1, "o 429 conta uma vez");
    assert_eq!(
        metrics.transient_errors.value(),
        1,
        "o 429 é transitório: conta como erro transitório"
    );
    assert_eq!(metrics.definitive_errors.value(), 0);
    assert_eq!(metrics.mirrored_messages.value(), 0);

    fake.set_mode(Mode::Normal);
    mirror.cycle().await.expect("ciclo sem limite");
    assert_eq!(metrics.attempts.value(), 5);
    assert_eq!(metrics.mirrored_messages.value(), 2);
    assert_eq!(
        metrics.responses_429.value(),
        1,
        "sem novo 429, o contador fica"
    );
}

#[tokio::test]
async fn poison_message_isolated_counts_and_logs_org_and_conversation() {
    let scenario = Scenario::new("isolada");
    let store = scenario.store();
    let org = create_org(&store, "isolada");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "5521977770003");
    store.matrix().enable(org).expect("liga o módulo");
    customer_msg(&store, org, conversation, "VENENO aqui");
    customer_msg(&store, org, conversation, "depois");

    let fake = Fake::new(Mode::Venenoso("VENENO"), "recusado");
    let metrics = new_metrics();
    let mirror = MatrixMirror::new(Arc::clone(&store), Signal(Arc::clone(&fake)), &config())
        .expect("monta o espelho")
        .with_metrics(Arc::clone(&metrics));
    let (capture, _guard) = capture_logs();

    for _ in 0..4 {
        mirror.cycle().await.expect("ciclo");
    }

    assert_eq!(metrics.definitive_errors.value(), 3);
    assert_eq!(metrics.isolated_messages.value(), 1);
    assert_eq!(
        metrics.mirrored_messages.value(),
        1,
        "a mensagem seguinte sai"
    );
    let log = capture.text();
    let line = log
        .lines()
        .find(|line| line.contains("mensagem isolada"))
        .unwrap_or_else(|| panic!("falta o aviso de isolamento no log:\n{log}"));
    assert!(
        line.contains(&conversation.to_string()) && line.contains(&org.to_string()),
        "o aviso de isolamento leva org e conversa; linha: {line}"
    );
}

const CANARY_PHONE: &str = "5521900418273";
const CANARY_TEXT: &str = "CANARIO-TEXTO-7a3f9";
const CANARIO_TOKEN: &str = "CANARIO-TOKEN-x81kq";

#[tokio::test]
async fn canary_value_never_appears_in_module_log() {
    let scenario = Scenario::new("canario");
    let store = scenario.store();
    let org = create_org(&store, "canario");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, CANARY_PHONE);
    store.matrix().enable(org).expect("liga o módulo");
    customer_msg(&store, org, conversation, CANARY_TEXT);
    let fake = Fake::new(Mode::Venenoso(CANARY_TEXT), CANARIO_TOKEN);
    let mirror = MatrixMirror::new(Arc::clone(&store), Signal(Arc::clone(&fake)), &config())
        .expect("monta o espelho")
        .with_metrics(new_metrics());

    let (capture, guard) = capture_logs();
    fake.set_mode(Mode::Outside);
    mirror.cycle().await.expect("ciclo transitório");
    fake.set_mode(Mode::Venenoso(CANARY_TEXT));
    for _ in 0..3 {
        mirror.cycle().await.expect("ciclo definitivo");
    }

    let room = "!canario-sala:matrix-dev.local";
    store
        .matrix()
        .ensure_room(org, conversation, room, None)
        .expect("grava a sala");
    store
        .matrix()
        .mark_room_ready(org, conversation)
        .expect("sala pronta");
    let ids = GhostIds::of_config(&config()).expect("ids");
    let delivery = AttendantResponse::with_caps(
        Arc::new(HarnessResponsePort::fresh(Arc::clone(&store))),
        None,
        ids,
        16,
        16,
        Duration::from_secs(300),
    )
    .with_metrics(new_metrics());
    let event: Value = json!({
        "type": "m.room.message",
        "room_id": room,
        "event_id": "$canario-1:matrix-dev.local",
        "sender": "@willian:matrix-dev.local",
        "content": { "msgtype": "m.text", "body": CANARY_TEXT },
    });
    delivery
        .deliver("txn-canario", vec![event])
        .await
        .expect("entrega aceita a resposta");
    drop(guard);

    let log = capture.text();
    assert!(
        log.contains("isolada") || log.contains("resposta do atendente enfileirada"),
        "o teste precisa ter logs de verdade para valer; log:\n{log}"
    );
    for canario in [CANARY_PHONE, CANARY_TEXT, CANARIO_TOKEN] {
        assert!(
            !log.contains(canario),
            "o valor-canário `{canario}` vazou para o log:\n{log}"
        );
    }
}

#[tokio::test]
async fn attendant_reply_counters_become_metrics() {
    let scenario = Scenario::new("contadores-resposta");
    let store = scenario.store();
    let org = create_org(&store, "contadores-resposta");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "5521977770004");
    store.matrix().enable(org).expect("liga o módulo");
    let room = "!contadores-sala:matrix-dev.local";
    store
        .matrix()
        .ensure_room(org, conversation, room, None)
        .expect("grava a sala");
    store
        .matrix()
        .mark_room_ready(org, conversation)
        .expect("sala pronta");

    let metrics = new_metrics();
    let ids = GhostIds::of_config(&config()).expect("ids");
    let delivery = AttendantResponse::with_caps(
        Arc::new(HarnessResponsePort::fresh(Arc::clone(&store))),
        None,
        ids,
        16,
        16,
        Duration::from_secs(300),
    )
    .with_metrics(Arc::clone(&metrics));

    let text = |event: &str, room: &str, body: Value| {
        json!({
            "type": "m.room.message",
            "room_id": room,
            "event_id": event,
            "sender": "@atendente:matrix-dev.local",
            "content": body,
        })
    };
    let human = || json!({ "msgtype": "m.text", "body": "olá" });

    delivery
        .deliver("txn-1", vec![text("$c-1:matrix-dev.local", room, human())])
        .await
        .expect("entrega");
    assert_eq!(metrics.responses_queued.value(), 1);

    delivery
        .deliver("txn-2", vec![text("$c-1:matrix-dev.local", room, human())])
        .await
        .expect("entrega");
    assert_eq!(metrics.responses_queued.value(), 1);
    assert_eq!(metrics.responses_duplicated.value(), 1);

    delivery
        .deliver(
            "txn-3",
            vec![text(
                "$c-2:matrix-dev.local",
                "!desconhecida:matrix-dev.local",
                human(),
            )],
        )
        .await
        .expect("entrega");
    assert_eq!(metrics.rooms_unknown.value(), 1);

    delivery
        .deliver(
            "txn-4",
            vec![text(
                "$c-3:matrix-dev.local",
                room,
                json!({ "msgtype": "m.image", "body": "foto.jpg" }),
            )],
        )
        .await
        .expect("entrega");
    assert_eq!(metrics.asset_notices.value(), 1);
    assert_eq!(
        metrics.responses_queued.value(),
        1,
        "mídia não vira resposta ao cliente"
    );

    let versao = store
        .conversations()
        .get(org, conversation)
        .expect("lê a conversa")
        .version;
    store
        .conversations()
        .close_audited(org, conversation, versao, "teste".to_owned())
        .expect("encerra a conversa");
    delivery
        .deliver("txn-5", vec![text("$c-4:matrix-dev.local", room, human())])
        .await
        .expect("entrega");
    assert_eq!(metrics.responses_conversation_closed.value(), 1);
    assert_eq!(
        metrics.responses_queued.value(),
        1,
        "conversa encerrada não enfileira para o cliente"
    );
}

async fn http_metrics(state: ServerState) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = listener.local_addr().expect("endereço");
    let (desligar, signal) = tokio::sync::oneshot::channel::<()>();
    let servidor = tokio::spawn(async move {
        let _ = poph_server::serve_at(listener, state, async {
            let _ = signal.await;
        })
        .await;
    });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("cliente HTTP");
    let mut body = None;
    for _ in 0..100 {
        if let Ok(reply) = client
            .get(format!("http://{endereco}/metrics"))
            .send()
            .await
        {
            body = Some(reply.text().await.expect("corpo do /metrics"));
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let _ = desligar.send(());
    let _ = servidor.await;
    body.expect("o /metrics respondeu")
}

fn state_with_module(label: &str, enabled: bool) -> (ServerState, PathBuf) {
    let dir =
        std::env::temp_dir().join(format!("poph-metricas-http-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("dados")).expect("pasta de dados");
    let mut cfg = HarnessConfig::default();
    cfg.data.directory = dir.join("dados");
    cfg.matrix = Some(MatrixConfig {
        enabled,
        ..config()
    });
    let store = Arc::new(Store::open(&cfg).expect("abre o banco"));
    let state = ServerState::new(store, cfg).expect("estado do servidor");
    (state, dir)
}

const MATRIX_NAMES: [&str; 15] = [
    "poph_matrix_atraso_cursor_segundos",
    "poph_matrix_salas_prontas_total",
    "poph_matrix_mensagens_espelhadas_total",
    "poph_matrix_midias_espelhadas_total",
    "poph_matrix_respostas_enfileiradas_total",
    "poph_matrix_respostas_duplicadas_total",
    "poph_matrix_respostas_conversa_encerrada_total",
    "poph_matrix_salas_desconhecidas_total",
    "poph_matrix_avisos_de_midia_total",
    "poph_matrix_erros_transitorios_total",
    "poph_matrix_erros_definitivos_total",
    "poph_matrix_respostas_429_total",
    "poph_matrix_tentativas_total",
    "poph_matrix_mensagens_isoladas_total",
    "poph_matrix_latencia_synapse_segundos",
];

#[tokio::test]
async fn module_metrics_absent_when_module_off() {
    let (state, dir) = state_with_module("desligado", false);
    assert!(
        state.matrix_metrics().expect("sem erro").is_none(),
        "com [matrix] desligado, as métricas do módulo não são declaradas"
    );
    let text = http_metrics(state).await;
    let _ = fs::remove_dir_all(&dir);
    assert!(
        text.contains("poph_memoria_processo_bytes"),
        "sanidade: o /metrics responde"
    );
    assert!(
        !text.contains("poph_matrix_"),
        "com o módulo desligado nenhuma poph_matrix_* pode aparecer:\n{text}"
    );
}

#[tokio::test]
async fn module_metrics_appear_once_when_prepared() {
    let (state, dir) = state_with_module("ligado", true);
    let first = state
        .matrix_metrics()
        .expect("prepara as métricas")
        .expect("módulo ligado");
    let second = state
        .matrix_metrics()
        .expect("segunda chamada")
        .expect("módulo ligado");
    assert!(
        Arc::ptr_eq(&first, &second),
        "mesmo handle nas duas chamadas"
    );
    first.mirrored_messages.increment(3);
    let text = http_metrics(state).await;
    let _ = fs::remove_dir_all(&dir);
    for name in MATRIX_NAMES {
        assert_eq!(
            text.matches(&format!("# HELP {name} ")).count(),
            1,
            "{name} deve aparecer exatamente uma vez no /metrics"
        );
    }
    assert!(
        text.contains("poph_matrix_mensagens_espelhadas_total 3\n"),
        "o valor incrementado aparece no /metrics"
    );
}

#[test]
fn throwaway_metrics_absent_from_process_registry() {
    let process = Registry::new();
    let _ = MetricsMatrix::register(&process).expect("registra no processo");
    let descartavel = MetricsMatrix::unregistered();
    descartavel.definitive_errors.increment(1);
    assert!(
        !process
            .render()
            .contains("poph_matrix_erros_definitivos_total 1")
    );
}
