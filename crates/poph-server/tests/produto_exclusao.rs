//! Exclusão de produto com `force` (HR-42f).
//!
//! Um item de carrinho aponta para o produto. Excluir o produto zera essa referência
//! (`ON DELETE SET NULL`), então sem `force=true` a API recusa com 409 e a contagem, e com
//! `force=true` apaga e o item continua no carrinho com o preço gravado.
//!
//! Servidor real, banco temporário, porta efêmera. Nenhum teste toca rede externa.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::Estado;
use poph_store::{NewConversation, Store};
use poph_types::config::GlobalTokenConfig;
use poph_types::{AgentId, HarnessConfig, OrgId, ProductId};
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

/// Valor do token global usado nos testes (o servidor guarda só o hash).
const TOKEN_GLOBAL: &str = "pophg_teste-global-hr42f";

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
            "poph-produto-exclusao-{rotulo}-{}-{carimbo}",
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
            Estado::novo(std::sync::Arc::clone(&store), config).expect("estado do servidor");

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
        corpo["id"].as_str().expect("id da org").to_owned()
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

    async fn criar_produto(&self, org_id: &str, nome: &str, centavos: i64) -> String {
        // Produto exige categoria: cria uma só para este produto (nome único por org).
        let (status, categoria) = self
            .pedir(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/categories"),
                TOKEN_GLOBAL,
                Some(json!({ "name": format!("Cat {nome}") })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar categoria: {categoria}");
        let categoria_id = categoria["id"].as_str().expect("id da categoria");
        let (status, corpo) = self
            .pedir(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/products"),
                TOKEN_GLOBAL,
                Some(json!({ "name": nome, "price_cents": centavos, "category_id": categoria_id })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar produto: {corpo}");
        corpo["id"].as_str().expect("id do produto").to_owned()
    }

    async fn criar_agente(&self, org_id: &str) -> String {
        let (status, corpo) = self
            .pedir(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/agents"),
                TOKEN_GLOBAL,
                Some(json!({ "name": "Agente", "slug": "agente" })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar agente: {corpo}");
        corpo["id"].as_str().expect("id do agente").to_owned()
    }

    /// Põe `quantidade` do produto no carrinho de uma conversa nova da org. Como a API ainda não
    /// tem rota de carrinho na administração, usa o armazenamento (o mesmo que o turno usa).
    fn colocar_no_carrinho(
        &self,
        org_id: &str,
        agente_id: &str,
        produto_id: &str,
        quantidade: u32,
    ) {
        let org: OrgId = org_id.parse().expect("org");
        let agente: AgentId = agente_id.parse().expect("agente");
        let produto: ProductId = produto_id.parse().expect("produto");
        let conversa = self
            .store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("abre a conversa")
            .id;
        self.store
            .carts()
            .add(org, conversa, agente, produto, quantidade)
            .expect("põe o produto no carrinho");
    }

    /// Linhas do carrinho de uma conversa qualquer da org: (produto, nome, preço unitário).
    fn linhas_do_carrinho(
        &self,
        org_id: &str,
        agente_id: &str,
    ) -> Vec<(Option<String>, String, i64)> {
        let org: OrgId = org_id.parse().expect("org");
        let agente: AgentId = agente_id.parse().expect("agente");
        let mut linhas = Vec::new();
        let conversas = self
            .store
            .conversations()
            .list(
                org,
                None,
                poph_store::Page {
                    after: None,
                    limit: 200,
                },
            )
            .expect("lista conversas");
        for conversa in conversas {
            if conversa.agent_id != agente {
                continue;
            }
            let resumo = self
                .store
                .carts()
                .summary(org, conversa.id)
                .expect("resumo do carrinho");
            for item in resumo.items {
                linhas.push((
                    item.product.map(|p| p.to_string()),
                    item.nome,
                    item.price_unit_cents,
                ));
            }
        }
        linhas
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

fn codigo(corpo: &Value) -> &str {
    corpo["code"].as_str().unwrap_or("")
}

#[tokio::test]
async fn produto_em_carrinho_sem_force_responde_409_e_nao_apaga() {
    let c = Cenario::novo("sem-force").await;
    let org = c.criar_org("Org Sem Force", "org-sem-force").await;
    let produto = c.criar_produto(&org, "Dedetização", 18000).await;
    let agente = c.criar_agente(&org).await;
    // Dois carrinhos (duas conversas) com o mesmo produto: a contagem tem de separar itens e carrinhos.
    c.colocar_no_carrinho(&org, &agente, &produto, 2);
    c.colocar_no_carrinho(&org, &agente, &produto, 1);
    let caminho = format!("/admin/v1/orgs/{org}/products/{produto}");

    let (status, resposta) = c.pedir(Method::DELETE, &caminho, TOKEN_GLOBAL, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{resposta}");
    assert_eq!(codigo(&resposta), "conflict");
    assert_eq!(resposta["details"]["itens_de_carrinho"], 2, "{resposta}");
    assert_eq!(resposta["details"]["carrinhos"], 2, "{resposta}");
    assert!(
        resposta["message"].as_str().unwrap().contains("force=true"),
        "{resposta}"
    );

    // Nada apagado: o produto continua, e os itens continuam com o nome.
    let (status, _) = c.pedir(Method::GET, &caminho, TOKEN_GLOBAL, None).await;
    assert_eq!(status, StatusCode::OK, "o produto não pode sumir sem force");
    let linhas = c.linhas_do_carrinho(&org, &agente);
    assert_eq!(linhas.len(), 2, "{linhas:?}");
    assert!(
        linhas
            .iter()
            .all(|(produto_linha, nome, _)| produto_linha.is_some() && nome == "Dedetização")
    );
}

#[tokio::test]
async fn com_force_apaga_e_o_item_fica_com_o_preco_gravado() {
    let c = Cenario::novo("com-force").await;
    let org = c.criar_org("Org Com Force", "org-com-force").await;
    let produto = c.criar_produto(&org, "Visita técnica", 18000).await;
    let agente = c.criar_agente(&org).await;
    c.colocar_no_carrinho(&org, &agente, &produto, 1);
    let caminho = format!("/admin/v1/orgs/{org}/products/{produto}");

    let (status, resposta) = c
        .pedir(
            Method::DELETE,
            &format!("{caminho}?force=true"),
            TOKEN_GLOBAL,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{resposta}");
    assert_eq!(resposta["deleted"], true);

    let (status, _) = c.pedir(Method::GET, &caminho, TOKEN_GLOBAL, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // O item continua no carrinho: sem produto, com o nome do aviso e o preço que já estava gravado.
    let linhas = c.linhas_do_carrinho(&org, &agente);
    assert_eq!(linhas.len(), 1, "{linhas:?}");
    let (produto_linha, nome, preco) = &linhas[0];
    assert!(produto_linha.is_none(), "{linhas:?}");
    assert_eq!(nome, "produto removido do catálogo");
    assert_eq!(*preco, 18000);
}

#[tokio::test]
async fn produto_sem_carrinho_apaga_sem_force() {
    let c = Cenario::novo("sem-vinculo").await;
    let org = c.criar_org("Org Livre", "org-livre").await;
    let produto = c.criar_produto(&org, "Sem uso", 5000).await;
    let caminho = format!("/admin/v1/orgs/{org}/products/{produto}");

    let (status, resposta) = c.pedir(Method::DELETE, &caminho, TOKEN_GLOBAL, None).await;
    assert_eq!(status, StatusCode::OK, "{resposta}");
    let (status, _) = c.pedir(Method::GET, &caminho, TOKEN_GLOBAL, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn token_de_outra_org_nao_apaga_e_produto_inexistente_responde_404() {
    let c = Cenario::novo("isolamento").await;
    let org_a = c.criar_org("Org A", "org-a-produto").await;
    let org_b = c.criar_org("Org B", "org-b-produto").await;
    let token_a = c.criar_token(&org_a).await;
    let produto_b = c.criar_produto(&org_b, "Produto de B", 7000).await;
    let agente_b = c.criar_agente(&org_b).await;
    c.colocar_no_carrinho(&org_b, &agente_b, &produto_b, 1);

    for sufixo in ["", "?force=true"] {
        let (status, _) = c
            .pedir(
                Method::DELETE,
                &format!("/admin/v1/orgs/{org_b}/products/{produto_b}{sufixo}"),
                &token_a,
                None,
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "sufixo {sufixo:?}");
    }
    let (status, _) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{org_b}/products/{produto_b}"),
            TOKEN_GLOBAL,
            None,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "o produto da outra org não pode sumir"
    );

    let inexistente = ProductId::generate();
    let (status, resposta) = c
        .pedir(
            Method::DELETE,
            &format!("/admin/v1/orgs/{org_b}/products/{inexistente}"),
            TOKEN_GLOBAL,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{resposta}");
}
