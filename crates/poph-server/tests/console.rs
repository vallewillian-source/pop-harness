//! Console embutido no binário (HR-38).
//!
//! Servidor de verdade numa porta efêmera, banco temporário e cliente HTTP de verdade. O que se
//! prova:
//!
//! - `/` e as rotas do cliente entregam a página do Console (SPA);
//! - as rotas do servidor (`/admin/v1`, `/healthz`, `/metrics`, `/webhook`) não são sombreadas e
//!   continuam respondendo no formato próprio — inclusive o erro de rota inexistente em JSON;
//! - asset inexistente dá 404 em JSON (nunca HTML com tipo errado);
//! - as fontes da SofaUI saem de `/res/fonts` quando o bundle está no binário;
//! - os cabeçalhos de segurança acompanham a página e o `HEAD` não devolve corpo.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use poph_server::Estado;
use poph_store::Store;
use poph_types::HarnessConfig;
use reqwest::{Client, Method, StatusCode};
use tokio::sync::oneshot;

struct Cenario {
    base: String,
    cliente: Client,
    pasta: PathBuf,
    desligar: Option<oneshot::Sender<()>>,
}

impl Cenario {
    async fn novo() -> Self {
        // O contador atômico evita que dois testes que chamem `novo()` no mesmo instante
        // (relógio de resolução grossa sob carga) dividam a mesma pasta e derrubem um ao outro
        // com "database is locked". O carimbo continua para não colidir entre execuções.
        static SEQUENCIA: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duracao| duracao.as_nanos())
            .unwrap_or(0);
        let sequencia = SEQUENCIA.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let pasta = std::env::temp_dir().join(format!(
            "poph-console-{}-{carimbo}-{sequencia}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        config.validar().expect("configuração válida");

        let store = std::sync::Arc::new(Store::abrir(&config).expect("abre o banco"));
        let estado = Estado::novo(store, config).expect("estado do servidor");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, sinal) = oneshot::channel();
        tokio::spawn(async move {
            let _ = poph_server::servir_em(listener, estado, async {
                let _ = sinal.await;
            })
            .await;
        });

        let cenario = Self {
            base: format!("http://{endereco}"),
            cliente: Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("cliente HTTP"),
            pasta,
            desligar: Some(desligar),
        };
        cenario.esperar_subir().await;
        cenario
    }

    async fn esperar_subir(&self) {
        let inicio = Instant::now();
        loop {
            match self
                .cliente
                .get(format!("{}/healthz", self.base))
                .send()
                .await
            {
                Ok(resposta) if resposta.status().is_success() => return,
                _ if inicio.elapsed() < Duration::from_secs(10) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                outro => panic!("servidor não subiu: {outro:?}"),
            }
        }
    }

    fn url(&self, caminho: &str) -> String {
        format!("{}{caminho}", self.base)
    }

    async fn obter(&self, caminho: &str) -> reqwest::Response {
        self.cliente
            .get(self.url(caminho))
            .send()
            .await
            .expect("requisição GET")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn tipo(resposta: &reqwest::Response) -> String {
    resposta
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|valor| valor.to_str().ok())
        .unwrap_or("")
        .to_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raiz_entrega_o_console_ou_a_pagina_de_aviso() {
    let cenario = Cenario::novo().await;
    let resposta = cenario.obter("/").await;
    assert_eq!(resposta.status(), StatusCode::OK);
    assert!(
        tipo(&resposta).starts_with("text/html"),
        "a raiz tem de ser HTML, veio `{}`",
        tipo(&resposta)
    );
    let corpo = resposta.text().await.expect("corpo");
    if poph_server::console::tem_bundle() {
        assert!(
            corpo.contains("<div id=\"root\">"),
            "sem o ponto de montagem da SPA: {corpo}"
        );
    } else {
        assert!(
            corpo.contains("Console não compilado"),
            "sem bundle, a página explica como gerar: {corpo}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rota_do_cliente_cai_no_index() {
    let cenario = Cenario::novo().await;
    let raiz = cenario
        .obter("/")
        .await
        .text()
        .await
        .expect("corpo da raiz");
    for rota in [
        "/orgs",
        "/orgs/algum-id",
        "/login",
        "/qualquer/rota/profunda",
    ] {
        let resposta = cenario.obter(rota).await;
        assert_eq!(resposta.status(), StatusCode::OK, "rota {rota}");
        assert!(tipo(&resposta).starts_with("text/html"), "rota {rota}");
        let corpo = resposta.text().await.expect("corpo");
        assert_eq!(corpo, raiz, "a rota {rota} tem de servir o index.html");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn asset_inexistente_nao_vira_pagina() {
    let cenario = Cenario::novo().await;
    let resposta = cenario.obter("/assets/nao-existe.js").await;
    assert_eq!(resposta.status(), StatusCode::NOT_FOUND);
    assert!(
        tipo(&resposta).starts_with("application/json"),
        "veio `{}`",
        tipo(&resposta)
    );
    let corpo: serde_json::Value = resposta.json().await.expect("JSON de erro");
    assert_eq!(corpo["code"], "not_found");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rotas_do_servidor_nao_sao_sombreadas() {
    let cenario = Cenario::novo().await;

    // API sem token continua 401 em JSON.
    let api = cenario.obter("/admin/v1/orgs").await;
    assert_eq!(api.status(), StatusCode::UNAUTHORIZED);
    assert!(tipo(&api).starts_with("application/json"));

    // Rota inexistente dentro da API continua erro em JSON, com o caminho.
    let inexistente = cenario.obter("/admin/v1/nao-existe").await;
    assert_eq!(inexistente.status(), StatusCode::NOT_FOUND);
    assert!(tipo(&inexistente).starts_with("application/json"));
    let corpo: serde_json::Value = inexistente.json().await.expect("JSON de erro");
    assert_eq!(corpo["code"], "not_found");
    assert_eq!(corpo["details"]["path"], "/admin/v1/nao-existe");

    // Saúde e métricas seguem como sempre.
    let saude = cenario.obter("/healthz").await;
    assert_eq!(saude.status(), StatusCode::OK);
    assert!(tipo(&saude).starts_with("application/json"));

    let metricas = cenario.obter("/metrics").await;
    assert_eq!(metricas.status(), StatusCode::OK);
    assert!(
        tipo(&metricas).starts_with("text/plain"),
        "veio `{}`",
        tipo(&metricas)
    );

    // Webhook do WhatsApp: fora do Console, com o formato próprio.
    let webhook = cenario.obter("/webhook/evolution/org/instancia").await;
    assert_ne!(webhook.status(), StatusCode::OK);
    assert!(tipo(&webhook).starts_with("application/json"));

    // A OpenAPI publicada continua JSON.
    let openapi = cenario.obter("/admin/v1/openapi.json").await;
    assert_eq!(openapi.status(), StatusCode::OK);
    assert!(tipo(&openapi).starts_with("application/json"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn metodo_diferente_de_get_nao_recebe_o_console() {
    let cenario = Cenario::novo().await;
    let resposta = cenario
        .cliente
        .request(Method::POST, cenario.url("/orgs"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .expect("POST /orgs");
    assert_eq!(resposta.status(), StatusCode::NOT_FOUND);
    assert!(tipo(&resposta).starts_with("application/json"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fontes_da_sofa_ui_saem_do_binario() {
    let cenario = Cenario::novo().await;
    let resposta = cenario
        .obter("/res/fonts/IBMPlexSans/latin-400-normal.woff2")
        .await;
    if poph_server::console::tem_bundle() {
        assert_eq!(resposta.status(), StatusCode::OK);
        assert_eq!(tipo(&resposta), "font/woff2");
        let corpo = resposta.bytes().await.expect("corpo da fonte");
        assert!(corpo.len() > 1_000, "fonte vazia ou truncada");
        // WOFF2 é um arquivo binário com a assinatura `wOF2`.
        assert_eq!(&corpo[..4], b"wOF2");
    } else {
        // Sem bundle não há fonte: 404 em JSON, nunca HTML no lugar da fonte.
        assert_eq!(resposta.status(), StatusCode::NOT_FOUND);
        assert!(tipo(&resposta).starts_with("application/json"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cabecalhos_de_seguranca_acompanham_a_pagina() {
    let cenario = Cenario::novo().await;
    let resposta = cenario.obter("/").await;
    let cabecalhos = resposta.headers();
    let csp = cabecalhos
        .get("content-security-policy")
        .and_then(|valor| valor.to_str().ok())
        .unwrap_or("");
    assert!(csp.contains("default-src 'self'"), "CSP ausente: `{csp}`");
    assert!(
        csp.contains("frame-ancestors 'none'"),
        "CSP sem anti-framing"
    );
    assert_eq!(
        cabecalhos
            .get("x-content-type-options")
            .and_then(|valor| valor.to_str().ok()),
        Some("nosniff")
    );
    assert_eq!(
        cabecalhos
            .get("referrer-policy")
            .and_then(|valor| valor.to_str().ok()),
        Some("no-referrer")
    );
    // A página entra sempre revalidada: um deploy novo não pode ficar preso no cache.
    assert_eq!(
        cabecalhos
            .get("cache-control")
            .and_then(|valor| valor.to_str().ok()),
        Some("no-cache")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn head_devolve_os_mesmos_cabecalhos_sem_corpo() {
    let cenario = Cenario::novo().await;
    let resposta = cenario
        .cliente
        .head(cenario.url("/"))
        .send()
        .await
        .expect("HEAD /");
    assert_eq!(resposta.status(), StatusCode::OK);
    assert!(tipo(&resposta).starts_with("text/html"));
    let corpo = resposta.bytes().await.expect("corpo do HEAD");
    assert!(corpo.is_empty(), "HEAD não devolve corpo");
}
