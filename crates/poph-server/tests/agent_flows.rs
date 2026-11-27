use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::ServerState;
use poph_store::{NewFlow, Page, Store};
use poph_types::config::GlobalTokenConfig;
use poph_types::{AgentId, FlowId, HarnessConfig, OrgId};
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-fx28";

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
    folder: PathBuf,
    store: Arc<Store>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Scenario {
    async fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let folder = std::env::temp_dir().join(format!(
            "poph-server-{label}-{}-{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(folder.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = folder.join("dados");
        config.server.port = 7481;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let estado = ServerState::new(Arc::clone(&store), config).expect("estado do servidor");

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

    fn store(&self) -> &Store {
        &self.store
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Option<Value>,
        if_match: Option<u64>,
    ) -> (StatusCode, Value) {
        let mut http_request = self
            .client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(token);
        if let Some(versao) = if_match {
            http_request = http_request.header("If-Match", versao.to_string());
        }
        if let Some(value) = body {
            http_request = http_request.json(&value);
        }
        let response = http_request.send().await.expect("pedido HTTP");
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
            .request(
                Method::POST,
                "/admin/v1/orgs",
                TOKEN_GLOBAL,
                Some(json!({ "name": nome, "slug": slug })),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar org: {body}");
        body["id"].as_str().expect("id da org").to_owned()
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
        body["value"].as_str().expect("valor do token").to_owned()
    }

    async fn create_agent(&self, org_id: &str, token: &str, slug: &str) -> String {
        let (status, body) = self
            .request(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/agents"),
                token,
                Some(json!({ "name": format!("Agente {slug}"), "slug": slug })),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar agente: {body}");
        body["id"].as_str().expect("id do agente").to_owned()
    }

    fn create_flow(&self, org_id: &str, slug: &str) -> String {
        let org: OrgId = org_id.parse().expect("org válida");
        self.store()
            .flows()
            .create(
                org,
                NewFlow {
                    slug: slug.to_owned(),
                    name: format!("Fluxo {slug}"),
                    description: String::new(),
                    rascunho_fonte: String::new(),
                },
            )
            .expect("cria o fluxo")
            .id
            .to_string()
    }

    fn agent_version(&self, org_id: &str, agent_id: &str) -> u64 {
        let org: OrgId = org_id.parse().expect("org válida");
        let agente: AgentId = agent_id.parse().expect("agente válido");
        self.store()
            .agents()
            .version(org, agente)
            .expect("versão do agente")
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

fn id_do_token(token: &str) -> &str {
    token
        .strip_prefix("poph_")
        .expect("prefixo do token")
        .split_once('_')
        .expect("separador do token")
        .0
}

fn slugs(body: &Value) -> Vec<String> {
    body["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["slug"].as_str().expect("slug").to_owned())
        .collect()
}

#[tokio::test]
async fn agent_flows_list_replace_version_cap_isolation_and_audit() {
    let c = Scenario::new("agente-fluxos").await;
    let oa = c.create_org("Org A", "fx28-org-a").await;
    let ob = c.create_org("Org B", "fx28-org-b").await;
    let ta = c.create_token(&oa, "a").await;
    let tb = c.create_token(&ob, "b").await;
    let ag = c.create_agent(&oa, &ta, "agente-a").await;
    let agb = c.create_agent(&ob, &tb, "agente-b").await;
    let f1 = c.create_flow(&oa, "fluxo_a1");
    let f2 = c.create_flow(&oa, "fluxo_a2");
    let fb = c.create_flow(&ob, "fluxo_b1");
    let route = format!("/admin/v1/orgs/{oa}/agents/{ag}/flows");

    let (s, body) = c.request(Method::GET, &route, &ta, None, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["items"], json!([]));
    assert_eq!(body["next_cursor"], Value::Null);

    let (s, body) = c
        .request(
            Method::PUT,
            &route,
            &ta,
            Some(json!({"flow_ids": [f1]})),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::PRECONDITION_REQUIRED, "{body}");
    let (s, body) = c
        .request(
            Method::PUT,
            &route,
            &ta,
            Some(json!({"flow_ids": [f1]})),
            Some(99),
        )
        .await;
    assert_eq!(s, StatusCode::PRECONDITION_FAILED, "{body}");
    assert_eq!(code(&body), "version_conflict");
    assert_eq!(c.agent_version(&oa, &ag), 1);

    let (s, body) = c
        .request(
            Method::PUT,
            &route,
            &ta,
            Some(json!({"flow_ids": [f1, f2]})),
            Some(1),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["version"], 2);
    assert_eq!(body["added"].as_array().unwrap().len(), 2);
    assert_eq!(body["removed"], json!([]));
    let (s, read_back) = c.request(Method::GET, &route, &ta, None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(slugs(&read_back), vec!["fluxo_a1", "fluxo_a2"]);

    let (s, body) = c
        .request(
            Method::PUT,
            &route,
            &ta,
            Some(json!({"flow_ids": [f1]})),
            Some(2),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["version"], 3);
    assert_eq!(body["added"], json!([]));
    assert_eq!(body["removed"][0]["id"], f2);

    let (s, body) = c
        .request(
            Method::PUT,
            &route,
            &ta,
            Some(json!({"flow_ids": []})),
            Some(3),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["version"], 4);
    assert_eq!(body["removed"][0]["id"], f1);
    let (_, read_back) = c.request(Method::GET, &route, &ta, None, None).await;
    assert_eq!(read_back["items"], json!([]));

    let (s, body) = c
        .request(
            Method::PUT,
            &route,
            &ta,
            Some(json!({"flow_ids": []})),
            Some(4),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["version"], 5);
    assert_eq!(body["added"], json!([]));
    assert_eq!(body["removed"], json!([]));

    let (s, body) = c
        .request(
            Method::PUT,
            &route,
            &ta,
            Some(json!({"flow_ids": [f1, fb]})),
            Some(5),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        c.agent_version(&oa, &ag),
        5,
        "versão não pode subir num 404"
    );
    let (_, read_back) = c.request(Method::GET, &route, &ta, None, None).await;
    assert_eq!(read_back["items"], json!([]));

    let missing = FlowId::generate().to_string();
    let (s, body) = c
        .request(
            Method::PUT,
            &route,
            &ta,
            Some(json!({"flow_ids": [missing]})),
            Some(5),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{body}");

    let (s, body) = c
        .request(
            Method::PUT,
            &route,
            &ta,
            Some(json!({"flow_ids": ["abc"]})),
            Some(5),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(code(&body), "invalid_id");
    let (s, body) = c
        .request(Method::PUT, &route, &ta, Some(json!({})), Some(5))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(code(&body), "invalid_request");
    assert_eq!(c.agent_version(&oa, &ag), 5);

    let muitos: Vec<String> = (1..=21)
        .map(|i| c.create_flow(&oa, &format!("fluxo_t{i:02}")))
        .collect();
    let (s, body) = c
        .request(
            Method::PUT,
            &route,
            &ta,
            Some(json!({"flow_ids": muitos})),
            Some(5),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(code(&body), "teto_de_fluxos_por_agente");
    assert_eq!(c.agent_version(&oa, &ag), 5);
    let (s, body) = c
        .request(
            Method::PUT,
            &route,
            &ta,
            Some(json!({"flow_ids": &muitos[..20]})),
            Some(5),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["version"], 6);
    assert_eq!(body["added"].as_array().unwrap().len(), 20);

    let (s, body) = c.request(Method::GET, &route, &tb, None, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    let (s, body) = c
        .request(
            Method::PUT,
            &route,
            &tb,
            Some(json!({"flow_ids": []})),
            Some(6),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    let route_b = format!("/admin/v1/orgs/{oa}/agents/{agb}/flows");
    let (s, body) = c.request(Method::GET, &route_b, &ta, None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(c.agent_version(&ob, &agb), 1, "agente de B não foi mexido");

    let (s, body) = c
        .request(
            Method::PATCH,
            &format!("/admin/v1/orgs/{oa}/agents/{ag}"),
            &ta,
            Some(json!({ "status": "inactive" })),
            Some(c.agent_version(&oa, &ag)),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, body) = c
        .request(
            Method::PUT,
            &route,
            &ta,
            Some(json!({"flow_ids": [f1]})),
            Some(c.agent_version(&oa, &ag)),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");

    let records = c
        .store()
        .audit()
        .list(oa.parse().unwrap(), Page::default())
        .unwrap();
    let from_agent: Vec<_> = records
        .iter()
        .filter(|r| {
            r.acao == "agent_flows_update"
                && r.entity == "agent"
                && r.entity_id.as_deref() == Some(ag.as_str())
        })
        .collect();
    assert_eq!(from_agent.len(), 6, "um registro por PUT bem-sucedido");
    for registro in from_agent {
        assert_eq!(registro.ator_tipo, "admin_token");
        assert_eq!(registro.actor_id.as_deref(), Some(id_do_token(&ta)));
        assert!(!format!("{registro:?}").contains(&ta));
    }

    let (s, body) = c
        .request(
            Method::PUT,
            &route,
            &ta,
            Some(json!({"flow_ids": [f1, f1]})),
            Some(c.agent_version(&oa, &ag)),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["added"], json!([]), "{body}");
    assert_eq!(body["removed"], json!([]), "{body}");
    let (_, read_back) = c.request(Method::GET, &route, &ta, None, None).await;
    assert_eq!(slugs(&read_back), vec!["fluxo_a1"]);
}
