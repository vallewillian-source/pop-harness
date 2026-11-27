//! Testes do espelho de estado (MX-15).
//!
//! Banco de verdade (`Store` numa pasta temporária, relógio controlado, para o par do cursor ser
//! previsível) e **Synapse falso** em memória: o duplo conta cada aviso, cada metadata e cada
//! movimento de espaço, e pode ser posto "fora do ar" para provar que a passagem não consome a
//! conversa em silêncio.
//!
//! O que se prova:
//! - `open→transferred`, `transferred→open` e `*→closed` escrevem **um** aviso e **um** metadata;
//! - repetir a passagem com a mesma situação não escreve nada (nem aviso, nem metadata);
//! - conversa que reabre no meio da passagem não recebe o metadata atrasado (nem `closed` nem
//!   `active` fora de hora);
//! - mudança de departamento move a sala de espaço, sem aviso duplicado;
//! - falha transitória não avança o cursor e a passagem seguinte aplica o estado, com o mesmo
//!   `txnId` (o aviso não duplica);
//! - duas conversas alteradas no mesmo milissegundo não se perdem (cursor `(updated_at, id)`);
//! - conversa sem sala e sala ainda em criação não geram aviso;
//! - org com o módulo desligado não recebe chamada nenhuma.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use poph_matrix::error::CallError;
use poph_matrix::rooms::RoomMetadata;
use poph_matrix::spaces::{Position, RoomForSpace, SpaceError};
use poph_server::matrix_state::{
    ConversationChanged, ConversationState, ConversationStates, OrgState, PAGE, RoomState,
    StateError, StatePort, StateStorePort, StateTarget, StoredCursor,
};
use poph_store::{ConversationStatus, NewAgent, NewConversation, NewDepartment, NewOrg, Store};
use poph_types::{AgentId, ConversationId, DepartmentId, MatrixConfig, OrgId};
use serde_json::Value;

// ───────────────────────────── cenário ─────────────────────────────

/// Relógio do banco: anda 1 ms a cada leitura ou fica parado, quando o teste congela.
struct RelogioDeTeste {
    agora: AtomicI64,
    congelado: std::sync::atomic::AtomicBool,
}

impl RelogioDeTeste {
    fn agora(&self) -> i64 {
        if self.congelado.load(Ordering::SeqCst) {
            return self.agora.load(Ordering::SeqCst);
        }
        self.agora.fetch_add(1, Ordering::SeqCst)
    }
}

/// Banco de verdade numa pasta temporária, com relógio que o teste controla.
struct Cenario {
    pasta: PathBuf,
    store: Option<Arc<Store>>,
    relogio: Arc<RelogioDeTeste>,
}

impl Cenario {
    fn new(rotulo: &str) -> Self {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta =
            std::env::temp_dir().join(format!("poph-estado-{rotulo}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("cria a pasta do teste");
        let relogio = Arc::new(RelogioDeTeste {
            agora: AtomicI64::new(1_000_000),
            congelado: std::sync::atomic::AtomicBool::new(false),
        });
        let pulso = Arc::clone(&relogio);
        let store = Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db"))
            .expect("abre o banco")
            .with_clock(Arc::new(move || pulso.agora()));
        Self {
            pasta,
            store: Some(Arc::new(store)),
            relogio,
        }
    }

    fn store(&self) -> Arc<Store> {
        Arc::clone(self.store.as_ref().expect("store viva"))
    }

    /// Congela o relógio num instante: as escritas seguintes caem no mesmo milissegundo.
    fn congelar(&self, em: i64) {
        self.relogio.agora.store(em, Ordering::SeqCst);
        self.relogio.congelado.store(true, Ordering::SeqCst);
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn config() -> MatrixConfig {
    MatrixConfig {
        enabled: true,
        homeserver_url: "http://127.0.0.1:1".to_owned(),
        server_name: "matrix-dev.local".to_owned(),
        prefix: "ph".to_owned(),
        concurrency: 4,
        interval_ms: 500,
        ..MatrixConfig::default()
    }
}

fn criar_org(store: &Store, slug: &str) -> OrgId {
    store
        .orgs()
        .create(NewOrg {
            name: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id
}

fn criar_departamento(store: &Store, org: OrgId, name: &str) -> DepartmentId {
    store
        .departments()
        .create(
            org,
            NewDepartment {
                name: name.to_owned(),
                description: String::new(),
            },
        )
        .expect("cria o departamento")
        .id
}

fn criar_agente(
    store: &Store,
    org: OrgId,
    slug: &str,
    departamento_padrao: Option<DepartmentId>,
) -> AgentId {
    store
        .agents()
        .create(
            org,
            NewAgent {
                name: "Bia".to_owned(),
                slug: slug.to_owned(),
                default_department: departamento_padrao,
                ..NewAgent::default()
            },
        )
        .expect("cria o agente")
        .id
}

fn criar_conversa(store: &Store, org: OrgId, agent: AgentId, peer: &str) -> ConversationId {
    store
        .conversations()
        .create(
            org,
            NewConversation {
                agent_id: agent,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: Some(peer.to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa")
        .id
}

/// Grava a sala da conversa já pronta (o que a MX-11 faz ao terminar a criação).
fn sala_pronta(store: &Store, org: OrgId, conversation: ConversationId) {
    store
        .matrix()
        .ensure_room(org, conversation, &sala_de(conversation), None)
        .expect("grava a sala");
    store
        .matrix()
        .mark_room_ready(org, conversation)
        .expect("marca pronta");
}

/// Grava a sala ainda em criação (queda no meio da MX-11).
fn sala_em_criacao(store: &Store, org: OrgId, conversation: ConversationId) {
    store
        .matrix()
        .ensure_room(org, conversation, &sala_de(conversation), None)
        .expect("grava a sala");
}

fn sala_de(conversation: ConversationId) -> String {
    format!("!{conversation}:matrix-dev.local")
}

/// Muda a situação da conversa pelo caminho de verdade (transfere, fecha, reabre).
fn definir_status(
    store: &Store,
    org: OrgId,
    conversation: ConversationId,
    status: ConversationStatus,
) {
    let atual = store
        .conversations()
        .get(org, conversation)
        .expect("conversa existe");
    match status {
        ConversationStatus::Open => {
            store
                .conversations()
                .reopen_audited(org, conversation, atual.version, "teste".to_owned())
                .expect("reabre");
        }
        ConversationStatus::Closed => {
            store
                .conversations()
                .close_audited(org, conversation, atual.version, "teste".to_owned())
                .expect("fecha");
        }
        ConversationStatus::Transferred => {
            store
                .conversations()
                .transfer(org, conversation, None, None, "{}".to_owned())
                .expect("transfere");
        }
    }
}

// ───────────────────────────── Synapse falso ─────────────────────────────

/// Um aviso que o módulo escreveu na sala.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Notice {
    room: String,
    txn: String,
    agent: AgentId,
    text: String,
}

/// Um metadata que o módulo reescreveu na sala.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MetadataEscrita {
    room: String,
    body: Value,
}

/// Uma renomeação de sala (`m.room.name`) escrita no Synapse falso (MX-33).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Renomeacao {
    room: String,
    name: String,
}

/// Uma troca de nome de exibição do fantasma do cliente (MX-33).
#[derive(Debug, Clone, PartialEq, Eq)]
struct FantasmaRenomeado {
    client: String,
    name: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Modo {
    /// Aceita tudo.
    Normal,
    /// A primeira escrita falha de forma transitória (Synapse indisponível).
    FalhaNaPrimeira,
    /// A n-ésima escrita (1 = a próxima) falha de forma transitória, uma vez só.
    FalhaNaEscrita(usize),
    /// Toda escrita falha de forma definitiva (recusa de contrato).
    Recusa,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Espaco {
    /// A sala já está no espaço do departamento.
    JaNoLugar,
    /// A sala mudou de espaço agora.
    Moveu,
}

struct Falso {
    avisos: Mutex<Vec<Notice>>,
    metadatas: Mutex<Vec<MetadataEscrita>>,
    renomeacoes: Mutex<Vec<Renomeacao>>,
    fantasmas: Mutex<Vec<FantasmaRenomeado>>,
    posicoes: Mutex<Vec<RoomForSpace>>,
    modo: Mutex<Modo>,
    space: Mutex<Espaco>,
    escritas: AtomicUsize,
}

impl Falso {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            avisos: Mutex::new(Vec::new()),
            metadatas: Mutex::new(Vec::new()),
            renomeacoes: Mutex::new(Vec::new()),
            fantasmas: Mutex::new(Vec::new()),
            posicoes: Mutex::new(Vec::new()),
            modo: Mutex::new(Modo::Normal),
            space: Mutex::new(Espaco::JaNoLugar),
            escritas: AtomicUsize::new(0),
        })
    }

    fn definir_modo(&self, modo: Modo) {
        *self.modo.lock().unwrap() = modo;
    }

    fn definir_espaco(&self, space: Espaco) {
        *self.space.lock().unwrap() = space;
    }

    fn avisos(&self) -> Vec<Notice> {
        self.avisos.lock().unwrap().clone()
    }

    fn metadatas(&self) -> Vec<MetadataEscrita> {
        self.metadatas.lock().unwrap().clone()
    }

    fn renomeacoes(&self) -> Vec<Renomeacao> {
        self.renomeacoes.lock().unwrap().clone()
    }

    fn fantasmas(&self) -> Vec<FantasmaRenomeado> {
        self.fantasmas.lock().unwrap().clone()
    }

    fn posicoes(&self) -> Vec<RoomForSpace> {
        self.posicoes.lock().unwrap().clone()
    }

    fn escritas(&self) -> usize {
        self.escritas.load(Ordering::SeqCst)
    }

    /// A escrita falha conforme o modo; a falha pedida acontece uma vez só.
    fn falhar(&self) -> Option<CallError> {
        let modo = *self.modo.lock().unwrap();
        let escrita = self.escritas.fetch_add(1, Ordering::SeqCst) + 1;
        match modo {
            Modo::Normal => None,
            Modo::FalhaNaEscrita(alvo) if escrita != alvo => None,
            Modo::FalhaNaEscrita(_) | Modo::FalhaNaPrimeira => {
                *self.modo.lock().unwrap() = Modo::Normal;
                Some(CallError::Response {
                    operation: "send",
                    status: 503,
                    errcode: Some("M_UNKNOWN".to_owned()),
                    message: "indisponível".to_owned(),
                    retry_after_ms: None,
                })
            }
            Modo::Recusa => Some(CallError::Response {
                operation: "send",
                status: 403,
                errcode: Some("M_FORBIDDEN".to_owned()),
                message: "recusado".to_owned(),
                retry_after_ms: None,
            }),
        }
    }
}

/// O duplo compartilhado com o teste: o laço fica com uma cópia do `Arc`, o teste com outra.
struct Sinal(Arc<Falso>);

impl StateTarget for Sinal {
    async fn notify(
        &self,
        room: &str,
        agent: AgentId,
        txn: &str,
        text: &str,
    ) -> Result<(), CallError> {
        if let Some(error) = self.0.falhar() {
            return Err(error);
        }
        self.0.avisos.lock().unwrap().push(Notice {
            room: room.to_owned(),
            txn: txn.to_owned(),
            agent,
            text: text.to_owned(),
        });
        Ok(())
    }

    async fn write_metadata(
        &self,
        room: &str,
        _agente: AgentId,
        metadata: &RoomMetadata,
    ) -> Result<(), CallError> {
        if let Some(error) = self.0.falhar() {
            return Err(error);
        }
        let body = serde_json::to_value(metadata).expect("metadata serializa");
        self.0.metadatas.lock().unwrap().push(MetadataEscrita {
            room: room.to_owned(),
            body,
        });
        Ok(())
    }

    async fn rename(&self, room: &str, _agente: AgentId, name: &str) -> Result<(), CallError> {
        if let Some(error) = self.0.falhar() {
            return Err(error);
        }
        self.0.renomeacoes.lock().unwrap().push(Renomeacao {
            room: room.to_owned(),
            name: name.to_owned(),
        });
        Ok(())
    }

    async fn rename_ghost(&self, client: &str, name: &str) -> Result<(), CallError> {
        if let Some(error) = self.0.falhar() {
            return Err(error);
        }
        self.0.fantasmas.lock().unwrap().push(FantasmaRenomeado {
            client: client.to_owned(),
            name: name.to_owned(),
        });
        Ok(())
    }

    async fn place(&self, dados: RoomForSpace) -> Result<Position, SpaceError> {
        self.0.posicoes.lock().unwrap().push(dados.clone());
        match *self.0.space.lock().unwrap() {
            Espaco::JaNoLugar => Ok(Position::AlreadyPositioned {
                space: "!espaco:matrix-dev.local".to_owned(),
            }),
            Espaco::Moveu => Ok(Position::Positioned {
                space: "!espaco:matrix-dev.local".to_owned(),
                moved: true,
                guests: 0,
                failed_invitations: 0,
            }),
        }
    }
}

fn laco(store: Arc<Store>, destino: Arc<Falso>) -> ConversationStates<Sinal, StateStorePort> {
    ConversationStates::new(StateStorePort::fresh(store), Sinal(destino), &config())
        .expect("monta o laço")
}

// ───────────────────────────── cenário padrão ─────────────────────────────

/// Org com o módulo ligado, um agente e uma conversa aberta com a sala pronta.
struct Mundo {
    agent: AgentId,
    conversation: ConversationId,
}

fn mundo(store: &Store, org: OrgId) -> Mundo {
    store.matrix().enable(org).expect("liga o módulo");
    let agent = criar_agente(store, org, "bia", None);
    let conversation = criar_conversa(store, org, agent, "5511999990000");
    sala_pronta(store, org, conversation);
    Mundo {
        agent,
        conversation,
    }
}

// ───────────────────────────── testes ─────────────────────────────

#[tokio::test]
async fn transferencia_avisa_e_reescreve_o_metadata() {
    let cenario = Cenario::new("transferencia");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::new();
    let state = laco(Arc::clone(&store), Arc::clone(&destino));

    definir_status(
        &store,
        org,
        mundo.conversation,
        ConversationStatus::Transferred,
    );
    let relatorio = state.cycle().await.expect("passagem");

    assert_eq!(relatorio.notified, 1, "{relatorio:?}");
    let avisos = destino.avisos();
    assert_eq!(avisos.len(), 1);
    assert_eq!(avisos[0].room, sala_de(mundo.conversation));
    assert_eq!(avisos[0].agent, mundo.agent);
    assert!(avisos[0].text.contains("Atendimento humano assumiu"));
    let metadatas = destino.metadatas();
    assert_eq!(metadatas.len(), 1);
    assert_eq!(metadatas[0].room, sala_de(mundo.conversation));
    assert_eq!(
        metadatas[0].body["conversation_id"],
        mundo.conversation.to_string()
    );
    assert_eq!(metadatas[0].body["status"], "active");
    // A sala guardou a situação espelhada: a próxima passagem não repete nada.
    let room = store
        .matrix()
        .room(org, mundo.conversation)
        .expect("lê a sala")
        .expect("sala existe");
    assert_eq!(room.last_status, Some(ConversationStatus::Transferred));
}

#[tokio::test]
async fn reabertura_avisa_que_o_agente_voltou() {
    let cenario = Cenario::new("reabertura");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::new();
    let state = laco(Arc::clone(&store), Arc::clone(&destino));

    definir_status(
        &store,
        org,
        mundo.conversation,
        ConversationStatus::Transferred,
    );
    state.cycle().await.expect("primeira passagem");
    assert_eq!(destino.avisos().len(), 1);

    definir_status(&store, org, mundo.conversation, ConversationStatus::Open);
    let relatorio = state.cycle().await.expect("segunda passagem");
    assert_eq!(relatorio.notified, 1, "{relatorio:?}");
    let avisos = destino.avisos();
    assert_eq!(avisos.len(), 2);
    assert!(avisos[1].text.contains("retomou o atendimento"));
    assert_ne!(avisos[0].txn, avisos[1].txn, "cada transição tem sua chave");
    assert_eq!(destino.metadatas().len(), 2);
}

#[tokio::test]
async fn encerramento_avisa_e_marca_o_metadata_como_closed() {
    let cenario = Cenario::new("encerramento");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::new();
    let state = laco(Arc::clone(&store), Arc::clone(&destino));

    definir_status(
        &store,
        org,
        mundo.conversation,
        ConversationStatus::Transferred,
    );
    state.cycle().await.expect("transferência");
    definir_status(&store, org, mundo.conversation, ConversationStatus::Closed);
    let relatorio = state.cycle().await.expect("encerramento");

    assert_eq!(relatorio.notified, 1, "{relatorio:?}");
    let avisos = destino.avisos();
    assert_eq!(avisos.len(), 2);
    assert!(avisos[1].text.contains("Conversa encerrada"));
    let metadatas = destino.metadatas();
    assert_eq!(metadatas.len(), 2);
    assert_eq!(metadatas[1].body["status"], "closed");
}

#[tokio::test]
async fn mesma_situacao_nao_escreve_nada() {
    let cenario = Cenario::new("mesma-situacao");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::new();
    let state = laco(Arc::clone(&store), Arc::clone(&destino));

    definir_status(
        &store,
        org,
        mundo.conversation,
        ConversationStatus::Transferred,
    );
    state.cycle().await.expect("primeira passagem");
    let escritas = destino.escritas();
    assert_eq!(escritas, 2, "um aviso e um metadata na transição");

    // A conversa volta a ser lida (mensagem nova sobe o `updated_at`), com a mesma situação: a
    // passagem seguinte não pode escrever nada no Synapse.
    store
        .conversations()
        .touch(org, mundo.conversation, 3_000_000)
        .expect("toca a conversa");
    let relatorio = state.cycle().await.expect("segunda passagem");
    assert_eq!(relatorio.checked, 1, "{relatorio:?}");
    assert_eq!(relatorio.without_change, 1, "{relatorio:?}");
    assert_eq!(relatorio.notified, 0, "{relatorio:?}");
    assert_eq!(destino.escritas(), escritas, "nenhuma escrita nova");
    assert_eq!(destino.avisos().len(), 1);
    assert_eq!(destino.metadatas().len(), 1);
}

#[tokio::test]
async fn conversa_de_org_desligada_nao_gera_chamada() {
    let cenario = Cenario::new("org-desligada");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agent = criar_agente(&store, org, "bia", None);
    let conversation = criar_conversa(&store, org, agent, "5511999990000");
    sala_pronta(&store, org, conversation);
    // Módulo desligado para a org (nunca ligado).
    definir_status(&store, org, conversation, ConversationStatus::Transferred);
    let destino = Falso::new();
    let state = laco(Arc::clone(&store), Arc::clone(&destino));

    let relatorio = state.cycle().await.expect("passagem");
    assert_eq!(relatorio.checked, 0, "{relatorio:?}");
    assert_eq!(destino.escritas(), 0);
}

#[tokio::test]
async fn conversa_sem_sala_nao_avisa_e_o_cursor_passa() {
    let cenario = Cenario::new("sem-sala");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    store.matrix().enable(org).expect("liga");
    let agent = criar_agente(&store, org, "bia", None);
    let conversation = criar_conversa(&store, org, agent, "5511999990000");
    // Sem sala: a MX-14 é quem cria.
    definir_status(&store, org, conversation, ConversationStatus::Transferred);
    let destino = Falso::new();
    let state = laco(Arc::clone(&store), Arc::clone(&destino));

    let relatorio = state.cycle().await.expect("passagem");
    assert_eq!(relatorio.without_room, 1, "{relatorio:?}");
    assert_eq!(destino.escritas(), 0);
    // O cursor passou: a conversa não fica presa.
    let cursores = store.matrix().read_cursors(org).expect("cursores");
    assert!(cursores.conversations.updated_at > 0);
}

#[tokio::test]
async fn sala_em_criacao_espera_a_passagem_seguinte() {
    let cenario = Cenario::new("em-criacao");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    store.matrix().enable(org).expect("liga");
    let agent = criar_agente(&store, org, "bia", None);
    let conversation = criar_conversa(&store, org, agent, "5511999990000");
    sala_em_criacao(&store, org, conversation);
    definir_status(&store, org, conversation, ConversationStatus::Transferred);
    let destino = Falso::new();
    let state = laco(Arc::clone(&store), Arc::clone(&destino));

    let relatorio = state.cycle().await.expect("passagem");
    assert_eq!(relatorio.deferred, 1, "{relatorio:?}");
    assert_eq!(
        destino.escritas(),
        0,
        "nada no Synapse antes da sala pronta"
    );
    let cursores = store.matrix().read_cursors(org).expect("cursores");
    assert_eq!(cursores.conversations.updated_at, 0, "cursor não andou");

    // A MX-11 termina a criação; a passagem seguinte avisa.
    store
        .matrix()
        .mark_room_ready(org, conversation)
        .expect("marca pronta");
    let relatorio = state.cycle().await.expect("segunda passagem");
    assert_eq!(relatorio.notified, 1, "{relatorio:?}");
    assert_eq!(destino.avisos().len(), 1);
}

#[tokio::test]
async fn falha_transitoria_nao_avanca_o_cursor_e_a_passagem_seguinte_aplica() {
    let cenario = Cenario::new("falha-transitoria");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::new();
    destino.definir_modo(Modo::FalhaNaPrimeira);
    let state = laco(Arc::clone(&store), Arc::clone(&destino));

    definir_status(
        &store,
        org,
        mundo.conversation,
        ConversationStatus::Transferred,
    );
    let relatorio = state.cycle().await.expect("passagem com falha");
    assert_eq!(relatorio.deferred, 1, "{relatorio:?}");
    assert_eq!(destino.avisos().len(), 0);
    let cursores = store.matrix().read_cursors(org).expect("cursores");
    assert_eq!(
        cursores.conversations.updated_at, 0,
        "o cursor não pode consumir a conversa em silêncio"
    );
    let room = store
        .matrix()
        .room(org, mundo.conversation)
        .expect("lê")
        .expect("existe");
    assert_eq!(room.last_status, None, "nada espelhado");

    // Synapse de volta: a passagem seguinte aplica o estado.
    let relatorio = state.cycle().await.expect("passagem seguinte");
    assert_eq!(relatorio.notified, 1, "{relatorio:?}");
    let avisos = destino.avisos();
    assert_eq!(avisos.len(), 1);
    assert!(avisos[0].txn.starts_with("ph_est_"));
}

#[tokio::test]
async fn erro_definitivo_pula_a_conversa_e_nao_trava_a_fila() {
    let cenario = Cenario::new("definitivo");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::new();
    destino.definir_modo(Modo::Recusa);
    let state = laco(Arc::clone(&store), Arc::clone(&destino));

    definir_status(
        &store,
        org,
        mundo.conversation,
        ConversationStatus::Transferred,
    );
    let relatorio = state.cycle().await.expect("passagem");
    assert_eq!(relatorio.skipped, 1, "{relatorio:?}");
    assert_eq!(relatorio.deferred, 0);
    assert!(destino.avisos().is_empty(), "nada foi escrito na sala");
    let cursores = store.matrix().read_cursors(org).expect("cursores");
    assert!(
        cursores.conversations.updated_at > 0,
        "recusa definitiva não pode travar o cursor"
    );
    // A conversa volta na próxima alteração dela, com o Synapse bom.
    destino.definir_modo(Modo::Normal);
    definir_status(&store, org, mundo.conversation, ConversationStatus::Closed);
    let relatorio = state.cycle().await.expect("segunda passagem");
    assert_eq!(relatorio.notified, 1, "{relatorio:?}");
    assert_eq!(destino.avisos().len(), 1);
    assert!(destino.avisos()[0].text.contains("Conversa encerrada"));
}

#[tokio::test]
async fn duas_conversas_no_mesmo_milissegundo_nao_se_perdem() {
    let cenario = Cenario::new("mesmo-instante");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    store.matrix().enable(org).expect("liga");
    let agent = criar_agente(&store, org, "bia", None);
    let uma = criar_conversa(&store, org, agent, "5511999990001");
    let outra = criar_conversa(&store, org, agent, "5511999990002");
    sala_pronta(&store, org, uma);
    sala_pronta(&store, org, outra);

    // As duas mudam no mesmo milissegundo: quem desempata é o id do cursor.
    cenario.congelar(2_000_000);
    definir_status(&store, org, uma, ConversationStatus::Transferred);
    definir_status(&store, org, outra, ConversationStatus::Transferred);
    let atualizadas: Vec<i64> = [uma, outra]
        .iter()
        .map(|conversation| {
            store
                .conversations()
                .get(org, *conversation)
                .expect("existe")
                .updated_at
        })
        .collect();
    assert_eq!(
        atualizadas[0], atualizadas[1],
        "mesmo instante de propósito"
    );

    let destino = Falso::new();
    let state = laco(Arc::clone(&store), Arc::clone(&destino));
    let relatorio = state.cycle().await.expect("passagem");
    assert_eq!(relatorio.notified, 2, "{relatorio:?}");
    assert_eq!(destino.avisos().len(), 2, "nenhuma conversa ficou de fora");

    let relatorio = state.cycle().await.expect("repetição");
    assert_eq!(
        relatorio.notified, 0,
        "nada novo na repetição: {relatorio:?}"
    );
}

#[tokio::test]
async fn paginas_cobrem_todas_as_conversas_alteradas() {
    let cenario = Cenario::new("paginas");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    store.matrix().enable(org).expect("liga");
    let agent = criar_agente(&store, org, "bia", None);
    // Página de 2 e 5 conversas: são 3 páginas.
    let mut conversas = Vec::new();
    for n in 0..5 {
        let conversation = criar_conversa(&store, org, agent, &format!("55119999900{n:02}"));
        sala_pronta(&store, org, conversation);
        definir_status(&store, org, conversation, ConversationStatus::Transferred);
        conversas.push(conversation);
    }
    let destino = Falso::new();
    let state = ConversationStates::new(
        StateStorePort::fresh(Arc::clone(&store)),
        Sinal(Arc::clone(&destino)),
        &config(),
    )
    .expect("monta")
    .with_page(2);

    let relatorio = state.cycle().await.expect("passagem");
    assert_eq!(relatorio.notified, 5, "{relatorio:?}");
    assert_eq!(destino.avisos().len(), 5);
    assert!(relatorio.pages >= 3, "{relatorio:?}");
    // A página do teste é menor que a do módulo: o laço não depende do tamanho.
    const { assert!(PAGE > 2) };
}

#[tokio::test]
async fn mudanca_de_departamento_move_a_sala_sem_aviso_duplicado() {
    let cenario = Cenario::new("departamento");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    store.matrix().enable(org).expect("liga");
    let suporte = criar_departamento(&store, org, "Suporte");
    let comercial = criar_departamento(&store, org, "Comercial");
    let agent = criar_agente(&store, org, "bia", Some(suporte));
    let conversation = criar_conversa(&store, org, agent, "5511999990000");
    sala_pronta(&store, org, conversation);
    // A sala já está no espaço do departamento padrão.
    store
        .matrix()
        .set_space_of_room(org, conversation, Some("!espaco-suporte:matrix-dev.local"))
        .expect("grava o espaço");

    let destino = Falso::new();
    destino.definir_espaco(Espaco::Moveu);
    let state = laco(Arc::clone(&store), Arc::clone(&destino));

    // Transferência para outro departamento: um aviso, um metadata e uma mudança de espaço.
    store
        .conversations()
        .transfer(org, conversation, Some(comercial), None, "{}".to_owned())
        .expect("transfere");
    let relatorio = state.cycle().await.expect("passagem");
    assert_eq!(relatorio.notified, 1, "{relatorio:?}");
    assert_eq!(relatorio.positioned, 1, "{relatorio:?}");
    assert_eq!(destino.avisos().len(), 1, "um aviso só");
    let posicoes = destino.posicoes();
    assert_eq!(posicoes.len(), 1);
    assert_eq!(posicoes[0].conversation, conversation);
    assert_eq!(
        posicoes[0]
            .department
            .as_ref()
            .map(|department| department.id),
        Some(comercial)
    );
    let metadatas = destino.metadatas();
    assert_eq!(metadatas.len(), 1);
    assert_eq!(metadatas[0].body["department_id"], comercial.to_string());

    // Repetir com a sala já no lugar novo: nenhum aviso novo.
    let relatorio = state.cycle().await.expect("repetição");
    assert_eq!(relatorio.notified, 0, "{relatorio:?}");
    assert_eq!(relatorio.positioned, 0, "{relatorio:?}");
    assert_eq!(destino.avisos().len(), 1);
}

#[tokio::test]
async fn conversa_sem_departamento_nao_fala_de_espaco() {
    let cenario = Cenario::new("sem-departamento");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::new();
    let state = laco(Arc::clone(&store), Arc::clone(&destino));

    definir_status(
        &store,
        org,
        mundo.conversation,
        ConversationStatus::Transferred,
    );
    state.cycle().await.expect("passagem");
    assert!(
        destino.posicoes().is_empty(),
        "sem departamento, sem espaço"
    );
    assert_eq!(destino.avisos().len(), 1);
}

#[tokio::test]
async fn falha_de_espaco_nao_desfaz_o_estado_ja_espelhado() {
    let cenario = Cenario::new("espaco-falha");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    store.matrix().enable(org).expect("liga");
    let suporte = criar_departamento(&store, org, "Suporte");
    let agent = criar_agente(&store, org, "bia", Some(suporte));
    let conversation = criar_conversa(&store, org, agent, "5511999990000");
    sala_pronta(&store, org, conversation);
    definir_status(&store, org, conversation, ConversationStatus::Transferred);

    let destino = Falso::new();
    let state = laco(Arc::clone(&store), Arc::clone(&destino));
    let relatorio = state.cycle().await.expect("passagem");

    // O espaço não tem o bot: a sala não entra, e o aviso/metadata continuam valendo.
    assert_eq!(relatorio.notified, 1, "{relatorio:?}");
    assert_eq!(relatorio.positioned, 0);
    assert_eq!(destino.avisos().len(), 1);
    let room = store
        .matrix()
        .room(org, conversation)
        .expect("lê")
        .expect("existe");
    assert_eq!(room.last_status, Some(ConversationStatus::Transferred));
}

// ───────────────────────────── corrida da situação ─────────────────────────────

/// Porta que mente numa leitura de propósito: prova o que acontece quando a conversa muda entre a
/// leitura que dispara a passagem e a reconferência de antes da escrita.
struct PortaQueMente {
    real: StateStorePort,
    leituras: AtomicUsize,
    mentira: Mutex<Option<ConversationStatus>>,
}

impl PortaQueMente {
    fn fresh(store: Arc<Store>) -> Self {
        Self {
            real: StateStorePort::fresh(store),
            leituras: AtomicUsize::new(0),
            mentira: Mutex::new(None),
        }
    }

    /// Na segunda leitura de conversa devolve esta situação.
    fn mentir_na_segunda(&self, status: ConversationStatus) {
        *self.mentira.lock().unwrap() = Some(status);
    }
}

impl StatePort for PortaQueMente {
    async fn orgs(&self) -> Result<Vec<OrgState>, poph_store::error::StoreError> {
        self.real.orgs().await
    }

    async fn cursors(&self, org: OrgId) -> Result<StoredCursor, poph_store::error::StoreError> {
        self.real.cursors(org).await
    }

    async fn changed_items(
        &self,
        apos: Option<(i64, ConversationId)>,
        limite: u32,
    ) -> Result<Vec<ConversationChanged>, poph_store::error::StoreError> {
        self.real.changed_items(apos, limite).await
    }

    async fn conversation(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Result<Option<ConversationState>, poph_store::error::StoreError> {
        let leitura = self.leituras.fetch_add(1, Ordering::SeqCst) + 1;
        let real = self.real.conversation(org, conversation).await?;
        if leitura != 2 {
            return Ok(real);
        }
        let mentira = *self.mentira.lock().unwrap();
        Ok(real.map(|mut vista| {
            if let Some(status) = mentira {
                vista.status = status;
            }
            vista
        }))
    }

    async fn room(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Result<Option<RoomState>, poph_store::error::StoreError> {
        self.real.room(org, conversation).await
    }

    async fn define_last_status(
        &self,
        org: OrgId,
        conversation: ConversationId,
        status: ConversationStatus,
    ) -> Result<(), poph_store::error::StoreError> {
        self.real
            .define_last_status(org, conversation, status)
            .await
    }

    async fn define_name_mirrored(
        &self,
        org: OrgId,
        conversation: ConversationId,
        name: &str,
    ) -> Result<(), poph_store::error::StoreError> {
        self.real
            .define_name_mirrored(org, conversation, name)
            .await
    }

    async fn advance_conversations(
        &self,
        org: OrgId,
        atualizado_em: i64,
        id: &str,
    ) -> Result<(), poph_store::error::StoreError> {
        self.real
            .advance_conversations(org, atualizado_em, id)
            .await
    }
}

#[tokio::test]
async fn conversa_fechada_no_meio_da_passagem_nao_recebe_metadata_de_reabertura() {
    let cenario = Cenario::new("corrida");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    // A sala já espelhou a transferência; a conversa está aberta de novo (reabertura pendente).
    store
        .matrix()
        .define_last_status(org, mundo.conversation, ConversationStatus::Transferred)
        .expect("grava a situação anterior");
    let destino = Falso::new();
    let porta = PortaQueMente::fresh(Arc::clone(&store));
    // A leitura que dispara vê a reabertura; a reconferência vê a conversa já encerrada.
    porta.mentir_na_segunda(ConversationStatus::Closed);
    let state =
        ConversationStates::new(porta, Sinal(Arc::clone(&destino)), &config()).expect("monta");

    let relatorio = state.cycle().await.expect("passagem");
    assert_eq!(relatorio.deferred, 1, "{relatorio:?}");
    assert!(
        destino.metadatas().is_empty(),
        "nenhum metadata atrasado: a conversa fechou no meio"
    );
    assert!(destino.avisos().is_empty(), "nenhum aviso atrasado");

    // Fecha a conversa de verdade: a passagem seguinte espelha o encerramento, e o metadata de
    // reabertura nunca existiu.
    definir_status(&store, org, mundo.conversation, ConversationStatus::Closed);
    let relatorio = state.cycle().await.expect("segunda passagem");
    assert_eq!(relatorio.notified, 1, "{relatorio:?}");
    let metadatas = destino.metadatas();
    assert_eq!(metadatas.len(), 1);
    assert_eq!(metadatas[0].body["status"], "closed");
    assert_eq!(destino.avisos().len(), 1);
    assert!(destino.avisos()[0].text.contains("Conversa encerrada"));
}

// ───────────────────────────── integração com o banco ─────────────────────────────

#[tokio::test]
async fn porta_do_store_le_o_que_o_banco_gravou() {
    let cenario = Cenario::new("porta");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let porta = StateStorePort::fresh(Arc::clone(&store));

    let orgs = porta.orgs().await.expect("orgs");
    assert_eq!(orgs.len(), 1);
    assert!(orgs[0].enabled);

    let cursores = porta.cursors(org).await.expect("cursores");
    assert_eq!(cursores.updated_at, 0);
    assert_eq!(cursores.id, None);

    let alteradas = porta.changed_items(None, 10).await.expect("alteradas");
    assert_eq!(alteradas.len(), 1);
    assert_eq!(alteradas[0].conversation, mundo.conversation);

    let conversation = porta
        .conversation(org, mundo.conversation)
        .await
        .expect("conversa")
        .expect("existe");
    assert_eq!(conversation.status, ConversationStatus::Open);
    assert_eq!(conversation.agent, mundo.agent);
    assert_eq!(conversation.channel, "whatsapp");
    assert_eq!(conversation.client, "5511999990000");
    assert_eq!(conversation.agent_name.as_deref(), Some("Bia"));

    let room = porta
        .room(org, mundo.conversation)
        .await
        .expect("sala")
        .expect("existe");
    assert!(room.ready);
    assert_eq!(room.last_status, None);

    porta
        .define_last_status(org, mundo.conversation, ConversationStatus::Transferred)
        .await
        .expect("grava situação");
    let room = porta
        .room(org, mundo.conversation)
        .await
        .expect("sala")
        .expect("existe");
    assert_eq!(room.last_status, Some(ConversationStatus::Transferred));

    porta
        .advance_conversations(org, 5_000, &mundo.conversation.to_string())
        .await
        .expect("avança");
    let cursores = porta.cursors(org).await.expect("cursores");
    assert_eq!(cursores.updated_at, 5_000);
    assert_eq!(cursores.id, Some(mundo.conversation));
}

#[tokio::test]
async fn conversa_apagada_entre_a_pagina_e_a_leitura_nao_quebra_a_passagem() {
    let cenario = Cenario::new("apagada");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::new();

    // Porta que lista uma conversa que não existe mais (a remoção acontece entre as duas leituras).
    let porta = PortaQueApaga {
        real: StateStorePort::fresh(Arc::clone(&store)),
    };
    let state =
        ConversationStates::new(porta, Sinal(Arc::clone(&destino)), &config()).expect("monta");
    let relatorio = state.cycle().await.expect("passagem");
    // A conversa de verdade é conferida (e nada muda); a inventada não quebra a passagem.
    assert_eq!(relatorio.checked, 1, "{relatorio:?}");
    assert_eq!(relatorio.without_change, 1, "{relatorio:?}");
    assert_eq!(relatorio.deferred, 0, "{relatorio:?}");
    assert_eq!(destino.escritas(), 0);
    let cursores = store.matrix().read_cursors(org).expect("cursores");
    assert!(
        cursores.conversations.updated_at >= INSTANTE_INVENTADO,
        "conversa que sumiu não prende o cursor"
    );
    let _ = mundo;
}

/// Instante da conversa que a porta inventa (maior que qualquer conversa do cenário).
const INSTANTE_INVENTADO: i64 = 9_000_000;

/// Porta que lista uma conversa inventada, que não existe no banco.
struct PortaQueApaga {
    real: StateStorePort,
}

impl StatePort for PortaQueApaga {
    async fn orgs(&self) -> Result<Vec<OrgState>, poph_store::error::StoreError> {
        self.real.orgs().await
    }

    async fn cursors(&self, org: OrgId) -> Result<StoredCursor, poph_store::error::StoreError> {
        self.real.cursors(org).await
    }

    async fn changed_items(
        &self,
        apos: Option<(i64, ConversationId)>,
        limite: u32,
    ) -> Result<Vec<ConversationChanged>, poph_store::error::StoreError> {
        let reais = self.real.changed_items(apos, limite).await?;
        let mut lista = reais;
        let org = lista.first().map(|alterada| alterada.org);
        if let Some(org) = org {
            // A conversa que o laço vai procurar não está mais no banco.
            lista.push(ConversationChanged {
                org,
                conversation: ConversationId::generate(),
                updated_at: INSTANTE_INVENTADO,
            });
        }
        Ok(lista)
    }

    async fn conversation(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Result<Option<ConversationState>, poph_store::error::StoreError> {
        self.real.conversation(org, conversation).await
    }

    async fn room(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Result<Option<RoomState>, poph_store::error::StoreError> {
        self.real.room(org, conversation).await
    }

    async fn define_last_status(
        &self,
        org: OrgId,
        conversation: ConversationId,
        status: ConversationStatus,
    ) -> Result<(), poph_store::error::StoreError> {
        self.real
            .define_last_status(org, conversation, status)
            .await
    }

    async fn define_name_mirrored(
        &self,
        org: OrgId,
        conversation: ConversationId,
        name: &str,
    ) -> Result<(), poph_store::error::StoreError> {
        self.real
            .define_name_mirrored(org, conversation, name)
            .await
    }

    async fn advance_conversations(
        &self,
        org: OrgId,
        atualizado_em: i64,
        id: &str,
    ) -> Result<(), poph_store::error::StoreError> {
        self.real
            .advance_conversations(org, atualizado_em, id)
            .await
    }
}

#[tokio::test]
async fn resposta_e_metadata_nao_levam_dado_de_cliente() {
    let cenario = Cenario::new("sem-dado");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::new();
    let state = laco(Arc::clone(&store), Arc::clone(&destino));

    definir_status(
        &store,
        org,
        mundo.conversation,
        ConversationStatus::Transferred,
    );
    state.cycle().await.expect("passagem");

    for aviso in destino.avisos() {
        assert!(
            !aviso.text.contains("5511999990000"),
            "o aviso não pode ter o telefone: {}",
            aviso.text
        );
    }
    for metadata in destino.metadatas() {
        let text = metadata.body.to_string();
        // O metadata leva o identificador do cliente (o Element precisa dele), nunca o nome.
        assert!(text.contains("conversation_id"));
        assert!(!text.contains("\"client_name\""), "{text}");
    }
}

// ───────────────────────────── nome do cliente (MX-33) ─────────────────────────────

/// Grava o nome do cliente pelo caminho de verdade da MX-32 (sobe `updated_at`, não `version`).
fn definir_nome(store: &Store, org: OrgId, conversation: ConversationId, name: &str) {
    let mudou = store
        .conversations()
        .set_customer_name(org, conversation, name)
        .expect("grava o nome do cliente");
    assert!(mudou, "o nome precisa ter mudado para o cursor enxergar");
}

/// Nome espelhado que a linha da sala guarda (o que a MX-33 compara).
fn nome_espelhado(store: &Store, org: OrgId, conversation: ConversationId) -> Option<String> {
    store
        .matrix()
        .room(org, conversation)
        .expect("lê a sala")
        .expect("sala existe")
        .name_client_mirrored
}

/// Toca a conversa sem mudar nada: o cursor volta a vê-la, como uma mensagem nova do cliente.
///
/// O relógio do cenário anda junto, para as escritas seguintes (o nome novo, por exemplo) ficarem
/// depois deste instante — senão ficariam atrás do cursor.
fn tocar(cenario: &Cenario, store: &Store, org: OrgId, conversation: ConversationId) {
    let quando = cenario.relogio.agora.fetch_add(60_000, Ordering::SeqCst) + 60_000;
    store
        .conversations()
        .touch(org, conversation, quando)
        .expect("toca a conversa");
}

#[tokio::test]
async fn nome_que_chega_depois_renomeia_sala_metadata_e_fantasma_uma_vez_cada() {
    let cenario = Cenario::new("nome-depois");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::new();
    let state = laco(Arc::clone(&store), Arc::clone(&destino));
    // A sala nasceu com o telefone: nenhum nome espelhado.
    assert_eq!(nome_espelhado(&store, org, mundo.conversation), None);

    definir_nome(&store, org, mundo.conversation, "Ana Souza");
    let relatorio = state.cycle().await.expect("passagem do nome");

    assert_eq!(relatorio.renamed, 1, "{relatorio:?}");
    let renomeacoes = destino.renomeacoes();
    assert_eq!(renomeacoes.len(), 1, "uma renomeação de sala");
    assert_eq!(renomeacoes[0].room, sala_de(mundo.conversation));
    assert_eq!(renomeacoes[0].name, "Ana Souza");

    let metadatas = destino.metadatas();
    assert_eq!(metadatas.len(), 1, "um metadata com o nome");
    assert_eq!(metadatas[0].body["client_name"], "Ana Souza");
    assert_eq!(metadatas[0].body["status"], "active");

    let fantasmas = destino.fantasmas();
    assert_eq!(fantasmas.len(), 1, "um nome de fantasma");
    assert_eq!(fantasmas[0].client, "5511999990000");
    assert_eq!(fantasmas[0].name, "Ana Souza");

    assert!(destino.avisos().is_empty(), "trocar nome não é aviso");
    assert_eq!(
        nome_espelhado(&store, org, mundo.conversation).as_deref(),
        Some("Ana Souza"),
        "o nome só é dado como espelhado no fim"
    );

    // Repetir o ciclo (a conversa volta a ser vista) não fala com o Synapse.
    let escritas = destino.escritas();
    tocar(&cenario, &store, org, mundo.conversation);
    let relatorio = state.cycle().await.expect("passagem repetida");
    assert_eq!(relatorio.renamed, 0, "{relatorio:?}");
    assert_eq!(destino.escritas(), escritas, "repetir não escreve nada");
    assert_eq!(destino.renomeacoes().len(), 1);
    assert_eq!(destino.metadatas().len(), 1);
    assert_eq!(destino.fantasmas().len(), 1);

    // Um segundo nome novo atualiza de novo, uma vez cada.
    definir_nome(&store, org, mundo.conversation, "Ana Maria");
    let relatorio = state.cycle().await.expect("passagem do nome novo");
    assert_eq!(relatorio.renamed, 1, "{relatorio:?}");
    assert_eq!(destino.renomeacoes().len(), 2);
    assert_eq!(destino.renomeacoes()[1].name, "Ana Maria");
    assert_eq!(destino.metadatas()[1].body["client_name"], "Ana Maria");
    assert_eq!(destino.fantasmas()[1].name, "Ana Maria");
    assert_eq!(
        nome_espelhado(&store, org, mundo.conversation).as_deref(),
        Some("Ana Maria")
    );
}

#[tokio::test]
async fn queda_entre_o_nome_e_o_metadata_retoma_sem_sala_nem_aviso_duplicado() {
    let cenario = Cenario::new("queda-no-nome");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::new();
    let state = laco(Arc::clone(&store), Arc::clone(&destino));

    definir_nome(&store, org, mundo.conversation, "Ana Souza");
    // A segunda escrita da passagem (o metadata) falha de forma transitória: é a queda entre as
    // três chamadas do nome.
    *destino.modo.lock().unwrap() = Modo::FalhaNaEscrita(2);
    let relatorio = state.cycle().await.expect("passagem que cai");

    assert_eq!(relatorio.deferred, 1, "{relatorio:?}");
    assert_eq!(destino.renomeacoes().len(), 1, "a sala foi renomeada");
    assert!(destino.metadatas().is_empty(), "o metadata não saiu");
    assert!(
        destino.fantasmas().is_empty(),
        "o fantasma vem depois do metadata"
    );
    assert_eq!(
        nome_espelhado(&store, org, mundo.conversation),
        None,
        "queda no meio não dá o nome como espelhado"
    );

    // A passagem seguinte refaz o caminho: as chamadas são idempotentes, e nada duplica.
    let relatorio = state.cycle().await.expect("passagem da retomada");
    assert_eq!(relatorio.renamed, 1, "{relatorio:?}");
    assert_eq!(destino.renomeacoes().len(), 2, "a renomeação se repete");
    assert_eq!(destino.renomeacoes()[1].name, "Ana Souza");
    assert_eq!(destino.metadatas().len(), 1, "um metadata só");
    assert_eq!(destino.fantasmas().len(), 1);
    assert!(destino.avisos().is_empty(), "nenhum aviso de atendimento");
    // A sala continua sendo uma só: a retomada não cria outra.
    assert_eq!(
        store
            .matrix()
            .room(org, mundo.conversation)
            .expect("lê a sala")
            .expect("sala existe")
            .room_id,
        sala_de(mundo.conversation)
    );
    assert_eq!(
        nome_espelhado(&store, org, mundo.conversation).as_deref(),
        Some("Ana Souza")
    );
}

#[tokio::test]
async fn nome_atrasado_nao_devolve_conversa_fechada_para_active_no_metadata() {
    let cenario = Cenario::new("nome-atrasado");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::new();

    // A conversa é encerrada e o nome do cliente chega logo depois. A sala ainda não espelhou o
    // encerramento (`last_status` vazio = conversa aberta), e é essa a leitura que a porta atrasada
    // devolve: por isso a renomeação precisa reler antes de escrever o metadata.
    definir_status(&store, org, mundo.conversation, ConversationStatus::Closed);
    definir_nome(&store, org, mundo.conversation, "Ana Souza");

    // A porta devolve a situação velha na primeira leitura da passagem: o laço lê a conversa como
    // aberta e a renomeação só não publica `active` porque relê antes de escrever.
    let atrasada = PortaAtrasada::fresh(Arc::clone(&store));
    let state = ConversationStates::new(atrasada, Sinal(Arc::clone(&destino)), &config())
        .expect("monta o laço");

    let relatorio = state.cycle().await.expect("passagem do nome");
    assert_eq!(relatorio.renamed, 1, "{relatorio:?}");
    let metadatas = destino.metadatas();
    assert_eq!(metadatas.len(), 1);
    assert_eq!(
        metadatas[0].body["status"], "closed",
        "o metadata do nome sai com o estado atual da conversa"
    );
    assert_eq!(metadatas[0].body["client_name"], "Ana Souza");
    assert!(
        destino.avisos().is_empty(),
        "renomear não é aviso e a leitura atrasada não gerou aviso"
    );
    // A conversa continua encerrada (o nome atrasado não reabriu nada).
    assert_eq!(
        store
            .conversations()
            .get(org, mundo.conversation)
            .expect("lê a conversa")
            .status,
        ConversationStatus::Closed
    );

    // O `last_status` não foi tocado pela renomeação: quem espelha a situação é o passo do estado.
    assert_eq!(
        store
            .matrix()
            .room(org, mundo.conversation)
            .expect("lê a sala")
            .expect("sala existe")
            .last_status,
        None,
        "renomear não espelha situação"
    );
}

#[tokio::test]
async fn conversa_sem_nome_novo_e_org_desligada_nao_falam_com_o_synapse() {
    let cenario = Cenario::new("sem-nome-novo");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::new();
    let state = laco(Arc::clone(&store), Arc::clone(&destino));

    // Sem nome do cliente e com a sala já pronta: a passagem não escreve nada.
    tocar(&cenario, &store, org, mundo.conversation);
    let relatorio = state.cycle().await.expect("passagem sem nome");
    assert_eq!(relatorio.renamed, 0, "{relatorio:?}");
    assert!(destino.renomeacoes().is_empty());
    assert!(destino.metadatas().is_empty());
    assert!(destino.fantasmas().is_empty());

    // Nome igual ao já espelhado também não escreve.
    definir_nome(&store, org, mundo.conversation, "Ana Souza");
    state.cycle().await.expect("passagem do nome");
    let escritas = destino.escritas();
    tocar(&cenario, &store, org, mundo.conversation);
    state.cycle().await.expect("passagem repetida");
    assert_eq!(destino.escritas(), escritas);

    // Org com o módulo desligado não gera chamada nenhuma, mesmo com nome novo na conversa.
    store.matrix().disable(org).expect("desliga a org");
    definir_nome(&store, org, mundo.conversation, "Ana Maria");
    let escritas = destino.escritas();
    let relatorio = state.cycle().await.expect("passagem com a org desligada");
    assert_eq!(relatorio.renamed, 0, "{relatorio:?}");
    assert_eq!(destino.escritas(), escritas, "org desligada não escreve");
    assert_eq!(
        nome_espelhado(&store, org, mundo.conversation).as_deref(),
        Some("Ana Souza"),
        "o nome espelhado fica como estava"
    );
}

/// Porta que devolve a situação **aberta** na primeira leitura de cada conversa (o atraso do ciclo)
/// e a situação real nas seguintes.
struct PortaAtrasada {
    real: StateStorePort,
    atrasadas: Mutex<std::collections::BTreeSet<String>>,
}

impl PortaAtrasada {
    fn fresh(store: Arc<Store>) -> Self {
        Self {
            real: StateStorePort::fresh(store),
            atrasadas: Mutex::new(std::collections::BTreeSet::new()),
        }
    }
}

impl StatePort for PortaAtrasada {
    async fn orgs(&self) -> Result<Vec<OrgState>, poph_store::error::StoreError> {
        self.real.orgs().await
    }

    async fn cursors(&self, org: OrgId) -> Result<StoredCursor, poph_store::error::StoreError> {
        self.real.cursors(org).await
    }

    async fn changed_items(
        &self,
        apos: Option<(i64, ConversationId)>,
        limite: u32,
    ) -> Result<Vec<ConversationChanged>, poph_store::error::StoreError> {
        self.real.changed_items(apos, limite).await
    }

    async fn conversation(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Result<Option<ConversationState>, poph_store::error::StoreError> {
        let primeira = self
            .atrasadas
            .lock()
            .unwrap()
            .insert(conversation.to_string());
        let real = self.real.conversation(org, conversation).await?;
        if !primeira {
            return Ok(real);
        }
        // Primeira leitura da passagem: a conversa ainda parece aberta.
        Ok(real.map(|mut vista| {
            vista.status = ConversationStatus::Open;
            vista
        }))
    }

    async fn room(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Result<Option<RoomState>, poph_store::error::StoreError> {
        self.real.room(org, conversation).await
    }

    async fn define_last_status(
        &self,
        org: OrgId,
        conversation: ConversationId,
        status: ConversationStatus,
    ) -> Result<(), poph_store::error::StoreError> {
        self.real
            .define_last_status(org, conversation, status)
            .await
    }

    async fn define_name_mirrored(
        &self,
        org: OrgId,
        conversation: ConversationId,
        name: &str,
    ) -> Result<(), poph_store::error::StoreError> {
        self.real
            .define_name_mirrored(org, conversation, name)
            .await
    }

    async fn advance_conversations(
        &self,
        org: OrgId,
        atualizado_em: i64,
        id: &str,
    ) -> Result<(), poph_store::error::StoreError> {
        self.real
            .advance_conversations(org, atualizado_em, id)
            .await
    }
}

#[test]
fn erro_de_relatorio_tem_codigo_curto() {
    // O código do erro do banco é curto e não carrega SQL nem valor de linha.
    let error = StateError::Armazenamento(poph_store::error::StoreError::WriterClosed);
    assert_eq!(error.code(), "armazenamento_indisponivel");
}
