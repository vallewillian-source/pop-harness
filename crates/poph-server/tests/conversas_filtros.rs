
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::time::Duration;

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use axum::http::StatusCode;
use poph_server::ServerState;
use poph_store::{
    Agente, Conversa, ConversationStatus, NewAgent, NewConversation, NewOrg, Org, Store,
};
use poph_types::{AgentId, HarnessConfig, OrgId};
use rand_core::OsRng;
use reqwest::{Client, Method};
use serde_json::Value;
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
            "poph-conversas-element-{rotulo}-{}-{n}",
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

    fn estado(&self) -> ServerState {
        ServerState::new(Arc::clone(&self.store), self.config.clone()).expect("estado do servidor")
    }

    fn org(&self, slug: &str) -> Org {
        self.store
            .orgs()
            .create(NewOrg {
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
            .create(Some(org), rotulo, &hash)
            .expect("cria o token");
        format!("poph_{}_{segredo}", criado.id)
    }

    fn agente(&self, org: &Org, slug: &str, nome: &str) -> Agente {
        self.store
            .agents()
            .create(
                org.id,
                NewAgent {
                    name: nome.to_owned(),
                    slug: slug.to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

async fn servir(estado: ServerState) -> (String, oneshot::Sender<()>) {
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
    if_match: Option<&str>,
) -> Resposta {
    let cliente = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("cliente HTTP");
    let mut pedido = cliente
        .request(metodo, format!("{base}{caminho}"))
        .bearer_auth(token);
    if let Some(valor) = if_match {
        pedido = pedido.header("If-Match", valor);
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

fn conversa_do_cliente(c: &Cenario, org: &Org, agente: AgentId, peer: &str) -> Conversa {
    c.store
        .conversations()
        .create(
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

fn itens_de(resposta: &Resposta) -> Vec<Value> {
    assert_eq!(resposta.status, StatusCode::OK, "{}", resposta.texto);
    resposta.json["items"]
        .as_array()
        .cloned()
        .expect("items da lista")
}

fn ids_de(itens: &[Value]) -> Vec<String> {
    itens
        .iter()
        .map(|v| v["id"].as_str().unwrap_or_default().to_owned())
        .collect()
}

fn caminho_lista(org: OrgId) -> String {
    format!("/admin/v1/orgs/{org}/conversations")
}

#[tokio::test]
async fn filtro_por_cliente_e_situacao_ficam_dentro_da_org() {
    let c = Cenario::novo("filtros");
    let a = c.org("a");
    let b = c.org("b");
    let token_a = c.token(a.id, "a");
    let agente_a = c.agente(&a, "bia", "Bia");
    let agente_b = c.agente(&b, "leo", "Leo");
    let c1 = conversa_do_cliente(&c, &a, agente_a.id, "5511");
    let c2 = conversa_do_cliente(&c, &a, agente_a.id, "5511");
    let c3 = conversa_do_cliente(&c, &a, agente_a.id, "5522");
    let d1 = conversa_do_cliente(&c, &b, agente_b.id, "5511");
    c.store
        .conversations()
        .transfer(a.id, c2.id, None, None, "{}".to_owned())
        .expect("transfere c2");
    c.store
        .conversations()
        .change_status(a.id, c3.id, 1, ConversationStatus::Closed, None)
        .expect("fecha c3");
    let (base, desligar) = servir(c.estado()).await;
    let lista = caminho_lista(a.id);

    let do_cliente = pedir(
        &base,
        Method::GET,
        &format!("{lista}?external_peer=5511"),
        &token_a,
        None,
    )
    .await;
    let itens = itens_de(&do_cliente);
    assert_eq!(
        ids_de(&itens),
        vec![c1.id.to_string(), c2.id.to_string()],
        "só as duas conversas do cliente, na org A"
    );
    assert!(
        !ids_de(&itens).contains(&d1.id.to_string()),
        "a conversa do mesmo cliente na org B não aparece"
    );

    let transferidas = pedir(
        &base,
        Method::GET,
        &format!("{lista}?status=transferred"),
        &token_a,
        None,
    )
    .await;
    assert_eq!(ids_de(&itens_de(&transferidas)), vec![c2.id.to_string()]);

    let os_dois = pedir(
        &base,
        Method::GET,
        &format!("{lista}?external_peer=5511&status=open"),
        &token_a,
        None,
    )
    .await;
    assert_eq!(
        ids_de(&itens_de(&os_dois)),
        vec![c1.id.to_string()],
        "cliente e situação juntos"
    );

    let fechadas = pedir(
        &base,
        Method::GET,
        &format!("{lista}?status=closed"),
        &token_a,
        None,
    )
    .await;
    assert_eq!(ids_de(&itens_de(&fechadas)), vec![c3.id.to_string()]);

    let sem_conversa = pedir(
        &base,
        Method::GET,
        &format!("{lista}?external_peer=9999"),
        &token_a,
        None,
    )
    .await;
    assert!(itens_de(&sem_conversa).is_empty());

    let _ = desligar.send(());
}

#[tokio::test]
async fn sort_recent_ordena_pela_atividade_e_devolve_next_cursor_nulo() {
    let c = Cenario::novo("recente");
    let org = c.org("a");
    let token = c.token(org.id, "a");
    let agente = c.agente(&org, "bia", "Bia");
    let c1 = conversa_do_cliente(&c, &org, agente.id, "5511");
    let c2 = conversa_do_cliente(&c, &org, agente.id, "5522");
    let c3 = conversa_do_cliente(&c, &org, agente.id, "5533");
    c.store
        .conversations()
        .touch(org.id, c1.id, 9_000_000)
        .expect("toca c1");
    c.store
        .conversations()
        .touch(org.id, c3.id, 9_000_000)
        .expect("toca c3");
    let (base, desligar) = servir(c.estado()).await;
    let lista = caminho_lista(org.id);

    let ordenadas = pedir(
        &base,
        Method::GET,
        &format!("{lista}?sort=recent"),
        &token,
        None,
    )
    .await;
    assert_eq!(ordenadas.status, StatusCode::OK, "{}", ordenadas.texto);
    assert_eq!(
        ids_de(&itens_de(&ordenadas)),
        vec![c3.id.to_string(), c1.id.to_string(), c2.id.to_string()]
    );
    assert!(
        ordenadas.json["next_cursor"].is_null(),
        "sort=recent não usa cursor: next_cursor vem nulo"
    );

    let duas = pedir(
        &base,
        Method::GET,
        &format!("{lista}?sort=recent&limit=2"),
        &token,
        None,
    )
    .await;
    assert_eq!(
        ids_de(&itens_de(&duas)),
        vec![c3.id.to_string(), c1.id.to_string()]
    );
    assert!(duas.json["next_cursor"].is_null());

    let cursor_vazio = pedir(
        &base,
        Method::GET,
        &format!("{lista}?sort=recent&cursor="),
        &token,
        None,
    )
    .await;
    assert_eq!(
        cursor_vazio.status,
        StatusCode::OK,
        "{}",
        cursor_vazio.texto
    );

    let do_cliente = pedir(
        &base,
        Method::GET,
        &format!("{lista}?sort=recent&external_peer=5522"),
        &token,
        None,
    )
    .await;
    assert_eq!(ids_de(&itens_de(&do_cliente)), vec![c2.id.to_string()]);

    let _ = desligar.send(());
}

#[tokio::test]
async fn sort_recent_recusa_cursor_e_limite_fora_da_faixa_com_400() {
    let c = Cenario::novo("recente-400");
    let org = c.org("a");
    let token = c.token(org.id, "a");
    let agente = c.agente(&org, "bia", "Bia");
    let conversa = conversa_do_cliente(&c, &org, agente.id, "5511");
    let (base, desligar) = servir(c.estado()).await;
    let lista = caminho_lista(org.id);

    let com_cursor = pedir(
        &base,
        Method::GET,
        &format!("{lista}?sort=recent&cursor={}", conversa.id),
        &token,
        None,
    )
    .await;
    assert_eq!(
        com_cursor.status,
        StatusCode::BAD_REQUEST,
        "cursor com sort=recent é recusado, não ignorado: {}",
        com_cursor.texto
    );

    for limite in ["0", "201"] {
        let fora = pedir(
            &base,
            Method::GET,
            &format!("{lista}?sort=recent&limit={limite}"),
            &token,
            None,
        )
        .await;
        assert_eq!(
            fora.status,
            StatusCode::BAD_REQUEST,
            "limit={limite} com sort=recent: {}",
            fora.texto
        );
    }

    let _ = desligar.send(());
}

#[tokio::test]
async fn valor_invalido_nos_filtros_devolve_400_sem_repetir_o_valor() {
    let c = Cenario::novo("invalidos");
    let org = c.org("a");
    let token = c.token(org.id, "a");
    let (base, desligar) = servir(c.estado()).await;
    let lista = caminho_lista(org.id);
    const ESTRANHO: &str = "xyzzy-valor-estranho";

    for consulta in [
        format!("{lista}?status={ESTRANHO}"),
        format!("{lista}?external_peer=%20%20"),
        format!("{lista}?external_peer="),
        format!("{lista}?sort={ESTRANHO}"),
        format!("{lista}?sort=recent&limit=0"),
    ] {
        let resposta = pedir(&base, Method::GET, &consulta, &token, None).await;
        assert_eq!(
            resposta.status,
            StatusCode::BAD_REQUEST,
            "{consulta}: {}",
            resposta.texto
        );
        assert!(
            !resposta.texto.contains(ESTRANHO),
            "o valor recebido não volta na mensagem de erro: {}",
            resposta.texto
        );
    }

    let _ = desligar.send(());
}

#[tokio::test]
async fn sem_sort_a_lista_segue_por_id_com_cursor_e_limit_ajustado() {
    let c = Cenario::novo("sem-sort");
    let org = c.org("a");
    let token = c.token(org.id, "a");
    let agente = c.agente(&org, "bia", "Bia");
    let ids: Vec<String> = (0..5)
        .map(|n| {
            conversa_do_cliente(&c, &org, agente.id, &format!("55{n}"))
                .id
                .to_string()
        })
        .collect();
    let (base, desligar) = servir(c.estado()).await;
    let lista = caminho_lista(org.id);

    let primeira = pedir(
        &base,
        Method::GET,
        &format!("{lista}?limit=2"),
        &token,
        None,
    )
    .await;
    assert_eq!(ids_de(&itens_de(&primeira)), ids[..2]);
    let cursor = primeira.json["next_cursor"]
        .as_str()
        .expect("página cheia traz next_cursor")
        .to_owned();
    assert_eq!(cursor, ids[1], "o cursor é o último id da página");

    let segunda = pedir(
        &base,
        Method::GET,
        &format!("{lista}?limit=2&cursor={cursor}"),
        &token,
        None,
    )
    .await;
    assert_eq!(ids_de(&itens_de(&segunda)), ids[2..4]);

    let ultima = pedir(
        &base,
        Method::GET,
        &format!("{lista}?limit=2&cursor={}", ids[3]),
        &token,
        None,
    )
    .await;
    assert_eq!(ids_de(&itens_de(&ultima)), ids[4..5]);
    assert!(ultima.json["next_cursor"].is_null());

    let grande = pedir(
        &base,
        Method::GET,
        &format!("{lista}?limit=500"),
        &token,
        None,
    )
    .await;
    assert_eq!(itens_de(&grande).len(), 5);

    let do_cliente = pedir(
        &base,
        Method::GET,
        &format!("{lista}?external_peer=550&limit=1"),
        &token,
        None,
    )
    .await;
    assert_eq!(ids_de(&itens_de(&do_cliente)), ids[..1]);

    let _ = desligar.send(());
}
