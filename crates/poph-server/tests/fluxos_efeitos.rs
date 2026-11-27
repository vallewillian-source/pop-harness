use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use poph_agent::Backoff;
use poph_evolution::{ATTACHMENT_CAP, ConfigEvolution, EvolutionClient};
use poph_flows::ProducedEffect;
use poph_flows::ir::Effect;
use poph_server::evolution::EvolutionChannel;
use poph_server::flow_effects::{
    FlowEffectOptions, FlowEffectReport, FlowEffects, RETRY_LIMIT_REASON, effect_of_transition,
};
use poph_server::flow_metrics::FlowMetrics;
use poph_server::whatsapp_output::{WhatsAppOutput, WhatsAppOutputOptions};
use poph_store::ConversationStatus;
use poph_store::{
    Agente, ExecutionStart, ExecutionState, FlowEffect, NewAgent, NewConversation, NewDepartment,
    NewOrg, Store, StoredExecution, TransitionEffect,
};
use poph_types::{
    DepartmentId, EffectState, EventActor, ExecutionStatus, FlowId, HarnessConfig, IntegrationId,
    OrgId, RecordEventKind, RecordId, RecordStatus, Registry as MetricsRegistry, RevisionId,
    Secret,
};
use serde_json::{Value, json};
use tracing_subscriber::fmt::MakeWriter;

const PHONE: &str = "5521999990001";
const TEAM_PHONE: &str = "5521988880002";
const CHAVE: &str = "chave-evo-fx21-teste";
const CAP: u64 = ATTACHMENT_CAP;

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

#[derive(Clone, Debug)]
struct Registro {
    path: String,
    apikey: Option<String>,
    body: Value,
}

#[derive(Clone)]
struct FakeEvolution {
    records: Arc<Mutex<Vec<Registro>>>,
    script: Arc<Mutex<Vec<(u16, Value)>>>,
    calls: Arc<AtomicUsize>,
}

async fn send_fake(
    State(evo): State<FakeEvolution>,
    Path(instancia): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let index = evo.calls.fetch_add(1, Ordering::SeqCst);
    let apikey = headers
        .get("apikey")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    evo.records.lock().unwrap().push(Registro {
        path: format!("instancia={instancia}"),
        apikey,
        body,
    });
    let script = evo.script.lock().unwrap();
    let (status, reply) = script
        .get(index)
        .or_else(|| script.last())
        .cloned()
        .unwrap_or((200, json!({"key": {"id": "MSG-FX21"}})));
    (
        StatusCode::from_u16(status).expect("status do roteiro"),
        Json(reply),
    )
}

async fn start_fake_evolution(
    script: Vec<(u16, Value)>,
) -> (String, FakeEvolution, tokio::task::JoinHandle<()>) {
    let evo = FakeEvolution {
        records: Arc::new(Mutex::new(Vec::new())),
        script: Arc::new(Mutex::new(script)),
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let router = Router::new()
        .route("/message/sendText/{instancia}", post(send_fake))
        .route("/message/sendMedia/{instancia}", post(send_fake))
        .with_state(evo.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera da Evolution falsa");
    let endereco = listener.local_addr().expect("endereço da Evolution falsa");
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{endereco}"), evo, task)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Instancia {
    Normal,
    NoKey,
    NoChannel,
}

struct Scenario {
    store: Arc<Store>,
    clock: Arc<AtomicI64>,
    org: OrgId,
    agente: Agente,
    integracao: Option<IntegrationId>,
    conversa: poph_types::ConversationId,
    ficha: RecordId,
    fluxo: FlowId,
    revision: RevisionId,
    evo: FakeEvolution,
    channels: Vec<EvolutionChannel>,
    metrics: Arc<FlowMetrics>,
    blobs: PathBuf,
    dir: PathBuf,
    evo_task: tokio::task::JoinHandle<()>,
}

impl Scenario {
    async fn fresh(label: &str, mode: Instancia, script: Vec<(u16, Value)>) -> Self {
        captured_logs();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("poph-fx21-{label}-{}-{stamp}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = dir.join("dados");
        config.server.port = 7481;
        config.validar().expect("configuração válida");
        let blobs = config.blobs();

        let clock = Arc::new(AtomicI64::new(1_700_000_000_000));
        let ponteiro = Arc::clone(&clock);
        let store = Arc::new(
            Store::open(&config)
                .expect("abre o banco")
                .with_clock(Arc::new(move || ponteiro.load(Ordering::SeqCst))),
        );

        let org = store
            .orgs()
            .create(NewOrg {
                name: "Loja FX-21".to_owned(),
                slug: "loja-fx21".to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Atendente FX-21".to_owned(),
                    slug: "atendente-fx21".to_owned(),
                    ..Default::default()
                },
            )
            .expect("cria o agente");

        let integracao: Option<IntegrationId> = if mode == Instancia::NoChannel {
            None
        } else {
            Some(
                store
                    .create_integration(org, agente.id, "whatsapp".to_owned(), "active".to_owned())
                    .expect("cria a integração")
                    .id
                    .parse()
                    .expect("id da integração"),
            )
        };

        let conversa = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente.id,
                    integration_id: integracao,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some(PHONE.to_owned()),
                    target_department: None,
                },
            )
            .expect("cria a conversa")
            .id;
        let ficha = store
            .cases()
            .by_conversation(org, conversa)
            .expect("consulta a Ficha")
            .expect("a conversa nasce com Ficha")
            .id;

        let (fluxo, revision) = published_flow(&store, org);

        let (url, evo, evo_task) = start_fake_evolution(script).await;
        let customer = if mode == Instancia::NoKey {
            None
        } else {
            Some(
                EvolutionClient::new(
                    ConfigEvolution::new(url, "loja", Secret::new(CHAVE.to_owned()))
                        .expect("config da Evolution falsa"),
                )
                .expect("cliente da Evolution falsa"),
            )
        };
        let channels = match (mode, integracao) {
            (Instancia::NoChannel, _) | (_, None) => Vec::new(),
            (_, Some(integracao)) => vec![EvolutionChannel {
                org,
                instance: "loja".to_owned(),
                integration: integracao,
                secret: None,
                client: customer,
                attachment_limit: CAP,
            }],
        };

        let registry = MetricsRegistry::new();
        let metrics =
            Arc::new(FlowMetrics::register(&registry).expect("registra as métricas de fluxos"));

        Self {
            store,
            clock,
            org,
            agente,
            integracao,
            conversa,
            ficha,
            fluxo,
            revision,
            evo,
            channels,
            metrics,
            blobs,
            dir,
            evo_task,
        }
    }

    fn effects(&self) -> FlowEffects {
        FlowEffects::new(
            Arc::clone(&self.store),
            &self.channels,
            self.blobs.clone(),
            Some(Arc::clone(&self.metrics)),
            FlowEffectOptions {
                backoff: Backoff {
                    base: Duration::from_millis(1),
                    factor: 2,
                    cap: Duration::from_millis(5),
                },
                missing_integration_delay: Duration::from_millis(10),
                ..FlowEffectOptions::default()
            },
        )
    }

    fn saida(&self) -> WhatsAppOutput {
        WhatsAppOutput::new(
            Arc::clone(&self.store),
            &self.channels,
            self.blobs.clone(),
            WhatsAppOutputOptions {
                backoff: Backoff {
                    base: Duration::from_millis(1),
                    factor: 2,
                    cap: Duration::from_millis(5),
                },
                ..WhatsAppOutputOptions::default()
            },
        )
    }

    fn advance(&self, ms: i64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }

    fn open(&self, effects: Vec<TransitionEffect>) -> StoredExecution {
        self.open_in_case(self.ficha, effects)
    }

    fn open_in_case(&self, ficha: RecordId, effects: Vec<TransitionEffect>) -> StoredExecution {
        self.store
            .flows()
            .release(self.org, self.agente.id, self.fluxo)
            .expect("libera o fluxo ao agente");
        self.store
            .executions()
            .start_execution(
                self.org,
                ExecutionStart {
                    chave_idempotencia: None,
                    case: ficha,
                    flow: self.fluxo,
                    revision: self.revision,
                    actor: EventActor::Agent,
                    ator_ref: Some("agente-de-teste"),
                    jev_decision: None,
                    initial_state: ExecutionState {
                        status: ExecutionStatus::Active,
                        no_atual: "boas_vindas".to_owned(),
                        dados_json: "{}".to_owned(),
                        acting_actors: Vec::new(),
                        result: None,
                        reason: None,
                    },
                    inbound_effects: effects,
                },
            )
            .expect("abre a execução")
    }

    fn other_case(&self, integracao: Option<IntegrationId>) -> RecordId {
        let canal = if integracao.is_some() {
            "whatsapp"
        } else {
            "console"
        };
        let conversa = self
            .store
            .conversations()
            .create(
                self.org,
                NewConversation {
                    agent_id: self.agente.id,
                    integration_id: integracao,
                    canal: canal.to_owned(),
                    peer_externo: Some("5521977770003".to_owned()),
                    target_department: None,
                },
            )
            .expect("cria a segunda conversa")
            .id;
        self.store
            .cases()
            .by_conversation(self.org, conversa)
            .expect("consulta")
            .expect("Ficha da segunda conversa")
            .id
    }

    fn case_effects(&self, execution: poph_types::ExecutionId) -> Vec<FlowEffect> {
        self.store
            .executions()
            .effects(self.org, execution)
            .expect("lê os efeitos")
    }

    fn events(&self, ficha: RecordId) -> Vec<poph_store::RecordEvent> {
        self.store
            .cases()
            .events(self.org, ficha, None, 100)
            .expect("lê os eventos")
    }

    fn conversa(&self) -> poph_store::Conversa {
        self.store
            .conversations()
            .get(self.org, self.conversa)
            .expect("lê a conversa")
    }

    fn outbox_items(&self) -> Vec<poph_store::OutboxItem> {
        self.store
            .outbox()
            .list_pending(self.org, 100)
            .expect("lê a outbox")
    }

    fn total_items(&self) -> i64 {
        let conn = rusqlite::Connection::open(self.store.harness_path()).expect("abre para contar");
        conn.query_row(
            "SELECT COUNT(*) FROM outbox WHERE org_id = ?1",
            rusqlite::params![self.org.to_string()],
            |line| line.get::<_, i64>(0),
        )
        .expect("conta a outbox")
    }

    fn records(&self) -> Vec<Registro> {
        self.evo.records.lock().unwrap().clone()
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        self.evo_task.abort();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn published_flow(store: &Store, org: OrgId) -> (FlowId, RevisionId) {
    let fluxo: FlowId = uuid::Uuid::now_v7().to_string().parse().expect("fluxo");
    let revision: RevisionId = uuid::Uuid::now_v7().to_string().parse().expect("revisão");
    let conn = rusqlite::Connection::open(store.harness_path()).expect("abre para gravar");
    conn.execute(
        "INSERT INTO fluxos (id, org_id, slug, nome, descricao, status, rascunho_fonte,
                             revisao_atual, version)
         VALUES (?1, ?2, 'boas_vindas', 'Boas-vindas', '', 'active', '', 1, 1)",
        rusqlite::params![fluxo.to_string(), org.to_string()],
    )
    .expect("fluxo");
    conn.execute(
        "INSERT INTO fluxo_revisoes (id, org_id, fluxo_id, numero, fonte, fonte_sha256,
                                    definicao_json, schema_versao, publicada_em, publicada_por,
                                    resolucoes_json)
         VALUES (?1, ?2, ?3, 1, 'fluxo()', 'sha', '{}', 1, 1, 'admin_token', '{}')",
        rusqlite::params![revision.to_string(), org.to_string(), fluxo.to_string()],
    )
    .expect("revisão");
    (fluxo, revision)
}

fn to_customer(texto: &str) -> TransitionEffect {
    effect_of_transition(&ProducedEffect {
        chave: "etapa:boas_vindas/0".to_owned(),
        no: "etapa:boas_vindas".to_owned(),
        index: 0,
        effect: Effect::MessageToCustomer {
            texto: texto.to_owned(),
        },
    })
}

fn notify(mensagem: &str) -> TransitionEffect {
    effect_of_transition(&ProducedEffect {
        chave: "acao:boas_vindas.registrar/0".to_owned(),
        no: "acao:boas_vindas.registrar".to_owned(),
        index: 0,
        effect: Effect::NotifyWhatsApp {
            para: TEAM_PHONE.to_owned(),
            mensagem: mensagem.to_owned(),
        },
    })
}

fn encaminhar(departamento: &str) -> TransitionEffect {
    effect_of_transition(&ProducedEffect {
        chave: "acao:boas_vindas.transferir/0".to_owned(),
        no: "acao:boas_vindas.transferir".to_owned(),
        index: 0,
        effect: Effect::Encaminhar {
            departamento: departamento.to_owned(),
        },
    })
}

fn departamento(scenario: &Scenario, nome: &str) -> DepartmentId {
    scenario
        .store
        .departments()
        .create(
            scenario.org,
            NewDepartment {
                name: nome.to_owned(),
                description: String::new(),
            },
        )
        .expect("cria o departamento")
        .id
}

#[tokio::test]
async fn customer_message_reaches_customer_once() {
    let scenario = Scenario::fresh("ao-cliente", Instancia::Normal, Vec::new()).await;
    let opened = scenario.open(vec![to_customer("Recebemos seu pedido!")]);
    let execution = opened.execution.id;
    assert_eq!(
        opened.effects.len(),
        1,
        "o efeito entrou na fila na abertura"
    );

    let effects = scenario.effects();
    let report = effects.cycle_for_org(scenario.org).await.expect("ciclo");
    assert_eq!(report.sent, 1, "um efeito entregue");

    let queue = scenario.case_effects(execution);
    assert_eq!(queue[0].state, EffectState::Sent);
    assert!(queue[0].enviado_em.is_some(), "carimbo de entrega");

    let items = scenario.outbox_items();
    assert_eq!(items.len(), 1, "um item na outbox");
    assert_eq!(items[0].conversation_id, scenario.conversa);
    assert_eq!(
        serde_json::from_str::<Value>(&items[0].content_json).expect("payload")["texto"],
        "Recebemos seu pedido!"
    );

    let saida = scenario.saida();
    let sent_items = saida.cycle().await.expect("ciclo da saída");
    assert_eq!(sent_items.sent, 1);
    let records = scenario.records();
    assert_eq!(records.len(), 1, "uma chamada à Evolution");
    assert_eq!(records[0].body["number"], PHONE);
    assert_eq!(records[0].body["text"], "Recebemos seu pedido!");
    assert_eq!(records[0].path, "instancia=loja", "pela instância da org");

    let report = effects
        .cycle_for_org(scenario.org)
        .await
        .expect("segundo ciclo");
    assert_eq!(report.sent, 0, "efeito entregue não volta para a fila");
    assert_eq!(scenario.total_items(), 1, "sem segundo item na outbox");

    assert_eq!(scenario.metrics.pending_effects.value(), 1);
    assert_eq!(scenario.metrics.sent_effects.value(), 1);
    assert_eq!(scenario.metrics.failed_effects.value(), 0);

    let sent_items: Vec<_> = scenario
        .events(scenario.ficha)
        .into_iter()
        .filter(|evento| evento.tipo == RecordEventKind::EffectSent)
        .collect();
    assert_eq!(sent_items.len(), 1, "um evento `efeito_enviado`");
}

#[tokio::test]
async fn notify_whatsapp_goes_to_team_not_customer() {
    let scenario = Scenario::fresh("notificar", Instancia::Normal, Vec::new()).await;
    let opened = scenario.open(vec![notify("Visita marcada para amanhã")]);
    let execution = opened.execution.id;

    let report = scenario
        .effects()
        .cycle_for_org(scenario.org)
        .await
        .expect("ciclo");
    assert_eq!(report.sent, 1);

    let records = scenario.records();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].body["number"], TEAM_PHONE,
        "vai para o número da equipe"
    );
    assert_ne!(records[0].body["number"], PHONE, "não vai para o cliente");
    assert_eq!(records[0].body["text"], "Visita marcada para amanhã");
    assert_eq!(records[0].apikey.as_deref(), Some(CHAVE));

    let queue = scenario.case_effects(execution);
    assert_eq!(queue[0].state, EffectState::Sent);
    assert!(
        scenario.outbox_items().is_empty(),
        "a notificação não usa a outbox da conversa"
    );
}

#[tokio::test]
async fn without_integration_effect_stays_pending_and_does_not_block_loop() {
    let scenario = Scenario::fresh("sem-chave", Instancia::NoKey, Vec::new()).await;
    let opened = scenario.open(vec![notify("aviso para a equipe")]);
    let execution = opened.execution.id;

    let effects = scenario.effects();
    let report = effects.cycle_for_org(scenario.org).await.expect("ciclo");
    assert_eq!(report.deferred, 1, "espera, não falha");
    assert_eq!(report.permanent_failures, 0);

    let queue = scenario.case_effects(execution);
    assert_eq!(queue[0].state, EffectState::Pending, "continua na fila");
    assert_eq!(queue[0].attempts, 0, "esperar não consome tentativa");
    assert!(queue[0].ultimo_erro.is_none(), "esperar não é erro");
    assert!(
        queue[0].proxima_tentativa.is_some(),
        "a espera fica marcada"
    );

    let report = effects.cycle_for_org(scenario.org).await.expect("ciclo");
    assert_eq!(report.deferred, 0, "a espera segura o efeito");
    assert_eq!(scenario.records().len(), 0, "nenhuma chamada à Evolution");

    let other = scenario.other_case(None);
    scenario.open_in_case(other, vec![to_customer("Mensagem de outra Ficha")]);
    scenario.advance(60);
    let report = effects.cycle_for_org(scenario.org).await.expect("ciclo");
    assert_eq!(report.sent, 1, "o outro efeito foi entregue");
    assert_eq!(report.deferred, 1, "o da equipe continua esperando");
}

#[tokio::test]
async fn crash_between_enqueue_and_confirm_resends_and_accepts_duplicate() {
    let scenario = Scenario::fresh("queda", Instancia::Normal, Vec::new()).await;
    let opened = scenario.open(vec![to_customer("Bem-vindo!")]);
    let execution = opened.execution.id;
    let id = scenario.case_effects(execution)[0].id;

    scenario
        .store
        .effects()
        .start_send(scenario.org, id)
        .expect("pega o efeito");
    let (_, item) = scenario
        .store
        .outbox()
        .enqueue(
            scenario.org,
            poph_store::NewOutboxItem {
                conversation_id: scenario.conversa,
                turn_id: poph_types::TurnId::generate(),
                seq: 0,
                tipo: "text".to_owned(),
                content_json: json!({"texto": "Bem-vindo!"}).to_string(),
            },
        )
        .expect("enfileira a mensagem");
    assert!(!item.id.to_string().is_empty(), "o item da outbox existe");
    assert_eq!(
        scenario.case_effects(execution)[0].state,
        EffectState::Sending,
        "a marca da tentativa ficou gravada"
    );

    let report = scenario
        .effects()
        .cycle_for_org(scenario.org)
        .await
        .expect("ciclo");
    assert_eq!(report.sent, 1, "o reenvio acontece");
    assert_eq!(scenario.case_effects(execution)[0].state, EffectState::Sent);

    let items = scenario.outbox_items();
    assert_eq!(items.len(), 2, "a duplicata rara é aceita, não escondida");
    scenario.saida().cycle().await.expect("ciclo da saída");
    assert_eq!(scenario.records().len(), 2, "as duas chegam ao cliente");
}

#[tokio::test]
async fn permanent_failure_becomes_failed_with_event_and_metric() {
    let scenario = Scenario::fresh(
        "definitiva",
        Instancia::Normal,
        vec![(400, json!({"message": "número inválido"}))],
    )
    .await;
    let opened = scenario.open(vec![notify("aviso")]);
    let execution = opened.execution.id;

    let report = scenario
        .effects()
        .cycle_for_org(scenario.org)
        .await
        .expect("ciclo");
    assert_eq!(report.permanent_failures, 1, "4xx não repete");

    let queue = scenario.case_effects(execution);
    assert_eq!(queue[0].state, EffectState::Failed);
    assert!(queue[0].ultimo_erro.is_some(), "o motivo fica gravado");
    assert_eq!(queue[0].attempts, 1);
    assert_eq!(scenario.metrics.failed_effects.value(), 1);

    let current = scenario
        .store
        .executions()
        .get(scenario.org, execution)
        .expect("lê a execução");
    assert_eq!(current.status, ExecutionStatus::Active);
    assert_eq!(scenario.conversa().status, ConversationStatus::Open);

    let failures: Vec<_> = scenario
        .events(scenario.ficha)
        .into_iter()
        .filter(|evento| evento.tipo == RecordEventKind::EffectFailed)
        .collect();
    assert_eq!(failures.len(), 1, "um evento `efeito_falhou`");

    let report = scenario
        .effects()
        .cycle_for_org(scenario.org)
        .await
        .expect("ciclo");
    assert_eq!(report, FlowEffectReport::default(), "nada mais a fazer");
}

#[tokio::test]
async fn transient_failure_retries_and_passes() {
    let scenario = Scenario::fresh(
        "transitoria",
        Instancia::Normal,
        vec![
            (503, json!({"message": "indisponível"})),
            (200, json!({"key": {"id": "MSG-FX21-2"}})),
        ],
    )
    .await;
    let opened = scenario.open(vec![notify("aviso")]);
    let execution = opened.execution.id;

    let effects = scenario.effects();
    let report = effects.cycle_for_org(scenario.org).await.expect("ciclo");
    assert_eq!(report.transient_failures, 1);
    let queue = scenario.case_effects(execution);
    assert_eq!(queue[0].state, EffectState::Pending, "volta para a fila");
    assert_eq!(queue[0].attempts, 1);
    assert!(queue[0].proxima_tentativa.is_some(), "espera marcada");

    let report = effects.cycle_for_org(scenario.org).await.expect("ciclo");
    assert_eq!(report, FlowEffectReport::default());
    assert_eq!(scenario.records().len(), 1, "uma tentativa até aqui");

    scenario.advance(50);
    let report = effects.cycle_for_org(scenario.org).await.expect("ciclo");
    assert_eq!(report.sent, 1);
    assert_eq!(scenario.records().len(), 2);
    let queue = scenario.case_effects(execution);
    assert_eq!(queue[0].state, EffectState::Sent);
    assert_eq!(queue[0].attempts, 2, "duas tentativas contadas");
}

#[tokio::test]
async fn forward_transfers_conversation_and_keeps_case_open() {
    let scenario = Scenario::fresh("encaminhar", Instancia::Normal, Vec::new()).await;
    let sales = departamento(&scenario, "Comercial");
    let opened = scenario.open(vec![encaminhar("Comercial")]);
    let execution = opened.execution.id;

    let report = scenario
        .effects()
        .cycle_for_org(scenario.org)
        .await
        .expect("ciclo");
    assert_eq!(report.sent, 1);

    let conversa = scenario.conversa();
    assert_eq!(conversa.status, ConversationStatus::Transferred);
    assert_eq!(conversa.target_department, Some(sales));

    let ficha = scenario
        .store
        .cases()
        .get(scenario.org, scenario.ficha)
        .expect("lê a Ficha");
    assert_eq!(ficha.status, RecordStatus::Open);

    let audit = scenario
        .store
        .audit()
        .list(
            scenario.org,
            poph_store::Page {
                after: None,
                limit: 50,
            },
        )
        .expect("lê a auditoria");
    let transfer = audit
        .iter()
        .find(|registro| registro.acao == "transfer")
        .expect("auditoria da transferência");
    assert_eq!(transfer.ator_tipo, "system");

    let types: Vec<RecordEventKind> = scenario
        .events(scenario.ficha)
        .into_iter()
        .map(|evento| evento.tipo)
        .collect();
    assert!(types.contains(&RecordEventKind::EffectSent));
    assert!(types.contains(&RecordEventKind::ServiceTransferred));

    assert_eq!(scenario.case_effects(execution)[0].state, EffectState::Sent);
}

#[tokio::test]
async fn missing_department_becomes_failed_without_reverting_transition() {
    let scenario = Scenario::fresh("sem-departamento", Instancia::Normal, Vec::new()).await;
    departamento(&scenario, "Comercial");
    departamento(&scenario, "Suporte");
    let opened = scenario.open(vec![encaminhar("Financeiro")]);
    let execution = opened.execution.id;

    let report = scenario
        .effects()
        .cycle_for_org(scenario.org)
        .await
        .expect("ciclo");
    assert_eq!(report.permanent_failures, 1);

    let queue = scenario.case_effects(execution);
    assert_eq!(queue[0].state, EffectState::Failed);
    let motivo = queue[0].ultimo_erro.clone().unwrap_or_default();
    assert!(
        motivo.contains("Financeiro"),
        "o motivo diz qual departamento faltou: {motivo}"
    );

    assert_eq!(scenario.conversa().status, ConversationStatus::Open);
    let current = scenario
        .store
        .executions()
        .get(scenario.org, execution)
        .expect("lê a execução");
    assert_eq!(current.status, ExecutionStatus::Active);
}

#[tokio::test]
async fn whatsapp_4000_character_limit_is_respected() {
    let scenario = Scenario::fresh("limite", Instancia::Normal, Vec::new()).await;
    let long = "a".repeat(5_000);
    scenario.open(vec![notify(&long)]);

    let report = scenario
        .effects()
        .cycle_for_org(scenario.org)
        .await
        .expect("ciclo");
    assert_eq!(report.sent, 1);

    let records = scenario.records();
    let sent = records[0].body["text"]
        .as_str()
        .expect("texto enviado")
        .to_owned();
    assert_eq!(sent.chars().count(), 4_000, "corta no teto do canal");
    assert!(sent.ends_with('…'), "e avisa que cortou: {sent:.10}");
}

#[tokio::test]
async fn phone_does_not_appear_in_log_or_stored_reason() {
    let logs = captured_logs();
    let scenario = Scenario::fresh(
        "telefone",
        Instancia::Normal,
        vec![
            (200, json!({"key": {"id": "MSG-FX21-3"}})),
            (400, json!({"message": "destino recusado"})),
        ],
    )
    .await;
    scenario.open(vec![notify("aviso 1")]);
    let other = scenario.other_case(scenario.integracao);
    let failure = scenario.open_in_case(other, vec![notify("aviso 2")]);
    let effects = scenario.effects();
    let report = effects.cycle_for_org(scenario.org).await.expect("ciclo");
    assert_eq!(report.sent, 1);
    assert_eq!(report.permanent_failures, 1);

    let motivo = scenario.case_effects(failure.execution.id)[0]
        .ultimo_erro
        .clone()
        .unwrap_or_default();
    assert!(!motivo.is_empty(), "a falha tem motivo");
    assert!(
        !motivo.contains(TEAM_PHONE),
        "motivo sem telefone: {motivo}"
    );
    assert!(!motivo.contains(CHAVE), "motivo sem chave: {motivo}");

    let texto = String::from_utf8_lossy(&logs.lock().unwrap()).to_string();
    assert!(!texto.is_empty(), "o coletor de log funcionou");
    assert!(
        !texto.contains(PHONE),
        "o telefone do cliente não pode aparecer no log"
    );
    assert!(
        !texto.contains(TEAM_PHONE),
        "o telefone da equipe não pode aparecer no log"
    );
    assert!(
        !texto.contains(CHAVE),
        "a chave da Evolution não pode aparecer no log"
    );
}

#[tokio::test]
async fn customer_message_waits_for_conversation_turn() {
    let scenario = Scenario::fresh("espera-turno", Instancia::Normal, Vec::new()).await;
    let opened = scenario.open(vec![to_customer("Mensagem fixa da etapa")]);
    let execution = opened.execution.id;

    let turno = scenario
        .store
        .turns()
        .start(scenario.org, scenario.conversa, Some(scenario.agente.id))
        .expect("abre o turno");

    let effects = scenario.effects();
    let report = effects.cycle_for_org(scenario.org).await.expect("ciclo");
    assert_eq!(report.deferred, 1);
    assert!(scenario.outbox_items().is_empty(), "nada na outbox ainda");
    let queue = scenario.case_effects(execution);
    assert_eq!(queue[0].state, EffectState::Pending);
    assert_eq!(
        queue[0].attempts, 0,
        "esperar o turno não consome tentativa"
    );

    scenario
        .store
        .turns()
        .finalise(
            scenario.org,
            turno.id,
            poph_store::TurnFinalization {
                state: Some(poph_store::TurnState::Completed),
                ..Default::default()
            },
        )
        .expect("fecha o turno");
    scenario.advance(50);
    let report = effects.cycle_for_org(scenario.org).await.expect("ciclo");
    assert_eq!(report.sent, 1);
    assert_eq!(scenario.outbox_items().len(), 1);
}

#[tokio::test]
async fn customer_message_with_closed_conversation_becomes_failed() {
    let scenario = Scenario::fresh("encerrada", Instancia::Normal, Vec::new()).await;
    let opened = scenario.open(vec![to_customer("Mensagem fixa")]);
    let execution = opened.execution.id;

    scenario
        .store
        .conversations()
        .close_audited(
            scenario.org,
            scenario.conversa,
            scenario.conversa().version,
            "atendente-de-teste".to_owned(),
        )
        .expect("fecha a conversa");

    let report = scenario
        .effects()
        .cycle_for_org(scenario.org)
        .await
        .expect("ciclo");
    assert_eq!(report.permanent_failures, 1);

    let queue = scenario.case_effects(execution);
    assert_eq!(queue[0].state, EffectState::Failed);
    assert!(
        queue[0]
            .ultimo_erro
            .as_deref()
            .unwrap_or_default()
            .contains("encerrada"),
        "o motivo diz que a conversa está encerrada: {:?}",
        queue[0].ultimo_erro
    );
    assert_eq!(
        scenario.total_items(),
        0,
        "nada entra na outbox de conversa encerrada"
    );
}

#[tokio::test]
async fn exhausted_attempts_become_failed_with_correct_reason() {
    let scenario = Scenario::fresh(
        "esgotado",
        Instancia::Normal,
        vec![(503, json!({"message": "indisponível"}))],
    )
    .await;
    let opened = scenario.open(vec![notify("aviso")]);
    let execution = opened.execution.id;
    let effects = FlowEffects::new(
        Arc::clone(&scenario.store),
        &scenario.channels,
        scenario.blobs.clone(),
        Some(Arc::clone(&scenario.metrics)),
        FlowEffectOptions {
            max_attempts: 2,
            backoff: Backoff {
                base: Duration::from_millis(1),
                factor: 2,
                cap: Duration::from_millis(5),
            },
            missing_integration_delay: Duration::from_millis(10),
            ..FlowEffectOptions::default()
        },
    );

    effects.cycle_for_org(scenario.org).await.expect("1ª");
    scenario.advance(50);
    effects.cycle_for_org(scenario.org).await.expect("2ª");
    scenario.advance(50);
    let report = effects.cycle_for_org(scenario.org).await.expect("3ª");
    assert_eq!(report.exhausted, 1);

    let queue = scenario.case_effects(execution);
    assert_eq!(queue[0].state, EffectState::Failed);
    assert_eq!(queue[0].attempts, 2, "o teto de tentativas vale");
    assert_eq!(queue[0].ultimo_erro.as_deref(), Some(RETRY_LIMIT_REASON));
    assert_eq!(scenario.metrics.failed_effects.value(), 1);
}
