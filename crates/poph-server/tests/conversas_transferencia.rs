use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::time::Duration;

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use axum::http::StatusCode;
use poph_server::ServerState;
use poph_store::{Agente, Conversa, NewAgent, NewConversation, NewDepartment, NewOrg, Org, Store};
use poph_types::{AgentId, HarnessConfig, OrgId};
use rand_core::OsRng;
use reqwest::{Client, Method};
use serde_json::{Value, json};
use tokio::sync::oneshot;

struct Scenario {
    dir: PathBuf,
    store: Arc<Store>,
    config: HarnessConfig,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "poph-conversas-transferencia-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dados")).expect("pasta de dados");
        let mut config = HarnessConfig::default();
        config.data.directory = dir.join("dados");
        config.validar().expect("configuração válida");
        let clock = Arc::new(AtomicI64::new(1_000_000));
        let store = Store::open(&config)
            .expect("abre o banco")
            .with_clock(Arc::new(move || clock.fetch_add(10, Ordering::SeqCst)));
        Self {
            dir,
            store: Arc::new(store),
            config,
        }
    }

    fn estado(&self) -> ServerState {
        ServerState::new(Arc::clone(&self.store), self.config.clone()).expect("estado do servidor")
    }

    fn org(&self, slug: &str) -> Org {
        self.store
            .orgs()
            .create(NewOrg {
                name: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
    }

    fn token(&self, org: OrgId, label: &str) -> String {
        let secret = format!("segredo-{label}");
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(secret.as_bytes(), &salt)
            .expect("hash argon2")
            .to_string();
        let created = self
            .store
            .tokens()
            .create(Some(org), label, &hash)
            .expect("cria o token");
        format!("poph_{}_{secret}", created.id)
    }

    fn agente(&self, org: &Org, slug: &str) -> Agente {
        self.store
            .agents()
            .create(
                org.id,
                NewAgent {
                    name: format!("Agente {slug}"),
                    slug: slug.to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
    }

    fn departamento(&self, org: &Org, nome: &str) -> poph_types::DepartmentId {
        self.store
            .departments()
            .create(
                org.id,
                NewDepartment {
                    name: nome.to_owned(),
                    description: String::new(),
                },
            )
            .expect("cria o departamento")
            .id
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

async fn serve(estado: ServerState) -> (String, oneshot::Sender<()>) {
    let app = poph_server::router(estado);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = format!("http://{}", listener.local_addr().expect("endereço"));
    let (desligar, signal) = oneshot::channel();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = signal.await;
            })
            .await;
    });
    (endereco, desligar)
}

struct Response {
    status: StatusCode,
    json: Value,
    texto: String,
}

async fn request(
    base: &str,
    method: Method,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> Response {
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("cliente HTTP");
    let mut call = client
        .request(method, format!("{base}{path}"))
        .bearer_auth(token);
    if let Some(payload) = body {
        call = call.json(&payload);
    }
    let response = call.send().await.expect("pedido HTTP");
    let status = response.status();
    let texto = response.text().await.unwrap_or_default();
    let json = serde_json::from_str(&texto).unwrap_or(Value::Null);
    Response {
        status,
        json,
        texto,
    }
}

fn conversa(c: &Scenario, org: &Org, agente: AgentId, peer: &str) -> Conversa {
    c.store
        .conversations()
        .create(
            org.id,
            NewConversation {
                agent_id: agente,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: Some(peer.to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa")
}

fn path_for(org: OrgId, conversa: impl std::fmt::Display, acao: &str) -> String {
    format!("/admin/v1/orgs/{org}/conversations/{conversa}/{acao}")
}

#[tokio::test]
async fn transfer_marks_conversation_and_audits_reason() {
    let c = Scenario::new("feliz");
    let org = c.org("a");
    let token = c.token(org.id, "a");
    let agente = c.agente(&org, "bia");
    let destination = c.departamento(&org, "Comercial");
    let target = conversa(&c, &org, agente.id, "5511");
    let (base, desligar) = serve(c.estado()).await;

    let response = request(
        &base,
        Method::POST,
        &path_for(org.id, target.id, "transfer"),
        &token,
        Some(json!({ "department_id": destination.to_string(), "reason": "rotina" })),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.texto);
    assert_eq!(response.json["id"], target.id.to_string());
    assert_eq!(response.json["status"], "transferred");
    assert_eq!(
        response.json["target_department_id"],
        destination.to_string()
    );

    let stored = c
        .store
        .conversations()
        .get(org.id, target.id)
        .expect("lê a conversa");
    assert_eq!(stored.status, poph_store::ConversationStatus::Transferred);
    assert_eq!(stored.target_department, Some(destination));
    assert_eq!(stored.version, target.version + 1);

    let audit = c
        .store
        .audit()
        .list(org.id, poph_store::Page::with_limit(10))
        .expect("lê a auditoria");
    let registro = audit
        .iter()
        .find(|registro| registro.acao == "transfer")
        .expect("a transferência é auditada");
    assert_eq!(registro.entity, "conversation");
    assert_eq!(
        registro.entity_id.as_deref(),
        Some(target.id.to_string().as_str())
    );
    assert_eq!(
        registro.detail_json,
        json!({
            "motivo": "rotina",
            "ator": "administrador",
            "department_id": destination.to_string(),
        })
        .to_string()
    );

    let _ = desligar.send(());
}

#[tokio::test]
async fn transfer_refuses_closed_conversation_from_other_org_and_invalid_body() {
    let c = Scenario::new("recusas");
    let org = c.org("a");
    let other_org = c.org("b");
    let token = c.token(org.id, "a");
    let token_b = c.token(other_org.id, "b");
    let agente = c.agente(&org, "bia");
    let agent_b = c.agente(&other_org, "leo");
    let destination = c.departamento(&org, "Comercial");
    let destination_b = c.departamento(&other_org, "Comercial");
    let target = conversa(&c, &org, agente.id, "5511");
    let from_other_org = conversa(&c, &other_org, agent_b.id, "5522");
    let closed_conversation = conversa(&c, &org, agente.id, "5533");
    c.store
        .conversations()
        .close_by_agent(org.id, closed_conversation.id, agente.id, "fim".to_owned())
        .expect("fecha a conversa");
    let (base, desligar) = serve(c.estado()).await;

    let body = json!({ "department_id": destination.to_string(), "reason": "rotina" });
    let response = request(
        &base,
        Method::POST,
        &path_for(org.id, from_other_org.id, "transfer"),
        &token,
        Some(body.clone()),
    )
    .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND, "{}", response.texto);
    assert_eq!(response.json["code"], "not_found");

    let response = request(
        &base,
        Method::POST,
        &path_for(other_org.id, target.id, "transfer"),
        &token_b,
        Some(json!({ "department_id": destination_b.to_string(), "reason": "rotina" })),
    )
    .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND, "{}", response.texto);

    let response = request(
        &base,
        Method::POST,
        &path_for(org.id, closed_conversation.id, "transfer"),
        &token,
        Some(body.clone()),
    )
    .await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{}", response.texto);
    assert_eq!(response.json["code"], "conversa_encerrada");

    for (label, bad_body) in [
        (
            "sem motivo",
            json!({ "department_id": destination.to_string() }),
        ),
        (
            "motivo vazio",
            json!({ "department_id": destination.to_string(), "reason": "  " }),
        ),
        (
            "departamento malformado",
            json!({ "department_id": "nao-e-id", "reason": "rotina" }),
        ),
    ] {
        let response = request(
            &base,
            Method::POST,
            &path_for(org.id, target.id, "transfer"),
            &token,
            Some(bad_body),
        )
        .await;
        assert_eq!(
            response.status,
            StatusCode::BAD_REQUEST,
            "{label}: {}",
            response.texto
        );
        assert_eq!(response.json["code"], "validation_error", "{label}");
    }

    let response = request(
        &base,
        Method::POST,
        &path_for(org.id, target.id, "transfer"),
        &token,
        Some(json!("nao-e-objeto")),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::BAD_REQUEST,
        "{}",
        response.texto
    );
    assert_eq!(response.json["code"], "invalid_request");

    let stored = c
        .store
        .conversations()
        .get(org.id, target.id)
        .expect("lê a conversa");
    assert_eq!(stored.status, poph_store::ConversationStatus::Open);
    assert_eq!(stored.version, target.version);

    let _ = desligar.send(());
}

#[tokio::test]
async fn list_cuts_by_last_activity_and_refuses_invalid_value() {
    let c = Scenario::new("inatividade");
    let org = c.org("a");
    let other_org = c.org("b");
    let token = c.token(org.id, "a");
    let agente = c.agente(&org, "bia");
    let agent_b = c.agente(&other_org, "leo");
    let first = conversa(&c, &org, agente.id, "5511");
    let second = conversa(&c, &org, agente.id, "5522");
    let third = conversa(&c, &org, agente.id, "5533");
    let foreign = conversa(&c, &other_org, agent_b.id, "5511");
    let (base, desligar) = serve(c.estado()).await;

    let cutoff = second.created_at + 1;
    let list_url = format!(
        "/admin/v1/orgs/{}/conversations?sort=recent&limit=50&last_activity_before={cutoff}",
        org.id
    );
    let response = request(&base, Method::GET, &list_url, &token, None).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.texto);
    let ids: Vec<&str> = response.json["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["id"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(ids, vec![second.id.to_string(), first.id.to_string()]);
    assert!(!ids.contains(&third.id.to_string().as_str()));
    assert!(!ids.contains(&foreign.id.to_string().as_str()));

    let response = request(
        &base,
        Method::GET,
        &format!(
            "/admin/v1/orgs/{}/conversations?last_activity_before=ontem",
            org.id
        ),
        &token,
        None,
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::BAD_REQUEST,
        "{}",
        response.texto
    );
    assert_eq!(response.json["code"], "invalid_request");

    let _ = desligar.send(());
}
