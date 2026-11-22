//! Testes de ponta a ponta da API administrativa (HR-20).
//!
//! Servidor de verdade (porta efêmera escolhida pelo sistema) sobre banco temporário, e cliente
//! HTTP de verdade: nada de chamar o handler direto. Cada cenário tem a própria pasta de dados.
//!
//! O que se prova aqui:
//! - token ausente, malformado ou revogado dá 401;
//! - token de org A recebe 403 em TODAS as rotas da org B, e a listagem de orgs só traz a própria;
//! - token de org não cria nem exclui org/token (ação de token global);
//! - `/healthz` e `/metrics` respondem sem autenticação, com o formato combinado;
//! - a OpenAPI publicada é válida (oas3 3.1) e cobre todas as rotas do roteador.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::Estado;
use poph_store::Store;
use poph_types::HarnessConfig;
use poph_types::config::TokenGlobalConfig;
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

/// Valor do token global usado nos testes (o servidor guarda só o hash).
const TOKEN_GLOBAL: &str = "pophg_teste-global-hr20";

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
            "poph-server-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.dados.diretorio = pasta.join("dados");
        if rotulo == "knowledge-real-model" {
            config.dados.modelos =
                Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/modelos"));
        }
        config.servidor.porta = 7480;
        config.admin.tokens_globais = vec![TokenGlobalConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = std::sync::Arc::new(Store::abrir(&config).expect("abre o banco"));
        let estado =
            Estado::novo(std::sync::Arc::clone(&store), config).expect("estado do servidor");

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

        let base = format!("http://{endereco}");
        let cliente = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("cliente HTTP");

        let cenario = Self {
            base,
            cliente,
            pasta,
            store,
            desligar: Some(desligar),
        };
        cenario.esperar_subir().await;
        cenario
    }

    /// Espera o `/healthz` responder (o `axum::serve` aceita conexões depois do spawn).
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

    /// Acesso direto ao armazenamento, para montar estado que a API ainda não expõe.
    fn store(&self) -> &Store {
        &self.store
    }

    fn url(&self, caminho: &str) -> String {
        format!("{}{caminho}", self.base)
    }

    /// Requisição autenticada (ou anônima, com `token` vazio).
    async fn pedir(
        &self,
        metodo: Method,
        caminho: &str,
        token: &str,
        corpo: Option<Value>,
        if_match: Option<u64>,
    ) -> (StatusCode, Value) {
        let mut pedido = self.cliente.request(metodo, self.url(caminho));
        if !token.is_empty() {
            pedido = pedido.bearer_auth(token);
        }
        if let Some(versao) = if_match {
            pedido = pedido.header("If-Match", versao.to_string());
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

    /// Envia bytes crus (`application/octet-stream`), como o upload da biblioteca de arquivos.
    async fn enviar_arquivo(
        &self,
        caminho: &str,
        token: &str,
        bytes: &[u8],
    ) -> (StatusCode, Value) {
        let resposta = self
            .cliente
            .post(self.url(caminho))
            .bearer_auth(token)
            .header("content-type", "application/octet-stream")
            .body(bytes.to_vec())
            .send()
            .await
            .expect("pedido HTTP");
        let status = resposta.status();
        let texto = resposta.text().await.unwrap_or_default();
        let json = if texto.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&texto).unwrap_or(Value::String(texto))
        };
        (status, json)
    }

    async fn criar_org(&self, nome: &str, slug: &str) -> Value {
        let (status, corpo) = self
            .pedir(
                Method::POST,
                "/admin/v1/orgs",
                TOKEN_GLOBAL,
                Some(json!({ "name": nome, "slug": slug })),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar org: {corpo}");
        corpo
    }

    async fn criar_token(&self, org_id: &str, rotulo: &str) -> String {
        let (status, corpo) = self
            .pedir(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/tokens"),
                TOKEN_GLOBAL,
                Some(json!({ "label": rotulo })),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar token: {corpo}");
        let valor = corpo["value"].as_str().expect("valor do token").to_owned();
        assert!(
            valor.starts_with("poph_"),
            "valor fora do formato novo: {valor}"
        );
        valor
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        // O Store pode ainda ter o banco aberto; a remoção é best-effort.
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn codigo(corpo: &Value) -> &str {
    corpo["code"].as_str().unwrap_or("")
}

#[tokio::test]
async fn biblioteca_consulta_isola_org_e_agente() {
    let c = Cenario::novo("biblioteca-api").await;
    let org_a = c.criar_org("Org A", "org-a").await;
    let org_b = c.criar_org("Org B", "org-b").await;
    let a = org_a["id"].as_str().unwrap();
    let b = org_b["id"].as_str().unwrap();
    let token = c.criar_token(a, "org a").await;
    let agente = c
        .store()
        .agentes()
        .criar(
            b.parse().unwrap(),
            poph_store::NovoAgente {
                nome: "Agente".into(),
                slug: "agente".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let arquivo = c
        .store()
        .biblioteca()
        .criar(
            b.parse().unwrap(),
            poph_store::NovoArquivoBiblioteca {
                agent_id: agente.id,
                nome: "Manual".into(),
                descricao: "Instruções".into(),
                mime: "application/pdf".into(),
                tamanho_bytes: 42,
                sha256: "a".repeat(64),
                blob_path: "privado".into(),
            },
        )
        .unwrap();
    let colecao = format!("/admin/v1/orgs/{b}/agents/{}/files", agente.id);
    let item = format!("{colecao}/{}", arquivo.id);
    let conteudo = format!("{item}/content");
    for rota in [&colecao, &item, &conteudo] {
        assert_eq!(
            c.pedir(Method::GET, rota, "", None, None).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            c.pedir(Method::GET, rota, &token, None, None).await.0,
            StatusCode::FORBIDDEN
        );
    }
    let (status, lista) = c
        .pedir(Method::GET, &colecao, TOKEN_GLOBAL, None, None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(lista["items"][0]["name"], "Manual");
    assert!(lista["items"][0].get("blob_path").is_none());
    let (status, mostrado) = c.pedir(Method::GET, &item, TOKEN_GLOBAL, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mostrado["id"], arquivo.id.to_string());
    let outro = c
        .store()
        .agentes()
        .criar(
            b.parse().unwrap(),
            poph_store::NovoAgente {
                nome: "Outro".into(),
                slug: "outro".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let rota_alheia = format!(
        "/admin/v1/orgs/{b}/agents/{}/files/{}",
        outro.id, arquivo.id
    );
    assert_eq!(
        c.pedir(Method::GET, &rota_alheia, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        c.pedir(Method::DELETE, &item, "", None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        c.pedir(Method::DELETE, &item, &token, None, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        c.pedir(Method::DELETE, &rota_alheia, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let token_b = c.criar_token(b, "org b").await;
    assert_eq!(
        c.pedir(Method::DELETE, &item, &token_b, None, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        c.pedir(Method::GET, &item, &token_b, None, None).await.0,
        StatusCode::NOT_FOUND
    );
    let registros = c
        .store()
        .auditoria()
        .listar(b.parse().unwrap(), poph_store::Pagina::default())
        .unwrap();
    let auditoria = registros
        .iter()
        .find(|r| r.entidade_id.as_deref() == Some(&arquivo.id.to_string()) && r.acao == "delete")
        .expect("exclusão auditada");
    assert_eq!(auditoria.ator_tipo, "admin_token");
    assert_eq!(
        auditoria.ator_id.as_deref(),
        token_b
            .strip_prefix("poph_")
            .and_then(|s| s.split_once('_'))
            .map(|(id, _)| id)
    );
    assert!(!format!("{auditoria:?}").contains(&token_b));
}

// ───────────────────────────── 401 ─────────────────────────────

#[tokio::test]
async fn sem_token_todas_as_rotas_dao_401() {
    let c = Cenario::novo("sem-token").await;
    let org = "0197c9a0-0000-7000-8000-000000000000";
    let rotas: &[(Method, &str, Option<Value>)] = &[
        (Method::GET, "/admin/v1/orgs", None),
        (Method::POST, "/admin/v1/orgs", Some(json!({"name": "x"}))),
        (Method::GET, &format!("/admin/v1/orgs/{org}").clone(), None),
        (
            Method::DELETE,
            &format!("/admin/v1/orgs/{org}").clone(),
            None,
        ),
        (
            Method::GET,
            &format!("/admin/v1/orgs/{org}/tokens").clone(),
            None,
        ),
        (
            Method::POST,
            &format!("/admin/v1/orgs/{org}/tokens").clone(),
            Some(json!({"label": "x"})),
        ),
        (
            Method::GET,
            &format!("/admin/v1/orgs/{org}/categories").clone(),
            None,
        ),
        (
            Method::POST,
            &format!("/admin/v1/orgs/{org}/categories").clone(),
            Some(json!({"name": "x"})),
        ),
        (
            Method::GET,
            &format!("/admin/v1/orgs/{org}/departments").clone(),
            None,
        ),
        (
            Method::POST,
            &format!("/admin/v1/orgs/{org}/departments").clone(),
            Some(json!({"name": "x"})),
        ),
    ];
    for (metodo, caminho, corpo) in rotas {
        let (status, resposta) = c
            .pedir(metodo.clone(), caminho, "", corpo.clone(), None)
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{metodo} {caminho}");
        assert_eq!(codigo(&resposta), "unauthorized", "{metodo} {caminho}");
    }

    // Token malformado e token desconhecido caem no mesmo 401.
    for valor in ["sem-prefixo", "poph_nao-e-uuid_segredo", "pophg_errado"] {
        let (status, resposta) = c
            .pedir(Method::GET, "/admin/v1/orgs", valor, None, None)
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "token `{valor}`");
        assert_eq!(codigo(&resposta), "unauthorized");
    }
}

// ───────────────────────────── Escopo por org ─────────────────────────────

#[tokio::test]
async fn token_de_org_nao_alcanca_outra_org_em_nenhuma_rota() {
    let c = Cenario::novo("escopo").await;
    let org_a = c.criar_org("Org A", "org-a").await;
    let org_b = c.criar_org("Org B", "org-b").await;
    let id_a = org_a["id"].as_str().expect("id da org A").to_owned();
    let id_b = org_b["id"].as_str().expect("id da org B").to_owned();
    let token_a = c.criar_token(&id_a, "token-da-a").await;

    // Categoria e departamento na org B, para provar que não são alcançáveis nem com id forjado.
    let (status, categoria_b) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{id_b}/categories"),
            TOKEN_GLOBAL,
            Some(json!({ "name": "Categoria B" })),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{categoria_b}");
    let categoria_b_id = categoria_b["id"]
        .as_str()
        .expect("id da categoria")
        .to_owned();
    let token_b_id = c.criar_token(&id_b, "token-da-b").await;
    // O valor do token de B começa com `poph_<id>_`; o id sai da parte do meio.
    let token_b_id = token_b_id
        .strip_prefix("poph_")
        .and_then(|resto| resto.split_once('_'))
        .map(|(id, _)| id.to_owned())
        .expect("id no valor do token");

    let rotas: Vec<(Method, String, Option<Value>, Option<u64>)> = vec![
        (Method::GET, format!("/admin/v1/orgs/{id_b}"), None, None),
        (
            Method::PATCH,
            format!("/admin/v1/orgs/{id_b}"),
            Some(json!({"name": "Invadida"})),
            Some(1),
        ),
        (Method::DELETE, format!("/admin/v1/orgs/{id_b}"), None, None),
        (
            Method::GET,
            format!("/admin/v1/orgs/{id_b}/tokens"),
            None,
            None,
        ),
        (
            Method::POST,
            format!("/admin/v1/orgs/{id_b}/tokens"),
            Some(json!({"label": "x"})),
            None,
        ),
        (
            Method::GET,
            format!("/admin/v1/orgs/{id_b}/tokens/{token_b_id}"),
            None,
            None,
        ),
        (
            Method::DELETE,
            format!("/admin/v1/orgs/{id_b}/tokens/{token_b_id}"),
            None,
            None,
        ),
        (
            Method::GET,
            format!("/admin/v1/orgs/{id_b}/categories"),
            None,
            None,
        ),
        (
            Method::POST,
            format!("/admin/v1/orgs/{id_b}/categories"),
            Some(json!({"name": "x"})),
            None,
        ),
        (
            Method::GET,
            format!("/admin/v1/orgs/{id_b}/categories/{categoria_b_id}"),
            None,
            None,
        ),
        (
            Method::PATCH,
            format!("/admin/v1/orgs/{id_b}/categories/{categoria_b_id}"),
            Some(json!({"name": "x"})),
            Some(1),
        ),
        (
            Method::DELETE,
            format!("/admin/v1/orgs/{id_b}/categories/{categoria_b_id}"),
            None,
            None,
        ),
        (
            Method::GET,
            format!("/admin/v1/orgs/{id_b}/departments"),
            None,
            None,
        ),
        (
            Method::POST,
            format!("/admin/v1/orgs/{id_b}/departments"),
            Some(json!({"name": "x"})),
            None,
        ),
        (
            Method::GET,
            format!("/admin/v1/orgs/{id_b}/departments/{categoria_b_id}"),
            None,
            None,
        ),
        (
            Method::DELETE,
            format!("/admin/v1/orgs/{id_b}/departments/{categoria_b_id}"),
            None,
            None,
        ),
    ];

    for (metodo, caminho, corpo, if_match) in rotas {
        let (status, resposta) = c
            .pedir(metodo.clone(), &caminho, &token_a, corpo, if_match)
            .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{metodo} {caminho} deveria ser barrado por escopo, veio {resposta}"
        );
        assert_eq!(codigo(&resposta), "forbidden", "{metodo} {caminho}");
    }

    // E a categoria da org B continua lá, intacta.
    let (status, _) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{id_b}/categories/{categoria_b_id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn token_de_org_so_enxerga_a_propria_org() {
    let c = Cenario::novo("propria-org").await;
    let org_a = c.criar_org("Org A", "org-a").await;
    let org_b = c.criar_org("Org B", "org-b").await;
    let id_a = org_a["id"].as_str().expect("id A").to_owned();
    let id_b = org_b["id"].as_str().expect("id B").to_owned();
    let token_a = c.criar_token(&id_a, "token-da-a").await;

    // Listagem de orgs: só a própria.
    let (status, pagina) = c
        .pedir(Method::GET, "/admin/v1/orgs", &token_a, None, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{pagina}");
    let itens = pagina["items"].as_array().expect("lista de orgs");
    assert_eq!(itens.len(), 1, "token de org só vê a própria: {pagina}");
    assert_eq!(itens[0]["id"].as_str(), Some(id_a.as_str()));
    assert!(itens.iter().all(|org| org["id"] != json!(id_b)));

    // A própria org: 200. Categoria e departamento da própria org: cria e lê.
    let (status, _) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{id_a}"),
            &token_a,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, categoria) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{id_a}/categories"),
            &token_a,
            Some(json!({ "name": "Categoria A" })),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{categoria}");

    let (status, departamento) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{id_a}/departments"),
            &token_a,
            Some(json!({ "name": "Departamento A" })),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{departamento}");

    for colecao in ["categories", "departments", "tokens"] {
        let (status, pagina) = c
            .pedir(
                Method::GET,
                &format!("/admin/v1/orgs/{id_a}/{colecao}"),
                &token_a,
                None,
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{colecao}: {pagina}");
        let itens = pagina["items"].as_array().expect("itens");
        assert!(!itens.is_empty(), "`{colecao}` da própria org veio vazia");
        assert!(
            itens.iter().all(|item| item["org_id"] == json!(id_a)),
            "`{colecao}` trouxe item de outra org: {pagina}"
        );
    }
}

#[tokio::test]
async fn token_de_org_nao_cria_org_nem_token() {
    let c = Cenario::novo("sem-global").await;
    let org_a = c.criar_org("Org A", "org-a").await;
    let id_a = org_a["id"].as_str().expect("id A").to_owned();
    let token_a = c.criar_token(&id_a, "token-da-a").await;

    let (status, resposta) = c
        .pedir(
            Method::POST,
            "/admin/v1/orgs",
            &token_a,
            Some(json!({ "name": "Org Nova" })),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{resposta}");
    assert_eq!(codigo(&resposta), "forbidden");

    let (status, resposta) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{id_a}/tokens"),
            &token_a,
            Some(json!({ "label": "outro" })),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{resposta}");

    let (status, resposta) = c
        .pedir(
            Method::DELETE,
            &format!("/admin/v1/orgs/{id_a}"),
            &token_a,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{resposta}");

    // Nada foi criado: a org continua lá e só existe uma org.
    let (_, pagina) = c
        .pedir(Method::GET, "/admin/v1/orgs", TOKEN_GLOBAL, None, None)
        .await;
    assert_eq!(pagina["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(pagina["items"][0]["id"].as_str(), Some(id_a.as_str()));
}

// ───────────────────────────── Token revogado ─────────────────────────────

#[tokio::test]
async fn token_revogado_deixa_de_autenticar() {
    let c = Cenario::novo("revogado").await;
    let org = c.criar_org("Org A", "org-a").await;
    let id = org["id"].as_str().expect("id").to_owned();
    let valor = c.criar_token(&id, "descartavel").await;
    let token_id = valor
        .strip_prefix("poph_")
        .and_then(|resto| resto.split_once('_'))
        .map(|(id, _)| id.to_owned())
        .expect("id no valor");

    let (status, _) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{id}"),
            &valor,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "token novo deveria autenticar");

    let (status, _) = c
        .pedir(
            Method::DELETE,
            &format!("/admin/v1/orgs/{id}/tokens/{token_id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, resposta) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{id}"),
            &valor,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{resposta}");
    assert_eq!(codigo(&resposta), "unauthorized");

    // E o segredo não aparece no registro do token (nem em lista nem em detalhe).
    let (_, lista) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{id}/tokens"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    let texto = lista.to_string();
    assert!(
        !texto.contains(&valor),
        "o valor do token vazou na listagem"
    );
    let segredo = valor.rsplit_once('_').map(|(_, s)| s).unwrap_or("");
    assert!(!texto.contains(segredo), "o segredo vazou na listagem");
}

// ───────────────────────────── Escritas e conflitos ─────────────────────────────

#[tokio::test]
async fn escrita_exige_if_match_e_versao_certa() {
    let c = Cenario::novo("if-match").await;
    let org = c.criar_org("Org A", "org-a").await;
    let id = org["id"].as_str().expect("id").to_owned();

    let (status, resposta) = c
        .pedir(
            Method::PATCH,
            &format!("/admin/v1/orgs/{id}"),
            TOKEN_GLOBAL,
            Some(json!({ "name": "Novo nome" })),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{resposta}");
    assert_eq!(codigo(&resposta), "precondition_required");

    let (status, resposta) = c
        .pedir(
            Method::PATCH,
            &format!("/admin/v1/orgs/{id}"),
            TOKEN_GLOBAL,
            Some(json!({ "name": "Novo nome" })),
            Some(99),
        )
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{resposta}");
    assert_eq!(codigo(&resposta), "version_conflict");

    let (status, resposta) = c
        .pedir(
            Method::PATCH,
            &format!("/admin/v1/orgs/{id}"),
            TOKEN_GLOBAL,
            Some(json!({ "name": "Novo nome" })),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{resposta}");
    assert_eq!(resposta["name"], json!("Novo nome"));
    assert_eq!(resposta["version"], json!(2));
}

#[tokio::test]
async fn org_com_agentes_so_sai_com_force() {
    let c = Cenario::novo("force").await;
    let org = c.criar_org("Org A", "org-a").await;
    let id = org["id"].as_str().expect("id").to_owned();

    // Cria um agente direto pelo armazenamento (a API de agentes é tarefa posterior).
    c.store()
        .agentes()
        .criar(
            id.parse().expect("org id"),
            poph_store::NovoAgente {
                nome: "Bia".to_owned(),
                slug: "bia".to_owned(),
                ..Default::default()
            },
        )
        .expect("agente");

    let (status, resposta) = c
        .pedir(
            Method::DELETE,
            &format!("/admin/v1/orgs/{id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{resposta}");
    assert_eq!(codigo(&resposta), "conflict");

    // A org continua lá.
    let (status, _) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = c
        .pedir(
            Method::DELETE,
            &format!("/admin/v1/orgs/{id}?force=true"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn categoria_com_vinculos_so_sai_com_force() {
    let c = Cenario::novo("categoria-force").await;
    let org = c.criar_org("Org Cat", "org-cat").await;
    let id = org["id"].as_str().expect("id").to_owned();
    let raiz = format!("/admin/v1/orgs/{id}");
    let criar = |caminho: String, corpo: Value| {
        let c = &c;
        async move {
            let (status, resposta) = c
                .pedir(Method::POST, &caminho, TOKEN_GLOBAL, Some(corpo), None)
                .await;
            assert_eq!(status, StatusCode::CREATED, "{resposta}");
            resposta
        }
    };
    let usada = criar(format!("{raiz}/categories"), json!({"name":"Usada"})).await;
    let usada_id = usada["id"].as_str().unwrap().to_owned();
    let livre = criar(format!("{raiz}/categories"), json!({"name":"Livre"})).await;
    let livre_id = livre["id"].as_str().unwrap().to_owned();
    let produto = criar(
        format!("{raiz}/products"),
        json!({"name":"Produto da usada", "category_id": usada_id}),
    )
    .await;
    let produto_id = produto["id"].as_str().unwrap().to_owned();
    let agente = criar(
        format!("{raiz}/agents"),
        json!({"name":"Agente", "slug":"agente"}),
    )
    .await;
    let agente_id = agente["id"].as_str().unwrap().to_owned();
    let (status, vinculado) = c
        .pedir(
            Method::PATCH,
            &format!("{raiz}/agents/{agente_id}"),
            TOKEN_GLOBAL,
            Some(json!({"product_category_ids":[usada_id]})),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{vinculado}");
    let cat_path = format!("{raiz}/categories/{usada_id}");

    // Sem force, com produto e agente vinculados: 409, com a contagem, e nada apagado.
    let (status, resposta) = c
        .pedir(Method::DELETE, &cat_path, TOKEN_GLOBAL, None, None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{resposta}");
    assert_eq!(codigo(&resposta), "conflict");
    assert!(
        resposta["message"].as_str().unwrap().contains("force=true"),
        "{resposta}"
    );
    assert_eq!(resposta["details"]["produtos"], 1);
    assert_eq!(resposta["details"]["agentes"], 1);
    assert_eq!(
        c.pedir(Method::GET, &cat_path, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        c.pedir(
            Method::GET,
            &format!("{raiz}/products/{produto_id}"),
            TOKEN_GLOBAL,
            None,
            None
        )
        .await
        .0,
        StatusCode::OK,
        "o produto da categoria não pode sumir sem force"
    );
    let (_, agente_antes) = c
        .pedir(
            Method::GET,
            &format!("{raiz}/agents/{agente_id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(agente_antes["product_category_ids"], json!([usada_id]));

    // Categoria sem vínculo: sem force, apaga como antes.
    let (status, _) = c
        .pedir(
            Method::DELETE,
            &format!("{raiz}/categories/{livre_id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Com force=true: apaga a categoria, os produtos e o vínculo com o agente.
    let (status, resposta) = c
        .pedir(
            Method::DELETE,
            &format!("{cat_path}?force=true"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{resposta}");
    assert_eq!(
        c.pedir(Method::GET, &cat_path, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        c.pedir(
            Method::GET,
            &format!("{raiz}/products/{produto_id}"),
            TOKEN_GLOBAL,
            None,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, agente_depois) = c
        .pedir(
            Method::GET,
            &format!("{raiz}/agents/{agente_id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(agente_depois["product_category_ids"], json!([]));
}

// ───────────────────────────── Fora do /admin: saúde e métricas ─────────────────────────────

#[tokio::test]
async fn healthz_e_metrics_respondem_sem_autenticacao() {
    let c = Cenario::novo("operacao").await;

    let resposta = c
        .cliente
        .get(c.url("/healthz"))
        .send()
        .await
        .expect("healthz");
    assert_eq!(resposta.status(), StatusCode::OK);
    let corpo: Value = resposta.json().await.expect("JSON do healthz");
    assert_eq!(corpo["status"], json!("ok"));
    assert_eq!(corpo["version"], json!(poph_server::VERSAO));
    assert!(corpo["uptime_seconds"].is_u64(), "uptime: {corpo}");
    // A versão acompanha a última migração do esquema (fonte única em `poph-store`).
    assert_eq!(
        corpo["databases"]["harness"],
        json!(poph_store::versao_alvo_harness())
    );
    assert!(corpo["databases"]["rag"].is_u64());

    let resposta = c
        .cliente
        .get(c.url("/metrics"))
        .send()
        .await
        .expect("metrics");
    assert_eq!(resposta.status(), StatusCode::OK);
    let tipo = resposta
        .headers()
        .get("content-type")
        .and_then(|valor| valor.to_str().ok())
        .unwrap_or("");
    assert!(tipo.starts_with("text/plain"), "content-type: {tipo}");
    let texto = resposta.text().await.expect("texto das métricas");

    // Os medidores que o critério pede: RAM, filas, agentes carregados e latências.
    for nome in [
        "poph_memoria_processo_bytes",
        "poph_fila_turnos",
        "poph_fila_saida",
        "poph_agentes_carregados",
        "poph_latencia_etapa_segundos",
    ] {
        assert!(
            texto.contains(nome),
            "faltou `{nome}` no /metrics:\n{texto}"
        );
    }
    assert!(texto.contains("# TYPE poph_memoria_processo_bytes gauge"));
    assert!(texto.contains("# TYPE poph_latencia_etapa_segundos histogram"));
    // A RAM do processo é maior que zero (estamos vivos lendo isso).
    let linha_ram = texto
        .lines()
        .find(|linha| linha.starts_with("poph_memoria_processo_bytes "))
        .expect("linha da RAM");
    let ram: i64 = linha_ram
        .rsplit(' ')
        .next()
        .and_then(|valor| valor.parse().ok())
        .expect("valor da RAM");
    assert!(ram > 0, "RAM medida: {ram}");
}

// ───────────────────────────── Erros de rota ─────────────────────────────

#[tokio::test]
async fn rota_inexistente_e_metodo_errado_respondem_no_formato_de_erro() {
    let c = Cenario::novo("rota").await;

    let resposta = c
        .cliente
        .get(c.url("/admin/v1/nao-existe"))
        .send()
        .await
        .expect("rota inexistente");
    assert_eq!(resposta.status(), StatusCode::NOT_FOUND);
    // `WWW-Authenticate` só vai no 401.
    assert!(resposta.headers().get("www-authenticate").is_none());
    let corpo: Value = resposta.json().await.expect("JSON do 404");
    assert_eq!(codigo(&corpo), "not_found");
    assert!(corpo["message"].is_string(), "404 sem message: {corpo}");
    assert_eq!(corpo["details"]["path"], json!("/admin/v1/nao-existe"));

    let resposta = c
        .cliente
        .put(c.url("/admin/v1/orgs"))
        .send()
        .await
        .expect("método errado");
    assert_eq!(resposta.status(), StatusCode::METHOD_NOT_ALLOWED);
    let corpo: Value = resposta.json().await.expect("JSON do 405");
    assert_eq!(codigo(&corpo), "method_not_allowed");
}

// ───────────────────────────── OpenAPI ─────────────────────────────

#[tokio::test]
async fn openapi_e_valida_e_cobre_todas_as_rotas() {
    let c = Cenario::novo("openapi").await;
    let resposta = c
        .cliente
        .get(c.url("/admin/v1/openapi.json"))
        .send()
        .await
        .expect("openapi");
    assert_eq!(resposta.status(), StatusCode::OK);
    let texto = resposta.text().await.expect("texto");
    let documento: Value = serde_json::from_str(&texto).expect("JSON válido");

    // Validação de esquema: o `oas3` desserializa a especificação 3.1 inteira (campos
    // desconhecidos, versão, `$ref`, tipos), então um documento inválido falha aqui.
    oas3::from_json(&texto).expect("OpenAPI 3.1 válida");

    // Cobre todas as rotas publicadas pelo roteador.
    let esperadas: &[(&str, &str)] = &[
        ("/admin/v1/orgs", "get"),
        ("/admin/v1/orgs", "post"),
        ("/admin/v1/orgs/{org_id}", "get"),
        ("/admin/v1/orgs/{org_id}", "patch"),
        ("/admin/v1/orgs/{org_id}", "delete"),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions",
            "get",
        ),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions",
            "post",
        ),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",
            "get",
        ),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",
            "patch",
        ),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",
            "delete",
        ),
        ("/admin/v1/orgs/{org_id}/tokens", "get"),
        ("/admin/v1/orgs/{org_id}/tokens", "post"),
        ("/admin/v1/orgs/{org_id}/tokens/{token_id}", "get"),
        ("/admin/v1/orgs/{org_id}/tokens/{token_id}", "delete"),
        ("/admin/v1/orgs/{org_id}/categories", "get"),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations",
            "get",
        ),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations",
            "post",
        ),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",
            "get",
        ),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",
            "patch",
        ),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",
            "delete",
        ),
        ("/admin/v1/orgs/{org_id}/products", "post"),
        ("/admin/v1/orgs/{org_id}/products/{product_id}", "patch"),
        ("/admin/v1/orgs/{org_id}/products/{product_id}", "delete"),
        ("/admin/v1/orgs/{org_id}/products", "get"),
        ("/admin/v1/orgs/{org_id}/products/{product_id}", "get"),
        ("/admin/v1/orgs/{org_id}/categories", "post"),
        ("/admin/v1/orgs/{org_id}/categories/{category_id}", "get"),
        ("/admin/v1/orgs/{org_id}/categories/{category_id}", "patch"),
        ("/admin/v1/orgs/{org_id}/categories/{category_id}", "delete"),
        ("/admin/v1/orgs/{org_id}/departments", "get"),
        ("/admin/v1/orgs/{org_id}/departments", "post"),
        ("/admin/v1/orgs/{org_id}/departments/{department_id}", "get"),
        (
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
            "patch",
        ),
        (
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
            "delete",
        ),
        ("/admin/v1/orgs/{org_id}/agents/{agent_id}/files", "get"),
        (
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}",
            "get",
        ),
        ("/healthz", "get"),
        ("/metrics", "get"),
    ];
    for (caminho, metodo) in esperadas {
        let operacao = &documento["paths"][caminho][metodo];
        assert!(
            !operacao.is_null(),
            "OpenAPI sem {metodo} {caminho}: {}",
            documento["paths"]
        );
        assert!(
            operacao["operationId"].is_string(),
            "{metodo} {caminho} sem operationId"
        );
        assert!(
            operacao["responses"].is_object(),
            "{metodo} {caminho} sem responses"
        );
    }

    // Rota da API protegida declara o esquema de segurança; saúde e métricas não.
    assert!(
        documento["paths"]["/admin/v1/orgs"]["get"]["security"].is_array(),
        "rota protegida sem security"
    );
    assert!(
        documento["components"]["securitySchemes"]["bearer"].is_object(),
        "esquema bearer ausente"
    );

    // Todo `$ref` interno resolve em `components` (nenhuma referência quebrada).
    let mut refs = Vec::new();
    coletar_refs(&documento, &mut refs);
    assert!(refs.len() > 10, "poucas referências: {}", refs.len());
    for referencia in refs {
        let alvo = referencia.trim_start_matches("#/");
        let resolvido = alvo.split('/').fold(&documento, |atual, parte| {
            atual.get(parte).unwrap_or(&Value::Null)
        });
        assert!(
            !resolvido.is_null(),
            "`$ref` quebrada no OpenAPI: {referencia}"
        );
    }
}

fn coletar_refs(valor: &Value, achados: &mut Vec<String>) {
    match valor {
        Value::Object(mapa) => {
            for (chave, filho) in mapa {
                if chave == "$ref"
                    && let Some(texto) = filho.as_str()
                    && texto.starts_with("#/")
                {
                    achados.push(texto.to_owned());
                }
                coletar_refs(filho, achados);
            }
        }
        Value::Array(itens) => {
            for item in itens {
                coletar_refs(item, achados);
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn conversas_mensagens_turnos_isolam_org_e_mostram_dados() {
    let c = Cenario::novo("conversas-api").await;
    let org_a = c.criar_org("Org A", "org-a").await;
    let org_b = c.criar_org("Org B", "org-b").await;
    let a = org_a["id"].as_str().unwrap();
    let b = org_b["id"].as_str().unwrap();
    let token_a = c.criar_token(a, "org a").await;
    let token_b = c.criar_token(b, "org b").await;
    let org = b.parse().unwrap();
    let agente = c
        .store()
        .agentes()
        .criar(
            org,
            poph_store::NovoAgente {
                nome: "Agente".into(),
                slug: "agente".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let conversa = c
        .store()
        .conversas()
        .criar(
            org,
            poph_store::NovaConversa {
                agent_id: agente.id,
                integration_id: None,
                canal: "cli".into(),
                peer_externo: None,
                departamento_alvo: None,
            },
        )
        .unwrap();
    c.store()
        .mensagens()
        .gravar(
            org,
            poph_store::NovaMensagem {
                conversation_id: conversa.id,
                direcao: poph_store::Direcao::Entrada,
                papel: poph_store::Papel::Usuario,
                texto: "olá".into(),
                anexos_json: "[]".into(),
                id_externo: None,
                turn_id: None,
                origem: poph_store::Origem::Agente,
                autor_nome: None,
            },
        )
        .unwrap();
    c.store()
        .turnos()
        .iniciar(org, conversa.id, Some(agente.id))
        .unwrap();
    let raiz = format!("/admin/v1/orgs/{b}/conversations");
    let detalhe = format!("{raiz}/{}", conversa.id);
    for rota in [
        &raiz,
        &detalhe,
        &format!("{detalhe}/messages"),
        &format!("{detalhe}/turns"),
    ] {
        assert_eq!(
            c.pedir(Method::GET, rota, "", None, None).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            c.pedir(Method::GET, rota, &token_a, None, None).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            c.pedir(Method::GET, rota, &token_b, None, None).await.0,
            StatusCode::OK
        );
    }
    let (_, corpo) = c
        .pedir(
            Method::GET,
            &format!("{detalhe}/messages"),
            &token_b,
            None,
            None,
        )
        .await;
    assert_eq!(corpo["items"][0]["text"], "olá");
    let (_, corpo) = c
        .pedir(
            Method::GET,
            &format!("{detalhe}/turns"),
            &token_b,
            None,
            None,
        )
        .await;
    assert_eq!(corpo["items"][0]["state"], "running");
    let transferida = c
        .store()
        .conversas()
        .mudar_status(
            org,
            conversa.id,
            conversa.versao,
            poph_store::StatusConversa::Transferida,
            None,
        )
        .unwrap();
    let reabrir = format!("{detalhe}/reopen");
    assert_eq!(
        c.pedir(
            Method::POST,
            &reabrir,
            &token_a,
            None,
            Some(transferida.versao)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        c.pedir(Method::POST, &reabrir, &token_b, None, Some(1))
            .await
            .0,
        StatusCode::PRECONDITION_FAILED
    );
    let (status, corpo) = c
        .pedir(
            Method::POST,
            &reabrir,
            &token_b,
            None,
            Some(transferida.versao),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(corpo["status"], "open");
    let registros = c
        .store()
        .auditoria()
        .listar(org, poph_store::Pagina::default())
        .unwrap();
    let registro = registros
        .iter()
        .find(|r| r.acao == "reopen" && r.entidade_id.as_deref() == Some(&conversa.id.to_string()))
        .expect("reabertura auditada");
    assert_eq!(
        registro.ator_id.as_deref(),
        token_b
            .strip_prefix("poph_")
            .and_then(|s| s.split_once('_'))
            .map(|(id, _)| id)
    );
}

#[tokio::test]
async fn produtos_consultas_isolam_organizacoes() {
    let c = Cenario::novo("produtos-api").await;
    let a = c.criar_org("Org A", "org-a").await;
    let b = c.criar_org("Org B", "org-b").await;
    let org_a = a["id"].as_str().unwrap();
    let org_b = b["id"].as_str().unwrap();
    let token_a = c.criar_token(org_a, "org a").await;
    let categoria = c
        .store()
        .categorias()
        .criar(
            org_b.parse().unwrap(),
            poph_store::NovaCategoria {
                nome: "Serviços".into(),
                descricao: String::new(),
            },
        )
        .unwrap();
    let produto = c
        .store()
        .produtos()
        .criar(
            org_b.parse().unwrap(),
            poph_store::NovoProduto {
                categoria_id: Some(categoria.id),
                nome: "Visita".into(),
                preco_centavos: 12345,
                ..Default::default()
            },
        )
        .unwrap();
    for caminho in [
        format!("/admin/v1/orgs/{org_b}/products"),
        format!("/admin/v1/orgs/{org_b}/products/{}", produto.id),
    ] {
        assert_eq!(
            c.pedir(Method::GET, &caminho, "", None, None).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            c.pedir(Method::GET, &caminho, &token_a, None, None).await.0,
            StatusCode::FORBIDDEN
        );
    }
    let (status, valor) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{org_b}/products/{}", produto.id),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(valor["name"], "Visita");
    assert_eq!(valor["price_cents"], 12345);
    let (status, valor) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{org_b}/products"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(valor["items"][0]["id"], produto.id.to_string());
}

#[tokio::test]
async fn auditoria_de_escritas_identifica_ator_e_nao_registra_segredo() {
    let c = Cenario::novo("audit-escritas").await;
    let org = c.criar_org("Auditoria", "audit-escritas").await;
    let org_id = org["id"].as_str().unwrap();
    let token = c.criar_token(org_id, "org").await;
    let token_id = token
        .strip_prefix("poph_")
        .unwrap()
        .split_once('_')
        .unwrap()
        .0;
    let rota_org = format!("/admin/v1/orgs/{org_id}");
    assert_eq!(
        c.pedir(
            Method::PATCH,
            &rota_org,
            &token,
            Some(json!({"name":"Mudou"})),
            Some(1)
        )
        .await
        .0,
        StatusCode::OK
    );
    for (rota, entidade) in [("categories", "category"), ("departments", "department")] {
        let colecao = format!("{rota_org}/{rota}");
        let (status, criado) = c
            .pedir(
                Method::POST,
                &colecao,
                &token,
                Some(json!({"name":"Exemplo"})),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{criado}");
        let id = criado["id"].as_str().unwrap();
        let item = format!("{colecao}/{id}");
        assert_eq!(
            c.pedir(
                Method::PATCH,
                &item,
                &token,
                Some(json!({"name":"Novo"})),
                Some(1)
            )
            .await
            .0,
            StatusCode::OK
        );
        assert_eq!(
            c.pedir(Method::DELETE, &item, &token, None, None).await.0,
            StatusCode::NO_CONTENT
        );
        let registros = c
            .store()
            .auditoria()
            .listar(org_id.parse().unwrap(), poph_store::Pagina::default())
            .unwrap();
        for acao in ["create", "update", "delete"] {
            let linha = registros
                .iter()
                .find(|r| {
                    r.entidade == entidade && r.acao == acao && r.entidade_id.as_deref() == Some(id)
                })
                .expect("registro de auditoria");
            assert_eq!(linha.ator_tipo, "admin_token");
            assert_eq!(linha.ator_id.as_deref(), Some(token_id));
            assert!(!format!("{linha:?}").contains(&token));
        }
    }
    let registros = c
        .store()
        .auditoria()
        .listar(org_id.parse().unwrap(), poph_store::Pagina::default())
        .unwrap();
    assert!(registros.iter().any(|r| r.entidade == "org"
        && r.acao == "create"
        && r.ator_id.as_deref() == Some("global:teste")));
    assert!(registros.iter().any(|r| r.entidade == "org"
        && r.acao == "update"
        && r.ator_id.as_deref() == Some(token_id)));
    assert!(registros.iter().any(|r| r.entidade == "token"
        && r.acao == "create"
        && r.entidade_id.as_deref() == Some(token_id)
        && r.ator_id.as_deref() == Some("global:teste")));
    let rota_token = format!("{rota_org}/tokens/{token_id}");
    assert_eq!(
        c.pedir(Method::DELETE, &rota_token, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let registros = c
        .store()
        .auditoria()
        .listar(org_id.parse().unwrap(), poph_store::Pagina::default())
        .unwrap();
    assert!(registros.iter().any(|r| r.entidade == "token"
        && r.acao == "revoke"
        && r.entidade_id.as_deref() == Some(token_id)
        && r.ator_id.as_deref() == Some("global:teste")));
}

#[tokio::test]
async fn toda_escrita_da_openapi_precisa_de_cobertura_de_auditoria() {
    use std::collections::BTreeSet;
    let c = Cenario::novo("audit-openapi").await;
    let (status, doc) = c.pedir(Method::GET, "/openapi.json", "", None, None).await;
    assert_eq!(status, StatusCode::OK);
    let esperado: BTreeSet<_> = [
        ("post", "/admin/v1/orgs"),
        ("patch", "/admin/v1/orgs/{org_id}"),
        ("delete", "/admin/v1/orgs/{org_id}"),
        ("post", "/admin/v1/orgs/{org_id}/agents"),
        ("patch", "/admin/v1/orgs/{org_id}/agents/{agent_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/agents/{agent_id}"),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions",
        ),
        (
            "patch",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/{group_id}",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/{group_id}",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/reindex",
        ),
        ("post", "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat"),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/messages",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/reopen",
        ),
        ("post", "/admin/v1/orgs/{org_id}/knowledge"),
        ("patch", "/admin/v1/orgs/{org_id}/knowledge/{group_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/knowledge/{group_id}"),
        ("post", "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items"),
        (
            "patch",
            "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}",
        ),
        ("post", "/admin/v1/orgs/{org_id}/tokens"),
        ("delete", "/admin/v1/orgs/{org_id}/tokens/{token_id}"),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations",
        ),
        (
            "patch",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/qrcode",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/disconnect",
        ),
        ("post", "/admin/v1/orgs/{org_id}/products"),
        ("patch", "/admin/v1/orgs/{org_id}/products/{product_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/products/{product_id}"),
        ("post", "/admin/v1/orgs/{org_id}/agents/{agent_id}/files"),
        ("post", "/admin/v1/orgs/{org_id}/categories"),
        ("patch", "/admin/v1/orgs/{org_id}/categories/{category_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/categories/{category_id}"),
        ("post", "/admin/v1/orgs/{org_id}/departments"),
        (
            "patch",
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/reopen",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/close",
        ),
        // HR-48e-1: a importação grava na org (a exportação é leitura).
        ("post", "/admin/v1/orgs/{org_id}/backup/import"),
        // MX-22: ligar/desligar o módulo Matrix e os espaços por departamento.
        ("put", "/admin/v1/orgs/{org_id}/matrix"),
        ("put", "/admin/v1/orgs/{org_id}/matrix/spaces"),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/matrix/spaces/{department_id}",
        ),
    ]
    .into_iter()
    .map(|(m, p)| (m.to_owned(), p.to_owned()))
    .collect();
    // Escritas de configuração da **instância** (não de org): não gravam dado de cliente nem
    // entram no `audit_log` por org. Ficam fora desta cobertura, mas continuam exigidas abaixo —
    // rota nova de qualquer tipo reprova aqui.
    let fora_da_auditoria_por_org =
        BTreeSet::from([("post".to_owned(), "/admin/v1/cluster/reload".to_owned())]);
    let mut publicadas = BTreeSet::new();
    for (caminho, metodos) in doc["paths"].as_object().unwrap() {
        for metodo in ["post", "put", "patch", "delete"] {
            if metodos.get(metodo).is_some()
                && !fora_da_auditoria_por_org.contains(&(metodo.to_owned(), caminho.clone()))
            {
                publicadas.insert((metodo.to_owned(), caminho.clone()));
            }
        }
    }
    assert_eq!(
        publicadas, esperado,
        "escrita nova exige teste de auditoria e atualização da cobertura"
    );
}

#[tokio::test]
async fn exclusao_de_org_com_e_sem_force_fica_na_auditoria_global() {
    let c = Cenario::novo("audit-exclusao-org").await;
    for (slug, force) in [("audit-org-vazia", false), ("audit-org-forcada", true)] {
        let org = c.criar_org("Org", slug).await;
        let id = org["id"].as_str().unwrap();
        if force {
            c.store()
                .agentes()
                .criar(
                    id.parse().unwrap(),
                    poph_store::NovoAgente {
                        nome: "Agente".into(),
                        slug: "agente".into(),
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        let url = format!("/admin/v1/orgs/{id}?force={force}");
        assert_eq!(
            c.pedir(Method::DELETE, &url, TOKEN_GLOBAL, None, None)
                .await
                .0,
            StatusCode::NO_CONTENT
        );
        let registros = c
            .store()
            .auditoria()
            .listar_globais(poph_store::Pagina::default())
            .unwrap();
        let acao = if force { "delete_force" } else { "delete" };
        let registro = registros
            .iter()
            .find(|r| r.entidade == "org" && r.acao == acao && r.entidade_id.as_deref() == Some(id))
            .expect("auditoria de exclusão");
        assert_eq!(registro.ator_tipo, "admin_token");
        assert_eq!(registro.ator_id.as_deref(), Some("global:teste"));
        assert!(!format!("{registro:?}").contains(TOKEN_GLOBAL));
    }
}

#[tokio::test]
async fn agentes_crud_escopo_versao_e_auditoria() {
    let c = Cenario::novo("agentes-crud").await;
    let a = c.criar_org("Org A", "agent-org-a").await;
    let b = c.criar_org("Org B", "agent-org-b").await;
    let oa = a["id"].as_str().unwrap();
    let ob = b["id"].as_str().unwrap();
    let ta = c.criar_token(oa, "a").await;
    let tb = c.criar_token(ob, "b").await;
    let token_id = tb.strip_prefix("poph_").unwrap().split_once('_').unwrap().0;
    let col = format!("/admin/v1/orgs/{ob}/agents");
    let (status, criado) = c
        .pedir(
            Method::POST,
            &col,
            &tb,
            Some(json!({"name":"Agente Teste", "slug":"agente-teste"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{criado}");
    let id = criado["id"].as_str().unwrap();
    let item = format!("{col}/{id}");
    for (metodo, path, corpo, versao) in [
        (Method::GET, col.as_str(), None, None),
        (
            Method::POST,
            col.as_str(),
            Some(json!({"name":"Intruso"})),
            None,
        ),
        (Method::GET, item.as_str(), None, None),
        (
            Method::PATCH,
            item.as_str(),
            Some(json!({"name":"Intruso"})),
            Some(1),
        ),
        (Method::DELETE, item.as_str(), None, None),
    ] {
        let (status, corpo) = c.pedir(metodo, path, &ta, corpo, versao).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{corpo}");
    }
    let (status, conflito) = c
        .pedir(
            Method::PATCH,
            &item,
            &tb,
            Some(json!({"name":"Novo"})),
            Some(20),
        )
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(conflito["code"], "version_conflict");
    assert_eq!(
        c.pedir(
            Method::PATCH,
            &item,
            &tb,
            Some(json!({"name":"Novo"})),
            None
        )
        .await
        .0,
        StatusCode::PRECONDITION_REQUIRED
    );
    let (status, atualizado) = c
        .pedir(
            Method::PATCH,
            &item,
            &tb,
            Some(json!({"name":"Novo"})),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{atualizado}");
    assert_eq!(atualizado["name"], "Novo");
    let (status, mostrado) = c.pedir(Method::GET, &item, &tb, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mostrado["name"], "Novo");
    assert_eq!(mostrado["product_category_ids"], json!([]));
    assert_eq!(mostrado["instructions"], Value::Null);
    assert_eq!(
        c.pedir(Method::GET, &col, &tb, None, None).await.1["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        c.pedir(Method::DELETE, &item, &tb, None, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        c.pedir(Method::GET, &item, &tb, None, None).await.0,
        StatusCode::NOT_FOUND
    );
    let registros = c
        .store()
        .auditoria()
        .listar(ob.parse().unwrap(), poph_store::Pagina::default())
        .unwrap();
    for acao in ["create", "update", "delete"] {
        let registro = registros
            .iter()
            .find(|r| {
                r.entidade == "agent" && r.acao == acao && r.entidade_id.as_deref() == Some(id)
            })
            .unwrap();
        assert_eq!(registro.ator_tipo, "admin_token");
        assert_eq!(registro.ator_id.as_deref(), Some(token_id));
        assert!(!format!("{registro:?}").contains(&tb));
    }
}

#[tokio::test]
async fn agentes_paridade_com_cli() {
    let c = Cenario::novo("agente-paridade").await;
    let org = c.criar_org("Paridade", "org-paridade-api").await;
    let org_id = org["id"].as_str().unwrap();
    let col = format!("/admin/v1/orgs/{org_id}/agents");
    let build = std::process::Command::new("cargo")
        .args(["build", "--locked", "-p", "poph"])
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .output()
        .expect("compila CLI para paridade");
    assert!(
        build.status.success(),
        "build CLI: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let cli = |args: &[&str]| -> Value {
        let exe = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/poph");
        let output = std::process::Command::new(exe)
            .args(["--org", org_id, "--offline", "agent"])
            .args(args)
            .env("POPH_DADOS_DIRETORIO", c.pasta.join("dados"))
            .output()
            .expect("executa CLI poph (construída junto com a suíte)");
        assert!(
            output.status.success(),
            "CLI: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let cli_criado = cli(&[
        "create",
        "--name",
        "Bia Atendente",
        "--slug",
        "bia-cli",
        "--json",
    ]);
    let (status, api_criado) = c
        .pedir(
            Method::POST,
            &col,
            TOKEN_GLOBAL,
            Some(json!({"name":"Bia Atendente", "slug":"bia-api"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{api_criado}");
    let cli_id = cli_criado["agent"]["id"].as_str().unwrap();
    let api_id = api_criado["id"].as_str().unwrap();
    let comparar = |mut a: Value, mut b: Value| {
        for campo in ["id", "slug", "created_at", "updated_at"] {
            a.as_object_mut().unwrap().remove(campo);
            b.as_object_mut().unwrap().remove(campo);
        }
        assert_eq!(a, b);
    };
    comparar(cli_criado["agent"].clone(), api_criado.clone());
    let cli_alterado = cli(&[
        "update",
        cli_id,
        "--if-match",
        "1",
        "--name",
        "Bia Nova",
        "--json",
    ]);
    let item = format!("{col}/{api_id}");
    let (status, api_alterado) = c
        .pedir(
            Method::PATCH,
            &item,
            TOKEN_GLOBAL,
            Some(json!({"name":"Bia Nova"})),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{api_alterado}");
    comparar(cli_alterado["agent"].clone(), api_alterado);
    cli(&["delete", cli_id, "--json"]);
    assert_eq!(
        c.pedir(Method::DELETE, &item, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        c.store()
            .agentes()
            .obter(org_id.parse().unwrap(), cli_id.parse().unwrap())
            .is_err()
    );
    assert!(
        c.store()
            .agentes()
            .obter(org_id.parse().unwrap(), api_id.parse().unwrap())
            .is_err()
    );
}

#[tokio::test]
async fn instrucoes_api_paridade_isolamento_e_auditoria() {
    let c = Cenario::novo("instrucoes").await;
    let a = c.criar_org("Org A", "ins-org-a").await;
    let b = c.criar_org("Org B", "ins-org-b").await;
    let oa = a["id"].as_str().unwrap();
    let ob = b["id"].as_str().unwrap();
    let ta = c.criar_token(oa, "a").await;
    let tb = c.criar_token(ob, "b").await;
    let token_id = tb.split('_').nth(1).unwrap();
    let org = ob.parse().unwrap();
    let novo = |slug: &str| poph_store::NovoAgente {
        nome: "Agente teste".into(),
        slug: slug.into(),
        ..Default::default()
    };
    let cli_id = c
        .store()
        .agentes()
        .criar(org, novo("instrucoes-cli"))
        .unwrap()
        .id;
    let api_id = c
        .store()
        .agentes()
        .criar(org, novo("instrucoes-api"))
        .unwrap()
        .id;
    let col = format!("/admin/v1/orgs/{ob}/agents/{api_id}/instructions");
    let item = format!("{col}/current");
    for (metodo, path, corpo, versao) in [
        (Method::GET, col.as_str(), None, None),
        (
            Method::POST,
            col.as_str(),
            Some(json!({"markdown":"x"})),
            None,
        ),
        (Method::GET, item.as_str(), None, None),
        (
            Method::PATCH,
            item.as_str(),
            Some(json!({"markdown":"x"})),
            Some(1),
        ),
        (Method::DELETE, item.as_str(), None, None),
    ] {
        assert_eq!(
            c.pedir(metodo, path, &ta, corpo, versao).await.0,
            StatusCode::FORBIDDEN
        );
    }
    let build = std::process::Command::new("cargo")
        .args(["build", "--locked", "-p", "poph"])
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let file = c.pasta.join("instructions.txt");
    fs::write(&file, "Primeira instrução").unwrap();
    let exe = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/poph");
    let cli = |args: &[&str]| -> Value {
        let output = std::process::Command::new(&exe)
            .args(["--org", ob, "--offline", "instructions"])
            .args(args)
            .env("POPH_DADOS_DIRETORIO", c.pasta.join("dados"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let id = cli_id.to_string();
    let file_path = file.to_str().unwrap();
    let comparar = |mut x: Value, mut y: Value| {
        for campo in ["agent_id", "created_at", "updated_at"] {
            x.as_object_mut().unwrap().remove(campo);
            y.as_object_mut().unwrap().remove(campo);
        }
        assert_eq!(x, y);
    };
    let criado = cli(&["create", &id, "--markdown-file", file_path, "--json"]);
    let (status, api) = c
        .pedir(
            Method::POST,
            &col,
            &tb,
            Some(json!({"markdown":"Primeira instrução"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{api}");
    comparar(criado["instructions"].clone(), api);
    assert_eq!(
        c.pedir(Method::GET, &col, &tb, None, None).await.1["items"][0]["markdown"],
        "Primeira instrução"
    );
    assert_eq!(
        c.pedir(Method::GET, &item, &tb, None, None).await.1["structured"],
        json!({})
    );
    let (status, erro) = c
        .pedir(
            Method::PATCH,
            &item,
            &tb,
            Some(json!({"markdown":"Alterada"})),
            Some(9),
        )
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(erro["code"], "version_conflict");
    assert_eq!(
        c.pedir(
            Method::PATCH,
            &item,
            &tb,
            Some(json!({"markdown":"Alterada"})),
            None
        )
        .await
        .0,
        StatusCode::PRECONDITION_REQUIRED
    );
    fs::write(&file, "Alterada").unwrap();
    let alterado = cli(&[
        "update",
        &id,
        "--if-match",
        "1",
        "--markdown-file",
        file_path,
        "--json",
    ]);
    let (status, api) = c
        .pedir(
            Method::PATCH,
            &item,
            &tb,
            Some(json!({"markdown":"Alterada"})),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{api}");
    comparar(alterado["instructions"].clone(), api);
    cli(&["delete", &id, "--json"]);
    assert_eq!(
        c.pedir(Method::DELETE, &item, &tb, None, None).await.0,
        StatusCode::OK
    );
    assert!(c.store().instrucoes().obter(org, cli_id).is_err());
    assert!(c.store().instrucoes().obter(org, api_id).is_err());
    let (status, erro) = c
        .pedir(
            Method::PATCH,
            &item,
            &tb,
            Some(json!({"markdown":"Antiga"})),
            Some(2),
        )
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(erro["details"]["current_version"], 0);
    assert!(c.store().instrucoes().obter(org, api_id).is_err());
    fs::write(&file, "{}").unwrap();
    let cli_estruturado = cli(&["create", &id, "--file", file_path, "--json"]);
    let (status, api_estruturado) = c
        .pedir(
            Method::POST,
            &col,
            &tb,
            Some(json!({"structured":{}})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{api_estruturado}");
    comparar(
        cli_estruturado["instructions"].clone(),
        api_estruturado.clone(),
    );
    assert_eq!(api_estruturado["derived_from_json"], true);
    assert_eq!(
        c.pedir(Method::GET, &item, &tb, None, None).await.1["markdown"],
        api_estruturado["markdown"]
    );
    let (status, erro) = c
        .pedir(
            Method::PATCH,
            &item,
            &tb,
            Some(json!({"structured":{"tools":[{"name":"inexistente", "enabled":true}]}})),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{erro}");
    assert_eq!(erro["code"], "validation_error");
    let (status, erro) = c.pedir(Method::PUT, &item, &tb, None, None).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(erro["code"], "method_not_allowed");
    let registros = c
        .store()
        .auditoria()
        .listar(org, poph_store::Pagina::default())
        .unwrap();
    for acao in ["create", "update", "delete"] {
        let r = registros
            .iter()
            .find(|r| {
                r.entidade == "agent_instructions"
                    && r.acao == acao
                    && r.entidade_id.as_deref() == Some(&api_id.to_string())
            })
            .unwrap();
        assert_eq!(r.ator_tipo, "admin_token");
        assert_eq!(r.ator_id.as_deref(), Some(token_id));
        assert!(!format!("{r:?}").contains(&tb));
    }
}

#[tokio::test]
async fn conhecimento_crud_conflito_e_isolamento() {
    let c = Cenario::novo("knowledge").await;
    let a = c.criar_org("Conhecimento A", "knowledge-a").await;
    let b = c.criar_org("Conhecimento B", "knowledge-b").await;
    let aid = a["id"].as_str().unwrap();
    let bid = b["id"].as_str().unwrap();
    let token = c.criar_token(aid, "teste").await;
    let colecao = format!("/admin/v1/orgs/{aid}/knowledge");
    let (status, g) = c
        .pedir(
            Method::POST,
            &colecao,
            &token,
            Some(json!({"name":"Guia"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{g}");
    let grupo = format!("{colecao}/{}", g["id"].as_str().unwrap());
    let itens = format!("{grupo}/items");
    for (metodo, rota) in [
        (Method::GET, format!("{colecao}?limit=erro")),
        (Method::GET, format!("{itens}?limit=erro")),
        (Method::DELETE, format!("{grupo}?force=erro")),
    ] {
        let (status, corpo) = c.pedir(metodo, &rota, &token, None, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(corpo["code"], "invalid_request");
        assert!(corpo.get("message").is_some() && corpo.get("details").is_some());
    }
    let (status, i) = c
        .pedir(
            Method::POST,
            &itens,
            &token,
            Some(json!({"title":"Pergunta","content":"Resposta"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{i}");
    let item = format!("{itens}/{}", i["id"].as_str().unwrap());
    for caminho in [&grupo, &item] {
        let (status, _) = c
            .pedir(Method::PATCH, caminho, &token, Some(json!({})), Some(99))
            .await;
        assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    }
    for (caminho, metodos) in [
        (&colecao, vec![Method::GET, Method::POST]),
        (&grupo, vec![Method::GET, Method::PATCH, Method::DELETE]),
        (&itens, vec![Method::GET, Method::POST]),
        (&item, vec![Method::GET, Method::PATCH, Method::DELETE]),
    ] {
        for metodo in metodos {
            let (status, _) = c
                .pedir(
                    metodo,
                    &caminho.replace(aid, bid),
                    &token,
                    Some(json!({})),
                    Some(1),
                )
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN);
        }
    }
    assert_eq!(
        c.pedir(
            Method::PATCH,
            &grupo,
            &token,
            Some(json!({"description":"mudou"})),
            Some(1)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        c.pedir(
            Method::PATCH,
            &item,
            &token,
            Some(json!({"content":"mudou"})),
            Some(1)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        c.pedir(Method::DELETE, &item, &token, None, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        c.pedir(
            Method::DELETE,
            &format!("{grupo}?force=true"),
            &token,
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let token_id = token
        .strip_prefix("poph_")
        .unwrap()
        .split_once('_')
        .unwrap()
        .0;
    let registros = c
        .store()
        .auditoria()
        .listar(aid.parse().unwrap(), poph_store::Pagina::default())
        .unwrap();
    for (entidade, id) in [
        ("knowledge_group", g["id"].as_str().unwrap()),
        ("knowledge_item", i["id"].as_str().unwrap()),
    ] {
        for acao in ["create", "update", "delete"] {
            let linha = registros
                .iter()
                .find(|r| {
                    r.entidade == entidade && r.acao == acao && r.entidade_id.as_deref() == Some(id)
                })
                .unwrap();
            assert_eq!(linha.ator_tipo, "admin_token");
            assert_eq!(linha.ator_id.as_deref(), Some(token_id));
            assert!(!format!("{linha:?}").contains(&token));
        }
    }
}

#[tokio::test]
async fn vinculo_base_autorizacao_e_modelo_ausente() {
    let c = Cenario::novo("knowledge-link").await;
    let a = c.criar_org("Base A", "kb-link-a").await;
    let b = c.criar_org("Base B", "kb-link-b").await;
    let aid = a["id"].as_str().unwrap();
    let bid = b["id"].as_str().unwrap();
    let token = c.criar_token(aid, "kb").await;
    let agente = c
        .store()
        .agentes()
        .criar(
            aid.parse().unwrap(),
            poph_store::NovoAgente {
                nome: "Teste".into(),
                slug: "teste".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let grupo = c
        .store()
        .conhecimento()
        .criar_grupo(
            aid.parse().unwrap(),
            poph_store::NovoGrupoConhecimento {
                nome: "Grupo".into(),
                descricao: String::new(),
            },
        )
        .unwrap();
    let caminho = format!(
        "/admin/v1/orgs/{aid}/agents/{}/knowledge/{}",
        agente.id, grupo.id
    );
    let reindex = format!(
        "/admin/v1/orgs/{aid}/agents/{}/knowledge/reindex",
        agente.id
    );
    for metodo in [Method::POST, Method::DELETE] {
        assert_eq!(
            c.pedir(
                metodo.clone(),
                &caminho.replace(aid, bid),
                &token,
                None,
                None
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        c.pedir(Method::POST, &reindex.replace(aid, bid), &token, None, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        c.pedir(Method::POST, &caminho, &token, None, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        c.store()
            .conhecimento()
            .grupos_do_agente(aid.parse().unwrap(), agente.id)
            .unwrap()
            .len(),
        1
    );
    let (status, resposta) = c.pedir(Method::POST, &reindex, &token, None, None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{resposta}");
    assert_eq!(resposta["code"], "embedding_unavailable");
    assert_eq!(
        c.pedir(Method::DELETE, &caminho, &token, None, None)
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        c.store()
            .conhecimento()
            .grupos_do_agente(aid.parse().unwrap(), agente.id)
            .unwrap()
            .is_empty()
    );
    let token_id = token
        .strip_prefix("poph_")
        .unwrap()
        .split_once('_')
        .unwrap()
        .0;
    let registros = c
        .store()
        .auditoria()
        .listar(aid.parse().unwrap(), poph_store::Pagina::default())
        .unwrap();
    let group_id = grupo.id.to_string();
    for acao in ["link", "unlink"] {
        let linha = registros
            .iter()
            .find(|r| {
                r.entidade == "knowledge_group"
                    && r.acao == acao
                    && r.entidade_id.as_deref() == Some(group_id.as_str())
            })
            .unwrap();
        assert_eq!(linha.ator_tipo, "admin_token");
        assert_eq!(linha.ator_id.as_deref(), Some(token_id));
        assert!(!format!("{linha:?}").contains(&token));
    }
}

#[tokio::test]
#[ignore = "exige modelo real instalado em data/modelos"]
async fn reindex_com_modelo_real_torna_item_pesquisavel_e_preserva_inalterado() {
    let c = Cenario::novo("knowledge-real-model").await;
    let org = c.criar_org("Base real", "base-real").await;
    let oid = org["id"].as_str().unwrap();
    let org_id = oid.parse().unwrap();
    let token = c.criar_token(oid, "reindex").await;
    let agente = c
        .store()
        .agentes()
        .criar(
            org_id,
            poph_store::NovoAgente {
                nome: "Teste".into(),
                slug: "teste".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let grupo = c
        .store()
        .conhecimento()
        .criar_grupo(
            org_id,
            poph_store::NovoGrupoConhecimento {
                nome: "Guia".into(),
                descricao: String::new(),
            },
        )
        .unwrap();
    c.store()
        .conhecimento()
        .vincular_agente(org_id, agente.id, grupo.id)
        .unwrap();
    let item = c
        .store()
        .conhecimento()
        .criar_item(
            org_id,
            poph_store::NovoItemConhecimento {
                grupo_id: grupo.id,
                titulo: "Prazo de entrega".into(),
                conteudo: "Entregamos todos os pedidos em três dias úteis.".into(),
                tags: vec![],
                posicao: 0,
            },
        )
        .unwrap();
    let rota = format!(
        "/admin/v1/orgs/{oid}/agents/{}/knowledge/reindex",
        agente.id
    );
    let (status, corpo) = c.pedir(Method::POST, &rota, &token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["indexed"]["itens_novos"], 1);
    let (status, corpo) = c.pedir(Method::POST, &rota, &token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["indexed"]["itens_inalterados"], 1);
    assert_eq!(corpo["indexed"]["embeddings_gerados"], 0);
    let cfg = poph_rag::ConfigModelo {
        diretorio: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/modelos"),
        ..Default::default()
    };
    let modelo = poph_rag::EmbedderOrt::abrir(&cfg, None).unwrap();
    let (indice, _) = poph_rag::Indice::abrir(
        c.store().caminho_rag(),
        std::sync::Arc::new(modelo),
        Default::default(),
    )
    .unwrap();
    let achados = indice
        .buscar_hibrida(
            org_id,
            agente.id,
            "qual o prazo de entrega?",
            &Default::default(),
        )
        .await
        .unwrap();
    assert!(achados.iter().any(|a| a.trecho.item == item.id));
    let registros = c
        .store()
        .auditoria()
        .listar(org_id, poph_store::Pagina::default())
        .unwrap();
    let token_id = token
        .strip_prefix("poph_")
        .unwrap()
        .split_once('_')
        .unwrap()
        .0;
    let linha = registros.iter().find(|r| r.acao == "reindex").unwrap();
    assert_eq!(linha.ator_id.as_deref(), Some(token_id));
    assert!(!format!("{linha:?}").contains(&token));
    let vinculo = format!(
        "/admin/v1/orgs/{oid}/agents/{}/knowledge/{}",
        agente.id, grupo.id
    );
    let (status, resposta) = c.pedir(Method::DELETE, &vinculo, &token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{resposta}");
    assert!(resposta["index_warning"].is_null());
    let achados = indice
        .buscar_hibrida(
            org_id,
            agente.id,
            "qual o prazo de entrega?",
            &Default::default(),
        )
        .await
        .unwrap();
    assert!(!achados.iter().any(|a| a.trecho.item == item.id));
}

#[tokio::test]
async fn conhecimento_paridade_cli_criacao_edicao_exclusao() {
    let c = Cenario::novo("kb-paridade").await;
    let org = c.criar_org("Base paridade", "kb-paridade").await;
    let oid = org["id"].as_str().unwrap();
    let cli = |args: &[&str]| -> Value {
        let exe = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/poph");
        let output = std::process::Command::new(exe)
            .args(["--org", oid, "--offline", "kb"])
            .args(args)
            .env("POPH_DADOS_DIRETORIO", c.pasta.join("dados"))
            .output()
            .expect("executa CLI");
        assert!(
            output.status.success(),
            "CLI: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let group_cli = cli(&[
        "group",
        "create",
        "--name",
        "CLI",
        "--description",
        "descrição",
        "--json",
    ]);
    let coll = format!("/admin/v1/orgs/{oid}/knowledge");
    let (status, group_api) = c
        .pedir(
            Method::POST,
            &coll,
            TOKEN_GLOBAL,
            Some(json!({"name":"API","description":"descrição"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let cid = group_cli["group"]["id"].as_str().unwrap();
    let aid = group_api["id"].as_str().unwrap();
    assert_eq!(group_cli["group"]["description"], group_api["description"]);
    let group_cli = cli(&[
        "group",
        "update",
        cid,
        "--if-match",
        "1",
        "--description",
        "nova descrição",
        "--json",
    ]);
    let group_path = format!("{coll}/{aid}");
    let (status, group_api) = c
        .pedir(
            Method::PATCH,
            &group_path,
            TOKEN_GLOBAL,
            Some(json!({"description":"nova descrição"})),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    for key in ["description", "status", "version"] {
        assert_eq!(group_cli["group"][key], group_api[key]);
    }
    let agente_cli = c
        .store()
        .agentes()
        .criar(
            oid.parse().unwrap(),
            poph_store::NovoAgente {
                nome: "CLI".into(),
                slug: "ag-cli".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let agente_api = c
        .store()
        .agentes()
        .criar(
            oid.parse().unwrap(),
            poph_store::NovoAgente {
                nome: "API".into(),
                slug: "ag-api".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let agente_cli_id = agente_cli.id.to_string();
    cli(&["link", cid, "--agent", &agente_cli_id, "--json"]);
    let vinculo = format!(
        "/admin/v1/orgs/{oid}/agents/{}/knowledge/{aid}",
        agente_api.id
    );
    assert_eq!(
        c.pedir(Method::POST, &vinculo, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        c.store()
            .conhecimento()
            .grupos_do_agente(oid.parse().unwrap(), agente_cli.id)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        c.store()
            .conhecimento()
            .grupos_do_agente(oid.parse().unwrap(), agente_api.id)
            .unwrap()
            .len(),
        1
    );
    cli(&["unlink", cid, "--agent", &agente_cli_id, "--json"]);
    assert_eq!(
        c.pedir(Method::DELETE, &vinculo, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        c.store()
            .conhecimento()
            .grupos_do_agente(oid.parse().unwrap(), agente_cli.id)
            .unwrap()
            .is_empty()
    );
    assert!(
        c.store()
            .conhecimento()
            .grupos_do_agente(oid.parse().unwrap(), agente_api.id)
            .unwrap()
            .is_empty()
    );
    let item_cli = cli(&[
        "item",
        "create",
        "--group",
        cid,
        "--title",
        "Título",
        "--content",
        "conteúdo",
        "--json",
    ]);
    let item_coll = format!("{group_path}/items");
    let (status, item_api) = c
        .pedir(
            Method::POST,
            &item_coll,
            TOKEN_GLOBAL,
            Some(json!({"title":"Título","content":"conteúdo"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let ci = item_cli["item"]["id"].as_str().unwrap();
    let ai = item_api["id"].as_str().unwrap();
    for key in ["title", "content", "tags", "position", "status", "version"] {
        assert_eq!(item_cli["item"][key], item_api[key], "{key}");
    }
    let item_cli = cli(&[
        "item",
        "update",
        ci,
        "--if-match",
        "1",
        "--content",
        "alterado",
        "--json",
    ]);
    let item_path = format!("{item_coll}/{ai}");
    let (status, item_api) = c
        .pedir(
            Method::PATCH,
            &item_path,
            TOKEN_GLOBAL,
            Some(json!({"content":"alterado"})),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    for key in ["title", "content", "tags", "position", "status", "version"] {
        assert_eq!(item_cli["item"][key], item_api[key], "{key}");
    }
    cli(&["item", "delete", ci, "--json"]);
    assert_eq!(
        c.pedir(Method::DELETE, &item_path, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        c.store()
            .conhecimento()
            .obter_item(oid.parse().unwrap(), ci.parse().unwrap())
            .is_err()
    );
    assert!(
        c.store()
            .conhecimento()
            .obter_item(oid.parse().unwrap(), ai.parse().unwrap())
            .is_err()
    );
    assert_eq!(
        c.pedir(Method::DELETE, &group_path, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::CONFLICT
    );
    cli(&["group", "delete", cid, "--force", "--json"]);
    assert_eq!(
        c.pedir(
            Method::DELETE,
            &format!("{group_path}?force=true"),
            TOKEN_GLOBAL,
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(
        c.store()
            .conhecimento()
            .obter_grupo(oid.parse().unwrap(), cid.parse().unwrap())
            .is_err()
    );
    assert!(
        c.store()
            .conhecimento()
            .obter_grupo(oid.parse().unwrap(), aid.parse().unwrap())
            .is_err()
    );
}

#[tokio::test]
async fn produtos_escritas_controle_de_versao_e_isolamento() {
    let c = Cenario::novo("produto-escritas").await;
    let a = c.criar_org("Org A", "org-a").await;
    let b = c.criar_org("Org B", "org-b").await;
    let org_a = a["id"].as_str().unwrap();
    let org_b = b["id"].as_str().unwrap();
    let token = c.criar_token(org_a, "org a").await;
    let cat = c
        .store()
        .categorias()
        .criar(
            org_b.parse().unwrap(),
            poph_store::NovaCategoria {
                nome: "Serviços".into(),
                descricao: String::new(),
            },
        )
        .unwrap();
    let colecao = format!("/admin/v1/orgs/{org_b}/products");
    let dados = json!({"category_id":cat.id.to_string(),"name":"Visita", "price_cents":12345});
    assert_eq!(
        c.pedir(Method::POST, &colecao, &token, Some(dados.clone()), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, p) = c
        .pedir(Method::POST, &colecao, TOKEN_GLOBAL, Some(dados), None)
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(p["price_cents"], 12345);
    let item = format!("{colecao}/{}", p["id"].as_str().unwrap());
    for metodo in [Method::PATCH, Method::DELETE] {
        assert_eq!(
            c.pedir(
                metodo,
                &item,
                &token,
                Some(json!({"name":"Outro"})),
                Some(1)
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        c.pedir(
            Method::PATCH,
            &item,
            TOKEN_GLOBAL,
            Some(json!({"price_cents":100})),
            Some(42)
        )
        .await
        .0,
        StatusCode::PRECONDITION_FAILED
    );
    let (status, alterado) = c
        .pedir(
            Method::PATCH,
            &item,
            TOKEN_GLOBAL,
            Some(json!({"price_cents":100})),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(alterado["price_cents"], 100);
    let (status, inativo) = c
        .pedir(
            Method::PATCH,
            &item,
            TOKEN_GLOBAL,
            Some(json!({"status":"inactive"})),
            Some(2),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{inativo}");
    let (_, ativos) = c
        .pedir(Method::GET, &colecao, TOKEN_GLOBAL, None, None)
        .await;
    assert!(ativos["items"].as_array().unwrap().is_empty());
    let (_, todos) = c
        .pedir(
            Method::GET,
            &format!("{colecao}?all=true"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(todos["items"].as_array().unwrap().len(), 1);
    let registros = c
        .store()
        .auditoria()
        .listar(org_b.parse().unwrap(), poph_store::Pagina::default())
        .unwrap();
    for acao in ["create", "update"] {
        let linha = registros
            .iter()
            .find(|r| {
                r.entidade == "product"
                    && r.acao == acao
                    && r.entidade_id.as_deref() == p["id"].as_str()
            })
            .unwrap();
        assert_eq!(linha.ator_tipo, "admin_token");
        assert_eq!(linha.ator_id.as_deref(), Some("global:teste"));
        assert!(!format!("{linha:?}").contains(TOKEN_GLOBAL));
    }
    assert_eq!(
        c.pedir(Method::DELETE, &item, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        c.pedir(Method::GET, &item, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let registros = c
        .store()
        .auditoria()
        .listar(org_b.parse().unwrap(), poph_store::Pagina::default())
        .unwrap();
    let linha = registros
        .iter()
        .find(|r| {
            r.entidade == "product"
                && r.acao == "delete"
                && r.entidade_id.as_deref() == p["id"].as_str()
        })
        .unwrap();
    assert_eq!(linha.ator_id.as_deref(), Some("global:teste"));
    assert!(!format!("{linha:?}").contains(TOKEN_GLOBAL));
}

#[tokio::test]
async fn integracoes_ciclo_isolamento_e_segredos() {
    #[derive(Clone)]
    struct Captura(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Captura {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let saida = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let buffer = saida.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || Captura(buffer.clone()))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    tracing::warn!("captura de log ativa");
    let c = Cenario::novo("integracoes").await;
    let a = c.criar_org("Org A", "org-a").await;
    let b = c.criar_org("Org B", "org-b").await;
    let oa = a["id"].as_str().unwrap();
    let ob = b["id"].as_str().unwrap();
    let token_a = c.criar_token(oa, "a").await;
    let token_b = c.criar_token(ob, "b").await;
    let agente = c
        .store()
        .agentes()
        .criar(
            ob.parse().unwrap(),
            poph_store::NovoAgente {
                nome: "Agente".into(),
                slug: "agente".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let col = format!("/admin/v1/orgs/{ob}/agents/{}/integrations", agente.id);
    let (st, erro) = c
        .pedir(
            Method::POST,
            &col,
            &token_b,
            Some(json!({"kind":"eco","config_json":{"senha":"valor-sensivel-teste"}})),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(codigo(&erro), "manifest_indisponivel");
    assert!(!erro.to_string().contains("valor-sensivel-teste"));
    for rota in [
        format!("{col}?limit=inválido"),
        format!("/admin/v1/orgs/{ob}/products?limit=inválido"),
    ] {
        let (status, erro) = c.pedir(Method::GET, &rota, &token_b, None, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(codigo(&erro), "invalid_request");
        assert!(erro.get("message").is_some() && erro.get("details").is_some());
    }
    let (_, lista) = c.pedir(Method::GET, &col, &token_b, None, None).await;
    assert_eq!(lista["items"].as_array().unwrap().len(), 0);
    let (st, v) = c
        .pedir(
            Method::POST,
            &col,
            &token_b,
            Some(json!({"kind":"eco","config_json":{}})),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(v["tem_segredo"], false);
    let id = v["id"].as_str().unwrap();
    let item = format!("{col}/{id}");
    let (st, desconhecido) = c
        .pedir(
            Method::POST,
            &col,
            &token_b,
            Some(json!({"kind":"eco", "senha":"valor-sensivel-teste"})),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_eq!(codigo(&desconhecido), "invalid_request");
    assert!(!desconhecido.to_string().contains("valor-sensivel-teste"));
    let (st, errado) = c.pedir(Method::PUT, &item, &token_b, None, None).await;
    assert_eq!(st, StatusCode::METHOD_NOT_ALLOWED);
    assert!(
        errado.get("code").is_some()
            && errado.get("message").is_some()
            && errado.get("details").is_some()
    );
    for (method, path, body) in [
        (Method::GET, &col, None),
        (Method::POST, &col, Some(json!({"kind":"eco"}))),
        (Method::GET, &item, None),
        (Method::PATCH, &item, Some(json!({"status":"inactive"}))),
        (Method::DELETE, &item, None),
    ] {
        let (st, _) = c.pedir(method, path, &token_a, body, Some(1)).await;
        assert_eq!(st, StatusCode::FORBIDDEN);
    }
    let outro_agente = c
        .store()
        .agentes()
        .criar(
            ob.parse().unwrap(),
            poph_store::NovoAgente {
                nome: "Outro agente".into(),
                slug: "outro-agente".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let caminho_errado = format!(
        "/admin/v1/orgs/{ob}/agents/{}/integrations/{id}",
        outro_agente.id
    );
    for metodo in [Method::GET, Method::PATCH, Method::DELETE] {
        let (st, _) = c
            .pedir(
                metodo,
                &caminho_errado,
                &token_b,
                Some(json!({"status":"inactive"})),
                Some(1),
            )
            .await;
        assert_eq!(st, StatusCode::NOT_FOUND);
    }
    let (st, conf) = c
        .pedir(
            Method::PATCH,
            &item,
            &token_b,
            Some(json!({"status":"inactive"})),
            Some(42),
        )
        .await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED);
    assert_eq!(codigo(&conf), "version_conflict");
    let (st, erro) = c
        .pedir(
            Method::PATCH,
            &item,
            &token_b,
            Some(json!({"config_json":{"token":"valor-sensivel-teste"},"status":"inactive"})),
            Some(1),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(codigo(&erro), "manifest_indisponivel");
    let (st, v) = c
        .pedir(
            Method::PATCH,
            &item,
            &token_b,
            Some(json!({"status":"inactive"})),
            Some(1),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["status"], "inactive");
    let verifica = std::process::Command::new("python3").arg("-c").arg("import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); assert c.execute('select config_json from integrations where id=?',(sys.argv[2],)).fetchone()[0]=='{}'").arg(c.store().caminho_harness()).arg(id).status().unwrap();
    assert!(verifica.success());
    let inject = std::process::Command::new("python3").arg("-c").arg("import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute('update integrations set secrets_enc=? where id=?',(b'valor-sensivel-teste',sys.argv[2])); c.commit()").arg(c.store().caminho_harness()).arg(id).status().unwrap();
    assert!(inject.success());
    let (st, v) = c.pedir(Method::GET, &item, &token_b, None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["tem_segredo"], true);
    assert!(!v.to_string().contains("valor-sensivel-teste"));
    let (st, _) = c.pedir(Method::DELETE, &item, &token_b, None, None).await;
    assert_eq!(st, StatusCode::OK);
    let logs = String::from_utf8(saida.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("captura de log ativa"));
    assert!(!logs.contains("valor-sensivel-teste"));
    assert!(!logs.contains(&token_b));
    let registros = c
        .store()
        .auditoria()
        .listar(ob.parse().unwrap(), poph_store::Pagina::default())
        .unwrap();
    let token_id = token_b.split('_').nth(1).unwrap();
    for acao in ["create", "update", "delete"] {
        let linha = registros
            .iter()
            .find(|r| {
                r.entidade == "integration"
                    && r.acao == acao
                    && r.entidade_id.as_deref() == Some(id)
            })
            .unwrap();
        assert_eq!(linha.ator_tipo, "admin_token");
        assert_eq!(linha.ator_id.as_deref(), Some(token_id));
        assert!(!format!("{linha:?}").contains(&token_b));
        assert!(!format!("{linha:?}").contains("valor-sensivel-teste"));
    }
}

#[tokio::test]
async fn produtos_paridade_cli_estado_no_banco() {
    let c = Cenario::novo("produto-paridade").await;
    let org = c.criar_org("Paridade", "produto-paridade").await;
    let oid = org["id"].as_str().unwrap();
    let categoria = c
        .store()
        .categorias()
        .criar(
            oid.parse().unwrap(),
            poph_store::NovaCategoria {
                nome: "Serviços".into(),
                descricao: String::new(),
            },
        )
        .unwrap();
    let build = std::process::Command::new("cargo")
        .args(["build", "--locked", "-p", "poph"])
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let cli = |args: &[&str]| -> Value {
        let out = std::process::Command::new(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/poph"),
        )
        .args(["--org", oid, "--offline", "product"])
        .args(args)
        .env("POPH_DADOS_DIRETORIO", c.pasta.join("dados"))
        .output()
        .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let cat = categoria.id.to_string();
    let criado = cli(&[
        "create",
        "--name",
        "Visita técnica",
        "--category",
        &cat,
        "--price",
        "12345",
        "--slug",
        "visita-cli",
        "--json",
    ]);
    let col = format!("/admin/v1/orgs/{oid}/products");
    let (st,api)=c.pedir(Method::POST,&col,TOKEN_GLOBAL,Some(json!({"name":"Visita técnica","category_id":cat,"price_cents":12345,"slug":"visita-api"})),None).await;
    assert_eq!(st, StatusCode::CREATED, "{api}");
    let cli_id = criado["product"]["id"].as_str().unwrap();
    let api_id = api["id"].as_str().unwrap();
    let comparar = || {
        let a = c
            .store()
            .produtos()
            .obter(oid.parse().unwrap(), cli_id.parse().unwrap())
            .unwrap();
        let b = c
            .store()
            .produtos()
            .obter(oid.parse().unwrap(), api_id.parse().unwrap())
            .unwrap();
        assert_eq!(
            (
                a.nome,
                a.categoria_id,
                a.preco_centavos,
                a.preco_texto,
                a.descricao,
                a.status,
                a.versao
            ),
            (
                b.nome,
                b.categoria_id,
                b.preco_centavos,
                b.preco_texto,
                b.descricao,
                b.status,
                b.versao
            )
        );
    };
    comparar();
    cli(&[
        "update",
        cli_id,
        "--if-match",
        "1",
        "--name",
        "Visita nova",
        "--price",
        "9000",
        "--json",
    ]);
    let item = format!("{col}/{api_id}");
    let (st, v) = c
        .pedir(
            Method::PATCH,
            &item,
            TOKEN_GLOBAL,
            Some(json!({"name":"Visita nova","price_cents":9000})),
            Some(1),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    comparar();
    cli(&["delete", cli_id, "--force", "--json"]);
    assert_eq!(
        c.pedir(Method::DELETE, &item, TOKEN_GLOBAL, None, None)
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        c.store()
            .produtos()
            .obter(oid.parse().unwrap(), cli_id.parse().unwrap())
            .is_err()
    );
    assert!(
        c.store()
            .produtos()
            .obter(oid.parse().unwrap(), api_id.parse().unwrap())
            .is_err()
    );
}

/// A lista é deliberadamente explícita: publicar uma rota nova exige exercitá-la aqui.
#[tokio::test]
async fn isolamento_por_org_cobre_todas_as_operacoes_publicadas() {
    use std::collections::BTreeSet;
    let c = Cenario::novo("isolamento-openapi").await;
    let a = c.criar_org("Organização A", "isola-a").await;
    let b = c.criar_org("Organização B", "isola-b").await;
    let token = c.criar_token(a["id"].as_str().unwrap(), "a").await;
    let org_b = b["id"].as_str().unwrap();
    let (_, doc) = c.pedir(Method::GET, "/openapi.json", "", None, None).await;
    let esperadas: &[(&str, &str)] = &[
        ("get", "/admin/v1/orgs/{org_id}"),
        ("delete", "/admin/v1/orgs/{org_id}"),
        ("patch", "/admin/v1/orgs/{org_id}"),
        ("get", "/admin/v1/orgs/{org_id}/agents"),
        ("post", "/admin/v1/orgs/{org_id}/agents"),
        ("get", "/admin/v1/orgs/{org_id}/agents/{agent_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/agents/{agent_id}"),
        ("patch", "/admin/v1/orgs/{org_id}/agents/{agent_id}"),
        ("get", "/admin/v1/orgs/{org_id}/agents/{agent_id}/files"),
        ("post", "/admin/v1/orgs/{org_id}/agents/{agent_id}/files"),
        (
            "get",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}/content",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",
        ),
        (
            "patch",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",
        ),
        (
            "patch",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/qrcode",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/state",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/disconnect",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/reindex",
        ),
        ("get", "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat"),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/messages",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/reopen",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/messages/{message_id}/attachments/{attachment_id}/content",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/{group_id}",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/{group_id}",
        ),
        ("get", "/admin/v1/orgs/{org_id}/categories"),
        ("post", "/admin/v1/orgs/{org_id}/categories"),
        ("get", "/admin/v1/orgs/{org_id}/categories/{category_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/categories/{category_id}"),
        ("patch", "/admin/v1/orgs/{org_id}/categories/{category_id}"),
        ("get", "/admin/v1/orgs/{org_id}/conversations"),
        (
            "get",
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/messages",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/reopen",
        ),
        (
            "post",
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/close",
        ),
        (
            "get",
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/turns",
        ),
        ("get", "/admin/v1/orgs/{org_id}/departments"),
        ("post", "/admin/v1/orgs/{org_id}/departments"),
        ("get", "/admin/v1/orgs/{org_id}/departments/{department_id}"),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
        ),
        (
            "patch",
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
        ),
        ("get", "/admin/v1/orgs/{org_id}/knowledge"),
        ("post", "/admin/v1/orgs/{org_id}/knowledge"),
        ("get", "/admin/v1/orgs/{org_id}/knowledge/{group_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/knowledge/{group_id}"),
        ("patch", "/admin/v1/orgs/{org_id}/knowledge/{group_id}"),
        ("get", "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items"),
        ("post", "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items"),
        (
            "get",
            "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}",
        ),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}",
        ),
        (
            "patch",
            "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}",
        ),
        ("get", "/admin/v1/orgs/{org_id}/products"),
        ("post", "/admin/v1/orgs/{org_id}/products"),
        ("get", "/admin/v1/orgs/{org_id}/products/{product_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/products/{product_id}"),
        ("patch", "/admin/v1/orgs/{org_id}/products/{product_id}"),
        ("get", "/admin/v1/orgs/{org_id}/backup"),
        ("post", "/admin/v1/orgs/{org_id}/backup/import"),
        ("get", "/admin/v1/orgs/{org_id}/manifests"),
        ("get", "/admin/v1/orgs/{org_id}/manifests/{kind}"),
        ("get", "/admin/v1/orgs/{org_id}/tokens"),
        ("post", "/admin/v1/orgs/{org_id}/tokens"),
        ("get", "/admin/v1/orgs/{org_id}/tokens/{token_id}"),
        ("delete", "/admin/v1/orgs/{org_id}/tokens/{token_id}"),
        // MX-22: estado do módulo Matrix e espaços por departamento.
        ("get", "/admin/v1/orgs/{org_id}/matrix"),
        ("put", "/admin/v1/orgs/{org_id}/matrix"),
        ("get", "/admin/v1/orgs/{org_id}/matrix/spaces"),
        ("put", "/admin/v1/orgs/{org_id}/matrix/spaces"),
        (
            "delete",
            "/admin/v1/orgs/{org_id}/matrix/spaces/{department_id}",
        ),
    ];
    let esperadas_set: BTreeSet<_> = esperadas
        .iter()
        .map(|(m, p)| ((*m).to_owned(), (*p).to_owned()))
        .collect();
    let mut publicadas = BTreeSet::new();
    for (path, methods) in doc["paths"].as_object().unwrap() {
        if !path.starts_with("/admin/v1/orgs/") {
            continue;
        }
        for method in ["get", "post", "put", "patch", "delete"] {
            if methods.get(method).is_some() {
                publicadas.insert((method.to_owned(), path.clone()));
            }
        }
    }
    assert_eq!(
        publicadas, esperadas_set,
        "rota nova exige teste de isolamento"
    );
    for (metodo, modelo) in esperadas {
        let mut caminho = modelo.replace("{org_id}", org_b);
        // IDs sintaticamente válidos; a verificação da org deve preceder a busca do item.
        for nome in [
            "agent_id",
            "file_id",
            "integration_id",
            "group_id",
            "item_id",
            "category_id",
            "department_id",
            "conversation_id",
            "product_id",
            "token_id",
            "kind",
        ] {
            caminho = caminho.replace(&format!("{{{nome}}}"), org_b);
        }
        let corpo = if *metodo == "post" && modelo.ends_with("/tokens") {
            Some(json!({"label":"teste"}))
        } else if *metodo == "post"
            && (modelo.ends_with("/categories") || modelo.ends_with("/departments"))
        {
            Some(json!({"name":"teste"}))
        } else if *metodo == "put" && modelo.ends_with("/matrix") {
            // Corpo válido de propósito: o `Json` é extraído antes do handler, então o escopo da org
            // só é conferido com o corpo já aceito (o mesmo vale para `post`/`patch`).
            Some(json!({"enabled":true}))
        } else if *metodo == "put" && modelo.ends_with("/matrix/spaces") {
            Some(json!({"department_id": org_b}))
        } else if *metodo == "post" || *metodo == "patch" {
            Some(json!({}))
        } else {
            None
        };
        let method = Method::from_bytes(metodo.to_uppercase().as_bytes()).unwrap();
        let (status, resposta) = c
            .pedir(method.clone(), &caminho, &token, corpo, Some(1))
            .await;
        assert!(
            status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
            "{method} {modelo}: esperado 403/404, recebido {status}: {resposta}"
        );
    }
}

#[tokio::test]
async fn todas_as_edicoes_rejeitam_versao_desatualizada() {
    async fn criar(c: &Cenario, path: &str, body: Value) -> Value {
        let (status, value) = c
            .pedir(Method::POST, path, TOKEN_GLOBAL, Some(body), None)
            .await;
        assert_eq!(status, StatusCode::CREATED, "{path}: {value}");
        value
    }
    let c = Cenario::novo("versoes-agregado").await;
    let org = c.criar_org("Versões", "versoes-agregado").await;
    let id = org["id"].as_str().unwrap();
    let raiz = format!("/admin/v1/orgs/{id}");
    let categoria = criar(
        &c,
        &format!("{raiz}/categories"),
        json!({"name":"Categoria"}),
    )
    .await;
    let departamento = criar(
        &c,
        &format!("{raiz}/departments"),
        json!({"name":"Departamento"}),
    )
    .await;
    let agente = criar(
        &c,
        &format!("{raiz}/agents"),
        json!({"name":"Agente", "slug":"agente"}),
    )
    .await;
    let agent_id = agente["id"].as_str().unwrap();
    let agent_path = format!("{raiz}/agents/{agent_id}");
    criar(
        &c,
        &format!("{agent_path}/instructions"),
        json!({"markdown":"Instruções"}),
    )
    .await;
    let integracao = criar(
        &c,
        &format!("{agent_path}/integrations"),
        json!({"kind":"eco"}),
    )
    .await;
    let grupo = criar(&c, &format!("{raiz}/knowledge"), json!({"name":"Base"})).await;
    let grupo_path = format!("{raiz}/knowledge/{}", grupo["id"].as_str().unwrap());
    let item = criar(
        &c,
        &format!("{grupo_path}/items"),
        json!({"title":"Título","content":"Conteúdo"}),
    )
    .await;
    let produto = criar(
        &c,
        &format!("{raiz}/products"),
        json!({"name":"Produto", "category_id":categoria["id"]}),
    )
    .await;
    let org_id = id.parse().unwrap();
    let conversa = c
        .store()
        .conversas()
        .criar(
            org_id,
            poph_store::NovaConversa {
                agent_id: agent_id.parse().unwrap(),
                integration_id: None,
                canal: "cli".into(),
                peer_externo: None,
                departamento_alvo: None,
            },
        )
        .unwrap();
    c.store()
        .conversas()
        .mudar_status(
            org_id,
            conversa.id,
            conversa.versao,
            poph_store::StatusConversa::Transferida,
            None,
        )
        .unwrap();
    let casos = [
        (Method::PATCH, raiz.clone(), json!({"name":"alterada"})),
        (
            Method::PATCH,
            format!("{raiz}/categories/{}", categoria["id"].as_str().unwrap()),
            json!({"name":"alterada"}),
        ),
        (
            Method::PATCH,
            format!(
                "{raiz}/departments/{}",
                departamento["id"].as_str().unwrap()
            ),
            json!({"name":"alterado"}),
        ),
        (
            Method::PATCH,
            agent_path.clone(),
            json!({"name":"alterado"}),
        ),
        (
            Method::PATCH,
            format!("{agent_path}/instructions/current"),
            json!({"markdown":"alterada"}),
        ),
        (
            Method::PATCH,
            format!(
                "{agent_path}/integrations/{}",
                integracao["id"].as_str().unwrap()
            ),
            json!({"status":"inactive"}),
        ),
        (
            Method::PATCH,
            grupo_path.clone(),
            json!({"name":"alterada"}),
        ),
        (
            Method::PATCH,
            format!("{grupo_path}/items/{}", item["id"].as_str().unwrap()),
            json!({"content":"alterado"}),
        ),
        (
            Method::PATCH,
            format!("{raiz}/products/{}", produto["id"].as_str().unwrap()),
            json!({"name":"alterado"}),
        ),
        (
            Method::POST,
            format!("{raiz}/conversations/{}/reopen", conversa.id),
            json!({}),
        ),
        (
            Method::POST,
            format!("{raiz}/conversations/{}/close", conversa.id),
            json!({}),
        ),
    ];
    for (metodo, caminho, corpo) in casos {
        let (status, resposta) = c
            .pedir(
                metodo.clone(),
                &caminho,
                TOKEN_GLOBAL,
                Some(corpo),
                Some(999),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::PRECONDITION_FAILED,
            "{metodo} {caminho}: {resposta}"
        );
        assert_eq!(codigo(&resposta), "version_conflict", "{metodo} {caminho}");
    }
}

#[tokio::test]
async fn categorias_e_departamentos_paridade_cli_api() {
    let c = Cenario::novo("paridade-categorias-departamentos").await;
    let a = c.criar_org("CLI", "paridade-cli").await;
    let b = c.criar_org("API", "paridade-api").await;
    let oa = a["id"].as_str().unwrap();
    let ob = b["id"].as_str().unwrap();
    let cli = |recurso: &str, args: &[&str]| -> Value {
        let exe = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/poph");
        let output = std::process::Command::new(exe)
            .args(["--org", oa, "--offline", recurso])
            .args(args)
            .env("POPH_DADOS_DIRETORIO", c.pasta.join("dados"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "CLI: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };
    for (recurso, rota, chave) in [
        ("category", "categories", "category"),
        ("department", "departments", "department"),
    ] {
        let pela_cli = cli(
            recurso,
            &[
                "create",
                "--name",
                "Exemplo",
                "--description",
                "inicial",
                "--json",
            ],
        );
        let caminho = format!("/admin/v1/orgs/{ob}/{rota}");
        let (status, pela_api) = c
            .pedir(
                Method::POST,
                &caminho,
                TOKEN_GLOBAL,
                Some(json!({"name":"Exemplo", "description":"inicial"})),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{pela_api}");
        let cli_id = pela_cli[chave]["id"].as_str().unwrap();
        let api_id = pela_api["id"].as_str().unwrap();
        let comparar = |mut a: Value, mut b: Value| {
            for campo in ["id", "org_id", "created_at", "updated_at"] {
                a.as_object_mut().unwrap().remove(campo);
                b.as_object_mut().unwrap().remove(campo);
            }
            assert_eq!(a, b);
        };
        comparar(pela_cli[chave].clone(), pela_api.clone());
        let atualizado_cli = cli(
            recurso,
            &[
                "update",
                cli_id,
                "--if-match",
                "1",
                "--description",
                "alterado",
                "--json",
            ],
        );
        let item = format!("{caminho}/{api_id}");
        let (status, atualizado_api) = c
            .pedir(
                Method::PATCH,
                &item,
                TOKEN_GLOBAL,
                Some(json!({"description":"alterado"})),
                Some(1),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{atualizado_api}");
        comparar(atualizado_cli[chave].clone(), atualizado_api);
        let mut args = vec!["delete", cli_id, "--json"];
        if recurso == "category" {
            args.push("--force");
        }
        cli(recurso, &args);
        let (status, resp) = c
            .pedir(Method::DELETE, &item, TOKEN_GLOBAL, None, None)
            .await;
        assert!(status.is_success(), "{resp}");
        assert_eq!(
            c.pedir(Method::GET, &item, TOKEN_GLOBAL, None, None)
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        let consulta_cli = std::process::Command::new(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/poph"),
        )
        .args(["--org", oa, "--offline", recurso, "show", cli_id, "--json"])
        .env("POPH_DADOS_DIRETORIO", c.pasta.join("dados"))
        .output()
        .unwrap();
        assert!(!consulta_cli.status.success());
    }
}

#[tokio::test]
async fn orgs_e_tokens_paridade_cli_api() {
    let c = Cenario::novo("paridade-org-token").await;
    let cli = |args: &[&str]| -> Value {
        let exe = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/poph");
        let output = std::process::Command::new(exe)
            .arg("--offline")
            .args(args)
            .env("POPH_DADOS_DIRETORIO", c.pasta.join("dados"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "CLI: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let criado_cli = cli(&[
        "org",
        "create",
        "--name",
        "Paridade",
        "--slug",
        "paridade-cli",
        "--json",
    ]);
    let (status, criado_api) = c
        .pedir(
            Method::POST,
            "/admin/v1/orgs",
            TOKEN_GLOBAL,
            Some(json!({"name":"Paridade", "slug":"paridade-api"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{criado_api}");
    let oa = criado_cli["org"]["id"].as_str().unwrap();
    let ob = criado_api["id"].as_str().unwrap();
    let comparar_org = |mut a: Value, mut b: Value| {
        for key in ["id", "slug", "created_at", "updated_at"] {
            a.as_object_mut().unwrap().remove(key);
            b.as_object_mut().unwrap().remove(key);
        }
        assert_eq!(a, b);
    };
    comparar_org(criado_cli["org"].clone(), criado_api.clone());
    let editado_cli = cli(&[
        "org",
        "update",
        oa,
        "--if-match",
        "1",
        "--name",
        "Editada",
        "--json",
    ]);
    let org_api = format!("/admin/v1/orgs/{ob}");
    let (status, editado_api) = c
        .pedir(
            Method::PATCH,
            &org_api,
            TOKEN_GLOBAL,
            Some(json!({"name":"Editada"})),
            Some(1),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{editado_api}");
    comparar_org(editado_cli["org"].clone(), editado_api);
    let token_cli = cli(&["--org", oa, "token", "create", "--label", "Teste", "--json"]);
    let caminho_tokens = format!("{org_api}/tokens");
    let (status, token_api) = c
        .pedir(
            Method::POST,
            &caminho_tokens,
            TOKEN_GLOBAL,
            Some(json!({"label":"Teste"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{token_api}");
    let token_a = token_cli["token"]["id"].as_str().unwrap();
    let token_b = token_api["token"]["id"].as_str().unwrap();
    let comparar_token = |mut a: Value, mut b: Value| {
        for key in ["id", "org_id", "created_at", "revoked_at"] {
            a.as_object_mut().unwrap().remove(key);
            b.as_object_mut().unwrap().remove(key);
        }
        assert_eq!(a, b);
    };
    comparar_token(token_cli["token"].clone(), token_api["token"].clone());
    cli(&["--org", oa, "token", "delete", token_a, "--json"]);
    let token_path = format!("{caminho_tokens}/{token_b}");
    let (status, resp) = c
        .pedir(Method::DELETE, &token_path, TOKEN_GLOBAL, None, None)
        .await;
    assert!(status.is_success(), "{resp}");
    let lido_cli = cli(&["--org", oa, "token", "show", token_a, "--json"]);
    let (status, lido_api) = c
        .pedir(Method::GET, &token_path, TOKEN_GLOBAL, None, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{lido_api}");
    comparar_token(lido_cli["token"].clone(), lido_api);
    cli(&["org", "delete", oa, "--force", "--json"]);
    let (status, resp) = c
        .pedir(
            Method::DELETE,
            &format!("{org_api}?force=true"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert!(status.is_success(), "{resp}");
    for id in [oa, ob] {
        assert!(c.store().orgaos().obter(id.parse().unwrap()).is_err());
    }
}

#[tokio::test]
async fn biblioteca_paridade_consulta_e_exclusao_cli_api() {
    let c = Cenario::novo("paridade-biblioteca").await;
    let org = c.criar_org("Biblioteca", "biblioteca-paridade").await;
    let oid = org["id"].as_str().unwrap();
    let org_id = oid.parse().unwrap();
    let agent = c
        .store()
        .agentes()
        .criar(
            org_id,
            poph_store::NovoAgente {
                nome: "Agente".into(),
                slug: "agente".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let novo = || poph_store::NovoArquivoBiblioteca {
        agent_id: agent.id,
        nome: "Manual".into(),
        descricao: "Guia".into(),
        mime: "application/pdf".into(),
        tamanho_bytes: 42,
        sha256: "a".repeat(64),
        blob_path: "exemplo".into(),
    };
    let a = c.store().biblioteca().criar(org_id, novo()).unwrap();
    let b = c.store().biblioteca().criar(org_id, novo()).unwrap();
    let agente = agent.id.to_string();
    let cli = |args: &[&str]| -> Value {
        let exe = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/poph");
        let out = std::process::Command::new(exe)
            .args(["--org", oid, "--offline", "file"])
            .args(args)
            .env("POPH_DADOS_DIRETORIO", c.pasta.join("dados"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "CLI: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let aid = a.id.to_string();
    let bid = b.id.to_string();
    let mostrada_cli = cli(&["show", &aid, "--agent", &agente, "--json"]);
    let colecao = format!("/admin/v1/orgs/{oid}/agents/{agente}/files");
    let (status, mostrada_api) = c
        .pedir(
            Method::GET,
            &format!("{colecao}/{bid}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    for (campo_cli, campo_api) in [
        ("nome", "name"),
        ("descricao", "description"),
        ("mime", "mime"),
        ("tamanho_bytes", "size_bytes"),
        ("sha256", "sha256"),
    ] {
        assert_eq!(mostrada_cli["arquivo"][campo_cli], mostrada_api[campo_api]);
    }
    cli(&["delete", &aid, "--agent", &agente, "--json"]);
    let (status, resp) = c
        .pedir(
            Method::DELETE,
            &format!("{colecao}/{bid}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    assert!(
        c.store()
            .biblioteca()
            .obter(org_id, agent.id, a.id)
            .is_err()
    );
    assert!(
        c.store()
            .biblioteca()
            .obter(org_id, agent.id, b.id)
            .is_err()
    );
}

/// PNG mínimo: a detecção por conteúdo lê a assinatura e imagem não passa pelo extrator.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89";

/// Executa `poph` contra o banco do cenário (mesma pasta de dados do servidor).
fn cli_poph(pasta: &std::path::Path, args: &[&str]) -> Value {
    let exe = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/poph");
    let saida = std::process::Command::new(exe)
        .args(args)
        .env("POPH_DADOS_DIRETORIO", pasta.join("dados"))
        .output()
        .expect("executa a CLI");
    assert!(
        saida.status.success(),
        "CLI `{}`: {}",
        args.join(" "),
        String::from_utf8_lossy(&saida.stderr)
    );
    serde_json::from_slice(&saida.stdout).expect("JSON da CLI")
}

#[tokio::test]
async fn biblioteca_upload_paridade_cli_api() {
    let c = Cenario::novo("paridade-upload").await;
    let org = c.criar_org("Upload", "paridade-upload").await;
    let oid = org["id"].as_str().unwrap().to_owned();
    let (status, agente) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{oid}/agents"),
            TOKEN_GLOBAL,
            Some(json!({"name":"Agente", "slug":"agente"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{agente}");
    let agent_id = agente["id"].as_str().unwrap().to_owned();

    // Mesmo arquivo para os dois caminhos: o conteúdo é o mesmo, então o blob é deduplicado.
    let arquivo = c.pasta.join("manual.png");
    fs::write(&arquivo, PNG).expect("arquivo de teste");
    let pela_cli = cli_poph(
        &c.pasta,
        &[
            "--org",
            &oid,
            "--offline",
            "file",
            "upload",
            arquivo.to_str().unwrap(),
            "--agent",
            &agent_id,
            "--name",
            "Manual",
            "--description",
            "Guia",
            "--json",
        ],
    );
    let rota = format!(
        "/admin/v1/orgs/{oid}/agents/{agent_id}/files?filename=manual.png&name=Manual&description=Guia"
    );
    let (status, pela_api) = c.enviar_arquivo(&rota, TOKEN_GLOBAL, PNG).await;
    assert_eq!(status, StatusCode::CREATED, "{pela_api}");

    // Mesmo estado final: campo por campo o registro gravado é o mesmo.
    for (campo_cli, campo_api) in [
        ("nome", "name"),
        ("descricao", "description"),
        ("mime", "mime"),
        ("tamanho_bytes", "size_bytes"),
        ("sha256", "sha256"),
    ] {
        assert_eq!(
            pela_cli["arquivo"][campo_cli], pela_api[campo_api],
            "campo `{campo_cli}` diferente entre CLI e API"
        );
    }
    let org_id = oid.parse().unwrap();
    let registros = c
        .store()
        .biblioteca()
        .listar(
            org_id,
            agent_id.parse().unwrap(),
            poph_store::Pagina::default(),
        )
        .expect("lista da biblioteca");
    assert_eq!(registros.len(), 2, "cada caminho gravou o seu registro");
    assert_eq!(registros[0].sha256, registros[1].sha256);
    assert_eq!(
        registros[0].blob_path, registros[1].blob_path,
        "mesmo conteúdo: um blob só, caminho igual"
    );
    let blob = c
        .pasta
        .join("dados")
        .join("blobs")
        .join(&registros[1].blob_path);
    assert!(blob.is_file(), "blob gravado em {}", blob.display());

    // E a criação pela API fica auditada, com ator identificado.
    let auditoria = c
        .store()
        .auditoria()
        .listar(org_id, poph_store::Pagina::default())
        .expect("auditoria");
    assert!(
        auditoria
            .iter()
            .any(|linha| linha.acao == "create" && linha.entidade == "file_library"),
        "criação pela API sem linha de auditoria: {auditoria:?}"
    );

    // Sem `filename` a recusa é no formato de erro da API (nunca corpo vazio nem texto do axum).
    let (status, erro) = c
        .enviar_arquivo(
            &format!("/admin/v1/orgs/{oid}/agents/{agent_id}/files"),
            TOKEN_GLOBAL,
            PNG,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{erro}");
    assert_eq!(codigo(&erro), "validation_error", "{erro}");
    assert!(
        erro["message"].as_str().is_some_and(|m| !m.is_empty()),
        "recusa sem mensagem: {erro}"
    );

    // Arquivo que a API recusa também é recusado pela CLI, e nada é gravado.
    let (status, erro) = c
        .enviar_arquivo(
            &format!("/admin/v1/orgs/{oid}/agents/{agent_id}/files?filename=programa.exe"),
            TOKEN_GLOBAL,
            b"conteudo qualquer",
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{erro}");
    assert_eq!(codigo(&erro), "validation_error", "{erro}");
    assert_eq!(
        c.store()
            .biblioteca()
            .contar(org_id, agent_id.parse().unwrap())
            .unwrap(),
        2,
        "arquivo recusado não pode virar registro"
    );

    // O limite de 16 MB é o da rota (a API não herda o teto curto padrão do axum, que cortaria
    // arquivos que o WhatsApp aceita).
    let mut grande = PNG.to_vec();
    grande.resize(3 * 1024 * 1024, 0);
    let (status, corpo) = c
        .enviar_arquivo(
            &format!("/admin/v1/orgs/{oid}/agents/{agent_id}/files?filename=grande.png"),
            TOKEN_GLOBAL,
            &grande,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{corpo}");
    assert_eq!(corpo["size_bytes"], json!(grande.len()));
}

#[tokio::test]
async fn vinculo_grupo_agente_paridade_cli_api() {
    let c = Cenario::novo("paridade-vinculo").await;
    let org = c.criar_org("Vínculo", "paridade-vinculo").await;
    let oid = org["id"].as_str().unwrap().to_owned();
    let org_id = oid.parse().unwrap();
    let agente = c
        .store()
        .agentes()
        .criar(
            org_id,
            poph_store::NovoAgente {
                nome: "Agente".into(),
                slug: "agente".into(),
                ..Default::default()
            },
        )
        .expect("agente");
    let aid = agente.id.to_string();

    let grupo_cli = cli_poph(
        &c.pasta,
        &[
            "--org",
            &oid,
            "--offline",
            "kb",
            "group",
            "create",
            "--name",
            "Pela CLI",
            "--json",
        ],
    );
    let grupo_cli_id = grupo_cli["group"]["id"].as_str().unwrap().to_owned();
    let (status, grupo_api) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{oid}/knowledge"),
            TOKEN_GLOBAL,
            Some(json!({"name":"Pela API"})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{grupo_api}");
    let grupo_api_id = grupo_api["id"].as_str().unwrap().to_owned();

    // Vincular: um grupo por cada caminho.
    cli_poph(
        &c.pasta,
        &[
            "--org",
            &oid,
            "--offline",
            "kb",
            "link",
            &grupo_cli_id,
            "--agent",
            &aid,
            "--json",
        ],
    );
    let (status, resposta) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{oid}/agents/{aid}/knowledge/{grupo_api_id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{resposta}");
    assert_eq!(resposta["linked"], json!(true));

    // Mesmo estado: cada grupo tem exatamente este agente vinculado.
    let estado = |grupo: &str| -> Vec<String> {
        let mut agentes: Vec<String> = c
            .store()
            .conhecimento()
            .agentes_do_grupo(org_id, grupo.parse().unwrap())
            .expect("agentes do grupo")
            .iter()
            .map(ToString::to_string)
            .collect();
        agentes.sort();
        agentes
    };
    assert_eq!(estado(&grupo_cli_id), vec![aid.clone()]);
    assert_eq!(estado(&grupo_api_id), estado(&grupo_cli_id));

    // Desvincular também pelos dois caminhos, com o mesmo estado final.
    cli_poph(
        &c.pasta,
        &[
            "--org",
            &oid,
            "--offline",
            "kb",
            "unlink",
            &grupo_cli_id,
            "--agent",
            &aid,
            "--json",
        ],
    );
    let (status, resposta) = c
        .pedir(
            Method::DELETE,
            &format!("/admin/v1/orgs/{oid}/agents/{aid}/knowledge/{grupo_api_id}"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{resposta}");
    assert!(estado(&grupo_cli_id).is_empty());
    assert!(estado(&grupo_api_id).is_empty());

    // E os dois vínculos (link e unlink) deixaram auditoria com ator.
    let auditoria = c
        .store()
        .auditoria()
        .listar(org_id, poph_store::Pagina::default())
        .expect("auditoria");
    for acao in ["link", "unlink"] {
        assert!(
            auditoria.iter().any(|linha| linha.acao == acao),
            "sem auditoria de `{acao}`: {auditoria:?}"
        );
    }
}

/// Exige o modelo real de embedding: o teste liga `data/modelos` na pasta de dados do cenário,
/// então o caminho de modelo da API e da CLI é o mesmo.
#[tokio::test]
#[ignore = "exige o modelo real instalado em data/modelos"]
async fn reindex_paridade_cli_api() {
    let c = Cenario::novo("paridade-reindex").await;
    let modelos = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/modelos");
    std::os::unix::fs::symlink(&modelos, c.pasta.join("dados").join("modelos"))
        .expect("link simbólico do modelo");

    let org = c.criar_org("Reindex", "paridade-reindex").await;
    let oid = org["id"].as_str().unwrap().to_owned();
    let org_id = oid.parse().unwrap();
    let agente = c
        .store()
        .agentes()
        .criar(
            org_id,
            poph_store::NovoAgente {
                nome: "Agente".into(),
                slug: "agente".into(),
                ..Default::default()
            },
        )
        .expect("agente");
    let aid = agente.id.to_string();
    let grupo = c
        .store()
        .conhecimento()
        .criar_grupo(
            org_id,
            poph_store::NovoGrupoConhecimento {
                nome: "Guia".into(),
                descricao: String::new(),
            },
        )
        .expect("grupo");
    c.store()
        .conhecimento()
        .vincular_agente(org_id, agente.id, grupo.id)
        .expect("vínculo");
    for (titulo, conteudo) in [
        (
            "Prazo de entrega",
            "Entregamos todos os pedidos em três dias úteis.",
        ),
        (
            "Troca",
            "A troca é aceita em até sete dias após o recebimento.",
        ),
    ] {
        c.store()
            .conhecimento()
            .criar_item(
                org_id,
                poph_store::NovoItemConhecimento {
                    grupo_id: grupo.id,
                    titulo: titulo.into(),
                    conteudo: conteudo.into(),
                    tags: vec![],
                    posicao: 0,
                },
            )
            .expect("item");
    }

    // 1) CLI reindexa primeiro: dois itens novos entram no índice.
    let pela_cli = cli_poph(
        &c.pasta,
        &[
            "--org",
            &oid,
            "--offline",
            "kb",
            "reindex",
            "--agent",
            &aid,
            "--json",
        ],
    );
    assert_eq!(pela_cli["agents"][0]["indexed"]["items_new"], json!(2));

    // 2) API reindexa em seguida: encontra o índice exatamente como a CLI deixou.
    let rota = format!("/admin/v1/orgs/{oid}/agents/{aid}/knowledge/reindex");
    let (status, pela_api) = c.pedir(Method::POST, &rota, TOKEN_GLOBAL, None, None).await;
    assert_eq!(status, StatusCode::OK, "{pela_api}");
    assert_eq!(pela_api["indexed"]["itens_novos"], json!(0));
    assert_eq!(pela_api["indexed"]["itens_alterados"], json!(0));
    assert_eq!(pela_api["indexed"]["itens_inalterados"], json!(2));
    assert_eq!(pela_api["indexed"]["embeddings_gerados"], json!(0));

    // 3) Item novo gravado direto no banco (sem a sincronização automática da API): a API
    // reindexa e a CLI, em seguida, encontra o índice exatamente como a API deixou.
    c.store()
        .conhecimento()
        .criar_item(
            org_id,
            poph_store::NovoItemConhecimento {
                grupo_id: grupo.id,
                titulo: "Frete".into(),
                conteudo: "O frete é grátis acima de cem reais.".into(),
                tags: vec![],
                posicao: 0,
            },
        )
        .expect("item novo");
    let (status, pela_api) = c.pedir(Method::POST, &rota, TOKEN_GLOBAL, None, None).await;
    assert_eq!(status, StatusCode::OK, "{pela_api}");
    assert_eq!(pela_api["indexed"]["itens_novos"], json!(1));
    assert_eq!(pela_api["indexed"]["itens_inalterados"], json!(2));
    let pela_cli = cli_poph(
        &c.pasta,
        &[
            "--org",
            &oid,
            "--offline",
            "kb",
            "reindex",
            "--agent",
            &aid,
            "--json",
        ],
    );
    let indice = &pela_cli["agents"][0]["indexed"];
    assert_eq!(indice["items_new"], json!(0));
    assert_eq!(indice["items_changed"], json!(0));
    assert_eq!(indice["items_unchanged"], json!(3));
    assert_eq!(indice["embeddings"], json!(0));
}

/// A tabela de `docs/API.md` é a fonte da verdade do inventário da HR-21: publicar rota nova sem
/// documentar, documentar rota inexistente ou trocar uma coluna (412, auditoria, isolamento)
/// reprova aqui.
#[tokio::test]
async fn docs_api_cobre_todas_as_rotas_publicadas() {
    use std::collections::BTreeSet;
    let c = Cenario::novo("docs-api").await;
    let (status, documento) = c.pedir(Method::GET, "/openapi.json", "", None, None).await;
    assert_eq!(status, StatusCode::OK);

    let caminho = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/API.md");
    let texto = fs::read_to_string(&caminho)
        .unwrap_or_else(|erro| panic!("leitura de {}: {erro}", caminho.display()));
    for secao in [
        "## Inventário da HR-21",
        "## Estado da prova",
        "## O que a HR-21 entrega e o que não entrega",
    ] {
        assert!(texto.contains(secao), "docs/API.md sem a seção `{secao}`");
    }

    /// Uma linha da tabela do inventário.
    struct Linha {
        metodo: String,
        rota: String,
        cli: String,
        versao: String,
        auditoria: String,
        isolamento: String,
        crua: String,
    }

    // Operações de configuração da instância (token global, sem escopo de org): não exigem
    // `If-Match`, não entram na auditoria por org e o isolamento por org não se aplica. A
    // documentação marca as três colunas como `n/a`.
    const GLOBAIS_DA_INSTANCIA: &[(&str, &str)] = &[("post", "/admin/v1/cluster/reload")];
    let metodos = ["get", "post", "put", "patch", "delete"];
    let mut linhas: Vec<Linha> = Vec::new();
    for linha in texto.lines() {
        let celulas: Vec<&str> = linha.split('|').map(str::trim).collect();
        // `| a | b |` → ["", "a", "b", ""]: sete colunas viram nove pedaços.
        if celulas.len() != 9 || celulas[1..8].iter().any(|celula| celula.is_empty()) {
            continue;
        }
        let metodo = celulas[2].to_lowercase();
        if !metodos.contains(&metodo.as_str()) {
            continue;
        }
        linhas.push(Linha {
            metodo,
            rota: celulas[3].trim_matches('`').to_owned(),
            cli: celulas[4].to_owned(),
            versao: celulas[5].to_owned(),
            auditoria: celulas[6].to_owned(),
            isolamento: celulas[7].to_owned(),
            crua: linha.trim().to_owned(),
        });
    }
    assert!(
        linhas.len() >= 60,
        "a tabela do inventário tem só {} linha(s)",
        linhas.len()
    );
    let documentadas: BTreeSet<(String, String)> = linhas
        .iter()
        .map(|linha| (linha.metodo.clone(), linha.rota.clone()))
        .collect();
    assert_eq!(
        documentadas.len(),
        linhas.len(),
        "rota repetida na tabela do inventário"
    );

    let mut publicadas: BTreeSet<(String, String)> = BTreeSet::new();
    for (rota, operacoes) in documento["paths"].as_object().expect("paths") {
        if !rota.starts_with("/admin/v1/") {
            continue;
        }
        for metodo in metodos {
            if operacoes.get(metodo).is_some() {
                publicadas.insert((metodo.to_owned(), rota.clone()));
            }
        }
    }
    assert_eq!(
        documentadas, publicadas,
        "docs/API.md e o OpenAPI divergem (documentar rota nova faz parte da entrega)"
    );

    for linha in &linhas {
        let global_da_instancia = GLOBAIS_DA_INSTANCIA
            .iter()
            .any(|(metodo, rota)| *metodo == linha.metodo && *rota == linha.rota);
        // 412: toda edição publicada tem conflito de versão, inclusive as duas ações de WhatsApp
        // da HR-44b (elas exigem `If-Match` sem subir a versão); o resto é `n/a`.
        let escrita_com_if_match = linha.metodo == "patch"
            || linha.rota.ends_with("/reopen")
            || linha.rota.ends_with("/close")
            || linha.rota.ends_with("/whatsapp/qrcode")
            || linha.rota.ends_with("/whatsapp/disconnect")
            || (linha.metodo == "put" && linha.rota.ends_with("/matrix"));
        let esperado = if escrita_com_if_match && !global_da_instancia {
            "sim"
        } else {
            "n/a"
        };
        assert_eq!(
            linha.versao, esperado,
            "coluna 412 errada em `{}`",
            linha.crua
        );

        // Auditoria: toda escrita de dado de org audita; leitura e configuração da instância são
        // `n/a`. `put` entra na lista porque as duas rotas `PUT` publicadas (módulo Matrix, MX-22)
        // são escrita de dado de org e auditam.
        let escrita = matches!(linha.metodo.as_str(), "post" | "put" | "patch" | "delete");
        assert_eq!(
            linha.auditoria,
            if escrita && !global_da_instancia {
                "sim"
            } else {
                "n/a"
            },
            "coluna auditoria errada em `{}`",
            linha.crua
        );

        // Isolamento por org: toda rota de org é escopada; a de configuração da instância exige
        // token global e não tem org no caminho.
        assert_eq!(
            linha.isolamento,
            if global_da_instancia { "n/a" } else { "sim" },
            "isolamento em `{}`",
            linha.crua
        );

        // Paridade: `sim` ou `exceção: <motivo>`.
        assert!(
            linha.cli == "sim" || linha.cli.starts_with("exceção: "),
            "coluna CLI precisa de `sim` ou `exceção: <motivo>` em `{}`",
            linha.crua
        );
    }
}

/// MX-06: `POST …/conversations/{id}/close` com `If-Match`. Cobre os seis status (200, 401, 403,
/// 404, 409, 412) e o 428 sem o cabeçalho; prova que as recusas não mudam a conversa nem gravam
/// auditoria, e que a auditoria `close` sobe a versão uma vez só.
#[tokio::test]
async fn fechar_conversa_cumpre_codigos_e_audita_uma_vez() {
    let c = Cenario::novo("fechar-conversa").await;
    let org_a = c.criar_org("Org A", "fecha-a").await;
    let org_b = c.criar_org("Org B", "fecha-b").await;
    let a = org_a["id"].as_str().unwrap();
    let b = org_b["id"].as_str().unwrap();
    let token_a = c.criar_token(a, "org a").await;
    let token_b = c.criar_token(b, "org b").await;
    let org: poph_types::OrgId = b.parse().unwrap();
    let agente = c
        .store()
        .agentes()
        .criar(
            org,
            poph_store::NovoAgente {
                nome: "Agente".into(),
                slug: "agente".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let conversa = c
        .store()
        .conversas()
        .criar(
            org,
            poph_store::NovaConversa {
                agent_id: agente.id,
                integration_id: None,
                canal: "cli".into(),
                peer_externo: None,
                departamento_alvo: None,
            },
        )
        .unwrap();
    let rota = format!("/admin/v1/orgs/{b}/conversations/{}/close", conversa.id);
    let auditorias_close = |c: &Cenario| {
        c.store()
            .auditoria()
            .listar(org, poph_store::Pagina::default())
            .unwrap()
            .iter()
            .filter(|r| {
                r.acao == "close" && r.entidade_id.as_deref() == Some(&conversa.id.to_string())
            })
            .count()
    };
    let estado = |c: &Cenario| {
        let atual = c.store().conversas().obter(org, conversa.id).unwrap();
        (atual.status.como_texto(), atual.versao)
    };

    // 401 sem token.
    assert_eq!(
        c.pedir(Method::POST, &rota, "", None, Some(1)).await.0,
        StatusCode::UNAUTHORIZED
    );
    // 403: token da org A não fecha conversa da org B.
    assert_eq!(
        c.pedir(Method::POST, &rota, &token_a, None, Some(1))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    // 428: sem If-Match.
    assert_eq!(
        c.pedir(Method::POST, &rota, &token_b, None, None).await.0,
        StatusCode::PRECONDITION_REQUIRED
    );
    // 412: versão antiga. Nada muda e nada é auditado.
    assert_eq!(
        c.pedir(Method::POST, &rota, &token_b, None, Some(99))
            .await
            .0,
        StatusCode::PRECONDITION_FAILED
    );
    assert_eq!(estado(&c), ("open", 1));
    assert_eq!(auditorias_close(&c), 0);
    // 404: conversa que não existe na org.
    let inexistente = format!(
        "/admin/v1/orgs/{b}/conversations/{}/close",
        poph_types::ConversationId::generate()
    );
    assert_eq!(
        c.pedir(Method::POST, &inexistente, &token_b, None, Some(1))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    // 200: fecha com a versão atual.
    let (status, corpo) = c.pedir(Method::POST, &rota, &token_b, None, Some(1)).await;
    assert_eq!(status, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["status"], "closed");
    assert_eq!(corpo["version"], 2);
    assert_eq!(estado(&c), ("closed", 2));
    assert_eq!(
        auditorias_close(&c),
        1,
        "uma auditoria `close` por fechamento"
    );
    // 409: já fechada. Com a versão atual (2), o pedido é de estado, não de versão.
    assert_eq!(
        c.pedir(Method::POST, &rota, &token_b, None, Some(2))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(estado(&c), ("closed", 2), "409 não sobe a versão");
    assert_eq!(auditorias_close(&c), 1, "409 não grava auditoria");
    // Auditoria: ator é o token que fechou.
    let registro = c
        .store()
        .auditoria()
        .listar(org, poph_store::Pagina::default())
        .unwrap()
        .into_iter()
        .find(|r| r.acao == "close")
        .expect("fechamento auditado");
    assert_eq!(
        registro.ator_id.as_deref(),
        token_b
            .strip_prefix("poph_")
            .and_then(|s| s.split_once('_'))
            .map(|(id, _)| id)
    );
}

/// MX-06: a próxima mensagem do cliente depois do fechamento abre conversa nova (a fechada não
/// conta na busca da caixa de entrada). Passa pelo mesmo caminho do canal (WhatsApp).
#[tokio::test]
async fn mensagem_depois_do_fechamento_abre_conversa_nova() {
    let c = Cenario::novo("fechar-e-reabrir-por-mensagem").await;
    let org_a = c.criar_org("Org A", "fim-a").await;
    let a = org_a["id"].as_str().unwrap();
    let token = c.criar_token(a, "org a").await;
    let org: poph_types::OrgId = a.parse().unwrap();
    let agente = c
        .store()
        .agentes()
        .criar(
            org,
            poph_store::NovoAgente {
                nome: "Agente".into(),
                slug: "agente".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let integracao = c
        .store()
        .integracao_criar(org, agente.id, "whatsapp".into(), "active".into())
        .unwrap()
        .id
        .parse::<poph_types::IntegrationId>()
        .unwrap();
    let entrada = |id_externo: &str| poph_store::NovaEntradaCanal {
        integration_id: integracao,
        agent_id: agente.id,
        canal: "whatsapp".to_owned(),
        id_externo: id_externo.to_owned(),
        peer: "5521999990000".to_owned(),
        evento_json: "{}".to_owned(),
        texto: "oi".to_owned(),
        anexos: Vec::new(),
        nome_cliente: None,
    };
    let primeira = c
        .store()
        .caixa_entrada()
        .receber_do_canal(org, entrada("MSG-1"))
        .unwrap();
    let conversa_1 = primeira.mensagem.conversation_id;
    let versao = c.store().conversas().obter(org, conversa_1).unwrap().versao;
    let (status, _) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{a}/conversations/{conversa_1}/close"),
            &token,
            None,
            Some(versao),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let segunda = c
        .store()
        .caixa_entrada()
        .receber_do_canal(org, entrada("MSG-2"))
        .unwrap();
    let conversa_2 = segunda.mensagem.conversation_id;
    assert_ne!(
        conversa_2, conversa_1,
        "a mensagem seguinte abre conversa nova"
    );
    assert_eq!(
        c.store().conversas().obter(org, conversa_1).unwrap().status,
        poph_store::StatusConversa::Fechada
    );
    assert_eq!(
        c.store().conversas().obter(org, conversa_2).unwrap().status,
        poph_store::StatusConversa::Aberta
    );
}

/// FX-07 (PRD L13): `reopen` de conversa `closed` passa a devolver `409 conversa_encerrada`, e não
/// reabre. O caminho para o cliente voltar é a mensagem nova dele, que abre outra conversa e outra
/// Ficha (teste `mensagem_depois_do_fechamento_abre_conversa_nova`). `transferred` continua reabrindo
/// (`conversas_isolam_organizacoes`, acima).
#[tokio::test]
async fn reabrir_conversa_encerrada_devolve_409_conversa_encerrada() {
    let c = Cenario::novo("reabrir-encerrada").await;
    let org_a = c.criar_org("Org A", "fim-reabrir").await;
    let a = org_a["id"].as_str().unwrap();
    let token = c.criar_token(a, "org a").await;
    let org: poph_types::OrgId = a.parse().unwrap();
    let agente = c
        .store()
        .agentes()
        .criar(
            org,
            poph_store::NovoAgente {
                nome: "Agente".into(),
                slug: "agente".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let conversa = c
        .store()
        .conversas()
        .criar(
            org,
            poph_store::NovaConversa {
                agent_id: agente.id,
                integration_id: None,
                canal: "console".to_owned(),
                peer_externo: Some("cliente-1".to_owned()),
                departamento_alvo: None,
            },
        )
        .unwrap();

    let (status, _) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{a}/conversations/{}/close", conversa.id),
            &token,
            None,
            Some(conversa.versao),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let fechada = c.store().conversas().obter(org, conversa.id).unwrap();
    let (status, corpo) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{a}/conversations/{}/reopen", conversa.id),
            &token,
            None,
            Some(fechada.versao),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(corpo["code"], "conversa_encerrada");

    // A conversa continua `closed` e a Ficha do atendimento também.
    let depois = c.store().conversas().obter(org, conversa.id).unwrap();
    assert_eq!(depois.status, poph_store::StatusConversa::Fechada);
    assert_eq!(depois.versao, fechada.versao, "a recusa não muda a versão");
    let ficha = c
        .store()
        .fichas()
        .por_conversa(org, conversa.id)
        .unwrap()
        .expect("a conversa fechada pelo administrador tem Ficha fechada");
    assert_eq!(ficha.status, poph_types::StatusFicha::Fechada);
    assert_eq!(ficha.fechada_por, Some(poph_types::FechadaPor::Admin));
}
