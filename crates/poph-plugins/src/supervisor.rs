//! Supervisor de plugin de integração em processo externo (HR-25, PRD §8.3).
//!
//! O plugin é um programa separado que fala **uma mensagem JSON por linha** com o Harness: lê
//! comandos na entrada padrão, escreve eventos, resultados e erros na saída padrão. O formato está
//! em [`poph_types::integracao`] e em `docs/PLUGIN_PROTOCOLO.md`.
//!
//! Por que stdio (e não socket Unix): o plugin já nasce com o ciclo de vida do Harness — quem
//! abre é este supervisor, o `stdin`/`stdout` existe em qualquer sistema, não há arquivo de socket
//! para limpar depois de uma queda (o socket fica para o dia em que duas instâncias dividirem o
//! mesmo plugin, que não é o caso da V1) e o fim do processo vira `EOF` na hora, sem timeout de
//! conexão. Isolamento é por processo: um plugin que trava, cai ou imprime lixo não afeta as outras
//! integrações nem o servidor.
//!
//! O que este módulo garante:
//! - **handshake obrigatório** com a versão de protocolo ([`PROTOCOLO_VERSAO`]) e o manifesto;
//! - **segredos só no handshake**: eles não vão para `argv` nem para o ambiente do processo;
//! - **teto de tamanho** por mensagem nos dois sentidos — a linha acima do teto é descartada e
//!   contada, nunca acumulada em memória;
//! - **timeout por comando**: quem estoura é abortado, o processo é morto e volta depois do
//!   backoff (nada de ficar pendurado esperando para sempre);
//! - **reinício com backoff** até um teto de falhas seguidas; depois disso a integração é marcada
//!   como [`EstadoIntegracao::Caida`] e não sobe mais sozinha;
//! - **lixo é ignorado e contado**, sem derrubar o supervisor.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use poph_types::{
    ComandoSaida, ErroIntegracao, EstadoIntegracao, EventoIntegracao, FuturoComando, FuturoSaude,
    Integration, IntegrationId, Manifesto, MensagemHost, MensagemPlugin, OrgId, PROTOCOLO_VERSAO,
    Receptor, Saude, Segredo, SendFile, SendText, SetPresence,
};
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{sleep, timeout};

use crate::leitor::{LeitorLinhas, Proxima};

/// Folga somada ao prazo do handshake na espera do construtor: o ator ainda tem de responder.
const FOLGA_CONSTRUTOR: Duration = Duration::from_secs(1);

/// Prazo para o processo morrer depois de morto (evita zumbi se ele ignorar o sinal).
const PRAZO_MORTE: Duration = Duration::from_secs(2);

/// Quantas linhas do plugin ficam na fila antes de a leitura parar (contrapressão).
///
/// Sem teto, um plugin que imprime em laço encheria a memória do Harness mais rápido do que o
/// supervisor consome. Com fila limitada, a leitura para, o cano do sistema operacional enche e o
/// plugin bloqueia na escrita: quem abusa fica para trás, o Harness não.
const FILA_LINHAS: usize = 256;

// ───────────────────────────── Configuração ─────────────────────────────

/// Espera exponencial com teto entre tentativas de reinício.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Espera {
    /// Espera da primeira repetição.
    pub base: Duration,
    /// Multiplicador de cada repetição (≥ 1).
    pub fator: u32,
    /// Teto da espera.
    pub teto: Duration,
}

impl Default for Espera {
    fn default() -> Self {
        Self {
            base: Duration::from_millis(500),
            fator: 2,
            teto: Duration::from_secs(60),
        }
    }
}

impl Espera {
    /// Espera depois da `falha`-ésima falha seguida (1 = primeira).
    pub fn espera(&self, falha: u32) -> Duration {
        let mut espera = self.base;
        for _ in 1..falha.max(1) {
            if espera >= self.teto {
                break;
            }
            espera = espera.saturating_mul(self.fator.max(1));
        }
        espera.min(self.teto)
    }
}

/// Limites e prazos do supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitesSupervisor {
    /// Teto de bytes por mensagem (linha) nos dois sentidos.
    pub max_mensagem_bytes: usize,
    /// Prazo do handshake (subida + manifesto).
    pub timeout_handshake: Duration,
    /// Prazo de cada comando de saída.
    pub timeout_comando: Duration,
    /// Prazo do pedido de saúde.
    pub timeout_saude: Duration,
    /// Espera entre reinícios.
    pub espera: Espera,
    /// Falhas seguidas antes de marcar a integração como caída.
    pub max_falhas: u32,
}

impl Default for LimitesSupervisor {
    fn default() -> Self {
        Self {
            max_mensagem_bytes: 1024 * 1024,
            timeout_handshake: Duration::from_secs(5),
            timeout_comando: Duration::from_secs(10),
            timeout_saude: Duration::from_secs(3),
            espera: Espera::default(),
            max_falhas: 5,
        }
    }
}

/// Como lançar e supervisionar o plugin.
#[derive(Clone)]
pub struct OpcoesPlugin {
    /// Programa a executar (caminho do binário do plugin).
    pub programa: PathBuf,
    /// Argumentos. **Nunca** ponha segredo aqui: eles aparecem em `ps` e no `/proc`.
    pub argumentos: Vec<String>,
    /// Variáveis de ambiente entregues ao plugin, além do mínimo (`PATH`, `HOME`, `LANG`, `TZ`…).
    ///
    /// O processo **não herda** o ambiente do Harness: as chaves do provedor de LLM, do Jev e da
    /// cifra não ficam visíveis a um programa de terceiro (PRD §8.3). Segredo da integração não
    /// entra aqui — o lugar dele é o handshake.
    pub ambiente: Vec<(String, String)>,
    /// Diretório de trabalho do processo.
    pub diretorio: Option<PathBuf>,
    /// Integração configurada (o `id` da tabela `integrations`).
    pub integracao: IntegrationId,
    /// Org dona.
    pub org: OrgId,
    /// Tipo da integração.
    pub kind: String,
    /// Configuração do operador (validada contra o JSON Schema do manifesto pela API/Console).
    pub config: serde_json::Value,
    /// Segredos do canal, entregues **só** no handshake.
    pub segredos: BTreeMap<String, Segredo>,
    /// Limites e prazos.
    pub limites: LimitesSupervisor,
}

impl OpcoesPlugin {
    /// Opções com os padrões de limite e configuração vazia.
    pub fn novo(programa: impl Into<PathBuf>, integracao: IntegrationId, org: OrgId) -> Self {
        Self {
            programa: programa.into(),
            argumentos: Vec::new(),
            ambiente: Vec::new(),
            diretorio: None,
            integracao,
            org,
            kind: "plugin".to_owned(),
            config: serde_json::json!({}),
            segredos: BTreeMap::new(),
            limites: LimitesSupervisor::default(),
        }
    }

    /// Tipo da integração.
    pub fn com_kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = kind.into();
        self
    }

    /// Configuração do operador.
    pub fn com_config(mut self, config: serde_json::Value) -> Self {
        self.config = config;
        self
    }

    /// Acrescenta um segredo (vai só no handshake).
    pub fn com_segredo(mut self, nome: impl Into<String>, valor: impl Into<String>) -> Self {
        self.segredos
            .insert(nome.into(), Segredo::new(valor.into()));
        self
    }

    /// Variável de ambiente explícita para o plugin (não use para segredo).
    pub fn com_ambiente(mut self, nome: impl Into<String>, valor: impl Into<String>) -> Self {
        self.ambiente.push((nome.into(), valor.into()));
        self
    }

    /// Limites e prazos.
    pub fn com_limites(mut self, limites: LimitesSupervisor) -> Self {
        self.limites = limites;
        self
    }
}

impl fmt::Debug for OpcoesPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpcoesPlugin")
            .field("programa", &self.programa)
            .field("argumentos", &self.argumentos)
            .field(
                "ambiente",
                &format_args!("{} variável(is)", self.ambiente.len()),
            )
            .field("diretorio", &self.diretorio)
            .field("integracao", &self.integracao)
            .field("org", &self.org)
            .field("kind", &self.kind)
            .field("config", &self.config)
            .field(
                "segredos",
                &format_args!("{} chave(s) [oculto]", self.segredos.len()),
            )
            .field("limites", &self.limites)
            .finish()
    }
}

// ───────────────────────────── Contadores ─────────────────────────────

/// Retrato dos contadores de uma integração (para `/metrics` e para o relatório).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelatoContadores {
    /// Linhas recebidas do plugin.
    pub linhas: u64,
    /// Linhas descartadas (acima do teto) ou que não são do protocolo.
    pub lixo: u64,
    /// Eventos de entrada aceitos.
    pub eventos: u64,
    /// Eventos de entrada recusados pela validação.
    pub eventos_invalidos: u64,
    /// Eventos de estado.
    pub estados: u64,
    /// Comandos enviados.
    pub comandos: u64,
    /// Pedidos de saúde.
    pub saudes: u64,
    /// Handshakes concluídos.
    pub handshakes: u64,
    /// Falhas seguidas contabilizadas (queda, timeout, handshake recusado).
    pub falhas: u64,
    /// Reinícios automáticos bem-sucedidos.
    pub reinicios: u64,
    /// Comandos que estouraram o prazo.
    pub tempo_esgotado: u64,
    /// Abortos pedidos por quem chamou (timeout).
    pub abortos: u64,
}

#[derive(Debug, Default)]
struct Contadores {
    linhas: AtomicU64,
    lixo: AtomicU64,
    eventos: AtomicU64,
    eventos_invalidos: AtomicU64,
    estados: AtomicU64,
    comandos: AtomicU64,
    saudes: AtomicU64,
    handshakes: AtomicU64,
    falhas: AtomicU64,
    reinicios: AtomicU64,
    tempo_esgotado: AtomicU64,
    abortos: AtomicU64,
}

impl Contadores {
    fn somar(&self, contador: &AtomicU64) {
        contador.fetch_add(1, Ordering::Relaxed);
    }

    fn relato(&self) -> RelatoContadores {
        let ler = |contador: &AtomicU64| contador.load(Ordering::Relaxed);
        RelatoContadores {
            linhas: ler(&self.linhas),
            lixo: ler(&self.lixo),
            eventos: ler(&self.eventos),
            eventos_invalidos: ler(&self.eventos_invalidos),
            estados: ler(&self.estados),
            comandos: ler(&self.comandos),
            saudes: ler(&self.saudes),
            handshakes: ler(&self.handshakes),
            falhas: ler(&self.falhas),
            reinicios: ler(&self.reinicios),
            tempo_esgotado: ler(&self.tempo_esgotado),
            abortos: ler(&self.abortos),
        }
    }
}

// ───────────────────────────── Estado compartilhado ─────────────────────────────

struct Comum {
    opcoes: OpcoesPlugin,
    estado: RwLock<EstadoIntegracao>,
    manifesto: RwLock<Option<Manifesto>>,
    contadores: Contadores,
    proximo_id: AtomicU64,
}

impl Comum {
    fn novo(opcoes: OpcoesPlugin) -> Self {
        Self {
            opcoes,
            estado: RwLock::new(EstadoIntegracao::Iniciando),
            manifesto: RwLock::new(None),
            contadores: Contadores::default(),
            proximo_id: AtomicU64::new(1),
        }
    }

    fn definir_estado(&self, novo: EstadoIntegracao) {
        let mut estado = self
            .estado
            .write()
            .unwrap_or_else(|veneno| veneno.into_inner());
        if *estado != novo {
            tracing::debug!(target: "poph_plugins", estado = ?novo, "estado da integração mudou");
        }
        *estado = novo;
    }

    fn estado(&self) -> EstadoIntegracao {
        self.estado
            .read()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .clone()
    }

    fn manifesto(&self) -> Option<Manifesto> {
        self.manifesto
            .read()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .clone()
    }

    fn definir_manifesto(&self, manifesto: Manifesto) {
        let mut guarda = self
            .manifesto
            .write()
            .unwrap_or_else(|veneno| veneno.into_inner());
        *guarda = Some(manifesto);
    }

    fn proximo_id(&self) -> u64 {
        self.proximo_id.fetch_add(1, Ordering::Relaxed)
    }
}

// ───────────────────────────── Handle público ─────────────────────────────

/// Integração em processo externo, supervisionada.
///
/// Clonar o handle não abre outro processo: todos os clones falam com o mesmo supervisor.
#[derive(Clone)]
pub struct Supervisor {
    controle: mpsc::UnboundedSender<Controle>,
    comum: Arc<Comum>,
}

impl fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Supervisor")
            .field("integracao", &self.comum.opcoes.integracao)
            .field("kind", &self.comum.opcoes.kind)
            .field("estado", &self.comum.estado())
            .finish_non_exhaustive()
    }
}

impl Supervisor {
    /// Sobe o plugin, faz o handshake e devolve o handle pronto para enviar.
    ///
    /// Falha se o programa não executar, se o handshake estourar o prazo, se a versão de protocolo
    /// não bater ou se o manifesto for inválido. Ao falhar, o processo é encerrado e nada fica
    /// rodando: quem chamou decide se tenta de novo.
    pub async fn iniciar(
        opcoes: OpcoesPlugin,
        receptor: Arc<dyn Receptor>,
    ) -> Result<Self, ErroIntegracao> {
        let comum = Arc::new(Comum::novo(opcoes));
        let (tx_controle, rx_controle) = mpsc::unbounded_channel();
        let (tx_linhas, rx_linhas) = mpsc::channel(FILA_LINHAS);
        let ator = Ator {
            comum: Arc::clone(&comum),
            receptor,
            controle: rx_controle,
            linhas: rx_linhas,
            tx_linhas,
            processo: None,
            comandos: HashMap::new(),
            saudes: HashMap::new(),
            falhas: 0,
            geracao: 0,
            reiniciar_em: None,
        };
        tokio::spawn(laco(ator));

        let (responder, resposta) = oneshot::channel();
        if tx_controle.send(Controle::Iniciar { responder }).is_err() {
            return Err(ErroIntegracao::Encerrada);
        }
        let prazo = comum.opcoes.limites.timeout_handshake + FOLGA_CONSTRUTOR;
        match timeout(prazo, resposta).await {
            Ok(Ok(resultado)) => resultado.map(|()| Self {
                controle: tx_controle,
                comum,
            }),
            Ok(Err(_)) => Err(ErroIntegracao::Indisponivel {
                motivo: "o supervisor do plugin encerrou durante o handshake".to_owned(),
            }),
            Err(_) => Err(ErroIntegracao::TempoEsgotado {
                operacao: "handshake".to_owned(),
                limite_ms: prazo.as_millis() as u64,
            }),
        }
    }

    /// Manifesto recebido no handshake (`None` antes dele).
    pub fn manifesto(&self) -> Option<Manifesto> {
        self.comum.manifesto()
    }

    /// Estado atual do vínculo.
    pub fn estado(&self) -> EstadoIntegracao {
        self.comum.estado()
    }

    /// Contadores desta integração.
    pub fn contadores(&self) -> RelatoContadores {
        self.comum.contadores.relato()
    }

    /// Integração (id, org e tipo) que este supervisor serve.
    pub fn integracao(&self) -> (IntegrationId, OrgId, String) {
        (
            self.comum.opcoes.integracao,
            self.comum.opcoes.org,
            self.comum.opcoes.kind.clone(),
        )
    }

    /// Espera o estado ficar pronto (ou caído/encerrado), sem dormir mais que o prazo.
    pub async fn aguardar_pronto(&self, prazo: Duration) -> Result<(), ErroIntegracao> {
        let limite = Instant::now() + prazo;
        loop {
            let estado = self.comum.estado();
            if estado.esta_pronta() {
                return Ok(());
            }
            if let EstadoIntegracao::Caida { falhas, motivo } = &estado {
                return Err(ErroIntegracao::Caida {
                    falhas: *falhas,
                    motivo: motivo.clone(),
                });
            }
            if estado == EstadoIntegracao::Encerrada {
                return Err(ErroIntegracao::Encerrada);
            }
            if Instant::now() >= limite {
                return Err(ErroIntegracao::Indisponivel {
                    motivo: format!("o plugin não ficou pronto em {} ms", prazo.as_millis()),
                });
            }
            sleep(Duration::from_millis(10)).await;
        }
    }

    /// Encerra o supervisor e mata o processo do plugin.
    pub async fn encerrar(&self) {
        let (responder, resposta) = oneshot::channel();
        if self
            .controle
            .send(Controle::Encerrar {
                responder: Some(responder),
            })
            .is_ok()
        {
            let prazo = PRAZO_MORTE + Duration::from_secs(1);
            let _ = timeout(prazo, resposta).await;
        }
    }

    async fn executar(
        &self,
        comando: ComandoSaida,
        operacao: &'static str,
    ) -> Result<(), ErroIntegracao> {
        let id = self.comum.proximo_id();
        let (responder, resposta) = oneshot::channel();
        if self
            .controle
            .send(Controle::Comando {
                id,
                comando,
                responder,
            })
            .is_err()
        {
            return Err(ErroIntegracao::Encerrada);
        }
        let prazo = self.comum.opcoes.limites.timeout_comando;
        match timeout(prazo, resposta).await {
            Ok(Ok(resultado)) => resultado,
            Ok(Err(_)) => Err(ErroIntegracao::Indisponivel {
                motivo: "o supervisor do plugin encerrou".to_owned(),
            }),
            Err(_) => {
                self.comum
                    .contadores
                    .somar(&self.comum.contadores.tempo_esgotado);
                // Aborta para o plugin travado não segurar o turno nem a próxima tentativa.
                let _ = self.controle.send(Controle::Abortar {
                    motivo: format!(
                        "o plugin não respondeu a `{operacao}` em {} ms",
                        prazo.as_millis()
                    ),
                });
                Err(ErroIntegracao::TempoEsgotado {
                    operacao: operacao.to_owned(),
                    limite_ms: prazo.as_millis() as u64,
                })
            }
        }
    }
}

impl Integration for Supervisor {
    fn manifesto(&self) -> Option<Manifesto> {
        self.comum.manifesto()
    }

    fn estado(&self) -> EstadoIntegracao {
        self.comum.estado()
    }

    fn enviar_texto(&self, comando: SendText) -> FuturoComando {
        let this = self.clone();
        Box::pin(async move {
            this.executar(ComandoSaida::EnviarTexto(comando), "enviar_texto")
                .await
        })
    }

    fn enviar_arquivo(&self, comando: SendFile) -> FuturoComando {
        let this = self.clone();
        Box::pin(async move {
            this.executar(ComandoSaida::EnviarArquivo(comando), "enviar_arquivo")
                .await
        })
    }

    fn definir_presenca(&self, comando: SetPresence) -> FuturoComando {
        let this = self.clone();
        Box::pin(async move {
            this.executar(ComandoSaida::DefinirPresenca(comando), "definir_presenca")
                .await
        })
    }

    fn saude(&self) -> FuturoSaude {
        let this = self.clone();
        Box::pin(async move {
            let id = this.comum.proximo_id();
            let (responder, resposta) = oneshot::channel();
            if this
                .controle
                .send(Controle::Saude { id, responder })
                .is_err()
            {
                return Err(ErroIntegracao::Encerrada);
            }
            this.comum.contadores.somar(&this.comum.contadores.saudes);
            let prazo = this.comum.opcoes.limites.timeout_saude;
            match timeout(prazo, resposta).await {
                Ok(Ok(resultado)) => resultado,
                Ok(Err(_)) => Err(ErroIntegracao::Indisponivel {
                    motivo: "o supervisor do plugin encerrou".to_owned(),
                }),
                Err(_) => {
                    this.comum
                        .contadores
                        .somar(&this.comum.contadores.tempo_esgotado);
                    let _ = this.controle.send(Controle::Abortar {
                        motivo: format!(
                            "o plugin não respondeu à saúde em {} ms",
                            prazo.as_millis()
                        ),
                    });
                    Err(ErroIntegracao::TempoEsgotado {
                        operacao: "saude".to_owned(),
                        limite_ms: prazo.as_millis() as u64,
                    })
                }
            }
        })
    }
}

// ───────────────────────────── Ator ─────────────────────────────

/// Pedido que chega ao ator.
enum Controle {
    Iniciar {
        responder: oneshot::Sender<Result<(), ErroIntegracao>>,
    },
    Comando {
        id: u64,
        comando: ComandoSaida,
        responder: oneshot::Sender<Result<(), ErroIntegracao>>,
    },
    Saude {
        id: u64,
        responder: oneshot::Sender<Result<Saude, ErroIntegracao>>,
    },
    Abortar {
        motivo: String,
    },
    Encerrar {
        responder: Option<oneshot::Sender<()>>,
    },
}

/// Resposta de um comando que o plugin ainda vai confirmar (operação, para a mensagem de recusa).
type ComandoPendente = (&'static str, oneshot::Sender<Result<(), ErroIntegracao>>);

/// O que chega da tarefa de leitura.
enum Entrada {
    Linha { geracao: u64, linha: String },
    Descartada { geracao: u64 },
    Fim { geracao: u64, motivo: String },
}

struct Processo {
    geracao: u64,
    filho: Child,
    entrada: ChildStdin,
}

struct Ator {
    comum: Arc<Comum>,
    receptor: Arc<dyn Receptor>,
    controle: mpsc::UnboundedReceiver<Controle>,
    linhas: mpsc::Receiver<Entrada>,
    tx_linhas: mpsc::Sender<Entrada>,
    processo: Option<Processo>,
    comandos: HashMap<u64, ComandoPendente>,
    saudes: HashMap<u64, oneshot::Sender<Result<Saude, ErroIntegracao>>>,
    falhas: u32,
    geracao: u64,
    reiniciar_em: Option<Instant>,
}

async fn laco(mut ator: Ator) {
    loop {
        let reiniciar_em = ator.reiniciar_em;
        tokio::select! {
            pedido = ator.controle.recv() => {
                let Some(pedido) = pedido else { break };
                if !ator.tratar_controle(pedido).await {
                    break;
                }
            }
            entrada = ator.linhas.recv() => {
                let Some(entrada) = entrada else { break };
                ator.tratar_entrada(entrada).await;
            }
            () = esperar_ate(reiniciar_em) => ator.reiniciar().await,
        }
    }
    ator.encerrar().await;
}

async fn esperar_ate(prazo: Option<Instant>) {
    match prazo {
        Some(prazo) => {
            let agora = Instant::now();
            if prazo > agora {
                sleep(prazo - agora).await;
            }
        }
        None => std::future::pending::<()>().await,
    }
}

impl Ator {
    /// Trata um pedido. `false` = encerrar o laço.
    async fn tratar_controle(&mut self, pedido: Controle) -> bool {
        match pedido {
            Controle::Iniciar { responder } => {
                let resultado = self.subir().await;
                if resultado.is_err() {
                    self.registrar_falha().await;
                }
                let _ = responder.send(resultado);
            }
            Controle::Comando {
                id,
                comando,
                responder,
            } => {
                let operacao = comando.nome();
                let Some(processo) = self.processo.as_mut() else {
                    let _ = responder.send(Err(self.erro_sem_processo()));
                    return true;
                };
                self.comandos.insert(id, (operacao, responder));
                self.comum.contadores.somar(&self.comum.contadores.comandos);
                let mensagem = MensagemHost::Comando { id, comando };
                let teto = self.comum.opcoes.limites.max_mensagem_bytes;
                match escrever(&mut processo.entrada, &mensagem, teto).await {
                    // Mensagem grande demais é erro de quem chamou, não do plugin: responde só este.
                    Err(erro @ ErroIntegracao::MensagemGrandeDemais { .. }) => {
                        if let Some((_, responder)) = self.comandos.remove(&id) {
                            let _ = responder.send(Err(erro));
                        }
                    }
                    Err(erro) => {
                        tracing::debug!(target: "poph_plugins", %erro, "falha ao escrever comando");
                        // O `registrar_falha` responde os pendentes (inclusive este) e agenda o reinício.
                        self.registrar_falha().await;
                    }
                    Ok(()) => {}
                }
            }
            Controle::Saude { id, responder } => {
                let Some(processo) = self.processo.as_mut() else {
                    let _ = responder.send(Err(self.erro_sem_processo()));
                    return true;
                };
                self.saudes.insert(id, responder);
                let mensagem = MensagemHost::Saude { id };
                let teto = self.comum.opcoes.limites.max_mensagem_bytes;
                if let Err(erro) = escrever(&mut processo.entrada, &mensagem, teto).await {
                    tracing::debug!(target: "poph_plugins", %erro, "falha ao escrever pedido de saúde");
                    self.registrar_falha().await;
                }
            }
            Controle::Abortar { motivo } => {
                self.comum.contadores.somar(&self.comum.contadores.abortos);
                tracing::warn!(target: "poph_plugins", %motivo, "abortando o plugin");
                self.registrar_falha().await;
            }
            Controle::Encerrar { responder } => {
                self.encerrar().await;
                if let Some(responder) = responder {
                    let _ = responder.send(());
                }
                return false;
            }
        }
        true
    }

    async fn tratar_entrada(&mut self, entrada: Entrada) {
        match entrada {
            Entrada::Linha { geracao, linha } => {
                if Some(geracao) != self.processo.as_ref().map(|processo| processo.geracao) {
                    return;
                }
                self.comum.contadores.somar(&self.comum.contadores.linhas);
                let mensagem: MensagemPlugin = match serde_json::from_str(&linha) {
                    Ok(mensagem) => mensagem,
                    Err(erro) => {
                        self.comum.contadores.somar(&self.comum.contadores.lixo);
                        tracing::debug!(target: "poph_plugins", %erro, "linha fora do protocolo ignorada");
                        return;
                    }
                };
                self.tratar_mensagem(mensagem);
            }
            Entrada::Descartada { geracao } => {
                if Some(geracao) == self.processo.as_ref().map(|processo| processo.geracao) {
                    self.comum.contadores.somar(&self.comum.contadores.lixo);
                    tracing::debug!(
                        target: "poph_plugins",
                        "linha acima do limite de tamanho ignorada"
                    );
                }
            }
            Entrada::Fim { geracao, motivo } => {
                if Some(geracao) != self.processo.as_ref().map(|processo| processo.geracao) {
                    return;
                }
                tracing::warn!(target: "poph_plugins", %motivo, "o plugin caiu");
                self.registrar_falha().await;
            }
        }
    }

    fn tratar_mensagem(&mut self, mensagem: MensagemPlugin) {
        match mensagem {
            MensagemPlugin::HandshakeOk { .. } => {
                // Handshake só vale dentro do `subir`; repetido, é lixo.
                self.comum.contadores.somar(&self.comum.contadores.lixo);
            }
            MensagemPlugin::Resultado { id, ok, erro } => {
                let Some((operacao, responder)) = self.comandos.remove(&id) else {
                    self.comum.contadores.somar(&self.comum.contadores.lixo);
                    return;
                };
                let resultado = if ok {
                    self.falhas = 0;
                    Ok(())
                } else {
                    let erro = erro.unwrap_or_else(|| poph_types::ErroComando {
                        codigo: "sem_motivo".to_owned(),
                        mensagem: "o plugin recusou sem dizer o motivo".to_owned(),
                        transitorio: false,
                    });
                    Err(ErroIntegracao::Recusado {
                        operacao: operacao.to_owned(),
                        codigo: erro.codigo,
                        mensagem: erro.mensagem,
                        transitoria: erro.transitorio,
                    })
                };
                let _ = responder.send(resultado);
            }
            MensagemPlugin::Saude { id, saude } => {
                let Some(responder) = self.saudes.remove(&id) else {
                    self.comum.contadores.somar(&self.comum.contadores.lixo);
                    return;
                };
                self.falhas = 0;
                let _ = responder.send(Ok(saude));
            }
            MensagemPlugin::Evento { evento } => match evento.validar() {
                Ok(()) => {
                    self.comum.contadores.somar(&self.comum.contadores.eventos);
                    self.receptor.receber(EventoIntegracao::Entrada(evento));
                }
                Err(erro) => {
                    self.comum
                        .contadores
                        .somar(&self.comum.contadores.eventos_invalidos);
                    tracing::debug!(target: "poph_plugins", %erro, "evento de entrada inválido ignorado");
                }
            },
            MensagemPlugin::Estado { estado } => {
                self.comum.contadores.somar(&self.comum.contadores.estados);
                self.receptor.receber(EventoIntegracao::Estado(estado));
            }
            MensagemPlugin::Erro {
                codigo,
                mensagem,
                transitorio,
            } => {
                self.comum.contadores.somar(&self.comum.contadores.lixo);
                tracing::debug!(target: "poph_plugins", %codigo, %mensagem, transitorio, "erro reportado pelo plugin");
                self.receptor
                    .receber(EventoIntegracao::Estado(poph_types::EventoEstado::Erro {
                        codigo,
                        mensagem,
                        transitorio,
                    }));
            }
        }
    }

    /// Erro para quando não há processo (subindo, caído ou encerrado).
    fn erro_sem_processo(&self) -> ErroIntegracao {
        match self.comum.estado() {
            EstadoIntegracao::Caida { falhas, motivo } => ErroIntegracao::Caida { falhas, motivo },
            EstadoIntegracao::Encerrada => ErroIntegracao::Encerrada,
            EstadoIntegracao::Reiniciando { espera_ms, .. } => ErroIntegracao::Indisponivel {
                motivo: format!("o plugin está reiniciando (faltam ~{espera_ms} ms)"),
            },
            EstadoIntegracao::Iniciando | EstadoIntegracao::Pronto => {
                ErroIntegracao::Indisponivel {
                    motivo: "o plugin ainda não está pronto".to_owned(),
                }
            }
        }
    }

    /// Sobe o processo e faz o handshake, esperando o manifesto dentro do prazo.
    async fn subir(&mut self) -> Result<(), ErroIntegracao> {
        if self.processo.is_some() {
            return Ok(());
        }
        self.reiniciar_em = None;
        self.comum.definir_estado(EstadoIntegracao::Iniciando);
        self.geracao = self.geracao.saturating_add(1);
        let geracao = self.geracao;

        let opcoes = &self.comum.opcoes;
        let mut comando = Command::new(&opcoes.programa);
        comando
            .args(&opcoes.argumentos)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(diretorio) = &opcoes.diretorio {
            comando.current_dir(diretorio);
        }
        // Ambiente mínimo: o plugin NÃO herda o do Harness (as chaves do LLM/Jev/cifra não são
        // dele), e segredo da integração nunca vai por aqui (PRD §8.3) — só no handshake.
        comando.env_clear();
        for nome in ["PATH", "HOME", "LANG", "LC_ALL", "TZ", "TMPDIR"] {
            if let Some(valor) = std::env::var_os(nome) {
                comando.env(nome, valor);
            }
        }
        for (nome, valor) in &opcoes.ambiente {
            comando.env(nome, valor);
        }
        let mut filho = comando
            .spawn()
            .map_err(|erro| ErroIntegracao::FalhaAoIniciar {
                programa: opcoes.programa.display().to_string(),
                motivo: erro.to_string(),
            })?;
        let entrada = filho
            .stdin
            .take()
            .ok_or_else(|| ErroIntegracao::FalhaAoIniciar {
                programa: opcoes.programa.display().to_string(),
                motivo: "sem entrada padrão".to_owned(),
            })?;
        let saida = filho
            .stdout
            .take()
            .ok_or_else(|| ErroIntegracao::FalhaAoIniciar {
                programa: opcoes.programa.display().to_string(),
                motivo: "sem saída padrão".to_owned(),
            })?;
        if let Some(erro_padrao) = filho.stderr.take() {
            tokio::spawn(drenar_erro_padrao(erro_padrao));
        }
        tokio::spawn(ler_saida(
            saida,
            self.tx_linhas.clone(),
            geracao,
            self.comum.opcoes.limites.max_mensagem_bytes,
        ));

        let mut processo = Processo {
            geracao,
            filho,
            entrada,
        };
        let handshake = MensagemHost::Handshake(poph_types::Handshake {
            protocolo: PROTOCOLO_VERSAO,
            integracao: opcoes.integracao,
            org: opcoes.org,
            kind: opcoes.kind.clone(),
            config: opcoes.config.clone(),
            segredos: opcoes
                .segredos
                .iter()
                .map(|(nome, valor)| (nome.clone(), valor.expor().clone()))
                .collect(),
        });
        escrever(
            &mut processo.entrada,
            &handshake,
            opcoes.limites.max_mensagem_bytes,
        )
        .await?;

        let prazo = Instant::now() + opcoes.limites.timeout_handshake;
        loop {
            let restante = prazo.saturating_duration_since(Instant::now());
            if restante.is_zero() {
                return Err(ErroIntegracao::TempoEsgotado {
                    operacao: "handshake".to_owned(),
                    limite_ms: opcoes.limites.timeout_handshake.as_millis() as u64,
                });
            }
            match timeout(restante, self.linhas.recv()).await {
                Err(_) => {
                    return Err(ErroIntegracao::TempoEsgotado {
                        operacao: "handshake".to_owned(),
                        limite_ms: opcoes.limites.timeout_handshake.as_millis() as u64,
                    });
                }
                Ok(None) => {
                    return Err(ErroIntegracao::Indisponivel {
                        motivo: "o leitor do plugin encerrou durante o handshake".to_owned(),
                    });
                }
                Ok(Some(Entrada::Fim {
                    geracao: origem,
                    motivo,
                })) if origem == geracao => {
                    return Err(ErroIntegracao::HandshakeRecusado {
                        motivo: format!("o plugin terminou antes do handshake: {motivo}"),
                    });
                }
                Ok(Some(Entrada::Linha {
                    geracao: origem,
                    linha,
                })) if origem == geracao => match serde_json::from_str::<MensagemPlugin>(&linha) {
                    Ok(MensagemPlugin::HandshakeOk {
                        protocolo,
                        manifesto,
                    }) => {
                        if protocolo != PROTOCOLO_VERSAO {
                            return Err(ErroIntegracao::ProtocoloIncompativel {
                                esperado: PROTOCOLO_VERSAO,
                                recebido: protocolo,
                            });
                        }
                        manifesto.validar()?;
                        self.comum.definir_manifesto(manifesto);
                        self.comum
                            .contadores
                            .somar(&self.comum.contadores.handshakes);
                        self.processo = Some(processo);
                        // As falhas seguidas **não** zeram aqui: subir não é sinal de que o plugin
                        // está saudável (um plugin que cai a cada comando sobe toda vez). O placar
                        // zera quando ele responde um comando ou uma saúde.
                        self.comum.definir_estado(EstadoIntegracao::Pronto);
                        return Ok(());
                    }
                    _ => {
                        // Outra mensagem antes do handshake é fora do contrato: conta e ignora.
                        self.comum.contadores.somar(&self.comum.contadores.lixo);
                    }
                },
                Ok(Some(_)) => {
                    // Entrada de uma geração antiga: descarta.
                }
            }
        }
    }

    /// Timer do reinício.
    async fn reiniciar(&mut self) {
        self.reiniciar_em = None;
        if self.comum.estado().esta_encerrada() {
            return;
        }
        match self.subir().await {
            Ok(()) => {
                self.comum
                    .contadores
                    .somar(&self.comum.contadores.reinicios);
                tracing::info!(target: "poph_plugins", "plugin reiniciado");
            }
            Err(erro) => {
                tracing::warn!(target: "poph_plugins", %erro, "o plugin não subiu");
                self.registrar_falha().await;
            }
        }
    }

    /// Conta a falha, derruba o processo, responde os pendentes e agenda o reinício (ou marca
    /// como caída, se passou do teto).
    async fn registrar_falha(&mut self) {
        self.falhas = self.falhas.saturating_add(1);
        self.comum.contadores.somar(&self.comum.contadores.falhas);
        if let Some(processo) = self.processo.take() {
            encerrar_processo(processo).await;
        }
        let indisponivel = ErroIntegracao::Indisponivel {
            motivo: "o plugin caiu; a mensagem pode ser retentada".to_owned(),
        };
        for (_, (_, responder)) in self.comandos.drain() {
            let _ = responder.send(Err(indisponivel.clone()));
        }
        for (_, responder) in self.saudes.drain() {
            let _ = responder.send(Err(indisponivel.clone()));
        }
        let limite = &self.comum.opcoes.limites;
        if self.falhas >= limite.max_falhas {
            let motivo = format!(
                "{} falhas seguidas (teto {}); não reinicia mais sozinho",
                self.falhas, limite.max_falhas
            );
            tracing::error!(target: "poph_plugins", %motivo, "integração marcada como caída");
            self.comum.definir_estado(EstadoIntegracao::Caida {
                falhas: self.falhas,
                motivo,
            });
            self.reiniciar_em = None;
        } else {
            let espera = limite.espera.espera(self.falhas);
            self.comum.definir_estado(EstadoIntegracao::Reiniciando {
                falhas: self.falhas,
                espera_ms: espera.as_millis() as u64,
            });
            self.reiniciar_em = Some(Instant::now() + espera);
        }
    }

    async fn encerrar(&mut self) {
        self.reiniciar_em = None;
        if let Some(processo) = self.processo.take() {
            let Processo {
                filho,
                entrada,
                geracao: _,
            } = processo;
            let mut entrada = entrada;
            let teto = self.comum.opcoes.limites.max_mensagem_bytes;
            let _ = escrever(&mut entrada, &MensagemHost::Encerrar, teto).await;
            // Fecha a entrada: o plugin que ignorar `encerrar` vê EOF e sai.
            drop(entrada);
            let mut filho = filho;
            if timeout(Duration::from_millis(500), filho.wait())
                .await
                .is_err()
            {
                let _ = filho.start_kill();
                let _ = timeout(PRAZO_MORTE, filho.wait()).await;
            }
        }
        let encerrada = ErroIntegracao::Encerrada;
        for (_, (_, responder)) in self.comandos.drain() {
            let _ = responder.send(Err(encerrada.clone()));
        }
        for (_, responder) in self.saudes.drain() {
            let _ = responder.send(Err(encerrada.clone()));
        }
        self.comum.definir_estado(EstadoIntegracao::Encerrada);
    }
}

// ───────────────────────────── Processo e leitura ─────────────────────────────

async fn escrever(
    entrada: &mut ChildStdin,
    mensagem: &MensagemHost,
    teto: usize,
) -> Result<(), ErroIntegracao> {
    let mut linha = serde_json::to_string(mensagem).map_err(|erro| ErroIntegracao::Protocolo {
        motivo: format!("não consegui serializar a mensagem: {erro}"),
    })?;
    if linha.len() + 1 > teto {
        return Err(ErroIntegracao::MensagemGrandeDemais {
            bytes: linha.len() + 1,
            teto,
        });
    }
    linha.push('\n');
    entrada
        .write_all(linha.as_bytes())
        .await
        .map_err(|erro| ErroIntegracao::Indisponivel {
            motivo: format!("o plugin não aceitou a mensagem: {erro}"),
        })?;
    entrada
        .flush()
        .await
        .map_err(|erro| ErroIntegracao::Indisponivel {
            motivo: format!("o plugin não aceitou a mensagem: {erro}"),
        })
}

async fn encerrar_processo(mut processo: Processo) {
    let _ = processo.filho.start_kill();
    let _ = timeout(PRAZO_MORTE, processo.filho.wait()).await;
}

async fn ler_saida(
    saida: tokio::process::ChildStdout,
    tx: mpsc::Sender<Entrada>,
    geracao: u64,
    limite: usize,
) {
    let mut leitor = LeitorLinhas::novo(saida, limite);
    loop {
        let entrada = match leitor.proxima().await {
            Ok(Proxima::Linha(linha)) => Entrada::Linha { geracao, linha },
            Ok(Proxima::Descartada) => Entrada::Descartada { geracao },
            Ok(Proxima::Fim) => Entrada::Fim {
                geracao,
                motivo: "o plugin fechou a saída padrão".to_owned(),
            },
            Err(erro) => Entrada::Fim {
                geracao,
                motivo: format!("falha ao ler a saída do plugin: {erro}"),
            },
        };
        let fim = matches!(entrada, Entrada::Fim { .. });
        if tx.send(entrada).await.is_err() {
            return;
        }
        if fim {
            return;
        }
    }
}

async fn drenar_erro_padrao(mut erro_padrao: tokio::process::ChildStderr) {
    use tokio::io::AsyncReadExt;

    let mut pedaco = [0u8; 4096];
    loop {
        match erro_padrao.read(&mut pedaco).await {
            Ok(0) | Err(_) => return,
            Ok(lidos) => {
                let texto = String::from_utf8_lossy(&pedaco[..lidos]);
                let texto = texto.trim();
                if !texto.is_empty() {
                    tracing::debug!(target: "poph_plugins", "plugin (stderr): {texto}");
                }
            }
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn espera_cresce_e_para_no_teto() {
        let espera = Espera {
            base: Duration::from_millis(100),
            fator: 2,
            teto: Duration::from_millis(500),
        };
        assert_eq!(espera.espera(0), Duration::from_millis(100));
        assert_eq!(espera.espera(1), Duration::from_millis(100));
        assert_eq!(espera.espera(2), Duration::from_millis(200));
        assert_eq!(espera.espera(3), Duration::from_millis(400));
        // A partir daqui o teto manda.
        assert_eq!(espera.espera(4), Duration::from_millis(500));
        assert_eq!(espera.espera(30), Duration::from_millis(500));
    }

    #[test]
    fn limites_padrao_sao_os_documentados() {
        let limites = LimitesSupervisor::default();
        assert_eq!(limites.max_mensagem_bytes, 1024 * 1024);
        assert_eq!(limites.timeout_handshake, Duration::from_secs(5));
        assert_eq!(limites.timeout_comando, Duration::from_secs(10));
        assert_eq!(limites.timeout_saude, Duration::from_secs(3));
        assert_eq!(limites.max_falhas, 5);
    }

    #[test]
    fn estado_pronto_e_o_unico_que_aceita_comando() {
        assert!(EstadoIntegracao::Pronto.esta_pronta());
        for estado in [
            EstadoIntegracao::Iniciando,
            EstadoIntegracao::Reiniciando {
                falhas: 1,
                espera_ms: 10,
            },
            EstadoIntegracao::Caida {
                falhas: 5,
                motivo: "x".to_owned(),
            },
            EstadoIntegracao::Encerrada,
        ] {
            assert!(!estado.esta_pronta(), "{estado:?}");
        }
        assert!(
            EstadoIntegracao::Caida {
                falhas: 5,
                motivo: "x".to_owned()
            }
            .esta_encerrada()
        );
        assert!(
            !EstadoIntegracao::Reiniciando {
                falhas: 1,
                espera_ms: 10
            }
            .esta_encerrada()
        );
    }
}
