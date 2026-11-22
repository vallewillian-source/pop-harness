//! Atores por conversa: uma tarefa e uma fila por conversa (PRD §6, passo 2).
//!
//! Garantias que este módulo dá ao turno:
//! - **Um turno por vez na mesma conversa.** Há um único consumidor da fila da conversa, então dois
//!   eventos nunca rodam em paralelo nela, e a ordem de chegada é preservada.
//! - **Estado durável no banco, não no ator.** O ator pode ser descartado por ociosidade a qualquer
//!   momento; quando a conversa volta, o [`Processador`] recebe `ao_acordar` e recarrega do SQLite.
//!   Nada do que já foi gravado se perde.
//! - **Descarte de ociosos.** Depois de `ociosidade` sem evento, o ator sai e libera a memória. O
//!   teto de atores vivos só despeja ator **ocioso** (com a fila vazia): despejar um ator ocupado
//!   criaria dois consumidores para a mesma conversa, que é justamente o que não pode acontecer.
//! - **Partição por org.** A chave é `(org, conversa)`: conversas de orgs diferentes nunca
//!   compartilham ator nem fila.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use poph_types::{ConversationId, InboxId, MessageId, OrgId};
use tokio::sync::mpsc;

use crate::erro::{ErroAgente, Resultado};
use crate::metricas::MetricasMemoria;

/// Futuro devolvido pelo processador (sem depender de `async_trait`/`futures`).
pub type Futuro = Pin<Box<dyn Future<Output = Resultado<()>> + Send + 'static>>;

/// O que o ator entrega ao processador: uma mensagem já gravada, de uma conversa de uma org.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tarefa {
    /// Org dona.
    pub org: OrgId,
    /// Conversa.
    pub conversa: ConversationId,
    /// Mensagem que disparou o turno (já na tabela `messages`/`inbox`).
    pub mensagem: MessageId,
    /// Item da `inbox` que originou o evento, quando veio de integração.
    ///
    /// O processador devolve a conclusão com `EntradaSaida::concluir_entrada` usando este id: é ele
    /// que tira o item da fila durável. `None` em evento que não passou pela `inbox` (chat da CLI).
    pub entrada: Option<InboxId>,
}

impl Tarefa {
    /// Evento que não veio da `inbox` (chat da CLI, teste).
    pub fn nova(org: OrgId, conversa: ConversationId, mensagem: MessageId) -> Self {
        Self {
            org,
            conversa,
            mensagem,
            entrada: None,
        }
    }

    /// Evento vindo da `inbox`: o processador conclui o item quando o turno termina.
    pub fn da_entrada(
        org: OrgId,
        conversa: ConversationId,
        mensagem: MessageId,
        entrada: InboxId,
    ) -> Self {
        Self {
            org,
            conversa,
            mensagem,
            entrada: Some(entrada),
        }
    }
}

/// Quem processa os eventos de uma conversa. A HR-12/HR-13 implementam isto com o turno de verdade;
/// os testes usam processadores de mentira.
pub trait Processador: Send + Sync + 'static {
    /// Processa um evento. Roda sozinho na conversa: pode ler e gravar no banco sem trava externa.
    fn processar(&self, tarefa: Tarefa) -> Futuro;

    /// Chamado quando um ator é criado (primeiro evento depois do descarte).
    ///
    /// É aqui que o estado da conversa é recarregado do banco. O padrão não faz nada.
    fn ao_acordar(&self, org: OrgId, conversa: ConversationId) -> Futuro {
        let _ = (org, conversa);
        Box::pin(async { Ok(()) })
    }
}

/// Ajustes dos atores.
#[derive(Debug, Clone)]
pub struct OpcoesAtores {
    /// Tempo sem evento até o ator sair da memória.
    pub ociosidade: Duration,
    /// Eventos que a fila de uma conversa aceita antes de recusar.
    pub fila_por_conversa: usize,
    /// Teto de atores vivos (0 = sem teto). Só despeja ator ocioso.
    pub max_atores: usize,
}

impl Default for OpcoesAtores {
    fn default() -> Self {
        Self {
            ociosidade: Duration::from_secs(90),
            fila_por_conversa: 64,
            // D-07 prevê 500 conversas ativas na carga de aceite; 4096 dá folga sem segurar memória.
            max_atores: 4096,
        }
    }
}

type Chave = (OrgId, ConversationId);

struct Entrada {
    id: u64,
    tx: mpsc::Sender<Tarefa>,
    pendentes: Arc<AtomicUsize>,
}

struct Interno {
    opcoes: OpcoesAtores,
    processador: Arc<dyn Processador>,
    mapa: Mutex<HashMap<Chave, Entrada>>,
    proximo_id: AtomicU64,
    vivos: AtomicI64,
    metricas: MetricasMemoria,
}

/// Gerente dos atores por conversa. É `Clone` barato (compartilha o mesmo estado).
#[derive(Clone)]
pub struct Atores {
    interno: Arc<Interno>,
}

impl Atores {
    /// Cria o gerente. `enviar` precisa ser chamado dentro de um runtime tokio.
    pub fn novo(
        processador: Arc<dyn Processador>,
        opcoes: OpcoesAtores,
        metricas: MetricasMemoria,
    ) -> Self {
        Self {
            interno: Arc::new(Interno {
                opcoes,
                processador,
                mapa: Mutex::new(HashMap::new()),
                proximo_id: AtomicU64::new(1),
                vivos: AtomicI64::new(0),
                metricas,
            }),
        }
    }

    /// Enfileira o evento na conversa. Cria o ator se ele não existir (ou se tiver sido despejado).
    ///
    /// Devolve erro quando a fila da conversa está cheia ([`ErroAgente::FilaCheia`]) — o chamador
    /// decide se reenvia (a `inbox` da HR-12 é o lugar de segurar).
    pub fn enviar(&self, tarefa: Tarefa) -> Resultado<()> {
        self.interno.enviar(tarefa)
    }

    /// Conversas com ator vivo.
    pub fn atores_vivos(&self) -> usize {
        usize::try_from(self.interno.vivos.load(Ordering::Relaxed).max(0)).unwrap_or(0)
    }

    /// Atores vivos agora (para métricas e testes).
    pub fn metricas(&self) -> &MetricasMemoria {
        &self.interno.metricas
    }

    /// Espera até todos os atores terem sido despejados por ociosidade (teto de tempo).
    ///
    /// Serve a testes e ao desligamento ordenado; não é caminho de produção.
    pub async fn aguardar_despejo(&self, limite: Duration) -> bool {
        let inicio = std::time::Instant::now();
        while inicio.elapsed() < limite {
            if self.atores_vivos() == 0 {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.atores_vivos() == 0
    }

    /// Espera até as filas esvaziarem (ainda com atores vivos). Para testes.
    pub async fn aguardar_filas_vazias(&self, limite: Duration) -> bool {
        let inicio = std::time::Instant::now();
        while inicio.elapsed() < limite {
            if self.interno.pendentes_totais() == 0 {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        self.interno.pendentes_totais() == 0
    }
}

impl Interno {
    fn travar(&self) -> MutexGuard<'_, HashMap<Chave, Entrada>> {
        self.mapa.lock().unwrap_or_else(|erro| erro.into_inner())
    }

    fn pendentes_totais(&self) -> usize {
        let mapa = self.travar();
        mapa.values()
            .map(|entrada| entrada.pendentes.load(Ordering::Relaxed))
            .sum()
    }

    fn enviar(self: &Arc<Self>, tarefa: Tarefa) -> Resultado<()> {
        // `tokio::spawn` entra em pânico fora de um runtime: recusar antes é melhor que derrubar.
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(ErroAgente::Interno(
                "ator por conversa precisa de um runtime tokio ativo".to_owned(),
            ));
        }
        let chave = (tarefa.org, tarefa.conversa);
        let mut mapa = self.travar();

        if let Some(entrada) = mapa.get(&chave) {
            entrada.pendentes.fetch_add(1, Ordering::AcqRel);
            match entrada.tx.try_send(tarefa) {
                Ok(()) => {
                    self.metricas.eventos_entregues.incrementar(1);
                    return Ok(());
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    entrada.pendentes.fetch_sub(1, Ordering::AcqRel);
                    self.metricas.eventos_recusados.incrementar(1);
                    return Err(ErroAgente::FilaCheia {
                        capacidade: self.opcoes.fila_por_conversa,
                    });
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    // Ator encerrou entre a leitura do mapa e o envio: recria abaixo.
                    entrada.pendentes.fetch_sub(1, Ordering::AcqRel);
                    mapa.remove(&chave);
                }
            }
        }

        self.criar(&mut mapa, chave, tarefa)
    }

    fn criar(
        self: &Arc<Self>,
        mapa: &mut HashMap<Chave, Entrada>,
        chave: Chave,
        tarefa: Tarefa,
    ) -> Resultado<()> {
        if self.opcoes.max_atores > 0 && mapa.len() >= self.opcoes.max_atores {
            let despejou = mapa
                .iter()
                .find(|(_, entrada)| entrada.pendentes.load(Ordering::Relaxed) == 0)
                .map(|(chave_ociosa, _)| *chave_ociosa);
            match despejou {
                Some(chave_ociosa) => {
                    // Derrubar o remetente faz o laço ocioso sair `Ok(None)`.
                    mapa.remove(&chave_ociosa);
                }
                None => {
                    // Todos ocupados: passar do teto é melhor que quebrar "um turno por conversa".
                    self.metricas.atores_acima_do_teto.incrementar(1);
                }
            }
        }

        let (tx, rx) = mpsc::channel(self.opcoes.fila_por_conversa.max(1));
        let pendentes = Arc::new(AtomicUsize::new(1));
        tx.try_send(tarefa).map_err(|_| ErroAgente::AtorEncerrado)?;
        let id = self.proximo_id.fetch_add(1, Ordering::Relaxed);
        mapa.insert(
            chave,
            Entrada {
                id,
                tx,
                pendentes: Arc::clone(&pendentes),
            },
        );
        self.metricas.eventos_entregues.incrementar(1);
        let vivos = self.vivos.fetch_add(1, Ordering::AcqRel) + 1;
        self.metricas.atores_vivos.definir(vivos);

        let interno = Arc::clone(self);
        drop(tokio::spawn(async move {
            interno.laco(chave, id, rx, pendentes).await;
        }));
        Ok(())
    }

    async fn laco(
        self: Arc<Self>,
        chave: Chave,
        id: u64,
        mut rx: mpsc::Receiver<Tarefa>,
        pendentes: Arc<AtomicUsize>,
    ) {
        if let Err(erro) = self.processador.ao_acordar(chave.0, chave.1).await {
            tracing::warn!(
                org = %chave.0,
                conversa = %chave.1,
                erro = %erro,
                "falha ao recarregar a conversa do banco ao acordar o ator"
            );
        }
        let mut despejado = false;
        loop {
            match tokio::time::timeout(self.opcoes.ociosidade, rx.recv()).await {
                Ok(Some(tarefa)) => {
                    if let Err(erro) = self.processador.processar(tarefa).await {
                        tracing::warn!(
                            org = %chave.0,
                            conversa = %chave.1,
                            erro = %erro,
                            "processador devolveu erro para o evento"
                        );
                    }
                    pendentes.fetch_sub(1, Ordering::AcqRel);
                }
                Ok(None) => break,
                Err(_) => {
                    let mut mapa = self.travar();
                    let pode_sair = mapa.get(&chave).is_some_and(|entrada| {
                        entrada.id == id && entrada.pendentes.load(Ordering::Relaxed) == 0
                    });
                    if pode_sair {
                        mapa.remove(&chave);
                        despejado = true;
                        break;
                    }
                    // Chegou evento no meio do timeout: volta ao laço e processa.
                }
            }
        }
        {
            let mut mapa = self.travar();
            if mapa.get(&chave).is_some_and(|entrada| entrada.id == id) {
                mapa.remove(&chave);
            }
        }
        let vivos = self.vivos.fetch_sub(1, Ordering::AcqRel) - 1;
        self.metricas.atores_vivos.definir(vivos.max(0));
        if despejado {
            self.metricas.atores_despejados.incrementar(1);
        }
        tracing::debug!(
            org = %chave.0,
            conversa = %chave.1,
            despejado,
            "ator da conversa encerrado"
        );
    }
}
