//! Testes HTTP da permissão de fluxos por agente (FX-28, PRD §6.2).
//!
//! Servidor de verdade (porta efêmera) sobre banco temporário, com cliente HTTP de verdade. Os
//! fluxos são criados direto no repositório, porque a rota de fluxo é da FX-27.
//!
//! O que fica provado aqui:
//! - `GET` lista vazia e depois a lista liberada, em ordem de slug;
//! - `PUT` exige `If-Match` com a versão do **agente** (428 sem, 412 com versão velha);
//! - a troca é a lista inteira, sobe a versão do agente e conta uma vez cada id repetido;
//! - lote com fluxo de outra org ou inexistente → 404 e nada muda (nem a lista, nem a versão);
//! - id que não é UUID → 400 `invalid_id`; corpo sem `flow_ids` → 400 `invalid_request`;
//! - teto de 20 (o 20º passa) e 21 → 409 `teto_de_fluxos_por_agente`, sem mudar nada;
//! - token de outra org → 403; agente de outra org pela rota → 404;
//! - agente `inactive` continua recebendo permissão;
//! - cada `PUT` bem-sucedido grava `agent_flows_update` com o autor do token, e os que falharam não.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::Estado;
use poph_store::{NovoFluxo, Pagina, Store};
use poph_types::config::GlobalTokenConfig;
use poph_types::{AgentId, FlowId, HarnessConfig, OrgId};
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

/// Valor do token global usado nos testes (o servidor guarda só o hash).
const TOKEN_GLOBAL: &str = "pophg_teste-global-fx28";

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
    store: Arc<Store>,
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
        config.data.directory = pasta.join("dados");
        config.server.port = 7481;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::abrir(&config).expect("abre o banco"));
        let estado = Estado::novo(Arc::clone(&store), config).expect("estado do servidor");

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

    fn store(&self) -> &Store {
        &self.store
    }

    async fn pedir(
        &self,
        metodo: Method,
        caminho: &str,
        token: &str,
        corpo: Option<Value>,
        if_match: Option<u64>,
    ) -> (StatusCode, Value) {
        let mut pedido = self
            .cliente
            .request(metodo, format!("{}{caminho}", self.base))
            .bearer_auth(token);
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

    async fn criar_org(&self, nome: &str, slug: &str) -> String {
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
        corpo["id"].as_str().expect("id da org").to_owned()
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
        corpo["value"].as_str().expect("valor do token").to_owned()
    }

    async fn criar_agente(&self, org_id: &str, token: &str, slug: &str) -> String {
        let (status, corpo) = self
            .pedir(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/agents"),
                token,
                Some(json!({ "name": format!("Agente {slug}"), "slug": slug })),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar agente: {corpo}");
        corpo["id"].as_str().expect("id do agente").to_owned()
    }

    /// Fluxo da org, criado direto no repositório (a rota de fluxo é da FX-27).
    fn criar_fluxo(&self, org_id: &str, slug: &str) -> String {
        let org: OrgId = org_id.parse().expect("org válida");
        self.store()
            .fluxos()
            .criar(
                org,
                NovoFluxo {
                    slug: slug.to_owned(),
                    nome: format!("Fluxo {slug}"),
                    descricao: String::new(),
                    rascunho_fonte: String::new(),
                },
            )
            .expect("cria o fluxo")
            .id
            .to_string()
    }

    fn versao_agente(&self, org_id: &str, agente_id: &str) -> u64 {
        let org: OrgId = org_id.parse().expect("org válida");
        let agente: AgentId = agente_id.parse().expect("agente válido");
        self.store()
            .agentes()
            .versao(org, agente)
            .expect("versão do agente")
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

/// Id do token (parte entre `poph_` e o segredo), para conferir o autor na auditoria.
fn id_do_token(token: &str) -> &str {
    token
        .strip_prefix("poph_")
        .expect("prefixo do token")
        .split_once('_')
        .expect("separador do token")
        .0
}

fn slugs(corpo: &Value) -> Vec<String> {
    corpo["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["slug"].as_str().expect("slug").to_owned())
        .collect()
}

#[tokio::test]
async fn agente_fluxos_lista_substitui_com_versao_teto_isolamento_e_auditoria() {
    let c = Cenario::novo("agente-fluxos").await;
    let oa = c.criar_org("Org A", "fx28-org-a").await;
    let ob = c.criar_org("Org B", "fx28-org-b").await;
    let ta = c.criar_token(&oa, "a").await;
    let tb = c.criar_token(&ob, "b").await;
    let ag = c.criar_agente(&oa, &ta, "agente-a").await;
    let agb = c.criar_agente(&ob, &tb, "agente-b").await;
    let f1 = c.criar_fluxo(&oa, "fluxo_a1");
    let f2 = c.criar_fluxo(&oa, "fluxo_a2");
    let fb = c.criar_fluxo(&ob, "fluxo_b1");
    let rota = format!("/admin/v1/orgs/{oa}/agents/{ag}/flows");

    // Lista vazia no começo.
    let (s, corpo) = c.pedir(Method::GET, &rota, &ta, None, None).await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["items"], json!([]));
    assert_eq!(corpo["next_cursor"], Value::Null);

    // Sem If-Match: 428. Com versão velha: 412. Nada muda nos dois casos.
    let (s, corpo) = c
        .pedir(
            Method::PUT,
            &rota,
            &ta,
            Some(json!({"flow_ids": [f1]})),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::PRECONDITION_REQUIRED, "{corpo}");
    let (s, corpo) = c
        .pedir(
            Method::PUT,
            &rota,
            &ta,
            Some(json!({"flow_ids": [f1]})),
            Some(99),
        )
        .await;
    assert_eq!(s, StatusCode::PRECONDITION_FAILED, "{corpo}");
    assert_eq!(codigo(&corpo), "version_conflict");
    assert_eq!(c.versao_agente(&oa, &ag), 1);

    // Libera dois: versão 1 → 2, os dois aparecem em `added`.
    let (s, corpo) = c
        .pedir(
            Method::PUT,
            &rota,
            &ta,
            Some(json!({"flow_ids": [f1, f2]})),
            Some(1),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["version"], 2);
    assert_eq!(corpo["added"].as_array().unwrap().len(), 2);
    assert_eq!(corpo["removed"], json!([]));
    let (s, lido) = c.pedir(Method::GET, &rota, &ta, None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(slugs(&lido), vec!["fluxo_a1", "fluxo_a2"]);

    // Troca para só um: f2 sai e aparece em `removed`.
    let (s, corpo) = c
        .pedir(
            Method::PUT,
            &rota,
            &ta,
            Some(json!({"flow_ids": [f1]})),
            Some(2),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["version"], 3);
    assert_eq!(corpo["added"], json!([]));
    assert_eq!(corpo["removed"][0]["id"], f2);

    // Lista vazia tira tudo.
    let (s, corpo) = c
        .pedir(
            Method::PUT,
            &rota,
            &ta,
            Some(json!({"flow_ids": []})),
            Some(3),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["version"], 4);
    assert_eq!(corpo["removed"][0]["id"], f1);
    let (_, lido) = c.pedir(Method::GET, &rota, &ta, None, None).await;
    assert_eq!(lido["items"], json!([]));

    // Mesma lista (vazia) de novo: ainda sobe a versão, como a doc diz.
    let (s, corpo) = c
        .pedir(
            Method::PUT,
            &rota,
            &ta,
            Some(json!({"flow_ids": []})),
            Some(4),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["version"], 5);
    assert_eq!(corpo["added"], json!([]));
    assert_eq!(corpo["removed"], json!([]));

    // Lote com fluxo de outra org: 404 e a lista inteira não muda (f1 não entra).
    let (s, corpo) = c
        .pedir(
            Method::PUT,
            &rota,
            &ta,
            Some(json!({"flow_ids": [f1, fb]})),
            Some(5),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{corpo}");
    assert_eq!(
        c.versao_agente(&oa, &ag),
        5,
        "versão não pode subir num 404"
    );
    let (_, lido) = c.pedir(Method::GET, &rota, &ta, None, None).await;
    assert_eq!(lido["items"], json!([]));

    // Fluxo inexistente: 404 também.
    let inexistente = FlowId::generate().to_string();
    let (s, corpo) = c
        .pedir(
            Method::PUT,
            &rota,
            &ta,
            Some(json!({"flow_ids": [inexistente]})),
            Some(5),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{corpo}");

    // Id que não é UUID e corpo sem `flow_ids`: 400, sem mudar nada.
    let (s, corpo) = c
        .pedir(
            Method::PUT,
            &rota,
            &ta,
            Some(json!({"flow_ids": ["abc"]})),
            Some(5),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{corpo}");
    assert_eq!(codigo(&corpo), "invalid_id");
    let (s, corpo) = c
        .pedir(Method::PUT, &rota, &ta, Some(json!({})), Some(5))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{corpo}");
    assert_eq!(codigo(&corpo), "invalid_request");
    assert_eq!(c.versao_agente(&oa, &ag), 5);

    // Teto: 20 passa, 21 recusa e a lista de 20 não é tocada.
    let muitos: Vec<String> = (1..=21)
        .map(|i| c.criar_fluxo(&oa, &format!("fluxo_t{i:02}")))
        .collect();
    let (s, corpo) = c
        .pedir(
            Method::PUT,
            &rota,
            &ta,
            Some(json!({"flow_ids": muitos})),
            Some(5),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{corpo}");
    assert_eq!(codigo(&corpo), "teto_de_fluxos_por_agente");
    assert_eq!(c.versao_agente(&oa, &ag), 5);
    let (s, corpo) = c
        .pedir(
            Method::PUT,
            &rota,
            &ta,
            Some(json!({"flow_ids": &muitos[..20]})),
            Some(5),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["version"], 6);
    assert_eq!(corpo["added"].as_array().unwrap().len(), 20);

    // Isolamento: token de B não lê nem grava na org A; token de A não alcança agente de B.
    let (s, corpo) = c.pedir(Method::GET, &rota, &tb, None, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{corpo}");
    let (s, corpo) = c
        .pedir(
            Method::PUT,
            &rota,
            &tb,
            Some(json!({"flow_ids": []})),
            Some(6),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{corpo}");
    let rota_b = format!("/admin/v1/orgs/{oa}/agents/{agb}/flows");
    let (s, corpo) = c.pedir(Method::GET, &rota_b, &ta, None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{corpo}");
    assert_eq!(c.versao_agente(&ob, &agb), 1, "agente de B não foi mexido");

    // Agente inativo continua aceitando permissão.
    let (s, corpo) = c
        .pedir(
            Method::PATCH,
            &format!("/admin/v1/orgs/{oa}/agents/{ag}"),
            &ta,
            Some(json!({ "status": "inactive" })),
            Some(c.versao_agente(&oa, &ag)),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    let (s, corpo) = c
        .pedir(
            Method::PUT,
            &rota,
            &ta,
            Some(json!({"flow_ids": [f1]})),
            Some(c.versao_agente(&oa, &ag)),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");

    // Auditoria: uma linha por PUT bem-sucedido (seis), com o autor do token e sem o segredo.
    let registros = c
        .store()
        .auditoria()
        .listar(oa.parse().unwrap(), Pagina::default())
        .unwrap();
    let do_agente: Vec<_> = registros
        .iter()
        .filter(|r| {
            r.acao == "agent_flows_update"
                && r.entidade == "agent"
                && r.entidade_id.as_deref() == Some(ag.as_str())
        })
        .collect();
    assert_eq!(do_agente.len(), 6, "um registro por PUT bem-sucedido");
    for registro in do_agente {
        assert_eq!(registro.ator_tipo, "admin_token");
        assert_eq!(registro.ator_id.as_deref(), Some(id_do_token(&ta)));
        assert!(!format!("{registro:?}").contains(&ta));
    }

    // Id repetido na lista conta uma vez: f1 já estava liberado, então nada entra nem sai, e a
    // leitura mostra o fluxo uma vez só.
    let (s, corpo) = c
        .pedir(
            Method::PUT,
            &rota,
            &ta,
            Some(json!({"flow_ids": [f1, f1]})),
            Some(c.versao_agente(&oa, &ag)),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["added"], json!([]), "{corpo}");
    assert_eq!(corpo["removed"], json!([]), "{corpo}");
    let (_, lido) = c.pedir(Method::GET, &rota, &ta, None, None).await;
    assert_eq!(slugs(&lido), vec!["fluxo_a1"]);
}
