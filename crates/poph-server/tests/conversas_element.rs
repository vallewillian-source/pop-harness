use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::time::Duration;

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use axum::http::StatusCode;
use poph_server::ServerState;
use poph_store::{Agente, Conversa, NewAgent, NewConversation, NewOrg, Org, Store};
use poph_types::{AgentId, ConversationId, HarnessConfig, OrgId};
use rand_core::OsRng;
use reqwest::{Client, Method};
use serde_json::Value;
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
            "poph-conversas-element-{label}-{}-{n}",
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

    fn agente(&self, org: &Org, slug: &str, nome: &str) -> Agente {
        self.store
            .agents()
            .create(
                org.id,
                NewAgent {
                    name: nome.to_owned(),
                    slug: slug.to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
    }

    fn conversa(&self, org: &Org, agente: AgentId) -> Conversa {
        self.store
            .conversations()
            .create(
                org.id,
                NewConversation {
                    agent_id: agente,
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some("5511999999999".to_owned()),
                    target_department: None,
                },
            )
            .expect("cria a conversa")
    }

    fn room(&self, org: &Org, conversa: ConversationId, room: &str, ready: bool) {
        self.store
            .matrix()
            .ensure_room(org.id, conversa, room, None)
            .expect("garante a sala");
        if ready {
            self.store
                .matrix()
                .mark_room_ready(org.id, conversa)
                .expect("sala pronta");
        }
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
    if_match: Option<&str>,
) -> Response {
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("cliente HTTP");
    let mut call = client
        .request(method, format!("{base}{path}"))
        .bearer_auth(token);
    if let Some(etag) = if_match {
        call = call.header("If-Match", etag);
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

async fn list_page(base: &str, org: OrgId, token: &str, limit: u32) -> Vec<Value> {
    let response = request(
        base,
        Method::GET,
        &format!("/admin/v1/orgs/{org}/conversations?limit={limit}"),
        token,
        None,
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "lista: {}", response.texto);
    response.json["items"]
        .as_array()
        .cloned()
        .expect("items da lista")
}

fn item(items: &[Value], id: ConversationId) -> &Value {
    items
        .iter()
        .find(|v| v["id"] == id.to_string())
        .expect("conversa na lista")
}

#[tokio::test]
async fn detail_close_reopen_and_list_return_name_agent_and_ready_room() {
    let c = Scenario::new("basico");
    let org = c.org("a");
    let token = c.token(org.id, "a");
    let agente = c.agente(&org, "bia", "Bia");
    let conversa = c.conversa(&org, agente.id);
    c.store
        .conversations()
        .set_customer_name(org.id, conversa.id, "Ana Souza")
        .expect("nome do cliente");
    c.store.matrix().enable(org.id).expect("liga o módulo");
    c.room(&org, conversa.id, "!ana:matrix-dev.local", true);
    let (base, desligar) = serve(c.estado()).await;

    let path = format!("/admin/v1/orgs/{}/conversations/{}", org.id, conversa.id);
    let detail = request(&base, Method::GET, &path, &token, None).await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.texto);
    assert_eq!(detail.json["client_name"], "Ana Souza");
    assert_eq!(detail.json["agent_name"], "Bia");
    assert_eq!(detail.json["matrix_room_id"], "!ana:matrix-dev.local");

    let items = list_page(&base, org.id, &token, 50).await;
    let in_list = item(&items, conversa.id);
    assert_eq!(in_list["client_name"], "Ana Souza");
    assert_eq!(in_list["agent_name"], "Bia");
    assert_eq!(in_list["matrix_room_id"], "!ana:matrix-dev.local");

    let closed = request(
        &base,
        Method::POST,
        &format!("{path}/close"),
        &token,
        Some("1"),
    )
    .await;
    assert_eq!(closed.status, StatusCode::OK, "{}", closed.texto);
    assert_eq!(closed.json["client_name"], "Ana Souza");
    assert_eq!(closed.json["matrix_room_id"], "!ana:matrix-dev.local");
    assert_eq!(closed.json["agent_name"], "Bia");

    let refused = request(
        &base,
        Method::POST,
        &format!("{path}/reopen"),
        &token,
        Some("2"),
    )
    .await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.texto);
    assert_eq!(refused.json["code"], "conversa_encerrada");

    c.store
        .conversations()
        .change_status(
            org.id,
            conversa.id,
            c.store
                .conversations()
                .get(org.id, conversa.id)
                .unwrap()
                .version,
            poph_store::ConversationStatus::Transferred,
            None,
        )
        .expect("transfere para humano");
    let transferred_version = c
        .store
        .conversations()
        .get(org.id, conversa.id)
        .unwrap()
        .version
        .to_string();
    let reopened = request(
        &base,
        Method::POST,
        &format!("{path}/reopen"),
        &token,
        Some(&transferred_version),
    )
    .await;
    assert_eq!(reopened.status, StatusCode::OK, "{}", reopened.texto);
    assert_eq!(reopened.json["matrix_room_id"], "!ana:matrix-dev.local");
    assert_eq!(reopened.json["client_name"], "Ana Souza");

    let _ = desligar.send(());
}

#[tokio::test]
async fn creating_room_empty_name_and_disabled_module_become_null() {
    let c = Scenario::new("nulos");
    let a = c.org("a");
    let b = c.org("b");
    let d = c.org("c");
    let token_a = c.token(a.id, "a");
    let token_b = c.token(b.id, "b");
    let token_c = c.token(d.id, "c");
    let agent_a = c.agente(&a, "bia", "Bia");
    let agent_b = c.agente(&b, "bia", "Bia");
    let agent_c = c.agente(&d, "bia", "Bia");

    c.store.matrix().enable(a.id).expect("liga A");
    let without_name = c.conversa(&a, agent_a.id);
    c.room(&a, without_name.id, "!criando:matrix-dev.local", false);

    let never_enabled = c.conversa(&b, agent_b.id);
    c.store
        .conversations()
        .set_customer_name(b.id, never_enabled.id, "Bruno Lima")
        .expect("nome do cliente");
    c.room(&b, never_enabled.id, "!pronta-b:matrix-dev.local", true);

    c.store.matrix().enable(d.id).expect("liga C");
    c.store.matrix().disable(d.id).expect("desliga C");
    let disabled = c.conversa(&d, agent_c.id);
    c.store
        .conversations()
        .set_customer_name(d.id, disabled.id, "Carla Dias")
        .expect("nome do cliente");
    c.room(&d, disabled.id, "!pronta-c:matrix-dev.local", true);

    let (base, desligar) = serve(c.estado()).await;

    let path = format!("/admin/v1/orgs/{}/conversations/{}", a.id, without_name.id);
    let detail = request(&base, Method::GET, &path, &token_a, None).await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.texto);
    assert_eq!(detail.json["client_name"], Value::Null, "nome vazio é null");
    assert_eq!(
        detail.json["matrix_room_id"],
        Value::Null,
        "sala em creating não sai"
    );
    assert_eq!(detail.json["agent_name"], "Bia");
    let items = list_page(&base, a.id, &token_a, 50).await;
    assert_eq!(item(&items, without_name.id)["matrix_room_id"], Value::Null);
    assert_eq!(item(&items, without_name.id)["client_name"], Value::Null);

    let path = format!("/admin/v1/orgs/{}/conversations/{}", b.id, never_enabled.id);
    let detail = request(&base, Method::GET, &path, &token_b, None).await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.texto);
    assert_eq!(detail.json["client_name"], "Bruno Lima");
    assert_eq!(
        detail.json["matrix_room_id"],
        Value::Null,
        "módulo nunca ligado na org: sala não sai"
    );
    let items = list_page(&base, b.id, &token_b, 50).await;
    assert_eq!(
        item(&items, never_enabled.id)["matrix_room_id"],
        Value::Null
    );

    let path = format!("/admin/v1/orgs/{}/conversations/{}", d.id, disabled.id);
    let detail = request(&base, Method::GET, &path, &token_c, None).await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.texto);
    assert_eq!(detail.json["client_name"], "Carla Dias");
    assert_eq!(
        detail.json["matrix_room_id"],
        Value::Null,
        "módulo desligado depois de ligado: sala não sai"
    );
    let items = list_page(&base, d.id, &token_c, 50).await;
    assert_eq!(item(&items, disabled.id)["matrix_room_id"], Value::Null);
    assert_eq!(item(&items, disabled.id)["client_name"], "Carla Dias");

    let _ = desligar.send(());
}

#[tokio::test]
async fn list_reads_names_and_rooms_in_batch_without_query_per_row() {
    let c = Scenario::new("lote");
    let org = c.org("a");
    let token = c.token(org.id, "a");
    c.store.matrix().enable(org.id).expect("liga");
    for n in 0..30 {
        let agente = c.agente(&org, &format!("ag-{n}"), &format!("Agente {n}"));
        let conversa = c.conversa(&org, agente.id);
        c.store
            .conversations()
            .set_customer_name(org.id, conversa.id, &format!("Cliente {n}"))
            .expect("nome");
        if n % 2 == 0 {
            c.room(
                &org,
                conversa.id,
                &format!("!sala{n}:matrix-dev.local"),
                true,
            );
        }
    }
    let (base, desligar) = serve(c.estado()).await;

    let before = c.store.reads_done();
    let small = list_page(&base, org.id, &token, 2).await;
    let small_reads = c.store.reads_done() - before;

    let before = c.store.reads_done();
    let large = list_page(&base, org.id, &token, 30).await;
    let large_reads = c.store.reads_done() - before;

    assert_eq!(small.len(), 2);
    assert_eq!(large.len(), 30);
    assert_eq!(
        small_reads, large_reads,
        "a página de 30 gastou {large_reads} leituras e a de 2 gastou {small_reads}: \
         a lista não pode fazer uma consulta por linha"
    );
    assert!(
        large_reads <= 5,
        "a página inteira devia gastar poucas leituras, gastou {large_reads}"
    );

    for entry in &large {
        assert!(entry["agent_name"].is_string(), "agent_name em todo item");
        let client_name = entry["client_name"].as_str().expect("client_name");
        let numero: usize = client_name
            .trim_start_matches("Cliente ")
            .parse()
            .expect("número do cliente");
        if numero.is_multiple_of(2) {
            assert_eq!(
                entry["matrix_room_id"],
                format!("!sala{numero}:matrix-dev.local")
            );
        } else {
            assert_eq!(entry["matrix_room_id"], Value::Null);
        }
    }

    let _ = desligar.send(());
}

#[tokio::test]
async fn conversation_from_other_org_stays_404_and_does_not_leak_name() {
    let c = Scenario::new("isolamento");
    let a = c.org("a");
    let b = c.org("b");
    let token_a = c.token(a.id, "a");
    let agent_b = c.agente(&b, "bia", "Bia da B");
    let conversation_b = c.conversa(&b, agent_b.id);
    c.store
        .conversations()
        .set_customer_name(b.id, conversation_b.id, "Cliente Secreto B")
        .expect("nome");
    c.store.matrix().enable(b.id).expect("liga a B");
    c.room(&b, conversation_b.id, "!secreta:matrix-dev.local", true);
    let (base, desligar) = serve(c.estado()).await;

    let through_a = request(
        &base,
        Method::GET,
        &format!(
            "/admin/v1/orgs/{}/conversations/{}",
            a.id, conversation_b.id
        ),
        &token_a,
        None,
    )
    .await;
    assert_eq!(
        through_a.status,
        StatusCode::NOT_FOUND,
        "{}",
        through_a.texto
    );
    assert!(
        !through_a.texto.contains("Cliente Secreto B")
            && !through_a.texto.contains("matrix-dev.local"),
        "o 404 não pode trazer dado da outra org"
    );

    let through_b_path = request(
        &base,
        Method::GET,
        &format!(
            "/admin/v1/orgs/{}/conversations/{}",
            b.id, conversation_b.id
        ),
        &token_a,
        None,
    )
    .await;
    assert_eq!(through_b_path.status, StatusCode::FORBIDDEN);

    let _ = desligar.send(());
}
