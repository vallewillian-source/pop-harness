use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::Arc;

use poph_server::ServerState;
use poph_store::Store;
use poph_types::HarnessConfig;
use reqwest::StatusCode;
use serde_json::Value;
use tokio::sync::oneshot;

struct Scenario {
    dir: PathBuf,
    dados: PathBuf,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("poph-token-api-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let dados = dir.join("dados");
        fs::create_dir_all(&dados).expect("pasta de dados");
        Self { dir, dados }
    }

    fn poph(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_poph"))
            .args(args)
            .current_dir(&self.dir)
            .env_clear()
            .env("POPH_DADOS_DIRETORIO", &self.dados)
            .output()
            .expect("executa o poph")
    }

    fn create_org(&self, slug: &str) -> String {
        let saida = self.poph(&[
            "--offline",
            "org",
            "create",
            "--name",
            &format!("Org {slug}"),
            "--slug",
            slug,
            "--json",
        ]);
        assert_eq!(exit_code(&saida), 0, "{}", texto(&saida.stderr));
        json(&saida)["org"]["id"]
            .as_str()
            .expect("id da org")
            .to_owned()
    }

    fn create_token(&self, slug: &str) -> String {
        let saida = self.poph(&[
            "--offline",
            "--org",
            slug,
            "token",
            "create",
            "--label",
            "api",
            "--json",
        ]);
        assert_eq!(exit_code(&saida), 0, "{}", texto(&saida.stderr));
        json(&saida)["valor_do_token"]
            .as_str()
            .expect("valor do token")
            .to_owned()
    }

    async fn start_server(&self) -> (String, oneshot::Sender<()>) {
        let mut config = HarnessConfig::default();
        config.data.directory = self.dados.clone();
        config.validar().expect("configuração válida");
        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let estado = ServerState::new(store, config).expect("estado do servidor");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, shutdown) = oneshot::channel();
        tokio::spawn(async move {
            let _ = poph_server::serve_at(listener, estado, async {
                let _ = shutdown.await;
            })
            .await;
        });
        let client = reqwest::Client::new();
        let base = format!("http://{endereco}");
        let started = std::time::Instant::now();
        loop {
            match client.get(format!("{base}/healthz")).send().await {
                Ok(response) if response.status().is_success() => break,
                _ if started.elapsed() < std::time::Duration::from_secs(10) => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                other => panic!("servidor não subiu: {other:?}"),
            }
        }
        (base, desligar)
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn exit_code(saida: &Output) -> i32 {
    saida.status.code().unwrap_or(-1)
}

fn texto(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn json(saida: &Output) -> Value {
    serde_json::from_str(&texto(&saida.stdout))
        .unwrap_or_else(|error| panic!("saída não é JSON ({error}): {}", texto(&saida.stdout)))
}

async fn request(client: &reqwest::Client, url: &str, token: &str) -> (StatusCode, Value) {
    let response = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .expect("pedido HTTP");
    let status = response.status();
    let json = response.json().await.unwrap_or(Value::Null);
    (status, json)
}

#[tokio::test(flavor = "multi_thread")]
async fn token_created_by_cli_authenticates_against_api() {
    let scenario = Scenario::new("e2e");
    let org_id = scenario.create_org("loja-teste");
    let other_org = scenario.create_org("outra-loja");
    let token = scenario.create_token("loja-teste");

    let rest = token.strip_prefix("poph_").expect("prefixo poph_");
    let (id, secret) = rest.split_once('_').expect("uuid e segredo");
    assert!(id.len() == 36 && secret.len() >= 20, "valor: {token}");
    assert!(!other_org.is_empty() && !org_id.is_empty());

    let (base, desligar) = scenario.start_server().await;
    let client = reqwest::Client::new();

    let (status, body) = request(&client, &format!("{base}/admin/v1/orgs"), &token).await;
    assert_eq!(status, StatusCode::OK, "listar orgs: {body}");
    let orgs = body["items"].as_array().expect("lista de orgs");
    assert_eq!(orgs.len(), 1, "token de org viu mais de uma org: {body}");
    assert_eq!(orgs[0]["id"].as_str(), Some(org_id.as_str()));

    let (status, _) = request(&client, &format!("{base}/admin/v1/orgs/{org_id}"), &token).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = request(
        &client,
        &format!("{base}/admin/v1/orgs/{other_org}"),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "outra org: {body}");

    let wrong = format!("poph_{id}_segredo-errado-que-nao-e-o-certo");
    let (status, _) = request(&client, &format!("{base}/admin/v1/orgs"), &wrong).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let missing = format!("poph_01920000-0000-7000-8000-000000000000_{secret}");
    let (status, _) = request(&client, &format!("{base}/admin/v1/orgs"), &missing).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let listar = scenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "token",
        "list",
        "--json",
    ]);
    assert_eq!(exit_code(&listar), 0, "{}", texto(&listar.stderr));
    assert!(
        !texto(&listar.stdout).contains(secret),
        "o segredo vazou na lista: {}",
        texto(&listar.stdout)
    );
    let mostrar = scenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "token",
        "show",
        id,
        "--json",
    ]);
    assert_eq!(exit_code(&mostrar), 0, "{}", texto(&mostrar.stderr));
    assert!(!texto(&mostrar.stdout).contains(secret));

    let _ = desligar.send(());
}
