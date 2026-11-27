use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use poph_agent::actors::{ActorOptions, Actors};
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::entry_output::{FlowOptions, Fluxo};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsFlow, MetricsMemory, MetricsTurn};
use poph_agent::turno::{TurnEngine, TurnOptions, TurnProcessor};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, LlmProvider, Script, Usage};
use poph_evolution::webhook::SECRET_HEADER;
use poph_evolution::{ATTACHMENT_CAP, ConfigEvolution, EvolutionClient};
use poph_server::ServerState;
use poph_server::evolution::EvolutionChannel;
use poph_store::{
    ConversationStatus, Direction, NewAgent, NewConversation, NewOrg, NewOutboxItem, Origin,
    OutboxState, Page, Store, TurnState,
};
use poph_types::{AgentId, ConversationId, HarnessConfig, IntegrationId, OrgId, Registry, Secret};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tracing_subscriber::fmt::MakeWriter;

const SECRET: &str = "segredo-webhook-turno-26h";
const EVO_KEY: &str = "chave-evo-turno-26h-teste";
const PHONE: &str = "5511999990001";
const JID: &str = "5511999990001@s.whatsapp.net";
const QUESTION: &str = "qual o prazo de entrega do pedido quatro quatro sete um";
const REPLY: &str = "o prazo é de cinco dias úteis após a confirmação";
const INSTANCIA: &str = "loja";

#[derive(Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .extend_from_slice(bytes);
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

fn logs_so_far() -> String {
    let bytes = captured_logs()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[derive(Debug, Clone)]
struct Send {
    apikey: Option<String>,
    body: Value,
}

#[derive(Clone, Default)]
struct FakeEvolution {
    sends: Arc<Mutex<Vec<Send>>>,
    script: Arc<Mutex<VecDeque<u16>>>,
    counter: Arc<AtomicUsize>,
}

impl FakeEvolution {
    fn sends(&self) -> Vec<Send> {
        self.sends
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}

async fn send_fake(
    State(evo): State<FakeEvolution>,
    Path(_instance): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let apikey = headers
        .get("apikey")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    evo.sends
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .push(Send { apikey, body });
    let n = evo.counter.fetch_add(1, Ordering::SeqCst) + 1;
    let status = evo
        .script
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .pop_front()
        .unwrap_or(200);
    (
        StatusCode::from_u16(status).expect("status do roteiro"),
        Json(json!({"key": {"id": format!("MSG-SAIDA-{n}")}})),
    )
}

async fn start_fake_evolution(evo: FakeEvolution) -> String {
    let router = Router::new()
        .route("/message/sendText/{instancia}", post(send_fake))
        .route("/message/sendMedia/{instancia}", post(send_fake))
        .with_state(evo);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera da Evolution falsa");
    let endereco = listener.local_addr().expect("endereço da Evolution falsa");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    format!("http://{endereco}")
}

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    integracao: IntegrationId,
    evo: FakeEvolution,
    url_evo: String,
}

impl Scenario {
    async fn new(label: &str) -> Self {
        captured_logs();
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let folder = std::env::temp_dir().join(format!(
            "poph-turno-26h-{label}-{}-{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("pasta do teste");
        let store = Arc::new(
            Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
                .expect("abre o banco"),
        );
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Loja Turno".to_owned(),
                slug: format!("loja-turno-{label}"),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Atendente Loja".to_owned(),
                    slug: "atendente-loja".to_owned(),
                    modelo: "gemini-3.8-flash-tiered".to_owned(),
                    grouping_window_ms: 0,
                    ..Default::default()
                },
            )
            .expect("cria o agente")
            .id;
        store
            .instructions()
            .save(
                org,
                agente,
                0,
                "{}",
                "# Atendente\n\nVocê atende a loja.",
                true,
            )
            .expect("grava as instruções");
        let integracao: IntegrationId = store
            .create_integration(org, agente, "whatsapp".to_owned(), "active".to_owned())
            .expect("cria a integração")
            .id
            .parse()
            .expect("id da integração");

        let evo = FakeEvolution::default();
        let url_evo = start_fake_evolution(evo.clone()).await;
        Self {
            folder,
            store,
            org,
            agente,
            integracao,
            evo,
            url_evo,
        }
    }

    fn canal(&self) -> EvolutionChannel {
        let client = EvolutionClient::new(
            ConfigEvolution::new(
                self.url_evo.clone(),
                INSTANCIA,
                Secret::new(EVO_KEY.to_owned()),
            )
            .expect("config da Evolution falsa"),
        )
        .expect("cliente da Evolution falsa");
        EvolutionChannel {
            org: self.org,
            instance: INSTANCIA.to_owned(),
            integration: self.integracao,
            secret: Some(Secret::new(SECRET.to_owned())),
            client: Some(client),
            attachment_limit: ATTACHMENT_CAP,
        }
    }

    fn fluxo(
        &self,
        registro: &Registry,
        scripts: Vec<Script>,
        options: FlowOptions,
    ) -> (Fluxo, Arc<FakeProvider>) {
        let fake = Arc::new(FakeProvider::with_scripts(scripts));
        let provedor: Arc<dyn LlmProvider> = fake.clone();
        let config = ConfigHot::new(
            Arc::clone(&self.store),
            HotConfigOptions::default(),
            MetricsMemory::own(),
        );
        let limiter = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
        let engine = TurnEngine::new(
            Arc::clone(&self.store),
            config,
            limiter,
            provedor,
            TurnOptions::default(),
            MetricsTurn::register(registro).expect("métricas do turno"),
        );
        let processor = Arc::new(TurnProcessor::new(Arc::new(engine)));
        let actors = Actors::new(
            processor.clone(),
            ActorOptions::default(),
            MetricsMemory::own(),
        );
        let fluxo = Fluxo::with_metrics(
            Arc::clone(&self.store),
            actors,
            options,
            MetricsFlow::register(registro).expect("métricas da fila"),
        );
        processor.attach_io(fluxo.clone());
        (fluxo, fake)
    }

    async fn start(&self, scripts: Vec<Script>, enable_turn: bool) -> Servidor {
        self.start_with(scripts, enable_turn, FlowOptions::default())
            .await
    }

    async fn start_with(
        &self,
        scripts: Vec<Script>,
        enable_turn: bool,
        options: FlowOptions,
    ) -> Servidor {
        let estado = ServerState::new(Arc::clone(&self.store), HarnessConfig::default())
            .expect("estado do servidor")
            .with_evolution_channels(vec![self.canal()]);
        let (estado, llm) = if enable_turn {
            let (fluxo, llm) = self.fluxo(&estado.registry, scripts, options);
            (
                estado.with_turn(fluxo, Duration::from_millis(20)),
                Some(llm),
            )
        } else {
            (estado, None)
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, signal) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = poph_server::serve_at(listener, estado, async {
                let _ = signal.await;
            })
            .await;
        });
        Servidor {
            base: format!("http://{endereco}"),
            client: Client::new(),
            org: self.org,
            llm,
            desligar: Some(desligar),
        }
    }

    fn conversa(&self) -> Option<ConversationId> {
        self.store
            .conversations()
            .list(self.org, None, Page::with_limit(10))
            .ok()?
            .first()
            .map(|conversa| conversa.id)
    }

    fn pending_inputs(&self) -> u64 {
        self.store
            .inbox()
            .count_pending(self.org)
            .expect("conta a fila de entrada")
    }

    fn outputs(&self, estado: OutboxState) -> u64 {
        self.store
            .outbox()
            .count_by_state(self.org, estado)
            .expect("conta a fila de saída")
    }

    fn turns(&self) -> Vec<poph_store::Turno> {
        match self.conversa() {
            Some(conversa) => self
                .store
                .turns()
                .list(self.org, conversa, Page::with_limit(50))
                .expect("lista os turnos"),
            None => Vec::new(),
        }
    }

    fn stored_replies(&self) -> Vec<String> {
        let Some(conversa) = self.conversa() else {
            return Vec::new();
        };
        self.store
            .messages()
            .list(self.org, conversa, Page::with_limit(100))
            .expect("lista as mensagens")
            .into_iter()
            .filter(|mensagem| mensagem.direction == Direction::Outbound)
            .map(|mensagem| mensagem.texto)
            .collect()
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.folder);
    }
}

struct Servidor {
    base: String,
    client: Client,
    org: OrgId,
    llm: Option<Arc<FakeProvider>>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Servidor {
    async fn webhook(&self, body: Value) -> (StatusCode, Value) {
        let url = format!("{}/webhook/evolution/{}/{INSTANCIA}", self.base, self.org);
        let response = self
            .client
            .post(url)
            .header(SECRET_HEADER, SECRET)
            .json(&body)
            .send()
            .await
            .expect("entrega o webhook");
        let status = response.status();
        let value = response.json::<Value>().await.unwrap_or(Value::Null);
        (status, value)
    }

    async fn metrics(&self) -> String {
        self.client
            .get(format!("{}/metrics", self.base))
            .send()
            .await
            .expect("lê as métricas")
            .text()
            .await
            .expect("texto das métricas")
    }

    fn llm_received(&self) -> usize {
        self.llm.as_ref().map_or(0, |llm| llm.received().len())
    }

    async fn desligar(mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
}

fn info_line(logs: &str, texto: &str) -> bool {
    logs.lines()
        .any(|line| line.contains(" INFO ") && line.contains(texto))
}

fn metric_value(texto: &str, nome: &str) -> f64 {
    for line in texto.lines() {
        if line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        if parts.next() == Some(nome)
            && let Some(value) = parts
                .next()
                .and_then(|raw_bytes| raw_bytes.parse::<f64>().ok())
        {
            return value;
        }
    }
    0.0
}

async fn wait(label: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..400 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let logs = logs_so_far();
    let cauda: String = logs
        .chars()
        .rev()
        .take(2500)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    panic!("não aconteceu a tempo: {label}\n--- cauda dos logs ---\n{cauda}");
}

fn text_event(id: &str, texto: &str) -> Value {
    json!({
        "event": "messages.upsert",
        "instance": INSTANCIA,
        "apikey": EVO_KEY,
        "destination": "http://harness.teste/webhook/evolution/x/loja",
        "data": {
            "key": {"id": id, "remoteJid": JID, "fromMe": false},
            "pushName": "Cliente Teste",
            "messageType": "conversation",
            "message": {"conversation": texto},
            "messageTimestamp": 1795177000
        }
    })
}

fn device_event(id: &str, texto: &str) -> Value {
    let mut body = text_event(id, texto);
    body["data"]["key"]["fromMe"] = json!(true);
    body
}

fn response(texto: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(texto.to_owned()),
        LlmEvent::Usage(Usage {
            input: 120,
            saida: 30,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

#[tokio::test]
async fn client_message_becomes_reply_via_evolution_once() {
    let scenario = Scenario::new("feliz").await;
    let servidor = scenario.start(vec![response(REPLY)], true).await;

    let (status, _) = servidor.webhook(text_event("MSG-FELIZ-1", QUESTION)).await;
    assert_eq!(status, StatusCode::OK);

    wait("a resposta sair pela Evolution", || {
        scenario.outputs(OutboxState::Sent) == 1
    })
    .await;

    assert_eq!(
        servidor.llm_received(),
        1,
        "um turno, uma chamada ao modelo"
    );
    let sends = scenario.evo.sends();
    assert_eq!(sends.len(), 1, "a resposta sai uma vez");
    assert_eq!(sends[0].apikey.as_deref(), Some(EVO_KEY));
    assert!(
        sends[0].body.to_string().contains(REPLY),
        "o envio leva a resposta do agente"
    );
    assert!(
        !sends[0].body.to_string().contains(EVO_KEY),
        "a chave não vai no corpo do envio"
    );

    assert_eq!(scenario.pending_inputs(), 0);
    let turns = scenario.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].state, TurnState::Completed);
    assert_eq!(scenario.stored_replies(), vec![REPLY.to_owned()]);

    let logs = logs_so_far();
    assert!(
        info_line(&logs, "turno concluído"),
        "o turno deixou uma linha de info"
    );
    for forbidden in [QUESTION, REPLY, PHONE, JID, EVO_KEY, SECRET] {
        assert!(
            !logs.contains(forbidden),
            "log não pode conter `{forbidden}`"
        );
    }

    servidor.desligar().await;
}

#[tokio::test]
async fn redelivery_of_same_event_does_not_open_second_turn() {
    let scenario = Scenario::new("reentrega").await;
    let servidor = scenario.start(vec![response(REPLY)], true).await;

    let (_, first_one) = servidor.webhook(text_event("MSG-REENT-1", QUESTION)).await;
    assert_eq!(first_one["situacao"], "nova");
    wait("a primeira resposta sair", || {
        scenario.outputs(OutboxState::Sent) == 1
    })
    .await;

    let (status, second_one) = servidor.webhook(text_event("MSG-REENT-1", QUESTION)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second_one["situacao"], "duplicada");
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert_eq!(servidor.llm_received(), 1, "a reentrega não abre turno");
    assert_eq!(
        scenario.evo.sends().len(),
        1,
        "a reentrega não gera segundo envio"
    );
    assert_eq!(scenario.stored_replies().len(), 1);

    servidor.desligar().await;
}

#[tokio::test]
async fn transferred_conversation_does_not_reply_and_closes_with_reason() {
    let scenario = Scenario::new("transferida").await;
    let servidor = scenario
        .start(
            vec![response(REPLY), response("isto não deveria sair")],
            true,
        )
        .await;

    servidor.webhook(text_event("MSG-TRANS-1", QUESTION)).await;
    wait("a primeira resposta sair", || {
        scenario.outputs(OutboxState::Sent) == 1
    })
    .await;

    let conversa = scenario.conversa().expect("a conversa existe");
    let current = scenario
        .store
        .conversations()
        .get(scenario.org, conversa)
        .expect("lê a conversa");
    scenario
        .store
        .conversations()
        .change_status(
            scenario.org,
            conversa,
            current.version,
            ConversationStatus::Transferred,
            None,
        )
        .expect("transfere a conversa");

    servidor
        .webhook(text_event("MSG-TRANS-2", "estou esperando alguém"))
        .await;
    wait("a entrada ser concluída", || {
        scenario.pending_inputs() == 0
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(
        servidor.llm_received(),
        1,
        "transferida não chama o modelo de novo"
    );
    assert_eq!(
        scenario.evo.sends().len(),
        1,
        "nada sai depois da transferência"
    );
    assert_eq!(scenario.outputs(OutboxState::Pending), 0);
    let turns = scenario.turns();
    let last = turns.last().expect("há turno");
    assert_eq!(last.state, TurnState::Skipped);
    assert!(
        last.error
            .as_deref()
            .unwrap_or_default()
            .contains("não responde"),
        "o motivo fica registrado no turno"
    );
    assert_eq!(scenario.stored_replies().len(), 1);

    servidor.desligar().await;
}

#[tokio::test]
async fn device_message_is_not_answered_by_agent() {
    let scenario = Scenario::new("aparelho").await;
    scenario
        .store
        .matrix()
        .enable(scenario.org)
        .expect("liga o módulo Matrix da org");
    let servidor = scenario
        .start(
            vec![response(REPLY), response("isto não deveria sair")],
            true,
        )
        .await;

    let (status, body) = servidor.webhook(device_event("MSG-AP-1", QUESTION)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["efeito"], "aparelho");
    assert_eq!(body["situacao"], "nova");

    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(servidor.llm_received(), 0, "o modelo não é chamado");
    assert!(scenario.evo.sends().is_empty(), "nada sai para o cliente");
    assert_eq!(
        scenario.pending_inputs(),
        0,
        "a mensagem do aparelho não entra na fila do turno"
    );
    assert_eq!(
        scenario.outputs(OutboxState::Pending),
        0,
        "fila de saída vazia"
    );
    assert!(scenario.turns().is_empty(), "nenhum turno aberto");

    let conversa = scenario.conversa().expect("a conversa existe no banco");
    let stored = scenario
        .store
        .messages()
        .list(scenario.org, conversa, Page::with_limit(20))
        .expect("lista as mensagens");
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].direction, Direction::Outbound);
    assert_eq!(stored[0].origin, Origin::Device);
    assert!(stored[0].turn_id.is_none());

    servidor.webhook(text_event("MSG-AP-2", QUESTION)).await;
    wait("a entrada ser concluída", || {
        scenario.pending_inputs() == 0
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        servidor.llm_received(),
        0,
        "conversa transferida pelo aparelho não chama o modelo"
    );
    assert_eq!(scenario.evo.sends().len(), 0, "nada sai depois da pausa");
    let turns = scenario.turns();
    let last = turns.last().expect("há turno");
    assert_eq!(last.state, TurnState::Skipped);

    servidor.desligar().await;
}

#[tokio::test]
async fn inactive_integration_does_not_reply_and_closes_with_reason() {
    let scenario = Scenario::new("inativa").await;
    let servidor = scenario.start(vec![response(REPLY)], true).await;

    let current = scenario
        .store
        .get_integration(scenario.org, scenario.agente, scenario.integracao)
        .expect("lê a integração");
    scenario
        .store
        .update_integration(
            scenario.org,
            scenario.agente,
            scenario.integracao,
            poph_store::IntegrationChange {
                version: current.version,
                status: Some("inactive"),
                ..Default::default()
            },
        )
        .expect("desativa a integração");

    let (status, _) = servidor.webhook(text_event("MSG-INAT-1", QUESTION)).await;
    assert_eq!(status, StatusCode::OK, "a entrega é aceita");
    wait("a entrada ser concluída", || {
        scenario.pending_inputs() == 0
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(
        servidor.llm_received(),
        0,
        "integração inativa não chama o modelo"
    );
    assert!(scenario.evo.sends().is_empty(), "nada sai pela Evolution");
    assert_eq!(scenario.outputs(OutboxState::Pending), 0);
    let turns = scenario.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].state, TurnState::Skipped);
    assert!(
        turns[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("não respondida"),
        "o motivo fica registrado no turno"
    );
    assert_eq!(
        scenario.stored_replies().len(),
        0,
        "a mensagem do cliente não gera resposta"
    );
    let logs = logs_so_far();
    assert!(
        info_line(&logs, "turno sem resposta"),
        "o motivo de não responder sai no log, em info"
    );
    assert!(
        !logs.contains(QUESTION),
        "o texto do cliente não vai para o log"
    );

    servidor.desligar().await;
}

#[tokio::test]
async fn without_turn_message_stays_queued_and_is_answered_on_start() {
    let scenario = Scenario::new("sem-turno").await;

    let stopped = scenario.start(Vec::new(), false).await;
    let (status, _) = stopped.webhook(text_event("MSG-FILA-1", QUESTION)).await;
    assert_eq!(status, StatusCode::OK);
    wait("a mensagem entrar na fila", || {
        scenario.pending_inputs() == 1
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        scenario.pending_inputs(),
        1,
        "a mensagem segue na fila, sem ser perdida"
    );
    assert!(scenario.evo.sends().is_empty());
    stopped.desligar().await;

    let servidor = scenario.start(vec![response(REPLY)], true).await;
    wait("a resposta da fila sair", || {
        scenario.outputs(OutboxState::Sent) == 1
    })
    .await;
    assert_eq!(servidor.llm_received(), 1);
    assert_eq!(scenario.evo.sends().len(), 1);
    assert_eq!(scenario.pending_inputs(), 0);

    servidor.desligar().await;
}

#[tokio::test]
async fn reply_stuck_in_sending_after_crash_goes_out_on_start() {
    let scenario = Scenario::new("queda").await;

    let conversa = scenario
        .store
        .conversations()
        .create(
            scenario.org,
            NewConversation {
                agent_id: scenario.agente,
                integration_id: Some(scenario.integracao),
                canal: "whatsapp".to_owned(),
                peer_externo: Some(PHONE.to_owned()),
                target_department: None,
            },
        )
        .expect("abre a conversa")
        .id;
    let turno = scenario
        .store
        .turns()
        .start(scenario.org, conversa, Some(scenario.agente))
        .expect("abre o turno");
    let (_, item) = scenario
        .store
        .outbox()
        .enqueue(
            scenario.org,
            NewOutboxItem {
                conversation_id: conversa,
                turn_id: turno.id,
                seq: 1,
                tipo: "text".to_owned(),
                content_json: json!({"texto": REPLY}).to_string(),
            },
        )
        .expect("enfileira a resposta");
    scenario
        .store
        .outbox()
        .start_send(scenario.org, item.id)
        .expect("marca como em envio");
    assert_eq!(scenario.outputs(OutboxState::Sending), 1);

    let servidor = scenario.start(Vec::new(), false).await;
    wait("a resposta retomada sair", || {
        scenario.outputs(OutboxState::Sent) == 1
    })
    .await;
    let sends = scenario.evo.sends();
    assert_eq!(sends.len(), 1, "sai uma vez, na retomada");
    assert!(sends[0].body.to_string().contains(REPLY));

    servidor.desligar().await;
}

#[tokio::test]
async fn interrupted_turn_after_crash_resumes_once_on_start() {
    let scenario = Scenario::new("queda-turno").await;

    let delivery = scenario
        .store
        .inbox()
        .receive_from_channel(
            scenario.org,
            poph_store::NewChannelInboxItem {
                integration_id: scenario.integracao,
                agent_id: scenario.agente,
                canal: "whatsapp".to_owned(),
                external_id: "MSG-QUEDA-TURNO".to_owned(),
                peer: PHONE.to_owned(),
                event_json: json!({"texto": QUESTION}).to_string(),
                texto: QUESTION.to_owned(),
                attachments: Vec::new(),
                customer_name: None,
            },
        )
        .expect("entrega do processo anterior");
    let conversa = delivery.mensagem.conversation_id;
    let interrupted_turn = scenario
        .store
        .turns()
        .start(scenario.org, conversa, Some(scenario.agente))
        .expect("turno do processo anterior");

    let servidor = scenario
        .start(vec![response(REPLY), response(REPLY)], true)
        .await;
    wait("a resposta retomada sair", || {
        scenario.outputs(OutboxState::Sent) == 1
    })
    .await;

    let stored = scenario
        .store
        .turns()
        .get(scenario.org, interrupted_turn.id)
        .expect("turno interrompido");
    assert_eq!(stored.state, TurnState::Failed);
    assert_eq!(
        stored.error.as_deref(),
        Some(poph_server::turn_input::INTERRUPTED_REASON)
    );

    assert_eq!(servidor.llm_received(), 1);
    assert_eq!(scenario.stored_replies(), vec![REPLY.to_owned()]);
    assert_eq!(scenario.evo.sends().len(), 1, "sai uma vez, na retomada");
    assert_eq!(scenario.pending_inputs(), 0);
    assert_eq!(
        scenario.outputs(OutboxState::Sending),
        0,
        "nada preso em envio"
    );

    let (status, _) = servidor.webhook(text_event("MSG-QUEDA-2", QUESTION)).await;
    assert_eq!(status, StatusCode::OK);
    wait("a segunda resposta sair", || {
        scenario.outputs(OutboxState::Sent) == 2
    })
    .await;
    assert_eq!(scenario.stored_replies().len(), 2);

    servidor.desligar().await;
}

#[tokio::test]
async fn turn_metrics_appear_in_metrics() {
    let scenario = Scenario::new("metricas").await;
    let servidor = scenario.start(vec![response(REPLY)], true).await;

    servidor.webhook(text_event("MSG-MET-1", QUESTION)).await;
    wait("a resposta sair", || {
        scenario.outputs(OutboxState::Sent) == 1
    })
    .await;

    let texto = servidor.metrics().await;
    assert_eq!(metric_value(&texto, "poph_turnos_iniciados_total"), 1.0);
    assert_eq!(metric_value(&texto, "poph_turnos_concluidos_total"), 1.0);
    assert_eq!(metric_value(&texto, "poph_turnos_falhos_total"), 0.0);

    servidor.desligar().await;
}

fn provider_failure() -> Script {
    Script::Failure(poph_core::llm::LlmError::Unavailable {
        tentativas: 3,
        last_error: "503 indisponível".to_owned(),
    })
}

#[tokio::test]
async fn model_failure_retries_and_reply_goes_out_once_via_evolution() {
    let scenario = Scenario::new("falha-repete").await;
    let options = FlowOptions::default().with_failure_model(3, Duration::from_millis(50));
    let servidor = scenario
        .start_with(
            vec![provider_failure(), provider_failure(), response(REPLY)],
            true,
            options,
        )
        .await;

    let (status, _) = servidor.webhook(text_event("MSG-FALHA-1", QUESTION)).await;
    assert_eq!(status, StatusCode::OK);

    wait("a resposta sair depois das repetições", || {
        scenario.outputs(OutboxState::Sent) == 1
    })
    .await;

    assert_eq!(servidor.llm_received(), 3, "uma chamada por tentativa");
    let sends = scenario.evo.sends();
    assert_eq!(sends.len(), 1, "a resposta sai uma vez");
    assert!(sends[0].body.to_string().contains(REPLY));
    assert_eq!(scenario.pending_inputs(), 0);
    let turns = scenario.turns();
    assert_eq!(
        turns
            .iter()
            .filter(|t| t.state == TurnState::Failed)
            .count(),
        2
    );
    assert_eq!(scenario.stored_replies(), vec![REPLY.to_owned()]);

    let metrics = servidor.metrics().await;
    assert_eq!(
        metric_value(&metrics, "poph_turnos_falha_modelo_adiadas_total"),
        2.0
    );
    assert_eq!(
        metric_value(&metrics, "poph_turnos_falha_modelo_definitivas_total"),
        0.0
    );
    let logs = logs_so_far();
    assert!(
        logs.contains("falha do modelo: mensagem adiada para nova tentativa"),
        "o adiamento deixou uma linha"
    );
    for forbidden in [QUESTION, REPLY, PHONE, JID, EVO_KEY, SECRET] {
        assert!(
            !logs.contains(forbidden),
            "log não pode conter `{forbidden}`"
        );
    }

    servidor.desligar().await;
}

#[tokio::test]
async fn model_failure_exhausted_sends_nothing_and_logs_reason() {
    let scenario = Scenario::new("falha-esgota").await;
    let options = FlowOptions::default().with_failure_model(3, Duration::from_millis(50));
    let servidor = scenario
        .start_with(
            vec![provider_failure(), provider_failure(), provider_failure()],
            true,
            options,
        )
        .await;

    servidor.webhook(text_event("MSG-ESGOTA-1", QUESTION)).await;

    wait("a entrada sair da fila depois da última tentativa", || {
        scenario.pending_inputs() == 0
    })
    .await;

    assert_eq!(servidor.llm_received(), 3);
    assert!(scenario.evo.sends().is_empty(), "nada foi enviado");
    assert!(scenario.stored_replies().is_empty());
    assert_eq!(scenario.outputs(OutboxState::Sent), 0);

    let metrics = servidor.metrics().await;
    assert_eq!(
        metric_value(&metrics, "poph_turnos_falha_modelo_definitivas_total"),
        1.0
    );
    let logs = logs_so_far();
    assert!(
        logs.contains("tentativas esgotadas; mensagem concluída sem resposta"),
        "a falha definitiva deixou uma linha"
    );
    for forbidden in [QUESTION, PHONE, JID, EVO_KEY, SECRET] {
        assert!(
            !logs.contains(forbidden),
            "log não pode conter `{forbidden}`"
        );
    }

    servidor.desligar().await;
}

#[tokio::test]
async fn provider_refusal_401_does_not_retry_warns_operation_and_sends_nothing() {
    let scenario = Scenario::new("recusa-401").await;
    let options = FlowOptions::default().with_failure_model(3, Duration::from_millis(50));
    let servidor = scenario
        .start_with(
            vec![Script::Failure(poph_core::llm::LlmError::Rejected {
                status: 401,
                mensagem: "chave recusada".to_owned(),
            })],
            true,
            options,
        )
        .await;

    servidor.webhook(text_event("MSG-401-1", QUESTION)).await;
    wait("a entrada sair da fila na primeira tentativa", || {
        scenario.pending_inputs() == 0
    })
    .await;

    assert_eq!(servidor.llm_received(), 1, "a recusa não é repetida");
    assert!(scenario.evo.sends().is_empty(), "nada foi enviado");
    assert!(scenario.stored_replies().is_empty());

    let metrics = servidor.metrics().await;
    assert_eq!(
        metric_value(&metrics, "poph_turnos_falha_modelo_recusadas_total"),
        1.0
    );
    assert_eq!(
        metric_value(&metrics, "poph_turnos_falha_modelo_adiadas_total"),
        0.0
    );
    let logs = logs_so_far();
    assert!(
        logs.contains("provedor recusou o pedido"),
        "o aviso da recusa apareceu no log"
    );
    for forbidden in [QUESTION, REPLY, PHONE, JID, EVO_KEY, SECRET] {
        assert!(
            !logs.contains(forbidden),
            "log não pode conter `{forbidden}`"
        );
    }

    servidor.desligar().await;
}
