use std::path::PathBuf;
use std::sync::Arc;

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::ServerState;
use poph_store::{NewAgent, NewOrg, OutboxState, Store};
use poph_types::{AgentId, ConversationId, HarnessConfig, OrgId, RecordStatus};
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
        let root = std::env::temp_dir().join(format!("poph-adapter-{}", OrgId::generate()));
        let mut config = HarnessConfig::default();
        config.data.directory = root.clone();
        config.adapter_chat.max_text_bytes = 256;
        let store = Arc::new(Store::open(&config).expect("open"));
        let account = add_account(&store, "primeira");
        Self {
            root,
            config,
            store,
            account,
        }
    }

    async fn start(&self) -> Server {
        let state = ServerState::new(self.store.clone(), self.config.clone()).expect("state");
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
    let secret = "credencial-ficticia-adaptador";
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
    async fn send(
        &self,
        account: &Account,
        method: Method,
        path: &str,
        data: Option<Value>,
        version: Option<u64>,
    ) -> Response {
        let mut builder = self
            .client
            .request(
                method,
                format!("{}/admin/v1/orgs/{}{}", self.url, account.org, path),
            )
            .bearer_auth(&account.token);
        if let Some(version) = version {
            builder = builder.header("If-Match", version.to_string());
        }
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
        channel: &str,
        reference: &str,
        create_case: Option<bool>,
        status: StatusCode,
    ) -> Value {
        let mut data = json!({"channel": channel, "external_ref": reference});
        if let Some(create_case) = create_case {
            data["create_case"] = json!(create_case);
        }
        let response = self
            .send(
                account,
                Method::POST,
                &format!("/agents/{agent}/adapter-conversations"),
                Some(data),
                None,
            )
            .await;
        read(response, status).await
    }

    async fn write(
        &self,
        account: &Account,
        conversation: &str,
        body: Value,
        status: StatusCode,
    ) -> Value {
        let response = self
            .send(
                account,
                Method::POST,
                &format!("/adapter-conversations/{conversation}/messages"),
                Some(body),
                None,
            )
            .await;
        read(response, status).await
    }
}

async fn read(response: Response, status: StatusCode) -> Value {
    let actual = response.status();
    let data: Value = response.json().await.expect("response JSON");
    assert_eq!(actual, status, "{data}");
    data
}

fn customer(text: &str, key: &str) -> Value {
    json!({"text": text, "origin": "customer", "idempotency_key": key, "external_message_id": format!("ext-{key}")})
}

#[tokio::test]
async fn popbot_defaults_to_no_case_and_honors_an_explicit_override() {
    let fixture = Fixture::new();
    let server = fixture.start().await;
    let account = &fixture.account;

    for (channel, reference, create_case, expected_case) in [
        ("popbot", "popbot-default", None, false),
        ("popbot", "popbot-with-case", Some(true), true),
        ("email", "email-default", None, true),
    ] {
        let conversation = server
            .open(
                account,
                account.agent,
                channel,
                reference,
                create_case,
                StatusCode::CREATED,
            )
            .await;
        assert_eq!(conversation["channel"], channel);
        assert_eq!(conversation["create_case"], expected_case, "{reference}");
        if expected_case {
            assert!(conversation["ficha_id"].is_string(), "{reference}");
        } else {
            assert_eq!(conversation.get("ficha_id"), Some(&Value::Null));
        }
    }
}

#[tokio::test]
async fn creates_resumes_writes_and_isolates_adapter_conversations() {
    let fixture = Fixture::new();
    let other = add_account(&fixture.store, "segunda");
    let another_agent = add_agent(&fixture.store, fixture.account.org, "outro-agente");
    let server = fixture.start().await;
    let account = &fixture.account;

    let created = server
        .open(
            account,
            account.agent,
            "email",
            "thread-1",
            None,
            StatusCode::CREATED,
        )
        .await;
    assert_eq!(created["channel"], "email");
    assert_eq!(created["external_ref"], "thread-1");
    assert_eq!(created["create_case"], true);
    assert_eq!(created["status"], "open");
    let conversation = created["id"].as_str().expect("conversation").to_owned();
    let case_id = created["ficha_id"].as_str().expect("case").to_owned();

    let stored = fixture
        .store
        .conversations()
        .get(account.org, conversation.parse().expect("id"))
        .expect("stored conversation");
    assert_eq!(stored.canal, "email");
    assert_eq!(stored.peer_externo.as_deref(), Some("thread-1"));
    assert_eq!(stored.integration_id, None);

    let resumed = server
        .open(
            account,
            account.agent,
            "email",
            "thread-1",
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(resumed, created);

    let other_agent = server
        .open(
            account,
            another_agent,
            "email",
            "thread-1",
            None,
            StatusCode::CREATED,
        )
        .await;
    assert_ne!(other_agent["id"], created["id"]);

    let elsewhere = server
        .open(
            &other,
            other.agent,
            "email",
            "thread-1",
            None,
            StatusCode::CREATED,
        )
        .await;
    assert_ne!(elsewhere["id"], created["id"]);

    let detail = server
        .send(
            account,
            Method::GET,
            &format!("/adapter-conversations/{conversation}"),
            None,
            None,
        )
        .await;
    assert_eq!(read(detail, StatusCode::OK).await, created);

    let crossed = server
        .client
        .get(format!(
            "{}/admin/v1/orgs/{}/adapter-conversations/{}",
            server.url, account.org, conversation
        ))
        .bearer_auth(&other.token)
        .send()
        .await
        .expect("crossed");
    assert_eq!(crossed.status(), StatusCode::FORBIDDEN);

    let foreign = server
        .send(
            account,
            Method::GET,
            &format!(
                "/adapter-conversations/{}",
                elsewhere["id"].as_str().unwrap()
            ),
            None,
            None,
        )
        .await;
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);

    let message = server
        .write(
            account,
            &conversation,
            customer("Primeira pergunta", "k1"),
            StatusCode::CREATED,
        )
        .await;
    assert_eq!(message["role"], "user");
    assert_eq!(message["origin"], "agent");
    assert_eq!(message["external_message_id"], "ext-k1");
    let message_id = message["id"].as_str().expect("message").to_owned();

    let replay = server
        .write(
            account,
            &conversation,
            customer("Primeira pergunta", "k1"),
            StatusCode::OK,
        )
        .await;
    assert_eq!(replay, message);

    let conflict = server
        .write(
            account,
            &conversation,
            customer("Outra pergunta", "k1"),
            StatusCode::CONFLICT,
        )
        .await;
    assert_eq!(conflict["code"], "idempotency_conflict");

    let agent_reply = server
        .write(
            account,
            &conversation,
            json!({"text": "Resposta do agente", "origin": "agent", "idempotency_key": "k2"}),
            StatusCode::CREATED,
        )
        .await;
    assert_eq!(agent_reply["role"], "assistant");
    assert_eq!(agent_reply["origin"], "agent");

    let human = server
        .write(
            account,
            &conversation,
            json!({"text": "Resposta do atendente", "origin": "human", "author_name": "Ana", "idempotency_key": "k3"}),
            StatusCode::CREATED,
        )
        .await;
    assert_eq!(human["role"], "assistant");
    assert_eq!(human["origin"], "human");
    assert_eq!(human["author_name"], "Ana");

    let invalid_origin = server
        .write(
            account,
            &conversation,
            json!({"text": "Oi", "origin": "visitor", "idempotency_key": "k4"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    assert_eq!(invalid_origin["details"]["field"], "origin");

    let history = server
        .send(
            account,
            Method::GET,
            &format!("/adapter-conversations/{conversation}/messages"),
            None,
            None,
        )
        .await;
    let history = read(history, StatusCode::OK).await;
    let texts: Vec<&str> = history["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .map(|item| item["text"].as_str().expect("text"))
        .collect();
    assert_eq!(
        texts,
        [
            "Resposta do atendente",
            "Resposta do agente",
            "Primeira pergunta"
        ]
    );
    assert_eq!(
        fixture
            .store
            .messages()
            .count_rows(account.org, conversation.parse().unwrap())
            .expect("count"),
        3
    );

    let first_page = server
        .send(
            account,
            Method::GET,
            &format!("/adapter-conversations/{conversation}/messages?limit=1"),
            None,
            None,
        )
        .await;
    let first_page = read(first_page, StatusCode::OK).await;
    assert_eq!(first_page["messages"].as_array().expect("page").len(), 1);

    let older = server
        .send(
            account,
            Method::GET,
            &format!("/adapter-conversations/{conversation}/messages?limit=10&before={message_id}"),
            None,
            None,
        )
        .await;
    let older = read(older, StatusCode::OK).await;
    assert!(older["messages"].as_array().expect("older").is_empty());

    for channel in ["test", "site", "whatsapp", "matrix", "api"] {
        let refused = server
            .open(
                account,
                account.agent,
                channel,
                "reservado",
                None,
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await;
        assert_eq!(refused["details"]["field"], "channel", "{channel}");
    }
    for channel in ["Email", "", "with space", "acentuação"] {
        let refused = server
            .open(
                account,
                account.agent,
                channel,
                "invalido",
                None,
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await;
        assert_eq!(refused["details"]["field"], "channel", "{channel}");
    }

    let without_case = server
        .open(
            account,
            account.agent,
            "email",
            "thread-2",
            Some(false),
            StatusCode::CREATED,
        )
        .await;
    assert_eq!(without_case["create_case"], false);
    assert!(without_case["ficha_id"].is_null());
    let without_case_id: ConversationId = without_case["id"].as_str().unwrap().parse().unwrap();
    assert!(
        fixture
            .store
            .cases()
            .by_conversation(account.org, without_case_id)
            .expect("ficha lookup")
            .is_none()
    );
    let body = server
        .write(
            account,
            without_case["id"].as_str().unwrap(),
            customer("Sem Ficha", "k5"),
            StatusCode::CREATED,
        )
        .await;
    assert_eq!(body["text"], "Sem Ficha");

    let detail_general = server
        .send(
            account,
            Method::GET,
            &format!("/conversations/{conversation}"),
            None,
            None,
        )
        .await;
    let detail_general = read(detail_general, StatusCode::OK).await;
    assert_eq!(detail_general["channel"], "email");
    assert_eq!(detail_general["external_peer"], "thread-1");
    assert!(detail_general["matrix_room_id"].is_null());

    let tested = server
        .send(
            account,
            Method::POST,
            &format!("/agents/{}/test-chat", account.agent),
            None,
            None,
        )
        .await;
    let tested = read(tested, StatusCode::CREATED).await;

    let listed = server
        .send(account, Method::GET, "/conversations", None, None)
        .await;
    let listed = read(listed, StatusCode::OK).await;
    let items = listed["items"].as_array().expect("items");
    assert!(
        items
            .iter()
            .any(|item| item["id"] == created["id"] && item["channel"] == "email")
    );
    assert!(
        items
            .iter()
            .any(|item| item["id"] == tested["id"] && item["channel"] == "test")
    );

    let now = fixture.store.now() + 60_000;
    let analytics = server
        .send(
            account,
            Method::GET,
            &format!("/analytics/conversations?from=0&to={now}"),
            None,
            None,
        )
        .await;
    let analytics = read(analytics, StatusCode::OK).await;
    let by_channel = analytics["by_channel"].as_array().expect("by_channel");
    let total = |channel: &str| {
        by_channel
            .iter()
            .find(|item| item["channel"] == channel)
            .map(|item| item["total"].as_u64().expect("total"))
    };
    assert_eq!(total("email"), Some(3));
    assert_eq!(total("test"), Some(1));

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
                .count_by_state(account.org, state)
                .expect("outbox"),
            0
        );
    }

    let snapshot = fixture
        .store
        .org_snapshot(account.org, true)
        .expect("snapshot");
    assert!(
        snapshot
            .tables
            .get("adapter_conversations")
            .expect("exported")
            .iter()
            .any(|row| row["id"] == created["id"])
    );
    assert_eq!(
        snapshot
            .tables
            .get("adapter_message_keys")
            .expect("keys")
            .len(),
        4
    );

    let transfer = server
        .send(
            account,
            Method::POST,
            &format!("/conversations/{conversation}/transfer"),
            Some(json!({"reason": "fila do time"})),
            None,
        )
        .await;
    let transfer = read(transfer, StatusCode::OK).await;
    assert_eq!(transfer["status"], "transferred");
    let version = transfer["version"].as_u64().expect("version");

    let resumed_transferred = server
        .open(
            account,
            account.agent,
            "email",
            "thread-1",
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(resumed_transferred["id"], created["id"]);

    let reopen = server
        .send(
            account,
            Method::POST,
            &format!("/conversations/{conversation}/reopen"),
            None,
            Some(version),
        )
        .await;
    let reopen = read(reopen, StatusCode::OK).await;
    assert_eq!(reopen["status"], "open");
    let version = reopen["version"].as_u64().expect("version");

    let closed = server
        .send(
            account,
            Method::POST,
            &format!("/conversations/{conversation}/close"),
            None,
            Some(version),
        )
        .await;
    let closed = read(closed, StatusCode::OK).await;
    assert_eq!(closed["status"], "closed");

    let closed_message = server
        .send(
            account,
            Method::POST,
            &format!("/adapter-conversations/{conversation}/messages"),
            Some(customer("Tarde demais", "k9")),
            None,
        )
        .await;
    assert_eq!(closed_message.status(), StatusCode::CONFLICT);

    let reopened = server
        .open(
            account,
            account.agent,
            "email",
            "thread-1",
            None,
            StatusCode::CREATED,
        )
        .await;
    assert_ne!(reopened["id"], created["id"]);
    assert_ne!(reopened["ficha_id"], created["ficha_id"]);
    let previous_case = fixture
        .store
        .cases()
        .get(account.org, case_id.parse().expect("case id"))
        .expect("old ficha");
    assert_eq!(previous_case.status, RecordStatus::Closed);

    let site = fixture
        .store
        .site_chat()
        .create(account.org, account.agent, "visita".to_owned())
        .expect("site")
        .0;
    let site_as_adapter = server
        .send(
            account,
            Method::GET,
            &format!("/adapter-conversations/{site}"),
            None,
            None,
        )
        .await;
    assert_eq!(site_as_adapter.status(), StatusCode::NOT_FOUND);
    let adapter_as_site = server
        .send(
            account,
            Method::GET,
            &format!(
                "/site-conversations/{}/messages",
                reopened["id"].as_str().unwrap()
            ),
            None,
            None,
        )
        .await;
    assert_eq!(adapter_as_site.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn closing_without_a_case_never_creates_one() {
    let fixture = Fixture::new();
    let server = fixture.start().await;
    let account = &fixture.account;
    let created = server
        .open(
            account,
            account.agent,
            "email",
            "sem-ficha",
            Some(false),
            StatusCode::CREATED,
        )
        .await;
    let conversation: ConversationId = created["id"].as_str().unwrap().parse().unwrap();
    let version = created["version"].as_u64().expect("version");
    let closed = server
        .send(
            account,
            Method::POST,
            &format!("/conversations/{conversation}/close"),
            None,
            Some(version),
        )
        .await;
    let closed = read(closed, StatusCode::OK).await;
    assert_eq!(closed["status"], "closed");
    assert!(
        fixture
            .store
            .cases()
            .by_conversation(account.org, conversation)
            .expect("ficha lookup")
            .is_none()
    );
}
