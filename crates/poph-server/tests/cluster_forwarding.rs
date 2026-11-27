use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::ServerState;
use poph_server::cluster::LiveCluster;
use poph_server::evolution::EvolutionChannel;
use poph_server::forwarding::{
    HOP_HEADER, INSTANCE_HEADER, MAP_HEADER, NONCE_HEADER, SIGNATURE_HEADER, TIMESTAMP_HEADER,
    body_hash, new_nonce, sign,
};
use poph_store::{NewOrg, Page, Store};
use poph_types::config::{ClusterConfig, ClusterInstance, GlobalTokenConfig};
use poph_types::{HarnessConfig, IntegrationId, OrgId, RecordStatus, Secret};
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tracing_subscriber::fmt::MakeWriter;

const TOKEN_GLOBAL: &str = "pophg_teste-global-hr50b";
const SECRET: &str = "SENTINELA-SEGREDO-ENCAMINHAMENTO-hr50b";
const WEBHOOK_SECRET: &str = "segredo-webhook-teste-hr50b";

#[derive(Clone)]
struct LogCapture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogCapture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for LogCapture {
    type Writer = LogCapture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();

fn captured_logs() -> Arc<Mutex<Vec<u8>>> {
    LOGS.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let _ = tracing_subscriber::fmt()
            .with_writer(LogCapture(Arc::clone(&buffer)))
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .try_init();
        buffer
    })
    .clone()
}

struct TestInstance {
    base: String,
    store: Arc<Store>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Drop for TestInstance {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
    }
}

struct InstanceSetup {
    store: Arc<Store>,
    config: HarnessConfig,
    #[allow(dead_code)]
    directory: PathBuf,
}

fn hash_argon2(value: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

fn directory(label: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let directory = std::env::temp_dir().join(format!(
        "poph-encaminhamento-{label}-{}-{stamp}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(directory.join("dados")).expect("pasta de dados");
    directory
}

fn prepare_instance(label: &str) -> InstanceSetup {
    let directory = directory(label);
    let mut config = HarnessConfig::default();
    config.data.directory = directory.join("dados");
    config.admin.tokens_globais = vec![GlobalTokenConfig {
        label: "teste".to_owned(),
        hash: hash_argon2(TOKEN_GLOBAL),
    }];
    config.validar().expect("configuração válida");
    let store = Arc::new(Store::open(&config).expect("abre o banco"));
    InstanceSetup {
        store,
        config,
        directory,
    }
}

fn map(instance_id: &str, url_a: &str, url_b: &str, org: &str, destination: &str) -> ClusterConfig {
    ClusterConfig {
        instance_id: instance_id.to_owned(),
        default_instance: Some("b".to_owned()),
        instances: vec![
            ClusterInstance {
                id: "a".to_owned(),
                url: url_a.to_owned(),
            },
            ClusterInstance {
                id: "b".to_owned(),
                url: url_b.to_owned(),
            },
        ],
        orgs: BTreeMap::from([(org.to_owned(), destination.to_owned())]),
    }
}

async fn start_instance(
    setup: InstanceSetup,
    listener: tokio::net::TcpListener,
    map: ClusterConfig,
    cluster_secret: Option<Secret>,
    channels: Vec<EvolutionChannel>,
) -> TestInstance {
    let estado = ServerState::new(Arc::clone(&setup.store), setup.config)
        .expect("estado do servidor")
        .with_cluster(LiveCluster {
            path: setup.directory.join("cluster.toml"),
            config: map,
            secret: cluster_secret,
        })
        .with_evolution_channels(channels);
    let endereco = listener.local_addr().expect("endereço local");
    let (desligar, signal) = oneshot::channel();
    tokio::spawn(async move {
        let _ = poph_server::serve_at(listener, estado, async {
            let _ = signal.await;
        })
        .await;
    });
    let instance = TestInstance {
        base: format!("http://{endereco}"),
        store: setup.store,
        desligar: Some(desligar),
    };
    let client = client();
    for _ in 0..500 {
        if let Ok(response) = client
            .get(format!("{}/healthz", instance.base))
            .send()
            .await
            && response.status().is_success()
        {
            return instance;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("instância não subiu");
}

fn client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("cliente HTTP")
}

async fn bind_listener() -> (tokio::net::TcpListener, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = listener.local_addr().expect("endereço local");
    (listener, format!("http://{endereco}"))
}

fn now() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    )
    .unwrap_or(0)
}

struct TestResponse {
    status: StatusCode,
    body: Value,
    raw: String,
}

impl TestResponse {
    fn assert_no_secret(&self) {
        assert!(
            !self.raw.contains(SECRET),
            "segredo na resposta: {}",
            self.raw
        );
        assert!(
            !self.raw.contains(&body_hash(SECRET.as_bytes())),
            "hash do segredo na resposta"
        );
    }
}

async fn request(
    method: Method,
    url: &str,
    token: Option<&str>,
    body: Option<&[u8]>,
    extras: &[(&str, String)],
) -> TestResponse {
    let mut request = client().request(method, url);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    for (nome, value) in extras {
        request = request.header(*nome, value);
    }
    if let Some(body) = body {
        request = request
            .header("content-type", "application/json")
            .body(body.to_vec());
    }
    let response = request.send().await.expect("resposta HTTP");
    let status = response.status();
    let raw = response.text().await.expect("corpo da resposta");
    let body = serde_json::from_str(&raw).unwrap_or(Value::Null);
    TestResponse { status, body, raw }
}

#[allow(clippy::too_many_arguments)]
fn signed_headers(
    secret: &Secret,
    method: &Method,
    target: &str,
    body: &[u8],
    instance: &str,
    hop: u32,
    stamp: i64,
    map: &str,
) -> Vec<(&'static str, String)> {
    signed_headers_with_nonce(
        secret,
        method,
        target,
        body,
        instance,
        hop,
        stamp,
        map,
        &new_nonce(),
    )
}

#[allow(clippy::too_many_arguments)]
fn signed_headers_with_nonce(
    secret: &Secret,
    method: &Method,
    target: &str,
    body: &[u8],
    instance: &str,
    hop: u32,
    stamp: i64,
    map: &str,
    nonce: &str,
) -> Vec<(&'static str, String)> {
    let signature = sign(secret, method, target, body, stamp, instance, hop, nonce);
    vec![
        (INSTANCE_HEADER, instance.to_owned()),
        (TIMESTAMP_HEADER, stamp.to_string()),
        (SIGNATURE_HEADER, signature),
        (HOP_HEADER, hop.to_string()),
        (MAP_HEADER, map.to_owned()),
        (NONCE_HEADER, nonce.to_owned()),
    ]
}

async fn two_instances(
    label: &str,
    with_channel_in_b: bool,
) -> (TestInstance, TestInstance, OrgId, ClusterConfig) {
    let (listener_a, url_a) = bind_listener().await;
    let (listener_b, url_b) = bind_listener().await;
    let setup_a = prepare_instance(&format!("{label}-a"));
    let setup_b = prepare_instance(&format!("{label}-b"));
    let org = setup_b
        .store
        .orgs()
        .create(NewOrg {
            name: "Org da instância b".to_owned(),
            slug: "org-da-instancia-b".to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org em b")
        .id;
    let org_text = org.to_string();

    let map_a = map("a", &url_a, &url_b, &org_text, "b");
    let map_b = map("b", &url_a, &url_b, &org_text, "b");
    let secret = Secret::new(SECRET.to_owned());

    let mut channels = Vec::new();
    if with_channel_in_b {
        channels.push(EvolutionChannel {
            org,
            instance: "loja".to_owned(),
            integration: IntegrationId::generate(),
            secret: Some(Secret::new(WEBHOOK_SECRET.to_owned())),
            client: None,
            attachment_limit: 1_048_576,
        });
    }

    let a = start_instance(setup_a, listener_a, map_a, Some(secret.clone()), Vec::new()).await;
    let b = start_instance(setup_b, listener_b, map_b, Some(secret), channels).await;
    (a, b, org, map("b", &url_a, &url_b, &org_text, "b"))
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_and_webhook_calls_reach_org_owner() {
    let (a, b, org, _map) = two_instances("remota", true).await;
    let path = format!("/admin/v1/orgs/{org}/agents");

    let on_owner = request(
        Method::GET,
        &format!("{}{path}", b.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(on_owner.status, StatusCode::OK, "{}", on_owner.raw);
    assert_eq!(on_owner.body["items"].as_array().map(Vec::len), Some(0));

    let via_a = request(
        Method::GET,
        &format!("{}{path}", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(via_a.status, StatusCode::OK, "{}", via_a.raw);
    assert_eq!(via_a.body, on_owner.body);
    via_a.assert_no_secret();

    let with_query = request(
        Method::GET,
        &format!("{}{path}?limit=1", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(with_query.status, StatusCode::OK, "{}", with_query.raw);
    assert_eq!(with_query.body, on_owner.body);

    let direct_backup = request(
        Method::GET,
        &format!("{}/admin/v1/orgs/{org}/backup", b.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(
        direct_backup.status,
        StatusCode::OK,
        "{}",
        direct_backup.raw
    );
    assert_eq!(direct_backup.body["formato"], json!("poph-backup-org"));
    let backup_via_a = request(
        Method::GET,
        &format!("{}/admin/v1/orgs/{org}/backup", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(backup_via_a.status, StatusCode::OK, "{}", backup_via_a.raw);
    assert_eq!(backup_via_a.body["org"], direct_backup.body["org"]);
    assert_eq!(backup_via_a.body["tabelas"], direct_backup.body["tabelas"]);
    assert_eq!(
        backup_via_a.body["org_origem"],
        direct_backup.body["org_origem"]
    );

    assert!(
        a.store.orgs().by_slug("org-da-instancia-b").is_err(),
        "a org não pode existir no banco de `a`"
    );

    let missing = OrgId::generate();
    let other = format!("/admin/v1/orgs/{missing}");
    let direct = request(
        Method::GET,
        &format!("{}{other}", b.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(direct.status, StatusCode::NOT_FOUND);
    let via_a = request(
        Method::GET,
        &format!("{}{other}", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(via_a.status, StatusCode::NOT_FOUND);
    assert_eq!(via_a.body["code"], json!("not_found"));

    let event = json!({
        "event": "connection.update",
        "instance": "loja",
        "data": {"state": "open"}
    })
    .to_string();
    let path_webhook = format!("/webhook/evolution/{org}/loja");
    let webhook = request(
        Method::POST,
        &format!("{}{path_webhook}", a.base),
        None,
        Some(event.as_bytes()),
        &[("x-evolution-webhook-secret", WEBHOOK_SECRET.to_owned())],
    )
    .await;
    assert_eq!(webhook.status, StatusCode::OK, "{}", webhook.raw);
    assert_eq!(webhook.body["recebido"], json!(true));
    assert_eq!(webhook.body["efeito"], json!("integracao_inexistente"));

    let other_event = json!({
        "event": "connection.update",
        "instance": "loja",
        "data": {"state": "connecting"}
    })
    .to_string();
    let assert_no_secret = request(
        Method::POST,
        &format!("{}{path_webhook}", a.base),
        None,
        Some(other_event.as_bytes()),
        &[],
    )
    .await;
    assert_eq!(
        assert_no_secret.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        assert_no_secret.raw
    );
    assert_eq!(
        assert_no_secret.body["code"],
        json!("webhook_nao_autorizado")
    );

    let logs = captured_logs();
    let texto = String::from_utf8_lossy(&logs.lock().unwrap()).to_string();
    assert!(!texto.contains(SECRET), "segredo no log: {texto}");
}

#[tokio::test(flavor = "multi_thread")]
async fn forwarding_signature_is_verified() {
    let (_a, b, org, map) = two_instances("assinatura", false).await;
    let secret = Secret::new(SECRET.to_owned());
    let target = format!("/admin/v1/orgs/{org}/agents");
    let url = format!("{}{target}", b.base);
    let fingerprint = map.map_fingerprint();

    let bons = signed_headers(
        &secret,
        &Method::GET,
        &target,
        b"",
        "a",
        1,
        now(),
        &fingerprint,
    );
    let validates = request(Method::GET, &url, Some(TOKEN_GLOBAL), None, &bons).await;
    assert_eq!(validates.status, StatusCode::OK, "{}", validates.raw);

    let repeated = request(Method::GET, &url, Some(TOKEN_GLOBAL), None, &bons).await;
    assert_eq!(
        repeated.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        repeated.raw
    );
    assert_eq!(repeated.body["code"], json!("assinatura_invalida"));

    let body = json!({"nome": "Loja", "slug": "loja"}).to_string();
    let assinado = signed_headers(
        &secret,
        &Method::POST,
        &format!("/admin/v1/orgs/{org}/agents"),
        body.as_bytes(),
        "a",
        1,
        now(),
        &fingerprint,
    );
    let adulterado = request(
        Method::POST,
        &format!("{}/admin/v1/orgs/{org}/agents", b.base),
        Some(TOKEN_GLOBAL),
        Some(br#"{"nome":"Outra","slug":"outra"}"#),
        &assinado,
    )
    .await;
    assert_eq!(
        adulterado.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        adulterado.raw
    );
    assert_eq!(adulterado.body["code"], json!("assinatura_invalida"));

    let old = signed_headers(
        &secret,
        &Method::GET,
        &target,
        b"",
        "a",
        1,
        now() - 120,
        &fingerprint,
    );
    let outside_window = request(Method::GET, &url, Some(TOKEN_GLOBAL), None, &old).await;
    assert_eq!(outside_window.status, StatusCode::UNAUTHORIZED);
    assert_eq!(outside_window.body["code"], json!("assinatura_invalida"));

    let unknown = signed_headers(
        &secret,
        &Method::GET,
        &target,
        b"",
        "z",
        1,
        now(),
        &fingerprint,
    );
    let instance_unknown = request(Method::GET, &url, Some(TOKEN_GLOBAL), None, &unknown).await;
    assert_eq!(instance_unknown.status, StatusCode::UNAUTHORIZED);
    assert_eq!(instance_unknown.body["code"], json!("assinatura_invalida"));

    let normal = request(Method::GET, &url, Some(TOKEN_GLOBAL), None, &[]).await;
    assert_eq!(normal.status, StatusCode::OK);
    let without_token = request(Method::GET, &url, None, None, &[]).await;
    assert_eq!(without_token.status, StatusCode::UNAUTHORIZED);
    assert_eq!(without_token.body["code"], json!("unauthorized"));
}

#[tokio::test(flavor = "multi_thread")]
async fn loop_and_mismatched_map_return_421() {
    let (a, b, org, map) = two_instances("laco", false).await;
    let secret = Secret::new(SECRET.to_owned());
    let target = format!("/admin/v1/orgs/{org}/agents");
    let url = format!("{}{target}", b.base);

    let two_hops = signed_headers(
        &secret,
        &Method::GET,
        &target,
        b"",
        "a",
        2,
        now(),
        &map.map_fingerprint(),
    );
    let loop_task = request(Method::GET, &url, Some(TOKEN_GLOBAL), None, &two_hops).await;
    assert_eq!(
        loop_task.status,
        StatusCode::MISDIRECTED_REQUEST,
        "{}",
        loop_task.raw
    );
    assert_eq!(loop_task.body["code"], json!("rota_em_laco"));

    let mut other = map.clone();
    other.instance_id = "a".to_owned();
    other.orgs.insert(org.to_string(), "a".to_owned());
    let other_fingerprint = other.map_fingerprint();
    assert_ne!(other_fingerprint, map.map_fingerprint());
    let divergent = signed_headers(
        &secret,
        &Method::GET,
        &target,
        b"",
        "a",
        1,
        now() - 5,
        &other_fingerprint,
    );
    let refused = request(Method::GET, &url, Some(TOKEN_GLOBAL), None, &divergent).await;
    assert_eq!(
        refused.status,
        StatusCode::MISDIRECTED_REQUEST,
        "{}",
        refused.raw
    );
    assert_eq!(refused.body["code"], json!("mapa_divergente"));
    assert_eq!(
        refused.body["details"]["fingerprint_recebido"],
        json!(other_fingerprint)
    );
    assert_eq!(
        refused.body["details"]["fingerprint_local"],
        json!(map.map_fingerprint())
    );

    let for_a = signed_headers(
        &secret,
        &Method::GET,
        &target,
        b"",
        "a",
        1,
        now() - 10,
        &map.map_fingerprint(),
    );
    let on_wrong_instance = request(
        Method::GET,
        &format!("{}{target}", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &for_a,
    )
    .await;
    assert_eq!(
        on_wrong_instance.status,
        StatusCode::MISDIRECTED_REQUEST,
        "{}",
        on_wrong_instance.raw
    );
    assert_eq!(on_wrong_instance.body["code"], json!("rota_em_laco"));
    assert_eq!(
        on_wrong_instance.body["details"]["instancia_dona"],
        json!("b")
    );

    let on_owner = request(Method::GET, &url, Some(TOKEN_GLOBAL), None, &for_a).await;
    assert_eq!(on_owner.status, StatusCode::OK, "{}", on_owner.raw);
}

#[tokio::test(flavor = "multi_thread")]
async fn unavailable_owner_returns_503() {
    let (listener_a, url_a) = bind_listener().await;
    let (listener_b, url_b) = bind_listener().await;
    drop(listener_b);

    let setup = prepare_instance("fora-do-ar-a");
    let org = setup
        .store
        .orgs()
        .create(NewOrg {
            name: "Org fora do ar".to_owned(),
            slug: "org-fora-do-ar".to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id;
    let map_a = map("a", &url_a, &url_b, &org.to_string(), "b");
    let a = start_instance(
        setup,
        listener_a,
        map_a,
        Some(Secret::new(SECRET.to_owned())),
        Vec::new(),
    )
    .await;

    let response = request(
        Method::GET,
        &format!("{}/admin/v1/orgs/{org}/agents", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        response.raw
    );
    assert_eq!(response.body["code"], json!("instancia_fora"));
    assert_eq!(response.body["details"]["instancia"], json!("b"));
    assert_eq!(response.body["details"]["url"], json!(url_b));
    response.assert_no_secret();
}

#[tokio::test(flavor = "multi_thread")]
async fn single_instance_mode_is_unchanged() {
    let (listener, _url) = bind_listener().await;
    let a = start_instance(
        prepare_instance("unica"),
        listener,
        ClusterConfig::default(),
        None,
        Vec::new(),
    )
    .await;

    let local = request(
        Method::GET,
        &format!("{}/admin/v1/orgs", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(local.status, StatusCode::OK, "{}", local.raw);

    let webhook = request(
        Method::POST,
        &format!("{}/webhook/evolution/{}/loja", a.base, OrgId::generate()),
        None,
        Some(b"{\"event\":\"connection.update\",\"instance\":\"loja\",\"data\":{\"state\":\"open\"}}"),
        &[("x-evolution-webhook-secret", WEBHOOK_SECRET.to_owned())],
    )
    .await;
    assert_eq!(webhook.status, StatusCode::NOT_FOUND, "{}", webhook.raw);

    let with_peer = request(
        Method::GET,
        &format!("{}/admin/v1/orgs/{}/agents", a.base, OrgId::generate()),
        Some(TOKEN_GLOBAL),
        None,
        &[
            (INSTANCE_HEADER, "a".to_owned()),
            (TIMESTAMP_HEADER, now().to_string()),
            (SIGNATURE_HEADER, "00".to_owned()),
            (HOP_HEADER, "1".to_owned()),
            (MAP_HEADER, "00".to_owned()),
        ],
    )
    .await;
    assert_eq!(
        with_peer.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        with_peer.raw
    );
    assert_eq!(with_peer.body["code"], json!("assinatura_invalida"));
}

#[tokio::test(flavor = "multi_thread")]
async fn nonce_distinguishes_requests_and_prevents_replay() {
    let (_a, b, org, map) = two_instances("nonce", false).await;
    let secret = Secret::new(SECRET.to_owned());
    let target = format!("/admin/v1/orgs/{org}/agents");
    let url = format!("{}{target}", b.base);
    let fingerprint = map.map_fingerprint();
    let stamp = now();

    let first = signed_headers(
        &secret,
        &Method::GET,
        &target,
        b"",
        "a",
        1,
        stamp,
        &fingerprint,
    );
    let second = signed_headers(
        &secret,
        &Method::GET,
        &target,
        b"",
        "a",
        1,
        stamp,
        &fingerprint,
    );
    assert_ne!(
        first.iter().find(|(nome, _)| *nome == NONCE_HEADER),
        second.iter().find(|(nome, _)| *nome == NONCE_HEADER),
        "cada pedido tem de levar um nonce próprio"
    );
    let um = request(Method::GET, &url, Some(TOKEN_GLOBAL), None, &first).await;
    assert_eq!(um.status, StatusCode::OK, "{}", um.raw);
    let two = request(Method::GET, &url, Some(TOKEN_GLOBAL), None, &second).await;
    assert_eq!(
        two.status,
        StatusCode::OK,
        "pedido idêntico no mesmo segundo com nonce novo tem de passar: {}",
        two.raw
    );

    let repeated = request(Method::GET, &url, Some(TOKEN_GLOBAL), None, &first).await;
    assert_eq!(
        repeated.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        repeated.raw
    );
    assert_eq!(repeated.body["code"], json!("assinatura_invalida"));
    assert!(
        repeated.body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("nonce"),
        "a mensagem precisa dizer que é o nonce: {}",
        repeated.raw
    );

    let without_nonce: Vec<(&'static str, String)> = signed_headers(
        &secret,
        &Method::GET,
        &target,
        b"",
        "a",
        1,
        now() - 3,
        &fingerprint,
    )
    .into_iter()
    .filter(|(nome, _)| *nome != NONCE_HEADER)
    .collect();
    let missing = request(Method::GET, &url, Some(TOKEN_GLOBAL), None, &without_nonce).await;
    assert_eq!(missing.status, StatusCode::UNAUTHORIZED, "{}", missing.raw);
    assert_eq!(missing.body["code"], json!("assinatura_invalida"));
    assert!(
        missing.body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("x-poph-nonce"),
        "a mensagem precisa apontar o cabeçalho que falta: {}",
        missing.raw
    );

    let malformed = signed_headers_with_nonce(
        &secret,
        &Method::GET,
        &target,
        b"",
        "a",
        1,
        now() - 6,
        &fingerprint,
        "abc",
    );
    let invalid = request(Method::GET, &url, Some(TOKEN_GLOBAL), None, &malformed).await;
    assert_eq!(invalid.status, StatusCode::UNAUTHORIZED, "{}", invalid.raw);
    assert_eq!(invalid.body["code"], json!("assinatura_invalida"));
    assert!(
        invalid.body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("malformado"),
        "a mensagem precisa dizer que o nonce está malformado: {}",
        invalid.raw
    );
    invalid.assert_no_secret();
}

#[tokio::test(flavor = "multi_thread")]
async fn cluster_listing_aggregates_instances_and_marks_unavailable_peers() {
    let (a, b, org, _map) = two_instances("listagem-cluster", false).await;

    let normal = request(
        Method::GET,
        &format!("{}/admin/v1/orgs", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(normal.status, StatusCode::OK, "{}", normal.raw);
    assert_eq!(normal.body["items"], json!([]));
    assert!(normal.body["next_cursor"].is_null(), "{}", normal.raw);
    assert!(
        normal.body.get("instancias").is_none(),
        "sem `cluster=true` não pode aparecer `instancias`: {}",
        normal.raw
    );

    let with_cluster = request(
        Method::GET,
        &format!("{}/admin/v1/orgs?cluster=true", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(with_cluster.status, StatusCode::OK, "{}", with_cluster.raw);
    let items = with_cluster.body["items"]
        .as_array()
        .unwrap_or_else(|| panic!("items: {}", with_cluster.raw));
    assert_eq!(items.len(), 1, "{}", with_cluster.raw);
    assert_eq!(items[0]["id"], json!(org.to_string()));
    assert_eq!(items[0]["instancia"], json!("b"));
    assert!(with_cluster.body["next_cursor"].is_null());
    let instances = with_cluster.body["instancias"]
        .as_array()
        .unwrap_or_else(|| panic!("instancias: {}", with_cluster.raw));
    assert_eq!(instances.len(), 2, "{}", with_cluster.raw);
    let local = instances
        .iter()
        .find(|instance| instance["id"] == json!("a"))
        .expect("instância local no relatório");
    assert_eq!(local["situacao"], json!("local"));
    let neighbor = instances
        .iter()
        .find(|instance| instance["id"] == json!("b"))
        .expect("instância vizinha no relatório");
    assert_eq!(neighbor["situacao"], json!("ok"));
    assert_eq!(neighbor["orgs"], json!(1));
    with_cluster.assert_no_secret();

    let local_org = a
        .store
        .orgs()
        .create(NewOrg {
            name: "Org local".to_owned(),
            slug: "org-local-hr50b2".to_owned(),
            config_json: String::new(),
        })
        .expect("cria org local")
        .id;

    drop(b);
    let on_failure = request(
        Method::GET,
        &format!("{}/admin/v1/orgs?cluster=true", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(on_failure.status, StatusCode::OK, "{}", on_failure.raw);
    let items = on_failure.body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{}", on_failure.raw);
    assert_eq!(items[0]["id"], json!(local_org.to_string()));
    assert_eq!(items[0]["instancia"], json!("a"));
    let instances = on_failure.body["instancias"]
        .as_array()
        .expect("instancias");
    let neighbor = instances
        .iter()
        .find(|instance| instance["id"] == json!("b"))
        .expect("instância vizinha no relatório");
    assert_eq!(neighbor["situacao"], json!("inacessivel"));
    assert_eq!(neighbor["orgs"], json!(0));
    assert!(
        neighbor["mensagem"].as_str().unwrap_or_default().len() > 5,
        "a instância fora do ar precisa de mensagem: {}",
        on_failure.raw
    );
    on_failure.assert_no_secret();
}

#[tokio::test(flavor = "multi_thread")]
async fn cluster_listing_counts_active_agents_per_instance() {
    let (a, b, org, _map) = two_instances("agentes-ativos-cluster", false).await;

    for slug in ["agente-um", "agente-dois"] {
        b.store
            .agents()
            .create(
                org,
                poph_store::NewAgent {
                    name: format!("Agente {slug}"),
                    slug: slug.to_owned(),
                    ..poph_store::NewAgent::default()
                },
            )
            .expect("cria o agente em b");
    }
    let inactive = b
        .store
        .agents()
        .create(
            org,
            poph_store::NewAgent {
                name: "Agente inativo".to_owned(),
                slug: "agente-inativo".to_owned(),
                ..poph_store::NewAgent::default()
            },
        )
        .expect("cria o agente inativo em b");
    let read = b.store.agents().get(org, inactive.id).expect("lê o agente");
    b.store
        .agents()
        .update(
            org,
            inactive.id,
            read.version,
            poph_store::AgentChange {
                status: Some(poph_store::Status::Inactive),
                ..poph_store::AgentChange::default()
            },
        )
        .expect("inativa o agente em b");

    let local_org = a
        .store
        .orgs()
        .create(NewOrg {
            name: "Org local".to_owned(),
            slug: "org-local-cf03a".to_owned(),
            config_json: String::new(),
        })
        .expect("cria org local")
        .id;
    a.store
        .agents()
        .create(
            local_org,
            poph_store::NewAgent {
                name: "Agente local".to_owned(),
                slug: "agente-local".to_owned(),
                ..poph_store::NewAgent::default()
            },
        )
        .expect("cria o agente local");

    let with_cluster = request(
        Method::GET,
        &format!("{}/admin/v1/orgs?cluster=true", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(with_cluster.status, StatusCode::OK, "{}", with_cluster.raw);
    let items = with_cluster.body["items"]
        .as_array()
        .unwrap_or_else(|| panic!("items: {}", with_cluster.raw));
    let remote = items
        .iter()
        .find(|item| item["id"] == json!(org.to_string()))
        .unwrap_or_else(|| panic!("org de b fora da listagem: {}", with_cluster.raw));
    assert_eq!(remote["instancia"], json!("b"), "{}", with_cluster.raw);
    assert_eq!(
        remote["agents_active_count"],
        json!(2),
        "a contagem da org de b tem de vir no payload remoto, sem o inativo: {}",
        with_cluster.raw
    );
    let local = items
        .iter()
        .find(|item| item["id"] == json!(local_org.to_string()))
        .unwrap_or_else(|| panic!("org local fora da listagem: {}", with_cluster.raw));
    assert_eq!(local["instancia"], json!("a"), "{}", with_cluster.raw);
    assert_eq!(
        local["agents_active_count"],
        json!(1),
        "{}",
        with_cluster.raw
    );
    with_cluster.assert_no_secret();
}

const TEST_FLOW: &str = r#"fluxo(
    id = "agendar_visita",
    nome = "Agendar visita",
    descricao = "Marca uma visita técnica no endereço do cliente.",
    inicio = "coleta",
    gatilho = gatilho(
        quando = "O cliente quer marcar, remarcar ou cancelar uma visita técnica.",
        nao_quando = "Só quer saber preço ou prazo, sem pedir visita.",
        exemplos = ["queria agendar uma visita", "pode vir aqui amanhã?"],
    ),
)

etapa(
    id = "coleta",
    nome = "Coletar dados",
    instrucao = "Pergunte o endereço e o período preferido. Uma pergunta por vez.",
    campos = [
        campo("endereco", "Endereço completo", tipo = "texto", obrigatorio = True),
    ],
    acoes = [
        acao("confirmar", "Dados completos", exige = ["endereco"], vai_para = fim("concluida", resultado = "Visita agendada")),
    ],
)
"#;

#[tokio::test(flavor = "multi_thread")]
async fn flow_and_case_routes_are_forwarded_to_owner() {
    let (a, b, org, _map) = two_instances("fluxos-fichas", false).await;
    let root = format!("/admin/v1/orgs/{org}");

    assert!(
        a.store.orgs().by_slug("org-da-instancia-b").is_err(),
        "a org não pode existir no banco de `a`"
    );

    let body = json!({"source": TEST_FLOW}).to_string();
    let validated_on_owner = request(
        Method::POST,
        &format!("{}{root}/flows/validate", b.base),
        Some(TOKEN_GLOBAL),
        Some(body.as_bytes()),
        &[],
    )
    .await;
    assert_eq!(
        validated_on_owner.status,
        StatusCode::OK,
        "{}",
        validated_on_owner.raw
    );
    assert_eq!(validated_on_owner.body["valid"], json!(true));
    let validated_via_a = request(
        Method::POST,
        &format!("{}{root}/flows/validate", a.base),
        Some(TOKEN_GLOBAL),
        Some(body.as_bytes()),
        &[],
    )
    .await;
    assert_eq!(
        validated_via_a.status,
        StatusCode::OK,
        "{}",
        validated_via_a.raw
    );
    assert_eq!(validated_via_a.body, validated_on_owner.body);

    let created = request(
        Method::POST,
        &format!("{}{root}/flows", a.base),
        Some(TOKEN_GLOBAL),
        Some(
            json!({"source": TEST_FLOW, "publish": true})
                .to_string()
                .as_bytes(),
        ),
        &[],
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.raw);
    assert_eq!(created.body["slug"], json!("agendar_visita"));
    assert!(
        b.store
            .flows()
            .by_slug(org, "agendar_visita")
            .expect("consulta em b")
            .is_some(),
        "o fluxo criado por `a` tem de existir em `b`"
    );
    assert!(
        a.store
            .flows()
            .by_slug(org, "agendar_visita")
            .expect("consulta em a")
            .is_none(),
        "`a` não grava nada: a org nem existe no banco dela"
    );

    let agent = request(
        Method::POST,
        &format!("{}{root}/agents", a.base),
        Some(TOKEN_GLOBAL),
        Some(
            json!({"name": "Atendente fictício", "slug": "atendente-ficticio"})
                .to_string()
                .as_bytes(),
        ),
        &[],
    )
    .await;
    assert_eq!(agent.status, StatusCode::CREATED, "{}", agent.raw);
    let agent_id = agent.body["id"].as_str().expect("id do agente").to_owned();
    let conversation = request(
        Method::POST,
        &format!("{}{root}/agents/{agent_id}/test-chat", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(
        conversation.status,
        StatusCode::CREATED,
        "{}",
        conversation.raw
    );

    let cases_via_a = request(
        Method::GET,
        &format!("{}{root}/fichas", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(cases_via_a.status, StatusCode::OK, "{}", cases_via_a.raw);
    let items = cases_via_a.body["items"]
        .as_array()
        .expect("lista de Fichas");
    assert_eq!(items.len(), 1, "a conversa de teste abriu uma Ficha");
    let ficha = items[0]["id"].as_str().expect("id da Ficha").to_owned();
    let cases_on_owner = request(
        Method::GET,
        &format!("{}{root}/fichas", b.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(cases_on_owner.body, cases_via_a.body);

    let closed = request(
        Method::POST,
        &format!("{}{root}/fichas/{ficha}/close", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[("if-match", "1".to_owned())],
    )
    .await;
    assert_eq!(closed.status, StatusCode::OK, "{}", closed.raw);
    assert_eq!(closed.body["status"], json!("closed"));
    let on_owner = b
        .store
        .cases()
        .get(org, ficha.parse().expect("id da Ficha"))
        .expect("Ficha em b");
    assert_eq!(on_owner.status, RecordStatus::Closed);
    assert_eq!(
        b.store
            .cases()
            .list(org, &Default::default(), Page::default())
            .expect("lista em b")
            .len(),
        1
    );
    assert_eq!(
        b.store
            .conversations()
            .list(org, None, Page::default())
            .expect("conversas em b")
            .len(),
        1,
        "a conversa também fecha, mas fica no histórico"
    );

    assert!(
        a.store.orgs().get(org).is_err(),
        "a org não pode passar a existir em `a`"
    );
}
