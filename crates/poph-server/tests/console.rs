use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use poph_server::ServerState;
use poph_store::Store;
use poph_types::HarnessConfig;
use reqwest::{Client, Method, StatusCode};
use tokio::sync::oneshot;

struct Scenario {
    base: String,
    client: Client,
    folder: PathBuf,
    desligar: Option<oneshot::Sender<()>>,
}

impl Scenario {
    async fn new() -> Self {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let folder = std::env::temp_dir().join(format!(
            "poph-console-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(folder.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = folder.join("dados");
        config.validar().expect("configuração válida");

        let store = std::sync::Arc::new(Store::open(&config).expect("abre o banco"));
        let estado = ServerState::new(store, config).expect("estado do servidor");

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

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(self.url(path))
            .send()
            .await
            .expect("requisição GET")
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

fn tipo(response: &reqwest::Response) -> String {
    response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn root_serves_console_or_notice_page() {
    let scenario = Scenario::new().await;
    let response = scenario.get("/").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        tipo(&response).starts_with("text/html"),
        "a raiz tem de ser HTML, veio `{}`",
        tipo(&response)
    );
    let body = response.text().await.expect("corpo");
    if poph_server::console::has_bundle() {
        assert!(
            body.contains("<div id=\"root\">"),
            "sem o ponto de montagem da SPA: {body}"
        );
    } else {
        assert!(
            body.contains("Console não compilado"),
            "sem bundle, a página explica como gerar: {body}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_route_falls_back_to_index() {
    let scenario = Scenario::new().await;
    let root = scenario.get("/").await.text().await.expect("corpo da raiz");
    for route in [
        "/orgs",
        "/orgs/algum-id",
        "/login",
        "/qualquer/rota/profunda",
    ] {
        let response = scenario.get(route).await;
        assert_eq!(response.status(), StatusCode::OK, "rota {route}");
        assert!(tipo(&response).starts_with("text/html"), "rota {route}");
        let body = response.text().await.expect("corpo");
        assert_eq!(body, root, "a rota {route} tem de servir o index.html");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_asset_is_not_page() {
    let scenario = Scenario::new().await;
    let response = scenario.get("/assets/nao-existe.js").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(
        tipo(&response).starts_with("application/json"),
        "veio `{}`",
        tipo(&response)
    );
    let body: serde_json::Value = response.json().await.expect("JSON de erro");
    assert_eq!(body["code"], "not_found");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_routes_are_not_shadowed() {
    let scenario = Scenario::new().await;

    let api = scenario.get("/admin/v1/orgs").await;
    assert_eq!(api.status(), StatusCode::UNAUTHORIZED);
    assert!(tipo(&api).starts_with("application/json"));

    let missing = scenario.get("/admin/v1/nao-existe").await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert!(tipo(&missing).starts_with("application/json"));
    let body: serde_json::Value = missing.json().await.expect("JSON de erro");
    assert_eq!(body["code"], "not_found");
    assert_eq!(body["details"]["path"], "/admin/v1/nao-existe");

    let health = scenario.get("/healthz").await;
    assert_eq!(health.status(), StatusCode::OK);
    assert!(tipo(&health).starts_with("application/json"));

    let metrics = scenario.get("/metrics").await;
    assert_eq!(metrics.status(), StatusCode::OK);
    assert!(
        tipo(&metrics).starts_with("text/plain"),
        "veio `{}`",
        tipo(&metrics)
    );

    let webhook = scenario.get("/webhook/evolution/org/instancia").await;
    assert_ne!(webhook.status(), StatusCode::OK);
    assert!(tipo(&webhook).starts_with("application/json"));

    let openapi = scenario.get("/admin/v1/openapi.json").await;
    assert_eq!(openapi.status(), StatusCode::OK);
    assert!(tipo(&openapi).starts_with("application/json"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_get_method_does_not_get_console() {
    let scenario = Scenario::new().await;
    let response = scenario
        .client
        .request(Method::POST, scenario.url("/orgs"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .expect("POST /orgs");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(tipo(&response).starts_with("application/json"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sofa_ui_fonts_come_from_binary() {
    let scenario = Scenario::new().await;
    let response = scenario
        .get("/res/fonts/IBMPlexSans/latin-400-normal.woff2")
        .await;
    if poph_server::console::has_bundle() {
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(tipo(&response), "font/woff2");
        let body = response.bytes().await.expect("corpo da fonte");
        assert!(body.len() > 1_000, "fonte vazia ou truncada");
        assert_eq!(&body[..4], b"wOF2");
    } else {
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(tipo(&response).starts_with("application/json"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn security_headers_accompany_page() {
    let scenario = Scenario::new().await;
    let response = scenario.get("/").await;
    let headers = response.headers();
    let csp = headers
        .get("content-security-policy")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    assert!(csp.contains("default-src 'self'"), "CSP ausente: `{csp}`");
    assert!(
        csp.contains("frame-ancestors 'none'"),
        "CSP sem anti-framing"
    );
    assert_eq!(
        headers
            .get("x-content-type-options")
            .and_then(|value| value.to_str().ok()),
        Some("nosniff")
    );
    assert_eq!(
        headers
            .get("referrer-policy")
            .and_then(|value| value.to_str().ok()),
        Some("no-referrer")
    );
    assert_eq!(
        headers
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("no-cache")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn head_returns_same_headers_without_body() {
    let scenario = Scenario::new().await;
    let response = scenario
        .client
        .head(scenario.url("/"))
        .send()
        .await
        .expect("HEAD /");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(tipo(&response).starts_with("text/html"));
    let body = response.bytes().await.expect("corpo do HEAD");
    assert!(body.is_empty(), "HEAD não devolve corpo");
}
