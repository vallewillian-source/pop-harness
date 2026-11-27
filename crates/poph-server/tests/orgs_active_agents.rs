use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::ServerState;
use poph_store::{AgentChange, Status, Store};
use poph_types::HarnessConfig;
use poph_types::config::GlobalTokenConfig;
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-cf03a";

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
    folder: PathBuf,
    store: std::sync::Arc<Store>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Scenario {
    async fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let folder = std::env::temp_dir().join(format!(
            "poph-orgs-ativos-{label}-{}-{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(folder.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = folder.join("dados");
        config.server.port = 7480;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = std::sync::Arc::new(Store::open(&config).expect("abre o banco"));
        let estado =
            ServerState::new(std::sync::Arc::clone(&store), config).expect("estado do servidor");

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
                .timeout(Duration::from_secs(20))
                .build()
                .expect("cliente HTTP"),
            folder,
            store,
            desligar: Some(desligar),
        };
        scenario.wait_until_up().await;
        scenario
    }

    async fn wait_until_up(&self) {
        let started = Instant::now();
        loop {
            match self
                .client
                .get(format!("{}/healthz", self.base))
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => return,
                _ if started.elapsed() < Duration::from_secs(10) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                other => panic!("servidor não subiu: {other:?}"),
            }
        }
    }

    async fn send_request(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = self.client.request(method, format!("{}{path}", self.base));
        if !token.is_empty() {
            request = request.bearer_auth(token);
        }
        if let Some(value) = body {
            request = request.json(&value);
        }
        let response = request.send().await.expect("pedido HTTP");
        let status = response.status();
        let texto = response.text().await.unwrap_or_default();
        let json = if texto.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&texto).unwrap_or(Value::String(texto))
        };
        (status, json)
    }

    async fn create_org(&self, nome: &str, slug: &str) -> String {
        let (status, body) = self
            .send_request(
                Method::POST,
                "/admin/v1/orgs",
                TOKEN_GLOBAL,
                Some(json!({ "name": nome, "slug": slug })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar org: {body}");
        assert_eq!(
            body["agents_active_count"],
            json!(0),
            "org recém-criada não tem agente: {body}"
        );
        body["id"].as_str().expect("id da org").to_owned()
    }

    async fn create_agent(&self, org_id: &str, slug: &str) -> String {
        let (status, body) = self
            .send_request(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/agents"),
                TOKEN_GLOBAL,
                Some(json!({ "name": format!("Agente {slug}"), "slug": slug })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar agente: {body}");
        body["id"].as_str().expect("id do agente").to_owned()
    }

    async fn create_token(&self, org_id: &str) -> String {
        let (status, body) = self
            .send_request(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/tokens"),
                TOKEN_GLOBAL,
                Some(json!({ "label": "org" })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar token: {body}");
        body["value"].as_str().expect("valor do token").to_owned()
    }

    async fn listar(&self, token: &str, query: &str) -> Value {
        let (status, body) = self
            .send_request(Method::GET, &format!("/admin/v1/orgs{query}"), token, None)
            .await;
        assert_eq!(status, StatusCode::OK, "listar orgs: {body}");
        body
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

fn active(item: &Value, org_id: &str) -> u32 {
    assert_eq!(item["id"], json!(org_id), "item de outra org: {item}");
    let value = item
        .get("agents_active_count")
        .unwrap_or_else(|| panic!("o item tem de trazer `agents_active_count` sempre: {item}"));
    assert!(
        value.is_u64(),
        "`agents_active_count` tem de ser número inteiro, não `null`: {item}"
    );
    u32::try_from(value.as_u64().unwrap_or(0)).expect("cabe em u32")
}

fn item(items: &[Value], org_id: &str) -> Value {
    items
        .iter()
        .find(|item| item["id"] == json!(org_id))
        .unwrap_or_else(|| panic!("org {org_id} fora da listagem: {items:?}"))
        .clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn listing_shows_active_agents_per_org() {
    let scenario = Scenario::new("contagem").await;
    let with_agents = scenario.create_org("Com agentes", "com-agentes").await;
    let without_agents = scenario.create_org("Sem agentes", "sem-agentes").await;

    for slug in ["agente-um", "agente-dois", "agente-tres"] {
        scenario.create_agent(&with_agents, slug).await;
    }
    let inactive = scenario.create_agent(&with_agents, "agente-quatro").await;
    let org: poph_types::OrgId = with_agents.parse().expect("org");
    let agente: poph_types::AgentId = inactive.parse().expect("agente");
    let current = scenario
        .store
        .agents()
        .get(org, agente)
        .expect("lê o agente");
    scenario
        .store
        .agents()
        .update(
            org,
            agente,
            current.version,
            AgentChange {
                status: Some(Status::Inactive),
                ..AgentChange::default()
            },
        )
        .expect("inativa o agente");

    let body = scenario.listar(TOKEN_GLOBAL, "?limit=200").await;
    let items = body["items"].as_array().expect("items");
    assert_eq!(
        active(&item(items, &with_agents), &with_agents),
        3,
        "só os três ativos entram na conta: {body}"
    );
    assert_eq!(
        active(&item(items, &without_agents), &without_agents),
        0,
        "org sem agente ativo devolve 0, e não some da lista: {body}"
    );

    let (status, one) = scenario
        .send_request(
            Method::GET,
            &format!("/admin/v1/orgs/{with_agents}"),
            TOKEN_GLOBAL,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "mostrar org: {one}");
    assert_eq!(active(&one, &with_agents), 3, "{one}");

    let token_org = scenario.create_token(&with_agents).await;
    let own = scenario.listar(&token_org, "").await;
    let items = own["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{own}");
    assert_eq!(active(&items[0], &with_agents), 3, "{own}");
}

#[tokio::test(flavor = "multi_thread")]
async fn cursor_pagination_stays_the_same() {
    let scenario = Scenario::new("paginacao").await;
    let mut created = Vec::new();
    for index in 0..3 {
        let org = scenario
            .create_org(&format!("Org {index}"), &format!("org-{index}"))
            .await;
        if index < 2 {
            scenario.create_agent(&org, "agente-a").await;
        }
        created.push(org);
    }

    let first = scenario.listar(TOKEN_GLOBAL, "?limit=2").await;
    let items = first["items"].as_array().expect("items");
    assert_eq!(items.len(), 2, "{first}");
    let cursor = first["next_cursor"]
        .as_str()
        .unwrap_or_else(|| panic!("a primeira página tem próxima: {first}"))
        .to_owned();
    for item in items {
        let id = item["id"].as_str().unwrap_or_default();
        assert!(item.get("agents_active_count").is_some(), "{item}");
        let expected = if id == created[2] { 0 } else { 1 };
        assert_eq!(active(item, id), expected, "página 1: {first}");
    }

    let second = scenario
        .listar(TOKEN_GLOBAL, &format!("?limit=2&cursor={cursor}"))
        .await;
    let items = second["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{second}");
    assert!(second["next_cursor"].is_null(), "{second}");
    assert_eq!(active(&items[0], &created[2]), 0, "{second}");
}

#[tokio::test(flavor = "multi_thread")]
async fn page_makes_no_per_row_query() {
    let scenario = Scenario::new("sem-n-mais-um").await;
    for index in 0..12 {
        let org = scenario
            .create_org(&format!("Org {index}"), &format!("org-{index}"))
            .await;
        scenario.create_agent(&org, "agente-a").await;
    }

    let before = scenario.store.reads_done();
    let small = scenario.listar(TOKEN_GLOBAL, "?limit=1").await;
    let reads_small = scenario.store.reads_done() - before;

    let before = scenario.store.reads_done();
    let large = scenario.listar(TOKEN_GLOBAL, "?limit=200").await;
    let reads_large = scenario.store.reads_done() - before;

    assert_eq!(small["items"].as_array().map(Vec::len), Some(1), "{small}");
    assert_eq!(large["items"].as_array().map(Vec::len), Some(12), "{large}");
    assert!(
        reads_large < 12,
        "a página de 12 orgs custou {reads_large} leituras: isso é uma consulta por linha          (a de 1 org custou {reads_small})"
    );
}
