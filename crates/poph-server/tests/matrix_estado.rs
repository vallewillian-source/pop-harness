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

use poph_matrix::erro::ErroChamada;
use poph_matrix::espacos::{ErroEspaco, Posicao, SalaParaEspaco};
use poph_matrix::salas::MetadataSala;
use poph_server::matrix_estado::{
    ConversaAlterada, ConversaEstado, CursorGravado, DestinoEstado, ErroEstado, EstadoDasConversas,
    EstadoOrg, PAGINA, PortaEstado, PortaEstadoStore, SalaEstado,
};
use poph_store::{NovaConversa, NovaOrg, NovoAgente, NovoDepartamento, StatusConversa, Store};
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
    fn novo(rotulo: &str) -> Self {
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
        let store = Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db"))
            .expect("abre o banco")
            .com_relogio(Arc::new(move || pulso.agora()));
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
        habilitado: true,
        homeserver_url: "http://127.0.0.1:1".to_owned(),
        server_name: "matrix-dev.local".to_owned(),
        prefixo: "ph".to_owned(),
        concorrencia: 4,
        intervalo_ms: 500,
        ..MatrixConfig::default()
    }
}

fn criar_org(store: &Store, slug: &str) -> OrgId {
    store
        .orgaos()
        .criar(NovaOrg {
            nome: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id
}

fn criar_departamento(store: &Store, org: OrgId, nome: &str) -> DepartmentId {
    store
        .departamentos()
        .criar(
            org,
            NovoDepartamento {
                nome: nome.to_owned(),
                descricao: String::new(),
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
        .agentes()
        .criar(
            org,
            NovoAgente {
                nome: "Bia".to_owned(),
                slug: slug.to_owned(),
                departamento_padrao,
                ..NovoAgente::default()
            },
        )
        .expect("cria o agente")
        .id
}

fn criar_conversa(store: &Store, org: OrgId, agente: AgentId, peer: &str) -> ConversationId {
    store
        .conversas()
        .criar(
            org,
            NovaConversa {
                agent_id: agente,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: Some(peer.to_owned()),
                departamento_alvo: None,
            },
        )
        .expect("cria a conversa")
        .id
}

/// Grava a sala da conversa já pronta (o que a MX-11 faz ao terminar a criação).
fn sala_pronta(store: &Store, org: OrgId, conversa: ConversationId) {
    store
        .matrix()
        .garantir_sala(org, conversa, &sala_de(conversa), None)
        .expect("grava a sala");
    store
        .matrix()
        .marcar_sala_pronta(org, conversa)
        .expect("marca pronta");
}

/// Grava a sala ainda em criação (queda no meio da MX-11).
fn sala_em_criacao(store: &Store, org: OrgId, conversa: ConversationId) {
    store
        .matrix()
        .garantir_sala(org, conversa, &sala_de(conversa), None)
        .expect("grava a sala");
}

fn sala_de(conversa: ConversationId) -> String {
    format!("!{conversa}:matrix-dev.local")
}

/// Muda a situação da conversa pelo caminho de verdade (transfere, fecha, reabre).
fn definir_status(store: &Store, org: OrgId, conversa: ConversationId, status: StatusConversa) {
    let atual = store
        .conversas()
        .obter(org, conversa)
        .expect("conversa existe");
    match status {
        StatusConversa::Aberta => {
            store
                .conversas()
                .reabrir_auditado(org, conversa, atual.versao, "teste".to_owned())
                .expect("reabre");
        }
        StatusConversa::Fechada => {
            store
                .conversas()
                .fechar_auditado(org, conversa, atual.versao, "teste".to_owned())
                .expect("fecha");
        }
        StatusConversa::Transferida => {
            store
                .conversas()
                .transferir(org, conversa, None, None, "{}".to_owned())
                .expect("transfere");
        }
    }
}

// ───────────────────────────── Synapse falso ─────────────────────────────

/// Um aviso que o módulo escreveu na sala.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Aviso {
    sala: String,
    txn: String,
    agente: AgentId,
    texto: String,
}

/// Um metadata que o módulo reescreveu na sala.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MetadataEscrita {
    sala: String,
    corpo: Value,
}

/// Uma renomeação de sala (`m.room.name`) escrita no Synapse falso (MX-33).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Renomeacao {
    sala: String,
    nome: String,
}

/// Uma troca de nome de exibição do fantasma do cliente (MX-33).
#[derive(Debug, Clone, PartialEq, Eq)]
struct FantasmaRenomeado {
    cliente: String,
    nome: String,
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
    avisos: Mutex<Vec<Aviso>>,
    metadatas: Mutex<Vec<MetadataEscrita>>,
    renomeacoes: Mutex<Vec<Renomeacao>>,
    fantasmas: Mutex<Vec<FantasmaRenomeado>>,
    posicoes: Mutex<Vec<SalaParaEspaco>>,
    modo: Mutex<Modo>,
    espaco: Mutex<Espaco>,
    escritas: AtomicUsize,
}

impl Falso {
    fn novo() -> Arc<Self> {
        Arc::new(Self {
            avisos: Mutex::new(Vec::new()),
            metadatas: Mutex::new(Vec::new()),
            renomeacoes: Mutex::new(Vec::new()),
            fantasmas: Mutex::new(Vec::new()),
            posicoes: Mutex::new(Vec::new()),
            modo: Mutex::new(Modo::Normal),
            espaco: Mutex::new(Espaco::JaNoLugar),
            escritas: AtomicUsize::new(0),
        })
    }

    fn definir_modo(&self, modo: Modo) {
        *self.modo.lock().unwrap() = modo;
    }

    fn definir_espaco(&self, espaco: Espaco) {
        *self.espaco.lock().unwrap() = espaco;
    }

    fn avisos(&self) -> Vec<Aviso> {
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

    fn posicoes(&self) -> Vec<SalaParaEspaco> {
        self.posicoes.lock().unwrap().clone()
    }

    fn escritas(&self) -> usize {
        self.escritas.load(Ordering::SeqCst)
    }

    /// A escrita falha conforme o modo; a falha pedida acontece uma vez só.
    fn falhar(&self) -> Option<ErroChamada> {
        let modo = *self.modo.lock().unwrap();
        let escrita = self.escritas.fetch_add(1, Ordering::SeqCst) + 1;
        match modo {
            Modo::Normal => None,
            Modo::FalhaNaEscrita(alvo) if escrita != alvo => None,
            Modo::FalhaNaEscrita(_) | Modo::FalhaNaPrimeira => {
                *self.modo.lock().unwrap() = Modo::Normal;
                Some(ErroChamada::Resposta {
                    operacao: "send",
                    status: 503,
                    errcode: Some("M_UNKNOWN".to_owned()),
                    mensagem: "indisponível".to_owned(),
                    retry_after_ms: None,
                })
            }
            Modo::Recusa => Some(ErroChamada::Resposta {
                operacao: "send",
                status: 403,
                errcode: Some("M_FORBIDDEN".to_owned()),
                mensagem: "recusado".to_owned(),
                retry_after_ms: None,
            }),
        }
    }
}

/// O duplo compartilhado com o teste: o laço fica com uma cópia do `Arc`, o teste com outra.
struct Sinal(Arc<Falso>);

impl DestinoEstado for Sinal {
    async fn avisar(
        &self,
        sala: &str,
        agente: AgentId,
        txn: &str,
        texto: &str,
    ) -> Result<(), ErroChamada> {
        if let Some(erro) = self.0.falhar() {
            return Err(erro);
        }
        self.0.avisos.lock().unwrap().push(Aviso {
            sala: sala.to_owned(),
            txn: txn.to_owned(),
            agente,
            texto: texto.to_owned(),
        });
        Ok(())
    }

    async fn escrever_metadata(
        &self,
        sala: &str,
        _agente: AgentId,
        metadata: &MetadataSala,
    ) -> Result<(), ErroChamada> {
        if let Some(erro) = self.0.falhar() {
            return Err(erro);
        }
        let corpo = serde_json::to_value(metadata).expect("metadata serializa");
        self.0.metadatas.lock().unwrap().push(MetadataEscrita {
            sala: sala.to_owned(),
            corpo,
        });
        Ok(())
    }

    async fn renomear(&self, sala: &str, _agente: AgentId, nome: &str) -> Result<(), ErroChamada> {
        if let Some(erro) = self.0.falhar() {
            return Err(erro);
        }
        self.0.renomeacoes.lock().unwrap().push(Renomeacao {
            sala: sala.to_owned(),
            nome: nome.to_owned(),
        });
        Ok(())
    }

    async fn renomear_fantasma(&self, cliente: &str, nome: &str) -> Result<(), ErroChamada> {
        if let Some(erro) = self.0.falhar() {
            return Err(erro);
        }
        self.0.fantasmas.lock().unwrap().push(FantasmaRenomeado {
            cliente: cliente.to_owned(),
            nome: nome.to_owned(),
        });
        Ok(())
    }

    async fn posicionar(&self, dados: SalaParaEspaco) -> Result<Posicao, ErroEspaco> {
        self.0.posicoes.lock().unwrap().push(dados.clone());
        match *self.0.espaco.lock().unwrap() {
            Espaco::JaNoLugar => Ok(Posicao::JaPosicionada {
                espaco: "!espaco:matrix-dev.local".to_owned(),
            }),
            Espaco::Moveu => Ok(Posicao::Posicionada {
                espaco: "!espaco:matrix-dev.local".to_owned(),
                movida: true,
                convidados: 0,
                convites_falhos: 0,
            }),
        }
    }
}

fn laco(store: Arc<Store>, destino: Arc<Falso>) -> EstadoDasConversas<Sinal, PortaEstadoStore> {
    EstadoDasConversas::novo(PortaEstadoStore::nova(store), Sinal(destino), &config())
        .expect("monta o laço")
}

// ───────────────────────────── cenário padrão ─────────────────────────────

/// Org com o módulo ligado, um agente e uma conversa aberta com a sala pronta.
struct Mundo {
    agente: AgentId,
    conversa: ConversationId,
}

fn mundo(store: &Store, org: OrgId) -> Mundo {
    store.matrix().ligar(org).expect("liga o módulo");
    let agente = criar_agente(store, org, "bia", None);
    let conversa = criar_conversa(store, org, agente, "5511999990000");
    sala_pronta(store, org, conversa);
    Mundo { agente, conversa }
}

// ───────────────────────────── testes ─────────────────────────────

#[tokio::test]
async fn transferencia_avisa_e_reescreve_o_metadata() {
    let cenario = Cenario::novo("transferencia");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::novo();
    let estado = laco(Arc::clone(&store), Arc::clone(&destino));

    definir_status(&store, org, mundo.conversa, StatusConversa::Transferida);
    let relatorio = estado.ciclo().await.expect("passagem");

    assert_eq!(relatorio.avisadas, 1, "{relatorio:?}");
    let avisos = destino.avisos();
    assert_eq!(avisos.len(), 1);
    assert_eq!(avisos[0].sala, sala_de(mundo.conversa));
    assert_eq!(avisos[0].agente, mundo.agente);
    assert!(avisos[0].texto.contains("Atendimento humano assumiu"));
    let metadatas = destino.metadatas();
    assert_eq!(metadatas.len(), 1);
    assert_eq!(metadatas[0].sala, sala_de(mundo.conversa));
    assert_eq!(
        metadatas[0].corpo["conversation_id"],
        mundo.conversa.to_string()
    );
    assert_eq!(metadatas[0].corpo["status"], "active");
    // A sala guardou a situação espelhada: a próxima passagem não repete nada.
    let sala = store
        .matrix()
        .sala(org, mundo.conversa)
        .expect("lê a sala")
        .expect("sala existe");
    assert_eq!(sala.ultimo_status, Some(StatusConversa::Transferida));
}

#[tokio::test]
async fn reabertura_avisa_que_o_agente_voltou() {
    let cenario = Cenario::novo("reabertura");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::novo();
    let estado = laco(Arc::clone(&store), Arc::clone(&destino));

    definir_status(&store, org, mundo.conversa, StatusConversa::Transferida);
    estado.ciclo().await.expect("primeira passagem");
    assert_eq!(destino.avisos().len(), 1);

    definir_status(&store, org, mundo.conversa, StatusConversa::Aberta);
    let relatorio = estado.ciclo().await.expect("segunda passagem");
    assert_eq!(relatorio.avisadas, 1, "{relatorio:?}");
    let avisos = destino.avisos();
    assert_eq!(avisos.len(), 2);
    assert!(avisos[1].texto.contains("retomou o atendimento"));
    assert_ne!(avisos[0].txn, avisos[1].txn, "cada transição tem sua chave");
    assert_eq!(destino.metadatas().len(), 2);
}

#[tokio::test]
async fn encerramento_avisa_e_marca_o_metadata_como_closed() {
    let cenario = Cenario::novo("encerramento");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::novo();
    let estado = laco(Arc::clone(&store), Arc::clone(&destino));

    definir_status(&store, org, mundo.conversa, StatusConversa::Transferida);
    estado.ciclo().await.expect("transferência");
    definir_status(&store, org, mundo.conversa, StatusConversa::Fechada);
    let relatorio = estado.ciclo().await.expect("encerramento");

    assert_eq!(relatorio.avisadas, 1, "{relatorio:?}");
    let avisos = destino.avisos();
    assert_eq!(avisos.len(), 2);
    assert!(avisos[1].texto.contains("Conversa encerrada"));
    let metadatas = destino.metadatas();
    assert_eq!(metadatas.len(), 2);
    assert_eq!(metadatas[1].corpo["status"], "closed");
}

#[tokio::test]
async fn mesma_situacao_nao_escreve_nada() {
    let cenario = Cenario::novo("mesma-situacao");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::novo();
    let estado = laco(Arc::clone(&store), Arc::clone(&destino));

    definir_status(&store, org, mundo.conversa, StatusConversa::Transferida);
    estado.ciclo().await.expect("primeira passagem");
    let escritas = destino.escritas();
    assert_eq!(escritas, 2, "um aviso e um metadata na transição");

    // A conversa volta a ser lida (mensagem nova sobe o `updated_at`), com a mesma situação: a
    // passagem seguinte não pode escrever nada no Synapse.
    store
        .conversas()
        .tocar(org, mundo.conversa, 3_000_000)
        .expect("toca a conversa");
    let relatorio = estado.ciclo().await.expect("segunda passagem");
    assert_eq!(relatorio.conferidas, 1, "{relatorio:?}");
    assert_eq!(relatorio.sem_mudanca, 1, "{relatorio:?}");
    assert_eq!(relatorio.avisadas, 0, "{relatorio:?}");
    assert_eq!(destino.escritas(), escritas, "nenhuma escrita nova");
    assert_eq!(destino.avisos().len(), 1);
    assert_eq!(destino.metadatas().len(), 1);
}

#[tokio::test]
async fn conversa_de_org_desligada_nao_gera_chamada() {
    let cenario = Cenario::novo("org-desligada");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org, "bia", None);
    let conversa = criar_conversa(&store, org, agente, "5511999990000");
    sala_pronta(&store, org, conversa);
    // Módulo desligado para a org (nunca ligado).
    definir_status(&store, org, conversa, StatusConversa::Transferida);
    let destino = Falso::novo();
    let estado = laco(Arc::clone(&store), Arc::clone(&destino));

    let relatorio = estado.ciclo().await.expect("passagem");
    assert_eq!(relatorio.conferidas, 0, "{relatorio:?}");
    assert_eq!(destino.escritas(), 0);
}

#[tokio::test]
async fn conversa_sem_sala_nao_avisa_e_o_cursor_passa() {
    let cenario = Cenario::novo("sem-sala");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    store.matrix().ligar(org).expect("liga");
    let agente = criar_agente(&store, org, "bia", None);
    let conversa = criar_conversa(&store, org, agente, "5511999990000");
    // Sem sala: a MX-14 é quem cria.
    definir_status(&store, org, conversa, StatusConversa::Transferida);
    let destino = Falso::novo();
    let estado = laco(Arc::clone(&store), Arc::clone(&destino));

    let relatorio = estado.ciclo().await.expect("passagem");
    assert_eq!(relatorio.sem_sala, 1, "{relatorio:?}");
    assert_eq!(destino.escritas(), 0);
    // O cursor passou: a conversa não fica presa.
    let cursores = store.matrix().ler_cursores(org).expect("cursores");
    assert!(cursores.conversas.atualizado_em > 0);
}

#[tokio::test]
async fn sala_em_criacao_espera_a_passagem_seguinte() {
    let cenario = Cenario::novo("em-criacao");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    store.matrix().ligar(org).expect("liga");
    let agente = criar_agente(&store, org, "bia", None);
    let conversa = criar_conversa(&store, org, agente, "5511999990000");
    sala_em_criacao(&store, org, conversa);
    definir_status(&store, org, conversa, StatusConversa::Transferida);
    let destino = Falso::novo();
    let estado = laco(Arc::clone(&store), Arc::clone(&destino));

    let relatorio = estado.ciclo().await.expect("passagem");
    assert_eq!(relatorio.adiadas, 1, "{relatorio:?}");
    assert_eq!(
        destino.escritas(),
        0,
        "nada no Synapse antes da sala pronta"
    );
    let cursores = store.matrix().ler_cursores(org).expect("cursores");
    assert_eq!(cursores.conversas.atualizado_em, 0, "cursor não andou");

    // A MX-11 termina a criação; a passagem seguinte avisa.
    store
        .matrix()
        .marcar_sala_pronta(org, conversa)
        .expect("marca pronta");
    let relatorio = estado.ciclo().await.expect("segunda passagem");
    assert_eq!(relatorio.avisadas, 1, "{relatorio:?}");
    assert_eq!(destino.avisos().len(), 1);
}

#[tokio::test]
async fn falha_transitoria_nao_avanca_o_cursor_e_a_passagem_seguinte_aplica() {
    let cenario = Cenario::novo("falha-transitoria");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::novo();
    destino.definir_modo(Modo::FalhaNaPrimeira);
    let estado = laco(Arc::clone(&store), Arc::clone(&destino));

    definir_status(&store, org, mundo.conversa, StatusConversa::Transferida);
    let relatorio = estado.ciclo().await.expect("passagem com falha");
    assert_eq!(relatorio.adiadas, 1, "{relatorio:?}");
    assert_eq!(destino.avisos().len(), 0);
    let cursores = store.matrix().ler_cursores(org).expect("cursores");
    assert_eq!(
        cursores.conversas.atualizado_em, 0,
        "o cursor não pode consumir a conversa em silêncio"
    );
    let sala = store
        .matrix()
        .sala(org, mundo.conversa)
        .expect("lê")
        .expect("existe");
    assert_eq!(sala.ultimo_status, None, "nada espelhado");

    // Synapse de volta: a passagem seguinte aplica o estado.
    let relatorio = estado.ciclo().await.expect("passagem seguinte");
    assert_eq!(relatorio.avisadas, 1, "{relatorio:?}");
    let avisos = destino.avisos();
    assert_eq!(avisos.len(), 1);
    assert!(avisos[0].txn.starts_with("ph_est_"));
}

#[tokio::test]
async fn erro_definitivo_pula_a_conversa_e_nao_trava_a_fila() {
    let cenario = Cenario::novo("definitivo");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::novo();
    destino.definir_modo(Modo::Recusa);
    let estado = laco(Arc::clone(&store), Arc::clone(&destino));

    definir_status(&store, org, mundo.conversa, StatusConversa::Transferida);
    let relatorio = estado.ciclo().await.expect("passagem");
    assert_eq!(relatorio.puladas, 1, "{relatorio:?}");
    assert_eq!(relatorio.adiadas, 0);
    assert!(destino.avisos().is_empty(), "nada foi escrito na sala");
    let cursores = store.matrix().ler_cursores(org).expect("cursores");
    assert!(
        cursores.conversas.atualizado_em > 0,
        "recusa definitiva não pode travar o cursor"
    );
    // A conversa volta na próxima alteração dela, com o Synapse bom.
    destino.definir_modo(Modo::Normal);
    definir_status(&store, org, mundo.conversa, StatusConversa::Fechada);
    let relatorio = estado.ciclo().await.expect("segunda passagem");
    assert_eq!(relatorio.avisadas, 1, "{relatorio:?}");
    assert_eq!(destino.avisos().len(), 1);
    assert!(destino.avisos()[0].texto.contains("Conversa encerrada"));
}

#[tokio::test]
async fn duas_conversas_no_mesmo_milissegundo_nao_se_perdem() {
    let cenario = Cenario::novo("mesmo-instante");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    store.matrix().ligar(org).expect("liga");
    let agente = criar_agente(&store, org, "bia", None);
    let uma = criar_conversa(&store, org, agente, "5511999990001");
    let outra = criar_conversa(&store, org, agente, "5511999990002");
    sala_pronta(&store, org, uma);
    sala_pronta(&store, org, outra);

    // As duas mudam no mesmo milissegundo: quem desempata é o id do cursor.
    cenario.congelar(2_000_000);
    definir_status(&store, org, uma, StatusConversa::Transferida);
    definir_status(&store, org, outra, StatusConversa::Transferida);
    let atualizadas: Vec<i64> = [uma, outra]
        .iter()
        .map(|conversa| {
            store
                .conversas()
                .obter(org, *conversa)
                .expect("existe")
                .atualizado_em
        })
        .collect();
    assert_eq!(
        atualizadas[0], atualizadas[1],
        "mesmo instante de propósito"
    );

    let destino = Falso::novo();
    let estado = laco(Arc::clone(&store), Arc::clone(&destino));
    let relatorio = estado.ciclo().await.expect("passagem");
    assert_eq!(relatorio.avisadas, 2, "{relatorio:?}");
    assert_eq!(destino.avisos().len(), 2, "nenhuma conversa ficou de fora");

    let relatorio = estado.ciclo().await.expect("repetição");
    assert_eq!(
        relatorio.avisadas, 0,
        "nada novo na repetição: {relatorio:?}"
    );
}

#[tokio::test]
async fn paginas_cobrem_todas_as_conversas_alteradas() {
    let cenario = Cenario::novo("paginas");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    store.matrix().ligar(org).expect("liga");
    let agente = criar_agente(&store, org, "bia", None);
    // Página de 2 e 5 conversas: são 3 páginas.
    let mut conversas = Vec::new();
    for n in 0..5 {
        let conversa = criar_conversa(&store, org, agente, &format!("55119999900{n:02}"));
        sala_pronta(&store, org, conversa);
        definir_status(&store, org, conversa, StatusConversa::Transferida);
        conversas.push(conversa);
    }
    let destino = Falso::novo();
    let estado = EstadoDasConversas::novo(
        PortaEstadoStore::nova(Arc::clone(&store)),
        Sinal(Arc::clone(&destino)),
        &config(),
    )
    .expect("monta")
    .com_pagina(2);

    let relatorio = estado.ciclo().await.expect("passagem");
    assert_eq!(relatorio.avisadas, 5, "{relatorio:?}");
    assert_eq!(destino.avisos().len(), 5);
    assert!(relatorio.paginas >= 3, "{relatorio:?}");
    // A página do teste é menor que a do módulo: o laço não depende do tamanho.
    const { assert!(PAGINA > 2) };
}

#[tokio::test]
async fn mudanca_de_departamento_move_a_sala_sem_aviso_duplicado() {
    let cenario = Cenario::novo("departamento");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    store.matrix().ligar(org).expect("liga");
    let suporte = criar_departamento(&store, org, "Suporte");
    let comercial = criar_departamento(&store, org, "Comercial");
    let agente = criar_agente(&store, org, "bia", Some(suporte));
    let conversa = criar_conversa(&store, org, agente, "5511999990000");
    sala_pronta(&store, org, conversa);
    // A sala já está no espaço do departamento padrão.
    store
        .matrix()
        .definir_espaco_da_sala(org, conversa, Some("!espaco-suporte:matrix-dev.local"))
        .expect("grava o espaço");

    let destino = Falso::novo();
    destino.definir_espaco(Espaco::Moveu);
    let estado = laco(Arc::clone(&store), Arc::clone(&destino));

    // Transferência para outro departamento: um aviso, um metadata e uma mudança de espaço.
    store
        .conversas()
        .transferir(org, conversa, Some(comercial), None, "{}".to_owned())
        .expect("transfere");
    let relatorio = estado.ciclo().await.expect("passagem");
    assert_eq!(relatorio.avisadas, 1, "{relatorio:?}");
    assert_eq!(relatorio.posicionadas, 1, "{relatorio:?}");
    assert_eq!(destino.avisos().len(), 1, "um aviso só");
    let posicoes = destino.posicoes();
    assert_eq!(posicoes.len(), 1);
    assert_eq!(posicoes[0].conversa, conversa);
    assert_eq!(
        posicoes[0]
            .departamento
            .as_ref()
            .map(|departamento| departamento.id),
        Some(comercial)
    );
    let metadatas = destino.metadatas();
    assert_eq!(metadatas.len(), 1);
    assert_eq!(metadatas[0].corpo["department_id"], comercial.to_string());

    // Repetir com a sala já no lugar novo: nenhum aviso novo.
    let relatorio = estado.ciclo().await.expect("repetição");
    assert_eq!(relatorio.avisadas, 0, "{relatorio:?}");
    assert_eq!(relatorio.posicionadas, 0, "{relatorio:?}");
    assert_eq!(destino.avisos().len(), 1);
}

#[tokio::test]
async fn conversa_sem_departamento_nao_fala_de_espaco() {
    let cenario = Cenario::novo("sem-departamento");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::novo();
    let estado = laco(Arc::clone(&store), Arc::clone(&destino));

    definir_status(&store, org, mundo.conversa, StatusConversa::Transferida);
    estado.ciclo().await.expect("passagem");
    assert!(
        destino.posicoes().is_empty(),
        "sem departamento, sem espaço"
    );
    assert_eq!(destino.avisos().len(), 1);
}

#[tokio::test]
async fn falha_de_espaco_nao_desfaz_o_estado_ja_espelhado() {
    let cenario = Cenario::novo("espaco-falha");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    store.matrix().ligar(org).expect("liga");
    let suporte = criar_departamento(&store, org, "Suporte");
    let agente = criar_agente(&store, org, "bia", Some(suporte));
    let conversa = criar_conversa(&store, org, agente, "5511999990000");
    sala_pronta(&store, org, conversa);
    definir_status(&store, org, conversa, StatusConversa::Transferida);

    let destino = Falso::novo();
    let estado = laco(Arc::clone(&store), Arc::clone(&destino));
    let relatorio = estado.ciclo().await.expect("passagem");

    // O espaço não tem o bot: a sala não entra, e o aviso/metadata continuam valendo.
    assert_eq!(relatorio.avisadas, 1, "{relatorio:?}");
    assert_eq!(relatorio.posicionadas, 0);
    assert_eq!(destino.avisos().len(), 1);
    let sala = store
        .matrix()
        .sala(org, conversa)
        .expect("lê")
        .expect("existe");
    assert_eq!(sala.ultimo_status, Some(StatusConversa::Transferida));
}

// ───────────────────────────── corrida da situação ─────────────────────────────

/// Porta que mente numa leitura de propósito: prova o que acontece quando a conversa muda entre a
/// leitura que dispara a passagem e a reconferência de antes da escrita.
struct PortaQueMente {
    real: PortaEstadoStore,
    leituras: AtomicUsize,
    mentira: Mutex<Option<StatusConversa>>,
}

impl PortaQueMente {
    fn nova(store: Arc<Store>) -> Self {
        Self {
            real: PortaEstadoStore::nova(store),
            leituras: AtomicUsize::new(0),
            mentira: Mutex::new(None),
        }
    }

    /// Na segunda leitura de conversa devolve esta situação.
    fn mentir_na_segunda(&self, status: StatusConversa) {
        *self.mentira.lock().unwrap() = Some(status);
    }
}

impl PortaEstado for PortaQueMente {
    async fn orgs(&self) -> Result<Vec<EstadoOrg>, poph_store::erro::ErroStore> {
        self.real.orgs().await
    }

    async fn cursores(&self, org: OrgId) -> Result<CursorGravado, poph_store::erro::ErroStore> {
        self.real.cursores(org).await
    }

    async fn alteradas(
        &self,
        apos: Option<(i64, ConversationId)>,
        limite: u32,
    ) -> Result<Vec<ConversaAlterada>, poph_store::erro::ErroStore> {
        self.real.alteradas(apos, limite).await
    }

    async fn conversa(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> Result<Option<ConversaEstado>, poph_store::erro::ErroStore> {
        let leitura = self.leituras.fetch_add(1, Ordering::SeqCst) + 1;
        let real = self.real.conversa(org, conversa).await?;
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

    async fn sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> Result<Option<SalaEstado>, poph_store::erro::ErroStore> {
        self.real.sala(org, conversa).await
    }

    async fn definir_ultimo_status(
        &self,
        org: OrgId,
        conversa: ConversationId,
        status: StatusConversa,
    ) -> Result<(), poph_store::erro::ErroStore> {
        self.real.definir_ultimo_status(org, conversa, status).await
    }

    async fn definir_nome_espelhado(
        &self,
        org: OrgId,
        conversa: ConversationId,
        nome: &str,
    ) -> Result<(), poph_store::erro::ErroStore> {
        self.real.definir_nome_espelhado(org, conversa, nome).await
    }

    async fn avancar_conversas(
        &self,
        org: OrgId,
        atualizado_em: i64,
        id: &str,
    ) -> Result<(), poph_store::erro::ErroStore> {
        self.real.avancar_conversas(org, atualizado_em, id).await
    }
}

#[tokio::test]
async fn conversa_fechada_no_meio_da_passagem_nao_recebe_metadata_de_reabertura() {
    let cenario = Cenario::novo("corrida");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    // A sala já espelhou a transferência; a conversa está aberta de novo (reabertura pendente).
    store
        .matrix()
        .definir_ultimo_status(org, mundo.conversa, StatusConversa::Transferida)
        .expect("grava a situação anterior");
    let destino = Falso::novo();
    let porta = PortaQueMente::nova(Arc::clone(&store));
    // A leitura que dispara vê a reabertura; a reconferência vê a conversa já encerrada.
    porta.mentir_na_segunda(StatusConversa::Fechada);
    let estado =
        EstadoDasConversas::novo(porta, Sinal(Arc::clone(&destino)), &config()).expect("monta");

    let relatorio = estado.ciclo().await.expect("passagem");
    assert_eq!(relatorio.adiadas, 1, "{relatorio:?}");
    assert!(
        destino.metadatas().is_empty(),
        "nenhum metadata atrasado: a conversa fechou no meio"
    );
    assert!(destino.avisos().is_empty(), "nenhum aviso atrasado");

    // Fecha a conversa de verdade: a passagem seguinte espelha o encerramento, e o metadata de
    // reabertura nunca existiu.
    definir_status(&store, org, mundo.conversa, StatusConversa::Fechada);
    let relatorio = estado.ciclo().await.expect("segunda passagem");
    assert_eq!(relatorio.avisadas, 1, "{relatorio:?}");
    let metadatas = destino.metadatas();
    assert_eq!(metadatas.len(), 1);
    assert_eq!(metadatas[0].corpo["status"], "closed");
    assert_eq!(destino.avisos().len(), 1);
    assert!(destino.avisos()[0].texto.contains("Conversa encerrada"));
}

// ───────────────────────────── integração com o banco ─────────────────────────────

#[tokio::test]
async fn porta_do_store_le_o_que_o_banco_gravou() {
    let cenario = Cenario::novo("porta");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let porta = PortaEstadoStore::nova(Arc::clone(&store));

    let orgs = porta.orgs().await.expect("orgs");
    assert_eq!(orgs.len(), 1);
    assert!(orgs[0].ligado);

    let cursores = porta.cursores(org).await.expect("cursores");
    assert_eq!(cursores.atualizado_em, 0);
    assert_eq!(cursores.id, None);

    let alteradas = porta.alteradas(None, 10).await.expect("alteradas");
    assert_eq!(alteradas.len(), 1);
    assert_eq!(alteradas[0].conversa, mundo.conversa);

    let conversa = porta
        .conversa(org, mundo.conversa)
        .await
        .expect("conversa")
        .expect("existe");
    assert_eq!(conversa.status, StatusConversa::Aberta);
    assert_eq!(conversa.agente, mundo.agente);
    assert_eq!(conversa.canal, "whatsapp");
    assert_eq!(conversa.cliente, "5511999990000");
    assert_eq!(conversa.nome_agente.as_deref(), Some("Bia"));

    let sala = porta
        .sala(org, mundo.conversa)
        .await
        .expect("sala")
        .expect("existe");
    assert!(sala.pronta);
    assert_eq!(sala.ultimo_status, None);

    porta
        .definir_ultimo_status(org, mundo.conversa, StatusConversa::Transferida)
        .await
        .expect("grava situação");
    let sala = porta
        .sala(org, mundo.conversa)
        .await
        .expect("sala")
        .expect("existe");
    assert_eq!(sala.ultimo_status, Some(StatusConversa::Transferida));

    porta
        .avancar_conversas(org, 5_000, &mundo.conversa.to_string())
        .await
        .expect("avança");
    let cursores = porta.cursores(org).await.expect("cursores");
    assert_eq!(cursores.atualizado_em, 5_000);
    assert_eq!(cursores.id, Some(mundo.conversa));
}

#[tokio::test]
async fn conversa_apagada_entre_a_pagina_e_a_leitura_nao_quebra_a_passagem() {
    let cenario = Cenario::novo("apagada");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::novo();

    // Porta que lista uma conversa que não existe mais (a remoção acontece entre as duas leituras).
    let porta = PortaQueApaga {
        real: PortaEstadoStore::nova(Arc::clone(&store)),
    };
    let estado =
        EstadoDasConversas::novo(porta, Sinal(Arc::clone(&destino)), &config()).expect("monta");
    let relatorio = estado.ciclo().await.expect("passagem");
    // A conversa de verdade é conferida (e nada muda); a inventada não quebra a passagem.
    assert_eq!(relatorio.conferidas, 1, "{relatorio:?}");
    assert_eq!(relatorio.sem_mudanca, 1, "{relatorio:?}");
    assert_eq!(relatorio.adiadas, 0, "{relatorio:?}");
    assert_eq!(destino.escritas(), 0);
    let cursores = store.matrix().ler_cursores(org).expect("cursores");
    assert!(
        cursores.conversas.atualizado_em >= INSTANTE_INVENTADO,
        "conversa que sumiu não prende o cursor"
    );
    let _ = mundo;
}

/// Instante da conversa que a porta inventa (maior que qualquer conversa do cenário).
const INSTANTE_INVENTADO: i64 = 9_000_000;

/// Porta que lista uma conversa inventada, que não existe no banco.
struct PortaQueApaga {
    real: PortaEstadoStore,
}

impl PortaEstado for PortaQueApaga {
    async fn orgs(&self) -> Result<Vec<EstadoOrg>, poph_store::erro::ErroStore> {
        self.real.orgs().await
    }

    async fn cursores(&self, org: OrgId) -> Result<CursorGravado, poph_store::erro::ErroStore> {
        self.real.cursores(org).await
    }

    async fn alteradas(
        &self,
        apos: Option<(i64, ConversationId)>,
        limite: u32,
    ) -> Result<Vec<ConversaAlterada>, poph_store::erro::ErroStore> {
        let reais = self.real.alteradas(apos, limite).await?;
        let mut lista = reais;
        let org = lista.first().map(|alterada| alterada.org);
        if let Some(org) = org {
            // A conversa que o laço vai procurar não está mais no banco.
            lista.push(ConversaAlterada {
                org,
                conversa: ConversationId::generate(),
                atualizado_em: INSTANTE_INVENTADO,
            });
        }
        Ok(lista)
    }

    async fn conversa(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> Result<Option<ConversaEstado>, poph_store::erro::ErroStore> {
        self.real.conversa(org, conversa).await
    }

    async fn sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> Result<Option<SalaEstado>, poph_store::erro::ErroStore> {
        self.real.sala(org, conversa).await
    }

    async fn definir_ultimo_status(
        &self,
        org: OrgId,
        conversa: ConversationId,
        status: StatusConversa,
    ) -> Result<(), poph_store::erro::ErroStore> {
        self.real.definir_ultimo_status(org, conversa, status).await
    }

    async fn definir_nome_espelhado(
        &self,
        org: OrgId,
        conversa: ConversationId,
        nome: &str,
    ) -> Result<(), poph_store::erro::ErroStore> {
        self.real.definir_nome_espelhado(org, conversa, nome).await
    }

    async fn avancar_conversas(
        &self,
        org: OrgId,
        atualizado_em: i64,
        id: &str,
    ) -> Result<(), poph_store::erro::ErroStore> {
        self.real.avancar_conversas(org, atualizado_em, id).await
    }
}

#[tokio::test]
async fn resposta_e_metadata_nao_levam_dado_de_cliente() {
    let cenario = Cenario::novo("sem-dado");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::novo();
    let estado = laco(Arc::clone(&store), Arc::clone(&destino));

    definir_status(&store, org, mundo.conversa, StatusConversa::Transferida);
    estado.ciclo().await.expect("passagem");

    for aviso in destino.avisos() {
        assert!(
            !aviso.texto.contains("5511999990000"),
            "o aviso não pode ter o telefone: {}",
            aviso.texto
        );
    }
    for metadata in destino.metadatas() {
        let texto = metadata.corpo.to_string();
        // O metadata leva o identificador do cliente (o Element precisa dele), nunca o nome.
        assert!(texto.contains("conversation_id"));
        assert!(!texto.contains("\"client_name\""), "{texto}");
    }
}

// ───────────────────────────── nome do cliente (MX-33) ─────────────────────────────

/// Grava o nome do cliente pelo caminho de verdade da MX-32 (sobe `updated_at`, não `version`).
fn definir_nome(store: &Store, org: OrgId, conversa: ConversationId, nome: &str) {
    let mudou = store
        .conversas()
        .definir_nome_cliente(org, conversa, nome)
        .expect("grava o nome do cliente");
    assert!(mudou, "o nome precisa ter mudado para o cursor enxergar");
}

/// Nome espelhado que a linha da sala guarda (o que a MX-33 compara).
fn nome_espelhado(store: &Store, org: OrgId, conversa: ConversationId) -> Option<String> {
    store
        .matrix()
        .sala(org, conversa)
        .expect("lê a sala")
        .expect("sala existe")
        .nome_cliente_espelhado
}

/// Toca a conversa sem mudar nada: o cursor volta a vê-la, como uma mensagem nova do cliente.
///
/// O relógio do cenário anda junto, para as escritas seguintes (o nome novo, por exemplo) ficarem
/// depois deste instante — senão ficariam atrás do cursor.
fn tocar(cenario: &Cenario, store: &Store, org: OrgId, conversa: ConversationId) {
    let quando = cenario.relogio.agora.fetch_add(60_000, Ordering::SeqCst) + 60_000;
    store
        .conversas()
        .tocar(org, conversa, quando)
        .expect("toca a conversa");
}

#[tokio::test]
async fn nome_que_chega_depois_renomeia_sala_metadata_e_fantasma_uma_vez_cada() {
    let cenario = Cenario::novo("nome-depois");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::novo();
    let estado = laco(Arc::clone(&store), Arc::clone(&destino));
    // A sala nasceu com o telefone: nenhum nome espelhado.
    assert_eq!(nome_espelhado(&store, org, mundo.conversa), None);

    definir_nome(&store, org, mundo.conversa, "Ana Souza");
    let relatorio = estado.ciclo().await.expect("passagem do nome");

    assert_eq!(relatorio.renomeadas, 1, "{relatorio:?}");
    let renomeacoes = destino.renomeacoes();
    assert_eq!(renomeacoes.len(), 1, "uma renomeação de sala");
    assert_eq!(renomeacoes[0].sala, sala_de(mundo.conversa));
    assert_eq!(renomeacoes[0].nome, "Ana Souza");

    let metadatas = destino.metadatas();
    assert_eq!(metadatas.len(), 1, "um metadata com o nome");
    assert_eq!(metadatas[0].corpo["client_name"], "Ana Souza");
    assert_eq!(metadatas[0].corpo["status"], "active");

    let fantasmas = destino.fantasmas();
    assert_eq!(fantasmas.len(), 1, "um nome de fantasma");
    assert_eq!(fantasmas[0].cliente, "5511999990000");
    assert_eq!(fantasmas[0].nome, "Ana Souza");

    assert!(destino.avisos().is_empty(), "trocar nome não é aviso");
    assert_eq!(
        nome_espelhado(&store, org, mundo.conversa).as_deref(),
        Some("Ana Souza"),
        "o nome só é dado como espelhado no fim"
    );

    // Repetir o ciclo (a conversa volta a ser vista) não fala com o Synapse.
    let escritas = destino.escritas();
    tocar(&cenario, &store, org, mundo.conversa);
    let relatorio = estado.ciclo().await.expect("passagem repetida");
    assert_eq!(relatorio.renomeadas, 0, "{relatorio:?}");
    assert_eq!(destino.escritas(), escritas, "repetir não escreve nada");
    assert_eq!(destino.renomeacoes().len(), 1);
    assert_eq!(destino.metadatas().len(), 1);
    assert_eq!(destino.fantasmas().len(), 1);

    // Um segundo nome novo atualiza de novo, uma vez cada.
    definir_nome(&store, org, mundo.conversa, "Ana Maria");
    let relatorio = estado.ciclo().await.expect("passagem do nome novo");
    assert_eq!(relatorio.renomeadas, 1, "{relatorio:?}");
    assert_eq!(destino.renomeacoes().len(), 2);
    assert_eq!(destino.renomeacoes()[1].nome, "Ana Maria");
    assert_eq!(destino.metadatas()[1].corpo["client_name"], "Ana Maria");
    assert_eq!(destino.fantasmas()[1].nome, "Ana Maria");
    assert_eq!(
        nome_espelhado(&store, org, mundo.conversa).as_deref(),
        Some("Ana Maria")
    );
}

#[tokio::test]
async fn queda_entre_o_nome_e_o_metadata_retoma_sem_sala_nem_aviso_duplicado() {
    let cenario = Cenario::novo("queda-no-nome");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::novo();
    let estado = laco(Arc::clone(&store), Arc::clone(&destino));

    definir_nome(&store, org, mundo.conversa, "Ana Souza");
    // A segunda escrita da passagem (o metadata) falha de forma transitória: é a queda entre as
    // três chamadas do nome.
    *destino.modo.lock().unwrap() = Modo::FalhaNaEscrita(2);
    let relatorio = estado.ciclo().await.expect("passagem que cai");

    assert_eq!(relatorio.adiadas, 1, "{relatorio:?}");
    assert_eq!(destino.renomeacoes().len(), 1, "a sala foi renomeada");
    assert!(destino.metadatas().is_empty(), "o metadata não saiu");
    assert!(
        destino.fantasmas().is_empty(),
        "o fantasma vem depois do metadata"
    );
    assert_eq!(
        nome_espelhado(&store, org, mundo.conversa),
        None,
        "queda no meio não dá o nome como espelhado"
    );

    // A passagem seguinte refaz o caminho: as chamadas são idempotentes, e nada duplica.
    let relatorio = estado.ciclo().await.expect("passagem da retomada");
    assert_eq!(relatorio.renomeadas, 1, "{relatorio:?}");
    assert_eq!(destino.renomeacoes().len(), 2, "a renomeação se repete");
    assert_eq!(destino.renomeacoes()[1].nome, "Ana Souza");
    assert_eq!(destino.metadatas().len(), 1, "um metadata só");
    assert_eq!(destino.fantasmas().len(), 1);
    assert!(destino.avisos().is_empty(), "nenhum aviso de atendimento");
    // A sala continua sendo uma só: a retomada não cria outra.
    assert_eq!(
        store
            .matrix()
            .sala(org, mundo.conversa)
            .expect("lê a sala")
            .expect("sala existe")
            .room_id,
        sala_de(mundo.conversa)
    );
    assert_eq!(
        nome_espelhado(&store, org, mundo.conversa).as_deref(),
        Some("Ana Souza")
    );
}

#[tokio::test]
async fn nome_atrasado_nao_devolve_conversa_fechada_para_active_no_metadata() {
    let cenario = Cenario::novo("nome-atrasado");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::novo();

    // A conversa é encerrada e o nome do cliente chega logo depois. A sala ainda não espelhou o
    // encerramento (`last_status` vazio = conversa aberta), e é essa a leitura que a porta atrasada
    // devolve: por isso a renomeação precisa reler antes de escrever o metadata.
    definir_status(&store, org, mundo.conversa, StatusConversa::Fechada);
    definir_nome(&store, org, mundo.conversa, "Ana Souza");

    // A porta devolve a situação velha na primeira leitura da passagem: o laço lê a conversa como
    // aberta e a renomeação só não publica `active` porque relê antes de escrever.
    let atrasada = PortaAtrasada::nova(Arc::clone(&store));
    let estado = EstadoDasConversas::novo(atrasada, Sinal(Arc::clone(&destino)), &config())
        .expect("monta o laço");

    let relatorio = estado.ciclo().await.expect("passagem do nome");
    assert_eq!(relatorio.renomeadas, 1, "{relatorio:?}");
    let metadatas = destino.metadatas();
    assert_eq!(metadatas.len(), 1);
    assert_eq!(
        metadatas[0].corpo["status"], "closed",
        "o metadata do nome sai com o estado atual da conversa"
    );
    assert_eq!(metadatas[0].corpo["client_name"], "Ana Souza");
    assert!(
        destino.avisos().is_empty(),
        "renomear não é aviso e a leitura atrasada não gerou aviso"
    );
    // A conversa continua encerrada (o nome atrasado não reabriu nada).
    assert_eq!(
        store
            .conversas()
            .obter(org, mundo.conversa)
            .expect("lê a conversa")
            .status,
        StatusConversa::Fechada
    );

    // O `last_status` não foi tocado pela renomeação: quem espelha a situação é o passo do estado.
    assert_eq!(
        store
            .matrix()
            .sala(org, mundo.conversa)
            .expect("lê a sala")
            .expect("sala existe")
            .ultimo_status,
        None,
        "renomear não espelha situação"
    );
}

#[tokio::test]
async fn conversa_sem_nome_novo_e_org_desligada_nao_falam_com_o_synapse() {
    let cenario = Cenario::novo("sem-nome-novo");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let mundo = mundo(&store, org);
    let destino = Falso::novo();
    let estado = laco(Arc::clone(&store), Arc::clone(&destino));

    // Sem nome do cliente e com a sala já pronta: a passagem não escreve nada.
    tocar(&cenario, &store, org, mundo.conversa);
    let relatorio = estado.ciclo().await.expect("passagem sem nome");
    assert_eq!(relatorio.renomeadas, 0, "{relatorio:?}");
    assert!(destino.renomeacoes().is_empty());
    assert!(destino.metadatas().is_empty());
    assert!(destino.fantasmas().is_empty());

    // Nome igual ao já espelhado também não escreve.
    definir_nome(&store, org, mundo.conversa, "Ana Souza");
    estado.ciclo().await.expect("passagem do nome");
    let escritas = destino.escritas();
    tocar(&cenario, &store, org, mundo.conversa);
    estado.ciclo().await.expect("passagem repetida");
    assert_eq!(destino.escritas(), escritas);

    // Org com o módulo desligado não gera chamada nenhuma, mesmo com nome novo na conversa.
    store.matrix().desligar(org).expect("desliga a org");
    definir_nome(&store, org, mundo.conversa, "Ana Maria");
    let escritas = destino.escritas();
    let relatorio = estado.ciclo().await.expect("passagem com a org desligada");
    assert_eq!(relatorio.renomeadas, 0, "{relatorio:?}");
    assert_eq!(destino.escritas(), escritas, "org desligada não escreve");
    assert_eq!(
        nome_espelhado(&store, org, mundo.conversa).as_deref(),
        Some("Ana Souza"),
        "o nome espelhado fica como estava"
    );
}

/// Porta que devolve a situação **aberta** na primeira leitura de cada conversa (o atraso do ciclo)
/// e a situação real nas seguintes.
struct PortaAtrasada {
    real: PortaEstadoStore,
    atrasadas: Mutex<std::collections::BTreeSet<String>>,
}

impl PortaAtrasada {
    fn nova(store: Arc<Store>) -> Self {
        Self {
            real: PortaEstadoStore::nova(store),
            atrasadas: Mutex::new(std::collections::BTreeSet::new()),
        }
    }
}

impl PortaEstado for PortaAtrasada {
    async fn orgs(&self) -> Result<Vec<EstadoOrg>, poph_store::erro::ErroStore> {
        self.real.orgs().await
    }

    async fn cursores(&self, org: OrgId) -> Result<CursorGravado, poph_store::erro::ErroStore> {
        self.real.cursores(org).await
    }

    async fn alteradas(
        &self,
        apos: Option<(i64, ConversationId)>,
        limite: u32,
    ) -> Result<Vec<ConversaAlterada>, poph_store::erro::ErroStore> {
        self.real.alteradas(apos, limite).await
    }

    async fn conversa(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> Result<Option<ConversaEstado>, poph_store::erro::ErroStore> {
        let primeira = self.atrasadas.lock().unwrap().insert(conversa.to_string());
        let real = self.real.conversa(org, conversa).await?;
        if !primeira {
            return Ok(real);
        }
        // Primeira leitura da passagem: a conversa ainda parece aberta.
        Ok(real.map(|mut vista| {
            vista.status = StatusConversa::Aberta;
            vista
        }))
    }

    async fn sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> Result<Option<SalaEstado>, poph_store::erro::ErroStore> {
        self.real.sala(org, conversa).await
    }

    async fn definir_ultimo_status(
        &self,
        org: OrgId,
        conversa: ConversationId,
        status: StatusConversa,
    ) -> Result<(), poph_store::erro::ErroStore> {
        self.real.definir_ultimo_status(org, conversa, status).await
    }

    async fn definir_nome_espelhado(
        &self,
        org: OrgId,
        conversa: ConversationId,
        nome: &str,
    ) -> Result<(), poph_store::erro::ErroStore> {
        self.real.definir_nome_espelhado(org, conversa, nome).await
    }

    async fn avancar_conversas(
        &self,
        org: OrgId,
        atualizado_em: i64,
        id: &str,
    ) -> Result<(), poph_store::erro::ErroStore> {
        self.real.avancar_conversas(org, atualizado_em, id).await
    }
}

#[test]
fn erro_de_relatorio_tem_codigo_curto() {
    // O código do erro do banco é curto e não carrega SQL nem valor de linha.
    let erro = ErroEstado::Armazenamento(poph_store::erro::ErroStore::EscritorEncerrado);
    assert_eq!(erro.codigo(), "armazenamento_indisponivel");
}
