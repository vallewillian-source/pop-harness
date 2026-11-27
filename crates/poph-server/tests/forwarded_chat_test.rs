use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_agent::actors::{ActorOptions, Actors};
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::entry_output::{FlowOptions, Fluxo};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::turno::{TurnEngine, TurnOptions, TurnProcessor};
use poph_core::llm::{FakeProvider, LlmError, LlmEvent, LlmProvider, Script};
use poph_server::ServerState;
use poph_server::cluster::LiveCluster;
use poph_store::{Direction, NewAgent, NewOrg, Page, Store, TurnState};
use poph_types::config::{ClusterConfig, ClusterInstance, GlobalTokenConfig};
use poph_types::{AgentId, ConversationId, HarnessConfig, OrgId, Secret};
use rand_core::OsRng;
use reqwest::{Client, StatusCode};
use serde_json::Value;
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-hr50b2";
const SECRET: &str = "SENTINELA-SEGREDO-SSE-hr50b2";
const POPH_BATCH: u32 = 50;

fn hash_argon2(value: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &salt)
        .expect("hash argon2")
        .to_string()
}

fn folder(label: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let folder = std::env::temp_dir().join(format!(
        "poph-sse-encaminhado-{label}-{}-{stamp}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(folder.join("dados")).expect("pasta de dados");
    folder
}

struct Instancia {
    base: String,
    #[allow(dead_code)]
    store: Arc<Store>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Drop for Instancia {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
    }
}

fn configuracao(folder: &std::path::Path) -> HarnessConfig {
    let mut config = HarnessConfig::default();
    config.data.directory = folder.join("dados");
    config.admin.tokens_globais = vec![GlobalTokenConfig {
        label: "teste".to_owned(),
        hash: hash_argon2(TOKEN_GLOBAL),
    }];
    config.validar().expect("configuração válida");
    config
}

async fn start_instance(
    instance_id: &str,
    url_a: &str,
    url_b: &str,
    org: &str,
    with_turn: bool,
    listener: tokio::net::TcpListener,
    folder: PathBuf,
) -> (Instancia, Arc<Store>) {
    let config = configuracao(&folder);
    let store = Arc::new(Store::open(&config).expect("abre o banco"));
    let map = ClusterConfig {
        instance_id: instance_id.to_owned(),
        default_instance: Some("b".to_owned()),
        instances: vec![
            ClusterInstance {
                id: "a".to_owned(),
                url: url_a.to_owned(),
            },
            ClusterInstance {
                id: "b".to_owned(),
                url: url_b.to_owned(),
            },
        ],
        orgs: BTreeMap::from([(org.to_owned(), "b".to_owned())]),
    };
    let mut estado = ServerState::new(Arc::clone(&store), config)
        .expect("estado do servidor")
        .with_cluster(LiveCluster {
            path: folder.join("cluster.toml"),
            config: map,
            secret: Some(Secret::new(SECRET.to_owned())),
        });

    if with_turn {
        let fake = Arc::new(FakeProvider::with_scripts(vec![Script::Pending {
            events: vec![LlmEvent::Texto("começou ".into())],
            after: Duration::from_secs(30),
            error: LlmError::TimedOut { seconds: 30 },
        }]));
        let provedor: Arc<dyn LlmProvider> = fake.clone();
        let engine = Arc::new(TurnEngine::new(
            Arc::clone(&store),
            ConfigHot::new(
                Arc::clone(&store),
                HotConfigOptions::default(),
                MetricsMemory::own(),
            ),
            Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own())),
            provedor,
            TurnOptions::default(),
            MetricsTurn::own(),
        ));
        let processor = Arc::new(TurnProcessor::new(Arc::clone(&engine)));
        let actors = Actors::new(
            processor.clone(),
            ActorOptions::default(),
            MetricsMemory::own(),
        );
        let fluxo = Fluxo::new(Arc::clone(&store), actors, FlowOptions::default());
        processor.attach_io(fluxo.clone());
        estado = estado
            .with_engine(engine)
            .with_turn(fluxo, Duration::from_millis(20));
    }

    let endereco = listener.local_addr().expect("endereço local");
    let (desligar, signal) = oneshot::channel();
    tokio::spawn(async move {
        let _ = poph_server::serve_at(listener, estado, async {
            let _ = signal.await;
        })
        .await;
    });
    let instancia = Instancia {
        base: format!("http://{endereco}"),
        store: Arc::clone(&store),
        desligar: Some(desligar),
    };
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("cliente HTTP");
    for _ in 0..500 {
        if let Ok(response) = client
            .get(format!("{}/healthz", instancia.base))
            .send()
            .await
            && response.status().is_success()
        {
            return (instancia, store);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("instância {instance_id} não subiu");
}

fn multipart(texto: &str) -> (String, Vec<u8>) {
    let limit = "hr50b2-boundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!("--{limit}\r\nContent-Disposition: form-data; name=\"text\"\r\n\r\n{texto}\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(format!("--{limit}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={limit}"), body)
}

#[tokio::test(flavor = "multi_thread")]
async fn sse_from_remote_org_flows_through_forwarding_and_disconnect_cancels_on_owner() {
    let listener_a = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta de a");
    let url_a = format!("http://{}", listener_a.local_addr().expect("endereço de a"));
    let listener_b = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta de b");
    let url_b = format!("http://{}", listener_b.local_addr().expect("endereço de b"));

    let folder_b = folder("preparo-b");
    let config_b = configuracao(&folder_b);
    let store_b = Arc::new(Store::open(&config_b).expect("banco de b"));
    let org: OrgId = store_b
        .orgs()
        .create(NewOrg {
            name: "Org da dona".to_owned(),
            slug: "org-da-dona-hr50b2".to_owned(),
            config_json: "{}".to_owned(),
        })
        .expect("org")
        .id;
    let agente: AgentId = store_b
        .agents()
        .create(
            org,
            NewAgent {
                name: "Bia".to_owned(),
                slug: "bia".to_owned(),
                grouping_window_ms: 1,
                ..Default::default()
            },
        )
        .expect("agente")
        .id;
    drop(store_b);

    let org_text = org.to_string();
    let folder_a = folder("a");
    let (a, store_a) =
        start_instance("a", &url_a, &url_b, &org_text, false, listener_a, folder_a).await;
    let (_b, store_b) =
        start_instance("b", &url_a, &url_b, &org_text, true, listener_b, folder_b).await;

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("cliente HTTP");
    let root = format!("/admin/v1/orgs/{org}/agents/{agente}/test-chat");

    let created = client
        .post(format!("{}{root}", a.base))
        .bearer_auth(TOKEN_GLOBAL)
        .send()
        .await
        .expect("cria conversa");
    assert_eq!(created.status(), StatusCode::CREATED);
    let body: Value = created.json().await.expect("json da conversa");
    let conversa = body["id"].as_str().expect("id da conversa").to_owned();
    assert!(
        store_a.orgs().get(org).is_err(),
        "a org não pode existir no banco de `a`"
    );

    let (tipo, bytes) = multipart("me responde");
    let started = Instant::now();
    let response = client
        .post(format!("{}{root}/{conversa}/messages", a.base))
        .bearer_auth(TOKEN_GLOBAL)
        .header("content-type", tipo)
        .body(bytes)
        .send()
        .await
        .expect("envia a mensagem");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "text/event-stream",
        "a resposta encaminhada tem de continuar sendo SSE"
    );

    use futures::StreamExt;
    let mut fluxo = response.bytes_stream();
    let mut received = String::new();
    let mut first_event = None;
    let mut first_text = None;
    while first_text.is_none() {
        let piece = tokio::time::timeout(Duration::from_secs(10), fluxo.next())
            .await
            .expect("o stream precisa abrir (sem buffer)")
            .expect("o SSE precisa emitir")
            .expect("bytes do SSE");
        received.push_str(&String::from_utf8_lossy(&piece));
        if first_event.is_none() && received.contains("event:") {
            first_event = Some(started.elapsed());
        }
        if received.contains("começou") {
            first_text = Some(started.elapsed());
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "SSE não atravessou: {received}"
        );
    }
    let first_event = first_event.expect("primeiro evento");
    let first_text = first_text.expect("primeiro texto");
    eprintln!(
        "encaminhado: primeiro evento em {} ms, primeiro texto em {} ms (o turno fica pendurado 30 s)",
        first_event.as_millis(),
        first_text.as_millis()
    );
    assert!(
        first_event < Duration::from_secs(5),
        "o primeiro evento tem de chegar sem buffer (chegou em {first_event:?})"
    );
    assert!(
        first_text < Duration::from_secs(10),
        "o texto do modelo tem de fluir antes do fim do turno ({first_text:?})"
    );
    assert!(
        !received.contains("evento: fim") && !received.contains("event: fim"),
        "o turno está pendurado; o fim não pode ter chegado: {received}"
    );

    drop(fluxo);
    let id: ConversationId = conversa.parse().expect("id da conversa");
    let wait_started = Instant::now();
    let estado = loop {
        let turns = store_b
            .turns()
            .list(org, id, Page::with_limit(50))
            .expect("turnos de b");
        if let Some(turno) = turns.last()
            && turno.state != TurnState::Running
        {
            break (turno.state.as_text().to_owned(), turno.error.clone());
        }
        assert!(
            wait_started.elapsed() < Duration::from_secs(15),
            "a dona não cancelou o turno depois da desconexão: {turns:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(estado.0, "failed", "o turno precisa terminar como falha");
    assert!(
        estado
            .1
            .as_deref()
            .unwrap_or_default()
            .to_lowercase()
            .contains("cancel"),
        "o turno precisa registrar o cancelamento: {estado:?}"
    );
    let outputs = store_b
        .messages()
        .list(org, id, Page::with_limit(200))
        .expect("mensagens de b")
        .iter()
        .filter(|mensagem| mensagem.direction == Direction::Outbound)
        .count();
    assert_eq!(outputs, 0, "nada de resposta parcial na conversa");
    assert_eq!(
        store_b
            .outbox()
            .list_pending(org, POPH_BATCH)
            .expect("fila de b")
            .len(),
        0,
        "nada pode ficar preso na fila da dona"
    );
}
