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
}
impl Drop for Scenario {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Scenario {
    fn insert_turn(
        &self,
        org: &str,
        agent: Option<&str>,
        conversation_agent: &str,
        timestamp: i64,
        input: i64,
        output: i64,
    ) {
        let conn = self.connection();
        let conversation = uuid::Uuid::now_v7().to_string();
        let placeholder = conversation_agent;
        conn.execute("INSERT INTO conversations (id,org_id,agent_id,channel,external_peer,status,version,created_at,updated_at) VALUES (?1,?2,?3,'whatsapp','teste','open',1,?4,?4)", rusqlite::params![conversation,org,placeholder,timestamp]).expect("conversation");
        conn.execute("INSERT INTO turns (id,org_id,conversation_id,agent_id,input_tokens,output_tokens,cost_micros,started_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)", rusqlite::params![uuid::Uuid::now_v7().to_string(),org,conversation,agent,input,output,2,timestamp]).expect("turn");
    }
}
fn usage_path(org: &str) -> String {
    format!("/admin/v1/orgs/{org}/usage/tokens")
}
#[tokio::test]
async fn aggregate_and_isolate() {
    let scenario = Scenario::new("usage").await;
    let org = scenario.create_org("primeira").await;
    let token = scenario.create_token(&org).await;
    let agent = scenario.create_agent(&org, &token, "agente").await;
    let other = scenario.create_org("segunda").await;
    let other_token = scenario.create_token(&other).await;
    let other_agent = scenario.create_agent(&other, &other_token, "agente").await;
    let path = format!("{}?from=0&to=86400001", usage_path(&org));
    let (status, empty) = scenario.get(&path, &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        empty,
        json!({"total_input_tokens":0,"total_output_tokens":0,"total_cost_micros":0,"turn_count":0,"by_agent":[],"by_day":[]})
    );
    scenario.insert_turn(&org, Some(&agent), &agent, -1, 90, 90);
    scenario.insert_turn(&org, Some(&agent), &agent, 0, 3, 4);
    scenario.insert_turn(&org, None, &agent, 86399999, 5, 6);
    scenario.insert_turn(&org, Some(&agent), &agent, 86400000, 7, 8);
    scenario.insert_turn(&org, Some(&agent), &agent, 86400001, 1, 2);
    scenario.insert_turn(&org, Some(&agent), &agent, 86400002, 90, 90);
    scenario.insert_turn(&other, Some(&other_agent), &other_agent, 0, 90, 90);
    let (status, body) = scenario.get(&path, &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total_input_tokens"], 16);
    assert_eq!(body["total_output_tokens"], 20);
    assert_eq!(body["total_cost_micros"], 8);
    assert_eq!(body["turn_count"], 4);
    assert_eq!(
        body["by_agent"],
        json!([{"agent_id":agent,"input_tokens":11,"output_tokens":14,"turn_count":3},{"agent_id":null,"input_tokens":5,"output_tokens":6,"turn_count":1}])
    );
    assert_eq!(
        body["by_day"],
        json!([{"date":"1970-01-01","output_tokens":10,"turn_count":2},{"date":"1970-01-02","output_tokens":10,"turn_count":2}])
    );
    let (_, second) = scenario
        .get(&format!("{}?from=0&to=1", usage_path(&other)), &other_token)
        .await;
    assert_eq!(second["turn_count"], 1);
    assert_eq!(second["total_output_tokens"], 90);
    assert_eq!(scenario.get(&path, "").await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        scenario.get(&path, &other_token).await.0,
        StatusCode::FORBIDDEN
    );
    let missing = uuid::Uuid::now_v7().to_string();
    assert_eq!(
        scenario
            .get(
                &format!("{}?from=0&to=1", usage_path(&missing)),
                GLOBAL_TOKEN
            )
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    for (q, field, reason) in [
        ("?from=1&to=1", "from", "from must be before to."),
        ("?to=1", "from", "Field required"),
        ("?from=x&to=1", "from", "Input should be a valid integer"),
        ("?from=0", "to", "Field required"),
    ] {
        let (status, err) = scenario
            .get(&format!("{}{q}", usage_path(&org)), &token)
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
        assert_eq!(err["details"]["field"], field);
        assert_eq!(err["details"]["reason"], reason);
    }
}
