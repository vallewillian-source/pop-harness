use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::ServerState;
use poph_server::cluster::{LiveCluster, load};
use poph_store::Store;
use poph_types::HarnessConfig;
use poph_types::Secret;
use poph_types::config::GlobalTokenConfig;
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-hr50a";
const SECRET: &str = "SENTINELA-SEGREDO-CLUSTER-hr50a";

fn hash_argon2(value: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

const MAP_ORIGINAL: &str = r#"
instance_id = "a"
default_instance = "a"

[[instances]]
id = "a"
url = "http://127.0.0.1:7481"

[[instances]]
id = "b"
url = "http://127.0.0.1:7482"

[orgs]
org_01 = "a"
org_02 = "b"
"#;

const MAP_SWAPPED: &str = r#"
instance_id = "a"
default_instance = "a"

[[instances]]
id = "a"
url = "http://127.0.0.1:7481"

[[instances]]
id = "b"
url = "http://127.0.0.1:7482"

[orgs]
org_01 = "b"
org_02 = "b"
"#;

const MAP_INVALID: &str = r#"
instance_id = "a"
default_instance = "b"

[[instances]]
id = "a"
url = "http://127.0.0.1:7481"

[[instances]]
id = "b"
url = "http://127.0.0.1:7482"

[orgs]
org_01 = "c"
"#;

struct Response {
    status: StatusCode,
    body: Value,
    raw: String,
}

struct Scenario {
    base: String,
    client: Client,
    folder: PathBuf,
    cluster_path: PathBuf,
    desligar: Option<oneshot::Sender<()>>,
}

impl Scenario {
    async fn new(label: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let folder = std::env::temp_dir().join(format!(
            "poph-cluster-reload-{label}-{}-{timestamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(folder.join("dados")).expect("pasta de dados");
        let cluster_path = folder.join("cluster.toml");
        fs::write(&cluster_path, MAP_ORIGINAL).expect("cluster.toml");

        let mut config = HarnessConfig::default();
        config.data.directory = folder.join("dados");
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let secret = Secret::new(SECRET.to_owned());
        let map = load(&cluster_path, Some(&secret)).expect("mapa inicial válido");
        let estado = ServerState::new(Arc::clone(&store), config)
            .expect("estado do servidor")
            .with_cluster(LiveCluster {
                path: cluster_path.clone(),
                config: map,
                secret: Some(secret),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, signal) = oneshot::channel();
        tokio::spawn(async move {
            let _ = poph_server::serve_at(listener, estado, async {
                let _ = signal.await;
            })
            .await;
        });
        let scenario = Self {
            base: format!("http://{endereco}"),
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("cliente HTTP"),
            folder,
            cluster_path,
            desligar: Some(desligar),
        };
        scenario.wait_boot().await;
        scenario
    }

    async fn wait_boot(&self) {
        for _ in 0..500 {
            if let Ok(r) = self
                .client
                .get(format!("{}/healthz", self.base))
                .send()
                .await
                && r.status().is_success()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("servidor não subiu");
    }

    fn write_map(&self, texto: &str) {
        fs::write(&self.cluster_path, texto).expect("escreve cluster.toml");
    }

    async fn reload(&self, token: &str) -> Response {
        self.ask(Method::POST, "/admin/v1/cluster/reload", token, None)
            .await
    }

    async fn ask(&self, method: Method, path: &str, token: &str, body: Option<Value>) -> Response {
        let mut request = self.client.request(method, format!("{}{path}", self.base));
        if !token.is_empty() {
            request = request.bearer_auth(token);
        }
        if let Some(value) = body {
            request = request.json(&value);
        }
        let response = request.send().await.expect("pedido HTTP");
        let status = response.status();
        let raw = response.text().await.unwrap_or_default();
        let body = serde_json::from_str(&raw).unwrap_or(Value::Null);
        Response { status, body, raw }
    }

    async fn org_token(&self) -> String {
        let org = self
            .ask(
                Method::POST,
                "/admin/v1/orgs",
                TOKEN_GLOBAL,
                Some(json!({"name": "Org do cluster", "slug": "org-do-cluster"})),
            )
            .await;
        assert_eq!(org.status, StatusCode::CREATED, "{}", org.raw);
        let id = org.body["id"].as_str().expect("id da org");
        let token = self
            .ask(
                Method::POST,
                &format!("/admin/v1/orgs/{id}/tokens"),
                TOKEN_GLOBAL,
                Some(json!({"label": "org"})),
            )
            .await;
        assert_eq!(token.status, StatusCode::CREATED, "{}", token.raw);
        token.body["value"]
            .as_str()
            .expect("valor do token")
            .to_owned()
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        let _ = fs::remove_dir_all(&self.folder);
    }
}

#[tokio::test]
async fn reload_reads_the_live_process_file_and_reports_what_changed() {
    let c = Scenario::new("troca").await;

    let first = c.reload(TOKEN_GLOBAL).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.raw);
    assert_eq!(first.body["mudou"], json!(false));
    assert_eq!(
        first.body["fingerprint_antes"],
        first.body["fingerprint_depois"]
    );
    assert_eq!(first.body["depois"]["modo"], json!("cluster"));
    assert_eq!(first.body["depois"]["instance_id"], json!("a"));
    assert_eq!(first.body["depois"]["segredo_configurado"], json!(true));
    let fingerprint_original = first.body["fingerprint_depois"]
        .as_str()
        .expect("fingerprint")
        .to_owned();

    c.write_map(MAP_SWAPPED);
    let second = c.reload(TOKEN_GLOBAL).await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.raw);
    assert_eq!(second.body["mudou"], json!(true));
    assert_eq!(
        second.body["fingerprint_antes"],
        json!(fingerprint_original),
        "a impressão anterior é a do mapa que estava em uso"
    );
    assert_ne!(
        second.body["fingerprint_depois"],
        json!(fingerprint_original)
    );
    let trocas = second.body["orgs_que_trocaram"].as_array().expect("trocas");
    assert_eq!(trocas.len(), 1, "só org_01 trocou: {trocas:?}");
    assert_eq!(trocas[0]["org"], json!("org_01"));
    assert_eq!(trocas[0]["de"], json!("a"));
    assert_eq!(trocas[0]["para"], json!("b"));
    let fingerprint_changed = second.body["fingerprint_depois"]
        .as_str()
        .expect("fingerprint")
        .to_owned();

    c.write_map(MAP_INVALID);
    let invalid = c.reload(TOKEN_GLOBAL).await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST, "{}", invalid.raw);
    assert_eq!(invalid.body["code"], json!("cluster_invalido"));
    let problems = invalid.body["details"]["problemas"]
        .as_array()
        .expect("problemas");
    assert!(
        problems
            .iter()
            .any(|p| p.as_str().is_some_and(|p| p.contains("orgs.org_01"))),
        "o problema aponta a chave: {problems:?}"
    );

    c.write_map(MAP_SWAPPED);
    let restore = c.reload(TOKEN_GLOBAL).await;
    assert_eq!(restore.status, StatusCode::OK, "{}", restore.raw);
    assert_eq!(
        restore.body["fingerprint_antes"],
        json!(fingerprint_changed),
        "o arquivo inválido não trocou o mapa em uso"
    );
    assert_eq!(restore.body["mudou"], json!(false));

    fs::remove_file(&c.cluster_path).expect("remove cluster.toml");
    let no_file = c.reload(TOKEN_GLOBAL).await;
    assert_eq!(no_file.status, StatusCode::OK, "{}", no_file.raw);
    assert_eq!(no_file.body["depois"]["modo"], json!("instancia_unica"));
    assert_eq!(no_file.body["mudou"], json!(true));

    for body in [first.raw, second.raw, invalid.raw, restore.raw, no_file.raw] {
        assert!(
            !body.contains(SECRET),
            "segredo do cluster vazou na resposta: {body}"
        );
    }
}

#[tokio::test]
async fn reload_requires_global_token() {
    let c = Scenario::new("autorizacao").await;

    let no_token = c.reload("").await;
    assert_eq!(
        no_token.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        no_token.raw
    );

    let of_org = c.org_token().await;
    let barrada = c.reload(&of_org).await;
    assert_eq!(barrada.status, StatusCode::FORBIDDEN, "{}", barrada.raw);
    assert_eq!(barrada.body["code"], json!("forbidden"));
    assert!(!barrada.raw.contains(SECRET));
}
