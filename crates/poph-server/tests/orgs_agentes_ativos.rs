//! `agents_active_count` na listagem de orgs (CF-03a): a coluna "agentes ativos" do Console.
//!
//! Servidor de verdade, porta efêmera e banco temporário; cliente HTTP de verdade. O que fica
//! provado aqui:
//!
//! - o campo vem **sempre** na listagem (sem `cluster`) e vale o número de agentes com
//!   `status = active` — inativo não conta;
//! - org sem agente ativo devolve `0` (e não some da lista);
//! - a paginação por cursor continua igual e cada página traz o campo;
//! - a página custa o mesmo número de leituras do banco seja de 1 org ou de 12: **não** existe
//!   consulta por linha (`Store::leituras_feitas`);
//! - a leitura de uma org só (`GET /admin/v1/orgs/{id}`) e a listagem de um token de org trazem o
//!   mesmo número, sem campo inventado;
//! - org recém-criada devolve `0`.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::ServerState;
use poph_store::{AgentChange, Status, Store};
use poph_types::HarnessConfig;
use poph_types::config::GlobalTokenConfig;
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

/// Valor do token global usado no teste (o servidor guarda só o hash).
const TOKEN_GLOBAL: &str = "pophg_teste-global-cf03a";

fn hash_argon2(valor: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(valor.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

/// Servidor no ar, com banco e pasta próprios. O `Drop` desliga e apaga a pasta.
struct Cenario {
    base: String,
    cliente: Client,
    pasta: PathBuf,
    store: std::sync::Arc<Store>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Cenario {
    async fn novo(rotulo: &str) -> Self {
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duracao| duracao.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-orgs-ativos-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        config.server.port = 7480;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = std::sync::Arc::new(Store::open(&config).expect("abre o banco"));
        let estado =
            ServerState::new(std::sync::Arc::clone(&store), config).expect("estado do servidor");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, sinal) = oneshot::channel();
        tokio::spawn(async move {
            let _ = poph_server::serve_at(listener, estado, async {
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
            store,
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

    async fn pedir(
        &self,
        metodo: Method,
        caminho: &str,
        token: &str,
        corpo: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut pedido = self
            .cliente
            .request(metodo, format!("{}{caminho}", self.base));
        if !token.is_empty() {
            pedido = pedido.bearer_auth(token);
        }
        if let Some(valor) = corpo {
            pedido = pedido.json(&valor);
        }
        let resposta = pedido.send().await.expect("pedido HTTP");
        let status = resposta.status();
        let texto = resposta.text().await.unwrap_or_default();
        let json = if texto.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&texto).unwrap_or(Value::String(texto))
        };
        (status, json)
    }

    async fn criar_org(&self, nome: &str, slug: &str) -> String {
        let (status, corpo) = self
            .pedir(
                Method::POST,
                "/admin/v1/orgs",
                TOKEN_GLOBAL,
                Some(json!({ "name": nome, "slug": slug })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar org: {corpo}");
        assert_eq!(
            corpo["agents_active_count"],
            json!(0),
            "org recém-criada não tem agente: {corpo}"
        );
        corpo["id"].as_str().expect("id da org").to_owned()
    }

    async fn criar_agente(&self, org_id: &str, slug: &str) -> String {
        let (status, corpo) = self
            .pedir(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/agents"),
                TOKEN_GLOBAL,
                Some(json!({ "name": format!("Agente {slug}"), "slug": slug })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar agente: {corpo}");
        corpo["id"].as_str().expect("id do agente").to_owned()
    }

    async fn criar_token(&self, org_id: &str) -> String {
        let (status, corpo) = self
            .pedir(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/tokens"),
                TOKEN_GLOBAL,
                Some(json!({ "label": "org" })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar token: {corpo}");
        corpo["value"].as_str().expect("valor do token").to_owned()
    }

    /// Itens da listagem, na ordem devolvida.
    async fn listar(&self, token: &str, consulta: &str) -> Value {
        let (status, corpo) = self
            .pedir(
                Method::GET,
                &format!("/admin/v1/orgs{consulta}"),
                token,
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "listar orgs: {corpo}");
        corpo
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

/// Acha o item da org na resposta e devolve o `agents_active_count`.
fn ativos(item: &Value, org_id: &str) -> u32 {
    assert_eq!(item["id"], json!(org_id), "item de outra org: {item}");
    let valor = item
        .get("agents_active_count")
        .unwrap_or_else(|| panic!("o item tem de trazer `agents_active_count` sempre: {item}"));
    assert!(
        valor.is_u64(),
        "`agents_active_count` tem de ser número inteiro, não `null`: {item}"
    );
    u32::try_from(valor.as_u64().unwrap_or(0)).expect("cabe em u32")
}

fn item(itens: &[Value], org_id: &str) -> Value {
    itens
        .iter()
        .find(|item| item["id"] == json!(org_id))
        .unwrap_or_else(|| panic!("org {org_id} fora da listagem: {itens:?}"))
        .clone()
}

/// Listagem: 3 ativos + 1 inativo contam 3; org sem agente devolve 0.
#[tokio::test(flavor = "multi_thread")]
async fn listagem_traz_agentes_ativos_por_org() {
    let cenario = Cenario::novo("contagem").await;
    let com_agentes = cenario.criar_org("Com agentes", "com-agentes").await;
    let sem_agentes = cenario.criar_org("Sem agentes", "sem-agentes").await;

    for slug in ["agente-um", "agente-dois", "agente-tres"] {
        cenario.criar_agente(&com_agentes, slug).await;
    }
    // O quarto agente fica inativo pelo repositório (o caminho da API para situação é o `PATCH`,
    // que exigiria `If-Match`; aqui o que interessa é a conta, não a rota).
    let inativo = cenario.criar_agente(&com_agentes, "agente-quatro").await;
    let org: poph_types::OrgId = com_agentes.parse().expect("org");
    let agente: poph_types::AgentId = inativo.parse().expect("agente");
    let atual = cenario
        .store
        .agents()
        .get(org, agente)
        .expect("lê o agente");
    cenario
        .store
        .agents()
        .update(
            org,
            agente,
            atual.version,
            AgentChange {
                status: Some(Status::Inactive),
                ..AgentChange::default()
            },
        )
        .expect("inativa o agente");

    let corpo = cenario.listar(TOKEN_GLOBAL, "?limit=200").await;
    let itens = corpo["items"].as_array().expect("items");
    assert_eq!(
        ativos(&item(itens, &com_agentes), &com_agentes),
        3,
        "só os três ativos entram na conta: {corpo}"
    );
    assert_eq!(
        ativos(&item(itens, &sem_agentes), &sem_agentes),
        0,
        "org sem agente ativo devolve 0, e não some da lista: {corpo}"
    );

    // Leitura de uma org só: mesmo número, não um zero de fachada.
    let (status, uma) = cenario
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{com_agentes}"),
            TOKEN_GLOBAL,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "mostrar org: {uma}");
    assert_eq!(ativos(&uma, &com_agentes), 3, "{uma}");

    // Token de org: vê só a própria, com o mesmo número.
    let token_org = cenario.criar_token(&com_agentes).await;
    let propria = cenario.listar(&token_org, "").await;
    let itens = propria["items"].as_array().expect("items");
    assert_eq!(itens.len(), 1, "{propria}");
    assert_eq!(ativos(&itens[0], &com_agentes), 3, "{propria}");
}

/// A paginação por cursor continua igual, e cada página traz o campo.
#[tokio::test(flavor = "multi_thread")]
async fn paginacao_por_cursor_segue_igual() {
    let cenario = Cenario::novo("paginacao").await;
    let mut criadas = Vec::new();
    for indice in 0..3 {
        let org = cenario
            .criar_org(&format!("Org {indice}"), &format!("org-{indice}"))
            .await;
        if indice < 2 {
            cenario.criar_agente(&org, "agente-a").await;
        }
        criadas.push(org);
    }

    let primeira = cenario.listar(TOKEN_GLOBAL, "?limit=2").await;
    let itens = primeira["items"].as_array().expect("items");
    assert_eq!(itens.len(), 2, "{primeira}");
    let cursor = primeira["next_cursor"]
        .as_str()
        .unwrap_or_else(|| panic!("a primeira página tem próxima: {primeira}"))
        .to_owned();
    for item in itens {
        let id = item["id"].as_str().unwrap_or_default();
        assert!(item.get("agents_active_count").is_some(), "{item}");
        let esperado = if id == criadas[2] { 0 } else { 1 };
        assert_eq!(ativos(item, id), esperado, "página 1: {primeira}");
    }

    let segunda = cenario
        .listar(TOKEN_GLOBAL, &format!("?limit=2&cursor={cursor}"))
        .await;
    let itens = segunda["items"].as_array().expect("items");
    assert_eq!(itens.len(), 1, "{segunda}");
    assert!(segunda["next_cursor"].is_null(), "{segunda}");
    assert_eq!(ativos(&itens[0], &criadas[2]), 0, "{segunda}");
}

/// Uma página custa o mesmo número de leituras seja de 1 org ou de 12: não há consulta por linha.
#[tokio::test(flavor = "multi_thread")]
async fn pagina_nao_faz_consulta_por_linha() {
    let cenario = Cenario::novo("sem-n-mais-um").await;
    for indice in 0..12 {
        let org = cenario
            .criar_org(&format!("Org {indice}"), &format!("org-{indice}"))
            .await;
        cenario.criar_agente(&org, "agente-a").await;
    }

    let antes = cenario.store.reads_done();
    let pequena = cenario.listar(TOKEN_GLOBAL, "?limit=1").await;
    let leituras_pequena = cenario.store.reads_done() - antes;

    let antes = cenario.store.reads_done();
    let grande = cenario.listar(TOKEN_GLOBAL, "?limit=200").await;
    let leituras_grande = cenario.store.reads_done() - antes;

    assert_eq!(
        pequena["items"].as_array().map(Vec::len),
        Some(1),
        "{pequena}"
    );
    assert_eq!(
        grande["items"].as_array().map(Vec::len),
        Some(12),
        "{grande}"
    );
    assert!(
        leituras_grande < 12,
        "a página de 12 orgs custou {leituras_grande} leituras: isso é uma consulta por linha          (a de 1 org custou {leituras_pequena})"
    );
}
