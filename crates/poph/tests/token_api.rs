//! Ponta a ponta do formato novo do token de org (HR-20, item 2).
//!
//! O token é **criado pela CLI** (`poph --offline token create`) e **usado na API**: o servidor é
//! subido em processo, na mesma pasta de dados, com porta efêmera. Prova que os dois lados
//! combinam o formato `poph_<uuid do token>_<segredo>`, que o segredo em claro só aparece na
//! criação e que um segredo errado não autentica.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::Arc;

use poph_server::Estado;
use poph_store::Store;
use poph_types::HarnessConfig;
use reqwest::StatusCode;
use serde_json::Value;
use tokio::sync::oneshot;

struct Cenario {
    pasta: PathBuf,
    dados: PathBuf,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta =
            std::env::temp_dir().join(format!("poph-token-api-{rotulo}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        let dados = pasta.join("dados");
        fs::create_dir_all(&dados).expect("pasta de dados");
        Self { pasta, dados }
    }

    fn poph(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_poph"))
            .args(args)
            .current_dir(&self.pasta)
            .env_clear()
            .env("POPH_DADOS_DIRETORIO", &self.dados)
            .output()
            .expect("executa o poph")
    }

    fn criar_org(&self, slug: &str) -> String {
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
        assert_eq!(codigo(&saida), 0, "{}", texto(&saida.stderr));
        json(&saida)["org"]["id"]
            .as_str()
            .expect("id da org")
            .to_owned()
    }

    /// Cria o token pela CLI e devolve o valor em claro (só existe nesta saída).
    fn criar_token(&self, slug: &str) -> String {
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
        assert_eq!(codigo(&saida), 0, "{}", texto(&saida.stderr));
        json(&saida)["valor_do_token"]
            .as_str()
            .expect("valor do token")
            .to_owned()
    }

    /// Sobe o servidor em processo na pasta de dados do teste e devolve o endereço.
    async fn subir_servidor(&self) -> (String, oneshot::Sender<()>) {
        let mut config = HarnessConfig::default();
        config.data.directory = self.dados.clone();
        config.validar().expect("configuração válida");
        let store = Arc::new(Store::open(&config).expect("abre o banco"));
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
        let cliente = reqwest::Client::new();
        let base = format!("http://{endereco}");
        let inicio = std::time::Instant::now();
        loop {
            match cliente.get(format!("{base}/healthz")).send().await {
                Ok(resposta) if resposta.status().is_success() => break,
                _ if inicio.elapsed() < std::time::Duration::from_secs(10) => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                outro => panic!("servidor não subiu: {outro:?}"),
            }
        }
        (base, desligar)
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn codigo(saida: &Output) -> i32 {
    saida.status.code().unwrap_or(-1)
}

fn texto(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn json(saida: &Output) -> Value {
    serde_json::from_str(&texto(&saida.stdout))
        .unwrap_or_else(|erro| panic!("saída não é JSON ({erro}): {}", texto(&saida.stdout)))
}

async fn pedir(cliente: &reqwest::Client, url: &str, token: &str) -> (StatusCode, Value) {
    let resposta = cliente
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .expect("pedido HTTP");
    let status = resposta.status();
    let json = resposta.json().await.unwrap_or(Value::Null);
    (status, json)
}

#[tokio::test(flavor = "multi_thread")]
async fn token_criado_pela_cli_autentica_na_api() {
    let cenario = Cenario::novo("e2e");
    let org_id = cenario.criar_org("loja-teste");
    let outro_org = cenario.criar_org("outra-loja");
    let token = cenario.criar_token("loja-teste");

    // Formato novo: `poph_<uuid v7 do token>_<segredo>`.
    let resto = token.strip_prefix("poph_").expect("prefixo poph_");
    let (id, segredo) = resto.split_once('_').expect("uuid e segredo");
    assert!(id.len() == 36 && segredo.len() >= 20, "valor: {token}");
    assert!(!outro_org.is_empty() && !org_id.is_empty());

    let (base, desligar) = cenario.subir_servidor().await;
    let cliente = reqwest::Client::new();

    // O token da CLI autentica e enxerga só a própria org.
    let (status, corpo) = pedir(&cliente, &format!("{base}/admin/v1/orgs"), &token).await;
    assert_eq!(status, StatusCode::OK, "listar orgs: {corpo}");
    let orgs = corpo["items"].as_array().expect("lista de orgs");
    assert_eq!(orgs.len(), 1, "token de org viu mais de uma org: {corpo}");
    assert_eq!(orgs[0]["id"].as_str(), Some(org_id.as_str()));

    // A própria org: 200. Outra org: barrada.
    let (status, _) = pedir(&cliente, &format!("{base}/admin/v1/orgs/{org_id}"), &token).await;
    assert_eq!(status, StatusCode::OK);
    let (status, corpo) = pedir(
        &cliente,
        &format!("{base}/admin/v1/orgs/{outro_org}"),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "outra org: {corpo}");

    // Segredo errado e uuid inexistente não autenticam.
    let errado = format!("poph_{id}_segredo-errado-que-nao-e-o-certo");
    let (status, _) = pedir(&cliente, &format!("{base}/admin/v1/orgs"), &errado).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let inexistente = format!("poph_01920000-0000-7000-8000-000000000000_{segredo}");
    let (status, _) = pedir(&cliente, &format!("{base}/admin/v1/orgs"), &inexistente).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Nem `list` nem `show` nem `--json` devolvem o valor depois da criação.
    let listar = cenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "token",
        "list",
        "--json",
    ]);
    assert_eq!(codigo(&listar), 0, "{}", texto(&listar.stderr));
    assert!(
        !texto(&listar.stdout).contains(segredo),
        "o segredo vazou na lista: {}",
        texto(&listar.stdout)
    );
    let mostrar = cenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "token",
        "show",
        id,
        "--json",
    ]);
    assert_eq!(codigo(&mostrar), 0, "{}", texto(&mostrar.stderr));
    assert!(!texto(&mostrar.stdout).contains(segredo));

    let _ = desligar.send(());
}
