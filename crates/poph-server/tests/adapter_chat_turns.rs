use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::turno::{
    DefaultRegistry, InternalTool, ToolContext, ToolFuture, TurnEngine, TurnOptions,
};
use poph_core::llm::{
    FakeProvider, FinishReason, LlmError, LlmEvent, LlmProvider, Script, ToolCall,
};
use poph_flows::ProducedEffect;
use poph_flows::ir::Effect;
use poph_server::ServerState;
use poph_server::flow_effects::{FlowEffectOptions, FlowEffects, effect_of_transition};
use poph_store::{
    AdapterTurnKey, AdapterTurnSubmission, Direction, ExecutionStart, ExecutionState,
    NewAdapterTurn, NewAgent, NewMessage, NewOrg, Origin, OutboxState, Role, Store,
    TransitionEffect,
};
use poph_types::{
    AgentId, ConversationId, EffectState, EventActor, ExecutionStatus, FlowId, HarnessConfig,
    OrgId, RevisionId, StructuredInstructions, ToolConfig,
};
use rand_core::OsRng;
use reqwest::{Client, Method, Response, StatusCode};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const TEAM_PHONE: &str = "5521988880002";

struct Account {
    org: OrgId,
    agent: AgentId,
    token: String,
}

struct Fixture {
    root: PathBuf,
    config: HarnessConfig,
    store: Arc<Store>,
    account: Account,
    tool: Arc<RecordingTool>,
}

struct Server {
    url: String,
    client: Client,
    task: JoinHandle<std::io::Result<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[derive(Default)]
struct RecordingTool {
    calls: AtomicUsize,
}

impl RecordingTool {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl InternalTool for RecordingTool {
    fn nome(&self) -> &'static str {
        "consultar_base"
    }

    fn descricao(&self) -> &'static str {
        "Consulta a base de conhecimento."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {"consulta": {"type": "string"}}})
    }

    fn run(&self, _context: ToolContext, _arguments: Value) -> ToolFuture {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok("O prazo é de cinco dias úteis.".to_owned()) })
    }
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("poph-adapter-turns-{}", OrgId::generate()));
        let mut config = HarnessConfig::default();
        config.data.directory = root.clone();
        config.adapter_chat.max_text_bytes = 256;
        let store = Arc::new(Store::open(&config).expect("open"));
        let account = add_account(&store, "primeira");
        let instructions = StructuredInstructions {
            tools: vec![ToolConfig::new("consultar_base")],
            ..Default::default()
        };
        instructions.validar().expect("closed tool list");
        store
            .instructions()
            .save(
                account.org,
                account.agent,
                0,
                &instructions.to_json(),
                "Você atende pelo adaptador.",
                true,
            )
            .expect("instructions");
        Self {
            root,
            config,
            store,
            account,
            tool: Arc::new(RecordingTool::default()),
        }
    }

    async fn start(&self, provider: Arc<dyn LlmProvider>) -> Server {
        let memory = MetricsMemory::own();
        let engine = TurnEngine::new(
            self.store.clone(),
            ConfigHot::new(
                self.store.clone(),
                HotConfigOptions::default(),
                memory.clone(),
            ),
            Arc::new(Limiter::new(LimitOptions::default(), memory)),
            provider,
            TurnOptions::default(),
            MetricsTurn::own(),
        );
        let registry = DefaultRegistry::new().com(Arc::clone(&self.tool) as Arc<dyn InternalTool>);
        let engine = engine.with_tools(Arc::new(registry));
        let state = ServerState::new(self.store.clone(), self.config.clone())
            .expect("state")
            .with_engine(Arc::new(engine));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let task = tokio::spawn(poph_server::serve_at(
            listener,
            state,
            std::future::pending(),
        ));
        Server {
            url,
            client: Client::new(),
            task,
        }
    }
}

fn add_agent(store: &Store, org: OrgId, slug: &str) -> AgentId {
    store
        .agents()
        .create(
            org,
            NewAgent {
                name: "Atendente de teste".to_owned(),
                slug: slug.to_owned(),
                ..Default::default()
            },
        )
        .expect("agent")
        .id
}

fn add_account(store: &Store, slug: &str) -> Account {
    let org = store
        .orgs()
        .create(NewOrg {
            name: "Organização de teste".to_owned(),
            slug: slug.to_owned(),
            config_json: "{}".to_owned(),
        })
        .expect("organization")
        .id;
    let agent = add_agent(store, org, slug);
    let secret = "credencial-ficticia-turnos";
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(secret.as_bytes(), &salt)
        .expect("hash")
        .to_string();
    let token = store
        .tokens()
        .create(Some(org), slug, &hash)
        .expect("token")
        .id;
    Account {
        org,
        agent,
        token: format!("poph_{token}_{secret}"),
    }
}

impl Server {
    async fn request(
        &self,
        account: &Account,
        method: Method,
        path: &str,
        data: Option<Value>,
    ) -> Response {
        let builder = self
            .client
            .request(
                method,
                format!("{}/admin/v1/orgs/{}{}", self.url, account.org, path),
            )
            .bearer_auth(&account.token);
        let request = match data {
            Some(body) => builder.json(&body),
            None => builder,
        };
        request.send().await.expect("request")
    }

    async fn open(
        &self,
        account: &Account,
        agent: AgentId,
        reference: &str,
        status: StatusCode,
    ) -> Value {
        let response = self
            .request(
                account,
                Method::POST,
                &format!("/agents/{agent}/adapter-conversations"),
                Some(json!({"channel": "email", "external_ref": reference})),
            )
            .await;
        read(response, status).await
    }

    async fn send_turn(
        &self,
        account: &Account,
        conversation: &str,
        key: &str,
        text: &str,
        status: StatusCode,
    ) -> Value {
        let response = self
            .request(
                account,
                Method::POST,
                &format!("/adapter-conversations/{conversation}/turns"),
                Some(json!({"text": text, "idempotency_key": key})),
            )
            .await;
        read(response, status).await
    }

    async fn turn(
        &self,
        account: &Account,
        conversation: &str,
        turn: &str,
        status: StatusCode,
    ) -> Value {
        let response = self
            .request(
                account,
                Method::GET,
                &format!("/adapter-conversations/{conversation}/turns/{turn}"),
                None,
            )
            .await;
        read(response, status).await
    }

    async fn cancel(
        &self,
        account: &Account,
        conversation: &str,
        turn: &str,
        status: StatusCode,
    ) -> Value {
        let response = self
            .request(
                account,
                Method::POST,
                &format!("/adapter-conversations/{conversation}/turns/{turn}/cancel"),
                None,
            )
            .await;
        read(response, status).await
    }

    async fn history(&self, account: &Account, conversation: &str) -> Value {
        let response = self
            .request(
                account,
                Method::GET,
                &format!("/adapter-conversations/{conversation}/messages"),
                None,
            )
            .await;
        read(response, StatusCode::OK).await
    }
}

async fn read(response: Response, status: StatusCode) -> Value {
    let actual = response.status();
    let data: Value = response.json().await.expect("response JSON");
    assert_eq!(actual, status, "{data}");
    data
}

fn answer(text: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(text.to_owned()),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

fn tool_call(id: &str, name: &str, arguments: Value) -> Script {
    Script::Events(vec![LlmEvent::Tool(ToolCall {
        id: id.to_owned(),
        nome: name.to_owned(),
        arguments,
    })])
}

async fn wait_terminal(
    server: &Server,
    account: &Account,
    conversation: &str,
    turn: &str,
) -> Value {
    for _ in 0..600 {
        let data = server
            .turn(account, conversation, turn, StatusCode::OK)
            .await;
        if matches!(
            data["status"].as_str(),
            Some("succeeded" | "failed" | "cancelled")
        ) {
            return data;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("turn {turn} did not reach a terminal state");
}

fn texts(history: &Value) -> Vec<String> {
    history["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .map(|item| item["text"].as_str().expect("text").to_owned())
        .collect()
}

fn assert_outbox_empty(fixture: &Fixture) {
    for state in [
        OutboxState::Pending,
        OutboxState::Sending,
        OutboxState::Sent,
        OutboxState::Failed,
    ] {
        assert_eq!(
            fixture
                .store
                .outbox()
                .count_by_state(fixture.account.org, state)
                .expect("outbox"),
            0,
            "adapter conversation must not write to the outbox"
        );
    }
}

#[tokio::test]
async fn turns_exchange_replies_and_store_history_without_outbox() {
    let fixture = Fixture::new();
    let other = add_account(&fixture.store, "segunda");
    let fake = Arc::new(FakeProvider::with_scripts(vec![
        answer("Primeira resposta."),
        answer("Segunda resposta."),
    ]));
    let server = fixture.start(fake.clone()).await;
    let account = &fixture.account;
    let conversation = server
        .open(account, account.agent, "thread-1", StatusCode::CREATED)
        .await;
    let conversation_id = conversation["id"]
        .as_str()
        .expect("conversation")
        .to_owned();

    let first = server
        .send_turn(account, &conversation_id, "k1", "Olá", StatusCode::ACCEPTED)
        .await;
    assert_eq!(first["status"], "queued");
    let first_turn = first["turn_id"].as_str().expect("turn").to_owned();
    let first_message = first["message_id"].as_str().expect("message").to_owned();
    let first = wait_terminal(&server, account, &conversation_id, &first_turn).await;
    assert_eq!(first["status"], "succeeded", "{first}");
    assert_eq!(first["reply_text"], "Primeira resposta.");
    assert_eq!(first["error_code"], Value::Null);
    assert!(first["input_tokens"].is_number());

    let history = server.history(account, &conversation_id).await;
    assert_eq!(
        texts(&history),
        ["Primeira resposta.", "Olá"],
        "the reply is stored as an agent message"
    );
    let stored = history["messages"].as_array().expect("messages");
    assert_eq!(stored[0]["role"], "assistant");
    assert_eq!(stored[0]["origin"], "agent");
    assert_eq!(stored[1]["role"], "user");
    assert_eq!(stored[1]["id"], first_message);

    let second = server
        .send_turn(
            account,
            &conversation_id,
            "k2",
            "Mais uma pergunta",
            StatusCode::ACCEPTED,
        )
        .await;
    let second_turn = second["turn_id"].as_str().expect("turn").to_owned();
    let second = wait_terminal(&server, account, &conversation_id, &second_turn).await;
    assert_eq!(second["reply_text"], "Segunda resposta.");
    assert_eq!(
        texts(&server.history(account, &conversation_id).await),
        [
            "Segunda resposta.",
            "Mais uma pergunta",
            "Primeira resposta.",
            "Olá"
        ]
    );
    let replay = server
        .send_turn(account, &conversation_id, "k1", "Olá", StatusCode::OK)
        .await;
    assert_eq!(replay["turn_id"], first_turn);
    assert_eq!(replay["status"], "succeeded");
    assert_eq!(
        fake.received().len(),
        2,
        "replay does not call the model again"
    );
    assert_outbox_empty(&fixture);

    let crossed = server
        .client
        .get(format!(
            "{}/admin/v1/orgs/{}/adapter-conversations/{conversation_id}/turns/{first_turn}",
            server.url, account.org
        ))
        .bearer_auth(&other.token)
        .send()
        .await
        .expect("crossed");
    assert_eq!(crossed.status(), StatusCode::FORBIDDEN);
    let foreign = server
        .turn(&other, &conversation_id, &first_turn, StatusCode::NOT_FOUND)
        .await;
    assert_eq!(foreign["code"], "not_found");
    let denied = server
        .cancel(&other, &conversation_id, &first_turn, StatusCode::NOT_FOUND)
        .await;
    assert_eq!(denied["code"], "not_found");
    let still = server
        .turn(account, &conversation_id, &first_turn, StatusCode::OK)
        .await;
    assert_eq!(still["status"], "succeeded");
}

#[tokio::test]
async fn knowledge_tool_runs_and_is_recorded_in_the_reply_path() {
    let fixture = Fixture::new();
    let fake = Arc::new(FakeProvider::with_scripts(vec![
        tool_call("call-1", "consultar_base", json!({"consulta": "prazo"})),
        answer("O prazo é de cinco dias úteis, conforme a base."),
    ]));
    let server = fixture.start(fake.clone()).await;
    let account = &fixture.account;
    let conversation = server
        .open(account, account.agent, "thread-tool", StatusCode::CREATED)
        .await;
    let conversation_id = conversation["id"]
        .as_str()
        .expect("conversation")
        .to_owned();
    let sent = server
        .send_turn(
            account,
            &conversation_id,
            "tool-1",
            "Qual o prazo?",
            StatusCode::ACCEPTED,
        )
        .await;
    let turn = sent["turn_id"].as_str().expect("turn").to_owned();
    let turn = wait_terminal(&server, account, &conversation_id, &turn).await;
    assert_eq!(turn["status"], "succeeded", "{turn}");
    assert_eq!(
        turn["reply_text"],
        "O prazo é de cinco dias úteis, conforme a base."
    );
    assert_eq!(fixture.tool.calls(), 1, "the tool ran once");
    assert_eq!(fake.received().len(), 2, "tool round plus final reply");
    assert_outbox_empty(&fixture);
}

#[tokio::test]
async fn replay_is_idempotent_and_a_second_turn_gets_409_until_cancelled() {
    let fixture = Fixture::new();
    let fake = Arc::new(FakeProvider::with_scripts(vec![Script::Pending {
        events: Vec::new(),
        after: Duration::from_secs(60),
        error: LlmError::Cancelled,
    }]));
    let server = fixture.start(fake).await;
    let account = &fixture.account;
    let conversation = server
        .open(account, account.agent, "thread-busy", StatusCode::CREATED)
        .await;
    let conversation_id = conversation["id"]
        .as_str()
        .expect("conversation")
        .to_owned();

    let first = server
        .send_turn(
            account,
            &conversation_id,
            "k1",
            "Demorado",
            StatusCode::ACCEPTED,
        )
        .await;
    let turn = first["turn_id"].as_str().expect("turn").to_owned();
    let message = first["message_id"].as_str().expect("message").to_owned();
    let replay = server
        .send_turn(account, &conversation_id, "k1", "Demorado", StatusCode::OK)
        .await;
    assert_eq!(replay["turn_id"], turn);
    assert_eq!(replay["message_id"], message);
    let conflict = server
        .send_turn(
            account,
            &conversation_id,
            "k1",
            "Outro texto",
            StatusCode::CONFLICT,
        )
        .await;
    assert_eq!(conflict["code"], "idempotency_conflict");
    let busy = server
        .send_turn(
            account,
            &conversation_id,
            "k2",
            "Segunda",
            StatusCode::CONFLICT,
        )
        .await;
    assert_eq!(busy["code"], "turn_in_progress");
    assert_eq!(busy["details"]["turn_id"], turn);
    let too_long = "a".repeat(257);
    let refused = server
        .send_turn(
            account,
            &conversation_id,
            "k3",
            &too_long,
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    assert_eq!(refused["details"]["field"], "text");

    let cancelled = server
        .cancel(account, &conversation_id, &turn, StatusCode::OK)
        .await;
    assert_eq!(cancelled["status"], "cancelled");
    let again = server
        .cancel(account, &conversation_id, &turn, StatusCode::OK)
        .await;
    assert_eq!(again["status"], "cancelled");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let turn_state = server
        .turn(account, &conversation_id, &turn, StatusCode::OK)
        .await;
    assert_eq!(turn_state["status"], "cancelled");
    assert_eq!(
        fixture
            .store
            .messages()
            .count_rows(account.org, conversation_id.parse().expect("id"))
            .expect("messages"),
        1,
        "the cancelled turn stores no reply"
    );
    assert_outbox_empty(&fixture);
}

#[tokio::test]
async fn restart_marks_running_turns_failed_and_processes_queued_turns_once() {
    let mut fixture = Fixture::new();
    let account_org = fixture.account.org;
    let account_agent = fixture.account.agent;
    let running = fixture
        .store
        .adapter_chat()
        .create_or_resume(
            account_org,
            account_agent,
            "email".to_owned(),
            "thread-running".to_owned(),
            true,
        )
        .expect("running conversation")
        .0;
    let queued = fixture
        .store
        .adapter_chat()
        .create_or_resume(
            account_org,
            account_agent,
            "email".to_owned(),
            "thread-queued".to_owned(),
            true,
        )
        .expect("queued conversation")
        .0;
    let submit = |conversation: ConversationId, key: &str| {
        let AdapterTurnSubmission::Accepted { turn, .. } = fixture
            .store
            .adapter_chat()
            .submit_turn(
                account_org,
                conversation,
                NewAdapterTurn {
                    idempotency_key: key.to_owned(),
                    text: "Mensagem antes da interrupção".to_owned(),
                    author_name: None,
                    external_message_id: None,
                },
            )
            .expect("accepted turn")
        else {
            panic!("turn was not accepted");
        };
        AdapterTurnKey {
            org: account_org,
            conversation,
            turn,
        }
    };
    let running_key = submit(running, "r1");
    let queued_key = submit(queued, "q1");
    assert!(
        fixture
            .store
            .adapter_chat()
            .claim_turn(running_key)
            .expect("claim")
            .is_some()
    );
    fixture.store = Arc::new(Store::open(&fixture.config).expect("reopen database"));
    let fake = Arc::new(FakeProvider::with_scripts(vec![answer(
        "Resposta recuperada.",
    )]));
    let server = fixture.start(fake.clone()).await;
    let account = &fixture.account;

    let interrupted = server
        .turn(
            account,
            &running.to_string(),
            &running_key.turn.to_string(),
            StatusCode::OK,
        )
        .await;
    assert_eq!(interrupted["status"], "failed");
    assert_eq!(interrupted["error_code"], "server_restarted");
    assert!(interrupted["finished_at"].is_number());
    let replay = server
        .send_turn(
            account,
            &running.to_string(),
            "r1",
            "Mensagem antes da interrupção",
            StatusCode::OK,
        )
        .await;
    assert_eq!(replay["turn_id"], running_key.turn.to_string());

    let recovered = wait_terminal(
        &server,
        account,
        &queued.to_string(),
        &queued_key.turn.to_string(),
    )
    .await;
    assert_eq!(recovered["status"], "succeeded", "{recovered}");
    assert_eq!(recovered["reply_text"], "Resposta recuperada.");
    let history = server.history(account, &queued.to_string()).await;
    assert_eq!(
        texts(&history),
        ["Resposta recuperada.", "Mensagem antes da interrupção"]
    );
    assert_eq!(fake.received().len(), 1);
    assert_outbox_empty(&fixture);
}

#[tokio::test]
async fn restart_keeps_a_turn_whose_reply_is_already_stored_succeeded() {
    let mut fixture = Fixture::new();
    let org = fixture.account.org;
    let agent = fixture.account.agent;
    let conversation = fixture
        .store
        .adapter_chat()
        .create_or_resume(
            org,
            agent,
            "email".to_owned(),
            "thread-stored-reply".to_owned(),
            true,
        )
        .expect("conversation")
        .0;
    let AdapterTurnSubmission::Accepted { turn, .. } = fixture
        .store
        .adapter_chat()
        .submit_turn(
            org,
            conversation,
            NewAdapterTurn {
                idempotency_key: "k1".to_owned(),
                text: "Pergunta".to_owned(),
                author_name: None,
                external_message_id: None,
            },
        )
        .expect("accepted turn")
    else {
        panic!("turn was not accepted");
    };
    let key = AdapterTurnKey {
        org,
        conversation,
        turn,
    };
    assert!(
        fixture
            .store
            .adapter_chat()
            .claim_turn(key)
            .expect("claim")
            .is_some()
    );
    let engine_turn = poph_types::TurnId::generate();
    fixture
        .store
        .adapter_chat()
        .record_engine_turn(key, engine_turn)
        .expect("engine turn");
    fixture
        .store
        .messages()
        .write(
            org,
            NewMessage {
                conversation_id: conversation,
                direction: Direction::Outbound,
                role: Role::Assistant,
                texto: "Resposta antes da queda.".to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: Some(engine_turn),
                origin: Origin::Agente,
                author_name: None,
            },
        )
        .expect("stored reply");
    fixture.store = Arc::new(Store::open(&fixture.config).expect("reopen database"));
    let fake = Arc::new(FakeProvider::with_scripts(Vec::new()));
    let server = fixture.start(fake.clone()).await;
    let account = &fixture.account;
    let state = server
        .turn(
            account,
            &conversation.to_string(),
            &turn.to_string(),
            StatusCode::OK,
        )
        .await;
    assert_eq!(state["status"], "succeeded", "{state}");
    assert_eq!(state["reply_text"], "Resposta antes da queda.");
    assert!(state["finished_at"].is_number());
    assert_eq!(
        fake.received().len(),
        0,
        "the stored reply is not generated again"
    );
    let history = server.history(account, &conversation.to_string()).await;
    assert_eq!(texts(&history), ["Resposta antes da queda.", "Pergunta"]);
    assert_outbox_empty(&fixture);
}

fn to_customer(text: &str) -> TransitionEffect {
    effect_of_transition(&ProducedEffect {
        chave: "etapa:boas_vindas/0".to_owned(),
        no: "etapa:boas_vindas".to_owned(),
        index: 0,
        effect: Effect::MessageToCustomer {
            texto: text.to_owned(),
        },
    })
}

fn published_flow(store: &Store, org: OrgId) -> (FlowId, RevisionId) {
    let fluxo: FlowId = uuid::Uuid::now_v7().to_string().parse().expect("flow");
    let revision: RevisionId = uuid::Uuid::now_v7().to_string().parse().expect("revision");
    let conn = rusqlite::Connection::open(store.harness_path()).expect("open for write");
    conn.execute(
        "INSERT INTO fluxos (id, org_id, slug, nome, descricao, status, rascunho_fonte,
                             revisao_atual, version)
         VALUES (?1, ?2, 'boas_vindas', 'Boas-vindas', '', 'active', '', 1, 1)",
        rusqlite::params![fluxo.to_string(), org.to_string()],
    )
    .expect("flow");
    conn.execute(
        "INSERT INTO fluxo_revisoes (id, org_id, fluxo_id, numero, fonte, fonte_sha256,
                                    definicao_json, schema_versao, publicada_em, publicada_por,
                                    resolucoes_json)
         VALUES (?1, ?2, ?3, 1, 'fluxo()', 'sha', '{}', 1, 1, 'admin_token', '{}')",
        rusqlite::params![revision.to_string(), org.to_string(), fluxo.to_string()],
    )
    .expect("revision");
    (fluxo, revision)
}

#[tokio::test]
async fn notification_effect_on_adapter_conversation_stays_pending() {
    let fixture = Fixture::new();
    let account = &fixture.account;
    let conversation = fixture
        .store
        .adapter_chat()
        .create_or_resume(
            account.org,
            account.agent,
            "email".to_owned(),
            "thread-notify".to_owned(),
            true,
        )
        .expect("conversation")
        .0;
    let ficha = fixture
        .store
        .adapter_chat()
        .get(account.org, conversation)
        .expect("detail")
        .ficha_id
        .expect("adapter conversation keeps its Case");
    let (flow, revision) = published_flow(&fixture.store, account.org);
    fixture
        .store
        .flows()
        .release(account.org, account.agent, flow)
        .expect("release flow");
    let notify = effect_of_transition(&ProducedEffect {
        chave: "acao:boas_vindas.registrar/0".to_owned(),
        no: "acao:boas_vindas.registrar".to_owned(),
        index: 0,
        effect: Effect::NotifyWhatsApp {
            para: TEAM_PHONE.to_owned(),
            mensagem: "Cliente pediu visita.".to_owned(),
        },
    });
    let execution = fixture
        .store
        .executions()
        .start_execution(
            account.org,
            ExecutionStart {
                chave_idempotencia: None,
                case: ficha,
                flow,
                revision,
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
                inbound_effects: vec![to_customer("Mensagem do fluxo ao cliente."), notify],
            },
        )
        .expect("start execution")
        .execution
        .id;

    let effects = FlowEffects::new(
        Arc::clone(&fixture.store),
        &[],
        fixture.root.join("blobs"),
        None,
        FlowEffectOptions {
            missing_integration_delay: Duration::from_millis(10),
            ..FlowEffectOptions::default()
        },
    );
    let report = effects
        .cycle_for_org(account.org)
        .await
        .expect("effect cycle");
    assert_eq!(report.deferred, 2, "waiting is not a failure");
    assert_eq!(report.permanent_failures, 0);
    let queued = fixture
        .store
        .executions()
        .effects(account.org, execution)
        .expect("effects");
    assert_eq!(queued.len(), 2);
    for effect in &queued {
        assert_eq!(effect.state, EffectState::Pending, "{}", effect.tipo);
        assert_eq!(effect.attempts, 0, "waiting consumes no attempt");
        assert!(effect.ultimo_erro.is_none());
        assert!(effect.proxima_tentativa.is_some());
    }
    assert!(
        queued
            .iter()
            .any(|effect| effect.tipo == "notificar_whatsapp")
    );
    assert!(
        queued
            .iter()
            .any(|effect| effect.tipo == "mensagem_ao_cliente")
    );
    assert_outbox_empty(&fixture);
}
