use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use poph_types::{ConversationId, InboxId, MessageId, OrgId};
use tokio::sync::mpsc;

use crate::error::{AgentError, Resultado};
use crate::metrics::MetricsMemory;

pub type BoxFuture = Pin<Box<dyn Future<Output = Resultado<()>> + Send + 'static>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Task {
    pub org: OrgId,
    pub conversa: ConversationId,
    pub mensagem: MessageId,
    pub entry: Option<InboxId>,
}

impl Task {
    pub fn new(org: OrgId, conversa: ConversationId, mensagem: MessageId) -> Self {
        Self {
            org,
            conversa,
            mensagem,
            entry: None,
        }
    }

    pub fn of_entry(
        org: OrgId,
        conversa: ConversationId,
        mensagem: MessageId,
        entry: InboxId,
    ) -> Self {
        Self {
            org,
            conversa,
            mensagem,
            entry: Some(entry),
        }
    }
}

pub trait Processor: Send + Sync + 'static {
    fn process(&self, task: Task) -> BoxFuture;

    fn to_acordar(&self, org: OrgId, conversa: ConversationId) -> BoxFuture {
        let _ = (org, conversa);
        Box::pin(async { Ok(()) })
    }
}

#[derive(Debug, Clone)]
pub struct ActorOptions {
    pub ociosidade: Duration,
    pub queue_by_conversation: usize,
    pub max_actors: usize,
}

impl Default for ActorOptions {
    fn default() -> Self {
        Self {
            ociosidade: Duration::from_secs(90),
            queue_by_conversation: 64,
            max_actors: 4096,
        }
    }
}

type Chave = (OrgId, ConversationId);

struct Entry {
    id: u64,
    tx: mpsc::Sender<Task>,
    pending: Arc<AtomicUsize>,
}

struct Internal {
    options: ActorOptions,
    processor: Arc<dyn Processor>,
    map: Mutex<HashMap<Chave, Entry>>,
    next_id: AtomicU64,
    live: AtomicI64,
    metrics: MetricsMemory,
}

#[derive(Clone)]
pub struct Actors {
    internal: Arc<Internal>,
}

impl Actors {
    pub fn new(
        processor: Arc<dyn Processor>,
        options: ActorOptions,
        metrics: MetricsMemory,
    ) -> Self {
        Self {
            internal: Arc::new(Internal {
                options,
                processor,
                map: Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(1),
                live: AtomicI64::new(0),
                metrics,
            }),
        }
    }

    pub fn send(&self, task: Task) -> Resultado<()> {
        self.internal.send(task)
    }

    pub fn actors_live(&self) -> usize {
        usize::try_from(self.internal.live.load(Ordering::Relaxed).max(0)).unwrap_or(0)
    }

    pub fn metrics(&self) -> &MetricsMemory {
        &self.internal.metrics
    }

    pub async fn wait_despejo(&self, limit: Duration) -> bool {
        let beginning = std::time::Instant::now();
        while beginning.elapsed() < limit {
            if self.actors_live() == 0 {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.actors_live() == 0
    }

    pub async fn wait_queues_empty(&self, limit: Duration) -> bool {
        let beginning = std::time::Instant::now();
        while beginning.elapsed() < limit {
            if self.internal.pending_totais() == 0 {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        self.internal.pending_totais() == 0
    }
}

impl Internal {
    fn lock(&self) -> MutexGuard<'_, HashMap<Chave, Entry>> {
        self.map.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn pending_totais(&self) -> usize {
        let map = self.lock();
        map.values()
            .map(|entry| entry.pending.load(Ordering::Relaxed))
            .sum()
    }

    fn send(self: &Arc<Self>, task: Task) -> Resultado<()> {
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(AgentError::Internal(
                "ator por conversa precisa de um runtime tokio ativo".to_owned(),
            ));
        }
        let chave = (task.org, task.conversa);
        let mut map = self.lock();

        if let Some(entry) = map.get(&chave) {
            entry.pending.fetch_add(1, Ordering::AcqRel);
            match entry.tx.try_send(task) {
                Ok(()) => {
                    self.metrics.events_delivered.increment(1);
                    return Ok(());
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    entry.pending.fetch_sub(1, Ordering::AcqRel);
                    self.metrics.events_refused.increment(1);
                    return Err(AgentError::QueueCheia {
                        capacidade: self.options.queue_by_conversation,
                    });
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    entry.pending.fetch_sub(1, Ordering::AcqRel);
                    map.remove(&chave);
                }
            }
        }

        self.criar(&mut map, chave, task)
    }

    fn criar(
        self: &Arc<Self>,
        map: &mut HashMap<Chave, Entry>,
        chave: Chave,
        task: Task,
    ) -> Resultado<()> {
        if self.options.max_actors > 0 && map.len() >= self.options.max_actors {
            let despejou = map
                .iter()
                .find(|(_, entry)| entry.pending.load(Ordering::Relaxed) == 0)
                .map(|(key_ociosa, _)| *key_ociosa);
            match despejou {
                Some(key_ociosa) => {
                    map.remove(&key_ociosa);
                }
                None => {
                    self.metrics.actors_above_cap.increment(1);
                }
            }
        }

        let (tx, rx) = mpsc::channel(self.options.queue_by_conversation.max(1));
        let pending = Arc::new(AtomicUsize::new(1));
        tx.try_send(task).map_err(|_| AgentError::ActorClosed)?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        map.insert(
            chave,
            Entry {
                id,
                tx,
                pending: Arc::clone(&pending),
            },
        );
        self.metrics.events_delivered.increment(1);
        let live = self.live.fetch_add(1, Ordering::AcqRel) + 1;
        self.metrics.actors_live.set(live);

        let internal = Arc::clone(self);
        drop(tokio::spawn(async move {
            internal.run(chave, id, rx, pending).await;
        }));
        Ok(())
    }

    async fn run(
        self: Arc<Self>,
        chave: Chave,
        id: u64,
        mut rx: mpsc::Receiver<Task>,
        pending: Arc<AtomicUsize>,
    ) {
        if let Err(error) = self.processor.to_acordar(chave.0, chave.1).await {
            tracing::warn!(
                org = %chave.0,
                conversa = %chave.1,
                error = %error,
                "falha ao recarregar a conversa do banco ao acordar o ator"
            );
        }
        let mut despejado = false;
        loop {
            match tokio::time::timeout(self.options.ociosidade, rx.recv()).await {
                Ok(Some(task)) => {
                    if let Err(error) = self.processor.process(task).await {
                        tracing::warn!(
                            org = %chave.0,
                            conversa = %chave.1,
                            error = %error,
                            "processador devolveu erro para o evento"
                        );
                    }
                    pending.fetch_sub(1, Ordering::AcqRel);
                }
                Ok(None) => break,
                Err(_) => {
                    let mut map = self.lock();
                    let can_exit = map.get(&chave).is_some_and(|entry| {
                        entry.id == id && entry.pending.load(Ordering::Relaxed) == 0
                    });
                    if can_exit {
                        map.remove(&chave);
                        despejado = true;
                        break;
                    }
                }
            }
        }
        {
            let mut map = self.lock();
            if map.get(&chave).is_some_and(|entry| entry.id == id) {
                map.remove(&chave);
            }
        }
        let live = self.live.fetch_sub(1, Ordering::AcqRel) - 1;
        self.metrics.actors_live.set(live.max(0));
        if despejado {
            self.metrics.actors_despejados.increment(1);
        }
        tracing::debug!(
            org = %chave.0,
            conversa = %chave.1,
            despejado,
            "ator da conversa encerrado"
        );
    }
}
