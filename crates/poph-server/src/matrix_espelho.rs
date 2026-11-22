//! Laço de espelhamento de mensagens do Matrix (MX-14).
//!
//! Transforma o que o Harness já gravou em eventos nas salas do Element. Ele **não** fica no
//! caminho do cliente: lê `messages` depois, em páginas, e entrega ao Synapse pelo
//! [`DestinoMatrix`]. O `poph serve` ainda não liga este laço (isso é a MX-20); quem liga chama
//! [`EspelhoMatrix::executar`].
//!
//! ## O que cada ciclo faz
//!
//! 1. Lê as orgs com o módulo ligado e o cursor de mensagens de cada uma (`matrix_estado`).
//! 2. Se algum cursor ficou à frente do maior `rowid` que existe, recua para esse ponto (R5).
//! 3. Lê `messages` a partir do menor cursor, em páginas de [`PAGINA`] linhas (memória limitada).
//! 4. Para cada mensagem, decide: já espelhada (pela posição ou pelo id no cursor), antes do corte
//!    do backfill, de org desligada, ou para espelhar. Para espelhar, passa pelo
//!    [`formato::decidir`]: `Erro` vai para `matrix_falhas`, sem chamar o Synapse.
//! 5. Entrega em paralelo **entre salas** (até `concorrencia`) e em ordem **dentro da sala**. Uma
//!    falha transitória interrompe a sala no ponto da falha.
//! 6. Avança o cursor só até a primeira linha não resolvida (marca-d'água contígua). Nada é
//!    pulado: a linha pendente volta no ciclo seguinte.
//!
//! ## Regras de falha
//!
//! - **Transitória** (rede, 5xx, prazo, 429 sem espera): fica pendente; o ciclo para no ponto e o
//!   próximo espera um atraso que dobra a cada ciclo com falha, até [`TETO_ATRASO`].
//! - **Definitiva** (4xx do Synapse): conta a tentativa por mensagem, em memória. Até
//!   [`TENTATIVAS_DEFINITIVAS`] fica pendente; ao chegar lá, grava em `matrix_falhas` com o código
//!   curto do erro e a fila segue. Um `txnId` repetido não duplica no Synapse (MX-02), então
//!   repetir é seguro.
//!
//! ## Por que a leitura é global e o cursor é por org
//!
//! `messages` é uma tabela só, com todas as orgs. O cursor fica por org (`matrix_estado`), mas o
//! laço lê uma vez, do menor cursor entre as orgs ligadas, e filtra por org depois. Assim uma
//! consulta serve a todas, e cada org só processa o que é dela e é posterior ao próprio cursor.
//!
//! ## Por que o cursor guarda o id
//!
//! O `rowid` pode ser reaproveitado quando a última linha é apagada. Por isso o cursor guarda
//! também o id da mensagem daquela posição: uma linha com o mesmo `rowid` e outro id é mensagem
//! nova e é espelhada; com o mesmo id, já foi espelhada e é pulada.
//!
//! ## O que este arquivo não faz
//!
//! Classificar eventos recebidos (MX-17), espelhar o estado da conversa (MX-15), ligar no `poph
//! serve` (MX-20) e métricas (MX-26). Nenhum log leva texto de cliente: só ids, contadores e o
//! código curto do erro.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use poph_matrix::cliente::ClienteMatrix;
use poph_matrix::erro::{Classificacao, ErroChamada};
use poph_matrix::espacos::{
    DepartamentoEspaco, Espacos, Posicao, SalaParaEspaco, resolver_departamento,
};
use poph_matrix::formato::{
    self, Decisao, Envio, MensagemParaEspelho, OrigemMensagem, PapelMensagem, Sentido,
};
use poph_matrix::salas::{
    ConversaParaSala, ErroPorta, ErroSala, EstadoGravado, PortaSalas, Provisionador, SalaGravada,
};
use poph_matrix::usuarios::{ErroFantasmas, Fantasma, IdsFantasmas};
use poph_store::{
    Direcao, ErroStore, EstadoSala, MensagemDesde, Origem, Papel, StatusConversa, Store,
};
use poph_types::{AgentId, ConversationId, MatrixConfig, MessageId, OrgId};
use serde_json::json;

use crate::matrix_inicio::PortaEspacosStore;
use crate::matrix_metricas::MetricasMatrix;

/// Linhas por página da leitura de `messages`. Define o teto de memória do ciclo.
pub const PAGINA: u32 = 200;

/// Páginas por ciclo. Depois disso o ciclo termina e o próximo continua do cursor.
pub const MAX_PAGINAS_POR_CICLO: u32 = 25;

/// Quantas falhas definitivas seguidas da mesma mensagem antes de isolá-la em `matrix_falhas`.
pub const TENTATIVAS_DEFINITIVAS: u32 = 3;

/// Espera máxima entre ciclos com falha transitória.
pub const TETO_ATRASO: Duration = Duration::from_secs(60);

/// Motivo gravado quando a mensagem não vira um envio válido.
const MOTIVO_FORMATO: &str = "mensagem_invalida";
/// Motivo gravado quando a sala não pôde ser garantida.
const MOTIVO_SALA: &str = "sala_rejeitada";
/// Motivo gravado quando o Synapse recusou o envio sem código curto.
const MOTIVO_ENVIO: &str = "envio_rejeitado";
/// Motivo gravado quando o usuário do aparelho não pôde entrar na sala (MX-42).
const MOTIVO_APARELHO: &str = "aparelho_fora_da_sala";
/// Tamanho máximo do motivo (o mesmo `CHECK` de `matrix_falhas`).
const LIMITE_MOTIVO: usize = 64;

// ───────────────────────────── ponto de extensão ─────────────────────────────

/// Lado Synapse do espelho: garantir a sala e enviar um evento.
///
/// A implementação real é [`DestinoReal`]; os testes usam um duplo em memória. Assim o laço e o
/// banco são de verdade nos testes, e só a rede é simulada.
pub trait DestinoMatrix: Send + Sync {
    /// Id da sala da conversa, criando-a se ainda não existir (MX-11).
    fn garantir_sala(
        &self,
        dados: &ConversaParaSala,
    ) -> impl Future<Output = Result<String, ErroSala>> + Send;

    /// Garante que o usuário do aparelho (MX-42) está dentro da sala antes de falar nela.
    fn garantir_aparelho(
        &self,
        dados: &ConversaParaSala,
        sala: &str,
        aparelho: &Fantasma,
    ) -> impl Future<Output = Result<(), ErroSala>> + Send;

    /// Envia um evento já decidido pelo formato, como o usuário virtual do envio.
    fn enviar(
        &self,
        sala: &str,
        envio: &Envio,
    ) -> impl Future<Output = Result<(), ErroChamada>> + Send;
}

/// Destino de verdade: a provisão de sala (MX-11) e o cliente da API do Matrix (MX-09).
pub struct DestinoReal {
    cliente: ClienteMatrix,
    provisionador: Provisionador<PortaSalasStore>,
    /// Banco do Harness: a leitura do departamento da conversa depois da criação da sala.
    store: Arc<Store>,
    /// Espaços por departamento (MX-12). `None` nos testes antigos, que só olham a sala.
    espacos: Option<Espacos<PortaEspacosStore>>,
    /// Números do módulo (MX-26): latência por operação e salas que ficaram prontas.
    metricas: Arc<MetricasMatrix>,
}

impl DestinoReal {
    /// Monta o destino a partir da seção `[matrix]` já validada e do cliente dela.
    pub fn montar(
        store: Arc<Store>,
        cliente: ClienteMatrix,
        config: &MatrixConfig,
    ) -> Result<Self, ErroFantasmas> {
        let provisionador =
            Provisionador::da_config(cliente.clone(), PortaSalasStore(Arc::clone(&store)), config)?;
        Ok(Self {
            cliente,
            provisionador,
            store,
            espacos: None,
            metricas: MetricasMatrix::descartaveis(),
        })
    }

    /// Liga os espaços por departamento (MX-12) ao espelho.
    ///
    /// Com eles, a sala entra no espaço **ao ser criada**, sem esperar a próxima alteração da
    /// conversa: o laço de estado (MX-15) pode ter visto esta conversa antes de a sala existir e,
    /// sem isso, a sala ficaria fora do espaço até o cliente escrever de novo.
    pub fn com_espacos(mut self, espacos: Espacos<PortaEspacosStore>) -> Self {
        self.espacos = Some(espacos);
        self
    }

    /// Usa os números do processo (`Estado::metricas_matrix`) em vez de um registro descartável.
    pub fn com_metricas(mut self, metricas: Arc<MetricasMatrix>) -> Self {
        self.metricas = metricas;
        self
    }
    /// Põe a sala recém-criada no espaço do departamento dela (MX-12).
    ///
    /// Repetir não escreve nada: quando os convites da passagem fecharam, o vínculo fica gravado e a
    /// passagem seguinte sai por `JaPosicionada`. Convite pendente não grava o vínculo (regra da
    /// MX-12), e a próxima passagem do laço de estado completa o que faltou.
    async fn posicionar_recem_criada(&self, dados: &ConversaParaSala, sala: &str) {
        let Some(espacos) = &self.espacos else {
            return;
        };
        let departamento = match self.departamento_da_conversa(dados) {
            Ok(departamento) => departamento,
            Err(motivo) => {
                tracing::warn!(
                    target: "poph_server::matrix_espelho",
                    org = %dados.org,
                    conversa = %dados.conversa,
                    motivo,
                    "não consegui ler o departamento da conversa; a sala fica sem espaço por ora"
                );
                return;
            }
        };
        let Some(departamento) = departamento else {
            return;
        };
        let agente = match self.provisionador.fantasma_do_agente(dados).await {
            Ok(agente) => agente,
            Err(_) => {
                tracing::warn!(
                    target: "poph_server::matrix_espelho",
                    org = %dados.org,
                    conversa = %dados.conversa,
                    motivo = "fantasma_do_agente_indisponivel",
                    "sala criada mas não posicionada no espaço do departamento"
                );
                return;
            }
        };
        let pedido = SalaParaEspaco {
            org: dados.org,
            conversa: dados.conversa,
            sala: sala.to_owned(),
            agente,
            departamento: Some(departamento),
            // Ligar um espaço que já existe no Element é escolha do operador (MX-22/25); aqui o
            // módulo usa (ou cria) o espaço do departamento.
            existente: None,
        };
        let inicio = Instant::now();
        let resultado = espacos.posicionar(&pedido).await;
        self.metricas
            .latencia_synapse_segundos
            .observar(inicio.elapsed().as_secs_f64());
        match resultado {
            Ok(Posicao::Posicionada {
                convites_falhos, ..
            }) if convites_falhos > 0 => {
                tracing::warn!(
                    target: "poph_server::matrix_espelho",
                    org = %dados.org,
                    conversa = %dados.conversa,
                    convites_falhos,
                    motivo = "convite_pendente_no_espaco",
                    "sala criada e posta no espaço, mas com convite pendente"
                );
            }
            Ok(_) => {
                tracing::info!(
                    target: "poph_server::matrix_espelho",
                    org = %dados.org,
                    conversa = %dados.conversa,
                    "sala criada já dentro do espaço do departamento"
                );
            }
            Err(erro) => {
                match erro.classificacao() {
                    Classificacao::Transitoria => self.metricas.erros_transitorios.incrementar(1),
                    Classificacao::Definitiva => self.metricas.erros_definitivos.incrementar(1),
                }
                tracing::warn!(
                    target: "poph_server::matrix_espelho",
                    org = %dados.org,
                    conversa = %dados.conversa,
                    motivo = "sala_nao_posicionada_no_espaco",
                    "sala criada mas o espaço do departamento não aceitou a sala; tenta na próxima alteração da conversa"
                );
            }
        }
    }

    /// Departamento que manda a sala para um espaço (P3): o alvo da conversa; senão o padrão do
    /// agente; senão nenhum. O nome vem do banco, porque é ele que nomeia o espaço criado.
    fn departamento_da_conversa(
        &self,
        dados: &ConversaParaSala,
    ) -> Result<Option<DepartamentoEspaco>, &'static str> {
        let conversa = self
            .store
            .conversas()
            .obter(dados.org, dados.conversa)
            .map_err(|_| "conversa_indisponivel")?;
        let padrao = self
            .store
            .agentes()
            .obter(dados.org, conversa.agent_id)
            .ok()
            .and_then(|agente| agente.departamento_padrao);
        let Some(escolhido) = resolver_departamento(conversa.departamento_alvo, padrao) else {
            return Ok(None);
        };
        match self.store.departamentos().obter(dados.org, escolhido) {
            Ok(departamento) => Ok(Some(DepartamentoEspaco {
                id: departamento.id,
                nome: departamento.nome,
            })),
            Err(_) => Err("departamento_indisponivel"),
        }
    }
}

impl DestinoMatrix for DestinoReal {
    async fn garantir_sala(&self, dados: &ConversaParaSala) -> Result<String, ErroSala> {
        let inicio = Instant::now();
        let resultado = self.provisionador.provisionar(dados).await;
        self.metricas
            .latencia_synapse_segundos
            .observar(inicio.elapsed().as_secs_f64());
        let pronta = resultado?;
        if !pronta.ja_estava_pronta {
            self.metricas.salas_prontas.incrementar(1);
            // Sala recém-criada: entra no espaço do departamento agora. Falha aqui não derruba a
            // criação (a sala existe e as mensagens podem ser espelhadas); o laço de estado tenta
            // de novo na próxima alteração da conversa.
            self.posicionar_recem_criada(dados, &pronta.room_id).await;
        }
        Ok(pronta.room_id)
    }

    async fn garantir_aparelho(
        &self,
        dados: &ConversaParaSala,
        sala: &str,
        aparelho: &Fantasma,
    ) -> Result<(), ErroSala> {
        self.provisionador
            .garantir_aparelho_na_sala(dados, sala, aparelho)
            .await
    }

    async fn enviar(&self, sala: &str, envio: &Envio) -> Result<(), ErroChamada> {
        let corpo = json!({ "msgtype": envio.msgtype, "body": envio.corpo });
        let inicio = Instant::now();
        let resultado = self
            .cliente
            .enviar_evento(
                envio.remetente.user_id(),
                sala,
                envio.tipo_evento,
                &envio.txn_id,
                &corpo,
            )
            .await
            .map(|_| ());
        self.metricas
            .latencia_synapse_segundos
            .observar(inicio.elapsed().as_secs_f64());
        resultado
    }
}

/// Porta de persistência da provisão (MX-11) sobre o repositório `matrix` do store.
pub struct PortaSalasStore(pub Arc<Store>);

impl PortaSalas for PortaSalasStore {
    fn sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> impl Future<Output = Result<Option<SalaGravada>, ErroPorta>> + Send {
        let store = Arc::clone(&self.0);
        async move {
            let sala = store.matrix().sala(org, conversa).map_err(erro_porta)?;
            Ok(sala.map(|sala| SalaGravada {
                room_id: sala.room_id,
                estado: estado_gravado(sala.estado),
            }))
        }
    }

    fn garantir_sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
        room_id: &str,
    ) -> impl Future<Output = Result<SalaGravada, ErroPorta>> + Send {
        let store = Arc::clone(&self.0);
        let room_id = room_id.to_owned();
        async move {
            let sala = store
                .matrix()
                .garantir_sala(org, conversa, &room_id, None)
                .map_err(erro_porta)?;
            Ok(SalaGravada {
                room_id: sala.room_id,
                estado: estado_gravado(sala.estado),
            })
        }
    }

    fn marcar_pronta(
        &self,
        org: OrgId,
        conversa: ConversationId,
        nome_espelhado: Option<&str>,
    ) -> impl Future<Output = Result<SalaGravada, ErroPorta>> + Send {
        let store = Arc::clone(&self.0);
        let nome = nome_espelhado.map(str::to_owned);
        async move {
            let sala = store
                .matrix()
                .marcar_sala_pronta_com_nome(org, conversa, nome.as_deref())
                .map_err(erro_porta)?;
            Ok(SalaGravada {
                room_id: sala.room_id,
                estado: estado_gravado(sala.estado),
            })
        }
    }

    fn teve_mensagens_antes(
        &self,
        org: OrgId,
        conversa: ConversationId,
        corte: i64,
    ) -> impl Future<Output = Result<bool, ErroPorta>> + Send {
        let store = Arc::clone(&self.0);
        async move {
            store
                .mensagens()
                .teve_mensagens_antes(org, conversa, corte)
                .map_err(erro_porta)
        }
    }
}

fn erro_porta(_: ErroStore) -> ErroPorta {
    // Sem o texto do erro do banco: ele pode carregar valor de cliente.
    ErroPorta::nova("armazenamento do módulo Matrix indisponível")
}

fn estado_gravado(estado: EstadoSala) -> EstadoGravado {
    match estado {
        EstadoSala::Criando => EstadoGravado::Criando,
        EstadoSala::Pronta => EstadoGravado::Pronta,
    }
}

// ───────────────────────────── erros e relatório ─────────────────────────────

/// Falha que interrompe o ciclo inteiro (o banco não respondeu).
#[derive(Debug, thiserror::Error)]
pub enum ErroEspelho {
    /// O armazenamento falhou antes ou no meio do ciclo.
    #[error("armazenamento do espelho do Matrix falhou: {0}")]
    Armazenamento(#[from] ErroStore),
}

impl ErroEspelho {
    /// Código curto para log. O texto do erro do banco nunca vai para o log: pode trazer SQL ou
    /// valores de linha.
    pub fn codigo(&self) -> &'static str {
        match self {
            Self::Armazenamento(_) => "armazenamento_indisponivel",
        }
    }
}

/// O que um ciclo fez. Só contagens e ids de org: nada de texto de cliente.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelatorioCiclo {
    /// Mensagens entregues ao Synapse neste ciclo.
    pub enviadas: u32,
    /// Mensagens que não entram na sala por regra (resposta humana, texto vazio, ferramenta).
    pub nao_espelhadas: u32,
    /// Mensagens já espelhadas antes (pela posição do cursor ou pelo id dele).
    pub ja_espelhadas: u32,
    /// Mensagens de orgs com o módulo desligado (não geram chamada alguma).
    pub de_org_desligada: u32,
    /// Mensagens anteriores ao corte do backfill (`enabled_at`).
    pub antes_do_corte: u32,
    /// Mensagens isoladas em `matrix_falhas` neste ciclo (formato ou falha definitiva repetida).
    pub isoladas: u32,
    /// Mensagens que ficaram pendentes (falha transitória, ou definitiva ainda dentro do limite).
    pub pendentes: u32,
    /// Quantas vezes o cursor de uma org recuou para o ponto seguro (R5).
    pub recuos: u32,
    /// Páginas lidas.
    pub paginas: u32,
    /// Maior página vista: o teto de linhas em memória ao mesmo tempo.
    pub maior_pagina: u32,
}

// ───────────────────────────── estado interno ─────────────────────────────

/// Org ligada, com o corte do backfill e o cursor atual dela.
struct OrgLigada {
    /// Instante (ms) em que a org ligou o módulo (`enabled_at`); 0 quando não há.
    corte: i64,
    /// `(rowid, id)` da última posição espelhada da org.
    cursor: (u64, String),
}

/// Dados da conversa necessários para o formato e para a sala, lidos uma vez por ciclo.
#[derive(Clone)]
struct DadosConversa {
    /// Identificador do cliente (o `peer`, ou o id da conversa quando o canal não tem um).
    identificador: String,
    /// Id do agente, como texto.
    agente: String,
    /// Id da integração (instância da Evolution) da conversa, como texto (MX-42).
    integracao: Option<String>,
    /// Dados para a provisão da sala.
    sala: ConversaParaSala,
}

/// Situação de uma mensagem depois de processada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Estado {
    /// Resolvida: enviada, pulada por regra, ou isolada. O cursor pode passar dela.
    Resolvida,
    /// Pendente: o cursor não pode passar dela.
    Pendente,
}

/// Resultado de uma tentativa de envio dentro de um grupo.
enum Desfecho {
    Enviada,
    Pendente,
    /// Falha definitiva repetida o bastante: isolar com este motivo e esta contagem.
    Isolar {
        motivo: String,
        tentativas: u32,
    },
}

/// Mensagens de uma conversa, em ordem, para serem entregues na mesma sala.
struct Grupo {
    dados: ConversaParaSala,
    /// `(índice na página, org, id, envio)`, em ordem de `rowid`.
    itens: Vec<(usize, OrgId, MessageId, Envio)>,
}

/// O que sobrou de uma página: até onde o cursor pode andar e se algo ficou pendente.
struct ResultadoPagina {
    /// Última posição da sequência contígua resolvida.
    prefixo: Option<(u64, MessageId)>,
    /// Alguma linha ficou pendente: o ciclo para aqui.
    pendente: bool,
}

// ───────────────────────────── o laço ─────────────────────────────

/// O laço de espelhamento. Um por processo; o estado em memória é só o de falhas definitivas e o
/// de falhas seguidas, ambos pequenos.
pub struct EspelhoMatrix<D: DestinoMatrix> {
    store: Arc<Store>,
    destino: D,
    ids: IdsFantasmas,
    concorrencia: usize,
    intervalo: Duration,
    pagina: u32,
    /// Falhas definitivas seguidas por mensagem. Só guarda mensagens que estão falhando.
    definitivas: Mutex<HashMap<MessageId, u32>>,
    /// Quantos ciclos seguidos terminaram com pendência.
    falhas_seguidas: AtomicU32,
    /// Pedido de parada; o ciclo em andamento termina e o laço não começa outro.
    parar: tokio::sync::watch::Sender<bool>,
    /// Números do módulo (MX-26). Sem injeção, cai num registro descartável.
    metricas: Arc<MetricasMatrix>,
    /// Relógio em milissegundos, para medir a idade da mensagem pendente mais antiga.
    agora_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl<D: DestinoMatrix> EspelhoMatrix<D> {
    /// Monta o laço com a seção `[matrix]` já validada (prefixo, servidor, concorrência e intervalo).
    pub fn novo(
        store: Arc<Store>,
        destino: D,
        config: &MatrixConfig,
    ) -> Result<Self, ErroFantasmas> {
        let ids = IdsFantasmas::da_config(config)?;
        let (parar, _) = tokio::sync::watch::channel(false);
        Ok(Self {
            store,
            destino,
            ids,
            concorrencia: (config.concorrencia as usize).max(1),
            intervalo: Duration::from_millis(config.intervalo_ms.max(1)),
            pagina: PAGINA,
            definitivas: Mutex::new(HashMap::new()),
            falhas_seguidas: AtomicU32::new(0),
            parar,
            metricas: MetricasMatrix::descartaveis(),
            agora_ms: Arc::new(relogio_do_sistema),
        })
    }

    /// Usa os números do processo (`Estado::metricas_matrix`) em vez de um registro descartável.
    pub fn com_metricas(mut self, metricas: Arc<MetricasMatrix>) -> Self {
        self.metricas = metricas;
        self
    }

    /// Troca o relógio do laço (testes medem o atraso sem esperar de verdade).
    pub fn com_relogio(mut self, agora_ms: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        self.agora_ms = agora_ms;
        self
    }

    /// Pede a parada. O ciclo em andamento termina a página atual; depois o laço sai.
    pub fn pedir_parada(&self) {
        self.parar.send_replace(true);
    }

    /// Atraso antes do próximo ciclo: o intervalo normal, ou dobrando a cada ciclo com falha, até
    /// [`TETO_ATRASO`].
    pub fn atraso_apos(&self, falhas_seguidas: u32) -> Duration {
        if falhas_seguidas == 0 {
            return self.intervalo;
        }
        let fator = 1u32
            .checked_shl(falhas_seguidas.min(10))
            .unwrap_or(u32::MAX);
        self.intervalo.saturating_mul(fator).min(TETO_ATRASO)
    }

    /// Roda ciclos até [`EspelhoMatrix::pedir_parada`]. Erro de um ciclo é registrado e o laço
    /// segue no próximo.
    pub async fn executar(self: Arc<Self>) {
        let mut parar = self.parar.subscribe();
        loop {
            if *parar.borrow() {
                break;
            }
            let falhas = match self.ciclo().await {
                Ok(relatorio) => {
                    tracing::debug!(
                        target: "poph_server::matrix_espelho",
                        enviadas = relatorio.enviadas,
                        pendentes = relatorio.pendentes,
                        isoladas = relatorio.isoladas,
                        "ciclo do espelho do Matrix"
                    );
                    if relatorio.pendentes > 0 {
                        self.falhas_seguidas.fetch_add(1, Ordering::Relaxed) + 1
                    } else {
                        self.falhas_seguidas.store(0, Ordering::Relaxed);
                        0
                    }
                }
                Err(erro) => {
                    tracing::warn!(
                        target: "poph_server::matrix_espelho",
                        motivo = erro.codigo(),
                        "ciclo do espelho do Matrix falhou; segue no próximo"
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

    /// Um ciclo completo: lê, decide, entrega e avança o cursor até a primeira pendência.
    pub async fn ciclo(&self) -> Result<RelatorioCiclo, ErroEspelho> {
        let mut relatorio = RelatorioCiclo::default();
        let ligadas = self.store.matrix().orgs_ligadas()?;
        if ligadas.is_empty() {
            self.metricas.atraso_cursor_segundos.definir(0);
            return Ok(relatorio);
        }

        let mut orgs: HashMap<OrgId, OrgLigada> = HashMap::new();
        for estado in ligadas.iter().filter(|estado| estado.ligado) {
            let cursores = self.store.matrix().ler_cursores(estado.org_id)?;
            orgs.insert(
                estado.org_id,
                OrgLigada {
                    corte: estado.ligado_em.unwrap_or(0),
                    cursor: (cursores.mensagens, cursores.mensagens_id),
                },
            );
        }
        if orgs.is_empty() {
            self.metricas.atraso_cursor_segundos.definir(0);
            return Ok(relatorio);
        }

        // R5: cursor à frente do maior rowid que existe (banco restaurado ou linhas apagadas).
        // Volta para o ponto seguro; o que vier depois é lido normalmente.
        let (maior, maior_id) = match self.store.mensagens().ultima_rowid()? {
            Some((rowid, id)) => (rowid, id.to_string()),
            None => (0, String::new()),
        };
        for (org, info) in orgs.iter_mut() {
            if info.cursor.0 > maior {
                self.store
                    .matrix()
                    .recuar_cursor_mensagens(*org, maior, &maior_id)?;
                info.cursor = (maior, maior_id.clone());
                relatorio.recuos += 1;
            }
        }

        // Lê a partir do menor cursor. `desde(x)` devolve rowid > x; com `x = cursor - 1` a linha
        // do próprio cursor entra, para o laço conferir o id dela.
        let menor = orgs.values().map(|info| info.cursor.0).min().unwrap_or(0);
        let mut posicao = menor.saturating_sub(1);

        let mut paginas = 0u32;
        while paginas < MAX_PAGINAS_POR_CICLO && !*self.parar.borrow() {
            let pagina = self.store.mensagens().desde(posicao, self.pagina)?;
            if pagina.is_empty() {
                break;
            }
            paginas += 1;
            relatorio.paginas += 1;
            relatorio.maior_pagina = relatorio.maior_pagina.max(pagina.len() as u32);

            let ultimo = pagina.last().map(|m| m.rowid).unwrap_or(posicao);
            let ultima_pagina = pagina.len() < self.pagina as usize;

            let resultado = self.processar_pagina(&orgs, pagina, &mut relatorio).await?;

            if let Some((rowid, id)) = &resultado.prefixo {
                let id_texto = id.to_string();
                for (org, info) in orgs.iter_mut() {
                    if *rowid > info.cursor.0 {
                        self.store
                            .matrix()
                            .avancar_mensagens_com_id(*org, *rowid, &id_texto)?;
                        info.cursor = (*rowid, id_texto.clone());
                    }
                }
            }

            if resultado.pendente {
                break;
            }
            posicao = ultimo;
            if ultima_pagina {
                break;
            }
        }
        self.medir_atraso(&orgs)?;
        Ok(relatorio)
    }

    /// Atualiza o atraso do cursor: a idade da primeira mensagem depois do cursor mais atrasado.
    /// Sem nada depois dele, o atraso é `0`. Assim o número sobe enquanto o Synapse recusa e volta
    /// a zero quando o laço alcança o fim.
    fn medir_atraso(&self, orgs: &HashMap<OrgId, OrgLigada>) -> Result<(), ErroEspelho> {
        let Some(menor) = orgs.values().map(|info| info.cursor.0).min() else {
            self.metricas.atraso_cursor_segundos.definir(0);
            return Ok(());
        };
        let atraso_ms = match self.store.mensagens().desde(menor, 1)?.first() {
            Some(primeira) => ((self.agora_ms)() - primeira.criado_em).max(0),
            None => 0,
        };
        self.metricas
            .atraso_cursor_segundos
            .definir(atraso_ms / 1000);
        Ok(())
    }

    /// Decide e entrega uma página, e devolve até onde ela resolveu.
    async fn processar_pagina(
        &self,
        orgs: &HashMap<OrgId, OrgLigada>,
        pagina: Vec<MensagemDesde>,
        relatorio: &mut RelatorioCiclo,
    ) -> Result<ResultadoPagina, ErroEspelho> {
        let mut estados = vec![Estado::Resolvida; pagina.len()];
        let mut grupos: Vec<Grupo> = Vec::new();
        let mut indice_grupo: HashMap<ConversationId, usize> = HashMap::new();
        // `None` quando a leitura da conversa falhou nesta página: as linhas dela ficam pendentes.
        let mut conversas: HashMap<ConversationId, Option<DadosConversa>> = HashMap::new();

        for (i, mensagem) in pagina.iter().enumerate() {
            let Some(org) = orgs.get(&mensagem.org_id) else {
                relatorio.de_org_desligada += 1;
                continue;
            };
            if mensagem.rowid < org.cursor.0
                || (mensagem.rowid == org.cursor.0 && mensagem.id.to_string() == org.cursor.1)
            {
                relatorio.ja_espelhadas += 1;
                continue;
            }
            if mensagem.criado_em < org.corte {
                relatorio.antes_do_corte += 1;
                continue;
            }

            let lida = conversas
                .entry(mensagem.conversation_id)
                .or_insert_with(|| {
                    self.dados_conversa(mensagem.org_id, mensagem.conversation_id, org.corte)
                        .ok()
                })
                .clone();
            let Some(dados) = lida else {
                // Não consegui ler a conversa agora: a linha fica pendente e volta no próximo ciclo.
                estados[i] = Estado::Pendente;
                continue;
            };

            let decisao = formato::decidir(
                &self.ids,
                &MensagemParaEspelho {
                    id: mensagem.id,
                    conversa: mensagem.conversation_id,
                    origem: origem_de(mensagem.origem),
                    sentido: sentido_de(mensagem.direcao),
                    papel: papel_de(mensagem.papel),
                    texto: &mensagem.texto,
                    identificador_cliente: &dados.identificador,
                    id_agente: &dados.agente,
                    id_integracao: dados.integracao.as_deref(),
                },
            );
            match decisao {
                Decisao::NaoEspelhar(_) => relatorio.nao_espelhadas += 1,
                Decisao::Erro(_) => match self.store.matrix().registrar_falha(
                    mensagem.org_id,
                    mensagem.id,
                    MOTIVO_FORMATO,
                    1,
                ) {
                    Ok(_) => {
                        relatorio.isoladas += 1;
                        self.metricas.mensagens_isoladas.incrementar(1);
                        tracing::warn!(
                            target: "poph_server::matrix_espelho",
                            org = %mensagem.org_id,
                            conversa = %mensagem.conversation_id,
                            motivo = MOTIVO_FORMATO,
                            "mensagem fora do formato do espelho: isolada em matrix_falhas"
                        );
                    }
                    Err(_) => estados[i] = Estado::Pendente,
                },
                Decisao::Espelhar(envio) => {
                    let posicao = match indice_grupo.get(&mensagem.conversation_id) {
                        Some(posicao) => *posicao,
                        None => {
                            grupos.push(Grupo {
                                dados: dados.sala.clone(),
                                itens: Vec::new(),
                            });
                            indice_grupo.insert(mensagem.conversation_id, grupos.len() - 1);
                            grupos.len() - 1
                        }
                    };
                    grupos[posicao]
                        .itens
                        .push((i, mensagem.org_id, mensagem.id, envio));
                }
            }
        }

        // Entrega: salas em paralelo, mensagens da mesma sala em sequência.
        let desfechos: Vec<Vec<(usize, OrgId, MessageId, Desfecho)>> =
            futures::stream::iter(grupos)
                .map(|grupo| self.entregar_grupo(grupo))
                .buffer_unordered(self.concorrencia)
                .collect()
                .await;

        for (indice, org, id, desfecho) in desfechos.into_iter().flatten() {
            match desfecho {
                Desfecho::Enviada => {
                    estados[indice] = Estado::Resolvida;
                    relatorio.enviadas += 1;
                    self.metricas.mensagens_espelhadas.incrementar(1);
                }
                Desfecho::Pendente => {
                    estados[indice] = Estado::Pendente;
                }
                Desfecho::Isolar { motivo, tentativas } => match self
                    .store
                    .matrix()
                    .registrar_falha(org, id, &motivo, tentativas)
                {
                    Ok(_) => {
                        estados[indice] = Estado::Resolvida;
                        relatorio.isoladas += 1;
                        self.metricas.mensagens_isoladas.incrementar(1);
                    }
                    Err(_) => estados[indice] = Estado::Pendente,
                },
            }
        }

        // Marca-d'água: até a primeira linha pendente, tudo resolvido.
        let mut prefixo = None;
        let mut pendente = false;
        for (i, mensagem) in pagina.iter().enumerate() {
            if estados[i] == Estado::Pendente {
                pendente = true;
                break;
            }
            prefixo = Some((mensagem.rowid, mensagem.id));
        }
        relatorio.pendentes += estados.iter().filter(|e| **e == Estado::Pendente).count() as u32;

        Ok(ResultadoPagina { prefixo, pendente })
    }

    /// Entrega as mensagens de uma sala em ordem. Para na primeira falha: o resto fica pendente.
    async fn entregar_grupo(&self, grupo: Grupo) -> Vec<(usize, OrgId, MessageId, Desfecho)> {
        let mut saida = Vec::with_capacity(grupo.itens.len());
        let mut itens = grupo.itens.into_iter();

        let conversa = grupo.dados.conversa;
        self.metricas.tentativas.incrementar(1);
        let sala = match self.destino.garantir_sala(&grupo.dados).await {
            Ok(sala) => sala,
            Err(erro) => {
                if chamada_da_sala(&erro).is_some_and(eh_429) {
                    self.metricas.respostas_429.incrementar(1);
                }
                // A falha da sala é cobrada na primeira mensagem do grupo.
                let motivo = MOTIVO_SALA.to_owned();
                let primeiro = itens.next();
                if let Some((indice, org, id, _)) = primeiro {
                    let desfecho =
                        self.desfecho_de(org, conversa, &id, erro.classificacao(), motivo);
                    saida.push((indice, org, id, desfecho));
                }
                for (indice, org, id, _) in itens {
                    saida.push((indice, org, id, Desfecho::Pendente));
                }
                return saida;
            }
        };

        // O usuário do aparelho entra na sala uma vez por passagem, e só se o grupo tem mensagem dele.
        let mut aparelho_na_sala = false;
        while let Some((indice, org, id, envio)) = itens.next() {
            self.metricas.tentativas.incrementar(1);
            if envio.origem == OrigemMensagem::Dispositivo && !aparelho_na_sala {
                match self
                    .destino
                    .garantir_aparelho(&grupo.dados, &sala, &envio.remetente)
                    .await
                {
                    Ok(()) => aparelho_na_sala = true,
                    Err(erro) => {
                        // Mesma regra dos outros fantasmas: classifica, repete na passagem seguinte
                        // (transitória) ou isola após as tentativas (definitiva). A mensagem continua
                        // gravada no Harness; só a entrega à sala espera.
                        let desfecho = self.desfecho_de(
                            org,
                            conversa,
                            &id,
                            erro.classificacao(),
                            MOTIVO_APARELHO.to_owned(),
                        );
                        saida.push((indice, org, id, desfecho));
                        for (indice, org, id, _) in itens {
                            saida.push((indice, org, id, Desfecho::Pendente));
                        }
                        return saida;
                    }
                }
            }
            match self.destino.enviar(&sala, &envio).await {
                Ok(()) => {
                    self.esquecer(&id);
                    saida.push((indice, org, id, Desfecho::Enviada));
                }
                Err(erro) => {
                    if eh_429(&erro) {
                        self.metricas.respostas_429.incrementar(1);
                    }
                    let motivo = motivo_de_envio(&erro);
                    let desfecho =
                        self.desfecho_de(org, conversa, &id, erro.classificacao(), motivo);
                    saida.push((indice, org, id, desfecho));
                    for (indice, org, id, _) in itens {
                        saida.push((indice, org, id, Desfecho::Pendente));
                    }
                    break;
                }
            }
        }
        saida
    }

    /// Decide o desfecho de uma falha: transitória fica pendente; definitiva conta a tentativa e,
    /// ao chegar em [`TENTATIVAS_DEFINITIVAS`], isola a mensagem.
    fn desfecho_de(
        &self,
        org: OrgId,
        conversa: ConversationId,
        id: &MessageId,
        classificacao: Classificacao,
        motivo: String,
    ) -> Desfecho {
        match classificacao {
            Classificacao::Transitoria => {
                self.metricas.erros_transitorios.incrementar(1);
                tracing::debug!(
                    target: "poph_server::matrix_espelho",
                    org = %org,
                    conversa = %conversa,
                    motivo = %motivo,
                    classe = "transitoria",
                    "falha no espelho do Matrix: a mensagem fica pendente"
                );
                Desfecho::Pendente
            }
            Classificacao::Definitiva => {
                self.metricas.erros_definitivos.incrementar(1);
                let tentativas = {
                    let mut mapa = self
                        .definitivas
                        .lock()
                        .unwrap_or_else(|veneno| veneno.into_inner());
                    let contagem = mapa.entry(*id).or_insert(0);
                    *contagem += 1;
                    *contagem
                };
                if tentativas >= TENTATIVAS_DEFINITIVAS {
                    self.esquecer(id);
                    tracing::warn!(
                        target: "poph_server::matrix_espelho",
                        org = %org,
                        conversa = %conversa,
                        motivo = %motivo,
                        tentativas,
                        classe = "definitiva",
                        "falha definitiva repetida no espelho do Matrix: mensagem isolada"
                    );
                    Desfecho::Isolar { motivo, tentativas }
                } else {
                    tracing::info!(
                        target: "poph_server::matrix_espelho",
                        org = %org,
                        conversa = %conversa,
                        motivo = %motivo,
                        tentativas,
                        classe = "definitiva",
                        "falha definitiva no espelho do Matrix: nova tentativa no próximo ciclo"
                    );
                    Desfecho::Pendente
                }
            }
        }
    }

    fn esquecer(&self, id: &MessageId) {
        let mut mapa = self
            .definitivas
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner());
        mapa.remove(id);
    }

    /// Dados da conversa para formato e sala. Canal sem `peer` usa o id da conversa como cliente.
    fn dados_conversa(
        &self,
        org: OrgId,
        conversa: ConversationId,
        corte: i64,
    ) -> Result<DadosConversa, ErroStore> {
        let conversa_lida = self.store.conversas().obter(org, conversa)?;
        // Mesma regra que a listagem usa para `external_peer` (RP-02): um só lugar, na store.
        let identificador = conversa_lida.identificador_cliente();
        let nome_agente = self
            .store
            .agentes()
            .obter(org, conversa_lida.agent_id)
            .ok()
            .map(|agente| agente.nome);
        let agente: AgentId = conversa_lida.agent_id;
        let sala = ConversaParaSala {
            org,
            conversa,
            cliente: identificador.clone(),
            // Nome que o WhatsApp mandou (`pushName`, MX-32), já normalizado no banco. Sem ele, a
            // sala e o fantasma do cliente caem no identificador.
            nome_cliente: conversa_lida.nome_cliente.clone(),
            agente,
            nome_agente,
            canal: conversa_lida.canal.clone(),
            departamento: conversa_lida.departamento_alvo,
            fechada: conversa_lida.status == StatusConversa::Fechada,
            corte_backfill: Some(corte),
        };
        Ok(DadosConversa {
            identificador,
            agente: agente.to_string(),
            integracao: conversa_lida.integration_id.map(|id| id.to_string()),
            sala,
        })
    }
}

// ───────────────────────────── conversões e motivos ─────────────────────────────

fn origem_de(origem: Origem) -> OrigemMensagem {
    match origem {
        Origem::Agente => OrigemMensagem::Agente,
        Origem::Humano => OrigemMensagem::Humana,
        Origem::Dispositivo => OrigemMensagem::Dispositivo,
    }
}

fn sentido_de(direcao: Direcao) -> Sentido {
    match direcao {
        Direcao::Entrada => Sentido::Entrada,
        Direcao::Saida => Sentido::Saida,
    }
}

fn papel_de(papel: Papel) -> PapelMensagem {
    match papel {
        Papel::Usuario => PapelMensagem::Cliente,
        Papel::Assistente => PapelMensagem::Agente,
        Papel::Ferramenta | Papel::Sistema => PapelMensagem::Outro,
    }
}

/// Código curto do erro do Synapse, quando ele existe e é seguro para gravar.
fn motivo_de_envio(erro: &ErroChamada) -> String {
    match erro {
        ErroChamada::Resposta {
            errcode: Some(codigo),
            ..
        } if codigo_seguro(codigo) => codigo.clone(),
        _ => MOTIVO_ENVIO.to_owned(),
    }
}

/// Chamada ao Synapse por trás de um erro de sala, quando o erro veio de uma chamada.
fn chamada_da_sala(erro: &ErroSala) -> Option<&ErroChamada> {
    match erro {
        ErroSala::Chamada(chamada) | ErroSala::Fantasmas(ErroFantasmas::Chamada(chamada)) => {
            Some(chamada)
        }
        _ => None,
    }
}

/// `429` do Synapse (limite de taxa).
fn eh_429(erro: &ErroChamada) -> bool {
    matches!(erro, ErroChamada::Resposta { status: 429, .. })
}

/// Relógio do sistema em milissegundos (o mesmo carimbo que o Harness grava nas mensagens).
fn relogio_do_sistema() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duracao| i64::try_from(duracao.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Só letras, números e `_`, até o limite da coluna: o que o Synapse manda como `errcode`.
fn codigo_seguro(codigo: &str) -> bool {
    !codigo.is_empty()
        && codigo.len() <= LIMITE_MOTIVO
        && codigo
            .chars()
            .all(|caractere| caractere.is_ascii_alphanumeric() || caractere == '_')
}
