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

    #[allow(dead_code)]
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

#[tokio::test]
async fn sla_boundaries_order_and_authorization() {
    let scenario = Scenario::new("sla").await;
    let org = scenario.create_org("primeira").await;
    let token = scenario.create_token(&org).await;
    let other = scenario.create_org("segunda").await;
    let other_token = scenario.create_token(&other).await;
    let conn = scenario.connection();
    for (id, owner, opened) in [
        ("a", org.as_str(), 40000),
        ("b", org.as_str(), -20000),
        ("c", org.as_str(), -80000),
        ("d", org.as_str(), 110000),
        ("x", other.as_str(), -80000),
    ] {
        conn.execute("INSERT INTO fichas(id,org_id,agent_id,canal,status,aberta_em) VALUES (?1,?2,'agent','whatsapp','open',?3)",rusqlite::params![id,owner,opened]).expect("ficha");
    }
    conn.execute("INSERT INTO ficha_eventos(id,org_id,ficha_id,seq,tipo,ator_tipo,created_at) VALUES ('e',?1,'c',1,'ficha_aberta','sistema',70000)",rusqlite::params![org]).expect("event");
    conn.execute("INSERT INTO fluxo_execucoes(id,org_id,ficha_id,fluxo_id,revisao_id,status,no_atual,iniciada_em) VALUES ('r',?1,'a','flow','revision','ativa','review',0)",rusqlite::params![org]).expect("run");
    let base = format!("/admin/v1/orgs/{org}/analytics/sla");
    let query = "?to=200000&now=100000&limit=3&age_bounds_minutes=1,2,3";
    let (status, body) = scenario.get(&format!("{base}{query}"), &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["age_buckets"],
        json!([{"min_minutes":0,"max_minutes":1,"count":1},{"min_minutes":1,"max_minutes":2,"count":1},{"min_minutes":2,"max_minutes":3,"count":1},{"min_minutes":3,"max_minutes":null,"count":1}])
    );
    assert_eq!(body["stale_items"][0]["ficha_id"], "b");
    assert_eq!(body["stale_items"][1]["ficha_id"], "a");
    assert_eq!(body["stale_items"][1]["current_step_key"], "review");
    assert_eq!(body["stale_items"][2]["idle_ms"], 30000);
    assert_eq!(body["stale_items"].as_array().expect("items").len(), 3);
    for (query, field) in [
        ("", "to"),
        ("?to=x", "to"),
        ("?to=1&limit=0", "limit"),
        ("?to=1&age_bounds_minutes=2,2", "age_bounds_minutes"),
        ("?to=1&age_bounds_minutes=0", "age_bounds_minutes"),
    ] {
        let (status, error) = scenario.get(&format!("{base}{query}"), &token).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error["details"]["field"], field);
    }
    assert_eq!(
        scenario.get(&format!("{base}?to=1"), "").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        scenario.get(&format!("{base}?to=1"), &other_token).await.0,
        StatusCode::FORBIDDEN
    );
    let missing = uuid::Uuid::now_v7();
    assert_eq!(
        scenario
            .get(
                &format!("/admin/v1/orgs/{missing}/analytics/sla?to=1"),
                GLOBAL_TOKEN
            )
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}
