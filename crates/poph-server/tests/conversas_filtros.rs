use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::time::Duration;

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use axum::http::StatusCode;
use poph_server::ServerState;
use poph_store::{
    Agente, Conversa, ConversationStatus, NewAgent, NewConversation, NewOrg, Org, Store,
};
use poph_types::{AgentId, HarnessConfig, OrgId};
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

fn conversation_for_client(c: &Scenario, org: &Org, agente: AgentId, peer: &str) -> Conversa {
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

fn items_of(response: &Response) -> Vec<Value> {
    assert_eq!(response.status, StatusCode::OK, "{}", response.texto);
    response.json["items"]
        .as_array()
        .cloned()
        .expect("items da lista")
}

fn ids_of(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .map(|v| v["id"].as_str().unwrap_or_default().to_owned())
        .collect()
}

fn list_path(org: OrgId) -> String {
    format!("/admin/v1/orgs/{org}/conversations")
}

#[tokio::test]
async fn client_and_status_filters_stay_inside_org() {
    let c = Scenario::new("filtros");
    let a = c.org("a");
    let b = c.org("b");
    let token_a = c.token(a.id, "a");
    let agent_a = c.agente(&a, "bia", "Bia");
    let agent_b = c.agente(&b, "leo", "Leo");
    let c1 = conversation_for_client(&c, &a, agent_a.id, "5511");
    let c2 = conversation_for_client(&c, &a, agent_a.id, "5511");
    let c3 = conversation_for_client(&c, &a, agent_a.id, "5522");
    let d1 = conversation_for_client(&c, &b, agent_b.id, "5511");
    c.store
        .conversations()
        .transfer(a.id, c2.id, None, None, "{}".to_owned())
        .expect("transfere c2");
    c.store
        .conversations()
        .change_status(a.id, c3.id, 1, ConversationStatus::Closed, None)
        .expect("fecha c3");
    let (base, desligar) = serve(c.estado()).await;
    let list_url = list_path(a.id);

    let by_client = request(
        &base,
        Method::GET,
        &format!("{list_url}?external_peer=5511"),
        &token_a,
        None,
    )
    .await;
    let items = items_of(&by_client);
    assert_eq!(
        ids_of(&items),
        vec![c1.id.to_string(), c2.id.to_string()],
        "só as duas conversas do cliente, na org A"
    );
    assert!(
        !ids_of(&items).contains(&d1.id.to_string()),
        "a conversa do mesmo cliente na org B não aparece"
    );

    let transferred = request(
        &base,
        Method::GET,
        &format!("{list_url}?status=transferred"),
        &token_a,
        None,
    )
    .await;
    assert_eq!(ids_of(&items_of(&transferred)), vec![c2.id.to_string()]);

    let both = request(
        &base,
        Method::GET,
        &format!("{list_url}?external_peer=5511&status=open"),
        &token_a,
        None,
    )
    .await;
    assert_eq!(
        ids_of(&items_of(&both)),
        vec![c1.id.to_string()],
        "cliente e situação juntos"
    );

    let closed = request(
        &base,
        Method::GET,
        &format!("{list_url}?status=closed"),
        &token_a,
        None,
    )
    .await;
    assert_eq!(ids_of(&items_of(&closed)), vec![c3.id.to_string()]);

    let none_found = request(
        &base,
        Method::GET,
        &format!("{list_url}?external_peer=9999"),
        &token_a,
        None,
    )
    .await;
    assert!(items_of(&none_found).is_empty());

    let _ = desligar.send(());
}

#[tokio::test]
async fn sort_recent_orders_by_activity_and_returns_null_next_cursor() {
    let c = Scenario::new("recente");
    let org = c.org("a");
    let token = c.token(org.id, "a");
    let agente = c.agente(&org, "bia", "Bia");
    let c1 = conversation_for_client(&c, &org, agente.id, "5511");
    let c2 = conversation_for_client(&c, &org, agente.id, "5522");
    let c3 = conversation_for_client(&c, &org, agente.id, "5533");
    c.store
        .conversations()
        .touch(org.id, c1.id, 9_000_000)
        .expect("toca c1");
    c.store
        .conversations()
        .touch(org.id, c3.id, 9_000_000)
        .expect("toca c3");
    let (base, desligar) = serve(c.estado()).await;
    let list_url = list_path(org.id);

    let ordered = request(
        &base,
        Method::GET,
        &format!("{list_url}?sort=recent"),
        &token,
        None,
    )
    .await;
    assert_eq!(ordered.status, StatusCode::OK, "{}", ordered.texto);
    assert_eq!(
        ids_of(&items_of(&ordered)),
        vec![c3.id.to_string(), c1.id.to_string(), c2.id.to_string()]
    );
    assert!(
        ordered.json["next_cursor"].is_null(),
        "sort=recent não usa cursor: next_cursor vem nulo"
    );

    let two = request(
        &base,
        Method::GET,
        &format!("{list_url}?sort=recent&limit=2"),
        &token,
        None,
    )
    .await;
    assert_eq!(
        ids_of(&items_of(&two)),
        vec![c3.id.to_string(), c1.id.to_string()]
    );
    assert!(two.json["next_cursor"].is_null());

    let empty_cursor = request(
        &base,
        Method::GET,
        &format!("{list_url}?sort=recent&cursor="),
        &token,
        None,
    )
    .await;
    assert_eq!(
        empty_cursor.status,
        StatusCode::OK,
        "{}",
        empty_cursor.texto
    );

    let by_client = request(
        &base,
        Method::GET,
        &format!("{list_url}?sort=recent&external_peer=5522"),
        &token,
        None,
    )
    .await;
    assert_eq!(ids_of(&items_of(&by_client)), vec![c2.id.to_string()]);

    let _ = desligar.send(());
}

#[tokio::test]
async fn sort_recent_rejects_cursor_and_out_of_range_limit_with_400() {
    let c = Scenario::new("recente-400");
    let org = c.org("a");
    let token = c.token(org.id, "a");
    let agente = c.agente(&org, "bia", "Bia");
    let conversa = conversation_for_client(&c, &org, agente.id, "5511");
    let (base, desligar) = serve(c.estado()).await;
    let list_url = list_path(org.id);

    let with_cursor = request(
        &base,
        Method::GET,
        &format!("{list_url}?sort=recent&cursor={}", conversa.id),
        &token,
        None,
    )
    .await;
    assert_eq!(
        with_cursor.status,
        StatusCode::BAD_REQUEST,
        "cursor com sort=recent é recusado, não ignorado: {}",
        with_cursor.texto
    );

    for limit in ["0", "201"] {
        let outside = request(
            &base,
            Method::GET,
            &format!("{list_url}?sort=recent&limit={limit}"),
            &token,
            None,
        )
        .await;
        assert_eq!(
            outside.status,
            StatusCode::BAD_REQUEST,
            "limit={limit} com sort=recent: {}",
            outside.texto
        );
    }

    let _ = desligar.send(());
}

#[tokio::test]
async fn invalid_filter_value_returns_400_without_echoing_value() {
    let c = Scenario::new("invalidos");
    let org = c.org("a");
    let token = c.token(org.id, "a");
    let (base, desligar) = serve(c.estado()).await;
    let list_url = list_path(org.id);
    const ODD_VALUE: &str = "xyzzy-valor-estranho";

    for query in [
        format!("{list_url}?status={ODD_VALUE}"),
        format!("{list_url}?external_peer=%20%20"),
        format!("{list_url}?external_peer="),
        format!("{list_url}?sort={ODD_VALUE}"),
        format!("{list_url}?sort=recent&limit=0"),
    ] {
        let response = request(&base, Method::GET, &query, &token, None).await;
        assert_eq!(
            response.status,
            StatusCode::BAD_REQUEST,
            "{query}: {}",
            response.texto
        );
        assert!(
            !response.texto.contains(ODD_VALUE),
            "o valor recebido não volta na mensagem de erro: {}",
            response.texto
        );
    }

    let _ = desligar.send(());
}

#[tokio::test]
async fn without_sort_list_follows_id_with_cursor_and_adjusted_limit() {
    let c = Scenario::new("sem-sort");
    let org = c.org("a");
    let token = c.token(org.id, "a");
    let agente = c.agente(&org, "bia", "Bia");
    let ids: Vec<String> = (0..5)
        .map(|n| {
            conversation_for_client(&c, &org, agente.id, &format!("55{n}"))
                .id
                .to_string()
        })
        .collect();
    let (base, desligar) = serve(c.estado()).await;
    let list_url = list_path(org.id);

    let first = request(
        &base,
        Method::GET,
        &format!("{list_url}?limit=2"),
        &token,
        None,
    )
    .await;
    assert_eq!(ids_of(&items_of(&first)), ids[..2]);
    let cursor = first.json["next_cursor"]
        .as_str()
        .expect("página cheia traz next_cursor")
        .to_owned();
    assert_eq!(cursor, ids[1], "o cursor é o último id da página");

    let second = request(
        &base,
        Method::GET,
        &format!("{list_url}?limit=2&cursor={cursor}"),
        &token,
        None,
    )
    .await;
    assert_eq!(ids_of(&items_of(&second)), ids[2..4]);

    let last = request(
        &base,
        Method::GET,
        &format!("{list_url}?limit=2&cursor={}", ids[3]),
        &token,
        None,
    )
    .await;
    assert_eq!(ids_of(&items_of(&last)), ids[4..5]);
    assert!(last.json["next_cursor"].is_null());

    let large = request(
        &base,
        Method::GET,
        &format!("{list_url}?limit=500"),
        &token,
        None,
    )
    .await;
    assert_eq!(items_of(&large).len(), 5);

    let by_client = request(
        &base,
        Method::GET,
        &format!("{list_url}?external_peer=550&limit=1"),
        &token,
        None,
    )
    .await;
    assert_eq!(ids_of(&items_of(&by_client)), ids[..1]);

    let _ = desligar.send(());
}
