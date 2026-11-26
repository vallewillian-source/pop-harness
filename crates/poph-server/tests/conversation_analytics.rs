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

        let store = Arc::new(Store::abrir(&config).expect("opens the database"));
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
            rusqlite::Connection::open(self.store.caminho_harness()).expect("opens the test db");
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

fn path(org: &str) -> String {
    format!("/admin/v1/orgs/{org}/analytics/conversations")
}

#[tokio::test]
async fn aggregate_filter_and_isolate() {
    let scenario = Scenario::new("conversation-analytics").await;
    let org = scenario.create_org("primeira").await;
    let token = scenario.create_token(&org).await;
    let agent = scenario.create_agent(&org, &token, "agente").await;
    let other = scenario.create_org("segunda").await;
    let other_token = scenario.create_token(&other).await;
    let other_agent = scenario.create_agent(&other, &other_token, "agente").await;
    let base = path(&org);
    let (status, empty) = scenario
        .get(&format!("{base}?from=0&to=86400000"), &token)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        empty["summary"],
        json!({"total":0,"open":0,"transferred":0,"closed":0,"avg_messages_per_conversation":null,"conversation_to_ficha_rate":null})
    );
    let conn = scenario.connection();
    for (id, owner, who, channel, status, created) in [
        ("a", org.as_str(), agent.as_str(), "whatsapp", "open", 1),
        (
            "b",
            org.as_str(),
            agent.as_str(),
            "whatsapp",
            "transferred",
            86400000,
        ),
        ("c", org.as_str(), agent.as_str(), "web", "closed", 100),
        ("d", org.as_str(), agent.as_str(), "web", "open", 86400000),
        (
            "e",
            other.as_str(),
            other_agent.as_str(),
            "web",
            "open",
            100,
        ),
    ] {
        conn.execute("INSERT INTO conversations (id,org_id,agent_id,channel,status,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?6)", rusqlite::params![id,owner,who,channel,status,created]).expect("conversation");
    }
    for (id, conversation, created) in
        [("m1", "a", 0), ("m2", "a", 90000000), ("m3", "b", 86399999)]
    {
        conn.execute("INSERT INTO messages (id,org_id,conversation_id,direction,role,created_at) VALUES (?1,?2,?3,'in','user',?4)", rusqlite::params![id,org,conversation,created]).expect("message");
    }
    conn.execute("INSERT INTO fichas(id,org_id,conversation_id,agent_id,canal,status,aberta_em) VALUES ('f',?1,'a',?2,'whatsapp','open',0)",rusqlite::params![org,agent]).expect("ficha");
    let (_, body) = scenario
        .get(
            &format!("{base}?from=0&to=86400000&tz_offset_minutes=-180"),
            &token,
        )
        .await;
    assert_eq!(
        body["summary"],
        json!({"total":3,"open":1,"transferred":1,"closed":1,"avg_messages_per_conversation":1.5,"conversation_to_ficha_rate":1.0/3.0})
    );
    assert_eq!(body["by_day"][0]["date"], "1969-12-31");
    assert_eq!(body["by_agent"][0]["agent_id"], agent);
    assert_eq!(
        body["by_channel"][0],
        json!({"channel":"whatsapp","total":2})
    );
    let (_, filtered) = scenario
        .get(&format!("{base}?from=0&to=86400000&channel=web"), &token)
        .await;
    assert_eq!(filtered["summary"]["total"], 1);
    assert_eq!(
        filtered["summary"]["avg_messages_per_conversation"],
        Value::Null
    );
    let (_, filtered) = scenario
        .get(
            &format!("{base}?from=0&to=86400000&agent_id=missing"),
            &token,
        )
        .await;
    assert_eq!(filtered["summary"]["total"], 0);
    assert_eq!(
        scenario.get(&format!("{base}?from=0&to=1"), "").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        scenario
            .get(&format!("{base}?from=0&to=1"), &other_token)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let missing = uuid::Uuid::now_v7().to_string();
    assert_eq!(
        scenario
            .get(&format!("{}?from=0&to=1", path(&missing)), GLOBAL_TOKEN)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    for (query, field) in [
        ("?to=1", "from"),
        ("?from=x&to=1", "from"),
        ("?from=1&to=1", "from"),
        ("?from=0&to=1&tz_offset_minutes=841", "tz_offset_minutes"),
    ] {
        let (status, error) = scenario.get(&format!("{base}{query}"), &token).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error["details"]["field"], field);
    }
}
