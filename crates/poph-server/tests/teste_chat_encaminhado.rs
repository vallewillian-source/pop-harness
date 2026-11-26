//! SSE de org remota chegando na dona pelo encaminhamento (HR-50b-2).
//!
//! Duas instâncias **reais** em portas efêmeras e bancos temporários, com o mapa igual dos dois
//! lados (só o `instance_id` muda) e a org + o agente vivendo em `b`. O turno de `b` roda sobre um
//! **provedor falso** que entrega um pedaço de texto e depois fica pendurado por 30 s.
//!
//! O que se prova:
//! - conversa e mensagem de org de `b` enviadas por `a` são atendidas por `b`;
//! - o SSE atravessa o encaminhamento **sem buffer**: o primeiro evento e o primeiro texto chegam
//!   em milissegundos, muito antes dos 30 s que o turno levaria para terminar;
//! - cliente que desconecta no meio do stream derruba o turno na dona (o pedido encaminhado é
//!   cancelado): o turno termina como falha com cancelamento, sem resposta parcial na fila;
//! - nada do turno fica no banco de `a` (quem atendeu foi `b`).

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
use poph_server::Estado;
use poph_server::cluster::ClusterVivo;
use poph_store::{Direction, NewAgent, NewOrg, Page, Store, TurnState};
use poph_types::config::{ClusterConfig, ClusterInstance, GlobalTokenConfig};
use poph_types::{AgentId, ConversationId, HarnessConfig, OrgId, Secret};
use rand_core::OsRng;
use reqwest::{Client, StatusCode};
use serde_json::Value;
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-hr50b2";
const SEGREDO: &str = "SENTINELA-SEGREDO-SSE-hr50b2";
const POPH_LOTE: u32 = 50;

fn hash_argon2(valor: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(valor.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

fn pasta(rotulo: &str) -> PathBuf {
    static CONTADOR: AtomicUsize = AtomicUsize::new(0);
    let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
    let carimbo = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pasta = std::env::temp_dir().join(format!(
        "poph-sse-encaminhado-{rotulo}-{}-{carimbo}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");
    pasta
}

struct Instancia {
    base: String,
    /// Banco da instância (o teste lê o de `b` direto do `Arc` devolvido por `iniciar`).
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

fn configuracao(pasta: &std::path::Path) -> HarnessConfig {
    let mut config = HarnessConfig::default();
    config.data.directory = pasta.join("dados");
    config.admin.tokens_globais = vec![GlobalTokenConfig {
        label: "teste".to_owned(),
        hash: hash_argon2(TOKEN_GLOBAL),
    }];
    config.validar().expect("configuração válida");
    config
}

/// Sobe uma instância; `motor` só existe em `b` (a dona da org).
async fn iniciar(
    instance_id: &str,
    url_a: &str,
    url_b: &str,
    org: &str,
    com_turno: bool,
    listener: tokio::net::TcpListener,
    pasta: PathBuf,
) -> (Instancia, Arc<Store>) {
    let config = configuracao(&pasta);
    let store = Arc::new(Store::open(&config).expect("abre o banco"));
    let mapa = ClusterConfig {
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
    let mut estado = Estado::novo(Arc::clone(&store), config)
        .expect("estado do servidor")
        .com_cluster(ClusterVivo {
            caminho: pasta.join("cluster.toml"),
            config: mapa,
            segredo: Some(Secret::new(SEGREDO.to_owned())),
        });

    if com_turno {
        // O turno de `b`: entrega "começou" e fica pendurado 30 s (tempo de sobra para provar que
        // o SSE atravessa sem esperar o fim).
        let fake = Arc::new(FakeProvider::with_scripts(vec![Script::Pending {
            events: vec![LlmEvent::Texto("começou ".into())],
            after: Duration::from_secs(30),
            error: LlmError::TimedOut { seconds: 30 },
        }]));
        let provedor: Arc<dyn LlmProvider> = fake.clone();
        let motor = Arc::new(TurnEngine::new(
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
        let processador = Arc::new(TurnProcessor::new(Arc::clone(&motor)));
        let atores = Actors::new(
            processador.clone(),
            ActorOptions::default(),
            MetricsMemory::own(),
        );
        let fluxo = Fluxo::new(Arc::clone(&store), atores, FlowOptions::default());
        processador.attach_io(fluxo.clone());
        estado = estado
            .com_motor(motor)
            .com_turno(fluxo, Duration::from_millis(20));
    }

    let endereco = listener.local_addr().expect("endereço local");
    let (desligar, sinal) = oneshot::channel();
    tokio::spawn(async move {
        let _ = poph_server::servir_em(listener, estado, async {
            let _ = sinal.await;
        })
        .await;
    });
    let instancia = Instancia {
        base: format!("http://{endereco}"),
        store: Arc::clone(&store),
        desligar: Some(desligar),
    };
    let cliente = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("cliente HTTP");
    for _ in 0..500 {
        if let Ok(resposta) = cliente
            .get(format!("{}/healthz", instancia.base))
            .send()
            .await
            && resposta.status().is_success()
        {
            return (instancia, store);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("instância {instance_id} não subiu");
}

fn multipart(texto: &str) -> (String, Vec<u8>) {
    let limite = "hr50b2-boundary";
    let mut corpo = Vec::new();
    corpo.extend_from_slice(
        format!("--{limite}\r\nContent-Disposition: form-data; name=\"text\"\r\n\r\n{texto}\r\n")
            .as_bytes(),
    );
    corpo.extend_from_slice(format!("--{limite}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={limite}"), corpo)
}

#[tokio::test(flavor = "multi_thread")]
async fn sse_de_org_remota_flui_pelo_encaminhamento_e_desconexao_cancela_na_dona() {
    // Duas portas efêmeras: os endereços precisam existir antes de os mapas serem montados.
    let listener_a = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta de a");
    let url_a = format!("http://{}", listener_a.local_addr().expect("endereço de a"));
    let listener_b = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta de b");
    let url_b = format!("http://{}", listener_b.local_addr().expect("endereço de b"));

    // A org e o agente vivem em `b`, que também tem o turno.
    let pasta_b = pasta("preparo-b");
    let config_b = configuracao(&pasta_b);
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

    let org_texto = org.to_string();
    let pasta_a = pasta("a");
    let (a, store_a) = iniciar("a", &url_a, &url_b, &org_texto, false, listener_a, pasta_a).await;
    let (_b, store_b) = iniciar("b", &url_a, &url_b, &org_texto, true, listener_b, pasta_b).await;

    let cliente = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("cliente HTTP");
    let raiz = format!("/admin/v1/orgs/{org}/agents/{agente}/test-chat");

    // Conversa criada por `a` e gravada em `b` (o POST também é encaminhado).
    let criada = cliente
        .post(format!("{}{raiz}", a.base))
        .bearer_auth(TOKEN_GLOBAL)
        .send()
        .await
        .expect("cria conversa");
    assert_eq!(criada.status(), StatusCode::CREATED);
    let corpo: Value = criada.json().await.expect("json da conversa");
    let conversa = corpo["id"].as_str().expect("id da conversa").to_owned();
    assert!(
        store_a.orgs().get(org).is_err(),
        "a org não pode existir no banco de `a`"
    );

    // Mensagem enviada por `a`: o SSE tem de chegar sem esperar o turno terminar.
    let (tipo, bytes) = multipart("me responde");
    let inicio = Instant::now();
    let resposta = cliente
        .post(format!("{}{raiz}/{conversa}/messages", a.base))
        .bearer_auth(TOKEN_GLOBAL)
        .header("content-type", tipo)
        .body(bytes)
        .send()
        .await
        .expect("envia a mensagem");
    assert_eq!(resposta.status(), StatusCode::OK);
    assert_eq!(
        resposta
            .headers()
            .get("content-type")
            .and_then(|valor| valor.to_str().ok())
            .unwrap_or_default(),
        "text/event-stream",
        "a resposta encaminhada tem de continuar sendo SSE"
    );

    use futures::StreamExt;
    let mut fluxo = resposta.bytes_stream();
    let mut recebido = String::new();
    let mut primeiro_evento = None;
    let mut primeiro_texto = None;
    while primeiro_texto.is_none() {
        let pedaco = tokio::time::timeout(Duration::from_secs(10), fluxo.next())
            .await
            .expect("o stream precisa abrir (sem buffer)")
            .expect("o SSE precisa emitir")
            .expect("bytes do SSE");
        recebido.push_str(&String::from_utf8_lossy(&pedaco));
        if primeiro_evento.is_none() && recebido.contains("event:") {
            primeiro_evento = Some(inicio.elapsed());
        }
        if recebido.contains("começou") {
            primeiro_texto = Some(inicio.elapsed());
        }
        assert!(
            inicio.elapsed() < Duration::from_secs(10),
            "SSE não atravessou: {recebido}"
        );
    }
    let primeiro_evento = primeiro_evento.expect("primeiro evento");
    let primeiro_texto = primeiro_texto.expect("primeiro texto");
    eprintln!(
        "encaminhado: primeiro evento em {} ms, primeiro texto em {} ms (o turno fica pendurado 30 s)",
        primeiro_evento.as_millis(),
        primeiro_texto.as_millis()
    );
    assert!(
        primeiro_evento < Duration::from_secs(5),
        "o primeiro evento tem de chegar sem buffer (chegou em {primeiro_evento:?})"
    );
    assert!(
        primeiro_texto < Duration::from_secs(10),
        "o texto do modelo tem de fluir antes do fim do turno ({primeiro_texto:?})"
    );
    assert!(
        !recebido.contains("evento: fim") && !recebido.contains("event: fim"),
        "o turno está pendurado; o fim não pode ter chegado: {recebido}"
    );

    // Cliente desconecta no meio: a dona tem de cancelar o turno, sem fila presa.
    drop(fluxo);
    let id: ConversationId = conversa.parse().expect("id da conversa");
    let inicio_da_espera = Instant::now();
    let estado = loop {
        let turnos = store_b
            .turns()
            .list(org, id, Page::with_limit(50))
            .expect("turnos de b");
        if let Some(turno) = turnos.last()
            && turno.state != TurnState::Running
        {
            break (turno.state.as_text().to_owned(), turno.error.clone());
        }
        assert!(
            inicio_da_espera.elapsed() < Duration::from_secs(15),
            "a dona não cancelou o turno depois da desconexão: {turnos:?}"
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
    let saidas = store_b
        .messages()
        .list(org, id, Page::with_limit(200))
        .expect("mensagens de b")
        .iter()
        .filter(|mensagem| mensagem.direction == Direction::Outbound)
        .count();
    assert_eq!(saidas, 0, "nada de resposta parcial na conversa");
    assert_eq!(
        store_b
            .outbox()
            .list_pending(org, POPH_LOTE)
            .expect("fila de b")
            .len(),
        0,
        "nada pode ficar preso na fila da dona"
    );
}
