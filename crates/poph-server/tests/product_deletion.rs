use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::ServerState;
use poph_store::{NewConversation, Store};
use poph_types::config::GlobalTokenConfig;
use poph_types::{AgentId, HarnessConfig, OrgId, ProductId};
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-hr42f";

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
            "poph-produto-exclusao-{label}-{}-{stamp}",
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
        body["id"].as_str().expect("id da org").to_owned()
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

    async fn create_product(&self, org_id: &str, nome: &str, cents: i64) -> String {
        let (status, category) = self
            .send_request(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/categories"),
                TOKEN_GLOBAL,
                Some(json!({ "name": format!("Cat {nome}") })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar categoria: {category}");
        let category_id = category["id"].as_str().expect("id da categoria");
        let (status, body) = self
            .send_request(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/products"),
                TOKEN_GLOBAL,
                Some(json!({ "name": nome, "price_cents": cents, "category_id": category_id })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar produto: {body}");
        body["id"].as_str().expect("id do produto").to_owned()
    }

    async fn create_agent(&self, org_id: &str) -> String {
        let (status, body) = self
            .send_request(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/agents"),
                TOKEN_GLOBAL,
                Some(json!({ "name": "Agente", "slug": "agente" })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar agente: {body}");
        body["id"].as_str().expect("id do agente").to_owned()
    }

    fn add_to_cart(&self, org_id: &str, agent_id: &str, product_id: &str, quantity: u32) {
        let org: OrgId = org_id.parse().expect("org");
        let agente: AgentId = agent_id.parse().expect("agente");
        let product: ProductId = product_id.parse().expect("produto");
        let conversa = self
            .store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("abre a conversa")
            .id;
        self.store
            .carts()
            .add(org, conversa, agente, product, quantity)
            .expect("põe o produto no carrinho");
    }

    fn cart_rows(&self, org_id: &str, agent_id: &str) -> Vec<(Option<String>, String, i64)> {
        let org: OrgId = org_id.parse().expect("org");
        let agente: AgentId = agent_id.parse().expect("agente");
        let mut rows = Vec::new();
        let conversas = self
            .store
            .conversations()
            .list(
                org,
                None,
                poph_store::Page {
                    after: None,
                    limit: 200,
                },
            )
            .expect("lista conversas");
        for conversa in conversas {
            if conversa.agent_id != agente {
                continue;
            }
            let summary = self
                .store
                .carts()
                .summary(org, conversa.id)
                .expect("resumo do carrinho");
            for item in summary.items {
                rows.push((
                    item.product.map(|p| p.to_string()),
                    item.nome,
                    item.price_unit_cents,
                ));
            }
        }
        rows
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

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or("")
}

#[tokio::test]
async fn product_in_cart_without_force_answers_409_and_keeps_it() {
    let c = Scenario::new("sem-force").await;
    let org = c.create_org("Org Sem Force", "org-sem-force").await;
    let product = c.create_product(&org, "Dedetização", 18000).await;
    let agente = c.create_agent(&org).await;
    c.add_to_cart(&org, &agente, &product, 2);
    c.add_to_cart(&org, &agente, &product, 1);
    let path = format!("/admin/v1/orgs/{org}/products/{product}");

    let (status, response) = c
        .send_request(Method::DELETE, &path, TOKEN_GLOBAL, None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{response}");
    assert_eq!(code(&response), "conflict");
    assert_eq!(response["details"]["itens_de_carrinho"], 2, "{response}");
    assert_eq!(response["details"]["carrinhos"], 2, "{response}");
    assert!(
        response["message"].as_str().unwrap().contains("force=true"),
        "{response}"
    );

    let (status, _) = c.send_request(Method::GET, &path, TOKEN_GLOBAL, None).await;
    assert_eq!(status, StatusCode::OK, "o produto não pode sumir sem force");
    let rows = c.cart_rows(&org, &agente);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(
        rows.iter()
            .all(|(product_row, nome, _)| product_row.is_some() && nome == "Dedetização")
    );
}

#[tokio::test]
async fn with_force_deletes_and_item_keeps_stored_price() {
    let c = Scenario::new("com-force").await;
    let org = c.create_org("Org Com Force", "org-com-force").await;
    let product = c.create_product(&org, "Visita técnica", 18000).await;
    let agente = c.create_agent(&org).await;
    c.add_to_cart(&org, &agente, &product, 1);
    let path = format!("/admin/v1/orgs/{org}/products/{product}");

    let (status, response) = c
        .send_request(
            Method::DELETE,
            &format!("{path}?force=true"),
            TOKEN_GLOBAL,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["deleted"], true);

    let (status, _) = c.send_request(Method::GET, &path, TOKEN_GLOBAL, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let rows = c.cart_rows(&org, &agente);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let (product_row, nome, price) = &rows[0];
    assert!(product_row.is_none(), "{rows:?}");
    assert_eq!(nome, "produto removido do catálogo");
    assert_eq!(*price, 18000);
}

#[tokio::test]
async fn product_without_cart_deletes_without_force() {
    let c = Scenario::new("sem-vinculo").await;
    let org = c.create_org("Org Livre", "org-livre").await;
    let product = c.create_product(&org, "Sem uso", 5000).await;
    let path = format!("/admin/v1/orgs/{org}/products/{product}");

    let (status, response) = c
        .send_request(Method::DELETE, &path, TOKEN_GLOBAL, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let (status, _) = c.send_request(Method::GET, &path, TOKEN_GLOBAL, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn token_from_other_org_does_not_delete_and_missing_product_answers_404() {
    let c = Scenario::new("isolamento").await;
    let org_a = c.create_org("Org A", "org-a-produto").await;
    let org_b = c.create_org("Org B", "org-b-produto").await;
    let token_a = c.create_token(&org_a).await;
    let product_b = c.create_product(&org_b, "Produto de B", 7000).await;
    let agent_b = c.create_agent(&org_b).await;
    c.add_to_cart(&org_b, &agent_b, &product_b, 1);

    for suffix in ["", "?force=true"] {
        let (status, _) = c
            .send_request(
                Method::DELETE,
                &format!("/admin/v1/orgs/{org_b}/products/{product_b}{suffix}"),
                &token_a,
                None,
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "sufixo {suffix:?}");
    }
    let (status, _) = c
        .send_request(
            Method::GET,
            &format!("/admin/v1/orgs/{org_b}/products/{product_b}"),
            TOKEN_GLOBAL,
            None,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "o produto da outra org não pode sumir"
    );

    let missing = ProductId::generate();
    let (status, response) = c
        .send_request(
            Method::DELETE,
            &format!("/admin/v1/orgs/{org_b}/products/{missing}"),
            TOKEN_GLOBAL,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{response}");
}
