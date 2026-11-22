//! Espelho de estado da conversa (MX-15): o que acontece na conversa aparece na sala.
//!
//! O laço de mensagens (MX-14) leva o texto para a sala; este leva o **estado**: quando a conversa
//! é transferida para uma pessoa, reaberta ou encerrada, a sala ganha um aviso curto (`m.notice`) e
//! o evento de metadata que o Element lê é reescrito. Quando a transferência muda o departamento, a
//! sala **muda de espaço** pelo mesmo caminho da MX-12.
//!
//! ## O que cada passagem faz
//!
//! 1. Lê as orgs com o módulo ligado e o cursor de conversas de cada uma (`matrix_estado`): o par
//!    (`updated_at`, id), não só o instante — duas conversas alteradas no mesmo milissegundo não se
//!    perdem.
//! 2. Lê `conversations` a partir do menor cursor, em páginas, e compara a situação de cada
//!    conversa com o que a sala já espelhou (`matrix_rooms.last_status`).
//! 3. Situação diferente: aviso na sala e metadata reescrito, **nesta ordem**, e só então o
//!    `last_status` é gravado. Mudança só de departamento: a sala muda de espaço, sem aviso.
//! 4. Nome do cliente que chegou (ou mudou) depois da criação da sala (`MX-33`): a sala é renomeada,
//!    o metadata é reescrito com o estado **atual** da conversa, o fantasma do cliente recebe o nome
//!    novo e só então o nome é dado como espelhado.
//! 5. Situação igual, nome igual e sala já no espaço certo: nenhuma escrita no Synapse.
//!
//! ## Regras que valem a pena saber
//!
//! - **O aviso não se repete.** O `txnId` do aviso é derivado de (conversa, instante da alteração,
//!   situação): repetir a passagem depois de uma falha não duplica o aviso (P7), e uma segunda
//!   transferência da mesma conversa tem outro instante — logo, outro aviso.
//! - **Falha transitória não consome a conversa.** Se o aviso ou o metadata não saírem, o cursor da
//!   org **não** avança por essa conversa: a passagem seguinte tenta de novo, com atraso dobrando.
//! - **Falha definitiva não trava a fila.** Aviso recusado com 4xx (fora 429) é registrado com o
//!   código do erro e contado em métrica; a conversa é pulada e volta na próxima alteração dela.
//!   Não há tabela de falhas do estado: a spec da MX-15 não prevê uma, e inventar outra seria mudar
//!   o esquema.
//! - **`closed` não volta a `active` por atraso.** Antes de escrever o metadata, a conversa é lida de
//!   novo: se a situação mudou no meio da passagem, nada é escrito e a conversa fica para a passagem
//!   seguinte, com a situação nova.
//! - **Sem sala, sem aviso.** Conversa que ainda não tem sala (a MX-14 é quem a cria, quando há
//!   mensagem para espelhar) é pulada: não há onde avisar.
//! - **Falha de espaço não trava o estado.** Mover a sala de espaço depende de permissão no espaço
//!   (um convite que o operador pode ter desfeito). Quando falha, o aviso e o metadata que já
//!   saíram ficam valendo, e o movimento é tentado de novo na próxima alteração da conversa.
//! - **Renomear não é aviso.** Trocar o nome da sala quando o nome do cliente aparece não gera
//!   `m.notice`: não é evento de atendimento. O nome nunca volta a vazio (conversa sem nome mantém
//!   o que a sala já tem) e não há varredura de backfill: a sala antiga se corrige na próxima
//!   alteração da conversa. Se um atendente renomeou a sala à mão, o nome do cliente sobrescreve.
//! - **O nome do cliente não vai para o log**: as linhas levam org, conversa e o código do erro.
//!
//! ## Fronteira
//!
//! A decisão mora aqui, em `poph-server`, com as duas bordas injetadas — a mesma escolha do
//! espelhamento (MX-14) e da resposta do atendente (MX-18):
//!
//! - [`PortaEstado`] é a persistência (conversas, sala e cursor); a implementação real é a
//!   [`PortaEstadoStore`], sobre o `Store`, e os testes injetam a delas.
//! - [`DestinoEstado`] é o lado Synapse (aviso, metadata e espaço); a implementação real é o
//!   [`DestinoDoSynapse`], e os testes usam um duplo que conta as chamadas.
//!
//! `poph-matrix` continua sem conhecer o banco, e `poph-agent` continua sem conhecer o módulo.
//! Nada aqui guarda ou registra texto de cliente, telefone ou token: o log leva org, conversa,
//! situação e o código do erro.
//!
//! ## Como ligar (MX-20)
//!
//! ```no_run
//! # use std::sync::Arc;
//! # use poph_server::matrix_estado::{DestinoDoSynapse, EstadoDasConversas, PortaEstadoStore};
//! # fn exemplo(
//! #     store: Arc<poph_store::Store>,
//! #     config: &poph_types::MatrixConfig,
//! #     cliente: poph_matrix::ClienteMatrix,
//! #     ids: poph_matrix::IdsFantasmas,
//! #     espacos: poph_matrix::Espacos<impl poph_matrix::PortaEspacos + Send + Sync + 'static>,
//! # ) -> Result<(), Box<dyn std::error::Error>> {
//! // O `poph serve` monta a porta, o destino e a peça, e sobe a peça numa tarefa própria.
//! // A porta dos espaços (MX-12) chega pronta de quem liga o módulo.
//! let porta = PortaEstadoStore::nova(Arc::clone(&store));
//! // Os fantasmas entram porque renomear a sala passa pelo serviço da MX-10 (`MX-33`).
//! let usuarios = Arc::new(poph_matrix::UsuariosVirtuais::novo(
//!     cliente.clone(),
//!     ids,
//!     poph_matrix::TETO_PADRAO_CACHE,
//! ));
//! let destino = DestinoDoSynapse::novo(cliente, usuarios, espacos);
//! let estado = Arc::new(EstadoDasConversas::novo(porta, destino, config)?);
//! tokio::spawn(estado.executar());
//! # Ok(())
//! # }
//! ```

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poph_matrix::MembroSala;
use poph_matrix::cliente::ClienteMatrix;
use poph_matrix::erro::{Classificacao, ErroChamada};
use poph_matrix::espacos::{
    DepartamentoEspaco, ErroEspaco, Espacos, PortaEspacos, Posicao, SalaParaEspaco,
    resolver_departamento,
};
use poph_matrix::salas::{ConversaParaSala, MetadataSala, TIPO_METADATA, TIPO_NOME};
use poph_matrix::usuarios::{ErroFantasmas, IdsFantasmas, UsuariosVirtuais};
use poph_store::erro::ErroStore;
use poph_store::{EstadoSala, StatusConversa, Store};
use poph_types::{AgentId, ConversationId, MatrixConfig, OrgId};
use serde_json::json;

use crate::matrix_metricas::MetricasMatrix;

/// Conversas conferidas por página. Define o teto de memória de uma passagem.
pub const PAGINA: u32 = 200;

/// Páginas por passagem. Depois disso a passagem termina e a próxima continua do cursor.
pub const MAX_PAGINAS_POR_CICLO: u32 = 20;

/// Espera máxima entre passagens com falha transitória.
pub const TETO_ATRASO: Duration = Duration::from_secs(60);

/// Teto do cache de "quem avisa nesta sala" (por processo).
pub const TETO_PADRAO_REMETENTES: usize = 1024;

/// Aviso quando uma pessoa da equipe assume a conversa.
pub const AVISO_TRANSFERIDA: &str =
    "Atendimento humano assumiu a conversa: o agente automático está pausado até ela ser reaberta.";

/// Aviso quando o agente volta a atender.
pub const AVISO_REABERTA: &str = "O agente automático retomou o atendimento desta conversa.";

/// Aviso quando a conversa é encerrada.
pub const AVISO_ENCERRADA: &str =
    "Conversa encerrada. Uma nova mensagem do cliente abre uma conversa nova.";

/// `msgtype` do aviso na sala.
const MSGTYPE_AVISO: &str = "m.notice";

/// Marca do `txnId` dos avisos de estado.
const MARCA_AVISO: &str = "ph_est";

/// Id mínimo de conversa, usado só para comparar o cursor de uma org que nunca espelhou nada.
///
/// Os ids de conversa são UUID v7 (começam por um instante em hexadecimal), então o UUID nulo é
/// menor que qualquer um deles: o par (`at`, nulo) fica antes de toda conversa do mesmo instante.
const ID_MINIMO: &str = "00000000-0000-0000-0000-000000000000";

// ───────────────────────────── o que o estado precisa saber ─────────────────────────────

/// Org com o módulo ligado ou desligado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EstadoOrg {
    /// Org dona.
    pub org: OrgId,
    /// Módulo ligado.
    pub ligado: bool,
}

/// Uma conversa alterada, como a leitura incremental a devolve: só o que o cursor precisa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversaAlterada {
    /// Org dona.
    pub org: OrgId,
    /// Conversa alterada.
    pub conversa: ConversationId,
    /// Instante da alteração (ms): a primeira metade do cursor.
    pub atualizado_em: i64,
}

/// Cursor de conversas gravado para uma org.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorGravado {
    /// Instante da última conversa conferida (ms).
    pub atualizado_em: i64,
    /// Id da última conversa conferida nesse instante; `None` quando a org nunca espelhou nada.
    pub id: Option<ConversationId>,
}

/// A conversa como o estado precisa vê-la, com o departamento e o agente já resolvidos em nome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversaEstado {
    /// Org dona.
    pub org: OrgId,
    /// Conversa.
    pub conversa: ConversationId,
    /// Situação atual.
    pub status: StatusConversa,
    /// Agente que atende.
    pub agente: AgentId,
    /// Nome do agente (vazio cai no id).
    pub nome_agente: Option<String>,
    /// Canal da conversa (`whatsapp`, `console`…).
    pub canal: String,
    /// Identificador do cliente no canal (o telefone, no WhatsApp).
    pub cliente: String,
    /// Nome do cliente, quando o Harness tiver um. Hoje é sempre `None`: a coluna
    /// `conversations.client_name` entra na MX-32 e o preenchimento aqui é da MX-33.
    pub nome_cliente: Option<String>,
    /// Departamento alvo da transferência, com o nome que o espaço mostra.
    pub departamento_alvo: Option<DepartamentoEspaco>,
    /// Departamento padrão do agente (P3: é o segundo da ordem de resolução).
    pub departamento_padrao: Option<DepartamentoEspaco>,
    /// Instante da alteração (ms), o mesmo que a leitura incremental devolve.
    pub atualizado_em: i64,
}

impl ConversaEstado {
    /// Departamento que manda a sala para um espaço (P3, com a função da MX-12).
    pub fn departamento(&self) -> Option<DepartamentoEspaco> {
        let alvo = self
            .departamento_alvo
            .as_ref()
            .map(|departamento| departamento.id);
        let padrao = self
            .departamento_padrao
            .as_ref()
            .map(|departamento| departamento.id);
        let escolhido = resolver_departamento(alvo, padrao)?;
        self.departamento_alvo
            .iter()
            .chain(self.departamento_padrao.iter())
            .find(|departamento| departamento.id == escolhido)
            .cloned()
    }

    /// A conversa como a provisão de sala e o metadata a enxergam.
    pub fn para_sala(&self, departamento: Option<DepartamentoEspaco>) -> ConversaParaSala {
        ConversaParaSala {
            org: self.org,
            conversa: self.conversa,
            cliente: self.cliente.clone(),
            nome_cliente: self.nome_cliente.clone(),
            agente: self.agente,
            nome_agente: self.nome_agente.clone(),
            canal: self.canal.clone(),
            departamento: departamento.map(|departamento| departamento.id),
            fechada: self.status == StatusConversa::Fechada,
            // O corte do backfill é da provisão da sala (MX-11); aqui só o metadata é montado.
            corte_backfill: None,
        }
    }
}

/// A sala da conversa, como o estado precisa vê-la.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SalaEstado {
    /// Id da sala no Synapse.
    pub room_id: String,
    /// Espaço do departamento em que a sala está gravada, quando houver.
    pub espaco: Option<String>,
    /// Última situação já espelhada (`None` quando a sala nunca teve o estado espelhado).
    pub ultimo_status: Option<StatusConversa>,
    /// Último nome do cliente já enviado ao Matrix (nome da sala, `client_name` do metadata e nome
    /// de exibição do fantasma). `None` = a sala ainda não recebeu nome (MX-33).
    pub nome_espelhado: Option<String>,
    /// A criação da sala já terminou (`creating` → `ready`).
    pub pronta: bool,
}

// ───────────────────────────── porta de persistência ─────────────────────────────

/// O que o espelho de estado precisa do banco.
///
/// A assinatura devolve `impl Future + Send` (e não `async fn`) porque quem liga o módulo sobe o
/// laço numa tarefa própria, como nas portas da MX-11 e da MX-18.
pub trait PortaEstado: Send + Sync {
    /// Orgs conhecidas, com o módulo ligado ou não.
    fn orgs(&self) -> impl Future<Output = Result<Vec<EstadoOrg>, ErroStore>> + Send;

    /// Cursor de conversas gravado para a org.
    fn cursores(&self, org: OrgId)
    -> impl Future<Output = Result<CursorGravado, ErroStore>> + Send;

    /// Conversas alteradas depois do par dado, em ordem, no máximo `limite`.
    fn alteradas(
        &self,
        apos: Option<(i64, ConversationId)>,
        limite: u32,
    ) -> impl Future<Output = Result<Vec<ConversaAlterada>, ErroStore>> + Send;

    /// A conversa, com agente e departamentos já resolvidos (`None` quando não existe mais).
    fn conversa(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> impl Future<Output = Result<Option<ConversaEstado>, ErroStore>> + Send;

    /// A sala da conversa (`None` quando ainda não existe).
    fn sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> impl Future<Output = Result<Option<SalaEstado>, ErroStore>> + Send;

    /// Grava a situação já espelhada.
    fn definir_ultimo_status(
        &self,
        org: OrgId,
        conversa: ConversationId,
        status: StatusConversa,
    ) -> impl Future<Output = Result<(), ErroStore>> + Send;

    /// Grava o último nome do cliente já espelhado na sala (MX-33), idempotente.
    fn definir_nome_espelhado(
        &self,
        org: OrgId,
        conversa: ConversationId,
        nome: &str,
    ) -> impl Future<Output = Result<(), ErroStore>> + Send;

    /// Avança o cursor de conversas da org (só para a frente).
    fn avancar_conversas(
        &self,
        org: OrgId,
        atualizado_em: i64,
        id: &str,
    ) -> impl Future<Output = Result<(), ErroStore>> + Send;
}

/// A porta de verdade, sobre o banco do Harness.
#[derive(Debug, Clone)]
pub struct PortaEstadoStore {
    store: Arc<Store>,
}

impl PortaEstadoStore {
    /// Monta a porta sobre o banco.
    pub fn nova(store: Arc<Store>) -> Self {
        Self { store }
    }
}

impl PortaEstado for PortaEstadoStore {
    fn orgs(&self) -> impl Future<Output = Result<Vec<EstadoOrg>, ErroStore>> + Send {
        let store = Arc::clone(&self.store);
        async move {
            let estados = store.matrix().orgs_ligadas()?;
            Ok(estados
                .into_iter()
                .map(|estado| EstadoOrg {
                    org: estado.org_id,
                    ligado: estado.ligado,
                })
                .collect())
        }
    }

    fn cursores(
        &self,
        org: OrgId,
    ) -> impl Future<Output = Result<CursorGravado, ErroStore>> + Send {
        let store = Arc::clone(&self.store);
        async move {
            let cursores = store.matrix().ler_cursores(org)?.conversas;
            Ok(CursorGravado {
                atualizado_em: cursores.atualizado_em,
                id: cursores.id.parse::<ConversationId>().ok(),
            })
        }
    }

    fn alteradas(
        &self,
        apos: Option<(i64, ConversationId)>,
        limite: u32,
    ) -> impl Future<Output = Result<Vec<ConversaAlterada>, ErroStore>> + Send {
        let store = Arc::clone(&self.store);
        async move {
            let conversas = store.conversas().alteradas_desde(apos, limite)?;
            Ok(conversas
                .into_iter()
                .map(|conversa| ConversaAlterada {
                    org: conversa.org_id,
                    conversa: conversa.id,
                    atualizado_em: conversa.atualizado_em,
                })
                .collect())
        }
    }

    fn conversa(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> impl Future<Output = Result<Option<ConversaEstado>, ErroStore>> + Send {
        let store = Arc::clone(&self.store);
        async move {
            let lida = match store.conversas().obter(org, conversa) {
                Ok(lida) => lida,
                Err(erro) if erro.eh_nao_encontrado() => return Ok(None),
                Err(erro) => return Err(erro),
            };
            let agente = store.agentes().obter(org, lida.agent_id).ok();
            let departamento = |id: poph_types::DepartmentId| {
                store
                    .departamentos()
                    .obter(org, id)
                    .ok()
                    .map(|departamento| DepartamentoEspaco {
                        id: departamento.id,
                        nome: departamento.nome,
                    })
            };
            // Mesma regra que a listagem usa para `external_peer` (RP-02): um só lugar, na store.
            let identificador = lida.identificador_cliente();
            Ok(Some(ConversaEstado {
                org,
                conversa,
                status: lida.status,
                agente: lida.agent_id,
                nome_agente: agente.as_ref().map(|agente| agente.nome.clone()),
                canal: lida.canal.clone(),
                cliente: identificador,
                // Nome do cliente (`pushName`, MX-32). Sem ele o metadata sai sem `client_name` —
                // e uma transferência apagaria o nome que a criação da sala já tinha posto.
                nome_cliente: lida.nome_cliente.clone(),
                departamento_alvo: lida.departamento_alvo.and_then(departamento),
                departamento_padrao: agente
                    .as_ref()
                    .and_then(|agente| agente.departamento_padrao)
                    .and_then(departamento),
                atualizado_em: lida.atualizado_em,
            }))
        }
    }

    fn sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> impl Future<Output = Result<Option<SalaEstado>, ErroStore>> + Send {
        let store = Arc::clone(&self.store);
        async move {
            let sala = store.matrix().sala(org, conversa)?;
            Ok(sala.map(|sala| SalaEstado {
                room_id: sala.room_id,
                espaco: sala.espaco,
                ultimo_status: sala.ultimo_status,
                nome_espelhado: sala.nome_cliente_espelhado,
                pronta: sala.estado == EstadoSala::Pronta,
            }))
        }
    }

    fn definir_ultimo_status(
        &self,
        org: OrgId,
        conversa: ConversationId,
        status: StatusConversa,
    ) -> impl Future<Output = Result<(), ErroStore>> + Send {
        let store = Arc::clone(&self.store);
        async move {
            store
                .matrix()
                .definir_ultimo_status(org, conversa, status)
                .map(|_| ())
        }
    }

    fn definir_nome_espelhado(
        &self,
        org: OrgId,
        conversa: ConversationId,
        nome: &str,
    ) -> impl Future<Output = Result<(), ErroStore>> + Send {
        let store = Arc::clone(&self.store);
        let nome = nome.to_owned();
        async move {
            store
                .matrix()
                .definir_nome_cliente_espelhado(org, conversa, &nome)
                .map(|_| ())
        }
    }

    fn avancar_conversas(
        &self,
        org: OrgId,
        atualizado_em: i64,
        id: &str,
    ) -> impl Future<Output = Result<(), ErroStore>> + Send {
        let store = Arc::clone(&self.store);
        let id = id.to_owned();
        async move {
            store
                .matrix()
                .avancar_conversas(org, atualizado_em, &id)
                .map(|_| ())
        }
    }
}

// ───────────────────────────── lado Synapse ─────────────────────────────

/// O que o espelho de estado escreve no Synapse.
///
/// Os testes usam um duplo que conta as chamadas: é por ele que se prova que a mesma situação não
/// gera escrita nenhuma e que uma transição gera um aviso e um metadata.
pub trait DestinoEstado: Send + Sync {
    /// Escreve o aviso (`m.notice`) na sala. `txn` é a chave de idempotência (P7).
    fn avisar(
        &self,
        sala: &str,
        agente: AgentId,
        txn: &str,
        texto: &str,
    ) -> impl Future<Output = Result<(), ErroChamada>> + Send;

    /// Reescreve o evento de metadata que o Element lê.
    fn escrever_metadata(
        &self,
        sala: &str,
        agente: AgentId,
        metadata: &MetadataSala,
    ) -> impl Future<Output = Result<(), ErroChamada>> + Send;

    /// Põe a sala no espaço do departamento (MX-12).
    fn posicionar(
        &self,
        dados: SalaParaEspaco,
    ) -> impl Future<Output = Result<Posicao, ErroEspaco>> + Send;

    /// Renomeia a sala (`m.room.name`) com o nome do cliente (MX-33). Idempotente no Synapse.
    fn renomear(
        &self,
        sala: &str,
        agente: AgentId,
        nome: &str,
    ) -> impl Future<Output = Result<(), ErroChamada>> + Send;

    /// Deixa o fantasma do cliente com o nome de exibição novo (MX-10, que só reescreve o perfil
    /// quando o nome muda). Idempotente.
    fn renomear_fantasma(
        &self,
        cliente: &str,
        nome: &str,
    ) -> impl Future<Output = Result<(), ErroChamada>> + Send;
}

/// Quem escreve o aviso na sala.
///
/// O aviso sai em nome do **bot do Application Service** quando ele já está na sala (é o remetente
/// neutro: não parece mensagem de um atendente nem do agente). Como a MX-11 não convida o bot para
/// a sala de atendimento, o normal hoje é o **fantasma do agente** — que é o dono da sala e com
/// certeza está dentro dela.
pub fn escolher_remetente(
    membros: &BTreeMap<String, MembroSala>,
    bot: &str,
    agente: &str,
) -> String {
    if membros.contains_key(bot) {
        bot.to_owned()
    } else {
        agente.to_owned()
    }
}

/// Destino de verdade: o cliente da API do Matrix (MX-09), os fantasmas da MX-10 e os espaços da
/// MX-12.
pub struct DestinoDoSynapse<P: PortaEspacos + Send + Sync> {
    cliente: ClienteMatrix,
    usuarios: Arc<UsuariosVirtuais>,
    ids: IdsFantasmas,
    espacos: Espacos<P>,
    /// Sala → quem avisa. Sem validade: a escolha só mudaria se o bot entrasse na sala depois, e
    /// nesse caso o aviso continua saindo por quem está lá.
    remetentes: Mutex<CacheRemetentes>,
}

impl<P: PortaEspacos + Send + Sync> DestinoDoSynapse<P> {
    /// Monta a partir do cliente, dos fantasmas da instância e dos espaços (MX-12).
    ///
    /// Os fantasmas entram porque renomear a sala quando o nome do cliente chega (MX-33) passa pelo
    /// mesmo serviço que registra e nomeia o fantasma do cliente (MX-10) — repetir é idempotente.
    pub fn novo(
        cliente: ClienteMatrix,
        usuarios: Arc<UsuariosVirtuais>,
        espacos: Espacos<P>,
    ) -> Self {
        let ids = usuarios.ids().clone();
        Self {
            cliente,
            usuarios,
            ids,
            espacos,
            remetentes: Mutex::new(CacheRemetentes::novo(TETO_PADRAO_REMETENTES)),
        }
    }

    /// Troca o teto do cache de remetentes (testes).
    pub fn com_teto_de_remetentes(mut self, teto: usize) -> Self {
        self.remetentes = Mutex::new(CacheRemetentes::novo(teto));
        self
    }

    /// Id do bot do Application Service (`@<prefixo>_as_bot:<servidor>`).
    pub fn bot(&self) -> String {
        self.ids.bot().user_id().to_owned()
    }

    /// Quem avisa nesta sala: o bot quando ele está dentro, senão o fantasma do agente.
    async fn remetente(&self, sala: &str, agente: AgentId) -> Option<String> {
        if let Some(guardado) = self.cache_remetentes().obter(sala) {
            return Some(guardado);
        }
        let Ok(fantasma) = self.ids.agente(&agente.to_string()) else {
            return None;
        };
        let escolhido = match self.cliente.membros(fantasma.user_id(), sala).await {
            Ok(membros) => escolher_remetente(&membros, &self.bot(), fantasma.user_id()),
            // Sem conseguir conferir, fala quem com certeza está na sala.
            Err(_) => fantasma.user_id().to_owned(),
        };
        self.cache_remetentes().guardar(sala, &escolhido);
        Some(escolhido)
    }

    fn cache_remetentes(&self) -> std::sync::MutexGuard<'_, CacheRemetentes> {
        self.remetentes
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
    }
}

impl<P: PortaEspacos + Send + Sync> DestinoEstado for DestinoDoSynapse<P> {
    async fn avisar(
        &self,
        sala: &str,
        agente: AgentId,
        txn: &str,
        texto: &str,
    ) -> Result<(), ErroChamada> {
        let Some(remetente) = self.remetente(sala, agente).await else {
            return Err(ErroChamada::Configuracao(
                "id de agente fora do padrão do módulo Matrix".to_owned(),
            ));
        };
        let corpo = json!({ "msgtype": MSGTYPE_AVISO, "body": texto });
        self.cliente
            .enviar_evento(&remetente, sala, "m.room.message", txn, &corpo)
            .await
            .map(|_| ())
    }

    async fn escrever_metadata(
        &self,
        sala: &str,
        agente: AgentId,
        metadata: &MetadataSala,
    ) -> Result<(), ErroChamada> {
        // Quem escreve é o fantasma do agente, como na criação da sala (MX-11): é ele que tem
        // poder de escrever evento de estado nesta sala (o bot do AS não é membro dela).
        let Ok(fantasma) = self.ids.agente(&agente.to_string()) else {
            return Err(ErroChamada::Configuracao(
                "id de agente fora do padrão do módulo Matrix".to_owned(),
            ));
        };
        let corpo =
            serde_json::to_value(metadata).map_err(|erro| ErroChamada::RespostaInvalida {
                operacao: "state (metadata)",
                motivo: format!("o metadata da sala não serializou: {erro}"),
            })?;
        self.cliente
            .enviar_estado(fantasma.user_id(), sala, TIPO_METADATA, None, &corpo)
            .await
            .map(|_| ())
    }

    async fn posicionar(&self, dados: SalaParaEspaco) -> Result<Posicao, ErroEspaco> {
        self.espacos.posicionar(&dados).await
    }

    async fn renomear(&self, sala: &str, agente: AgentId, nome: &str) -> Result<(), ErroChamada> {
        // Quem escreve é o fantasma do agente, como no metadata: é ele que tem poder de evento de
        // estado nesta sala (o bot do AS não é membro dela).
        let Ok(fantasma) = self.ids.agente(&agente.to_string()) else {
            return Err(ErroChamada::Configuracao(
                "id de agente fora do padrão do módulo Matrix".to_owned(),
            ));
        };
        let corpo = json!({ "name": nome });
        self.cliente
            .enviar_estado(fantasma.user_id(), sala, TIPO_NOME, None, &corpo)
            .await
            .map(|_| ())
    }

    async fn renomear_fantasma(&self, cliente: &str, nome: &str) -> Result<(), ErroChamada> {
        self.usuarios
            .garantir_cliente(cliente, nome)
            .await
            .map(|_| ())
            .map_err(chamada_dos_fantasmas)
    }
}

/// O erro de fantasma como erro de chamada, para o laço tratar os dois do mesmo jeito.
fn chamada_dos_fantasmas(erro: ErroFantasmas) -> ErroChamada {
    match erro {
        ErroFantasmas::Chamada(chamada) => chamada,
        // Identificador inválido ou id divergente não é problema de rede: é configuração.
        outro => ErroChamada::Configuracao(outro.to_string()),
    }
}

impl<P: PortaEspacos + Send + Sync> std::fmt::Debug for DestinoDoSynapse<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // O cliente e os espaços ficam de fora: nenhum dos dois precisa expor token no `Debug`.
        f.debug_struct("DestinoDoSynapse")
            .field("bot", &self.bot())
            .finish_non_exhaustive()
    }
}

/// Mapa sala → remetente, com teto (FIFO). Acertar o cache não fala com o Synapse.
struct CacheRemetentes {
    teto: usize,
    itens: BTreeMap<String, String>,
    ordem: VecDeque<String>,
}

impl CacheRemetentes {
    fn novo(teto: usize) -> Self {
        Self {
            teto: teto.max(1),
            itens: BTreeMap::new(),
            ordem: VecDeque::new(),
        }
    }

    fn obter(&self, sala: &str) -> Option<String> {
        self.itens.get(sala).cloned()
    }

    fn guardar(&mut self, sala: &str, remetente: &str) {
        if self.itens.contains_key(sala) {
            return;
        }
        while self.itens.len() >= self.teto {
            let Some(antiga) = self.ordem.pop_front() else {
                break;
            };
            self.itens.remove(&antiga);
        }
        self.itens.insert(sala.to_owned(), remetente.to_owned());
        self.ordem.push_back(sala.to_owned());
    }
}

// ───────────────────────────── textos e chaves ─────────────────────────────

/// Texto do aviso de uma mudança de situação.
///
/// Curto e sem dado de cliente: o aviso aparece na sala para qualquer membro.
pub fn aviso_de_status(
    status: StatusConversa,
    departamento: Option<&DepartamentoEspaco>,
) -> String {
    match status {
        StatusConversa::Transferida => match departamento {
            Some(departamento) => format!(
                "Atendimento humano assumiu a conversa (departamento {}): o agente automático está pausado até ela ser reaberta.",
                departamento.nome_do_espaco()
            ),
            None => AVISO_TRANSFERIDA.to_owned(),
        },
        StatusConversa::Aberta => AVISO_REABERTA.to_owned(),
        StatusConversa::Fechada => AVISO_ENCERRADA.to_owned(),
    }
}

/// Chave de idempotência do aviso: (conversa, instante da alteração, situação).
///
/// Repetir a passagem depois de uma falha usa o mesmo `txnId` e o Synapse não duplica o evento
/// (P7); uma segunda transferência da mesma conversa tem outro instante, então tem aviso próprio.
pub fn txn_do_aviso(conversa: &ConversaEstado) -> String {
    let corpo: String = conversa
        .conversa
        .to_string()
        .chars()
        .filter(char::is_ascii_hexdigit)
        .collect();
    format!(
        "{MARCA_AVISO}_{corpo}_{}_{}",
        conversa.atualizado_em,
        conversa.status.como_texto()
    )
}

/// Motivo curto de uma falha de chamada, seguro para log (sem corpo nem endereço).
fn motivo_da_chamada(erro: &ErroChamada) -> String {
    match erro {
        ErroChamada::Resposta {
            status, errcode, ..
        } => match errcode {
            Some(codigo) => format!("{status}_{codigo}"),
            None => format!("http_{status}"),
        },
        ErroChamada::Rede { .. } => "rede".to_owned(),
        ErroChamada::Prazo { .. } => "prazo".to_owned(),
        ErroChamada::Configuracao(_) => "configuracao".to_owned(),
        ErroChamada::RespostaInvalida { .. } => "resposta_invalida".to_owned(),
    }
}

/// Motivo curto de uma falha de espaço, seguro para log.
fn motivo_do_espaco(erro: &ErroEspaco) -> &'static str {
    match erro {
        ErroEspaco::Chamada(_) => "chamada",
        ErroEspaco::Armazenamento(_) => "armazenamento",
        ErroEspaco::SemPermissao { .. } => "sem_permissao",
        ErroEspaco::EspacoInvalido { .. } => "espaco_invalido",
        ErroEspaco::Interno { .. } => "interno",
    }
}

// ───────────────────────────── o laço ─────────────────────────────

/// Falha que interrompe a passagem inteira (o banco não respondeu).
#[derive(Debug, thiserror::Error)]
pub enum ErroEstado {
    /// O armazenamento falhou antes ou no meio da passagem.
    #[error("armazenamento do estado do Matrix falhou: {0}")]
    Armazenamento(#[from] ErroStore),
}

impl ErroEstado {
    /// Código curto para log. O texto do erro do banco nunca vai para o log.
    pub fn codigo(&self) -> &'static str {
        match self {
            Self::Armazenamento(_) => "armazenamento_indisponivel",
        }
    }
}

/// O que uma passagem fez. Só contagens: nada de texto de cliente.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelatorioCiclo {
    /// Conversas de org ligada conferidas.
    pub conferidas: u32,
    /// Mudanças de situação espelhadas (aviso + metadata + `last_status`).
    pub avisadas: u32,
    /// Salas que mudaram de espaço.
    pub posicionadas: u32,
    /// Salas renomeadas porque o nome do cliente chegou ou mudou (MX-33).
    pub renomeadas: u32,
    /// Conversas sem sala: não há onde avisar (a MX-14 cria a sala quando há mensagem).
    pub sem_sala: u32,
    /// Conversas com a situação já espelhada: nenhum aviso e nenhum metadata escrito.
    pub sem_mudanca: u32,
    /// Conversas adiadas por falha transitória (o cursor fica atrás delas).
    pub adiadas: u32,
    /// Conversas puladas por falha definitiva (o cursor passou; voltam na próxima alteração).
    pub puladas: u32,
    /// Páginas lidas.
    pub paginas: u32,
}

/// O que aconteceu com uma conversa nesta passagem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Desfecho {
    /// Resolvida: o cursor pode passar dela.
    Feito,
    /// Pendente: falha transitória, e o cursor não pode passar dela.
    Adiado,
}

/// Cursor de uma org durante a passagem.
struct CursorOrg {
    /// Instante do último ponto resolvido.
    atualizado_em: i64,
    /// Id do último ponto resolvido.
    id: String,
    /// Uma conversa desta org ficou pendente: o cursor para nela.
    bloqueada: bool,
    /// O cursor andou nesta passagem (só então é gravado).
    mudou: bool,
}

/// O laço do espelho de estado. Um por processo; o estado em memória é só o contador de falhas
/// seguidas.
pub struct EstadoDasConversas<D: DestinoEstado, P: PortaEstado> {
    porta: P,
    destino: D,
    ids: IdsFantasmas,
    intervalo: Duration,
    pagina: u32,
    /// Quantas passagens seguidas terminaram com pendência.
    falhas_seguidas: AtomicU32,
    /// Pedido de parada; a passagem em andamento termina e o laço não começa outra.
    parar: tokio::sync::watch::Sender<bool>,
    /// Números do módulo (MX-26). Sem injeção, cai num registro descartável.
    metricas: Arc<MetricasMatrix>,
}

impl<D: DestinoEstado, P: PortaEstado> EstadoDasConversas<D, P> {
    /// Monta o laço com a seção `[matrix]` já validada (o intervalo é o mesmo do espelhamento).
    pub fn novo(porta: P, destino: D, config: &MatrixConfig) -> Result<Self, ErroFantasmas> {
        let ids = IdsFantasmas::da_config(config)?;
        let (parar, _) = tokio::sync::watch::channel(false);
        Ok(Self {
            porta,
            destino,
            ids,
            intervalo: Duration::from_millis(config.intervalo_ms.max(1)),
            pagina: PAGINA,
            falhas_seguidas: AtomicU32::new(0),
            parar,
            metricas: MetricasMatrix::descartaveis(),
        })
    }

    /// Usa os números do processo (`Estado::metricas_matrix`) em vez de um registro descartável.
    pub fn com_metricas(mut self, metricas: Arc<MetricasMatrix>) -> Self {
        self.metricas = metricas;
        self
    }

    /// Troca o tamanho da página (testes com muitas conversas).
    pub fn com_pagina(mut self, pagina: u32) -> Self {
        self.pagina = pagina.max(1);
        self
    }

    /// Pede a parada: a passagem em andamento termina e o laço sai.
    pub fn pedir_parada(&self) {
        self.parar.send_replace(true);
    }

    /// Atraso antes da próxima passagem: o intervalo normal, ou dobrando a cada passagem com
    /// pendência, até [`TETO_ATRASO`].
    pub fn atraso_apos(&self, falhas_seguidas: u32) -> Duration {
        if falhas_seguidas == 0 {
            return self.intervalo;
        }
        let fator = 1u32
            .checked_shl(falhas_seguidas.min(10))
            .unwrap_or(u32::MAX);
        self.intervalo.saturating_mul(fator).min(TETO_ATRASO)
    }

    /// Roda passagens até [`EstadoDasConversas::pedir_parada`]. Erro de uma passagem é registrado e
    /// o laço segue na próxima.
    pub async fn executar(self: Arc<Self>) {
        let mut parar = self.parar.subscribe();
        loop {
            if *parar.borrow() {
                break;
            }
            let falhas = match self.ciclo().await {
                Ok(relatorio) => {
                    tracing::debug!(
                        target: "poph_server::matrix_estado",
                        conferidas = relatorio.conferidas,
                        avisadas = relatorio.avisadas,
                        adiadas = relatorio.adiadas,
                        "passagem do estado das conversas no Matrix"
                    );
                    if relatorio.adiadas > 0 {
                        self.falhas_seguidas.fetch_add(1, Ordering::Relaxed) + 1
                    } else {
                        self.falhas_seguidas.store(0, Ordering::Relaxed);
                        0
                    }
                }
                Err(erro) => {
                    tracing::warn!(
                        target: "poph_server::matrix_estado",
                        motivo = erro.codigo(),
                        "passagem do estado das conversas no Matrix falhou; segue na próxima"
                    );
                    self.falhas_seguidas.fetch_add(1, Ordering::Relaxed) + 1
                }
            };
            tokio::select! {
                _ = tokio::time::sleep(self.atraso_apos(falhas)) => {}
                _ = parar.changed() => break,
            }
        }
    }

    /// Uma passagem: lê as conversas alteradas e espelha o estado de cada uma.
    pub async fn ciclo(&self) -> Result<RelatorioCiclo, ErroEstado> {
        let mut relatorio = RelatorioCiclo::default();

        let mut orgs: BTreeMap<OrgId, CursorOrg> = BTreeMap::new();
        for estado in self.porta.orgs().await? {
            if !estado.ligado {
                continue;
            }
            let cursores = self.porta.cursores(estado.org).await?;
            orgs.insert(
                estado.org,
                CursorOrg {
                    atualizado_em: cursores.atualizado_em,
                    id: cursores
                        .id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| ID_MINIMO.to_owned()),
                    bloqueada: false,
                    mudou: false,
                },
            );
        }
        if orgs.is_empty() {
            return Ok(relatorio);
        }

        // A leitura é global (uma consulta serve a todas as orgs); o cursor é por org, então a
        // página começa no menor par e cada org filtra o que é seu.
        let mut apos = orgs
            .values()
            .map(|cursor| (cursor.atualizado_em, cursor.id.clone()))
            .min()
            .unwrap_or((0, ID_MINIMO.to_owned()));

        let mut paginas = 0u32;
        while paginas < MAX_PAGINAS_POR_CICLO && !*self.parar.borrow() {
            let inicio = conversa_do_par(&apos);
            let pagina = self.porta.alteradas(inicio, self.pagina).await?;
            if pagina.is_empty() {
                break;
            }
            paginas += 1;
            relatorio.paginas += 1;
            let ultima_pagina = pagina.len() < self.pagina as usize;

            for alterada in &pagina {
                let Some(cursor) = orgs.get_mut(&alterada.org) else {
                    continue;
                };
                if cursor.bloqueada {
                    continue;
                }
                let par = (alterada.atualizado_em, alterada.conversa.to_string());
                if par <= (cursor.atualizado_em, cursor.id.clone()) {
                    continue;
                }
                // A conversa pode ter sumido entre a leitura da página e esta.
                let Some(conversa) = self.porta.conversa(alterada.org, alterada.conversa).await?
                else {
                    cursor.atualizado_em = par.0;
                    cursor.id = par.1;
                    cursor.mudou = true;
                    continue;
                };
                relatorio.conferidas += 1;
                match self.espelhar(&conversa, &mut relatorio).await? {
                    Desfecho::Feito => {
                        cursor.atualizado_em = par.0;
                        cursor.id = par.1;
                        cursor.mudou = true;
                    }
                    Desfecho::Adiado => cursor.bloqueada = true,
                }
            }

            let Some(ultima) = pagina.last() else {
                break;
            };
            apos = (ultima.atualizado_em, ultima.conversa.to_string());
            if ultima_pagina {
                break;
            }
        }

        // O cursor só anda depois de a passagem dar certo para aquelas conversas.
        for (org, cursor) in &orgs {
            if !cursor.mudou {
                continue;
            }
            self.porta
                .avancar_conversas(*org, cursor.atualizado_em, &cursor.id)
                .await?;
        }
        Ok(relatorio)
    }

    /// Espelha o estado de uma conversa.
    async fn espelhar(
        &self,
        alvo: &ConversaEstado,
        relatorio: &mut RelatorioCiclo,
    ) -> Result<Desfecho, ErroEstado> {
        let Some(sala) = self.porta.sala(alvo.org, alvo.conversa).await? else {
            relatorio.sem_sala += 1;
            return Ok(Desfecho::Feito);
        };
        if !sala.pronta {
            // Sala no meio da criação (MX-11): a passagem seguinte termina o serviço.
            relatorio.adiadas += 1;
            return Ok(Desfecho::Adiado);
        }

        // `last_status` vazio é "nunca espelhei": a sala nasceu com o metadata da criação, e a
        // referência é a conversa aberta (o estado em que toda conversa nasce).
        let espelhada = sala.ultimo_status.unwrap_or(StatusConversa::Aberta);
        let mut fresca: Option<ConversaEstado> = None;

        if espelhada != alvo.status {
            // Reconferência antes de escrever: se a conversa mudou no meio da passagem, o metadata
            // não pode sair com a situação velha (é o caso de uma conversa que acabou de encerrar).
            let Some(atual) = self.porta.conversa(alvo.org, alvo.conversa).await? else {
                relatorio.sem_sala += 1;
                return Ok(Desfecho::Feito);
            };
            if atual.status != alvo.status {
                relatorio.adiadas += 1;
                return Ok(Desfecho::Adiado);
            }

            let departamento = atual.departamento();
            let texto = aviso_de_status(atual.status, departamento.as_ref());
            let txn = txn_do_aviso(&atual);

            let inicio = Instant::now();
            let resultado = self
                .destino
                .avisar(&sala.room_id, atual.agente, &txn, &texto)
                .await;
            self.medir_synapse(inicio.elapsed().as_secs_f64());
            if let Some(desfecho) = self.tratar(resultado, &atual, "aviso", relatorio) {
                return Ok(desfecho);
            }

            let metadata = MetadataSala::da_conversa(&atual.para_sala(departamento));
            let inicio = Instant::now();
            let resultado = self
                .destino
                .escrever_metadata(&sala.room_id, atual.agente, &metadata)
                .await;
            self.medir_synapse(inicio.elapsed().as_secs_f64());
            if let Some(desfecho) = self.tratar(resultado, &atual, "metadata", relatorio) {
                return Ok(desfecho);
            }

            self.porta
                .definir_ultimo_status(atual.org, atual.conversa, atual.status)
                .await?;
            relatorio.avisadas += 1;
            tracing::info!(
                target: "poph_server::matrix_estado",
                org = %atual.org,
                conversa = %atual.conversa,
                situacao = atual.status.como_texto(),
                "estado da conversa espelhado na sala"
            );
            fresca = Some(atual);
        } else {
            relatorio.sem_mudanca += 1;
        }

        let origem = fresca.as_ref().unwrap_or(alvo);

        // Nome do cliente que chegou depois da criação da sala (MX-33). Roda depois do estado para
        // o metadata escrito ali já levar o nome novo, e antes do espaço, que é independente.
        let metadata_ja_escrita = fresca.is_some();
        if let Some(desfecho) = self
            .renomear_se_preciso(origem, &sala, metadata_ja_escrita, relatorio)
            .await?
        {
            return Ok(desfecho);
        }

        self.posicionar(origem, &sala, relatorio).await;
        Ok(Desfecho::Feito)
    }

    /// Renomeia a sala, o metadata e o fantasma do cliente quando o nome dele chegou (ou mudou)
    /// depois da criação da sala (MX-33).
    ///
    /// A ordem é a da spec: (a) `m.room.name`; (b) metadata com o estado **atual** da conversa;
    /// (c) fantasma do cliente com o nome novo; (d) só então o nome espelhado é gravado. Cair no
    /// meio custa repetir as três chamadas (todas idempotentes), nunca sala nem aviso duplicado.
    ///
    /// Devolve `Some(desfecho)` quando a conversa tem de parar aqui (falha transitória → a passagem
    /// seguinte tenta de novo; definitiva → a conversa volta na próxima alteração), e `None` quando
    /// não havia nada a fazer ou o serviço terminou.
    async fn renomear_se_preciso(
        &self,
        conversa: &ConversaEstado,
        sala: &SalaEstado,
        metadata_ja_escrita: bool,
        relatorio: &mut RelatorioCiclo,
    ) -> Result<Option<Desfecho>, ErroEstado> {
        let Some(nome) = nome_do_cliente(conversa.nome_cliente.as_deref()) else {
            // Conversa sem nome mantém o que a sala já tem: nome nunca volta a vazio.
            return Ok(None);
        };
        if sala.nome_espelhado.as_deref() == Some(nome.as_str()) {
            return Ok(None);
        }

        // Relê antes de escrever: o metadata sai do estado ATUAL da conversa, para um nome atrasado
        // nunca devolver uma conversa `closed` a `active` (a mesma proteção do aviso de situação).
        let Some(atual) = self.porta.conversa(conversa.org, conversa.conversa).await? else {
            return Ok(Some(Desfecho::Feito));
        };
        let Some(nome) = nome_do_cliente(atual.nome_cliente.as_deref()) else {
            return Ok(None);
        };
        if sala.nome_espelhado.as_deref() == Some(nome.as_str()) {
            return Ok(None);
        }
        let metadata_ja_escrita = metadata_ja_escrita && atual == *conversa;

        // (a) nome da sala.
        let inicio = Instant::now();
        let resultado = self
            .destino
            .renomear(&sala.room_id, atual.agente, &nome)
            .await;
        self.medir_synapse(inicio.elapsed().as_secs_f64());
        if let Some(desfecho) = self.tratar(resultado, &atual, "nome da sala", relatorio) {
            return Ok(Some(desfecho));
        }

        // (b) metadata com o estado atual — pulado quando o passo do estado acabou de escrevê-lo
        // com esta mesma conversa (o `client_name` já foi).
        if !metadata_ja_escrita {
            let metadata = MetadataSala::da_conversa(&atual.para_sala(atual.departamento()));
            let inicio = Instant::now();
            let resultado = self
                .destino
                .escrever_metadata(&sala.room_id, atual.agente, &metadata)
                .await;
            self.medir_synapse(inicio.elapsed().as_secs_f64());
            if let Some(desfecho) = self.tratar(resultado, &atual, "metadata do nome", relatorio) {
                return Ok(Some(desfecho));
            }
        }

        // (c) fantasma do cliente com o nome novo.
        let inicio = Instant::now();
        let resultado = self.destino.renomear_fantasma(&atual.cliente, &nome).await;
        self.medir_synapse(inicio.elapsed().as_secs_f64());
        if let Some(desfecho) = self.tratar(resultado, &atual, "nome do fantasma", relatorio) {
            return Ok(Some(desfecho));
        }

        // (d) só depois das três chamadas o nome é dado como espelhado.
        self.porta
            .definir_nome_espelhado(atual.org, atual.conversa, &nome)
            .await?;
        relatorio.renomeadas += 1;
        // Sem o nome no log: o id da conversa basta para achar a sala.
        tracing::info!(
            target: "poph_server::matrix_estado",
            org = %atual.org,
            conversa = %atual.conversa,
            "nome do cliente atualizado na sala (sala, metadata e fantasma)"
        );
        Ok(None)
    }

    /// Contabiliza a chamada ao Synapse e diz se a conversa fica para depois.
    ///
    /// `None` = deu certo e a passagem segue; `Some(desfecho)` = parar por aqui.
    fn tratar(
        &self,
        resultado: Result<(), ErroChamada>,
        conversa: &ConversaEstado,
        etapa: &'static str,
        relatorio: &mut RelatorioCiclo,
    ) -> Option<Desfecho> {
        let erro = resultado.err()?;
        let motivo = motivo_da_chamada(&erro);
        match erro.classificacao() {
            Classificacao::Transitoria => {
                self.metricas.erros_transitorios.incrementar(1);
                if erro.retry_after_ms().is_some() {
                    self.metricas.respostas_429.incrementar(1);
                }
                relatorio.adiadas += 1;
                tracing::warn!(
                    target: "poph_server::matrix_estado",
                    org = %conversa.org,
                    conversa = %conversa.conversa,
                    etapa,
                    motivo,
                    "estado não espelhado agora; a passagem seguinte tenta de novo"
                );
                Some(Desfecho::Adiado)
            }
            Classificacao::Definitiva => {
                self.metricas.erros_definitivos.incrementar(1);
                relatorio.puladas += 1;
                tracing::warn!(
                    target: "poph_server::matrix_estado",
                    org = %conversa.org,
                    conversa = %conversa.conversa,
                    etapa,
                    motivo,
                    "estado recusado pelo Synapse; a conversa volta na próxima alteração"
                );
                Some(Desfecho::Feito)
            }
        }
    }

    /// Põe a sala no espaço do departamento atual (MX-12), sem aviso.
    ///
    /// Falha aqui não desfaz o estado já espelhado nem trava a fila: fica no log, com o motivo.
    async fn posicionar(
        &self,
        conversa: &ConversaEstado,
        sala: &SalaEstado,
        relatorio: &mut RelatorioCiclo,
    ) {
        let departamento = conversa.departamento();
        if departamento.is_none() && sala.espaco.is_none() {
            return;
        }
        let Ok(agente) = self.ids.agente(&conversa.agente.to_string()) else {
            tracing::warn!(
                target: "poph_server::matrix_estado",
                org = %conversa.org,
                conversa = %conversa.conversa,
                motivo = "id_de_agente_invalido",
                "sala não posicionada no espaço do departamento"
            );
            return;
        };
        let dados = SalaParaEspaco {
            org: conversa.org,
            conversa: conversa.conversa,
            sala: sala.room_id.clone(),
            agente,
            departamento,
            // Ligar um espaço que já existe no Element é escolha do operador (MX-22/25); aqui o
            // módulo usa (ou cria) o espaço do departamento.
            existente: None,
        };
        let inicio = Instant::now();
        let resultado = self.destino.posicionar(dados).await;
        self.medir_synapse(inicio.elapsed().as_secs_f64());
        match resultado {
            Ok(Posicao::Posicionada { movida, .. }) if movida => {
                relatorio.posicionadas += 1;
                tracing::info!(
                    target: "poph_server::matrix_estado",
                    org = %conversa.org,
                    conversa = %conversa.conversa,
                    "sala movida para o espaço do departamento novo"
                );
            }
            // Convite pendente: o vínculo não foi gravado de propósito (MX-12), então a próxima
            // alteração desta conversa repete a passagem e convida quem ficou de fora.
            Ok(Posicao::Posicionada {
                convites_falhos, ..
            }) if convites_falhos > 0 => {
                tracing::warn!(
                    target: "poph_server::matrix_estado",
                    org = %conversa.org,
                    conversa = %conversa.conversa,
                    convites_falhos,
                    motivo = "convite_pendente_no_espaco",
                    "sala não ficou gravada no espaço porque há convite pendente; tenta na próxima alteração"
                );
            }
            Ok(_) => {}
            Err(erro) => {
                match erro.classificacao() {
                    Classificacao::Transitoria => self.metricas.erros_transitorios.incrementar(1),
                    Classificacao::Definitiva => self.metricas.erros_definitivos.incrementar(1),
                }
                tracing::warn!(
                    target: "poph_server::matrix_estado",
                    org = %conversa.org,
                    conversa = %conversa.conversa,
                    motivo = motivo_do_espaco(&erro),
                    "sala não entrou no espaço do departamento; tenta na próxima alteração"
                );
            }
        }
    }

    /// Conta a chamada e mede a duração (as métricas são as da MX-26, sem série nova).
    fn medir_synapse(&self, segundos: f64) {
        self.metricas.tentativas.incrementar(1);
        self.metricas.latencia_synapse_segundos.observar(segundos);
    }
}

/// Nome do cliente que vale para a sala: vazio ou só espaço não conta (MX-33).
fn nome_do_cliente(nome: Option<&str>) -> Option<String> {
    nome.map(str::trim)
        .filter(|nome| !nome.is_empty())
        .map(str::to_owned)
}

/// Par do cursor como a leitura incremental o recebe (`None` só quando o instante é o começo).
fn conversa_do_par(par: &(i64, String)) -> Option<(i64, ConversationId)> {
    let id = par.1.parse::<ConversationId>().ok()?;
    Some((par.0, id))
}

#[cfg(test)]
mod testes {
    use super::*;
    use poph_types::DepartmentId;

    fn departamento(nome: &str) -> DepartamentoEspaco {
        DepartamentoEspaco {
            id: DepartmentId::generate(),
            nome: nome.to_owned(),
        }
    }

    fn conversa(status: StatusConversa, atualizado_em: i64) -> ConversaEstado {
        ConversaEstado {
            org: OrgId::generate(),
            conversa: ConversationId::generate(),
            status,
            agente: AgentId::generate(),
            nome_agente: Some("Bia".to_owned()),
            canal: "whatsapp".to_owned(),
            cliente: "5511999990000".to_owned(),
            nome_cliente: None,
            departamento_alvo: None,
            departamento_padrao: None,
            atualizado_em,
        }
    }

    #[test]
    fn aviso_de_cada_situacao_e_curto_e_sem_dado_de_cliente() {
        assert_eq!(
            aviso_de_status(StatusConversa::Aberta, None),
            AVISO_REABERTA
        );
        assert_eq!(
            aviso_de_status(StatusConversa::Fechada, None),
            AVISO_ENCERRADA
        );
        assert_eq!(
            aviso_de_status(StatusConversa::Transferida, None),
            AVISO_TRANSFERIDA
        );
        let com_departamento = aviso_de_status(
            StatusConversa::Transferida,
            Some(&departamento("Suporte técnico")),
        );
        assert!(com_departamento.contains("departamento Suporte técnico"));
        for texto in [AVISO_REABERTA, AVISO_TRANSFERIDA, AVISO_ENCERRADA] {
            assert!(!texto.contains('@'), "{texto}");
            assert!(!texto.chars().any(|c| c.is_ascii_digit()), "{texto}");
        }
    }

    #[test]
    fn txn_do_aviso_muda_com_instante_e_situacao() {
        let mut uma = conversa(StatusConversa::Transferida, 1_000);
        let primeira = txn_do_aviso(&uma);
        assert_eq!(primeira, txn_do_aviso(&uma), "repetir dá a mesma chave");
        assert!(primeira.starts_with("ph_est_"));
        assert!(!primeira.contains('-'), "{primeira}");
        uma.atualizado_em = 2_000;
        assert_ne!(primeira, txn_do_aviso(&uma), "outra transição, outra chave");
        uma.status = StatusConversa::Fechada;
        assert_ne!(primeira, txn_do_aviso(&uma));
        let outra = conversa(StatusConversa::Transferida, 1_000);
        assert_ne!(
            primeira,
            txn_do_aviso(&outra),
            "outra conversa, outra chave"
        );
    }

    #[test]
    fn nome_do_cliente_ignora_vazio_e_espaco() {
        assert_eq!(nome_do_cliente(None), None);
        assert_eq!(nome_do_cliente(Some("")), None);
        assert_eq!(nome_do_cliente(Some("   \n ")), None);
        assert_eq!(
            nome_do_cliente(Some("  Ana Souza ")),
            Some("Ana Souza".to_owned())
        );
    }

    #[test]
    fn departamento_resolve_alvo_antes_do_padrao() {
        let padrao = departamento("Geral");
        let alvo = departamento("Suporte");
        let mut uma = conversa(StatusConversa::Aberta, 1);
        uma.departamento_padrao = Some(padrao.clone());
        assert_eq!(uma.departamento(), Some(padrao));
        uma.departamento_alvo = Some(alvo.clone());
        assert_eq!(uma.departamento(), Some(alvo));
        uma.departamento_padrao = None;
        uma.departamento_alvo = None;
        assert_eq!(uma.departamento(), None);
    }

    #[test]
    fn remetente_do_aviso_e_o_bot_quando_ele_esta_na_sala() {
        let bot = "@ph_as_bot:matrix-dev.local";
        let agente = "@ph_ag_1:matrix-dev.local";
        let mut membros = BTreeMap::new();
        membros.insert(
            agente.to_owned(),
            MembroSala {
                display_name: None,
                avatar_url: None,
            },
        );
        assert_eq!(escolher_remetente(&membros, bot, agente), agente);
        membros.insert(
            bot.to_owned(),
            MembroSala {
                display_name: None,
                avatar_url: None,
            },
        );
        assert_eq!(escolher_remetente(&membros, bot, agente), bot);
    }

    #[test]
    fn cache_de_remetentes_respeita_o_teto() {
        let mut cache = CacheRemetentes::novo(2);
        cache.guardar("!a", "@bot");
        cache.guardar("!b", "@bot");
        cache.guardar("!c", "@bot");
        assert_eq!(cache.obter("!a"), None, "o mais antigo saiu");
        assert_eq!(cache.obter("!b").as_deref(), Some("@bot"));
        assert_eq!(cache.obter("!c").as_deref(), Some("@bot"));
    }

    #[test]
    fn atraso_dobra_com_falhas_e_tem_teto() {
        let config = MatrixConfig {
            habilitado: true,
            homeserver_url: "http://127.0.0.1:1".to_owned(),
            server_name: "matrix-dev.local".to_owned(),
            prefixo: "ph".to_owned(),
            concorrencia: 4,
            intervalo_ms: 1_000,
            ..MatrixConfig::default()
        };
        let laco = EstadoDasConversas::novo(PortaNula, DestinoNulo, &config).expect("monta o laço");
        assert_eq!(laco.atraso_apos(0), Duration::from_secs(1));
        assert_eq!(laco.atraso_apos(1), Duration::from_secs(2));
        assert_eq!(laco.atraso_apos(3), Duration::from_secs(8));
        assert_eq!(laco.atraso_apos(9), TETO_ATRASO);
    }

    /// Porta que não tem nada: só serve para montar o laço no teste do atraso.
    struct PortaNula;

    impl PortaEstado for PortaNula {
        async fn orgs(&self) -> Result<Vec<EstadoOrg>, ErroStore> {
            Ok(Vec::new())
        }

        async fn cursores(&self, _org: OrgId) -> Result<CursorGravado, ErroStore> {
            Ok(CursorGravado {
                atualizado_em: 0,
                id: None,
            })
        }

        async fn alteradas(
            &self,
            _apos: Option<(i64, ConversationId)>,
            _limite: u32,
        ) -> Result<Vec<ConversaAlterada>, ErroStore> {
            Ok(Vec::new())
        }

        async fn conversa(
            &self,
            _org: OrgId,
            _conversa: ConversationId,
        ) -> Result<Option<ConversaEstado>, ErroStore> {
            Ok(None)
        }

        async fn sala(
            &self,
            _org: OrgId,
            _conversa: ConversationId,
        ) -> Result<Option<SalaEstado>, ErroStore> {
            Ok(None)
        }

        async fn definir_ultimo_status(
            &self,
            _org: OrgId,
            _conversa: ConversationId,
            _status: StatusConversa,
        ) -> Result<(), ErroStore> {
            Ok(())
        }

        async fn definir_nome_espelhado(
            &self,
            _org: OrgId,
            _conversa: ConversationId,
            _nome: &str,
        ) -> Result<(), ErroStore> {
            Ok(())
        }

        async fn avancar_conversas(
            &self,
            _org: OrgId,
            _atualizado_em: i64,
            _id: &str,
        ) -> Result<(), ErroStore> {
            Ok(())
        }
    }

    /// Destino que não faz nada: só serve para montar o laço no teste do atraso.
    struct DestinoNulo;

    impl DestinoEstado for DestinoNulo {
        async fn avisar(
            &self,
            _sala: &str,
            _agente: AgentId,
            _txn: &str,
            _texto: &str,
        ) -> Result<(), ErroChamada> {
            Ok(())
        }

        async fn escrever_metadata(
            &self,
            _sala: &str,
            _agente: AgentId,
            _metadata: &MetadataSala,
        ) -> Result<(), ErroChamada> {
            Ok(())
        }

        async fn posicionar(&self, _dados: SalaParaEspaco) -> Result<Posicao, ErroEspaco> {
            Ok(Posicao::SemDepartamento)
        }

        async fn renomear(
            &self,
            _sala: &str,
            _agente: AgentId,
            _nome: &str,
        ) -> Result<(), ErroChamada> {
            Ok(())
        }

        async fn renomear_fantasma(&self, _cliente: &str, _nome: &str) -> Result<(), ErroChamada> {
            Ok(())
        }
    }
}
