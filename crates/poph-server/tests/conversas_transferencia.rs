use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::time::Duration;

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use axum::http::StatusCode;
use poph_server::Estado;
use poph_store::{Agente, Conversa, NewAgent, NewConversation, NewDepartment, NewOrg, Org, Store};
use poph_types::{AgentId, HarnessConfig, OrgId};
use rand_core::OsRng;
use reqwest::{Client, Method};
use serde_json::{Value, json};
use tokio::sync::oneshot;

struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    config: HarnessConfig,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta = std::env::temp_dir().join(format!(
            "poph-conversas-transferencia-{rotulo}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");
        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        config.validar().expect("configuração válida");
        let relogio = Arc::new(AtomicI64::new(1_000_000));
        let store = Store::open(&config)
            .expect("abre o banco")
            .with_clock(Arc::new(move || relogio.fetch_add(10, Ordering::SeqCst)));
        Self {
            pasta,
            store: Arc::new(store),
            config,
        }
    }

    fn estado(&self) -> Estado {
        Estado::novo(Arc::clone(&self.store), self.config.clone()).expect("estado do servidor")
    }

    fn org(&self, slug: &str) -> Org {
        self.store
            .orgs()
            .criar(NewOrg {
                name: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
    }

    fn token(&self, org: OrgId, rotulo: &str) -> String {
        let segredo = format!("segredo-{rotulo}");
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(segredo.as_bytes(), &salt)
            .expect("hash argon2")
            .to_string();
        let criado = self
            .store
            .tokens()
            .criar(Some(org), rotulo, &hash)
            .expect("cria o token");
        format!("poph_{}_{segredo}", criado.id)
    }

    fn agente(&self, org: &Org, slug: &str) -> Agente {
        self.store
            .agents()
            .criar(
                org.id,
                NewAgent {
                    name: format!("Agente {slug}"),
                    slug: slug.to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
    }

    fn departamento(&self, org: &Org, nome: &str) -> poph_types::DepartmentId {
        self.store
            .departments()
            .criar(
                org.id,
                NewDepartment {
                    name: nome.to_owned(),
                    description: String::new(),
                },
            )
            .expect("cria o departamento")
            .id
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

async fn servir(estado: Estado) -> (String, oneshot::Sender<()>) {
    let app = poph_server::router(estado);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = format!("http://{}", listener.local_addr().expect("endereço"));
    let (desligar, sinal) = oneshot::channel();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = sinal.await;
            })
            .await;
    });
    (endereco, desligar)
}

struct Resposta {
    status: StatusCode,
    json: Value,
    texto: String,
}

async fn pedir(
    base: &str,
    metodo: Method,
    caminho: &str,
    token: &str,
    corpo: Option<Value>,
) -> Resposta {
    let cliente = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("cliente HTTP");
    let mut pedido = cliente
        .request(metodo, format!("{base}{caminho}"))
        .bearer_auth(token);
    if let Some(corpo) = corpo {
        pedido = pedido.json(&corpo);
    }
    let resposta = pedido.send().await.expect("pedido HTTP");
    let status = resposta.status();
    let texto = resposta.text().await.unwrap_or_default();
    let json = serde_json::from_str(&texto).unwrap_or(Value::Null);
    Resposta {
        status,
        json,
        texto,
    }
}

fn conversa(c: &Cenario, org: &Org, agente: AgentId, peer: &str) -> Conversa {
    c.store
        .conversas()
        .criar(
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

fn caminho(org: OrgId, conversa: impl std::fmt::Display, acao: &str) -> String {
    format!("/admin/v1/orgs/{org}/conversations/{conversa}/{acao}")
}

#[tokio::test]
async fn transferir_marca_a_conversa_e_audita_o_motivo() {
    let c = Cenario::novo("feliz");
    let org = c.org("a");
    let token = c.token(org.id, "a");
    let agente = c.agente(&org, "bia");
    let destino = c.departamento(&org, "Comercial");
    let alvo = conversa(&c, &org, agente.id, "5511");
    let (base, desligar) = servir(c.estado()).await;

    let resposta = pedir(
        &base,
        Method::POST,
        &caminho(org.id, alvo.id, "transfer"),
        &token,
        Some(json!({ "department_id": destino.to_string(), "reason": "rotina" })),
    )
    .await;
    assert_eq!(resposta.status, StatusCode::OK, "{}", resposta.texto);
    assert_eq!(resposta.json["id"], alvo.id.to_string());
    assert_eq!(resposta.json["status"], "transferred");
    assert_eq!(resposta.json["target_department_id"], destino.to_string());

    let gravada = c
        .store
        .conversas()
        .get(org.id, alvo.id)
        .expect("lê a conversa");
    assert_eq!(gravada.status, poph_store::ConversationStatus::Transferred);
    assert_eq!(gravada.target_department, Some(destino));
    assert_eq!(gravada.version, alvo.version + 1);

    let auditoria = c
        .store
        .audit()
        .listar(org.id, poph_store::Page::with_limit(10))
        .expect("lê a auditoria");
    let registro = auditoria
        .iter()
        .find(|registro| registro.acao == "transfer")
        .expect("a transferência é auditada");
    assert_eq!(registro.entity, "conversation");
    assert_eq!(
        registro.entity_id.as_deref(),
        Some(alvo.id.to_string().as_str())
    );
    assert_eq!(
        registro.detail_json,
        json!({
            "motivo": "rotina",
            "ator": "administrador",
            "department_id": destino.to_string(),
        })
        .to_string()
    );

    let _ = desligar.send(());
}

#[tokio::test]
async fn transferir_recusa_conversa_encerrada_da_outra_org_e_corpo_invalido() {
    let c = Cenario::novo("recusas");
    let org = c.org("a");
    let outra = c.org("b");
    let token = c.token(org.id, "a");
    let token_b = c.token(outra.id, "b");
    let agente = c.agente(&org, "bia");
    let agente_b = c.agente(&outra, "leo");
    let destino = c.departamento(&org, "Comercial");
    let destino_b = c.departamento(&outra, "Comercial");
    let alvo = conversa(&c, &org, agente.id, "5511");
    let da_outra = conversa(&c, &outra, agente_b.id, "5522");
    let encerrada = conversa(&c, &org, agente.id, "5533");
    c.store
        .conversas()
        .fechar_pelo_agente(org.id, encerrada.id, agente.id, "fim".to_owned())
        .expect("fecha a conversa");
    let (base, desligar) = servir(c.estado()).await;

    let corpo = json!({ "department_id": destino.to_string(), "reason": "rotina" });
    let resposta = pedir(
        &base,
        Method::POST,
        &caminho(org.id, da_outra.id, "transfer"),
        &token,
        Some(corpo.clone()),
    )
    .await;
    assert_eq!(resposta.status, StatusCode::NOT_FOUND, "{}", resposta.texto);
    assert_eq!(resposta.json["code"], "not_found");

    let resposta = pedir(
        &base,
        Method::POST,
        &caminho(outra.id, alvo.id, "transfer"),
        &token_b,
        Some(json!({ "department_id": destino_b.to_string(), "reason": "rotina" })),
    )
    .await;
    assert_eq!(resposta.status, StatusCode::NOT_FOUND, "{}", resposta.texto);

    let resposta = pedir(
        &base,
        Method::POST,
        &caminho(org.id, encerrada.id, "transfer"),
        &token,
        Some(corpo.clone()),
    )
    .await;
    assert_eq!(resposta.status, StatusCode::CONFLICT, "{}", resposta.texto);
    assert_eq!(resposta.json["code"], "conversa_encerrada");

    for (rotulo, corpo_ruim) in [
        (
            "sem motivo",
            json!({ "department_id": destino.to_string() }),
        ),
        (
            "motivo vazio",
            json!({ "department_id": destino.to_string(), "reason": "  " }),
        ),
        (
            "departamento malformado",
            json!({ "department_id": "nao-e-id", "reason": "rotina" }),
        ),
    ] {
        let resposta = pedir(
            &base,
            Method::POST,
            &caminho(org.id, alvo.id, "transfer"),
            &token,
            Some(corpo_ruim),
        )
        .await;
        assert_eq!(
            resposta.status,
            StatusCode::BAD_REQUEST,
            "{rotulo}: {}",
            resposta.texto
        );
        assert_eq!(resposta.json["code"], "validation_error", "{rotulo}");
    }

    let resposta = pedir(
        &base,
        Method::POST,
        &caminho(org.id, alvo.id, "transfer"),
        &token,
        Some(json!("nao-e-objeto")),
    )
    .await;
    assert_eq!(
        resposta.status,
        StatusCode::BAD_REQUEST,
        "{}",
        resposta.texto
    );
    assert_eq!(resposta.json["code"], "invalid_request");

    let gravada = c
        .store
        .conversas()
        .get(org.id, alvo.id)
        .expect("lê a conversa");
    assert_eq!(gravada.status, poph_store::ConversationStatus::Open);
    assert_eq!(gravada.version, alvo.version);

    let _ = desligar.send(());
}

#[tokio::test]
async fn a_lista_corta_pela_ultima_atividade_e_recusa_valor_invalido() {
    let c = Cenario::novo("inatividade");
    let org = c.org("a");
    let outra = c.org("b");
    let token = c.token(org.id, "a");
    let agente = c.agente(&org, "bia");
    let agente_b = c.agente(&outra, "leo");
    let primeira = conversa(&c, &org, agente.id, "5511");
    let segunda = conversa(&c, &org, agente.id, "5522");
    let terceira = conversa(&c, &org, agente.id, "5533");
    let alheia = conversa(&c, &outra, agente_b.id, "5511");
    let (base, desligar) = servir(c.estado()).await;

    let corte = segunda.created_at + 1;
    let lista = format!(
        "/admin/v1/orgs/{}/conversations?sort=recent&limit=50&last_activity_before={corte}",
        org.id
    );
    let resposta = pedir(&base, Method::GET, &lista, &token, None).await;
    assert_eq!(resposta.status, StatusCode::OK, "{}", resposta.texto);
    let ids: Vec<&str> = resposta.json["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["id"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(ids, vec![segunda.id.to_string(), primeira.id.to_string()]);
    assert!(!ids.contains(&terceira.id.to_string().as_str()));
    assert!(!ids.contains(&alheia.id.to_string().as_str()));

    let resposta = pedir(
        &base,
        Method::GET,
        &format!(
            "/admin/v1/orgs/{}/conversations?last_activity_before=ontem",
            org.id
        ),
        &token,
        None,
    )
    .await;
    assert_eq!(
        resposta.status,
        StatusCode::BAD_REQUEST,
        "{}",
        resposta.texto
    );
    assert_eq!(resposta.json["code"], "invalid_request");

    let _ = desligar.send(());
}
