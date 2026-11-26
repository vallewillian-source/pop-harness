//! Dados da conversa que o Element usa (RP-01), na API administrativa, com banco e servidor HTTP
//! de verdade. Não há Synapse aqui: a sala entra pelo repositório do módulo, e o que a API devolve
//! é o que se confere.
//!
//! O que se prova:
//! - detalhe, lista, fechar e reabrir trazem `client_name` (nome do cliente, `null` quando vazio),
//!   `agent_name` e `matrix_room_id` (só com a sala `ready` e o módulo ligado na org);
//! - sala em `creating` e módulo desligado (nunca ligado, ou desligado depois) viram `null`;
//! - a lista lê os nomes dos agentes e as salas em lote: o número de leituras de uma página não
//!   cresce com o tamanho dela (teste que morde com uma consulta por linha);
//! - conversa de outra org continua `404` no detalhe, sem vazar o nome do cliente dela.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::time::Duration;

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use axum::http::StatusCode;
use poph_server::Estado;
use poph_store::{Agente, Conversa, NovaConversa, NovaOrg, NovoAgente, Org, Store};
use poph_types::{AgentId, ConversationId, HarnessConfig, OrgId};
use rand_core::OsRng;
use reqwest::{Client, Method};
use serde_json::Value;
use tokio::sync::oneshot;

/// Banco, configuração e relógio de um teste; a pasta é apagada no fim.
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
        let store = Store::abrir(&config)
            .expect("abre o banco")
            .com_relogio(Arc::new(move || relogio.fetch_add(10, Ordering::SeqCst)));
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
            .orgaos()
            .criar(NovaOrg {
                nome: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
    }

    /// Token de org no formato que a API autentica: `poph_<id>_<segredo>`.
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

    fn agente(&self, org: &Org, slug: &str, nome: &str) -> Agente {
        self.store
            .agentes()
            .criar(
                org.id,
                NovoAgente {
                    nome: nome.to_owned(),
                    slug: slug.to_owned(),
                    ..NovoAgente::default()
                },
            )
            .expect("cria o agente")
    }

    fn conversa(&self, org: &Org, agente: AgentId) -> Conversa {
        self.store
            .conversas()
            .criar(
                org.id,
                NovaConversa {
                    agent_id: agente,
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some("5511999999999".to_owned()),
                    departamento_alvo: None,
                },
            )
            .expect("cria a conversa")
    }

    /// Sala `ready` para a conversa (ou `creating`, quando `pronta` é falso).
    fn sala(&self, org: &Org, conversa: ConversationId, room: &str, pronta: bool) {
        self.store
            .matrix()
            .garantir_sala(org.id, conversa, room, None)
            .expect("garante a sala");
        if pronta {
            self.store
                .matrix()
                .marcar_sala_pronta(org.id, conversa)
                .expect("sala pronta");
        }
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

/// Sobe o router numa porta efêmera.
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

/// Lista uma página de conversas da org e devolve os itens.
async fn lista(base: &str, org: OrgId, token: &str, limite: u32) -> Vec<Value> {
    let resposta = pedir(
        base,
        Method::GET,
        &format!("/admin/v1/orgs/{org}/conversations?limit={limite}"),
        token,
        None,
    )
    .await;
    assert_eq!(resposta.status, StatusCode::OK, "lista: {}", resposta.texto);
    resposta.json["items"]
        .as_array()
        .cloned()
        .expect("items da lista")
}

fn item(itens: &[Value], id: ConversationId) -> &Value {
    itens
        .iter()
        .find(|v| v["id"] == id.to_string())
        .expect("conversa na lista")
}

#[tokio::test]
async fn detalhe_fechar_reabrir_e_lista_trazem_nome_agente_e_sala_pronta() {
    let c = Cenario::novo("basico");
    let org = c.org("a");
    let token = c.token(org.id, "a");
    let agente = c.agente(&org, "bia", "Bia");
    let conversa = c.conversa(&org, agente.id);
    c.store
        .conversas()
        .definir_nome_cliente(org.id, conversa.id, "Ana Souza")
        .expect("nome do cliente");
    c.store.matrix().ligar(org.id).expect("liga o módulo");
    c.sala(&org, conversa.id, "!ana:matrix-dev.local", true);
    let (base, desligar) = servir(c.estado()).await;

    let caminho = format!("/admin/v1/orgs/{}/conversations/{}", org.id, conversa.id);
    let detalhe = pedir(&base, Method::GET, &caminho, &token, None).await;
    assert_eq!(detalhe.status, StatusCode::OK, "{}", detalhe.texto);
    assert_eq!(detalhe.json["client_name"], "Ana Souza");
    assert_eq!(detalhe.json["agent_name"], "Bia");
    assert_eq!(detalhe.json["matrix_room_id"], "!ana:matrix-dev.local");

    let itens = lista(&base, org.id, &token, 50).await;
    let na_lista = item(&itens, conversa.id);
    assert_eq!(na_lista["client_name"], "Ana Souza");
    assert_eq!(na_lista["agent_name"], "Bia");
    assert_eq!(na_lista["matrix_room_id"], "!ana:matrix-dev.local");

    let fechada = pedir(
        &base,
        Method::POST,
        &format!("{caminho}/close"),
        &token,
        Some("1"),
    )
    .await;
    assert_eq!(fechada.status, StatusCode::OK, "{}", fechada.texto);
    assert_eq!(fechada.json["client_name"], "Ana Souza");
    assert_eq!(fechada.json["matrix_room_id"], "!ana:matrix-dev.local");
    assert_eq!(fechada.json["agent_name"], "Bia");

    // FX-07: conversa `closed` não reabre mais (PRD L13) — o caminho é a mensagem nova do cliente,
    // que abre outra conversa e outra Ficha. O que este teste prova (o `reopen` devolve nome e sala)
    // passa a ser verificado na conversa `transferred`, que continua reabrindo.
    let encerrada = pedir(
        &base,
        Method::POST,
        &format!("{caminho}/reopen"),
        &token,
        Some("2"),
    )
    .await;
    assert_eq!(
        encerrada.status,
        StatusCode::CONFLICT,
        "{}",
        encerrada.texto
    );
    assert_eq!(encerrada.json["code"], "conversa_encerrada");

    c.store
        .conversas()
        .mudar_status(
            org.id,
            conversa.id,
            c.store
                .conversas()
                .obter(org.id, conversa.id)
                .unwrap()
                .versao,
            poph_store::StatusConversa::Transferida,
            None,
        )
        .expect("transfere para humano");
    let versao_transferida = c
        .store
        .conversas()
        .obter(org.id, conversa.id)
        .unwrap()
        .versao
        .to_string();
    let reaberta = pedir(
        &base,
        Method::POST,
        &format!("{caminho}/reopen"),
        &token,
        Some(&versao_transferida),
    )
    .await;
    assert_eq!(reaberta.status, StatusCode::OK, "{}", reaberta.texto);
    assert_eq!(reaberta.json["matrix_room_id"], "!ana:matrix-dev.local");
    assert_eq!(reaberta.json["client_name"], "Ana Souza");

    let _ = desligar.send(());
}

#[tokio::test]
async fn sala_em_criacao_nome_vazio_e_modulo_desligado_viram_nulo() {
    let c = Cenario::novo("nulos");
    // Três orgs, cada uma com um caso, para que um desligamento não mascare o outro:
    // A ligada (sala ainda em criação, sem nome); B nunca ligada (sala pronta, com nome);
    // C ligada e depois desligada (sala pronta, com nome).
    let a = c.org("a");
    let b = c.org("b");
    let d = c.org("c");
    let token_a = c.token(a.id, "a");
    let token_b = c.token(b.id, "b");
    let token_c = c.token(d.id, "c");
    let agente_a = c.agente(&a, "bia", "Bia");
    let agente_b = c.agente(&b, "bia", "Bia");
    let agente_c = c.agente(&d, "bia", "Bia");

    c.store.matrix().ligar(a.id).expect("liga A");
    let sem_nome = c.conversa(&a, agente_a.id);
    c.sala(&a, sem_nome.id, "!criando:matrix-dev.local", false);

    let nunca_ligada = c.conversa(&b, agente_b.id);
    c.store
        .conversas()
        .definir_nome_cliente(b.id, nunca_ligada.id, "Bruno Lima")
        .expect("nome do cliente");
    c.sala(&b, nunca_ligada.id, "!pronta-b:matrix-dev.local", true);

    c.store.matrix().ligar(d.id).expect("liga C");
    c.store.matrix().desligar(d.id).expect("desliga C");
    let desligada = c.conversa(&d, agente_c.id);
    c.store
        .conversas()
        .definir_nome_cliente(d.id, desligada.id, "Carla Dias")
        .expect("nome do cliente");
    c.sala(&d, desligada.id, "!pronta-c:matrix-dev.local", true);

    let (base, desligar) = servir(c.estado()).await;

    // A: sala em creating não sai; nome vazio vira null; o agente aparece.
    let caminho = format!("/admin/v1/orgs/{}/conversations/{}", a.id, sem_nome.id);
    let detalhe = pedir(&base, Method::GET, &caminho, &token_a, None).await;
    assert_eq!(detalhe.status, StatusCode::OK, "{}", detalhe.texto);
    assert_eq!(
        detalhe.json["client_name"],
        Value::Null,
        "nome vazio é null"
    );
    assert_eq!(
        detalhe.json["matrix_room_id"],
        Value::Null,
        "sala em creating não sai"
    );
    assert_eq!(detalhe.json["agent_name"], "Bia");
    let itens = lista(&base, a.id, &token_a, 50).await;
    assert_eq!(item(&itens, sem_nome.id)["matrix_room_id"], Value::Null);
    assert_eq!(item(&itens, sem_nome.id)["client_name"], Value::Null);

    // B: nunca ligada, sala pronta e nome: a sala não sai.
    let caminho = format!("/admin/v1/orgs/{}/conversations/{}", b.id, nunca_ligada.id);
    let detalhe = pedir(&base, Method::GET, &caminho, &token_b, None).await;
    assert_eq!(detalhe.status, StatusCode::OK, "{}", detalhe.texto);
    assert_eq!(detalhe.json["client_name"], "Bruno Lima");
    assert_eq!(
        detalhe.json["matrix_room_id"],
        Value::Null,
        "módulo nunca ligado na org: sala não sai"
    );
    let itens = lista(&base, b.id, &token_b, 50).await;
    assert_eq!(item(&itens, nunca_ligada.id)["matrix_room_id"], Value::Null);

    // C: ligada e desligada de novo, sala pronta: a sala não sai, o nome continua.
    let caminho = format!("/admin/v1/orgs/{}/conversations/{}", d.id, desligada.id);
    let detalhe = pedir(&base, Method::GET, &caminho, &token_c, None).await;
    assert_eq!(detalhe.status, StatusCode::OK, "{}", detalhe.texto);
    assert_eq!(detalhe.json["client_name"], "Carla Dias");
    assert_eq!(
        detalhe.json["matrix_room_id"],
        Value::Null,
        "módulo desligado depois de ligado: sala não sai"
    );
    let itens = lista(&base, d.id, &token_c, 50).await;
    assert_eq!(item(&itens, desligada.id)["matrix_room_id"], Value::Null);
    assert_eq!(item(&itens, desligada.id)["client_name"], "Carla Dias");

    let _ = desligar.send(());
}

#[tokio::test]
async fn lista_le_nomes_e_salas_em_lote_sem_consulta_por_linha() {
    let c = Cenario::novo("lote");
    let org = c.org("a");
    let token = c.token(org.id, "a");
    c.store.matrix().ligar(org.id).expect("liga");
    // Trinta conversas, cada uma com o seu agente: uma consulta por linha seria trinta leituras.
    for n in 0..30 {
        let agente = c.agente(&org, &format!("ag-{n}"), &format!("Agente {n}"));
        let conversa = c.conversa(&org, agente.id);
        c.store
            .conversas()
            .definir_nome_cliente(org.id, conversa.id, &format!("Cliente {n}"))
            .expect("nome");
        if n % 2 == 0 {
            c.sala(
                &org,
                conversa.id,
                &format!("!sala{n}:matrix-dev.local"),
                true,
            );
        }
    }
    let (base, desligar) = servir(c.estado()).await;

    // Mede as leituras que cada página gasta; a página pequena e a grande têm de gastar o mesmo.
    let antes = c.store.leituras_feitas();
    let pequena = lista(&base, org.id, &token, 2).await;
    let leituras_pequena = c.store.leituras_feitas() - antes;

    let antes = c.store.leituras_feitas();
    let grande = lista(&base, org.id, &token, 30).await;
    let leituras_grande = c.store.leituras_feitas() - antes;

    assert_eq!(pequena.len(), 2);
    assert_eq!(grande.len(), 30);
    assert_eq!(
        leituras_pequena, leituras_grande,
        "a página de 30 gastou {leituras_grande} leituras e a de 2 gastou {leituras_pequena}: \
         a lista não pode fazer uma consulta por linha"
    );
    assert!(
        leituras_grande <= 5,
        "a página inteira devia gastar poucas leituras, gastou {leituras_grande}"
    );

    // Os dados chegam corretos em toda a página: nome de agente em cada item, sala só nas prontas.
    for valor in &grande {
        assert!(valor["agent_name"].is_string(), "agent_name em todo item");
        let nome_cliente = valor["client_name"].as_str().expect("client_name");
        let numero: usize = nome_cliente
            .trim_start_matches("Cliente ")
            .parse()
            .expect("número do cliente");
        if numero.is_multiple_of(2) {
            assert_eq!(
                valor["matrix_room_id"],
                format!("!sala{numero}:matrix-dev.local")
            );
        } else {
            assert_eq!(valor["matrix_room_id"], Value::Null);
        }
    }

    let _ = desligar.send(());
}

#[tokio::test]
async fn conversa_de_outra_org_continua_404_e_nao_vaza_o_nome() {
    let c = Cenario::novo("isolamento");
    let a = c.org("a");
    let b = c.org("b");
    let token_a = c.token(a.id, "a");
    let agente_b = c.agente(&b, "bia", "Bia da B");
    let conversa_b = c.conversa(&b, agente_b.id);
    c.store
        .conversas()
        .definir_nome_cliente(b.id, conversa_b.id, "Cliente Secreto B")
        .expect("nome");
    c.store.matrix().ligar(b.id).expect("liga a B");
    c.sala(&b, conversa_b.id, "!secreta:matrix-dev.local", true);
    let (base, desligar) = servir(c.estado()).await;

    // Token da A pedindo a conversa da B pelo caminho da A: não existe ali.
    let pela_a = pedir(
        &base,
        Method::GET,
        &format!("/admin/v1/orgs/{}/conversations/{}", a.id, conversa_b.id),
        &token_a,
        None,
    )
    .await;
    assert_eq!(pela_a.status, StatusCode::NOT_FOUND, "{}", pela_a.texto);
    assert!(
        !pela_a.texto.contains("Cliente Secreto B") && !pela_a.texto.contains("matrix-dev.local"),
        "o 404 não pode trazer dado da outra org"
    );

    // Token da A pelo caminho da B: recusado antes de qualquer leitura.
    let pelo_caminho_b = pedir(
        &base,
        Method::GET,
        &format!("/admin/v1/orgs/{}/conversations/{}", b.id, conversa_b.id),
        &token_a,
        None,
    )
    .await;
    assert_eq!(pelo_caminho_b.status, StatusCode::FORBIDDEN);

    let _ = desligar.send(());
}
