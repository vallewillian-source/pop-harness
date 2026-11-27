//! Testes da MX-19: a falha definitiva da resposta humana aparece na sala, uma vez só.
//!
//! Banco em pasta temporária, o consumidor de verdade da fila de saída (`SaidaWhatsApp`), uma
//! **Evolution falsa** que responde o roteiro do teste e um **Synapse falso** que anota cada
//! evento escrito na sala (e pode fingir estar fora do ar). Nada de rede externa.
//!
//! O que se prova:
//! - falha definitiva vira **um** aviso na sala, em `m.notice`, com o motivo curto e em nome do
//!   fantasma do agente;
//! - passar de novo pela mesma falha não escreve nada;
//! - reinício do processo (um segundo `Store` sobre o mesmo arquivo, sem memória compartilhada)
//!   não repete o aviso: a marca fica no banco;
//! - falha transitória seguida de sucesso não gera aviso;
//! - Synapse fora do ar adia e a passagem seguinte avisa;
//! - conversa sem sala é marcada e não tenta nada;
//! - resposta que não é humana, e org com o módulo desligado, não geram aviso;
//! - o texto do cliente, o telefone e os tokens não aparecem no aviso nem no log.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{post, put};
use poph_agent::Backoff;
use poph_evolution::{ATTACHMENT_CAP, ConfigEvolution, EvolutionClient};
use poph_matrix::{ClientConfig, GhostIds, MatrixClient};
use poph_server::evolution::CanalEvolution;
use poph_server::matrix_falhas_aviso::{AvisoDeFalhas, AvisoNoSynapse, PortaFalhasDoHarness};
use poph_server::saida_whatsapp::{OpcoesSaidaWhatsApp, SaidaWhatsApp};
use poph_store::{
    HumanResponse, NewAgent, NewConversation, NewHumanResponse, NewOrg, NewOutboxItem, OutboxItem,
    OutboxState, OutcomeNotice, Store,
};
use poph_types::{AgentId, ConversationId, IntegrationId, OrgId, Secret, TurnId};
use serde_json::{Value, json};
use tracing_subscriber::fmt::MakeWriter;

// ───────────────────────────────── constantes ─────────────────────────────────

const PREFIXO: &str = "ph";
const SERVER_NAME: &str = "matrix-dev.local";
const INSTANCIA: &str = "loja";
const CHAVE_EVO: &str = "chave-evo-mx19-teste";

/// Número de cliente falso. Não é de ninguém.
const TELEFONE: &str = "5511999990000";
/// Texto do atendente: marcado, para provar que não vaza para o log.
const TEXTO_ATENDENTE: &str = "CANARIO-TEXTO-mx19-resposta-do-atendente";
/// Valor-canário do token que o Harness usa no Synapse: não pode aparecer em log.
const AS_TOKEN: &str = "CANARIO-AS-mx19-7d2e-TAO-SECRETO";

const SALA: &str = "!sala-mx19:matrix-dev.local";
const EVENTO: &str = "$evento-atendente-mx19:matrix-dev.local";
const ATENDENTE: &str = "@ana:matrix-dev.local";
const NOME_ATENDENTE: &str = "Ana Atendente";

/// Texto que o aviso deve levar quando a Evolution recusa de vez.
const AVISO_RECUSADO: &str =
    "Não consegui entregar sua mensagem ao cliente: o WhatsApp recusou o envio";
/// Texto do aviso quando as tentativas acabam (a fila grava a constante da MX-19).
const AVISO_ESGOTADO: &str = "Não consegui entregar sua mensagem ao cliente: o WhatsApp não respondeu depois de várias tentativas";

// ───────────────────────────────── captura de logs ─────────────────────────────────

#[derive(Clone)]
struct Captura(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captura {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Captura {
    type Writer = Captura;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Um coletor de logs para o processo de testes inteiro (o subscriber é global).
fn logs_capturados() -> Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let _ = tracing_subscriber::fmt()
            .with_writer(Captura(Arc::clone(&buffer)))
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .try_init();
        buffer
    })
    .clone()
}

// ───────────────────────────────── Synapse falso ─────────────────────────────────

/// Um evento que o Harness conseguiu escrever na sala.
#[derive(Clone, Debug)]
struct Escrita {
    room: String,
    tipo: String,
    txn: String,
    /// Fantasma em nome de quem foi escrito (`?user_id=`).
    como: Option<String>,
    body: Value,
}

#[derive(Clone, Default)]
struct SynapseFalso {
    escritas: Arc<Mutex<Vec<Escrita>>>,
    /// Quantas chamadas chegaram (inclusive as que foram recusadas de propósito).
    chamadas: Arc<AtomicUsize>,
    /// Quantas próximas chamadas devem falhar com `503`.
    falhar: Arc<Mutex<usize>>,
}

async fn enviar_falso(
    State(falso): State<SynapseFalso>,
    Path((room, tipo, txn)): Path<(String, String, String)>,
    Query(consulta): Query<HashMap<String, String>>,
    _cabecalhos: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    falso.chamadas.fetch_add(1, Ordering::SeqCst);
    let mut falhar = falso.falhar.lock().unwrap();
    if *falhar > 0 {
        *falhar -= 1;
        drop(falhar);
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "errcode": "M_UNKNOWN" })),
        );
    }
    let mut escritas = falso.escritas.lock().unwrap();
    escritas.push(Escrita {
        room,
        tipo,
        txn,
        como: consulta.get("user_id").cloned(),
        body,
    });
    let posicao = escritas.len();
    (
        StatusCode::OK,
        Json(json!({ "event_id": format!("$aviso-{posicao}:matrix-dev.local") })),
    )
}

async fn subir_synapse_falso() -> (String, SynapseFalso) {
    let falso = SynapseFalso::default();
    let roteador = Router::new()
        .route(
            "/_matrix/client/v3/rooms/{room}/send/{kind}/{txn}",
            put(enviar_falso),
        )
        .with_state(falso.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera do Synapse falso");
    let endereco = listener.local_addr().expect("endereço do Synapse falso");
    tokio::spawn(async move {
        let _ = axum::serve(listener, roteador).await;
    });
    (format!("http://{endereco}"), falso)
}

// ───────────────────────────────── Evolution falsa ─────────────────────────────────

#[derive(Clone)]
struct EvolucaoFalsa {
    roteiro: Arc<Mutex<VecDeque<(u16, Value)>>>,
}

async fn enviar_pelo_whatsapp(
    State(falsa): State<EvolucaoFalsa>,
    Path(_instancia): Path<String>,
    Json(_corpo): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let (status, resposta) = falsa
        .roteiro
        .lock()
        .unwrap()
        .pop_front()
        .unwrap_or((200, json!({ "key": { "id": "MSG-MX19" } })));
    (
        StatusCode::from_u16(status).expect("status do roteiro"),
        Json(resposta),
    )
}

async fn subir_evolution_falsa() -> (String, EvolucaoFalsa) {
    let falsa = EvolucaoFalsa {
        roteiro: Arc::new(Mutex::new(VecDeque::new())),
    };
    let roteador = Router::new()
        .route("/message/sendText/{instance}", post(enviar_pelo_whatsapp))
        .with_state(falsa.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera da Evolution falsa");
    let endereco = listener.local_addr().expect("endereço da Evolution falsa");
    tokio::spawn(async move {
        let _ = axum::serve(listener, roteador).await;
    });
    (format!("http://{endereco}"), falsa)
}

// ───────────────────────────────── cenário ─────────────────────────────────

struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agent: AgentId,
    integracao: IntegrationId,
    /// Conversa de WhatsApp com sala pronta no Synapse.
    conversation: ConversationId,
    synapse: SynapseFalso,
    url_synapse: String,
    evo: EvolucaoFalsa,
    saida: SaidaWhatsApp,
}

impl Cenario {
    async fn new(rotulo: &str) -> Self {
        Self::novo_com_tentativas(rotulo, OpcoesSaidaWhatsApp::default().max_tentativas).await
    }

    /// Igual a [`Cenario::novo`], com o limite de tentativas da fila escolhido pelo teste.
    async fn novo_com_tentativas(rotulo: &str, max_tentativas: u32) -> Self {
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-matrix-falhas-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("pasta do teste");
        let store = Arc::new(
            Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre o banco"),
        );

        let org = criar_org(&store, "loja-falhas", "Loja Falhas");
        let agent = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Atendente Falhas".to_owned(),
                    slug: "atendente-falhas".to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        let agent_id: AgentId = agent;
        let integracao: IntegrationId = store
            .create_integration(org, agent_id, "whatsapp".to_owned(), "active".to_owned())
            .expect("cria a integração")
            .id
            .parse()
            .expect("id da integração");
        store.matrix().enable(org).expect("liga o módulo");
        let conversation = conversa_whatsapp(&store, org, agent_id, integracao, TELEFONE);
        store
            .matrix()
            .ensure_room(org, conversation, SALA, None)
            .expect("grava a sala");
        store
            .matrix()
            .mark_room_ready(org, conversation)
            .expect("sala pronta");

        let (url_synapse, synapse) = subir_synapse_falso().await;
        let (url_evo, evo) = subir_evolution_falsa().await;
        let client = EvolutionClient::new(
            ConfigEvolution::new(url_evo, INSTANCIA, Secret::new(CHAVE_EVO.to_owned()))
                .expect("config da Evolution falsa"),
        )
        .expect("cliente da Evolution falsa");
        let canais = vec![CanalEvolution {
            org,
            instancia: INSTANCIA.to_owned(),
            integracao,
            segredo: None,
            cliente: Some(client),
            teto_midia: ATTACHMENT_CAP,
        }];
        let opcoes = OpcoesSaidaWhatsApp {
            // Espera curta para a nova tentativa não depender de relógio.
            backoff: Backoff {
                base: Duration::from_millis(1),
                factor: 2,
                cap: Duration::from_millis(5),
            },
            max_tentativas,
            ..OpcoesSaidaWhatsApp::default()
        };
        let saida = SaidaWhatsApp::novo(Arc::clone(&store), &canais, pasta.join("blobs"), opcoes);
        logs_capturados();
        Self {
            pasta,
            store,
            org,
            agent: agent_id,
            integracao,
            conversation,
            synapse,
            url_synapse,
            evo,
            saida,
        }
    }

    /// Uma conversa nova de WhatsApp na org do cenário, com ou sem sala.
    fn outra_conversa(&self, com_sala: bool) -> ConversationId {
        let conversation = conversa_whatsapp(
            &self.store,
            self.org,
            self.agent,
            self.integracao,
            "5511888880000",
        );
        if com_sala {
            self.store
                .matrix()
                .ensure_room(self.org, conversation, "!outra-mx19:matrix-dev.local", None)
                .expect("grava a sala");
            self.store
                .matrix()
                .mark_room_ready(self.org, conversation)
                .expect("sala pronta");
        }
        conversation
    }

    /// O atendente escreve na sala: a resposta entra na fila de saída.
    fn resposta_humana(&self, conversation: ConversationId) {
        let fresh = NewHumanResponse {
            event_id: &format!("{EVENTO}-{conversation}"),
            raw_text: TEXTO_ATENDENTE,
            sent_text: &format!("**[{NOME_ATENDENTE}]**:\n{TEXTO_ATENDENTE}"),
            author_id: ATENDENTE,
            author_name: NOME_ATENDENTE,
        };
        match self
            .store
            .messages()
            .register_response_human(self.org, conversation, fresh)
            .expect("grava a resposta humana")
        {
            HumanResponse::Stored(_) => {}
            outro => panic!("a resposta humana devia ser gravada, veio {outro:?}"),
        }
    }

    /// Item de saída de agente (não humano) para o teste de "não é resposta humana".
    fn saida_do_agente(&self, conversation: ConversationId) -> OutboxItem {
        let (_, item) = self
            .store
            .outbox()
            .enqueue(
                self.org,
                NewOutboxItem {
                    conversation_id: conversation,
                    turn_id: TurnId::generate(),
                    seq: 0,
                    tipo: "text".to_owned(),
                    content_json: json!({ "texto": "resposta do agente" }).to_string(),
                },
            )
            .expect("enfileira a saída do agente");
        item
    }

    /// Roda uma passagem do consumidor do WhatsApp com o roteiro da Evolution falsa.
    async fn ciclo(&self, roteiro: Vec<(u16, Value)>) {
        *self.evo.roteiro.lock().unwrap() = roteiro.into_iter().collect();
        self.saida
            .ciclo()
            .await
            .expect("ciclo da saída do WhatsApp");
    }

    /// O serviço de aviso, como um processo novo o montaria (sem nada na memória).
    fn aviso(&self, store: Arc<Store>) -> AvisoDeFalhas {
        let client = MatrixClient::new(
            ClientConfig::new(
                self.url_synapse.clone(),
                SERVER_NAME.to_owned(),
                Secret::new(AS_TOKEN.to_owned()),
            )
            .expect("config do cliente do Matrix"),
        )
        .expect("cliente do Matrix");
        let ids = GhostIds::new(PREFIXO, SERVER_NAME).expect("ids dos fantasmas");
        AvisoDeFalhas::new(
            Arc::new(PortaFalhasDoHarness::fresh(store)),
            AvisoNoSynapse::new(client, ids),
        )
    }

    fn escritas(&self) -> Vec<Escrita> {
        self.synapse.escritas.lock().unwrap().clone()
    }

    fn state(&self, state: OutboxState) -> u64 {
        self.store
            .outbox()
            .count_by_state(self.org, state)
            .expect("conta a fila")
    }

    fn agent_ghost(&self) -> String {
        GhostIds::new(PREFIXO, SERVER_NAME)
            .expect("ids")
            .agent(&self.agent.to_string())
            .expect("fantasma do agente")
            .user_id()
            .to_owned()
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn criar_org(store: &Store, slug: &str, name: &str) -> OrgId {
    store
        .orgs()
        .create(NewOrg {
            name: name.to_owned(),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id
}

fn conversa_whatsapp(
    store: &Store,
    org: OrgId,
    agent: AgentId,
    integracao: IntegrationId,
    telefone: &str,
) -> ConversationId {
    store
        .conversations()
        .create(
            org,
            NewConversation {
                agent_id: agent,
                integration_id: Some(integracao),
                canal: "whatsapp".to_owned(),
                peer_externo: Some(telefone.to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa")
        .id
}

fn texto_do_aviso(escrita: &Escrita) -> String {
    escrita
        .body
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

// ───────────────────────────────── testes ─────────────────────────────────

#[tokio::test]
async fn falha_definitiva_da_resposta_humana_avisa_a_sala_uma_vez() {
    let cenario = Cenario::new("aviso-uma-vez").await;
    cenario.resposta_humana(cenario.conversation);

    // O canal recusa de vez (4xx): a fila grava `failed`.
    cenario
        .ciclo(vec![(400, json!({ "message": "número inválido" }))])
        .await;
    assert_eq!(
        cenario.state(OutboxState::Failed),
        1,
        "a fila marcou a falha"
    );

    let relato = cenario.aviso(Arc::clone(&cenario.store)).passar().await;
    assert_eq!(relato.avisados, 1, "um aviso escrito");

    let escritas = cenario.escritas();
    assert_eq!(escritas.len(), 1, "uma escrita na sala");
    let aviso = &escritas[0];
    assert_eq!(aviso.room, SALA, "o aviso vai para a sala da conversa");
    assert_eq!(aviso.tipo, "m.room.message");
    assert_eq!(
        aviso.body["msgtype"], "m.notice",
        "aviso é m.notice, para não voltar como resposta"
    );
    assert_eq!(texto_do_aviso(aviso), AVISO_RECUSADO);
    assert_eq!(
        aviso.como.as_deref(),
        Some(cenario.agent_ghost().as_str()),
        "escrito em nome do fantasma do agente, que está na sala"
    );
    assert!(aviso.txn.starts_with("ph_falha_"));

    // Mesma falha, nova passagem: nada a escrever, e nem chega ao Synapse.
    let chamadas_antes = cenario.synapse.chamadas.load(Ordering::SeqCst);
    let segundo = cenario.aviso(Arc::clone(&cenario.store)).passar().await;
    assert_eq!(
        segundo,
        Default::default(),
        "a segunda passagem não faz nada"
    );
    assert_eq!(
        cenario.synapse.chamadas.load(Ordering::SeqCst),
        chamadas_antes
    );
    assert_eq!(cenario.escritas().len(), 1, "continua um aviso só");
}

#[tokio::test]
async fn reinicio_do_processo_nao_repete_o_aviso() {
    let cenario = Cenario::new("reinicio").await;
    cenario.resposta_humana(cenario.conversation);
    cenario
        .ciclo(vec![(400, json!({ "message": "recusado" }))])
        .await;

    {
        let antes = cenario.aviso(Arc::clone(&cenario.store)).passar().await;
        assert_eq!(antes.avisados, 1);
    }
    assert_eq!(cenario.escritas().len(), 1);

    // Processo novo: outro `Store` sobre o mesmo arquivo, sem nada na memória do serviço antigo.
    let reaberto = Arc::new(
        Store::open_at(
            &cenario.pasta.join("harness.db"),
            &cenario.pasta.join("rag.db"),
        )
        .expect("reabre o banco"),
    );
    let depois = cenario.aviso(reaberto).passar().await;
    assert_eq!(
        depois,
        Default::default(),
        "a marca do banco segura o aviso"
    );
    assert_eq!(
        cenario.escritas().len(),
        1,
        "nenhum segundo aviso depois do reinício"
    );
}

#[tokio::test]
async fn laco_de_fundo_avisa_sozinho_e_para_no_pedido() {
    let cenario = Cenario::new("laco").await;
    cenario.resposta_humana(cenario.conversation);
    cenario
        .ciclo(vec![(400, json!({ "message": "recusado" }))])
        .await;

    let aviso = Arc::new(cenario.aviso(Arc::clone(&cenario.store)));
    let tarefa = tokio::spawn(Arc::clone(&aviso).executar(Duration::from_millis(10)));

    // O laço escreve o aviso sem ninguém chamar `passar`.
    let inicio = std::time::Instant::now();
    while cenario.escritas().is_empty() {
        assert!(
            inicio.elapsed() < Duration::from_secs(10),
            "o laço não avisou"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(cenario.escritas().len(), 1);

    // Pedido de parada: o laço termina sem esperar o próximo intervalo.
    aviso.pedir_parada();
    tokio::time::timeout(Duration::from_secs(5), tarefa)
        .await
        .expect("o laço para no pedido")
        .expect("tarefa do laço sem pânico");
    assert_eq!(cenario.escritas().len(), 1, "continua um aviso só");
}

#[tokio::test]
async fn falha_transitoria_seguida_de_sucesso_nao_gera_aviso() {
    let cenario = Cenario::new("transitoria").await;
    cenario.resposta_humana(cenario.conversation);

    // Primeira tentativa: 5xx, que é transitório. O item volta para a fila (`sending`).
    cenario
        .ciclo(vec![(500, json!({ "message": "erro interno" }))])
        .await;
    assert_eq!(
        cenario.state(OutboxState::Sending),
        1,
        "a falha ainda não é definitiva"
    );
    assert_eq!(cenario.state(OutboxState::Failed), 0);

    let no_meio = cenario.aviso(Arc::clone(&cenario.store)).passar().await;
    assert_eq!(no_meio, Default::default(), "falha transitória não avisa");
    assert!(cenario.escritas().is_empty());

    // A tentativa seguinte dá certo.
    tokio::time::sleep(Duration::from_millis(20)).await;
    cenario
        .ciclo(vec![(200, json!({ "key": { "id": "MSG-OK" } }))])
        .await;
    assert_eq!(cenario.state(OutboxState::Sent), 1, "o cliente recebeu");

    let depois = cenario.aviso(Arc::clone(&cenario.store)).passar().await;
    assert_eq!(depois, Default::default());
    assert!(
        cenario.escritas().is_empty(),
        "nenhum aviso quando a entrega deu certo"
    );
}

#[tokio::test]
async fn tentativas_esgotadas_avisam_com_o_motivo_proprio() {
    // Uma tentativa só: a primeira falha transitória já esgota e vira `failed`.
    let cenario = Cenario::novo_com_tentativas("esgotado", 1).await;
    cenario.resposta_humana(cenario.conversation);
    cenario
        .ciclo(vec![(500, json!({ "message": "erro interno" }))])
        .await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    cenario.ciclo(vec![]).await;
    assert_eq!(
        cenario.state(OutboxState::Failed),
        1,
        "esgotou as tentativas"
    );

    let relato = cenario.aviso(Arc::clone(&cenario.store)).passar().await;
    assert_eq!(relato.avisados, 1);
    let escritas = cenario.escritas();
    assert_eq!(texto_do_aviso(&escritas[0]), AVISO_ESGOTADO);
}

#[tokio::test]
async fn synapse_fora_do_ar_adia_e_a_passagem_seguinte_avisa() {
    let cenario = Cenario::new("synapse-fora").await;
    cenario.resposta_humana(cenario.conversation);
    cenario
        .ciclo(vec![(400, json!({ "message": "recusado" }))])
        .await;

    // O Synapse recusa o primeiro aviso com 503.
    *cenario.synapse.falhar.lock().unwrap() = 1;
    let primeira = cenario.aviso(Arc::clone(&cenario.store)).passar().await;
    assert_eq!(
        primeira.adiados, 1,
        "erro transitório: fica para a próxima passagem"
    );
    assert_eq!(primeira.avisados, 0);
    assert!(cenario.escritas().is_empty());

    // Sem marca no banco, a passagem seguinte tenta de novo e escreve.
    let segunda = cenario.aviso(Arc::clone(&cenario.store)).passar().await;
    assert_eq!(segunda.avisados, 1);
    assert_eq!(cenario.escritas().len(), 1);
}

#[tokio::test]
async fn conversa_sem_sala_e_marcada_e_nao_tenta_nada() {
    let cenario = Cenario::new("sem-sala").await;
    let conversation = cenario.outra_conversa(false);
    cenario.resposta_humana(conversation);
    cenario
        .ciclo(vec![(400, json!({ "message": "recusado" }))])
        .await;

    let relato = cenario.aviso(Arc::clone(&cenario.store)).passar().await;
    assert_eq!(relato.sem_sala, 1);
    assert_eq!(
        cenario.synapse.chamadas.load(Ordering::SeqCst),
        0,
        "nada chegou ao Synapse"
    );

    let segundo = cenario.aviso(Arc::clone(&cenario.store)).passar().await;
    assert_eq!(
        segundo,
        Default::default(),
        "a marca impede de consultar de novo"
    );
}

#[tokio::test]
async fn saida_que_nao_e_resposta_humana_nao_gera_aviso() {
    let cenario = Cenario::new("nao-humana").await;
    // Um item de saída do agente (não é mensagem do atendente) que falha de vez.
    let _ = cenario.saida_do_agente(cenario.conversation);
    cenario
        .ciclo(vec![(400, json!({ "message": "recusado" }))])
        .await;
    assert_eq!(cenario.state(OutboxState::Failed), 1);

    let relato = cenario.aviso(Arc::clone(&cenario.store)).passar().await;
    assert_eq!(relato, Default::default(), "só resposta humana é avisada");
    assert!(cenario.escritas().is_empty());
}

#[tokio::test]
async fn org_com_o_modulo_desligado_nao_recebe_aviso() {
    let cenario = Cenario::new("desligado").await;
    cenario.resposta_humana(cenario.conversation);
    cenario
        .ciclo(vec![(400, json!({ "message": "recusado" }))])
        .await;

    cenario
        .store
        .matrix()
        .disable(cenario.org)
        .expect("desliga o módulo");
    let relato = cenario.aviso(Arc::clone(&cenario.store)).passar().await;
    assert_eq!(
        relato,
        Default::default(),
        "módulo desligado: nenhuma passagem"
    );
    assert!(cenario.escritas().is_empty());
}

#[tokio::test]
async fn texto_do_cliente_telefone_e_token_nao_aparecem_no_aviso_nem_no_log() {
    let cenario = Cenario::new("canario").await;
    cenario.resposta_humana(cenario.conversation);
    cenario
        .ciclo(vec![(
            400,
            json!({ "message": "recusado para 5511999990000" }),
        )])
        .await;

    let logs = logs_capturados();
    logs.lock().unwrap().clear();
    let _ = cenario.aviso(Arc::clone(&cenario.store)).passar().await;

    let escritas = cenario.escritas();
    assert_eq!(escritas.len(), 1);
    let aviso = texto_do_aviso(&escritas[0]);
    assert!(!aviso.contains(TELEFONE), "telefone não vai à sala");
    assert!(
        !aviso.contains(TEXTO_ATENDENTE),
        "texto do atendente não vai à sala"
    );

    let saida = String::from_utf8_lossy(&logs.lock().unwrap()).into_owned();
    assert!(
        saida.contains("aviso de falha de resposta humana escrito na sala"),
        "o log existe"
    );
    for canario in [TELEFONE, TEXTO_ATENDENTE, AS_TOKEN, "recusado para"] {
        assert!(!saida.contains(canario), "log não pode conter `{canario}`");
    }
}

#[test]
fn desfecho_do_aviso_e_idempotente_e_isolado_por_org() {
    let pasta = std::env::temp_dir().join(format!(
        "poph-matrix-falhas-desfecho-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(&pasta).expect("pasta");
    let store = Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("banco");
    let org = criar_org(&store, "loja-desfecho", "Loja Desfecho");
    let agent = store
        .agents()
        .create(
            org,
            NewAgent {
                name: "A".to_owned(),
                slug: "a".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("agente")
        .id;
    let integracao: IntegrationId = store
        .create_integration(org, agent, "whatsapp".to_owned(), "active".to_owned())
        .expect("integração")
        .id
        .parse()
        .expect("id da integração");
    let conversation = conversa_whatsapp(&store, org, agent, integracao, TELEFONE);
    let (_, item) = store
        .outbox()
        .enqueue(
            org,
            NewOutboxItem {
                conversation_id: conversation,
                turn_id: TurnId::generate(),
                seq: 0,
                tipo: "text".to_owned(),
                content_json: "{}".to_owned(),
            },
        )
        .expect("enfileira");

    let primeira = store
        .matrix()
        .record_notice_outcome(org, item.id, OutcomeNotice::Notify, "recusado_pelo_canal")
        .expect("grava");
    let segunda = store
        .matrix()
        .record_notice_outcome(org, item.id, OutcomeNotice::Notify, "recusado_pelo_canal")
        .expect("grava de novo");
    assert!(primeira, "a primeira gravação é nova");
    assert!(!segunda, "a segunda não muda nada");

    // Outra org não grava desfecho para um item que não é dela.
    let outra = criar_org(&store, "loja-desfecho-b", "Loja B");
    let de_outra = store
        .matrix()
        .record_notice_outcome(outra, item.id, OutcomeNotice::Refused, "nao_informado")
        .expect("grava");
    assert!(!de_outra, "item de outra org não ganha desfecho");
    let falhas_da_outra = store
        .matrix()
        .human_response_failures(outra, 10)
        .expect("lista");
    assert!(
        falhas_da_outra.is_empty(),
        "a outra org não enxerga a falha"
    );
    let _ = fs::remove_dir_all(&pasta);
}
