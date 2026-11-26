use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use poph_plugins::{Backoff, EventCollector, PluginOptions, Supervisor, SupervisorLimits};
use poph_types::{
    EventSink, Integration, IntegrationError, IntegrationEvent, IntegrationId, IntegrationState,
    OrgId, PROTOCOL_VERSION, Presence, SendFile, SendText, SetPresence, StateEvent,
};

fn fixture_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_plugin-fixtura"))
}

fn fast_limits() -> SupervisorLimits {
    SupervisorLimits {
        max_message_bytes: 1024 * 1024,
        timeout_handshake: Duration::from_secs(2),
        command_timeout: Duration::from_millis(300),
        health_timeout: Duration::from_millis(300),
        backoff: Backoff {
            base: Duration::from_millis(30),
            factor: 2,
            cap: Duration::from_millis(200),
        },
        max_failures: 3,
    }
}

struct Scenario {
    root: PathBuf,
}

impl Scenario {
    fn new(name: &str) -> Self {
        static COUNTER: Mutex<u64> = Mutex::new(0);
        let mut counter = COUNTER.lock().unwrap();
        *counter += 1;
        let root = std::env::temp_dir().join(format!(
            "poph-plugin-{name}-{}-{}",
            std::process::id(),
            *counter
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("criar a pasta do cenário");
        let root = root.canonicalize().unwrap_or(root);
        Self { root }
    }

    fn dump(&self) -> PathBuf {
        self.root.join("dump")
    }

    fn options(&self, mode: &str) -> PluginOptions {
        PluginOptions::new(
            fixture_binary(),
            IntegrationId::generate(),
            OrgId::generate(),
        )
        .with_kind("fixtura")
        .with_config(json!({
            "modo": mode,
            "dump_dir": self.dump().display().to_string(),
        }))
        .with_limits(fast_limits())
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.dump().join(name)).unwrap_or_default()
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn send_text(supervisor: &Supervisor, text: &str) -> poph_types::CommandFuture {
    supervisor.send_text(SendText {
        idempotency: "turno-1:0".to_owned(),
        peer: "5511900000000".to_owned(),
        text: text.to_owned(),
    })
}

async fn wait_until(condition: impl Fn() -> bool, deadline: Duration) -> bool {
    let limit = Instant::now() + deadline;
    while Instant::now() < limit {
        if condition() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    condition()
}

fn log_capture() -> &'static Arc<Mutex<Vec<u8>>> {
    struct Writer(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    static LOG: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOG.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let writer_target = Arc::clone(&buffer);
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::TRACE)
                .with_ansi(false)
                .with_writer(move || Writer(Arc::clone(&writer_target)))
                .finish(),
        );
        buffer
    })
}

fn current_log() -> String {
    String::from_utf8_lossy(&log_capture().lock().unwrap()).into_owned()
}

#[tokio::test]
async fn handshake_manifest_health_text_file_and_presence() {
    let scenario = Scenario::new("contrato");
    let collector = Arc::new(EventCollector::new());
    let receiver: Arc<dyn EventSink> = collector.clone();
    let integration = IntegrationId::generate();
    let org = OrgId::generate();
    let options = PluginOptions::new(fixture_binary(), integration, org)
        .with_kind("fixtura")
        .with_config(json!({
            "modo": "normal",
            "dump_dir": scenario.dump().display().to_string(),
        }))
        .with_limits(fast_limits());
    let supervisor = Supervisor::start(options, receiver.clone())
        .await
        .expect("handshake");
    let (integration_id, org_do_supervisor, kind) = supervisor.integration();
    assert_eq!(integration_id, integration);
    assert_eq!(org_do_supervisor, org);
    assert_eq!(kind, "fixtura");

    let manifest = supervisor.manifest().expect("manifesto no handshake");
    assert_eq!(manifest.kind, "fixtura");
    assert_eq!(manifest.protocol, PROTOCOL_VERSION);
    assert!(manifest.capabilities.text);
    assert!(manifest.capabilities.file_out);
    assert!(!manifest.capabilities.read_receipt);
    assert_eq!(manifest.limits.max_attachment_bytes, 16_777_216);
    assert_eq!(supervisor.state(), IntegrationState::Ready);

    let health = supervisor.health().await.expect("saúde");
    assert!(health.ok);
    assert_eq!(health.detail.as_deref(), Some("fixtura"));

    send_text(&supervisor, "olá").await.expect("texto entregue");
    supervisor
        .send_file(SendFile {
            idempotency: "turno-1:1".to_owned(),
            peer: "5511900000000".to_owned(),
            name: "manual.pdf".to_owned(),
            mime: "application/pdf".to_owned(),
            size_bytes: 1_234,
            sha256: "a".repeat(64),
            blob_ref: "01a1f6bd-0000-7000-8000-000000000000/aaaaaaaa".to_owned(),
            caption: Some("segue o manual".to_owned()),
        })
        .await
        .expect("arquivo entregue");
    supervisor
        .set_presence(SetPresence {
            peer: "5511900000000".to_owned(),
            state: Presence::Typing,
        })
        .await
        .expect("presença");

    let commands: Vec<Value> = scenario
        .read("comandos.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).expect("linha de comando em JSON"))
        .collect();
    assert_eq!(commands.len(), 3, "três comandos: {commands:?}");
    assert_eq!(commands[0]["comando"]["tipo"], "enviar_texto");
    assert_eq!(commands[0]["comando"]["idempotencia"], "turno-1:0");
    assert_eq!(commands[0]["comando"]["texto"], "olá");
    assert_eq!(commands[1]["comando"]["tipo"], "enviar_arquivo");
    assert_eq!(commands[1]["comando"]["mime"], "application/pdf");
    assert_eq!(commands[1]["comando"]["legenda"], "segue o manual");
    assert_eq!(commands[2]["comando"]["tipo"], "definir_presenca");
    assert_eq!(commands[2]["comando"]["estado"], "digitando");

    let counters = supervisor.counters();
    assert_eq!(counters.handshakes, 1);
    assert_eq!(counters.commands, 3);
    assert!(counters.health_checks >= 1, "{counters:?}");
    assert_eq!(counters.junk, 0, "{counters:?}");

    supervisor.shutdown().await;
    assert_eq!(supervisor.state(), IntegrationState::Closed);
    assert_eq!(scenario.read("saiu.txt").trim(), "sim");
}

#[tokio::test]
async fn inbound_event_and_state_arrive_normalized() {
    let scenario = Scenario::new("evento");
    let collector = Arc::new(EventCollector::new());
    let receiver: Arc<dyn EventSink> = collector.clone();
    let options = scenario.options("normal").with_config(json!({
        "modo": "normal",
        "dump_dir": scenario.dump().display().to_string(),
        "emitir_evento": true,
        "emitir_estado": "conectado",
        "id_externo": "msg-fixtura-7",
        "peer": "5511988887777",
        "texto": "quero um orçamento",
    }));
    let supervisor = Supervisor::start(options, receiver.clone())
        .await
        .expect("handshake");
    let (integration, _, _) = supervisor.integration();

    let events = collector.wait_for(2, Duration::from_secs(2)).await;
    assert_eq!(events.len(), 2, "entrada + estado: {events:?}");

    let inbound = events
        .iter()
        .find_map(|event| match event {
            IntegrationEvent::Inbound(inbound) => Some(inbound.clone()),
            IntegrationEvent::Estado(_) => None,
        })
        .expect("evento de entrada");
    assert_eq!(inbound.integration, integration);
    assert_eq!(inbound.id_externo, "msg-fixtura-7");
    assert_eq!(inbound.peer, "5511988887777");
    assert_eq!(inbound.text, "quero um orçamento");
    assert_eq!(inbound.occurred_at, 1_795_140_000_000);
    assert!(inbound.attachments.is_empty());
    assert_eq!(inbound.validar(), Ok(()));

    let states = collector.states();
    assert_eq!(states.len(), 1, "{states:?}");
    assert!(matches!(states[0], StateEvent::Connected { .. }));

    let counters = supervisor.counters();
    assert_eq!(counters.events, 1);
    assert_eq!(counters.states, 1);
    supervisor.shutdown().await;
}

#[tokio::test]
async fn plugin_that_hangs_on_the_handshake_is_aborted_by_the_deadline() {
    let scenario = Scenario::new("trava-handshake");
    let collector = Arc::new(EventCollector::new());
    let receiver: Arc<dyn EventSink> = collector.clone();
    let start = Instant::now();
    let error = Supervisor::start(scenario.options("trava_no_handshake"), receiver.clone())
        .await
        .expect_err("o handshake tinha de estourar");
    let elapsed = start.elapsed();
    assert!(
        matches!(error, IntegrationError::Timeout { .. }),
        "erro: {error}"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "demorou {elapsed:?} para desistir"
    );
}

#[tokio::test]
async fn plugin_that_hangs_on_a_command_times_out_and_returns_after_the_restart() {
    let scenario = Scenario::new("trava-comando");
    let collector = Arc::new(EventCollector::new());
    let receiver: Arc<dyn EventSink> = collector.clone();
    let supervisor = Supervisor::start(
        scenario.options("trava_no_primeiro_comando"),
        receiver.clone(),
    )
    .await
    .expect("handshake");

    let start = Instant::now();
    let error = send_text(&supervisor, "primeira")
        .await
        .expect_err("o comando tinha de estourar");
    assert!(
        matches!(error, IntegrationError::Timeout { .. }),
        "erro: {error}"
    );
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(error.is_transient());
    assert!(supervisor.counters().timed_out >= 1);

    assert!(
        wait_until(
            || supervisor.counters().restarts >= 1,
            Duration::from_secs(3)
        )
        .await,
        "o plugin não foi reiniciado: {:?}",
        supervisor.counters()
    );
    supervisor
        .wait_ready(Duration::from_secs(2))
        .await
        .expect("pronto depois do reinício");
    send_text(&supervisor, "segunda")
        .await
        .expect("depois do reinício responde");
    supervisor.shutdown().await;
}

#[tokio::test]
async fn plugin_that_crashes_is_restarted_with_backoff_and_answers_again() {
    let scenario = Scenario::new("cai");
    let collector = Arc::new(EventCollector::new());
    let receiver: Arc<dyn EventSink> = collector.clone();
    let supervisor = Supervisor::start(
        scenario.options("cai_no_primeiro_comando"),
        receiver.clone(),
    )
    .await
    .expect("handshake");

    let error = send_text(&supervisor, "derruba")
        .await
        .expect_err("o plugin sai antes de responder");
    assert!(error.is_transient(), "erro: {error}");

    assert!(
        wait_until(
            || supervisor.counters().restarts >= 1,
            Duration::from_secs(3)
        )
        .await,
        "não houve reinício: {:?}",
        supervisor.counters()
    );
    assert!(supervisor.counters().failures >= 1);
    supervisor
        .wait_ready(Duration::from_secs(2))
        .await
        .expect("pronto depois do reinício");
    send_text(&supervisor, "depois da queda")
        .await
        .expect("voltou a responder");
    assert_eq!(supervisor.counters().handshakes, 2);
    supervisor.shutdown().await;
}

#[tokio::test]
async fn junk_is_ignored_and_counted_without_affecting_another_integration() {
    let scenario_a = Scenario::new("lixo");
    let scenario_b = Scenario::new("limpo");
    let collector = Arc::new(EventCollector::new());
    let receiver: Arc<dyn EventSink> = collector.clone();
    let with_junk = Supervisor::start(scenario_a.options("lixo"), receiver.clone())
        .await
        .expect("handshake do plugin com lixo");
    let clean = Supervisor::start(scenario_b.options("normal"), receiver.clone())
        .await
        .expect("handshake do plugin normal");

    assert!(
        wait_until(|| with_junk.counters().junk >= 2, Duration::from_secs(2)).await,
        "lixo não contado: {:?}",
        with_junk.counters()
    );
    assert_eq!(clean.counters().junk, 0);

    send_text(&with_junk, "ainda funciona")
        .await
        .expect("a integração com lixo continua respondendo");
    send_text(&clean, "também funciona")
        .await
        .expect("a outra integração não foi afetada");
    assert_eq!(with_junk.state(), IntegrationState::Ready);
    assert_eq!(clean.state(), IntegrationState::Ready);

    with_junk.shutdown().await;
    clean.shutdown().await;
}

#[tokio::test]
async fn the_secret_goes_only_in_the_handshake_and_does_not_leak() {
    const SECRET: &str = "sk-fixtura-nao-vaza-42";
    let scenario = Scenario::new("segredo");
    let collector = Arc::new(EventCollector::new());
    let receiver: Arc<dyn EventSink> = collector.clone();
    let options = scenario
        .options("normal")
        .with_config(json!({
            "modo": "normal",
            "dump_dir": scenario.dump().display().to_string(),
            "emitir_evento": true,
        }))
        .with_secret("token", SECRET)
        .with_env("POPH_TESTE_AMBIENTE", "visivel");

    let debug_dump = format!("{options:?} {options:#?}");
    assert!(!debug_dump.contains(SECRET), "Debug vazou: {debug_dump}");

    let supervisor = Supervisor::start(options, receiver.clone())
        .await
        .expect("handshake");

    let secrets = scenario.read("segredos.txt");
    assert_eq!(secrets.trim(), format!("token={}", SECRET.len()));

    let arguments = scenario.read("args.txt");
    assert!(!arguments.contains(SECRET), "argv vazou: {arguments}");
    let environment = scenario.read("env.txt");
    assert!(!environment.contains(SECRET), "ambiente vazou");
    assert!(
        environment.contains("POPH_TESTE_AMBIENTE=visivel"),
        "a variável explícita não chegou: {environment}"
    );
    assert!(
        environment.contains("PATH="),
        "o mínimo (PATH) tem de ir: {environment}"
    );
    for (name, _) in std::env::vars() {
        if matches!(
            name.as_str(),
            "PATH" | "HOME" | "LANG" | "LC_ALL" | "TZ" | "TMPDIR" | "POPH_TESTE_AMBIENTE"
        ) {
            continue;
        }
        assert!(
            !environment.contains(&format!("{name}=")),
            "o plugin herdou `{name}` do ambiente do Harness"
        );
    }

    let events = collector.wait_for(1, Duration::from_secs(2)).await;
    assert!(!events.is_empty(), "nenhum evento chegou");
    for event in &events {
        let json = match event {
            IntegrationEvent::Inbound(inbound) => serde_json::to_string(inbound),
            IntegrationEvent::Estado(state) => serde_json::to_string(state),
        }
        .expect("evento serializável");
        assert!(!json.contains(SECRET), "evento vazou: {json}");
        assert!(!format!("{event:?}").contains(SECRET));
    }

    let large_text = "x".repeat(2 * 1024 * 1024);
    let error = send_text(&supervisor, &large_text)
        .await
        .expect_err("mensagem acima do teto");
    assert!(!error.to_string().contains(SECRET));
    let protocol_error = IntegrationError::Refused {
        operation: "enviar_texto".to_owned(),
        code: "401".to_owned(),
        message: "sessão caiu".to_owned(),
        transient: false,
    };
    assert!(!format!("{protocol_error:?}").contains(SECRET));
    assert!(
        !current_log().contains(SECRET),
        "o segredo apareceu no log:\n{}",
        current_log()
    );

    supervisor.shutdown().await;
}

#[tokio::test]
async fn a_message_above_the_cap_is_refused_without_taking_the_plugin_down() {
    let scenario = Scenario::new("teto");
    let collector = Arc::new(EventCollector::new());
    let receiver: Arc<dyn EventSink> = collector.clone();
    let limits = SupervisorLimits {
        max_message_bytes: 1024,
        ..fast_limits()
    };
    let options = scenario.options("normal").with_limits(limits);
    let supervisor = Supervisor::start(options, receiver.clone())
        .await
        .expect("handshake");

    let error = send_text(&supervisor, &"y".repeat(4096))
        .await
        .expect_err("mensagem acima do teto");
    assert!(
        matches!(error, IntegrationError::MessageTooLarge { cap: 1024, .. }),
        "erro: {error}"
    );
    assert_eq!(supervisor.state(), IntegrationState::Ready);
    assert_eq!(supervisor.counters().failures, 0);
    send_text(&supervisor, "curto")
        .await
        .expect("comando curto continua passando");
    supervisor.shutdown().await;
}

#[tokio::test]
async fn a_different_protocol_version_is_refused() {
    let scenario = Scenario::new("protocolo");
    let collector = Arc::new(EventCollector::new());
    let receiver: Arc<dyn EventSink> = collector.clone();
    let error = Supervisor::start(scenario.options("protocolo_errado"), receiver.clone())
        .await
        .expect_err("versão diferente tinha de falhar");
    match error {
        IntegrationError::IncompatibleProtocol { expected, received } => {
            assert_eq!(expected, PROTOCOL_VERSION);
            assert_eq!(received, 999);
        }
        other => panic!("erro inesperado: {other}"),
    }
}

#[tokio::test]
async fn an_invalid_manifest_is_refused_in_the_handshake() {
    let scenario = Scenario::new("manifesto");
    let collector = Arc::new(EventCollector::new());
    let receiver: Arc<dyn EventSink> = collector.clone();
    let error = Supervisor::start(scenario.options("manifesto_invalido"), receiver.clone())
        .await
        .expect_err("manifesto inválido tinha de falhar");
    assert!(
        matches!(error, IntegrationError::HandshakeRefused { .. }),
        "erro: {error}"
    );
}

#[tokio::test]
async fn after_shutdown_there_is_no_process_and_no_command() {
    let scenario = Scenario::new("encerrar");
    let collector = Arc::new(EventCollector::new());
    let receiver: Arc<dyn EventSink> = collector.clone();
    let supervisor = Supervisor::start(scenario.options("normal"), receiver.clone())
        .await
        .expect("handshake");
    supervisor.shutdown().await;
    assert_eq!(supervisor.state(), IntegrationState::Closed);
    let error = send_text(&supervisor, "depois de encerrar")
        .await
        .expect_err("não há mais processo");
    assert!(matches!(error, IntegrationError::Closed), "{error}");
}

#[tokio::test]
async fn an_integration_marked_as_down_stops_restarting() {
    let scenario = Scenario::new("caida");
    let collector = Arc::new(EventCollector::new());
    let receiver: Arc<dyn EventSink> = collector.clone();
    let supervisor = Supervisor::start(scenario.options("cai_em_todo_comando"), receiver.clone())
        .await
        .expect("handshake");

    for _ in 0..8 {
        if supervisor.state().is_closed() {
            break;
        }
        let _ = supervisor.wait_ready(Duration::from_secs(2)).await;
        if supervisor.state().is_closed() {
            break;
        }
        let _ = send_text(&supervisor, "derruba de novo").await;
        let _ = wait_until(
            || !supervisor.state().is_ready(),
            Duration::from_millis(500),
        )
        .await;
    }
    assert!(
        wait_until(|| supervisor.state().is_closed(), Duration::from_secs(3)).await,
        "não foi marcada como caída: {:?}",
        supervisor.state()
    );
    assert!(matches!(supervisor.state(), IntegrationState::Down { .. }));
    let error = send_text(&supervisor, "não volta mais")
        .await
        .expect_err("caída não envia");
    assert!(matches!(error, IntegrationError::Down { .. }), "{error}");
}
