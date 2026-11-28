use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::ServerState;
use poph_store::Store;
use poph_types::HarnessConfig;
use poph_types::config::GlobalTokenConfig;
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-hr20";

fn hash_argon2(value: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

struct Scenario {
    base: String,
    client: Client,
    directory: PathBuf,
    store: std::sync::Arc<Store>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Scenario {
    async fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let directory = std::env::temp_dir().join(format!(
            "poph-server-{label}-{}-{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(directory.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = directory.join("dados");
        if label == "knowledge-real-model" {
            config.data.models =
                Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/modelos"));
        }
        config.server.port = 7480;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = std::sync::Arc::new(Store::open(&config).expect("abre o banco"));
        let server_state =
            ServerState::new(std::sync::Arc::clone(&store), config).expect("estado do servidor");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let address = listener.local_addr().expect("endereço local");
        let (shutdown, signal) = oneshot::channel();
        tokio::spawn(async move {
            let _ = poph_server::serve_at(listener, server_state, async {
                let _ = signal.await;
            })
            .await;
        });

        let base = format!("http://{address}");
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("cliente HTTP");

        let scenario = Self {
            base,
            client,
            directory,
            store,
            shutdown: Some(shutdown),
        };
        scenario.wait_until_ready().await;
        scenario
    }

    async fn wait_until_ready(&self) {
        let started_at = Instant::now();
        loop {
            match self
                .client
                .get(format!("{}/healthz", self.base))
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => return,
                _ if started_at.elapsed() < Duration::from_secs(10) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                other => panic!("servidor não subiu: {other:?}"),
            }
        }
    }

    fn store(&self) -> &Store {
        &self.store
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Option<Value>,
        if_match: Option<u64>,
    ) -> (StatusCode, Value) {
        let mut request = self.client.request(method, self.url(path));
        if !token.is_empty() {
            request = request.bearer_auth(token);
        }
        if let Some(version) = if_match {
            request = request.header("If-Match", version.to_string());
        }
        if let Some(value) = body {
            request = request.json(&value);
        }
        let response = request.send().await.expect("pedido HTTP");
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        let json = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or(Value::String(text))
        };
        (status, json)
    }

    async fn upload_file(&self, path: &str, token: &str, bytes: &[u8]) -> (StatusCode, Value) {
        let response = self
            .client
            .post(self.url(path))
            .bearer_auth(token)
            .header("content-type", "application/octet-stream")
            .body(bytes.to_vec())
            .send()
            .await
            .expect("pedido HTTP");
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        let json = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or(Value::String(text))
        };
        (status, json)
    }

    async fn create_org(&self, display_name: &str, slug: &str) -> Value {
        let (status, body) = self
            .request(
                Method::POST,
                "/admin/v1/orgs",
                TOKEN_GLOBAL,
                Some(json!({ "name": display_name, "slug": slug })),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar org: {body}");
        body
    }

    async fn create_token(&self, org_id: &str, label: &str) -> String {
        let (status, body) = self
            .request(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/tokens"),
                TOKEN_GLOBAL,
                Some(json!({ "label": label })),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar token: {body}");
        let value = body["value"].as_str().expect("valor do token").to_owned();
        assert!(
            value.starts_with("poph_"),
            "valor fora do formato novo: {value}"
        );
        value
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or("")
}

#[tokio::test]
async fn library_lookup_is_scoped_to_org_and_agent() {
    let c = Scenario::new("biblioteca-api").await;
    let org_a = c.create_org("Org A", "org-a").await;
    let org_b = c.create_org("Org B", "org-b").await;
    let a = org_a["id"].as_str().unwrap();
    let b = org_b["id"].as_str().unwrap();
    let token = c.create_token(a, "org a").await;
    let agent = c
        .store()
        .agents()
        .create(
            b.parse().unwrap(),
            poph_store::NewAgent {
                name: "Agente".into(),
                slug: "agente".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let file = c
        .store()
        .library()
        .create(
            b.parse().unwrap(),
            poph_store::NewLibraryFile {
                agent_id: agent.id,
                name: "Manual".into(),
                description: "Instruções".into(),
                mime: "application/pdf".into(),
                size_bytes: 42,
                sha256: "a".repeat(64),
                blob_path: "privado".into(),
            },
        )
        .unwrap();
    let collection = format!("/admin/v1/orgs/{b}/agents/{}/files", agent.id);
    let item = format!("{collection}/{}", file.id);
    let content = format!("{item}/content");
    for route in [&collection, &item, &content] {
        assert_eq!(
            c.request(Method::GET, route, "", None, None).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            c.request(Method::GET, route, &token, None, None).await.0,
            StatusCode::FORBIDDEN
        );
    }
    let (status, list) = c
        .request(Method::GET, &collection, TOKEN_GLOBAL, None, None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["items"][0]["name"], "Manual");
    assert!(list["items"][0].get("blob_path").is_none());
    let (status, shown) = c
        .request(Method::GET, &item, TOKEN_GLOBAL, None, None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(shown["id"], file.id.to_string());
    let other = c
        .store()
        .agents()
        .create(
            b.parse().unwrap(),
            poph_store::NewAgent {
                name: "Outro".into(),
                slug: "outro".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let route_foreign = format!("/admin/v1/orgs/{b}/agents/{}/files/{}", other.id, file.id);
    assert_eq!(
        c.request(Method::GET, &route_foreign, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        c.request(Method::DELETE, &item, "", None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        c.request(Method::DELETE, &item, &token, None, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        c.request(Method::DELETE, &route_foreign, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let token_b = c.create_token(b, "org b").await;
    assert_eq!(
        c.request(Method::DELETE, &item, &token_b, None, None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        c.request(Method::GET, &item, &token_b, None, None).await.0,
        StatusCode::NOT_FOUND
    );
    let records = c
        .store()
        .audit()
        .list(b.parse().unwrap(), poph_store::Page::default())
        .unwrap();
    let audit = records
        .iter()
        .find(|r| r.entity_id.as_deref() == Some(&file.id.to_string()) && r.acao == "delete")
        .expect("exclusão auditada");
    assert_eq!(audit.ator_tipo, "admin_token");
    assert_eq!(
        audit.actor_id.as_deref(),
        token_b
            .strip_prefix("poph_")
            .and_then(|s| s.split_once('_'))
            .map(|(id, _)| id)
    );
    assert!(!format!("{audit:?}").contains(&token_b));
}

#[tokio::test]
async fn all_admin_routes_require_token() {
    let c = Scenario::new("sem-token").await;
    let org = "0197c9a0-0000-7000-8000-000000000000";
    let routes: &[(Method, &str, Option<Value>)] = &[
        (Method::GET, "/admin/v1/orgs", None),
        (Method::POST, "/admin/v1/orgs", Some(json!({"name": "x"}))),
        (Method::GET, &format!("/admin/v1/orgs/{org}").clone(), None),
        (
            Method::DELETE,
            &format!("/admin/v1/orgs/{org}").clone(),
            None,
        ),
        (
            Method::GET,
            &format!("/admin/v1/orgs/{org}/tokens").clone(),
            None,
        ),
        (
            Method::POST,
            &format!("/admin/v1/orgs/{org}/tokens").clone(),
            Some(json!({"label": "x"})),
        ),
        (
            Method::GET,
            &format!("/admin/v1/orgs/{org}/categories").clone(),
            None,
        ),
        (
            Method::POST,
            &format!("/admin/v1/orgs/{org}/categories").clone(),
            Some(json!({"name": "x"})),
        ),
        (
            Method::GET,
            &format!("/admin/v1/orgs/{org}/departments").clone(),
            None,
        ),
        (
            Method::POST,
            &format!("/admin/v1/orgs/{org}/departments").clone(),
            Some(json!({"name": "x"})),
        ),
    ];
    for (method, path, body) in routes {
        let (status, response) = c
            .request(method.clone(), path, "", body.clone(), None)
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
        assert_eq!(code(&response), "unauthorized", "{method} {path}");
    }

    for value in ["sem-prefixo", "poph_nao-e-uuid_segredo", "pophg_errado"] {
        let (status, response) = c
            .request(Method::GET, "/admin/v1/orgs", value, None, None)
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "token `{value}`");
        assert_eq!(code(&response), "unauthorized");
    }
}

#[tokio::test]
async fn org_token_cannot_access_other_org_on_any_route() {
    let c = Scenario::new("escopo").await;
    let org_a = c.create_org("Org A", "org-a").await;
    let org_b = c.create_org("Org B", "org-b").await;
    let id_a = org_a["id"].as_str().expect("id da org A").to_owned();
    let id_b = org_b["id"].as_str().expect("id da org B").to_owned();
    let token_a = c.create_token(&id_a, "token-da-a").await;

    let (status, category_b) = c
        .request(
            Method::POST,
            &format!("/admin/v1/orgs/{id_b}/categories"),
            TOKEN_GLOBAL,
            Some(json!({ "name": "Categoria B" })),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{category_b}");
    let category_b_id = category_b["id"]
        .as_str()
        .expect("id da categoria")
        .to_owned();
    let token_b_id = c.create_token(&id_b, "token-da-b").await;
    let token_b_id = token_b_id
        .strip_prefix("poph_")
        .and_then(|remaining| remaining.split_once('_'))
        .map(|(id, _)| id.to_owned())
        .expect("id no valor do token");

    let routes: Vec<(Method, String, Option<Value>, Option<u64>)> = vec![
        (Method::GET, format!("/admin/v1/orgs/{id_b}"), None, None),
        (
            Method::PATCH,
            format!("/admin/v1/orgs/{id_b}"),
            Some(json!({"name": "Invadida"})),
            Some(1),
        ),
        (Method::DELETE, format!("/admin/v1/orgs/{id_b}"), None, None),
        (
            Method::GET,
            format!("/admin/v1/orgs/{id_b}/tokens"),
            None,
            None,
        ),
        (
            Method::POST,
            format!("/admin/v1/orgs/{id_b}/tokens"),
            Some(json!({"label": "x"})),
            None,
        ),
        (
            Method::GET,
            format!("/admin/v1/orgs/{id_b}/tokens/{token_b_id}"),
            None,
            None,
        ),
        (
            Method::DELETE,
            format!("/admin/v1/orgs/{id_b}/tokens/{token_b_id}"),
            None,
            None,
        ),
        (
            Method::GET,
            format!("/admin/v1/orgs/{id_b}/categories"),
            None,
            None,
        ),
        (
            Method::POST,
            format!("/admin/v1/orgs/{id_b}/categories"),
            Some(json!({"name": "x"})),
            None,
        ),
        (
            Method::GET,
            format!("/admin/v1/orgs/{id_b}/categories/{category_b_id}"),
            None,
            None,
        ),
        (
            Method::PATCH,
            format!("/admin/v1/orgs/{id_b}/categories/{category_b_id}"),
            Some(json!({"name": "x"})),
            Some(1),
        ),
        (
            Method::DELETE,
            format!("/admin/v1/orgs/{id_b}/categories/{category_b_id}"),
            None,
            None,
        ),
        (
            Method::GET,
            format!("/admin/v1/orgs/{id_b}/departments"),
            None,
            None,
        ),
        (
            Method::POST,
            format!("/admin/v1/orgs/{id_b}/departments"),
            Some(json!({"name": "x"})),
            None,
        ),
        (
            Method::GET,
            format!("/admin/v1/orgs/{id_b}/departments/{category_b_id}"),
            None,
            None,
        ),
        (
            Method::DELETE,
            format!("/admin/v1/orgs/{id_b}/departments/{category_b_id}"),
            None,
            None,
        ),
    ];

    for (method, path, body, if_match) in routes {
        let (status, response) = c
            .request(method.clone(), &path, &token_a, body, if_match)
            .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} {path} deveria ser barrado por escopo, veio {response}"
        );
        assert_eq!(code(&response), "forbidden", "{method} {path}");
    }

    let (status, _) = c
        .request(
            Method::GET,
            &format!("/admin/v1/orgs/{id_b}/categories/{category_b_id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn org_token_lists_only_its_org() {
    let c = Scenario::new("propria-org").await;
    let org_a = c.create_org("Org A", "org-a").await;
    let org_b = c.create_org("Org B", "org-b").await;
    let id_a = org_a["id"].as_str().expect("id A").to_owned();
    let id_b = org_b["id"].as_str().expect("id B").to_owned();
    let token_a = c.create_token(&id_a, "token-da-a").await;

    let (status, page) = c
        .request(Method::GET, "/admin/v1/orgs", &token_a, None, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let items = page["items"].as_array().expect("lista de orgs");
    assert_eq!(items.len(), 1, "token de org só vê a própria: {page}");
    assert_eq!(items[0]["id"].as_str(), Some(id_a.as_str()));
    assert!(items.iter().all(|org| org["id"] != json!(id_b)));

    let (status, _) = c
        .request(
            Method::GET,
            &format!("/admin/v1/orgs/{id_a}"),
            &token_a,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, category) = c
        .request(
            Method::POST,
            &format!("/admin/v1/orgs/{id_a}/categories"),
            &token_a,
            Some(json!({ "name": "Categoria A" })),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{category}");

    let (status, department) = c
        .request(
            Method::POST,
            &format!("/admin/v1/orgs/{id_a}/departments"),
            &token_a,
            Some(json!({ "name": "Departamento A" })),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{department}");

    for collection in ["categories", "departments", "tokens"] {
        let (status, page) = c
            .request(
                Method::GET,
                &format!("/admin/v1/orgs/{id_a}/{collection}"),
                &token_a,
                None,
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{collection}: {page}");
        let items = page["items"].as_array().expect("itens");
        assert!(
            !items.is_empty(),
            "`{collection}` da própria org veio vazia"
        );
        assert!(
            items.iter().all(|item| item["org_id"] == json!(id_a)),
            "`{collection}` trouxe item de outra org: {page}"
        );
    }
}

#[tokio::test]
async fn org_token_cannot_create_org_or_token() {
    let c = Scenario::new("sem-global").await;
    let org_a = c.create_org("Org A", "org-a").await;
    let id_a = org_a["id"].as_str().expect("id A").to_owned();
    let token_a = c.create_token(&id_a, "token-da-a").await;

    let (status, response) = c
        .request(
            Method::POST,
            "/admin/v1/orgs",
            &token_a,
            Some(json!({ "name": "Org Nova" })),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{response}");
    assert_eq!(code(&response), "forbidden");

    let (status, response) = c
        .request(
            Method::POST,
            &format!("/admin/v1/orgs/{id_a}/tokens"),
            &token_a,
            Some(json!({ "label": "outro" })),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{response}");

    let (status, response) = c
        .request(
            Method::DELETE,
            &format!("/admin/v1/orgs/{id_a}"),
            &token_a,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{response}");

    let (_, page) = c
        .request(Method::GET, "/admin/v1/orgs", TOKEN_GLOBAL, None, None)
        .await;
    assert_eq!(page["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(page["items"][0]["id"].as_str(), Some(id_a.as_str()));
}

#[tokio::test]
async fn revoked_token_cannot_authenticate() {
    let c = Scenario::new("revogado").await;
    let org = c.create_org("Org A", "org-a").await;
    let id = org["id"].as_str().expect("id").to_owned();
    let value = c.create_token(&id, "descartavel").await;
    let token_id = value
        .strip_prefix("poph_")
        .and_then(|remaining| remaining.split_once('_'))
        .map(|(id, _)| id.to_owned())
        .expect("id no valor");

    let (status, _) = c
        .request(
            Method::GET,
            &format!("/admin/v1/orgs/{id}"),
            &value,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "token novo deveria autenticar");

    let (status, _) = c
        .request(
            Method::DELETE,
            &format!("/admin/v1/orgs/{id}/tokens/{token_id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, response) = c
        .request(
            Method::GET,
            &format!("/admin/v1/orgs/{id}"),
            &value,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{response}");
    assert_eq!(code(&response), "unauthorized");

    let (_, list) = c
        .request(
            Method::GET,
            &format!("/admin/v1/orgs/{id}/tokens"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    let text = list.to_string();
    assert!(!text.contains(&value), "o valor do token vazou na listagem");
    let secret = value.rsplit_once('_').map(|(_, s)| s).unwrap_or("");
    assert!(!text.contains(secret), "o segredo vazou na listagem");
}

#[tokio::test]
async fn writes_require_current_if_match() {
    let c = Scenario::new("if-match").await;
    let org = c.create_org("Org A", "org-a").await;
    let id = org["id"].as_str().expect("id").to_owned();

    let (status, response) = c
        .request(
            Method::PATCH,
            &format!("/admin/v1/orgs/{id}"),
            TOKEN_GLOBAL,
            Some(json!({ "name": "Novo nome" })),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{response}");
    assert_eq!(code(&response), "precondition_required");

    let (status, response) = c
        .request(
            Method::PATCH,
            &format!("/admin/v1/orgs/{id}"),
            TOKEN_GLOBAL,
            Some(json!({ "name": "Novo nome" })),
            Some(99),
        )
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{response}");
    assert_eq!(code(&response), "version_conflict");

    let (status, response) = c
        .request(
            Method::PATCH,
            &format!("/admin/v1/orgs/{id}"),
            TOKEN_GLOBAL,
            Some(json!({ "name": "Novo nome" })),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["name"], json!("Novo nome"));
    assert_eq!(response["version"], json!(2));
}

#[tokio::test]
async fn deleting_org_with_agents_requires_force() {
    let c = Scenario::new("force").await;
    let org = c.create_org("Org A", "org-a").await;
    let id = org["id"].as_str().expect("id").to_owned();

    c.store()
        .agents()
        .create(
            id.parse().expect("org id"),
            poph_store::NewAgent {
                name: "Bia".to_owned(),
                slug: "bia".to_owned(),
                ..Default::default()
            },
        )
        .expect("agente");

    let (status, response) = c
        .request(
            Method::DELETE,
            &format!("/admin/v1/orgs/{id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{response}");
    assert_eq!(code(&response), "conflict");

    let (status, _) = c
        .request(
            Method::GET,
            &format!("/admin/v1/orgs/{id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = c
        .request(
            Method::DELETE,
            &format!("/admin/v1/orgs/{id}?force=true"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = c
        .request(
            Method::GET,
            &format!("/admin/v1/orgs/{id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleting_linked_category_requires_force() {
    let c = Scenario::new("categoria-force").await;
    let org = c.create_org("Org Cat", "org-cat").await;
    let id = org["id"].as_str().expect("id").to_owned();
    let root = format!("/admin/v1/orgs/{id}");
    let create = |path: String, body: Value| {
        let c = &c;
        async move {
            let (status, response) = c
                .request(Method::POST, &path, TOKEN_GLOBAL, Some(body), None)
                .await;
            assert_eq!(status, StatusCode::CREATED, "{response}");
            response
        }
    };
    let used = create(format!("{root}/categories"), json!({"name":"Usada"})).await;
    let used_id = used["id"].as_str().unwrap().to_owned();
    let free = create(format!("{root}/categories"), json!({"name":"Livre"})).await;
    let free_id = free["id"].as_str().unwrap().to_owned();
    let product = create(
        format!("{root}/products"),
        json!({"name":"Produto da usada", "category_id": used_id}),
    )
    .await;
    let product_id = product["id"].as_str().unwrap().to_owned();
    let agent = create(
        format!("{root}/agents"),
        json!({"name":"Agente", "slug":"agente"}),
    )
    .await;
    let agent_id = agent["id"].as_str().unwrap().to_owned();
    let (status, linked) = c
        .request(
            Method::PATCH,
            &format!("{root}/agents/{agent_id}"),
            TOKEN_GLOBAL,
            Some(json!({"product_category_ids":[used_id]})),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{linked}");
    let cat_path = format!("{root}/categories/{used_id}");

    let (status, response) = c
        .request(Method::DELETE, &cat_path, TOKEN_GLOBAL, None, None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{response}");
    assert_eq!(code(&response), "conflict");
    assert!(
        response["message"].as_str().unwrap().contains("force=true"),
        "{response}"
    );
    assert_eq!(response["details"]["produtos"], 1);
    assert_eq!(response["details"]["agentes"], 1);
    assert_eq!(
        c.request(Method::GET, &cat_path, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        c.request(
            Method::GET,
            &format!("{root}/products/{product_id}"),
            TOKEN_GLOBAL,
            None,
            None
        )
        .await
        .0,
        StatusCode::OK,
        "o produto da categoria não pode sumir sem force"
    );
    let (_, agent_before) = c
        .request(
            Method::GET,
            &format!("{root}/agents/{agent_id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(agent_before["product_category_ids"], json!([used_id]));

    let (status, _) = c
        .request(
            Method::DELETE,
            &format!("{root}/categories/{free_id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, response) = c
        .request(
            Method::DELETE,
            &format!("{cat_path}?force=true"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{response}");
    assert_eq!(
        c.request(Method::GET, &cat_path, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        c.request(
            Method::GET,
            &format!("{root}/products/{product_id}"),
            TOKEN_GLOBAL,
            None,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, agent_after) = c
        .request(
            Method::GET,
            &format!("{root}/agents/{agent_id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(agent_after["product_category_ids"], json!([]));
}

#[tokio::test]
async fn healthz_and_metrics_are_public() {
    let c = Scenario::new("operacao").await;

    let response = c
        .client
        .get(c.url("/healthz"))
        .send()
        .await
        .expect("healthz");
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.expect("JSON do healthz");
    assert_eq!(body["status"], json!("ok"));
    assert_eq!(body["version"], json!(poph_server::VERSAO));
    assert!(body["uptime_seconds"].is_u64(), "uptime: {body}");
    assert_eq!(
        body["databases"]["harness"],
        json!(poph_store::harness_target_version())
    );
    assert!(body["databases"]["rag"].is_u64());

    let response = c
        .client
        .get(c.url("/metrics"))
        .send()
        .await
        .expect("metrics");
    assert_eq!(response.status(), StatusCode::OK);
    let kind = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    assert!(kind.starts_with("text/plain"), "content-type: {kind}");
    let text = response.text().await.expect("texto das métricas");

    for display_name in [
        "poph_memoria_processo_bytes",
        "poph_fila_turnos",
        "poph_fila_saida",
        "poph_agentes_carregados",
        "poph_latencia_etapa_segundos",
    ] {
        assert!(
            text.contains(display_name),
            "faltou `{display_name}` no /metrics:\n{text}"
        );
    }
    assert!(text.contains("# TYPE poph_memoria_processo_bytes gauge"));
    assert!(text.contains("# TYPE poph_latencia_etapa_segundos histogram"));
    let line_ram = text
        .lines()
        .find(|line| line.starts_with("poph_memoria_processo_bytes "))
        .expect("linha da RAM");
    let ram: i64 = line_ram
        .rsplit(' ')
        .next()
        .and_then(|value| value.parse().ok())
        .expect("valor da RAM");
    assert!(ram > 0, "RAM medida: {ram}");
}

#[tokio::test]
async fn wrong_method_and_missing_route_use_error_format() {
    let c = Scenario::new("rota").await;

    let response = c
        .client
        .get(c.url("/admin/v1/nao-existe"))
        .send()
        .await
        .expect("rota inexistente");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(response.headers().get("www-authenticate").is_none());
    let body: Value = response.json().await.expect("JSON do 404");
    assert_eq!(code(&body), "not_found");
    assert!(body["message"].is_string(), "404 sem message: {body}");
    assert_eq!(body["details"]["path"], json!("/admin/v1/nao-existe"));

    let response = c
        .client
        .put(c.url("/admin/v1/orgs"))
        .send()
        .await
        .expect("método errado");
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    let body: Value = response.json().await.expect("JSON do 405");
    assert_eq!(code(&body), "method_not_allowed");
}

#[tokio::test]
async fn openapi_is_valid_and_covers_every_route() {
    let c = Scenario::new("openapi").await;
    let response = c
        .client
        .get(c.url("/admin/v1/openapi.json"))
        .send()
        .await
        .expect("openapi");
    assert_eq!(response.status(), StatusCode::OK);
    let text = response.text().await.expect("texto");
    let document: Value = serde_json::from_str(&text).expect("JSON válido");

    oas3::from_json(&text).expect("OpenAPI 3.1 válida");

    let expected: &[(&str, &str)] = &[
        ("/admin/v1/orgs", "get"),
        ("/admin/v1/orgs", "post"),
        ("/admin/v1/orgs/{org_id}", "get"),
        ("/admin/v1/orgs/{org_id}", "patch"),
        ("/admin/v1/orgs/{org_id}", "delete"),
        ("/admin/v1/orgs/{org_id}/agents/{agent_id}/flows", "get"),
        ("/admin/v1/orgs/{org_id}/agents/{agent_id}/flows", "put"),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions",
            "get",
        ),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions",
            "post",
        ),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",
            "get",
        ),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",
            "patch",
        ),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",
            "delete",
        ),
        ("/admin/v1/orgs/{org_id}/tokens", "get"),
        ("/admin/v1/orgs/{org_id}/tokens", "post"),
        ("/admin/v1/orgs/{org_id}/tokens/{token_id}", "get"),
        ("/admin/v1/orgs/{org_id}/tokens/{token_id}", "delete"),
        ("/admin/v1/orgs/{org_id}/categories", "get"),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations",
            "get",
        ),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations",
            "post",
        ),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",
            "get",
        ),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",
            "patch",
        ),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",
            "delete",
        ),
        ("/admin/v1/orgs/{org_id}/products", "post"),
        ("/admin/v1/orgs/{org_id}/products/{product_id}", "patch"),
        ("/admin/v1/orgs/{org_id}/products/{product_id}", "delete"),
        ("/admin/v1/orgs/{org_id}/products", "get"),
        ("/admin/v1/orgs/{org_id}/products/{product_id}", "get"),
        ("/admin/v1/orgs/{org_id}/categories", "post"),
        ("/admin/v1/orgs/{org_id}/categories/{category_id}", "get"),
        ("/admin/v1/orgs/{org_id}/categories/{category_id}", "patch"),
        ("/admin/v1/orgs/{org_id}/categories/{category_id}", "delete"),
        ("/admin/v1/orgs/{org_id}/departments", "get"),
        ("/admin/v1/orgs/{org_id}/departments", "post"),
        ("/admin/v1/orgs/{org_id}/departments/{department_id}", "get"),
        (
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
            "patch",
        ),
        (
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
            "delete",
        ),
        ("/admin/v1/orgs/{org_id}/agents/{agent_id}/files", "get"),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}",
            "get",
        ),
        ("/healthz", "get"),
        ("/metrics", "get"),
    ];
    for (path, method) in expected {
        let operation = &document["paths"][path][method];
        assert!(
            !operation.is_null(),
            "OpenAPI sem {method} {path}: {}",
            document["paths"]
        );
        assert!(
            operation["operationId"].is_string(),
            "{method} {path} sem operationId"
        );
        assert!(
            operation["responses"].is_object(),
            "{method} {path} sem responses"
        );
    }

    assert!(
        document["paths"]["/admin/v1/orgs"]["get"]["security"].is_array(),
        "rota protegida sem security"
    );
    assert!(
        document["components"]["securitySchemes"]["bearer"].is_object(),
        "esquema bearer ausente"
    );

    let mut refs = Vec::new();
    collect_refs(&document, &mut refs);
    assert!(refs.len() > 10, "poucas referências: {}", refs.len());
    for reference in refs {
        let target = reference.trim_start_matches("#/");
        let resolvido = target.split('/').fold(&document, |current, part| {
            current.get(part).unwrap_or(&Value::Null)
        });
        assert!(
            !resolvido.is_null(),
            "`$ref` quebrada no OpenAPI: {reference}"
        );
    }
}

fn collect_refs(value: &Value, findings: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if key == "$ref"
                    && let Some(text) = child.as_str()
                    && text.starts_with("#/")
                {
                    findings.push(text.to_owned());
                }
                collect_refs(child, findings);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_refs(item, findings);
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn conversations_messages_and_turns_expose_org_scoped_data() {
    let c = Scenario::new("conversas-api").await;
    let org_a = c.create_org("Org A", "org-a").await;
    let org_b = c.create_org("Org B", "org-b").await;
    let a = org_a["id"].as_str().unwrap();
    let b = org_b["id"].as_str().unwrap();
    let token_a = c.create_token(a, "org a").await;
    let token_b = c.create_token(b, "org b").await;
    let org = b.parse().unwrap();
    let agent = c
        .store()
        .agents()
        .create(
            org,
            poph_store::NewAgent {
                name: "Agente".into(),
                slug: "agente".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let conversation = c
        .store()
        .conversations()
        .create(
            org,
            poph_store::NewConversation {
                agent_id: agent.id,
                integration_id: None,
                canal: "cli".into(),
                peer_externo: None,
                target_department: None,
            },
        )
        .unwrap();
    c.store()
        .messages()
        .write(
            org,
            poph_store::NewMessage {
                conversation_id: conversation.id,
                direction: poph_store::Direction::Inbound,
                role: poph_store::Role::User,
                texto: "olá".into(),
                attachments_json: "[]".into(),
                external_id: None,
                turn_id: None,
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
        )
        .unwrap();
    c.store()
        .turns()
        .start(org, conversation.id, Some(agent.id))
        .unwrap();
    let root = format!("/admin/v1/orgs/{b}/conversations");
    let detail = format!("{root}/{}", conversation.id);
    for route in [
        &root,
        &detail,
        &format!("{detail}/messages"),
        &format!("{detail}/turns"),
    ] {
        assert_eq!(
            c.request(Method::GET, route, "", None, None).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            c.request(Method::GET, route, &token_a, None, None).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            c.request(Method::GET, route, &token_b, None, None).await.0,
            StatusCode::OK
        );
    }
    let (_, body) = c
        .request(
            Method::GET,
            &format!("{detail}/messages"),
            &token_b,
            None,
            None,
        )
        .await;
    assert_eq!(body["items"][0]["text"], "olá");
    let (_, body) = c
        .request(
            Method::GET,
            &format!("{detail}/turns"),
            &token_b,
            None,
            None,
        )
        .await;
    assert_eq!(body["items"][0]["state"], "running");
    let transferred = c
        .store()
        .conversations()
        .change_status(
            org,
            conversation.id,
            conversation.version,
            poph_store::ConversationStatus::Transferred,
            None,
        )
        .unwrap();
    let reopen = format!("{detail}/reopen");
    assert_eq!(
        c.request(
            Method::POST,
            &reopen,
            &token_a,
            None,
            Some(transferred.version)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        c.request(Method::POST, &reopen, &token_b, None, Some(1))
            .await
            .0,
        StatusCode::PRECONDITION_FAILED
    );
    let (status, body) = c
        .request(
            Method::POST,
            &reopen,
            &token_b,
            None,
            Some(transferred.version),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "open");
    let records = c
        .store()
        .audit()
        .list(org, poph_store::Page::default())
        .unwrap();
    let record = records
        .iter()
        .find(|r| {
            r.acao == "reopen" && r.entity_id.as_deref() == Some(&conversation.id.to_string())
        })
        .expect("reabertura auditada");
    assert_eq!(
        record.actor_id.as_deref(),
        token_b
            .strip_prefix("poph_")
            .and_then(|s| s.split_once('_'))
            .map(|(id, _)| id)
    );
}

#[tokio::test]
async fn product_queries_are_org_scoped() {
    let c = Scenario::new("produtos-api").await;
    let a = c.create_org("Org A", "org-a").await;
    let b = c.create_org("Org B", "org-b").await;
    let org_a = a["id"].as_str().unwrap();
    let org_b = b["id"].as_str().unwrap();
    let token_a = c.create_token(org_a, "org a").await;
    let category = c
        .store()
        .categories()
        .create(
            org_b.parse().unwrap(),
            poph_store::NewCategory {
                name: "Serviços".into(),
                description: String::new(),
            },
        )
        .unwrap();
    let product = c
        .store()
        .products()
        .create(
            org_b.parse().unwrap(),
            poph_store::NewProduct {
                category_id: Some(category.id),
                name: "Visita".into(),
                price_cents: 12345,
                ..Default::default()
            },
        )
        .unwrap();
    for path in [
        format!("/admin/v1/orgs/{org_b}/products"),
        format!("/admin/v1/orgs/{org_b}/products/{}", product.id),
    ] {
        assert_eq!(
            c.request(Method::GET, &path, "", None, None).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            c.request(Method::GET, &path, &token_a, None, None).await.0,
            StatusCode::FORBIDDEN
        );
    }
    let (status, value) = c
        .request(
            Method::GET,
            &format!("/admin/v1/orgs/{org_b}/products/{}", product.id),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["name"], "Visita");
    assert_eq!(value["price_cents"], 12345);
    let (status, value) = c
        .request(
            Method::GET,
            &format!("/admin/v1/orgs/{org_b}/products"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["items"][0]["id"], product.id.to_string());
}

#[tokio::test]
async fn write_audit_identifies_actor_and_redacts_secrets() {
    let c = Scenario::new("audit-escritas").await;
    let org = c.create_org("Auditoria", "audit-escritas").await;
    let org_id = org["id"].as_str().unwrap();
    let token = c.create_token(org_id, "org").await;
    let token_id = token
        .strip_prefix("poph_")
        .unwrap()
        .split_once('_')
        .unwrap()
        .0;
    let route_org = format!("/admin/v1/orgs/{org_id}");
    assert_eq!(
        c.request(
            Method::PATCH,
            &route_org,
            &token,
            Some(json!({"name":"Mudou"})),
            Some(1)
        )
        .await
        .0,
        StatusCode::OK
    );
    for (route, entity) in [("categories", "category"), ("departments", "department")] {
        let collection = format!("{route_org}/{route}");
        let (status, created) = c
            .request(
                Method::POST,
                &collection,
                &token,
                Some(json!({"name":"Exemplo"})),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["id"].as_str().unwrap();
        let item = format!("{collection}/{id}");
        assert_eq!(
            c.request(
                Method::PATCH,
                &item,
                &token,
                Some(json!({"name":"Novo"})),
                Some(1)
            )
            .await
            .0,
            StatusCode::OK
        );
        assert_eq!(
            c.request(Method::DELETE, &item, &token, None, None).await.0,
            StatusCode::NO_CONTENT
        );
        let records = c
            .store()
            .audit()
            .list(org_id.parse().unwrap(), poph_store::Page::default())
            .unwrap();
        for action in ["create", "update", "delete"] {
            let line = records
                .iter()
                .find(|r| {
                    r.entity == entity && r.acao == action && r.entity_id.as_deref() == Some(id)
                })
                .expect("registro de auditoria");
            assert_eq!(line.ator_tipo, "admin_token");
            assert_eq!(line.actor_id.as_deref(), Some(token_id));
            assert!(!format!("{line:?}").contains(&token));
        }
    }
    let records = c
        .store()
        .audit()
        .list(org_id.parse().unwrap(), poph_store::Page::default())
        .unwrap();
    assert!(records.iter().any(|r| r.entity == "org"
        && r.acao == "create"
        && r.actor_id.as_deref() == Some("global:teste")));
    assert!(records.iter().any(|r| r.entity == "org"
        && r.acao == "update"
        && r.actor_id.as_deref() == Some(token_id)));
    assert!(records.iter().any(|r| r.entity == "token"
        && r.acao == "create"
        && r.entity_id.as_deref() == Some(token_id)
        && r.actor_id.as_deref() == Some("global:teste")));
    let route_token = format!("{route_org}/tokens/{token_id}");
    assert_eq!(
        c.request(Method::DELETE, &route_token, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let records = c
        .store()
        .audit()
        .list(org_id.parse().unwrap(), poph_store::Page::default())
        .unwrap();
    assert!(records.iter().any(|r| r.entity == "token"
        && r.acao == "revoke"
        && r.entity_id.as_deref() == Some(token_id)
        && r.actor_id.as_deref() == Some("global:teste")));
}

#[tokio::test]
async fn all_openapi_writes_require_audit_coverage() {
    use std::collections::BTreeSet;
    let c = Scenario::new("audit-openapi").await;
    let (status, doc) = c
        .request(Method::GET, "/openapi.json", "", None, None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let expected: BTreeSet<_> = [
        ("post", "/admin/v1/orgs"),
        ("patch", "/admin/v1/orgs/{org_id}"),
        ("delete", "/admin/v1/orgs/{org_id}"),
        ("post", "/admin/v1/orgs/{org_id}/agents"),
        ("patch", "/admin/v1/orgs/{org_id}/agents/{agent_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/agents/{agent_id}"),
        ("put", "/admin/v1/orgs/{org_id}/agents/{agent_id}/flows"),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions",
        ),
        (
            "patch",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/{group_id}",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/{group_id}",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/reindex",
        ),
        ("post", "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat"),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/messages",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/reopen",
        ),
        ("post", "/admin/v1/orgs/{org_id}/knowledge"),
        ("patch", "/admin/v1/orgs/{org_id}/knowledge/{group_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/knowledge/{group_id}"),
        ("post", "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items"),
        (
            "patch",
            "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}",
        ),
        ("post", "/admin/v1/orgs/{org_id}/tokens"),
        ("delete", "/admin/v1/orgs/{org_id}/tokens/{token_id}"),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations",
        ),
        (
            "patch",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/qrcode",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/disconnect",
        ),
        ("post", "/admin/v1/orgs/{org_id}/products"),
        ("patch", "/admin/v1/orgs/{org_id}/products/{product_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/products/{product_id}"),
        ("post", "/admin/v1/orgs/{org_id}/agents/{agent_id}/files"),
        ("post", "/admin/v1/orgs/{org_id}/categories"),
        ("patch", "/admin/v1/orgs/{org_id}/categories/{category_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/categories/{category_id}"),
        ("post", "/admin/v1/orgs/{org_id}/departments"),
        (
            "patch",
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/reopen",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/close",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/transfer",
        ),
        ("post", "/admin/v1/orgs/{org_id}/backup/import"),
        ("put", "/admin/v1/orgs/{org_id}/matrix"),
        ("put", "/admin/v1/orgs/{org_id}/matrix/spaces"),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/matrix/spaces/{department_id}",
        ),
        ("post", "/admin/v1/orgs/{org_id}/flows"),
        ("patch", "/admin/v1/orgs/{org_id}/flows/{flow_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/flows/{flow_id}"),
        ("post", "/admin/v1/orgs/{org_id}/flows/{flow_id}/publish"),
        ("post", "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs"),
        ("post", "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs/{run_id}/actions"),
        ("post", "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs/{run_id}/abandon"),
        ("post", "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/close"),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/site-conversations",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/messages",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/turns/{turn_id}/cancel",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/adapter-conversations",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/adapter-conversations/{id}/messages",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/adapter-conversations/{id}/turns",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/adapter-conversations/{id}/turns/{turn_id}/cancel",
        ),
    ]
    .into_iter()
    .map(|(m, p)| (m.to_owned(), p.to_owned()))
    .collect();
    let excluded_from_org_audit = BTreeSet::from([
        ("post".to_owned(), "/admin/v1/cluster/reload".to_owned()),
        (
            "post".to_owned(),
            "/admin/v1/orgs/{org_id}/flows/validate".to_owned(),
        ),
        (
            "post".to_owned(),
            "/admin/v1/orgs/{org_id}/flows/format".to_owned(),
        ),
    ]);
    let mut published = BTreeSet::new();
    for (path, methods) in doc["paths"].as_object().unwrap() {
        for method in ["post", "put", "patch", "delete"] {
            if methods.get(method).is_some()
                && !excluded_from_org_audit.contains(&(method.to_owned(), path.clone()))
            {
                published.insert((method.to_owned(), path.clone()));
            }
        }
    }
    assert_eq!(
        published, expected,
        "escrita nova exige teste de auditoria e atualização da cobertura"
    );
}

#[tokio::test]
async fn org_deletions_are_audited_globally_with_or_without_force() {
    let c = Scenario::new("audit-exclusao-org").await;
    for (slug, force) in [("audit-org-vazia", false), ("audit-org-forcada", true)] {
        let org = c.create_org("Org", slug).await;
        let id = org["id"].as_str().unwrap();
        if force {
            c.store()
                .agents()
                .create(
                    id.parse().unwrap(),
                    poph_store::NewAgent {
                        name: "Agente".into(),
                        slug: "agente".into(),
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        let url = format!("/admin/v1/orgs/{id}?force={force}");
        assert_eq!(
            c.request(Method::DELETE, &url, TOKEN_GLOBAL, None, None)
                .await
                .0,
            StatusCode::NO_CONTENT
        );
        let records = c
            .store()
            .audit()
            .list_globals(poph_store::Page::default())
            .unwrap();
        let action = if force { "delete_force" } else { "delete" };
        let record = records
            .iter()
            .find(|r| r.entity == "org" && r.acao == action && r.entity_id.as_deref() == Some(id))
            .expect("auditoria de exclusão");
        assert_eq!(record.ator_tipo, "admin_token");
        assert_eq!(record.actor_id.as_deref(), Some("global:teste"));
        assert!(!format!("{record:?}").contains(TOKEN_GLOBAL));
    }
}

#[tokio::test]
async fn agent_crud_enforces_scope_version_and_audit() {
    let c = Scenario::new("agentes-crud").await;
    let a = c.create_org("Org A", "agent-org-a").await;
    let b = c.create_org("Org B", "agent-org-b").await;
    let oa = a["id"].as_str().unwrap();
    let ob = b["id"].as_str().unwrap();
    let ta = c.create_token(oa, "a").await;
    let tb = c.create_token(ob, "b").await;
    let token_id = tb.strip_prefix("poph_").unwrap().split_once('_').unwrap().0;
    let col = format!("/admin/v1/orgs/{ob}/agents");
    let (status, created) = c
        .request(
            Method::POST,
            &col,
            &tb,
            Some(json!({"name":"Agente Teste", "slug":"agente-teste"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap();
    let item = format!("{col}/{id}");
    for (method, path, body, version) in [
        (Method::GET, col.as_str(), None, None),
        (
            Method::POST,
            col.as_str(),
            Some(json!({"name":"Intruso"})),
            None,
        ),
        (Method::GET, item.as_str(), None, None),
        (
            Method::PATCH,
            item.as_str(),
            Some(json!({"name":"Intruso"})),
            Some(1),
        ),
        (Method::DELETE, item.as_str(), None, None),
    ] {
        let (status, body) = c.request(method, path, &ta, body, version).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    }
    let (status, conflict) = c
        .request(
            Method::PATCH,
            &item,
            &tb,
            Some(json!({"name":"Novo"})),
            Some(20),
        )
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(conflict["code"], "version_conflict");
    assert_eq!(
        c.request(
            Method::PATCH,
            &item,
            &tb,
            Some(json!({"name":"Novo"})),
            None
        )
        .await
        .0,
        StatusCode::PRECONDITION_REQUIRED
    );
    let (status, updated) = c
        .request(
            Method::PATCH,
            &item,
            &tb,
            Some(json!({"name":"Novo"})),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], "Novo");
    let (status, shown) = c.request(Method::GET, &item, &tb, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(shown["name"], "Novo");
    assert_eq!(shown["product_category_ids"], json!([]));
    assert_eq!(shown["instructions"], Value::Null);
    assert_eq!(
        c.request(Method::GET, &col, &tb, None, None).await.1["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        c.request(Method::DELETE, &item, &tb, None, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        c.request(Method::GET, &item, &tb, None, None).await.0,
        StatusCode::NOT_FOUND
    );
    let records = c
        .store()
        .audit()
        .list(ob.parse().unwrap(), poph_store::Page::default())
        .unwrap();
    for action in ["create", "update", "delete"] {
        let record = records
            .iter()
            .find(|r| r.entity == "agent" && r.acao == action && r.entity_id.as_deref() == Some(id))
            .unwrap();
        assert_eq!(record.ator_tipo, "admin_token");
        assert_eq!(record.actor_id.as_deref(), Some(token_id));
        assert!(!format!("{record:?}").contains(&tb));
    }
}

#[tokio::test]
async fn agent_cli_api_parity() {
    let c = Scenario::new("agente-paridade").await;
    let org = c.create_org("Paridade", "org-paridade-api").await;
    let org_id = org["id"].as_str().unwrap();
    let col = format!("/admin/v1/orgs/{org_id}/agents");
    let cli = |args: &[&str]| -> Value {
        let exe = cli_poph_exe();
        let output = std::process::Command::new(exe)
            .args(["--org", org_id, "--offline", "agent"])
            .args(args)
            .env("POPH_DADOS_DIRETORIO", c.directory.join("dados"))
            .output()
            .expect("executa CLI poph (construída junto com a suíte)");
        assert!(
            output.status.success(),
            "CLI: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let cli_created = cli(&[
        "create",
        "--name",
        "Bia Atendente",
        "--slug",
        "bia-cli",
        "--json",
    ]);
    let (status, api_created) = c
        .request(
            Method::POST,
            &col,
            TOKEN_GLOBAL,
            Some(json!({"name":"Bia Atendente", "slug":"bia-api"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{api_created}");
    let cli_id = cli_created["agent"]["id"].as_str().unwrap();
    let api_id = api_created["id"].as_str().unwrap();
    let compare = |mut a: Value, mut b: Value| {
        for field in ["id", "slug", "created_at", "updated_at"] {
            a.as_object_mut().unwrap().remove(field);
            b.as_object_mut().unwrap().remove(field);
        }
        assert_eq!(a, b);
    };
    compare(cli_created["agent"].clone(), api_created.clone());
    let cli_changed = cli(&[
        "update",
        cli_id,
        "--if-match",
        "1",
        "--name",
        "Bia Nova",
        "--json",
    ]);
    let item = format!("{col}/{api_id}");
    let (status, api_changed) = c
        .request(
            Method::PATCH,
            &item,
            TOKEN_GLOBAL,
            Some(json!({"name":"Bia Nova"})),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{api_changed}");
    compare(cli_changed["agent"].clone(), api_changed);
    cli(&["delete", cli_id, "--json"]);
    assert_eq!(
        c.request(Method::DELETE, &item, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        c.store()
            .agents()
            .get(org_id.parse().unwrap(), cli_id.parse().unwrap())
            .is_err()
    );
    assert!(
        c.store()
            .agents()
            .get(org_id.parse().unwrap(), api_id.parse().unwrap())
            .is_err()
    );
}

#[tokio::test]
async fn instructions_api_parity_isolation_and_audit() {
    let c = Scenario::new("instrucoes").await;
    let a = c.create_org("Org A", "ins-org-a").await;
    let b = c.create_org("Org B", "ins-org-b").await;
    let oa = a["id"].as_str().unwrap();
    let ob = b["id"].as_str().unwrap();
    let ta = c.create_token(oa, "a").await;
    let tb = c.create_token(ob, "b").await;
    let token_id = tb.split('_').nth(1).unwrap();
    let org = ob.parse().unwrap();
    let new = |slug: &str| poph_store::NewAgent {
        name: "Agente teste".into(),
        slug: slug.into(),
        ..Default::default()
    };
    let cli_id = c
        .store()
        .agents()
        .create(org, new("instrucoes-cli"))
        .unwrap()
        .id;
    let api_id = c
        .store()
        .agents()
        .create(org, new("instrucoes-api"))
        .unwrap()
        .id;
    let col = format!("/admin/v1/orgs/{ob}/agents/{api_id}/instructions");
    let item = format!("{col}/current");
    for (method, path, body, version) in [
        (Method::GET, col.as_str(), None, None),
        (
            Method::POST,
            col.as_str(),
            Some(json!({"markdown":"x"})),
            None,
        ),
        (Method::GET, item.as_str(), None, None),
        (
            Method::PATCH,
            item.as_str(),
            Some(json!({"markdown":"x"})),
            Some(1),
        ),
        (Method::DELETE, item.as_str(), None, None),
    ] {
        assert_eq!(
            c.request(method, path, &ta, body, version).await.0,
            StatusCode::FORBIDDEN
        );
    }
    let file = c.directory.join("instructions.txt");
    fs::write(&file, "Primeira instrução").unwrap();
    let exe = cli_poph_exe();
    let cli = |args: &[&str]| -> Value {
        let output = std::process::Command::new(exe)
            .args(["--org", ob, "--offline", "instructions"])
            .args(args)
            .env("POPH_DADOS_DIRETORIO", c.directory.join("dados"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let id = cli_id.to_string();
    let file_path = file.to_str().unwrap();
    let compare = |mut x: Value, mut y: Value| {
        for field in ["agent_id", "created_at", "updated_at"] {
            x.as_object_mut().unwrap().remove(field);
            y.as_object_mut().unwrap().remove(field);
        }
        assert_eq!(x, y);
    };
    let created = cli(&["create", &id, "--markdown-file", file_path, "--json"]);
    let (status, api) = c
        .request(
            Method::POST,
            &col,
            &tb,
            Some(json!({"markdown":"Primeira instrução"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{api}");
    compare(created["instructions"].clone(), api);
    assert_eq!(
        c.request(Method::GET, &col, &tb, None, None).await.1["items"][0]["markdown"],
        "Primeira instrução"
    );
    assert_eq!(
        c.request(Method::GET, &item, &tb, None, None).await.1["structured"],
        json!({})
    );
    let (status, error) = c
        .request(
            Method::PATCH,
            &item,
            &tb,
            Some(json!({"markdown":"Alterada"})),
            Some(9),
        )
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(error["code"], "version_conflict");
    assert_eq!(
        c.request(
            Method::PATCH,
            &item,
            &tb,
            Some(json!({"markdown":"Alterada"})),
            None
        )
        .await
        .0,
        StatusCode::PRECONDITION_REQUIRED
    );
    fs::write(&file, "Alterada").unwrap();
    let changed = cli(&[
        "update",
        &id,
        "--if-match",
        "1",
        "--markdown-file",
        file_path,
        "--json",
    ]);
    let (status, api) = c
        .request(
            Method::PATCH,
            &item,
            &tb,
            Some(json!({"markdown":"Alterada"})),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{api}");
    compare(changed["instructions"].clone(), api);
    cli(&["delete", &id, "--json"]);
    assert_eq!(
        c.request(Method::DELETE, &item, &tb, None, None).await.0,
        StatusCode::OK
    );
    assert!(c.store().instructions().get(org, cli_id).is_err());
    assert!(c.store().instructions().get(org, api_id).is_err());
    let (status, error) = c
        .request(
            Method::PATCH,
            &item,
            &tb,
            Some(json!({"markdown":"Antiga"})),
            Some(2),
        )
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(error["details"]["current_version"], 0);
    assert!(c.store().instructions().get(org, api_id).is_err());
    fs::write(&file, "{}").unwrap();
    let cli_structured = cli(&["create", &id, "--file", file_path, "--json"]);
    let (status, api_structured) = c
        .request(
            Method::POST,
            &col,
            &tb,
            Some(json!({"structured":{}})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{api_structured}");
    compare(
        cli_structured["instructions"].clone(),
        api_structured.clone(),
    );
    assert_eq!(api_structured["derived_from_json"], true);
    assert_eq!(
        c.request(Method::GET, &item, &tb, None, None).await.1["markdown"],
        api_structured["markdown"]
    );
    let (status, error) = c
        .request(
            Method::PATCH,
            &item,
            &tb,
            Some(json!({"structured":{"tools":[{"name":"inexistente", "enabled":true}]}})),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(error["code"], "validation_error");
    let (status, error) = c.request(Method::PUT, &item, &tb, None, None).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(error["code"], "method_not_allowed");
    let records = c
        .store()
        .audit()
        .list(org, poph_store::Page::default())
        .unwrap();
    for action in ["create", "update", "delete"] {
        let r = records
            .iter()
            .find(|r| {
                r.entity == "agent_instructions"
                    && r.acao == action
                    && r.entity_id.as_deref() == Some(&api_id.to_string())
            })
            .unwrap();
        assert_eq!(r.ator_tipo, "admin_token");
        assert_eq!(r.actor_id.as_deref(), Some(token_id));
        assert!(!format!("{r:?}").contains(&tb));
    }
}

#[tokio::test]
async fn knowledge_crud_handles_conflict_and_isolation() {
    let c = Scenario::new("knowledge").await;
    let a = c.create_org("Conhecimento A", "knowledge-a").await;
    let b = c.create_org("Conhecimento B", "knowledge-b").await;
    let aid = a["id"].as_str().unwrap();
    let bid = b["id"].as_str().unwrap();
    let token = c.create_token(aid, "teste").await;
    let collection = format!("/admin/v1/orgs/{aid}/knowledge");
    let (status, g) = c
        .request(
            Method::POST,
            &collection,
            &token,
            Some(json!({"name":"Guia"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{g}");
    let group = format!("{collection}/{}", g["id"].as_str().unwrap());
    let items = format!("{group}/items");
    for (method, route) in [
        (Method::GET, format!("{collection}?limit=erro")),
        (Method::GET, format!("{items}?limit=erro")),
        (Method::DELETE, format!("{group}?force=erro")),
    ] {
        let (status, body) = c.request(method, &route, &token, None, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_request");
        assert!(body.get("message").is_some() && body.get("details").is_some());
    }
    let (status, i) = c
        .request(
            Method::POST,
            &items,
            &token,
            Some(json!({"title":"Pergunta","content":"Resposta"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{i}");
    let item = format!("{items}/{}", i["id"].as_str().unwrap());
    for path in [&group, &item] {
        let (status, _) = c
            .request(Method::PATCH, path, &token, Some(json!({})), Some(99))
            .await;
        assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    }
    for (path, methods) in [
        (&collection, vec![Method::GET, Method::POST]),
        (&group, vec![Method::GET, Method::PATCH, Method::DELETE]),
        (&items, vec![Method::GET, Method::POST]),
        (&item, vec![Method::GET, Method::PATCH, Method::DELETE]),
    ] {
        for method in methods {
            let (status, _) = c
                .request(
                    method,
                    &path.replace(aid, bid),
                    &token,
                    Some(json!({})),
                    Some(1),
                )
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN);
        }
    }
    assert_eq!(
        c.request(
            Method::PATCH,
            &group,
            &token,
            Some(json!({"description":"mudou"})),
            Some(1)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        c.request(
            Method::PATCH,
            &item,
            &token,
            Some(json!({"content":"mudou"})),
            Some(1)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        c.request(Method::DELETE, &item, &token, None, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        c.request(
            Method::DELETE,
            &format!("{group}?force=true"),
            &token,
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let token_id = token
        .strip_prefix("poph_")
        .unwrap()
        .split_once('_')
        .unwrap()
        .0;
    let records = c
        .store()
        .audit()
        .list(aid.parse().unwrap(), poph_store::Page::default())
        .unwrap();
    for (entity, id) in [
        ("knowledge_group", g["id"].as_str().unwrap()),
        ("knowledge_item", i["id"].as_str().unwrap()),
    ] {
        for action in ["create", "update", "delete"] {
            let line = records
                .iter()
                .find(|r| {
                    r.entity == entity && r.acao == action && r.entity_id.as_deref() == Some(id)
                })
                .unwrap();
            assert_eq!(line.ator_tipo, "admin_token");
            assert_eq!(line.actor_id.as_deref(), Some(token_id));
            assert!(!format!("{line:?}").contains(&token));
        }
    }
}

#[tokio::test]
async fn knowledge_link_authorization_and_missing_model() {
    let c = Scenario::new("knowledge-link").await;
    let a = c.create_org("Base A", "kb-link-a").await;
    let b = c.create_org("Base B", "kb-link-b").await;
    let aid = a["id"].as_str().unwrap();
    let bid = b["id"].as_str().unwrap();
    let token = c.create_token(aid, "kb").await;
    let agent = c
        .store()
        .agents()
        .create(
            aid.parse().unwrap(),
            poph_store::NewAgent {
                name: "Teste".into(),
                slug: "teste".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let group = c
        .store()
        .knowledge()
        .create_group(
            aid.parse().unwrap(),
            poph_store::NewKnowledgeGroup {
                name: "Grupo".into(),
                description: String::new(),
            },
        )
        .unwrap();
    let path = format!(
        "/admin/v1/orgs/{aid}/agents/{}/knowledge/{}",
        agent.id, group.id
    );
    let reindex = format!("/admin/v1/orgs/{aid}/agents/{}/knowledge/reindex", agent.id);
    for method in [Method::POST, Method::DELETE] {
        assert_eq!(
            c.request(method.clone(), &path.replace(aid, bid), &token, None, None)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        c.request(Method::POST, &reindex.replace(aid, bid), &token, None, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        c.request(Method::POST, &path, &token, None, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        c.store()
            .knowledge()
            .agent_groups(aid.parse().unwrap(), agent.id)
            .unwrap()
            .len(),
        1
    );
    let (status, response) = c.request(Method::POST, &reindex, &token, None, None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{response}");
    assert_eq!(response["code"], "embedding_unavailable");
    assert_eq!(
        c.request(Method::DELETE, &path, &token, None, None).await.0,
        StatusCode::OK
    );
    assert!(
        c.store()
            .knowledge()
            .agent_groups(aid.parse().unwrap(), agent.id)
            .unwrap()
            .is_empty()
    );
    let token_id = token
        .strip_prefix("poph_")
        .unwrap()
        .split_once('_')
        .unwrap()
        .0;
    let records = c
        .store()
        .audit()
        .list(aid.parse().unwrap(), poph_store::Page::default())
        .unwrap();
    let group_id = group.id.to_string();
    for action in ["link", "unlink"] {
        let line = records
            .iter()
            .find(|r| {
                r.entity == "knowledge_group"
                    && r.acao == action
                    && r.entity_id.as_deref() == Some(group_id.as_str())
            })
            .unwrap();
        assert_eq!(line.ator_tipo, "admin_token");
        assert_eq!(line.actor_id.as_deref(), Some(token_id));
        assert!(!format!("{line:?}").contains(&token));
    }
}

#[tokio::test]
#[ignore = "exige modelo real instalado em data/modelos"]
async fn reindex_with_real_model_makes_item_searchable_and_preserves_unchanged_items() {
    let c = Scenario::new("knowledge-real-model").await;
    let org = c.create_org("Base real", "base-real").await;
    let oid = org["id"].as_str().unwrap();
    let org_id = oid.parse().unwrap();
    let token = c.create_token(oid, "reindex").await;
    let agent = c
        .store()
        .agents()
        .create(
            org_id,
            poph_store::NewAgent {
                name: "Teste".into(),
                slug: "teste".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let group = c
        .store()
        .knowledge()
        .create_group(
            org_id,
            poph_store::NewKnowledgeGroup {
                name: "Guia".into(),
                description: String::new(),
            },
        )
        .unwrap();
    c.store()
        .knowledge()
        .link_agent(org_id, agent.id, group.id)
        .unwrap();
    let item = c
        .store()
        .knowledge()
        .create_item(
            org_id,
            poph_store::NewKnowledgeItem {
                group_id: group.id,
                title: "Prazo de entrega".into(),
                content: "Entregamos todos os pedidos em três dias úteis.".into(),
                tags: vec![],
                position: 0,
            },
        )
        .unwrap();
    let route = format!("/admin/v1/orgs/{oid}/agents/{}/knowledge/reindex", agent.id);
    let (status, body) = c.request(Method::POST, &route, &token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["indexed"]["itens_novos"], 1);
    let (status, body) = c.request(Method::POST, &route, &token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["indexed"]["itens_inalterados"], 1);
    assert_eq!(body["indexed"]["embeddings_gerados"], 0);
    let cfg = poph_rag::ModelConfig {
        diretorio: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/modelos"),
        ..Default::default()
    };
    let model = poph_rag::OrtEmbedder::open_model(&cfg, None).unwrap();
    let (index, _) = poph_rag::Index::open(
        c.store().rag_path(),
        std::sync::Arc::new(model),
        Default::default(),
    )
    .unwrap();
    let findings = index
        .search_hybrid(
            org_id,
            agent.id,
            "qual o prazo de entrega?",
            &Default::default(),
        )
        .await
        .unwrap();
    assert!(findings.iter().any(|a| a.chunk.item == item.id));
    let records = c
        .store()
        .audit()
        .list(org_id, poph_store::Page::default())
        .unwrap();
    let token_id = token
        .strip_prefix("poph_")
        .unwrap()
        .split_once('_')
        .unwrap()
        .0;
    let line = records.iter().find(|r| r.acao == "reindex").unwrap();
    assert_eq!(line.actor_id.as_deref(), Some(token_id));
    assert!(!format!("{line:?}").contains(&token));
    let link = format!(
        "/admin/v1/orgs/{oid}/agents/{}/knowledge/{}",
        agent.id, group.id
    );
    let (status, response) = c.request(Method::DELETE, &link, &token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert!(response["index_warning"].is_null());
    let findings = index
        .search_hybrid(
            org_id,
            agent.id,
            "qual o prazo de entrega?",
            &Default::default(),
        )
        .await
        .unwrap();
    assert!(!findings.iter().any(|a| a.chunk.item == item.id));
}

#[tokio::test]
async fn knowledge_cli_api_create_update_delete_parity() {
    let c = Scenario::new("kb-paridade").await;
    let org = c.create_org("Base paridade", "kb-paridade").await;
    let oid = org["id"].as_str().unwrap();
    let cli = |args: &[&str]| -> Value {
        let exe = cli_poph_exe();
        let output = std::process::Command::new(exe)
            .args(["--org", oid, "--offline", "kb"])
            .args(args)
            .env("POPH_DADOS_DIRETORIO", c.directory.join("dados"))
            .output()
            .expect("executa CLI");
        assert!(
            output.status.success(),
            "CLI: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let group_cli = cli(&[
        "group",
        "create",
        "--name",
        "CLI",
        "--description",
        "descrição",
        "--json",
    ]);
    let coll = format!("/admin/v1/orgs/{oid}/knowledge");
    let (status, group_api) = c
        .request(
            Method::POST,
            &coll,
            TOKEN_GLOBAL,
            Some(json!({"name":"API","description":"descrição"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let cid = group_cli["group"]["id"].as_str().unwrap();
    let aid = group_api["id"].as_str().unwrap();
    assert_eq!(group_cli["group"]["description"], group_api["description"]);
    let group_cli = cli(&[
        "group",
        "update",
        cid,
        "--if-match",
        "1",
        "--description",
        "nova descrição",
        "--json",
    ]);
    let group_path = format!("{coll}/{aid}");
    let (status, group_api) = c
        .request(
            Method::PATCH,
            &group_path,
            TOKEN_GLOBAL,
            Some(json!({"description":"nova descrição"})),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    for key in ["description", "status", "version"] {
        assert_eq!(group_cli["group"][key], group_api[key]);
    }
    let agent_cli = c
        .store()
        .agents()
        .create(
            oid.parse().unwrap(),
            poph_store::NewAgent {
                name: "CLI".into(),
                slug: "ag-cli".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let agent_api = c
        .store()
        .agents()
        .create(
            oid.parse().unwrap(),
            poph_store::NewAgent {
                name: "API".into(),
                slug: "ag-api".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let agent_cli_id = agent_cli.id.to_string();
    cli(&["link", cid, "--agent", &agent_cli_id, "--json"]);
    let link = format!(
        "/admin/v1/orgs/{oid}/agents/{}/knowledge/{aid}",
        agent_api.id
    );
    assert_eq!(
        c.request(Method::POST, &link, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        c.store()
            .knowledge()
            .agent_groups(oid.parse().unwrap(), agent_cli.id)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        c.store()
            .knowledge()
            .agent_groups(oid.parse().unwrap(), agent_api.id)
            .unwrap()
            .len(),
        1
    );
    cli(&["unlink", cid, "--agent", &agent_cli_id, "--json"]);
    assert_eq!(
        c.request(Method::DELETE, &link, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        c.store()
            .knowledge()
            .agent_groups(oid.parse().unwrap(), agent_cli.id)
            .unwrap()
            .is_empty()
    );
    assert!(
        c.store()
            .knowledge()
            .agent_groups(oid.parse().unwrap(), agent_api.id)
            .unwrap()
            .is_empty()
    );
    let item_cli = cli(&[
        "item",
        "create",
        "--group",
        cid,
        "--title",
        "Título",
        "--content",
        "conteúdo",
        "--json",
    ]);
    let item_coll = format!("{group_path}/items");
    let (status, item_api) = c
        .request(
            Method::POST,
            &item_coll,
            TOKEN_GLOBAL,
            Some(json!({"title":"Título","content":"conteúdo"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let ci = item_cli["item"]["id"].as_str().unwrap();
    let ai = item_api["id"].as_str().unwrap();
    for key in ["title", "content", "tags", "position", "status", "version"] {
        assert_eq!(item_cli["item"][key], item_api[key], "{key}");
    }
    let item_cli = cli(&[
        "item",
        "update",
        ci,
        "--if-match",
        "1",
        "--content",
        "alterado",
        "--json",
    ]);
    let item_path = format!("{item_coll}/{ai}");
    let (status, item_api) = c
        .request(
            Method::PATCH,
            &item_path,
            TOKEN_GLOBAL,
            Some(json!({"content":"alterado"})),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    for key in ["title", "content", "tags", "position", "status", "version"] {
        assert_eq!(item_cli["item"][key], item_api[key], "{key}");
    }
    cli(&["item", "delete", ci, "--json"]);
    assert_eq!(
        c.request(Method::DELETE, &item_path, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        c.store()
            .knowledge()
            .get_item(oid.parse().unwrap(), ci.parse().unwrap())
            .is_err()
    );
    assert!(
        c.store()
            .knowledge()
            .get_item(oid.parse().unwrap(), ai.parse().unwrap())
            .is_err()
    );
    assert_eq!(
        c.request(Method::DELETE, &group_path, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::CONFLICT
    );
    cli(&["group", "delete", cid, "--force", "--json"]);
    assert_eq!(
        c.request(
            Method::DELETE,
            &format!("{group_path}?force=true"),
            TOKEN_GLOBAL,
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(
        c.store()
            .knowledge()
            .get_group(oid.parse().unwrap(), cid.parse().unwrap())
            .is_err()
    );
    assert!(
        c.store()
            .knowledge()
            .get_group(oid.parse().unwrap(), aid.parse().unwrap())
            .is_err()
    );
}

#[tokio::test]
async fn product_writes_enforce_version_and_isolation() {
    let c = Scenario::new("produto-escritas").await;
    let a = c.create_org("Org A", "org-a").await;
    let b = c.create_org("Org B", "org-b").await;
    let org_a = a["id"].as_str().unwrap();
    let org_b = b["id"].as_str().unwrap();
    let token = c.create_token(org_a, "org a").await;
    let cat = c
        .store()
        .categories()
        .create(
            org_b.parse().unwrap(),
            poph_store::NewCategory {
                name: "Serviços".into(),
                description: String::new(),
            },
        )
        .unwrap();
    let collection = format!("/admin/v1/orgs/{org_b}/products");
    let data = json!({"category_id":cat.id.to_string(),"name":"Visita", "price_cents":12345});
    assert_eq!(
        c.request(Method::POST, &collection, &token, Some(data.clone()), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, p) = c
        .request(Method::POST, &collection, TOKEN_GLOBAL, Some(data), None)
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(p["price_cents"], 12345);
    let item = format!("{collection}/{}", p["id"].as_str().unwrap());
    for method in [Method::PATCH, Method::DELETE] {
        assert_eq!(
            c.request(
                method,
                &item,
                &token,
                Some(json!({"name":"Outro"})),
                Some(1)
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        c.request(
            Method::PATCH,
            &item,
            TOKEN_GLOBAL,
            Some(json!({"price_cents":100})),
            Some(42)
        )
        .await
        .0,
        StatusCode::PRECONDITION_FAILED
    );
    let (status, changed) = c
        .request(
            Method::PATCH,
            &item,
            TOKEN_GLOBAL,
            Some(json!({"price_cents":100})),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(changed["price_cents"], 100);
    let (status, inactive) = c
        .request(
            Method::PATCH,
            &item,
            TOKEN_GLOBAL,
            Some(json!({"status":"inactive"})),
            Some(2),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{inactive}");
    let (_, active) = c
        .request(Method::GET, &collection, TOKEN_GLOBAL, None, None)
        .await;
    assert!(active["items"].as_array().unwrap().is_empty());
    let (_, all) = c
        .request(
            Method::GET,
            &format!("{collection}?all=true"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(all["items"].as_array().unwrap().len(), 1);
    let records = c
        .store()
        .audit()
        .list(org_b.parse().unwrap(), poph_store::Page::default())
        .unwrap();
    for action in ["create", "update"] {
        let line = records
            .iter()
            .find(|r| {
                r.entity == "product"
                    && r.acao == action
                    && r.entity_id.as_deref() == p["id"].as_str()
            })
            .unwrap();
        assert_eq!(line.ator_tipo, "admin_token");
        assert_eq!(line.actor_id.as_deref(), Some("global:teste"));
        assert!(!format!("{line:?}").contains(TOKEN_GLOBAL));
    }
    assert_eq!(
        c.request(Method::DELETE, &item, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        c.request(Method::GET, &item, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let records = c
        .store()
        .audit()
        .list(org_b.parse().unwrap(), poph_store::Page::default())
        .unwrap();
    let line = records
        .iter()
        .find(|r| {
            r.entity == "product"
                && r.acao == "delete"
                && r.entity_id.as_deref() == p["id"].as_str()
        })
        .unwrap();
    assert_eq!(line.actor_id.as_deref(), Some("global:teste"));
    assert!(!format!("{line:?}").contains(TOKEN_GLOBAL));
}

#[tokio::test]
async fn integration_lifecycle_isolation_and_secrets() {
    #[derive(Clone)]
    struct LogCapture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for LogCapture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let buffer = output.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || LogCapture(buffer.clone()))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    tracing::warn!("captura de log ativa");
    let c = Scenario::new("integracoes").await;
    let a = c.create_org("Org A", "org-a").await;
    let b = c.create_org("Org B", "org-b").await;
    let oa = a["id"].as_str().unwrap();
    let ob = b["id"].as_str().unwrap();
    let token_a = c.create_token(oa, "a").await;
    let token_b = c.create_token(ob, "b").await;
    let agent = c
        .store()
        .agents()
        .create(
            ob.parse().unwrap(),
            poph_store::NewAgent {
                name: "Agente".into(),
                slug: "agente".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let col = format!("/admin/v1/orgs/{ob}/agents/{}/integrations", agent.id);
    let (st, error) = c
        .request(
            Method::POST,
            &col,
            &token_b,
            Some(json!({"kind":"eco","config_json":{"senha":"valor-sensivel-teste"}})),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code(&error), "manifest_indisponivel");
    assert!(!error.to_string().contains("valor-sensivel-teste"));
    for route in [
        format!("{col}?limit=inválido"),
        format!("/admin/v1/orgs/{ob}/products?limit=inválido"),
    ] {
        let (status, error) = c.request(Method::GET, &route, &token_b, None, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(code(&error), "invalid_request");
        assert!(error.get("message").is_some() && error.get("details").is_some());
    }
    let (_, list) = c.request(Method::GET, &col, &token_b, None, None).await;
    assert_eq!(list["items"].as_array().unwrap().len(), 0);
    let (st, v) = c
        .request(
            Method::POST,
            &col,
            &token_b,
            Some(json!({"kind":"eco","config_json":{}})),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(v["tem_segredo"], false);
    let id = v["id"].as_str().unwrap();
    let item = format!("{col}/{id}");
    let (st, unknown) = c
        .request(
            Method::POST,
            &col,
            &token_b,
            Some(json!({"kind":"eco", "senha":"valor-sensivel-teste"})),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_eq!(code(&unknown), "invalid_request");
    assert!(!unknown.to_string().contains("valor-sensivel-teste"));
    let (st, wrong) = c.request(Method::PUT, &item, &token_b, None, None).await;
    assert_eq!(st, StatusCode::METHOD_NOT_ALLOWED);
    assert!(
        wrong.get("code").is_some()
            && wrong.get("message").is_some()
            && wrong.get("details").is_some()
    );
    for (method, path, body) in [
        (Method::GET, &col, None),
        (Method::POST, &col, Some(json!({"kind":"eco"}))),
        (Method::GET, &item, None),
        (Method::PATCH, &item, Some(json!({"status":"inactive"}))),
        (Method::DELETE, &item, None),
    ] {
        let (st, _) = c.request(method, path, &token_a, body, Some(1)).await;
        assert_eq!(st, StatusCode::FORBIDDEN);
    }
    let other_agent = c
        .store()
        .agents()
        .create(
            ob.parse().unwrap(),
            poph_store::NewAgent {
                name: "Outro agente".into(),
                slug: "outro-agente".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let path_wrong = format!(
        "/admin/v1/orgs/{ob}/agents/{}/integrations/{id}",
        other_agent.id
    );
    for method in [Method::GET, Method::PATCH, Method::DELETE] {
        let (st, _) = c
            .request(
                method,
                &path_wrong,
                &token_b,
                Some(json!({"status":"inactive"})),
                Some(1),
            )
            .await;
        assert_eq!(st, StatusCode::NOT_FOUND);
    }
    let (st, conf) = c
        .request(
            Method::PATCH,
            &item,
            &token_b,
            Some(json!({"status":"inactive"})),
            Some(42),
        )
        .await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED);
    assert_eq!(code(&conf), "version_conflict");
    let (st, error) = c
        .request(
            Method::PATCH,
            &item,
            &token_b,
            Some(json!({"config_json":{"token":"valor-sensivel-teste"},"status":"inactive"})),
            Some(1),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code(&error), "manifest_indisponivel");
    let (st, v) = c
        .request(
            Method::PATCH,
            &item,
            &token_b,
            Some(json!({"status":"inactive"})),
            Some(1),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["status"], "inactive");
    let check = std::process::Command::new("python3").arg("-c").arg("import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); assert c.execute('select config_json from integrations where id=?',(sys.argv[2],)).fetchone()[0]=='{}'").arg(c.store().harness_path()).arg(id).status().unwrap();
    assert!(check.success());
    let inject = std::process::Command::new("python3").arg("-c").arg("import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute('update integrations set secrets_enc=? where id=?',(b'valor-sensivel-teste',sys.argv[2])); c.commit()").arg(c.store().harness_path()).arg(id).status().unwrap();
    assert!(inject.success());
    let (st, v) = c.request(Method::GET, &item, &token_b, None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["tem_segredo"], true);
    assert!(!v.to_string().contains("valor-sensivel-teste"));
    let (st, _) = c.request(Method::DELETE, &item, &token_b, None, None).await;
    assert_eq!(st, StatusCode::OK);
    let logs = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("captura de log ativa"));
    assert!(!logs.contains("valor-sensivel-teste"));
    assert!(!logs.contains(&token_b));
    let records = c
        .store()
        .audit()
        .list(ob.parse().unwrap(), poph_store::Page::default())
        .unwrap();
    let token_id = token_b.split('_').nth(1).unwrap();
    for action in ["create", "update", "delete"] {
        let line = records
            .iter()
            .find(|r| {
                r.entity == "integration" && r.acao == action && r.entity_id.as_deref() == Some(id)
            })
            .unwrap();
        assert_eq!(line.ator_tipo, "admin_token");
        assert_eq!(line.actor_id.as_deref(), Some(token_id));
        assert!(!format!("{line:?}").contains(&token_b));
        assert!(!format!("{line:?}").contains("valor-sensivel-teste"));
    }
}

#[tokio::test]
async fn product_cli_api_persistence_parity() {
    let c = Scenario::new("produto-paridade").await;
    let org = c.create_org("Paridade", "produto-paridade").await;
    let oid = org["id"].as_str().unwrap();
    let category = c
        .store()
        .categories()
        .create(
            oid.parse().unwrap(),
            poph_store::NewCategory {
                name: "Serviços".into(),
                description: String::new(),
            },
        )
        .unwrap();
    let cli = |args: &[&str]| -> Value {
        let out = std::process::Command::new(cli_poph_exe())
            .args(["--org", oid, "--offline", "product"])
            .args(args)
            .env("POPH_DADOS_DIRETORIO", c.directory.join("dados"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let cat = category.id.to_string();
    let created = cli(&[
        "create",
        "--name",
        "Visita técnica",
        "--category",
        &cat,
        "--price",
        "12345",
        "--slug",
        "visita-cli",
        "--json",
    ]);
    let col = format!("/admin/v1/orgs/{oid}/products");
    let (st,api)=c.request(Method::POST,&col,TOKEN_GLOBAL,Some(json!({"name":"Visita técnica","category_id":cat,"price_cents":12345,"slug":"visita-api"})),None).await;
    assert_eq!(st, StatusCode::CREATED, "{api}");
    let cli_id = created["product"]["id"].as_str().unwrap();
    let api_id = api["id"].as_str().unwrap();
    let compare = || {
        let a = c
            .store()
            .products()
            .get(oid.parse().unwrap(), cli_id.parse().unwrap())
            .unwrap();
        let b = c
            .store()
            .products()
            .get(oid.parse().unwrap(), api_id.parse().unwrap())
            .unwrap();
        assert_eq!(
            (
                a.name,
                a.category_id,
                a.price_cents,
                a.price_text,
                a.description,
                a.status,
                a.version
            ),
            (
                b.name,
                b.category_id,
                b.price_cents,
                b.price_text,
                b.description,
                b.status,
                b.version
            )
        );
    };
    compare();
    cli(&[
        "update",
        cli_id,
        "--if-match",
        "1",
        "--name",
        "Visita nova",
        "--price",
        "9000",
        "--json",
    ]);
    let item = format!("{col}/{api_id}");
    let (st, v) = c
        .request(
            Method::PATCH,
            &item,
            TOKEN_GLOBAL,
            Some(json!({"name":"Visita nova","price_cents":9000})),
            Some(1),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    compare();
    cli(&["delete", cli_id, "--force", "--json"]);
    assert_eq!(
        c.request(Method::DELETE, &item, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        c.store()
            .products()
            .get(oid.parse().unwrap(), cli_id.parse().unwrap())
            .is_err()
    );
    assert!(
        c.store()
            .products()
            .get(oid.parse().unwrap(), api_id.parse().unwrap())
            .is_err()
    );
}

#[tokio::test]
async fn org_isolation_covers_all_published_operations() {
    use std::collections::BTreeSet;
    let c = Scenario::new("isolamento-openapi").await;
    let a = c.create_org("Organização A", "isola-a").await;
    let b = c.create_org("Organização B", "isola-b").await;
    let token = c.create_token(a["id"].as_str().unwrap(), "a").await;
    let org_b = b["id"].as_str().unwrap();
    let (_, doc) = c
        .request(Method::GET, "/openapi.json", "", None, None)
        .await;
    let expected: &[(&str, &str)] = &[
        ("get", "/admin/v1/orgs/{org_id}"),
        ("delete", "/admin/v1/orgs/{org_id}"),
        ("patch", "/admin/v1/orgs/{org_id}"),
        ("get", "/admin/v1/orgs/{org_id}/agents"),
        ("post", "/admin/v1/orgs/{org_id}/agents"),
        ("get", "/admin/v1/orgs/{org_id}/agents/{agent_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/agents/{agent_id}"),
        ("patch", "/admin/v1/orgs/{org_id}/agents/{agent_id}"),
        ("get", "/admin/v1/orgs/{org_id}/agents/{agent_id}/flows"),
        ("put", "/admin/v1/orgs/{org_id}/agents/{agent_id}/flows"),
        ("get", "/admin/v1/orgs/{org_id}/agents/{agent_id}/files"),
        ("post", "/admin/v1/orgs/{org_id}/agents/{agent_id}/files"),
        (
            "get",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}/content",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",
        ),
        (
            "patch",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",
        ),
        (
            "patch",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/qrcode",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/state",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/disconnect",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/reindex",
        ),
        ("get", "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat"),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/messages",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/reopen",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/messages/{message_id}/attachments/{attachment_id}/content",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/{group_id}",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/{group_id}",
        ),
        ("get", "/admin/v1/orgs/{org_id}/categories"),
        ("post", "/admin/v1/orgs/{org_id}/categories"),
        ("get", "/admin/v1/orgs/{org_id}/categories/{category_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/categories/{category_id}"),
        ("patch", "/admin/v1/orgs/{org_id}/categories/{category_id}"),
        ("get", "/admin/v1/orgs/{org_id}/conversations"),
        (
            "get",
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/messages",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/reopen",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/close",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/transfer",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/turns",
        ),
        ("get", "/admin/v1/orgs/{org_id}/departments"),
        ("post", "/admin/v1/orgs/{org_id}/departments"),
        ("get", "/admin/v1/orgs/{org_id}/departments/{department_id}"),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
        ),
        (
            "patch",
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
        ),
        ("get", "/admin/v1/orgs/{org_id}/knowledge"),
        ("post", "/admin/v1/orgs/{org_id}/knowledge"),
        ("get", "/admin/v1/orgs/{org_id}/knowledge/{group_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/knowledge/{group_id}"),
        ("patch", "/admin/v1/orgs/{org_id}/knowledge/{group_id}"),
        ("get", "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items"),
        ("post", "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items"),
        (
            "get",
            "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}",
        ),
        (
            "patch",
            "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}",
        ),
        ("get", "/admin/v1/orgs/{org_id}/products"),
        ("post", "/admin/v1/orgs/{org_id}/products"),
        ("get", "/admin/v1/orgs/{org_id}/products/{product_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/products/{product_id}"),
        ("patch", "/admin/v1/orgs/{org_id}/products/{product_id}"),
        ("get", "/admin/v1/orgs/{org_id}/backup"),
        ("post", "/admin/v1/orgs/{org_id}/backup/import"),
        ("get", "/admin/v1/orgs/{org_id}/manifests"),
        ("get", "/admin/v1/orgs/{org_id}/manifests/{kind}"),
        ("get", "/admin/v1/orgs/{org_id}/tokens"),
        ("post", "/admin/v1/orgs/{org_id}/tokens"),
        ("get", "/admin/v1/orgs/{org_id}/tokens/{token_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/tokens/{token_id}"),
        ("get", "/admin/v1/orgs/{org_id}/matrix"),
        ("put", "/admin/v1/orgs/{org_id}/matrix"),
        ("get", "/admin/v1/orgs/{org_id}/matrix/spaces"),
        ("put", "/admin/v1/orgs/{org_id}/matrix/spaces"),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/matrix/spaces/{department_id}",
        ),
        ("post", "/admin/v1/orgs/{org_id}/flows/validate"),
        ("post", "/admin/v1/orgs/{org_id}/flows/format"),
        ("get", "/admin/v1/orgs/{org_id}/flows"),
        ("post", "/admin/v1/orgs/{org_id}/flows"),
        ("get", "/admin/v1/orgs/{org_id}/flows/{flow_id}"),
        ("patch", "/admin/v1/orgs/{org_id}/flows/{flow_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/flows/{flow_id}"),
        ("post", "/admin/v1/orgs/{org_id}/flows/{flow_id}/publish"),
        ("get", "/admin/v1/orgs/{org_id}/flows/{flow_id}/revisions"),
        (
            "get",
            "/admin/v1/orgs/{org_id}/flows/{flow_id}/revisions/{number}",
        ),
        ("get", "/admin/v1/orgs/{org_id}/flows/{flow_id}/diff"),
        ("get", "/admin/v1/orgs/{org_id}/flows/{flow_id}/preview"),
        ("get", "/admin/v1/orgs/{org_id}/fichas"),
        ("get", "/admin/v1/orgs/{org_id}/fichas/{ficha_id}"),
        ("get", "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/events"),
        (
            "post",
            "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs/{run_id}/actions",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs/{run_id}/abandon",
        ),
        ("post", "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/close"),
        ("get", "/admin/v1/orgs/{org_id}/analytics/conversations"),
        ("get", "/admin/v1/orgs/{org_id}/analytics/sla"),
        ("get", "/admin/v1/orgs/{org_id}/usage/tokens"),
        (
            "get",
            "/admin/v1/orgs/{org_id}/analytics/operational-summary",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/site-conversations",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/messages",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/messages",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/turns/{turn_id}",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/turns/{turn_id}/cancel",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/events",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/events/stream",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/adapter-conversations",
        ),
        ("get", "/admin/v1/orgs/{org_id}/adapter-conversations/{id}"),
        (
            "post",
            "/admin/v1/orgs/{org_id}/adapter-conversations/{id}/messages",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/adapter-conversations/{id}/messages",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/adapter-conversations/{id}/turns",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/adapter-conversations/{id}/turns/{turn_id}",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/adapter-conversations/{id}/turns/{turn_id}/cancel",
        ),
    ];
    let expected_set: BTreeSet<_> = expected
        .iter()
        .map(|(m, p)| ((*m).to_owned(), (*p).to_owned()))
        .collect();
    let mut published = BTreeSet::new();
    for (path, methods) in doc["paths"].as_object().unwrap() {
        if !path.starts_with("/admin/v1/orgs/") {
            continue;
        }
        for method in ["get", "post", "put", "patch", "delete"] {
            if methods.get(method).is_some() {
                published.insert((method.to_owned(), path.clone()));
            }
        }
    }
    assert_eq!(
        published, expected_set,
        "rota nova exige teste de isolamento"
    );
    for (method, model) in expected {
        let mut path = model.replace("{org_id}", org_b);
        for display_name in [
            "agent_id",
            "file_id",
            "integration_id",
            "group_id",
            "item_id",
            "category_id",
            "department_id",
            "conversation_id",
            "product_id",
            "token_id",
            "kind",
            "flow_id",
            "ficha_id",
            "run_id",
            "number",
            "id",
            "turn_id",
        ] {
            path = path.replace(&format!("{{{display_name}}}"), org_b);
        }
        let body = if *method == "post" && model.ends_with("/tokens") {
            Some(json!({"label":"teste"}))
        } else if *method == "post"
            && (model.ends_with("/categories") || model.ends_with("/departments"))
        {
            Some(json!({"name":"teste"}))
        } else if *method == "put" && model.ends_with("/matrix") {
            Some(json!({"enabled":true}))
        } else if *method == "put" && model.ends_with("/matrix/spaces") {
            Some(json!({"department_id": org_b}))
        } else if *method == "post" || *method == "patch" {
            Some(json!({}))
        } else {
            None
        };
        let method = Method::from_bytes(method.to_uppercase().as_bytes()).unwrap();
        let (status, response) = c
            .request(method.clone(), &path, &token, body, Some(1))
            .await;
        assert!(
            status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
            "{method} {model}: esperado 403/404, recebido {status}: {response}"
        );
    }
}

#[tokio::test]
async fn all_updates_reject_stale_version() {
    async fn create(c: &Scenario, path: &str, body: Value) -> Value {
        let (status, value) = c
            .request(Method::POST, path, TOKEN_GLOBAL, Some(body), None)
            .await;
        assert_eq!(status, StatusCode::CREATED, "{path}: {value}");
        value
    }
    let c = Scenario::new("versoes-agregado").await;
    let org = c.create_org("Versões", "versoes-agregado").await;
    let id = org["id"].as_str().unwrap();
    let root = format!("/admin/v1/orgs/{id}");
    let category = create(
        &c,
        &format!("{root}/categories"),
        json!({"name":"Categoria"}),
    )
    .await;
    let department = create(
        &c,
        &format!("{root}/departments"),
        json!({"name":"Departamento"}),
    )
    .await;
    let agent = create(
        &c,
        &format!("{root}/agents"),
        json!({"name":"Agente", "slug":"agente"}),
    )
    .await;
    let agent_id = agent["id"].as_str().unwrap();
    let agent_path = format!("{root}/agents/{agent_id}");
    create(
        &c,
        &format!("{agent_path}/instructions"),
        json!({"markdown":"Instruções"}),
    )
    .await;
    let integration = create(
        &c,
        &format!("{agent_path}/integrations"),
        json!({"kind":"eco"}),
    )
    .await;
    let group = create(&c, &format!("{root}/knowledge"), json!({"name":"Base"})).await;
    let group_path = format!("{root}/knowledge/{}", group["id"].as_str().unwrap());
    let item = create(
        &c,
        &format!("{group_path}/items"),
        json!({"title":"Título","content":"Conteúdo"}),
    )
    .await;
    let product = create(
        &c,
        &format!("{root}/products"),
        json!({"name":"Produto", "category_id":category["id"]}),
    )
    .await;
    let org_id = id.parse().unwrap();
    let conversation = c
        .store()
        .conversations()
        .create(
            org_id,
            poph_store::NewConversation {
                agent_id: agent_id.parse().unwrap(),
                integration_id: None,
                canal: "cli".into(),
                peer_externo: None,
                target_department: None,
            },
        )
        .unwrap();
    c.store()
        .conversations()
        .change_status(
            org_id,
            conversation.id,
            conversation.version,
            poph_store::ConversationStatus::Transferred,
            None,
        )
        .unwrap();
    let cases = [
        (Method::PATCH, root.clone(), json!({"name":"alterada"})),
        (
            Method::PATCH,
            format!("{root}/categories/{}", category["id"].as_str().unwrap()),
            json!({"name":"alterada"}),
        ),
        (
            Method::PATCH,
            format!("{root}/departments/{}", department["id"].as_str().unwrap()),
            json!({"name":"alterado"}),
        ),
        (
            Method::PATCH,
            agent_path.clone(),
            json!({"name":"alterado"}),
        ),
        (
            Method::PATCH,
            format!("{agent_path}/instructions/current"),
            json!({"markdown":"alterada"}),
        ),
        (
            Method::PATCH,
            format!(
                "{agent_path}/integrations/{}",
                integration["id"].as_str().unwrap()
            ),
            json!({"status":"inactive"}),
        ),
        (
            Method::PATCH,
            group_path.clone(),
            json!({"name":"alterada"}),
        ),
        (
            Method::PATCH,
            format!("{group_path}/items/{}", item["id"].as_str().unwrap()),
            json!({"content":"alterado"}),
        ),
        (
            Method::PATCH,
            format!("{root}/products/{}", product["id"].as_str().unwrap()),
            json!({"name":"alterado"}),
        ),
        (
            Method::POST,
            format!("{root}/conversations/{}/reopen", conversation.id),
            json!({}),
        ),
        (
            Method::POST,
            format!("{root}/conversations/{}/close", conversation.id),
            json!({}),
        ),
    ];
    for (method, path, body) in cases {
        let (status, response) = c
            .request(method.clone(), &path, TOKEN_GLOBAL, Some(body), Some(999))
            .await;
        assert_eq!(
            status,
            StatusCode::PRECONDITION_FAILED,
            "{method} {path}: {response}"
        );
        assert_eq!(code(&response), "version_conflict", "{method} {path}");
    }
}

#[tokio::test]
async fn category_and_department_cli_api_parity() {
    let c = Scenario::new("paridade-categorias-departamentos").await;
    let a = c.create_org("CLI", "paridade-cli").await;
    let b = c.create_org("API", "paridade-api").await;
    let oa = a["id"].as_str().unwrap();
    let ob = b["id"].as_str().unwrap();
    let cli = |resource: &str, args: &[&str]| -> Value {
        let exe = cli_poph_exe();
        let output = std::process::Command::new(exe)
            .args(["--org", oa, "--offline", resource])
            .args(args)
            .env("POPH_DADOS_DIRETORIO", c.directory.join("dados"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "CLI: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };
    for (resource, route, key) in [
        ("category", "categories", "category"),
        ("department", "departments", "department"),
    ] {
        let via_cli = cli(
            resource,
            &[
                "create",
                "--name",
                "Exemplo",
                "--description",
                "inicial",
                "--json",
            ],
        );
        let path = format!("/admin/v1/orgs/{ob}/{route}");
        let (status, via_api) = c
            .request(
                Method::POST,
                &path,
                TOKEN_GLOBAL,
                Some(json!({"name":"Exemplo", "description":"inicial"})),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{via_api}");
        let cli_id = via_cli[key]["id"].as_str().unwrap();
        let api_id = via_api["id"].as_str().unwrap();
        let compare = |mut a: Value, mut b: Value| {
            for field in ["id", "org_id", "created_at", "updated_at"] {
                a.as_object_mut().unwrap().remove(field);
                b.as_object_mut().unwrap().remove(field);
            }
            assert_eq!(a, b);
        };
        compare(via_cli[key].clone(), via_api.clone());
        let updated_cli = cli(
            resource,
            &[
                "update",
                cli_id,
                "--if-match",
                "1",
                "--description",
                "alterado",
                "--json",
            ],
        );
        let item = format!("{path}/{api_id}");
        let (status, updated_api) = c
            .request(
                Method::PATCH,
                &item,
                TOKEN_GLOBAL,
                Some(json!({"description":"alterado"})),
                Some(1),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{updated_api}");
        compare(updated_cli[key].clone(), updated_api);
        let mut args = vec!["delete", cli_id, "--json"];
        if resource == "category" {
            args.push("--force");
        }
        cli(resource, &args);
        let (status, resp) = c
            .request(Method::DELETE, &item, TOKEN_GLOBAL, None, None)
            .await;
        assert!(status.is_success(), "{resp}");
        assert_eq!(
            c.request(Method::GET, &item, TOKEN_GLOBAL, None, None)
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        let cli_lookup = std::process::Command::new(cli_poph_exe())
            .args(["--org", oa, "--offline", resource, "show", cli_id, "--json"])
            .env("POPH_DADOS_DIRETORIO", c.directory.join("dados"))
            .output()
            .unwrap();
        assert!(!cli_lookup.status.success());
    }
}

#[tokio::test]
async fn org_and_token_cli_api_parity() {
    let c = Scenario::new("paridade-org-token").await;
    let cli = |args: &[&str]| -> Value {
        let exe = cli_poph_exe();
        let output = std::process::Command::new(exe)
            .arg("--offline")
            .args(args)
            .env("POPH_DADOS_DIRETORIO", c.directory.join("dados"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "CLI: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let created_cli = cli(&[
        "org",
        "create",
        "--name",
        "Paridade",
        "--slug",
        "paridade-cli",
        "--json",
    ]);
    let (status, created_api) = c
        .request(
            Method::POST,
            "/admin/v1/orgs",
            TOKEN_GLOBAL,
            Some(json!({"name":"Paridade", "slug":"paridade-api"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created_api}");
    let oa = created_cli["org"]["id"].as_str().unwrap();
    let ob = created_api["id"].as_str().unwrap();
    let compare_org = |mut a: Value, mut b: Value| {
        for key in [
            "id",
            "slug",
            "created_at",
            "updated_at",
            "agents_active_count",
        ] {
            a.as_object_mut().unwrap().remove(key);
            b.as_object_mut().unwrap().remove(key);
        }
        assert_eq!(a, b);
    };
    compare_org(created_cli["org"].clone(), created_api.clone());
    let edit_cli = cli(&[
        "org",
        "update",
        oa,
        "--if-match",
        "1",
        "--name",
        "Editada",
        "--json",
    ]);
    let org_api = format!("/admin/v1/orgs/{ob}");
    let (status, edit_api) = c
        .request(
            Method::PATCH,
            &org_api,
            TOKEN_GLOBAL,
            Some(json!({"name":"Editada"})),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{edit_api}");
    compare_org(edit_cli["org"].clone(), edit_api);
    let token_cli = cli(&["--org", oa, "token", "create", "--label", "Teste", "--json"]);
    let tokens_path = format!("{org_api}/tokens");
    let (status, token_api) = c
        .request(
            Method::POST,
            &tokens_path,
            TOKEN_GLOBAL,
            Some(json!({"label":"Teste"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{token_api}");
    let token_a = token_cli["token"]["id"].as_str().unwrap();
    let token_b = token_api["token"]["id"].as_str().unwrap();
    let compare_token = |mut a: Value, mut b: Value| {
        for key in ["id", "org_id", "created_at", "revoked_at"] {
            a.as_object_mut().unwrap().remove(key);
            b.as_object_mut().unwrap().remove(key);
        }
        assert_eq!(a, b);
    };
    compare_token(token_cli["token"].clone(), token_api["token"].clone());
    cli(&["--org", oa, "token", "delete", token_a, "--json"]);
    let token_path = format!("{tokens_path}/{token_b}");
    let (status, resp) = c
        .request(Method::DELETE, &token_path, TOKEN_GLOBAL, None, None)
        .await;
    assert!(status.is_success(), "{resp}");
    let read_cli = cli(&["--org", oa, "token", "show", token_a, "--json"]);
    let (status, read_api) = c
        .request(Method::GET, &token_path, TOKEN_GLOBAL, None, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{read_api}");
    compare_token(read_cli["token"].clone(), read_api);
    cli(&["org", "delete", oa, "--force", "--json"]);
    let (status, resp) = c
        .request(
            Method::DELETE,
            &format!("{org_api}?force=true"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert!(status.is_success(), "{resp}");
    for id in [oa, ob] {
        assert!(c.store().orgs().get(id.parse().unwrap()).is_err());
    }
}

#[tokio::test]
async fn library_cli_api_lookup_and_delete_parity() {
    let c = Scenario::new("paridade-biblioteca").await;
    let org = c.create_org("Biblioteca", "biblioteca-paridade").await;
    let oid = org["id"].as_str().unwrap();
    let org_id = oid.parse().unwrap();
    let agent = c
        .store()
        .agents()
        .create(
            org_id,
            poph_store::NewAgent {
                name: "Agente".into(),
                slug: "agente".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let new = || poph_store::NewLibraryFile {
        agent_id: agent.id,
        name: "Manual".into(),
        description: "Guia".into(),
        mime: "application/pdf".into(),
        size_bytes: 42,
        sha256: "a".repeat(64),
        blob_path: "exemplo".into(),
    };
    let a = c.store().library().create(org_id, new()).unwrap();
    let b = c.store().library().create(org_id, new()).unwrap();
    let agent_id = agent.id.to_string();
    let cli = |args: &[&str]| -> Value {
        let exe = cli_poph_exe();
        let out = std::process::Command::new(exe)
            .args(["--org", oid, "--offline", "file"])
            .args(args)
            .env("POPH_DADOS_DIRETORIO", c.directory.join("dados"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "CLI: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let aid = a.id.to_string();
    let bid = b.id.to_string();
    let shown_cli = cli(&["show", &aid, "--agent", &agent_id, "--json"]);
    let collection = format!("/admin/v1/orgs/{oid}/agents/{agent_id}/files");
    let (status, shown_api) = c
        .request(
            Method::GET,
            &format!("{collection}/{bid}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    for (cli_field, api_field) in [
        ("nome", "name"),
        ("descricao", "description"),
        ("mime", "mime"),
        ("tamanho_bytes", "size_bytes"),
        ("sha256", "sha256"),
    ] {
        assert_eq!(shown_cli["arquivo"][cli_field], shown_api[api_field]);
    }
    cli(&["delete", &aid, "--agent", &agent_id, "--json"]);
    let (status, resp) = c
        .request(
            Method::DELETE,
            &format!("{collection}/{bid}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    assert!(c.store().library().get(org_id, agent.id, a.id).is_err());
    assert!(c.store().library().get(org_id, agent.id, b.id).is_err());
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89";

fn cli_poph_exe() -> &'static std::path::Path {
    static EXE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    EXE.get_or_init(|| {
        let built = std::process::Command::new(env!("CARGO"))
            .args(["build", "--locked", "-p", "poph"])
            .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
            .output()
            .expect("compila a CLI para paridade");
        assert!(
            built.status.success(),
            "build CLI: {}",
            String::from_utf8_lossy(&built.stderr)
        );
        let target = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
            .parent()
            .expect("CARGO_TARGET_TMPDIR fica dentro do target-dir");
        let exe = target.join("debug").join("poph");
        assert!(
            exe.is_file(),
            "binário da CLI não encontrado em {}",
            exe.display()
        );
        exe
    })
}

fn cli_poph(directory: &std::path::Path, args: &[&str]) -> Value {
    let exe = cli_poph_exe();
    let output = std::process::Command::new(exe)
        .args(args)
        .env("POPH_DADOS_DIRETORIO", directory.join("dados"))
        .output()
        .expect("executa a CLI");
    assert!(
        output.status.success(),
        "CLI `{}`: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON da CLI")
}

#[tokio::test]
async fn library_cli_api_upload_parity() {
    let c = Scenario::new("paridade-upload").await;
    let org = c.create_org("Upload", "paridade-upload").await;
    let oid = org["id"].as_str().unwrap().to_owned();
    let (status, agent) = c
        .request(
            Method::POST,
            &format!("/admin/v1/orgs/{oid}/agents"),
            TOKEN_GLOBAL,
            Some(json!({"name":"Agente", "slug":"agente"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{agent}");
    let agent_id = agent["id"].as_str().unwrap().to_owned();

    let file = c.directory.join("manual.png");
    fs::write(&file, PNG).expect("arquivo de teste");
    let via_cli = cli_poph(
        &c.directory,
        &[
            "--org",
            &oid,
            "--offline",
            "file",
            "upload",
            file.to_str().unwrap(),
            "--agent",
            &agent_id,
            "--name",
            "Manual",
            "--description",
            "Guia",
            "--json",
        ],
    );
    let route = format!(
        "/admin/v1/orgs/{oid}/agents/{agent_id}/files?filename=manual.png&name=Manual&description=Guia"
    );
    let (status, via_api) = c.upload_file(&route, TOKEN_GLOBAL, PNG).await;
    assert_eq!(status, StatusCode::CREATED, "{via_api}");

    for (cli_field, api_field) in [
        ("nome", "name"),
        ("descricao", "description"),
        ("mime", "mime"),
        ("tamanho_bytes", "size_bytes"),
        ("sha256", "sha256"),
    ] {
        assert_eq!(
            via_cli["arquivo"][cli_field], via_api[api_field],
            "campo `{cli_field}` diferente entre CLI e API"
        );
    }
    let org_id = oid.parse().unwrap();
    let records = c
        .store()
        .library()
        .list(
            org_id,
            agent_id.parse().unwrap(),
            poph_store::Page::default(),
        )
        .expect("lista da biblioteca");
    assert_eq!(records.len(), 2, "cada caminho gravou o seu registro");
    assert_eq!(records[0].sha256, records[1].sha256);
    assert_eq!(
        records[0].blob_path, records[1].blob_path,
        "mesmo conteúdo: um blob só, caminho igual"
    );
    let blob = c
        .directory
        .join("dados")
        .join("blobs")
        .join(&records[1].blob_path);
    assert!(blob.is_file(), "blob gravado em {}", blob.display());

    let audit = c
        .store()
        .audit()
        .list(org_id, poph_store::Page::default())
        .expect("auditoria");
    assert!(
        audit
            .iter()
            .any(|line| line.acao == "create" && line.entity == "file_library"),
        "criação pela API sem linha de auditoria: {audit:?}"
    );

    let (status, error) = c
        .upload_file(
            &format!("/admin/v1/orgs/{oid}/agents/{agent_id}/files"),
            TOKEN_GLOBAL,
            PNG,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(code(&error), "validation_error", "{error}");
    assert!(
        error["message"].as_str().is_some_and(|m| !m.is_empty()),
        "recusa sem mensagem: {error}"
    );

    let (status, error) = c
        .upload_file(
            &format!("/admin/v1/orgs/{oid}/agents/{agent_id}/files?filename=programa.exe"),
            TOKEN_GLOBAL,
            b"conteudo qualquer",
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(code(&error), "validation_error", "{error}");
    assert_eq!(
        c.store()
            .library()
            .count_rows(org_id, agent_id.parse().unwrap())
            .unwrap(),
        2,
        "arquivo recusado não pode virar registro"
    );

    let mut large = PNG.to_vec();
    large.resize(3 * 1024 * 1024, 0);
    let (status, body) = c
        .upload_file(
            &format!("/admin/v1/orgs/{oid}/agents/{agent_id}/files?filename=grande.png"),
            TOKEN_GLOBAL,
            &large,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["size_bytes"], json!(large.len()));
}

#[tokio::test]
async fn agent_knowledge_group_link_cli_api_parity() {
    let c = Scenario::new("paridade-vinculo").await;
    let org = c.create_org("Vínculo", "paridade-vinculo").await;
    let oid = org["id"].as_str().unwrap().to_owned();
    let org_id = oid.parse().unwrap();
    let agent = c
        .store()
        .agents()
        .create(
            org_id,
            poph_store::NewAgent {
                name: "Agente".into(),
                slug: "agente".into(),
                ..Default::default()
            },
        )
        .expect("agente");
    let aid = agent.id.to_string();

    let group_cli = cli_poph(
        &c.directory,
        &[
            "--org",
            &oid,
            "--offline",
            "kb",
            "group",
            "create",
            "--name",
            "Pela CLI",
            "--json",
        ],
    );
    let group_cli_id = group_cli["group"]["id"].as_str().unwrap().to_owned();
    let (status, group_api) = c
        .request(
            Method::POST,
            &format!("/admin/v1/orgs/{oid}/knowledge"),
            TOKEN_GLOBAL,
            Some(json!({"name":"Pela API"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{group_api}");
    let group_api_id = group_api["id"].as_str().unwrap().to_owned();

    cli_poph(
        &c.directory,
        &[
            "--org",
            &oid,
            "--offline",
            "kb",
            "link",
            &group_cli_id,
            "--agent",
            &aid,
            "--json",
        ],
    );
    let (status, response) = c
        .request(
            Method::POST,
            &format!("/admin/v1/orgs/{oid}/agents/{aid}/knowledge/{group_api_id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["linked"], json!(true));

    let server_state = |group: &str| -> Vec<String> {
        let mut agents: Vec<String> = c
            .store()
            .knowledge()
            .agents_from_group(org_id, group.parse().unwrap())
            .expect("agentes do grupo")
            .iter()
            .map(ToString::to_string)
            .collect();
        agents.sort();
        agents
    };
    assert_eq!(server_state(&group_cli_id), vec![aid.clone()]);
    assert_eq!(server_state(&group_api_id), server_state(&group_cli_id));

    cli_poph(
        &c.directory,
        &[
            "--org",
            &oid,
            "--offline",
            "kb",
            "unlink",
            &group_cli_id,
            "--agent",
            &aid,
            "--json",
        ],
    );
    let (status, response) = c
        .request(
            Method::DELETE,
            &format!("/admin/v1/orgs/{oid}/agents/{aid}/knowledge/{group_api_id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert!(server_state(&group_cli_id).is_empty());
    assert!(server_state(&group_api_id).is_empty());

    let audit = c
        .store()
        .audit()
        .list(org_id, poph_store::Page::default())
        .expect("auditoria");
    for action in ["link", "unlink"] {
        assert!(
            audit.iter().any(|line| line.acao == action),
            "sem auditoria de `{action}`: {audit:?}"
        );
    }
}

#[tokio::test]
#[ignore = "exige o modelo real instalado em data/modelos"]
async fn reindex_cli_api_parity() {
    let c = Scenario::new("paridade-reindex").await;
    let models = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/modelos");
    std::os::unix::fs::symlink(&models, c.directory.join("dados").join("modelos"))
        .expect("link simbólico do modelo");

    let org = c.create_org("Reindex", "paridade-reindex").await;
    let oid = org["id"].as_str().unwrap().to_owned();
    let org_id = oid.parse().unwrap();
    let agent = c
        .store()
        .agents()
        .create(
            org_id,
            poph_store::NewAgent {
                name: "Agente".into(),
                slug: "agente".into(),
                ..Default::default()
            },
        )
        .expect("agente");
    let aid = agent.id.to_string();
    let group = c
        .store()
        .knowledge()
        .create_group(
            org_id,
            poph_store::NewKnowledgeGroup {
                name: "Guia".into(),
                description: String::new(),
            },
        )
        .expect("grupo");
    c.store()
        .knowledge()
        .link_agent(org_id, agent.id, group.id)
        .expect("vínculo");
    for (title, content) in [
        (
            "Prazo de entrega",
            "Entregamos todos os pedidos em três dias úteis.",
        ),
        (
            "Troca",
            "A troca é aceita em até sete dias após o recebimento.",
        ),
    ] {
        c.store()
            .knowledge()
            .create_item(
                org_id,
                poph_store::NewKnowledgeItem {
                    group_id: group.id,
                    title: title.into(),
                    content: content.into(),
                    tags: vec![],
                    position: 0,
                },
            )
            .expect("item");
    }

    let via_cli = cli_poph(
        &c.directory,
        &[
            "--org",
            &oid,
            "--offline",
            "kb",
            "reindex",
            "--agent",
            &aid,
            "--json",
        ],
    );
    assert_eq!(via_cli["agents"][0]["indexed"]["items_new"], json!(2));

    let route = format!("/admin/v1/orgs/{oid}/agents/{aid}/knowledge/reindex");
    let (status, via_api) = c
        .request(Method::POST, &route, TOKEN_GLOBAL, None, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{via_api}");
    assert_eq!(via_api["indexed"]["itens_novos"], json!(0));
    assert_eq!(via_api["indexed"]["itens_alterados"], json!(0));
    assert_eq!(via_api["indexed"]["itens_inalterados"], json!(2));
    assert_eq!(via_api["indexed"]["embeddings_gerados"], json!(0));

    c.store()
        .knowledge()
        .create_item(
            org_id,
            poph_store::NewKnowledgeItem {
                group_id: group.id,
                title: "Frete".into(),
                content: "O frete é grátis acima de cem reais.".into(),
                tags: vec![],
                position: 0,
            },
        )
        .expect("item novo");
    let (status, via_api) = c
        .request(Method::POST, &route, TOKEN_GLOBAL, None, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{via_api}");
    assert_eq!(via_api["indexed"]["itens_novos"], json!(1));
    assert_eq!(via_api["indexed"]["itens_inalterados"], json!(2));
    let via_cli = cli_poph(
        &c.directory,
        &[
            "--org",
            &oid,
            "--offline",
            "kb",
            "reindex",
            "--agent",
            &aid,
            "--json",
        ],
    );
    let index = &via_cli["agents"][0]["indexed"];
    assert_eq!(index["items_new"], json!(0));
    assert_eq!(index["items_changed"], json!(0));
    assert_eq!(index["items_unchanged"], json!(3));
    assert_eq!(index["embeddings"], json!(0));
}

#[tokio::test]
async fn api_docs_cover_all_published_routes() {
    use std::collections::BTreeSet;
    let c = Scenario::new("docs-api").await;
    let (status, document) = c
        .request(Method::GET, "/openapi.json", "", None, None)
        .await;
    assert_eq!(status, StatusCode::OK);

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/API.md");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("leitura de {}: {error}", path.display()));
    for section in [
        "## Inventário da HR-21",
        "## Estado da prova",
        "## O que a HR-21 entrega e o que não entrega",
    ] {
        assert!(
            text.contains(section),
            "docs/API.md sem a seção `{section}`"
        );
    }

    struct Row {
        method: String,
        route: String,
        cli: String,
        version: String,
        audit: String,
        isolation: String,
        raw_line: String,
    }

    const INSTANCE_GLOBALS: &[(&str, &str)] = &[("post", "/admin/v1/cluster/reload")];
    let methods = ["get", "post", "put", "patch", "delete"];
    let mut lines: Vec<Row> = Vec::new();
    for line in text.lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() != 9 || cells[1..8].iter().any(|cell| cell.is_empty()) {
            continue;
        }
        let method = cells[2].to_lowercase();
        if !methods.contains(&method.as_str()) {
            continue;
        }
        lines.push(Row {
            method,
            route: cells[3].trim_matches('`').to_owned(),
            cli: cells[4].to_owned(),
            version: cells[5].to_owned(),
            audit: cells[6].to_owned(),
            isolation: cells[7].to_owned(),
            raw_line: line.trim().to_owned(),
        });
    }
    assert!(
        lines.len() >= 60,
        "a tabela do inventário tem só {} linha(s)",
        lines.len()
    );
    let documentadas: BTreeSet<(String, String)> = lines
        .iter()
        .map(|line| (line.method.clone(), line.route.clone()))
        .collect();
    assert_eq!(
        documentadas.len(),
        lines.len(),
        "rota repetida na tabela do inventário"
    );

    let mut published: BTreeSet<(String, String)> = BTreeSet::new();
    for (route, operations) in document["paths"].as_object().expect("paths") {
        if !route.starts_with("/admin/v1/") {
            continue;
        }
        for method in methods {
            if operations.get(method).is_some() {
                published.insert((method.to_owned(), route.clone()));
            }
        }
    }
    assert_eq!(
        documentadas, published,
        "docs/API.md e o OpenAPI divergem (documentar rota nova faz parte da entrega)"
    );

    for line in &lines {
        let instance_global = INSTANCE_GLOBALS
            .iter()
            .any(|(method, route)| *method == line.method && *route == line.route);
        let write_with_if_match = line.method == "patch"
            || line.route.ends_with("/reopen")
            || line.route.ends_with("/close")
            || line.route.ends_with("/whatsapp/qrcode")
            || line.route.ends_with("/whatsapp/disconnect")
            || (line.method == "put" && line.route.ends_with("/matrix"))
            || (line.method == "put" && line.route.ends_with("/agents/{agent_id}/flows"))
            || line.route.ends_with("/flows/{flow_id}/publish")
            || (line.method == "post" && line.route.ends_with("/flow-runs"))
            || line.route.ends_with("/flow-runs/{run_id}/actions")
            || line.route.ends_with("/flow-runs/{run_id}/abandon");
        let expected = if write_with_if_match && !instance_global {
            "sim"
        } else {
            "n/a"
        };
        assert_eq!(
            line.version, expected,
            "coluna 412 errada em `{}`",
            line.raw_line
        );

        let so_calcula =
            line.route.ends_with("/flows/validate") || line.route.ends_with("/flows/format");
        let write =
            matches!(line.method.as_str(), "post" | "put" | "patch" | "delete") && !so_calcula;
        assert_eq!(
            line.audit,
            if write && !instance_global {
                "sim"
            } else {
                "n/a"
            },
            "coluna auditoria errada em `{}`",
            line.raw_line
        );

        assert_eq!(
            line.isolation,
            if instance_global { "n/a" } else { "sim" },
            "isolamento em `{}`",
            line.raw_line
        );

        assert!(
            line.cli == "sim" || line.cli.starts_with("exceção: "),
            "coluna CLI precisa de `sim` ou `exceção: <motivo>` em `{}`",
            line.raw_line
        );
    }
}

#[tokio::test]
async fn close_conversation_returns_expected_statuses_and_audits_once() {
    let c = Scenario::new("fechar-conversa").await;
    let org_a = c.create_org("Org A", "fecha-a").await;
    let org_b = c.create_org("Org B", "fecha-b").await;
    let a = org_a["id"].as_str().unwrap();
    let b = org_b["id"].as_str().unwrap();
    let token_a = c.create_token(a, "org a").await;
    let token_b = c.create_token(b, "org b").await;
    let org: poph_types::OrgId = b.parse().unwrap();
    let agent = c
        .store()
        .agents()
        .create(
            org,
            poph_store::NewAgent {
                name: "Agente".into(),
                slug: "agente".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let conversation = c
        .store()
        .conversations()
        .create(
            org,
            poph_store::NewConversation {
                agent_id: agent.id,
                integration_id: None,
                canal: "cli".into(),
                peer_externo: None,
                target_department: None,
            },
        )
        .unwrap();
    let route = format!("/admin/v1/orgs/{b}/conversations/{}/close", conversation.id);
    let close_audits = |c: &Scenario| {
        c.store()
            .audit()
            .list(org, poph_store::Page::default())
            .unwrap()
            .iter()
            .filter(|r| {
                r.acao == "close" && r.entity_id.as_deref() == Some(&conversation.id.to_string())
            })
            .count()
    };
    let server_state = |c: &Scenario| {
        let current = c.store().conversations().get(org, conversation.id).unwrap();
        (current.status.as_text(), current.version)
    };

    assert_eq!(
        c.request(Method::POST, &route, "", None, Some(1)).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        c.request(Method::POST, &route, &token_a, None, Some(1))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        c.request(Method::POST, &route, &token_b, None, None)
            .await
            .0,
        StatusCode::PRECONDITION_REQUIRED
    );
    assert_eq!(
        c.request(Method::POST, &route, &token_b, None, Some(99))
            .await
            .0,
        StatusCode::PRECONDITION_FAILED
    );
    assert_eq!(server_state(&c), ("open", 1));
    assert_eq!(close_audits(&c), 0);
    let missing = format!(
        "/admin/v1/orgs/{b}/conversations/{}/close",
        poph_types::ConversationId::generate()
    );
    assert_eq!(
        c.request(Method::POST, &missing, &token_b, None, Some(1))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, body) = c
        .request(Method::POST, &route, &token_b, None, Some(1))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "closed");
    assert_eq!(body["version"], 2);
    assert_eq!(server_state(&c), ("closed", 2));
    assert_eq!(close_audits(&c), 1, "uma auditoria `close` por fechamento");
    assert_eq!(
        c.request(Method::POST, &route, &token_b, None, Some(2))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(server_state(&c), ("closed", 2), "409 não sobe a versão");
    assert_eq!(close_audits(&c), 1, "409 não grava auditoria");
    let record = c
        .store()
        .audit()
        .list(org, poph_store::Page::default())
        .unwrap()
        .into_iter()
        .find(|r| r.acao == "close")
        .expect("fechamento auditado");
    assert_eq!(
        record.actor_id.as_deref(),
        token_b
            .strip_prefix("poph_")
            .and_then(|s| s.split_once('_'))
            .map(|(id, _)| id)
    );
}

#[tokio::test]
async fn message_after_closure_opens_new_conversation() {
    let c = Scenario::new("fechar-e-reabrir-por-mensagem").await;
    let org_a = c.create_org("Org A", "fim-a").await;
    let a = org_a["id"].as_str().unwrap();
    let token = c.create_token(a, "org a").await;
    let org: poph_types::OrgId = a.parse().unwrap();
    let agent = c
        .store()
        .agents()
        .create(
            org,
            poph_store::NewAgent {
                name: "Agente".into(),
                slug: "agente".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let integration = c
        .store()
        .create_integration(org, agent.id, "whatsapp".into(), "active".into())
        .unwrap()
        .id
        .parse::<poph_types::IntegrationId>()
        .unwrap();
    let entry = |external_id: &str| poph_store::NewChannelInboxItem {
        integration_id: integration,
        agent_id: agent.id,
        canal: "whatsapp".to_owned(),
        external_id: external_id.to_owned(),
        peer: "5521999990000".to_owned(),
        event_json: "{}".to_owned(),
        texto: "oi".to_owned(),
        attachments: Vec::new(),
        customer_name: None,
    };
    let first = c
        .store()
        .inbox()
        .receive_from_channel(org, entry("MSG-1"))
        .unwrap();
    let conversation_1 = first.mensagem.conversation_id;
    let version = c
        .store()
        .conversations()
        .get(org, conversation_1)
        .unwrap()
        .version;
    let (status, _) = c
        .request(
            Method::POST,
            &format!("/admin/v1/orgs/{a}/conversations/{conversation_1}/close"),
            &token,
            None,
            Some(version),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let second = c
        .store()
        .inbox()
        .receive_from_channel(org, entry("MSG-2"))
        .unwrap();
    let conversation_2 = second.mensagem.conversation_id;
    assert_ne!(
        conversation_2, conversation_1,
        "a mensagem seguinte abre conversa nova"
    );
    assert_eq!(
        c.store()
            .conversations()
            .get(org, conversation_1)
            .unwrap()
            .status,
        poph_store::ConversationStatus::Closed
    );
    assert_eq!(
        c.store()
            .conversations()
            .get(org, conversation_2)
            .unwrap()
            .status,
        poph_store::ConversationStatus::Open
    );
}

#[tokio::test]
async fn reopen_closed_conversation_returns_conflict() {
    let c = Scenario::new("reabrir-encerrada").await;
    let org_a = c.create_org("Org A", "fim-reabrir").await;
    let a = org_a["id"].as_str().unwrap();
    let token = c.create_token(a, "org a").await;
    let org: poph_types::OrgId = a.parse().unwrap();
    let agent = c
        .store()
        .agents()
        .create(
            org,
            poph_store::NewAgent {
                name: "Agente".into(),
                slug: "agente".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let conversation = c
        .store()
        .conversations()
        .create(
            org,
            poph_store::NewConversation {
                agent_id: agent.id,
                integration_id: None,
                canal: "console".to_owned(),
                peer_externo: Some("cliente-1".to_owned()),
                target_department: None,
            },
        )
        .unwrap();

    let (status, _) = c
        .request(
            Method::POST,
            &format!("/admin/v1/orgs/{a}/conversations/{}/close", conversation.id),
            &token,
            None,
            Some(conversation.version),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let closed = c.store().conversations().get(org, conversation.id).unwrap();
    let (status, body) = c
        .request(
            Method::POST,
            &format!(
                "/admin/v1/orgs/{a}/conversations/{}/reopen",
                conversation.id
            ),
            &token,
            None,
            Some(closed.version),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "conversa_encerrada");

    let after = c.store().conversations().get(org, conversation.id).unwrap();
    assert_eq!(after.status, poph_store::ConversationStatus::Closed);
    assert_eq!(after.version, closed.version, "a recusa não muda a versão");
    let service_case = c
        .store()
        .cases()
        .by_conversation(org, conversation.id)
        .unwrap()
        .expect("a conversa fechada pelo administrador tem Ficha fechada");
    assert_eq!(service_case.status, poph_types::RecordStatus::Closed);
    assert_eq!(service_case.fechada_por, Some(poph_types::ClosedBy::Admin));
}
