use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::{Json, Router, routing::post};
use futures::StreamExt;
use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasMemoria, MetricasTurno};
use poph_agent::turno::{HttpToolRegistry, Motor, OpcoesTurno};
use poph_core::llm::{
    FakeProvider, FinishReason, LlmError, LlmEvent, LlmLimiter, LlmProvider, Message,
    OpenAiCompatibleClient, OpenAiCompatibleConfig, Part, Request, Script, ToolCall,
};
use poph_server::Estado;
use poph_store::{
    ConversationStatus, NewAgent, NewOrg, NewOutboxItem, NewRecord, SiteSubmission, SiteTurnKey,
    Store,
};
use poph_types::{
    AgentId, BaseMetrics, ConversationId, HarnessConfig, OrgId, Registry, Secret,
    StructuredInstructions, ToolConfig, TurnId,
};
use rand_core::OsRng;
use reqwest::{Client, Method, Response, StatusCode};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

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

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("poph-site-{}", OrgId::generate()));
        let mut config = HarnessConfig::default();
        config.data.directory = root.clone();
        config.site_chat.max_text_bytes = 256;
        let store = Arc::new(Store::open(&config).expect("open"));
        let account = add_account(&store, "primeira");
        Self {
            root,
            config,
            store,
            account,
        }
    }

    async fn start(&self, provider: Arc<dyn LlmProvider>) -> Server {
        let memory = MetricasMemoria::proprio();
        let motor = Motor::novo(
            self.store.clone(),
            ConfigQuente::nova(
                self.store.clone(),
                OpcoesConfigQuente::default(),
                memory.clone(),
            ),
            Arc::new(Limitador::novo(OpcoesLimites::default(), memory)),
            provider,
            OpcoesTurno::default(),
            MetricasTurno::proprio(),
        );
        let http_tools = HttpToolRegistry::from_config(&self.config.http_tools, |name| {
            (name == "POPH_HTTP_TOOL_TEST")
                .then(|| Secret::new("Bearer chave-ficticia-do-servico".to_owned()))
        })
        .expect("HTTP tools");
        let motor = motor.with_http_tools(http_tools);
        let state = Estado::novo(self.store.clone(), self.config.clone())
            .expect("state")
            .com_motor(Arc::new(motor));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let task = tokio::spawn(poph_server::servir_em(
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
    let secret = "credencial-ficticia-chat-site";
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

    async fn create(
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
                &format!("/agents/{agent}/site-conversations"),
                Some(json!({"external_ref": reference})),
            )
            .await;
        read(response, status).await
    }

    async fn send(
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
                &format!("/site-conversations/{conversation}/messages"),
                Some(json!({"idempotency_key": key, "text": text})),
            )
            .await;
        read(response, status).await
    }

    async fn stream(&self, account: &Account, conversation: &str, after: i64) -> Response {
        let response = self
            .client
            .get(format!(
                "{}/admin/v1/orgs/{}/site-conversations/{conversation}/events/stream?after=0",
                self.url, account.org
            ))
            .bearer_auth(&account.token)
            .header("Last-Event-ID", after)
            .send()
            .await
            .expect("stream");
        assert_eq!(response.status(), StatusCode::OK);
        response
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

async fn wait_for_event(response: Response, kind: &str) -> Vec<Value> {
    let mut incoming = response.bytes_stream();
    let mut buffer = Vec::new();
    let mut events = Vec::new();
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(next) = incoming.next().await {
            buffer.extend_from_slice(&next.expect("stream data"));
            while let Some(end) = buffer.windows(2).position(|pair| pair == b"\n\n") {
                let frame =
                    String::from_utf8(buffer.drain(..end + 2).collect()).expect("event utf8");
                let Some(data) = frame.lines().find_map(|line| line.strip_prefix("data: ")) else {
                    continue;
                };
                let item: Value = serde_json::from_str(data).expect("event JSON");
                assert!(
                    frame
                        .lines()
                        .any(|line| line == format!("id: {}", item["seq"]))
                );
                let target = item["type"] == kind;
                events.push(item);
                if target {
                    return events;
                }
            }
        }
        panic!("stream ended before {kind}");
    })
    .await
    .expect("event deadline")
}

fn prompt_text(request: &Request) -> String {
    request
        .messages
        .iter()
        .filter_map(|message| match message {
            Message::User(parts) => Some(parts.iter().filter_map(|part| match part {
                Part::Texto(text) => Some(text.as_str()),
                _ => None,
            })),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn two_visitors_exchange_messages_with_isolated_history_and_replays() {
    let fixture = Fixture::new();
    let other = add_account(&fixture.store, "segunda");
    let another_agent = add_agent(&fixture.store, fixture.account.org, "outro-agente");
    let fake = Arc::new(FakeProvider::with_scripts(
        (0..4).map(|_| answer("Resposta de teste.")).collect(),
    ));
    let server = fixture.start(fake.clone()).await;
    let account = &fixture.account;
    let one = server
        .create(
            account,
            account.agent,
            "primeiro visitante",
            StatusCode::CREATED,
        )
        .await;
    let repeat = server
        .create(account, account.agent, "primeiro visitante", StatusCode::OK)
        .await;
    assert_eq!(one, repeat);
    let same_ref_other_agent = server
        .create(
            account,
            another_agent,
            "primeiro visitante",
            StatusCode::CREATED,
        )
        .await;
    assert_ne!(one, same_ref_other_agent);
    let two = server
        .create(
            account,
            account.agent,
            "segundo visitante",
            StatusCode::CREATED,
        )
        .await;
    let elsewhere = server
        .create(
            &other,
            other.agent,
            "primeiro visitante",
            StatusCode::CREATED,
        )
        .await;
    assert_ne!(one, two);
    assert_ne!(one, elsewhere);
    let one = one["conversation_id"].as_str().expect("conversation");
    let two = two["conversation_id"].as_str().expect("conversation");
    let conversation = fixture
        .store
        .conversations()
        .get(account.org, one.parse().expect("id"))
        .expect("stored conversation");
    assert_eq!(conversation.canal, "site");
    assert_eq!(conversation.integration_id, None);
    assert_eq!(
        conversation.peer_externo.as_deref(),
        Some("primeiro visitante")
    );

    let mut cursors = [0i64; 2];
    let mut latest_turn = Value::Null;
    for (index, (id, key, text)) in [
        (one, "a", "Pergunta exclusiva da Aurora"),
        (two, "a", "Pergunta exclusiva da Lua"),
        (one, "b", "Segunda pergunta da Aurora"),
        (two, "b", "Segunda pergunta da Lua"),
    ]
    .into_iter()
    .enumerate()
    {
        let cursor = &mut cursors[index % 2];
        let stream = server.stream(account, id, *cursor).await;
        let accepted = server
            .send(account, id, key, text, StatusCode::ACCEPTED)
            .await;
        assert_eq!(accepted["status"], "accepted");
        let events = wait_for_event(stream, "turn_end").await;
        assert_eq!(
            events
                .iter()
                .map(|event| event["type"].as_str().expect("type"))
                .collect::<Vec<_>>(),
            ["message", "turn_start", "text_delta", "message", "turn_end"]
        );
        assert!(
            events
                .windows(2)
                .all(|pair| pair[0]["seq"].as_i64() < pair[1]["seq"].as_i64())
        );
        assert!(events[0]["seq"].as_i64().expect("seq") > *cursor);
        *cursor = events.last().expect("final")["seq"].as_i64().expect("seq");
        assert!(
            events
                .iter()
                .all(|event| event
                    .as_object()
                    .expect("object")
                    .keys()
                    .all(|key| matches!(
                        key.as_str(),
                        "seq" | "type" | "turn_id" | "message_id" | "role" | "text" | "error_code"
                    )))
        );
        let replay = server.send(account, id, key, text, StatusCode::OK).await;
        assert_eq!(accepted, replay);
        let conflict = server
            .send(account, id, key, "Outro texto", StatusCode::CONFLICT)
            .await;
        assert_eq!(conflict["code"], "idempotency_conflict");
        latest_turn = accepted["turn_id"].clone();
        assert_eq!(
            fixture
                .store
                .outbox()
                .count_turn(
                    account.org,
                    latest_turn.as_str().expect("turn id").parse().expect("id")
                )
                .expect("outbox"),
            0
        );
        assert!(
            fixture
                .store
                .cases()
                .by_conversation(account.org, id.parse().expect("id"))
                .expect("case")
                .is_none()
        );
    }
    assert_eq!(fake.received().len(), 4);
    let requests = fake.received();
    assert!(prompt_text(&requests[2]).contains("Pergunta exclusiva da Aurora"));
    assert!(!prompt_text(&requests[2]).contains("Pergunta exclusiva da Lua"));
    assert!(prompt_text(&requests[3]).contains("Pergunta exclusiva da Lua"));
    assert!(!prompt_text(&requests[3]).contains("Pergunta exclusiva da Aurora"));

    let history_path = format!("/site-conversations/{one}/messages");
    let first_page = read(
        server
            .request(
                account,
                Method::GET,
                &format!("{history_path}?limit=1"),
                None,
            )
            .await,
        StatusCode::OK,
    )
    .await;
    let first_id = first_page["messages"][0]["id"]
        .as_str()
        .expect("message id");
    assert_eq!(first_page["messages"][0]["role"], "assistant");
    let older = read(
        server
            .request(
                account,
                Method::GET,
                &format!("{history_path}?before={first_id}"),
                None,
            )
            .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        older["messages"].as_array().expect("older messages").len(),
        3
    );
    assert_eq!(older["messages"][0]["text"], "Segunda pergunta da Aurora");

    read(
        server
            .request(&other, Method::GET, &history_path, None)
            .await,
        StatusCode::NOT_FOUND,
    )
    .await;
    read(
        server
            .request(
                account,
                Method::GET,
                &format!(
                    "/site-conversations/{one}/turns/{}",
                    latest_turn.as_str().expect("turn")
                ),
                None,
            )
            .await,
        StatusCode::NOT_FOUND,
    )
    .await;
    let unauthorized = server
        .client
        .get(format!(
            "{}/admin/v1/orgs/{}{}",
            server.url, account.org, history_path
        ))
        .send()
        .await
        .expect("unauthenticated request");
    read(unauthorized, StatusCode::UNAUTHORIZED).await;
    let wrong_token = server
        .client
        .get(format!(
            "{}/admin/v1/orgs/{}{}",
            server.url, account.org, history_path
        ))
        .bearer_auth(&other.token)
        .send()
        .await
        .expect("cross-org request");
    read(wrong_token, StatusCode::FORBIDDEN).await;

    let id: ConversationId = one.parse().expect("conversation id");
    let turn: TurnId = fixture
        .store
        .turns()
        .list(account.org, id, poph_store::Page::default())
        .expect("turns")[0]
        .id;
    assert!(
        fixture
            .store
            .conversations()
            .transfer(account.org, id, None, None, "{}".to_owned())
            .is_err()
    );
    assert!(
        fixture
            .store
            .cases()
            .create(
                account.org,
                NewRecord {
                    conversation_id: Some(id),
                    agent_id: account.agent,
                    canal: "site".to_owned(),
                    peer_externo: None,
                }
            )
            .is_err()
    );
    assert!(
        fixture
            .store
            .outbox()
            .enqueue(
                account.org,
                NewOutboxItem {
                    conversation_id: id,
                    turn_id: turn,
                    seq: 0,
                    tipo: "text".to_owned(),
                    content_json: "{}".to_owned(),
                }
            )
            .is_err()
    );
    assert_eq!(
        fixture
            .store
            .conversations()
            .get(account.org, id)
            .expect("conversation")
            .status,
        ConversationStatus::Open
    );
    assert!(
        fixture
            .store
            .matrix()
            .room(account.org, id)
            .expect("room")
            .is_none()
    );
}

#[tokio::test]
async fn overlapping_turns_cancel_and_resume_without_owning_the_subscription() {
    let fixture = Fixture::new();
    let fake = Arc::new(FakeProvider::with_scripts(vec![
        Script::Pending {
            events: vec![LlmEvent::Texto("Trecho inicial.".to_owned())],
            after: Duration::from_secs(50),
            error: LlmError::Cancelled,
        },
        answer("Resposta depois do cancelamento."),
    ]));
    let account = &fixture.account;
    let server = fixture.start(fake).await;
    let data = server
        .create(
            account,
            account.agent,
            "visitante do cancelamento",
            StatusCode::CREATED,
        )
        .await;
    let id = data["conversation_id"].as_str().expect("conversation");
    let opened = server.stream(account, id, 0).await;
    let accepted = server
        .send(
            account,
            id,
            "original",
            "Mensagem em andamento",
            StatusCode::ACCEPTED,
        )
        .await;
    let earlier = wait_for_event(opened, "text_delta").await;
    let last = earlier.last().expect("delta")["seq"].as_i64().expect("seq");
    let busy = server
        .send(
            account,
            id,
            "different",
            "Mensagem sobreposta",
            StatusCode::CONFLICT,
        )
        .await;
    assert_eq!(busy["code"], "turn_in_progress");
    assert_eq!(busy["details"]["turn_id"], accepted["turn_id"]);
    let turn_id = accepted["turn_id"].as_str().expect("turn");
    let turn_path = format!("/site-conversations/{id}/turns/{turn_id}");
    let running = read(
        server.request(account, Method::GET, &turn_path, None).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(running["status"], "running");
    let resumed = server.stream(account, id, last).await;
    let path = format!("{turn_path}/cancel");
    let cancelled = read(
        server.request(account, Method::POST, &path, None).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(cancelled["status"], "cancelled");
    assert_eq!(
        cancelled,
        read(
            server.request(account, Method::POST, &path, None).await,
            StatusCode::OK
        )
        .await
    );
    let events = wait_for_event(resumed, "turn_cancelled").await;
    assert_eq!(events.len(), 1);
    let last = events[0]["seq"].as_i64().expect("cancel sequence");
    let history = read(
        server
            .request(
                account,
                Method::GET,
                &format!("/site-conversations/{id}/messages"),
                None,
            )
            .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(history["messages"].as_array().expect("messages").len(), 1);

    let follow_up = server
        .send(
            account,
            id,
            "different",
            "Outra mensagem",
            StatusCode::ACCEPTED,
        )
        .await;
    let events = wait_for_event(server.stream(account, id, last).await, "turn_end").await;
    assert_eq!(events.len(), 5);
    let path = format!(
        "/site-conversations/{id}/turns/{}/cancel",
        follow_up["turn_id"].as_str().expect("turn")
    );
    let terminal = read(
        server.request(account, Method::POST, &path, None).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(terminal["status"], "succeeded");
}

#[tokio::test]
async fn restart_marks_interrupted_turns_failed_and_processes_queued_turns() {
    let mut fixture = Fixture::new();
    let account = &fixture.account;
    let (running, _) = fixture
        .store
        .site_chat()
        .create(
            account.org,
            account.agent,
            "visitante em execução".to_owned(),
        )
        .expect("conversation");
    let (queued, _) = fixture
        .store
        .site_chat()
        .create(account.org, account.agent, "visitante na fila".to_owned())
        .expect("conversation");
    let submit = |conversation| {
        let SiteSubmission::Accepted(_, turn) = fixture
            .store
            .site_chat()
            .submit(
                account.org,
                conversation,
                "primeira".to_owned(),
                "Mensagem antes da interrupção".to_owned(),
            )
            .expect("accepted turn")
        else {
            panic!("turn was not accepted");
        };
        SiteTurnKey {
            org: account.org,
            conversation,
            turn,
        }
    };
    let running_key = submit(running);
    let queued_key = submit(queued);
    assert!(
        fixture
            .store
            .site_chat()
            .claim(running_key)
            .expect("claim")
            .is_some()
    );
    fixture.store = Arc::new(Store::open(&fixture.config).expect("reopen database"));
    let fake = Arc::new(FakeProvider::with_scripts(vec![answer(
        "Resposta recuperada.",
    )]));
    let server = fixture.start(fake.clone()).await;
    let path = format!("/site-conversations/{running}/turns/{}", running_key.turn);
    let result = read(
        server.request(account, Method::GET, &path, None).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(result["status"], "failed");
    assert_eq!(result["error_code"], "server_restarted");
    assert!(result["finished_at"].is_number());
    let replay = server
        .send(
            account,
            &running.to_string(),
            "primeira",
            "Mensagem antes da interrupção",
            StatusCode::OK,
        )
        .await;
    assert_eq!(replay["turn_id"], running_key.turn.to_string());
    let events = wait_for_event(
        server.stream(account, &running.to_string(), 0).await,
        "turn_failed",
    )
    .await;
    assert_eq!(
        events.last().expect("failure")["error_code"],
        "server_restarted"
    );
    wait_for_event(
        server.stream(account, &queued.to_string(), 0).await,
        "turn_end",
    )
    .await;
    assert_eq!(
        fixture
            .store
            .site_chat()
            .turn(queued_key)
            .expect("queued turn")
            .status,
        "succeeded"
    );
    assert_eq!(fake.received().len(), 1);
}

#[tokio::test]
async fn tool_policy_and_request_limits_are_enforced() {
    let fixture = Fixture::new();
    let account = &fixture.account;
    let fake = Arc::new(FakeProvider::with_scripts(vec![]));
    let server = fixture.start(fake.clone()).await;
    let invalid = server
        .create(account, account.agent, "", StatusCode::UNPROCESSABLE_ENTITY)
        .await;
    assert_eq!(invalid["details"]["field"], "external_ref");
    let data = server
        .create(
            account,
            account.agent,
            "visitante de teste",
            StatusCode::CREATED,
        )
        .await;
    let id = data["conversation_id"].as_str().expect("conversation");
    let invalid = server
        .send(
            account,
            id,
            "x",
            &"á".repeat(129),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    assert_eq!(invalid["details"]["field"], "text");
    let invalid = server
        .send(account, id, "", "Texto", StatusCode::UNPROCESSABLE_ENTITY)
        .await;
    assert_eq!(invalid["details"]["field"], "idempotency_key");
    let invalid = read(
        server
            .request(
                account,
                Method::GET,
                &format!("/site-conversations/{id}/events?limit=201"),
                None,
            )
            .await,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
    assert_eq!(invalid["details"]["field"], "limit");
    let instructions = StructuredInstructions {
        tools: vec![ToolConfig::new("enviar_arquivo")],
        ..Default::default()
    };
    fixture
        .store
        .instructions()
        .save(
            account.org,
            account.agent,
            0,
            &serde_json::to_string(&instructions).expect("instructions"),
            "Orientações.",
            true,
        )
        .expect("save");
    let blocked = server
        .create(
            account,
            account.agent,
            "novo visitante",
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    assert_eq!(blocked["code"], "agent_not_public");
    let blocked = server
        .send(account, id, "x", "Texto", StatusCode::UNPROCESSABLE_ENTITY)
        .await;
    assert_eq!(blocked["code"], "agent_not_public");
    assert!(fake.received().is_empty());
}

async fn measure(provider: Arc<dyn LlmProvider>, label: &str, samples: usize) {
    let fixture = Fixture::new();
    let account = &fixture.account;
    let agent = fixture
        .store
        .agents()
        .get(account.org, account.agent)
        .expect("benchmark agent");
    fixture
        .store
        .agents()
        .update(
            account.org,
            account.agent,
            agent.version,
            poph_store::AgentChange {
                modelo: Some(label.to_owned()),
                max_tokens_saida: Some(64),
                ..Default::default()
            },
        )
        .expect("benchmark model");
    let server = fixture.start(provider).await;
    let value = server
        .create(
            account,
            account.agent,
            "visitante de medição",
            StatusCode::CREATED,
        )
        .await;
    let conversation = value["conversation_id"].as_str().expect("conversation");
    let mut cursor = 0;
    let mut accept_ms = Vec::new();
    let mut turn_ms = Vec::new();
    for sample in 0..samples {
        let stream = server.stream(account, conversation, cursor).await;
        let now = Instant::now();
        server
            .send(
                account,
                conversation,
                &sample.to_string(),
                "Responda apenas 'Olá.'",
                StatusCode::ACCEPTED,
            )
            .await;
        accept_ms.push(now.elapsed().as_secs_f64() * 1_000.0);
        let events = wait_for_event(stream, "turn_end").await;
        turn_ms.push(now.elapsed().as_secs_f64() * 1_000.0);
        cursor = events.last().expect("turn end")["seq"]
            .as_i64()
            .expect("seq");
    }
    let (accept_p50, accept_p95) = percentiles(&mut accept_ms);
    let (turn_p50, turn_p95) = percentiles(&mut turn_ms);
    println!(
        "{label}: {samples}/{samples}; submit p50={accept_p50:.2} ms p95={accept_p95:.2} ms; submit to terminal SSE p50={turn_p50:.2} ms p95={turn_p95:.2} ms"
    );
}

fn percentiles(samples: &mut [f64]) -> (f64, f64) {
    samples.sort_by(f64::total_cmp);
    (
        samples[(samples.len() * 50).div_ceil(100) - 1],
        samples[(samples.len() * 95).div_ceil(100) - 1],
    )
}

#[tokio::test]
#[ignore = "latency measurement"]
async fn measure_site_fake_model() {
    measure(
        Arc::new(FakeProvider::with_scripts(
            (0..20).map(|_| answer("Olá.")).collect(),
        )),
        "simulated",
        20,
    )
    .await;
}

#[tokio::test]
#[ignore = "requires a real model provider"]
async fn measure_site_real_model() {
    let model = std::env::var("PA82B_MODEL").expect("PA82B_MODEL");
    let provider = OpenAiCompatibleClient::new(
        OpenAiCompatibleConfig {
            url_base: std::env::var("PA82B_MODEL_URL").expect("PA82B_MODEL_URL"),
            chave: Secret::new(std::env::var("PA82B_MODEL_KEY").expect("PA82B_MODEL_KEY")),
            modelo: model.clone(),
            timeout: Duration::from_secs(40),
            retentativas: 0,
            base_wait: Duration::from_secs(1),
            temperatura: 0.0,
            max_tokens_saida: 64,
            recusa_turno_modelo: false,
        },
        LlmLimiter::new(1, 1).expect("limits"),
        BaseMetrics::register(&Registry::new()).expect("metrics"),
    )
    .expect("model provider");
    measure(Arc::new(provider), &model, 20).await;
}

#[derive(Default)]
struct HttpToolStub {
    calls: Vec<(HeaderMap, Value)>,
    effects: usize,
}

async fn lead_stub(
    State(stub): State<Arc<Mutex<HttpToolStub>>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let outcome = body["arguments"]["consent"].as_bool();
    {
        let mut state = stub.lock().expect("stub lock");
        if outcome == Some(true) {
            state.effects += 1;
        }
        state.calls.push((headers, body));
    }
    match outcome {
        Some(true) => (
            StatusCode::OK,
            Json(json!({"lead":{"status":"registered_notified"},"discard":"internal"})),
        ),
        Some(false) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"discard":"private details"})),
        ),
        None => {
            tokio::time::sleep(Duration::from_secs(3)).await;
            (StatusCode::OK, Json(json!({})))
        }
    }
}

fn register_lead_call(id: &str, consent: Option<bool>) -> Script {
    Script::Events(vec![LlmEvent::Tool(ToolCall {
        id: id.to_owned(),
        nome: "register_lead".to_owned(),
        arguments: json!({"consent": consent, "org_id": "ignorar", "message_id": "ignorar"}),
    })])
}

#[tokio::test]
async fn configured_http_tool_carries_site_context_and_replays_without_repeating_effects() {
    let mut fixture = Fixture::new();
    let stub = Arc::new(Mutex::new(HttpToolStub::default()));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("stub listener");
    let url = format!(
        "http://{}/register",
        listener.local_addr().expect("stub address")
    );
    let router = Router::new()
        .route("/register", post(lead_stub))
        .with_state(stub.clone());
    let stub_task = tokio::spawn(async move { axum::serve(listener, router).await });

    let account = &fixture.account;
    let config: HarnessConfig = toml::from_str(&format!(
        r#"
[site_chat]
allowed_tools = ["register_lead"]

[http_tools."{}".register_lead]
url = "{}"
method = "POST"
authorization_env = "POPH_HTTP_TOOL_TEST"
description = "Registra o interesse do visitante."
parameters = {{ type = "object" }}
response_fields = {{ status = "/lead/status" }}
"#,
        account.org, url
    ))
    .expect("TOML");
    config.validar().expect("configured tool and site policy");
    fixture.config.site_chat = config.site_chat;
    fixture.config.http_tools = config.http_tools;
    let definition = fixture
        .config
        .http_tools
        .get_mut(&account.org)
        .and_then(|tools| tools.get_mut("register_lead"))
        .expect("definition");
    assert_eq!(definition.timeout_ms, 5_000);
    definition.timeout_ms = 500;

    let instructions = StructuredInstructions {
        tools: vec![ToolConfig::new("register_lead")],
        ..Default::default()
    };
    instructions.validar().expect("closed list");
    fixture
        .store
        .instructions()
        .save(
            account.org,
            account.agent,
            0,
            &instructions.to_json(),
            "Orientações.",
            true,
        )
        .expect("save");

    let other = add_account(&fixture.store, "outra-org");
    fixture
        .store
        .instructions()
        .save(
            other.org,
            other.agent,
            0,
            &instructions.to_json(),
            "Orientações.",
            true,
        )
        .expect("other organization instructions");

    let fake = Arc::new(FakeProvider::with_scripts(vec![
        register_lead_call("original-call", Some(true)),
        register_lead_call("original-call", Some(true)),
        answer("O aviso foi enviado."),
        register_lead_call("original-call", Some(false)),
        answer("O serviço de registro está indisponível."),
        register_lead_call("slow-call", None),
        answer("O serviço demorou para responder."),
        register_lead_call("foreign-call", Some(true)),
        answer("O registro não está disponível."),
    ]));
    let server = fixture.start(fake.clone()).await;
    let conversation = server
        .create(account, account.agent, "visitante", StatusCode::CREATED)
        .await;
    let conversation_id = conversation["conversation_id"]
        .as_str()
        .expect("conversation");

    let mut submitted = Vec::new();
    let mut after = 0;
    for n in 0..3 {
        let message = server
            .send(
                account,
                conversation_id,
                &format!("message-{n}"),
                "Autorizo o contato.",
                StatusCode::ACCEPTED,
            )
            .await;
        let turn_id = message["turn_id"].as_str().expect("turn id");
        let events = wait_for_event(
            server.stream(account, conversation_id, after).await,
            "turn_end",
        )
        .await;
        after = events
            .last()
            .and_then(|event| event["seq"].as_i64())
            .expect("sequence");
        assert!(
            events
                .iter()
                .any(|event| event["turn_id"] == turn_id && event["type"] == "turn_end")
        );
        let turn = read(
            server
                .request(
                    account,
                    Method::GET,
                    &format!("/site-conversations/{conversation_id}/turns/{turn_id}"),
                    None,
                )
                .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(turn["status"], "succeeded");
        submitted.push(message);
    }

    let other_conversation = server
        .create(&other, other.agent, "outro visitante", StatusCode::CREATED)
        .await;
    let other_id = other_conversation["conversation_id"]
        .as_str()
        .expect("other conversation");
    server
        .send(
            &other,
            other_id,
            "other-message",
            "Autorizo o contato.",
            StatusCode::ACCEPTED,
        )
        .await;
    wait_for_event(server.stream(&other, other_id, 0).await, "turn_end").await;

    {
        let state = stub.lock().expect("stub lock");
        assert_eq!(state.effects, 1);
        assert_eq!(state.calls.len(), 3);
        let expected_ids = ["original-call", "original-call", "slow-call"];
        for ((headers, body), (message, call_id)) in
            state.calls.iter().zip(submitted.iter().zip(expected_ids))
        {
            assert_eq!(headers["authorization"], "Bearer chave-ficticia-do-servico");
            assert_eq!(
                headers["idempotency-key"].to_str().expect("key"),
                format!(
                    "{}:{call_id}",
                    message["turn_id"].as_str().expect("turn id")
                )
            );
            assert_eq!(
                body,
                &json!({
                    "context": {
                        "org_id": account.org,
                        "agent_id": account.agent,
                        "conversation_id": conversation_id,
                        "turn_id": message["turn_id"],
                        "message_id": message["message_id"]
                    },
                    "call_id": call_id,
                    "arguments": {
                        "consent": body["arguments"]["consent"],
                        "org_id": "ignorar",
                        "message_id": "ignorar"
                    }
                })
            );
        }
    }

    let requests = fake.received();
    assert_eq!(requests[7].org, other.org);
    assert!(requests[7].tools.is_empty());
    assert!(matches!(
        requests[8].messages.last(),
        Some(Message::Resultado { id, content })
            if id == "foreign-call" && content.starts_with("erro:")
    ));
    let tool = &requests[0].tools[0];
    assert_eq!(tool.nome, "register_lead");
    assert_eq!(tool.parameters, json!({"type": "object"}));
    for (request, id, result) in [
        (
            &requests[1],
            "original-call",
            r#"{"status":"registered_notified"}"#,
        ),
        (
            &requests[2],
            "original-call",
            r#"{"status":"registered_notified"}"#,
        ),
        (
            &requests[4],
            "original-call",
            "erro: http_tool_http_error: serviço recusou a chamada",
        ),
        (
            &requests[6],
            "slow-call",
            "erro: http_tool_timeout: serviço demorou para responder",
        ),
    ] {
        assert_eq!(
            request.messages.last(),
            Some(&Message::Resultado {
                id: id.to_owned(),
                content: result.to_owned()
            }),
        );
    }
    stub_task.abort();
}
