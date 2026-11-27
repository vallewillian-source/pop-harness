use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{post, put};
use poph_agent::Backoff;
use poph_evolution::{ATTACHMENT_CAP, ConfigEvolution, EvolutionClient};
use poph_matrix::{ClientConfig, GhostIds, MatrixClient};
use poph_server::evolution::EvolutionChannel;
use poph_server::matrix_failure_notice::{FailureNotice, HarnessFailuresPort, SynapseNotice};
use poph_server::whatsapp_output::{WhatsAppOutput, WhatsAppOutputOptions};
use poph_store::{
    HumanResponse, NewAgent, NewConversation, NewHumanResponse, NewOrg, NewOutboxItem, OutboxItem,
    OutboxState, OutcomeNotice, Store,
};
use poph_types::{AgentId, ConversationId, IntegrationId, OrgId, Secret, TurnId};
use serde_json::{Value, json};
use tracing_subscriber::fmt::MakeWriter;

const PREFIX: &str = "ph";
const SERVER_NAME: &str = "matrix-dev.local";
const INSTANCIA: &str = "loja";
const EVO_KEY: &str = "chave-evo-mx19-teste";

const PHONE: &str = "5511999990000";
const ATTENDANT_TEXT: &str = "CANARIO-TEXTO-mx19-resposta-do-atendente";
const AS_TOKEN: &str = "CANARIO-AS-mx19-7d2e-TAO-SECRETO";

const ROOM: &str = "!sala-mx19:matrix-dev.local";
const EVENTO: &str = "$evento-atendente-mx19:matrix-dev.local";
const ATTENDANT: &str = "@ana:matrix-dev.local";
const ATTENDANT_NAME: &str = "Ana Atendente";

const NOTICE_REFUSED: &str =
    "Não consegui entregar sua mensagem ao cliente: o WhatsApp recusou o envio";
const NOTICE_EXHAUSTED: &str = "Não consegui entregar sua mensagem ao cliente: o WhatsApp não respondeu depois de várias tentativas";

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
struct Write {
    room: String,
    tipo: String,
    txn: String,
    as_user: Option<String>,
    body: Value,
}

#[derive(Clone, Default)]
struct FakeSynapse {
    writes: Arc<Mutex<Vec<Write>>>,
    calls: Arc<AtomicUsize>,
    fail: Arc<Mutex<usize>>,
}

async fn fake_send(
    State(fake): State<FakeSynapse>,
    Path((room, tipo, txn)): Path<(String, String, String)>,
    Query(query): Query<HashMap<String, String>>,
    _headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    fake.calls.fetch_add(1, Ordering::SeqCst);
    let mut fail = fake.fail.lock().unwrap();
    if *fail > 0 {
        *fail -= 1;
        drop(fail);
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "errcode": "M_UNKNOWN" })),
        );
    }
    let mut writes = fake.writes.lock().unwrap();
    writes.push(Write {
        room,
        tipo,
        txn,
        as_user: query.get("user_id").cloned(),
        body,
    });
    let position = writes.len();
    (
        StatusCode::OK,
        Json(json!({ "event_id": format!("$aviso-{position}:matrix-dev.local") })),
    )
}

async fn start_fake_synapse() -> (String, FakeSynapse) {
    let fake = FakeSynapse::default();
    let router = Router::new()
        .route(
            "/_matrix/client/v3/rooms/{room}/send/{kind}/{txn}",
            put(fake_send),
        )
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera do Synapse falso");
    let endereco = listener.local_addr().expect("endereço do Synapse falso");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{endereco}"), fake)
}

#[derive(Clone)]
struct FakeEvolution {
    script: Arc<Mutex<VecDeque<(u16, Value)>>>,
}

async fn send_via_whatsapp(
    State(fake_evolution): State<FakeEvolution>,
    Path(_instance): Path<String>,
    Json(_body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let (status, response) = fake_evolution
        .script
        .lock()
        .unwrap()
        .pop_front()
        .unwrap_or((200, json!({ "key": { "id": "MSG-MX19" } })));
    (
        StatusCode::from_u16(status).expect("status do roteiro"),
        Json(response),
    )
}

async fn start_fake_evolution() -> (String, FakeEvolution) {
    let fake_evolution = FakeEvolution {
        script: Arc::new(Mutex::new(VecDeque::new())),
    };
    let router = Router::new()
        .route("/message/sendText/{instance}", post(send_via_whatsapp))
        .with_state(fake_evolution.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera da Evolution falsa");
    let endereco = listener.local_addr().expect("endereço da Evolution falsa");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{endereco}"), fake_evolution)
}

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agent: AgentId,
    integracao: IntegrationId,
    conversation: ConversationId,
    synapse: FakeSynapse,
    url_synapse: String,
    evo: FakeEvolution,
    saida: WhatsAppOutput,
}

impl Scenario {
    async fn new(label: &str) -> Self {
        Self::new_with_attempts(label, WhatsAppOutputOptions::default().max_attempts).await
    }

    async fn new_with_attempts(label: &str, max_attempts: u32) -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let folder = std::env::temp_dir().join(format!(
            "poph-matrix-falhas-{label}-{}-{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("pasta do teste");
        let store = Arc::new(
            Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
                .expect("abre o banco"),
        );

        let org = create_org(&store, "loja-falhas", "Loja Falhas");
        let agent = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Atendente Falhas".to_owned(),
                    slug: "atendente-falhas".to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        let agent_id: AgentId = agent;
        let integracao: IntegrationId = store
            .create_integration(org, agent_id, "whatsapp".to_owned(), "active".to_owned())
            .expect("cria a integração")
            .id
            .parse()
            .expect("id da integração");
        store.matrix().enable(org).expect("liga o módulo");
        let conversation = whatsapp_conversation(&store, org, agent_id, integracao, PHONE);
        store
            .matrix()
            .ensure_room(org, conversation, ROOM, None)
            .expect("grava a sala");
        store
            .matrix()
            .mark_room_ready(org, conversation)
            .expect("sala pronta");

        let (url_synapse, synapse) = start_fake_synapse().await;
        let (url_evo, evo) = start_fake_evolution().await;
        let client = EvolutionClient::new(
            ConfigEvolution::new(url_evo, INSTANCIA, Secret::new(EVO_KEY.to_owned()))
                .expect("config da Evolution falsa"),
        )
        .expect("cliente da Evolution falsa");
        let channels = vec![EvolutionChannel {
            org,
            instance: INSTANCIA.to_owned(),
            integration: integracao,
            secret: None,
            client: Some(client),
            attachment_limit: ATTACHMENT_CAP,
        }];
        let options = WhatsAppOutputOptions {
            backoff: Backoff {
                base: Duration::from_millis(1),
                factor: 2,
                cap: Duration::from_millis(5),
            },
            max_attempts,
            ..WhatsAppOutputOptions::default()
        };
        let saida =
            WhatsAppOutput::new(Arc::clone(&store), &channels, folder.join("blobs"), options);
        captured_logs();
        Self {
            folder,
            store,
            org,
            agent: agent_id,
            integracao,
            conversation,
            synapse,
            url_synapse,
            evo,
            saida,
        }
    }

    fn other_conversation(&self, with_room: bool) -> ConversationId {
        let conversation = whatsapp_conversation(
            &self.store,
            self.org,
            self.agent,
            self.integracao,
            "5511888880000",
        );
        if with_room {
            self.store
                .matrix()
                .ensure_room(self.org, conversation, "!outra-mx19:matrix-dev.local", None)
                .expect("grava a sala");
            self.store
                .matrix()
                .mark_room_ready(self.org, conversation)
                .expect("sala pronta");
        }
        conversation
    }

    fn human_response(&self, conversation: ConversationId) {
        let fresh = NewHumanResponse {
            event_id: &format!("{EVENTO}-{conversation}"),
            raw_text: ATTENDANT_TEXT,
            sent_text: &format!("**[{ATTENDANT_NAME}]**:\n{ATTENDANT_TEXT}"),
            author_id: ATTENDANT,
            author_name: ATTENDANT_NAME,
        };
        match self
            .store
            .messages()
            .register_response_human(self.org, conversation, fresh)
            .expect("grava a resposta humana")
        {
            HumanResponse::Stored(_) => {}
            other => panic!("a resposta humana devia ser gravada, veio {other:?}"),
        }
    }

    fn agent_output(&self, conversation: ConversationId) -> OutboxItem {
        let (_, item) = self
            .store
            .outbox()
            .enqueue(
                self.org,
                NewOutboxItem {
                    conversation_id: conversation,
                    turn_id: TurnId::generate(),
                    seq: 0,
                    tipo: "text".to_owned(),
                    content_json: json!({ "texto": "resposta do agente" }).to_string(),
                },
            )
            .expect("enfileira a saída do agente");
        item
    }

    async fn cycle(&self, script: Vec<(u16, Value)>) {
        *self.evo.script.lock().unwrap() = script.into_iter().collect();
        self.saida
            .cycle()
            .await
            .expect("ciclo da saída do WhatsApp");
    }

    fn notice(&self, store: Arc<Store>) -> FailureNotice {
        let client = MatrixClient::new(
            ClientConfig::new(
                self.url_synapse.clone(),
                SERVER_NAME.to_owned(),
                Secret::new(AS_TOKEN.to_owned()),
            )
            .expect("config do cliente do Matrix"),
        )
        .expect("cliente do Matrix");
        let ids = GhostIds::new(PREFIX, SERVER_NAME).expect("ids dos fantasmas");
        FailureNotice::new(
            Arc::new(HarnessFailuresPort::fresh(store)),
            SynapseNotice::new(client, ids),
        )
    }

    fn writes(&self) -> Vec<Write> {
        self.synapse.writes.lock().unwrap().clone()
    }

    fn state(&self, state: OutboxState) -> u64 {
        self.store
            .outbox()
            .count_by_state(self.org, state)
            .expect("conta a fila")
    }

    fn agent_ghost(&self) -> String {
        GhostIds::new(PREFIX, SERVER_NAME)
            .expect("ids")
            .agent(&self.agent.to_string())
            .expect("fantasma do agente")
            .user_id()
            .to_owned()
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.folder);
    }
}

fn create_org(store: &Store, slug: &str, name: &str) -> OrgId {
    store
        .orgs()
        .create(NewOrg {
            name: name.to_owned(),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id
}

fn whatsapp_conversation(
    store: &Store,
    org: OrgId,
    agent: AgentId,
    integracao: IntegrationId,
    phone: &str,
) -> ConversationId {
    store
        .conversations()
        .create(
            org,
            NewConversation {
                agent_id: agent,
                integration_id: Some(integracao),
                canal: "whatsapp".to_owned(),
                peer_externo: Some(phone.to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa")
        .id
}

fn notice_text(written: &Write) -> String {
    written
        .body
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn final_human_response_failure_notices_room_once() {
    let scenario = Scenario::new("aviso-uma-vez").await;
    scenario.human_response(scenario.conversation);

    scenario
        .cycle(vec![(400, json!({ "message": "número inválido" }))])
        .await;
    assert_eq!(
        scenario.state(OutboxState::Failed),
        1,
        "a fila marcou a falha"
    );

    let report = scenario.notice(Arc::clone(&scenario.store)).pass().await;
    assert_eq!(report.notified, 1, "um aviso escrito");

    let writes = scenario.writes();
    assert_eq!(writes.len(), 1, "uma escrita na sala");
    let notice = &writes[0];
    assert_eq!(notice.room, ROOM, "o aviso vai para a sala da conversa");
    assert_eq!(notice.tipo, "m.room.message");
    assert_eq!(
        notice.body["msgtype"], "m.notice",
        "aviso é m.notice, para não voltar como resposta"
    );
    assert_eq!(notice_text(notice), NOTICE_REFUSED);
    assert_eq!(
        notice.as_user.as_deref(),
        Some(scenario.agent_ghost().as_str()),
        "escrito em nome do fantasma do agente, que está na sala"
    );
    assert!(notice.txn.starts_with("ph_falha_"));

    let calls_before = scenario.synapse.calls.load(Ordering::SeqCst);
    let second = scenario.notice(Arc::clone(&scenario.store)).pass().await;
    assert_eq!(
        second,
        Default::default(),
        "a segunda passagem não faz nada"
    );
    assert_eq!(scenario.synapse.calls.load(Ordering::SeqCst), calls_before);
    assert_eq!(scenario.writes().len(), 1, "continua um aviso só");
}

#[tokio::test]
async fn process_restart_does_not_repeat_notice() {
    let scenario = Scenario::new("reinicio").await;
    scenario.human_response(scenario.conversation);
    scenario
        .cycle(vec![(400, json!({ "message": "recusado" }))])
        .await;

    {
        let before = scenario.notice(Arc::clone(&scenario.store)).pass().await;
        assert_eq!(before.notified, 1);
    }
    assert_eq!(scenario.writes().len(), 1);

    let reaberto = Arc::new(
        Store::open_at(
            &scenario.folder.join("harness.db"),
            &scenario.folder.join("rag.db"),
        )
        .expect("reabre o banco"),
    );
    let after = scenario.notice(reaberto).pass().await;
    assert_eq!(after, Default::default(), "a marca do banco segura o aviso");
    assert_eq!(
        scenario.writes().len(),
        1,
        "nenhum segundo aviso depois do reinício"
    );
}

#[tokio::test]
async fn background_loop_notices_alone_and_stops_on_request() {
    let scenario = Scenario::new("laco").await;
    scenario.human_response(scenario.conversation);
    scenario
        .cycle(vec![(400, json!({ "message": "recusado" }))])
        .await;

    let notice = Arc::new(scenario.notice(Arc::clone(&scenario.store)));
    let task = tokio::spawn(Arc::clone(&notice).run(Duration::from_millis(10)));

    let started_at = std::time::Instant::now();
    while scenario.writes().is_empty() {
        assert!(
            started_at.elapsed() < Duration::from_secs(10),
            "o laço não avisou"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(scenario.writes().len(), 1);

    notice.ask_stop();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("o laço para no pedido")
        .expect("tarefa do laço sem pânico");
    assert_eq!(scenario.writes().len(), 1, "continua um aviso só");
}

#[tokio::test]
async fn transient_failure_followed_by_success_does_not_notice() {
    let scenario = Scenario::new("transitoria").await;
    scenario.human_response(scenario.conversation);

    scenario
        .cycle(vec![(500, json!({ "message": "erro interno" }))])
        .await;
    assert_eq!(
        scenario.state(OutboxState::Sending),
        1,
        "a falha ainda não é definitiva"
    );
    assert_eq!(scenario.state(OutboxState::Failed), 0);

    let in_between = scenario.notice(Arc::clone(&scenario.store)).pass().await;
    assert_eq!(
        in_between,
        Default::default(),
        "falha transitória não avisa"
    );
    assert!(scenario.writes().is_empty());

    tokio::time::sleep(Duration::from_millis(20)).await;
    scenario
        .cycle(vec![(200, json!({ "key": { "id": "MSG-OK" } }))])
        .await;
    assert_eq!(scenario.state(OutboxState::Sent), 1, "o cliente recebeu");

    let after = scenario.notice(Arc::clone(&scenario.store)).pass().await;
    assert_eq!(after, Default::default());
    assert!(
        scenario.writes().is_empty(),
        "nenhum aviso quando a entrega deu certo"
    );
}

#[tokio::test]
async fn exhausted_attempts_notice_with_own_reason() {
    let scenario = Scenario::new_with_attempts("esgotado", 1).await;
    scenario.human_response(scenario.conversation);
    scenario
        .cycle(vec![(500, json!({ "message": "erro interno" }))])
        .await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    scenario.cycle(vec![]).await;
    assert_eq!(
        scenario.state(OutboxState::Failed),
        1,
        "esgotou as tentativas"
    );

    let report = scenario.notice(Arc::clone(&scenario.store)).pass().await;
    assert_eq!(report.notified, 1);
    let writes = scenario.writes();
    assert_eq!(notice_text(&writes[0]), NOTICE_EXHAUSTED);
}

#[tokio::test]
async fn offline_synapse_defers_and_next_pass_notices() {
    let scenario = Scenario::new("synapse-fora").await;
    scenario.human_response(scenario.conversation);
    scenario
        .cycle(vec![(400, json!({ "message": "recusado" }))])
        .await;

    *scenario.synapse.fail.lock().unwrap() = 1;
    let first = scenario.notice(Arc::clone(&scenario.store)).pass().await;
    assert_eq!(
        first.deferred, 1,
        "erro transitório: fica para a próxima passagem"
    );
    assert_eq!(first.notified, 0);
    assert!(scenario.writes().is_empty());

    let second_pass = scenario.notice(Arc::clone(&scenario.store)).pass().await;
    assert_eq!(second_pass.notified, 1);
    assert_eq!(scenario.writes().len(), 1);
}

#[tokio::test]
async fn conversation_without_room_is_marked_and_tries_nothing() {
    let scenario = Scenario::new("sem-sala").await;
    let conversation = scenario.other_conversation(false);
    scenario.human_response(conversation);
    scenario
        .cycle(vec![(400, json!({ "message": "recusado" }))])
        .await;

    let report = scenario.notice(Arc::clone(&scenario.store)).pass().await;
    assert_eq!(report.without_room, 1);
    assert_eq!(
        scenario.synapse.calls.load(Ordering::SeqCst),
        0,
        "nada chegou ao Synapse"
    );

    let second = scenario.notice(Arc::clone(&scenario.store)).pass().await;
    assert_eq!(
        second,
        Default::default(),
        "a marca impede de consultar de novo"
    );
}

#[tokio::test]
async fn output_that_is_not_human_response_does_not_notice() {
    let scenario = Scenario::new("nao-humana").await;
    let _ = scenario.agent_output(scenario.conversation);
    scenario
        .cycle(vec![(400, json!({ "message": "recusado" }))])
        .await;
    assert_eq!(scenario.state(OutboxState::Failed), 1);

    let report = scenario.notice(Arc::clone(&scenario.store)).pass().await;
    assert_eq!(report, Default::default(), "só resposta humana é avisada");
    assert!(scenario.writes().is_empty());
}

#[tokio::test]
async fn org_with_module_off_receives_no_notice() {
    let scenario = Scenario::new("desligado").await;
    scenario.human_response(scenario.conversation);
    scenario
        .cycle(vec![(400, json!({ "message": "recusado" }))])
        .await;

    scenario
        .store
        .matrix()
        .disable(scenario.org)
        .expect("desliga o módulo");
    let report = scenario.notice(Arc::clone(&scenario.store)).pass().await;
    assert_eq!(
        report,
        Default::default(),
        "módulo desligado: nenhuma passagem"
    );
    assert!(scenario.writes().is_empty());
}

#[tokio::test]
async fn customer_text_phone_and_token_appear_in_neither_notice_nor_log() {
    let scenario = Scenario::new("canario").await;
    scenario.human_response(scenario.conversation);
    scenario
        .cycle(vec![(
            400,
            json!({ "message": "recusado para 5511999990000" }),
        )])
        .await;

    let logs = captured_logs();
    logs.lock().unwrap().clear();
    let _ = scenario.notice(Arc::clone(&scenario.store)).pass().await;

    let writes = scenario.writes();
    assert_eq!(writes.len(), 1);
    let notice = notice_text(&writes[0]);
    assert!(!notice.contains(PHONE), "telefone não vai à sala");
    assert!(
        !notice.contains(ATTENDANT_TEXT),
        "texto do atendente não vai à sala"
    );

    let saida = String::from_utf8_lossy(&logs.lock().unwrap()).into_owned();
    assert!(
        saida.contains("aviso de falha de resposta humana escrito na sala"),
        "o log existe"
    );
    for canario in [PHONE, ATTENDANT_TEXT, AS_TOKEN, "recusado para"] {
        assert!(!saida.contains(canario), "log não pode conter `{canario}`");
    }
}

#[test]
fn notice_outcome_is_idempotent_and_isolated_per_org() {
    let folder = std::env::temp_dir().join(format!(
        "poph-matrix-falhas-desfecho-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).expect("pasta");
    let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db")).expect("banco");
    let org = create_org(&store, "loja-desfecho", "Loja Desfecho");
    let agent = store
        .agents()
        .create(
            org,
            NewAgent {
                name: "A".to_owned(),
                slug: "a".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("agente")
        .id;
    let integracao: IntegrationId = store
        .create_integration(org, agent, "whatsapp".to_owned(), "active".to_owned())
        .expect("integração")
        .id
        .parse()
        .expect("id da integração");
    let conversation = whatsapp_conversation(&store, org, agent, integracao, PHONE);
    let (_, item) = store
        .outbox()
        .enqueue(
            org,
            NewOutboxItem {
                conversation_id: conversation,
                turn_id: TurnId::generate(),
                seq: 0,
                tipo: "text".to_owned(),
                content_json: "{}".to_owned(),
            },
        )
        .expect("enfileira");

    let first = store
        .matrix()
        .record_notice_outcome(org, item.id, OutcomeNotice::Notify, "recusado_pelo_canal")
        .expect("grava");
    let second_pass = store
        .matrix()
        .record_notice_outcome(org, item.id, OutcomeNotice::Notify, "recusado_pelo_canal")
        .expect("grava de novo");
    assert!(first, "a primeira gravação é nova");
    assert!(!second_pass, "a segunda não muda nada");

    let other_org = create_org(&store, "loja-desfecho-b", "Loja B");
    let from_other = store
        .matrix()
        .record_notice_outcome(other_org, item.id, OutcomeNotice::Refused, "nao_informado")
        .expect("grava");
    assert!(!from_other, "item de outra org não ganha desfecho");
    let other_org_failures = store
        .matrix()
        .human_response_failures(other_org, 10)
        .expect("lista");
    assert!(
        other_org_failures.is_empty(),
        "a outra org não enxerga a falha"
    );
    let _ = fs::remove_dir_all(&folder);
}
