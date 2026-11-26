use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::Estado;
use poph_store::Store;
use poph_types::HarnessConfig;
use poph_types::config::GlobalTokenConfig;
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const GLOBAL_TOKEN: &str = "pophg_teste-global-analytics";

fn hash_argon2(value: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &salt)
        .expect("hash argon2")
        .to_string()
}

struct Scenario {
    base: String,
    client: Client,
    dir: PathBuf,
    store: Arc<Store>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Scenario {
    async fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "poph-analytics-{label}-{}-{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dados")).expect("data dir");

        let mut config = HarnessConfig::default();
        config.data.directory = dir.join("dados");
        config.server.port = 7483;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(GLOBAL_TOKEN),
        }];
        config.validar().expect("valid configuration");

        let store = Arc::new(Store::open(&config).expect("opens the database"));
        let estado = Estado::novo(Arc::clone(&store), config).expect("server state");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("ephemeral port");
        let address = listener.local_addr().expect("local address");
        let (shutdown, signal) = oneshot::channel();
        tokio::spawn(async move {
            let _ = poph_server::servir_em(listener, estado, async {
                let _ = signal.await;
            })
            .await;
        });

        let scenario = Self {
            base: format!("http://{address}"),
            client: Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("http client"),
            dir,
            store,
            shutdown: Some(shutdown),
        };
        scenario.wait_until_up().await;
        scenario
    }

    async fn wait_until_up(&self) {
        let start = Instant::now();
        loop {
            match self
                .client
                .get(format!("{}/healthz", self.base))
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => return,
                _ if start.elapsed() < Duration::from_secs(10) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                other => panic!("server did not start: {other:?}"),
            }
        }
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut call = self
            .client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(token);
        if let Some(value) = body {
            call = call.json(&value);
        }
        let response = call.send().await.expect("http call");
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        let json = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or(Value::String(text))
        };
        (status, json)
    }

    async fn get(&self, path: &str, token: &str) -> (StatusCode, Value) {
        self.request(Method::GET, path, token, None).await
    }

    async fn create_org(&self, slug: &str) -> String {
        let (status, body) = self
            .request(
                Method::POST,
                "/admin/v1/orgs",
                GLOBAL_TOKEN,
                Some(json!({ "name": format!("Org {slug}"), "slug": slug })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "create org: {body}");
        body["id"].as_str().expect("org id").to_owned()
    }

    async fn create_token(&self, org: &str) -> String {
        let (status, body) = self
            .request(
                Method::POST,
                &format!("/admin/v1/orgs/{org}/tokens"),
                GLOBAL_TOKEN,
                Some(json!({ "label": "token de teste" })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "create token: {body}");
        body["value"].as_str().expect("token value").to_owned()
    }

    async fn create_agent(&self, org: &str, token: &str, slug: &str) -> String {
        let (status, body) = self
            .request(
                Method::POST,
                &format!("/admin/v1/orgs/{org}/agents"),
                token,
                Some(json!({ "name": format!("Agente {slug}"), "slug": slug })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "create agent: {body}");
        body["id"].as_str().expect("agent id").to_owned()
    }

    fn connection(&self) -> rusqlite::Connection {
        let conn =
            rusqlite::Connection::open(self.store.harness_path()).expect("opens the test db");
        conn.busy_timeout(Duration::from_secs(10))
            .expect("database timeout");
        conn
    }

    fn insert_open_case(&self, org: &str, agent: &str, opened_at: i64) -> String {
        let id = uuid::Uuid::now_v7().to_string();
        self.connection()
            .execute(
                "INSERT INTO fichas (id, org_id, conversation_id, agent_id, canal, peer_externo,
                                     status, aberta_em, fechada_em, fechada_por, fechada_motivo,
                                     version)
                 VALUES (?1, ?2, NULL, ?3, 'whatsapp', 'cliente-ficticio', 'open', ?4, NULL, NULL,
                         NULL, 1)",
                rusqlite::params![id, org, agent, opened_at],
            )
            .expect("inserts the open case");
        id
    }

    fn insert_closed_case(&self, org: &str, agent: &str, opened_at: i64, closed_at: i64) -> String {
        let id = uuid::Uuid::now_v7().to_string();
        self.connection()
            .execute(
                "INSERT INTO fichas (id, org_id, conversation_id, agent_id, canal, peer_externo,
                                     status, aberta_em, fechada_em, fechada_por, fechada_motivo,
                                     version)
                 VALUES (?1, ?2, NULL, ?3, 'whatsapp', 'cliente-ficticio', 'closed', ?4, ?5,
                         'cliente', 'atendimento encerrado', 1)",
                rusqlite::params![id, org, agent, opened_at, closed_at],
            )
            .expect("inserts the closed case");
        id
    }

    fn insert_conversation(&self, org: &str, agent: &str, situation: &str, now: i64) -> String {
        let id = uuid::Uuid::now_v7().to_string();
        self.connection()
            .execute(
                "INSERT INTO conversations (id, org_id, agent_id, integration_id, channel,
                                            external_peer, status, target_department_id, summary,
                                            version, created_at, updated_at, last_message_at)
                 VALUES (?1, ?2, ?3, NULL, 'whatsapp', 'cliente-ficticio', ?4, NULL, NULL, 1, ?5,
                         ?5, ?5)",
                rusqlite::params![id, org, agent, situation, now],
            )
            .expect("inserts the conversation");
        id
    }

    fn insert_flow_run(
        &self,
        org: &str,
        case: &str,
        situation: &str,
        node: &str,
        started: i64,
    ) -> String {
        let id = uuid::Uuid::now_v7().to_string();
        let finished = (situation != "ativa").then_some(started + 1000);
        self.connection()
            .execute(
                "INSERT INTO fluxo_execucoes (id, org_id, ficha_id, fluxo_id, revisao_id, status,
                                              no_atual, dados_json, resultado, motivo_fim,
                                              iniciada_em, finalizada_em, version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '{}', NULL, NULL, ?8, ?9, 1)",
                rusqlite::params![
                    id,
                    org,
                    case,
                    uuid::Uuid::now_v7().to_string(),
                    uuid::Uuid::now_v7().to_string(),
                    situation,
                    node,
                    started,
                    finished
                ],
            )
            .expect("inserts the flow run");
        id
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn summary_path(org: &str) -> String {
    format!("/admin/v1/orgs/{org}/analytics/operational-summary")
}

fn as_integer(value: &Value, field: &str) -> u64 {
    value[field]
        .as_u64()
        .unwrap_or_else(|| panic!("field `{field}` is an integer: {value}"))
}

#[tokio::test]
async fn empty_org_answers_zeros() {
    let scenario = Scenario::new("empty").await;
    let org = scenario.create_org("vazia").await;
    let token = scenario.create_token(&org).await;

    let (status, body) = scenario
        .get(&format!("{}?from=1000&to=2000", summary_path(&org)), &token)
        .await;
    assert_eq!(status, StatusCode::OK, "summary: {body}");
    assert_eq!(as_integer(&body, "open_fichas"), 0);
    assert_eq!(as_integer(&body, "closed_fichas"), 0);
    assert_eq!(as_integer(&body, "open_conversations"), 0);
    assert_eq!(body["avg_resolution_ms"], Value::Null);
    assert_eq!(
        body["active_flow_nodes"],
        json!([]),
        "empty list, not null: {body}"
    );
}

#[tokio::test]
async fn the_window_cuts_open_and_closed_cases() {
    let scenario = Scenario::new("window").await;
    let org = scenario.create_org("janela").await;
    let token = scenario.create_token(&org).await;
    let agent = scenario.create_agent(&org, &token, "atendente").await;

    scenario.insert_open_case(&org, &agent, 500);
    scenario.insert_open_case(&org, &agent, 1500);
    scenario.insert_open_case(&org, &agent, 2500);
    scenario.insert_closed_case(&org, &agent, 0, 1000);
    scenario.insert_closed_case(&org, &agent, 500, 1200);
    scenario.insert_closed_case(&org, &agent, 0, 900);
    scenario.insert_closed_case(&org, &agent, 0, 2000);

    let (status, body) = scenario
        .get(&format!("{}?from=1000&to=2000", summary_path(&org)), &token)
        .await;
    assert_eq!(status, StatusCode::OK, "summary: {body}");
    assert_eq!(
        as_integer(&body, "open_fichas"),
        2,
        "opened before `to`, not cut by `from`: {body}"
    );
    assert_eq!(
        as_integer(&body, "closed_fichas"),
        2,
        "`[from, to)`: the ones closed at 1000 and 1200 are in; 900 and 2000 are out: {body}"
    );
    assert_eq!(
        body["avg_resolution_ms"], 850.0,
        "mean of the two closed inside the window, (1000 + 700) / 2: {body}"
    );
}

#[tokio::test]
async fn the_mean_comes_from_the_cases_closed_in_the_window() {
    let scenario = Scenario::new("mean").await;
    let org = scenario.create_org("media").await;
    let token = scenario.create_token(&org).await;
    let agent = scenario.create_agent(&org, &token, "atendente").await;

    scenario.insert_closed_case(&org, &agent, 0, 600_000);
    scenario.insert_closed_case(&org, &agent, 0, 1_200_000);

    let (status, body) = scenario
        .get(&format!("{}?from=0&to=2000000", summary_path(&org)), &token)
        .await;
    assert_eq!(status, StatusCode::OK, "summary: {body}");
    assert_eq!(as_integer(&body, "closed_fichas"), 2);
    assert_eq!(body["avg_resolution_ms"], 900_000.0, "mean: {body}");
}

#[tokio::test]
async fn active_flow_runs_are_counted_per_node() {
    let scenario = Scenario::new("nodes").await;
    let org = scenario.create_org("nos").await;
    let token = scenario.create_token(&org).await;
    let agent = scenario.create_agent(&org, &token, "atendente").await;

    let open = scenario.insert_open_case(&org, &agent, 500);
    let other_open = scenario.insert_open_case(&org, &agent, 600);
    let closed = scenario.insert_closed_case(&org, &agent, 100, 400);
    let later = scenario.insert_open_case(&org, &agent, 2500);

    scenario.insert_flow_run(&org, &open, "ativa", "aprovacao_orcamento", 700);
    scenario.insert_flow_run(&org, &other_open, "ativa", "aprovacao_orcamento", 800);
    scenario.insert_flow_run(&org, &closed, "concluida", "aprovacao_orcamento", 200);
    scenario.insert_flow_run(&org, &later, "ativa", "aprovacao_orcamento", 2600);

    let (status, body) = scenario
        .get(&format!("{}?from=1000&to=2000", summary_path(&org)), &token)
        .await;
    assert_eq!(status, StatusCode::OK, "summary: {body}");
    assert_eq!(
        body["active_flow_nodes"],
        json!([{ "node_id": "aprovacao_orcamento", "count": 2 }]),
        "only open cases created before `to`, and the finished run stays out: {body}"
    );
}

#[tokio::test]
async fn two_orgs_do_not_leak_into_each_other() {
    let scenario = Scenario::new("orgs").await;
    let first = scenario.create_org("primeira").await;
    let first_token = scenario.create_token(&first).await;
    let first_agent = scenario
        .create_agent(&first, &first_token, "agente-um")
        .await;
    let second = scenario.create_org("segunda").await;
    let second_token = scenario.create_token(&second).await;
    let second_agent = scenario
        .create_agent(&second, &second_token, "agente-dois")
        .await;

    scenario.insert_open_case(&first, &first_agent, 500);
    scenario.insert_open_case(&first, &first_agent, 600);
    scenario.insert_closed_case(&first, &first_agent, 100, 1200);
    scenario.insert_conversation(&first, &first_agent, "open", 500);
    scenario.insert_conversation(&first, &first_agent, "transferred", 500);
    scenario.insert_conversation(&first, &first_agent, "closed", 500);

    scenario.insert_closed_case(&second, &second_agent, 100, 1300);
    scenario.insert_conversation(&second, &second_agent, "closed", 500);

    let (status, body) = scenario
        .get(
            &format!("{}?from=1000&to=2000", summary_path(&first)),
            &first_token,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "first summary: {body}");
    assert_eq!(as_integer(&body, "open_fichas"), 2);
    assert_eq!(as_integer(&body, "closed_fichas"), 1);
    assert_eq!(
        as_integer(&body, "open_conversations"),
        2,
        "a transferred conversation is still being handled; a closed one is out: {body}"
    );

    let (status, body) = scenario
        .get(
            &format!("{}?from=1000&to=2000", summary_path(&second)),
            &second_token,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "second summary: {body}");
    assert_eq!(as_integer(&body, "open_fichas"), 0);
    assert_eq!(as_integer(&body, "closed_fichas"), 1);
    assert_eq!(as_integer(&body, "open_conversations"), 0);
}

#[tokio::test]
async fn an_invalid_window_answers_422_with_the_field() {
    let scenario = Scenario::new("invalid").await;
    let org = scenario.create_org("invalida").await;
    let token = scenario.create_token(&org).await;

    for (query, field, reason) in [
        ("?from=2000&to=1000", "from", "from must be before to."),
        ("?from=1000&to=1000", "from", "from must be before to."),
        ("?to=1000", "from", "Field required"),
        ("?from=1000", "to", "Field required"),
        (
            "?from=abc&to=1000",
            "from",
            "Input should be a valid integer",
        ),
        (
            "?from=1000&to=2000.5",
            "to",
            "Input should be a valid integer",
        ),
    ] {
        let (status, body) = scenario
            .get(&format!("{}{query}", summary_path(&org)), &token)
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}: {body}");
        assert_eq!(body["code"], "validation_error", "{query}: {body}");
        assert_eq!(body["details"]["field"], field, "{query}: {body}");
        assert_eq!(body["details"]["reason"], reason, "{query}: {body}");
    }
}

#[tokio::test]
async fn isolation_and_authentication() {
    let scenario = Scenario::new("isolation").await;
    let org = scenario.create_org("isolamento").await;
    let token = scenario.create_token(&org).await;
    let other = scenario.create_org("outra").await;
    let other_token = scenario.create_token(&other).await;

    let path = format!("{}?from=0&to=1000", summary_path(&org));

    let (status, body) = scenario.get(&path, "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "no token: {body}");

    let (status, body) = scenario.get(&path, &other_token).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "token from another org: {body}"
    );

    let missing = uuid::Uuid::now_v7().to_string();
    let (status, body) = scenario
        .get(
            &format!("{}?from=0&to=1000", summary_path(&missing)),
            GLOBAL_TOKEN,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "org that does not exist, with a global token: {body}"
    );

    let (status, body) = scenario.get(&path, &token).await;
    assert_eq!(status, StatusCode::OK, "own org reads: {body}");
}
