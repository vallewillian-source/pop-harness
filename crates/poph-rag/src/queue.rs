use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use poph_types::OrgId;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc};

use crate::embedder::{Embedder, ModelIdentity};
use crate::error::RagError;
use crate::metrics::RagMetrics;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueConfig {
    pub batch_size: usize,
    pub batch_window: Duration,
    pub per_org_concurrency: usize,
    pub total_concurrency: usize,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            batch_size: 32,
            batch_window: Duration::from_millis(10),
            per_org_concurrency: 4,
            total_concurrency: 32,
        }
    }
}

impl QueueConfig {
    pub fn validate(&self) -> Result<(), RagError> {
        if self.batch_size == 0 {
            return Err(RagError::Configuration(
                "tamanho do lote deve ser maior que zero".to_owned(),
            ));
        }
        if self.per_org_concurrency == 0 || self.total_concurrency == 0 {
            return Err(RagError::Configuration(
                "os limites de simultaneidade devem ser maiores que zero".to_owned(),
            ));
        }
        Ok(())
    }
}

type Reply = mpsc::UnboundedSender<(usize, Result<Vec<f32>, RagError>)>;

type OrgSlots = HashMap<OrgId, (Arc<Semaphore>, Arc<RequestCount>)>;

struct Item {
    text: String,
    position: usize,
    reply: Reply,
}

#[derive(Default)]
struct Pending {
    by_org: HashMap<OrgId, VecDeque<Item>>,
    rotation: VecDeque<OrgId>,
    total: usize,
}

impl Pending {
    fn add(&mut self, org: OrgId, items: Vec<Item>) {
        if items.is_empty() {
            return;
        }
        self.total += items.len();
        let queue = self.by_org.entry(org).or_default();
        let was_empty = queue.is_empty();
        queue.extend(items);
        if was_empty {
            self.rotation.push_back(org);
        }
    }

    fn take_batch(&mut self, max: usize) -> Vec<Item> {
        let mut batch = Vec::new();
        while batch.len() < max {
            let Some(org) = self.rotation.pop_front() else {
                break;
            };
            let Some(queue) = self.by_org.get_mut(&org) else {
                continue;
            };
            let mut next = None;
            while let Some(candidate) = queue.pop_front() {
                self.total -= 1;
                if !candidate.reply.is_closed() {
                    next = Some(candidate);
                    break;
                }
            }
            let has_more = !queue.is_empty();
            if has_more {
                self.rotation.push_back(org);
            } else {
                self.by_org.remove(&org);
            }
            if let Some(item) = next {
                batch.push(item);
            }
        }
        batch
    }
}

#[derive(Default)]
struct RequestCount {
    current: AtomicUsize,
    peak: AtomicUsize,
}

impl RequestCount {
    fn enter(&self) {
        let now = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
    }

    fn leave(&self) {
        self.current.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Shared {
    pending: Mutex<Pending>,
    notification: Notify,
    closed: AtomicBool,
    metrics: Option<RagMetrics>,
}

pub struct EmbeddingQueue {
    shared: Arc<Shared>,
    per_org_concurrency: usize,
    global_slots: Arc<Semaphore>,
    global_count: Arc<RequestCount>,
    org_slots: Mutex<OrgSlots>,
}

struct Admission {
    _org: OwnedSemaphorePermit,
    _total: OwnedSemaphorePermit,
    org_count: Arc<RequestCount>,
    global_count: Arc<RequestCount>,
}

impl Drop for Admission {
    fn drop(&mut self) {
        self.org_count.leave();
        self.global_count.leave();
    }
}

impl EmbeddingQueue {
    pub fn start(
        embedder: Arc<dyn Embedder>,
        config: QueueConfig,
        metrics: Option<RagMetrics>,
    ) -> Result<Self, RagError> {
        config.validate()?;
        let shared = Arc::new(Shared {
            pending: Mutex::new(Pending::default()),
            notification: Notify::new(),
            closed: AtomicBool::new(false),
            metrics,
        });
        tokio::spawn(run_queue(embedder, Arc::clone(&shared), config.clone()));
        Ok(Self {
            shared,
            per_org_concurrency: config.per_org_concurrency,
            global_slots: Arc::new(Semaphore::new(config.total_concurrency)),
            global_count: Arc::new(RequestCount::default()),
            org_slots: Mutex::new(HashMap::new()),
        })
    }

    pub async fn embed(&self, org: OrgId, texts: Vec<String>) -> Result<Vec<Vec<f32>>, RagError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let total = texts.len();
        let _admission = self.admit(org).await?;

        let (tx, mut rx) = mpsc::unbounded_channel();
        let items: Vec<Item> = texts
            .into_iter()
            .enumerate()
            .map(|(position, text)| Item {
                text,
                position,
                reply: tx.clone(),
            })
            .collect();
        drop(tx);
        {
            let mut pending = lock_pending(&self.shared.pending);
            if self.shared.closed.load(Ordering::SeqCst) {
                return Err(RagError::QueueClosed);
            }
            pending.add(org, items);
        }
        self.shared.notification.notify_one();
        if let Some(metrics) = &self.shared.metrics {
            metrics.pending_chunks.set(self.pending_chunks() as i64);
        }

        let mut output: Vec<Option<Vec<f32>>> = vec![None; total];
        for _ in 0..total {
            match rx.recv().await {
                Some((position, Ok(vector))) => output[position] = Some(vector),
                Some((_, Err(error))) => return Err(error),
                None => return Err(RagError::QueueClosed),
            }
        }
        output
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or(RagError::QueueClosed)
    }

    pub fn pending_chunks(&self) -> usize {
        lock_pending(&self.shared.pending).total
    }

    pub fn active_requests(&self) -> usize {
        self.global_count.current.load(Ordering::SeqCst)
    }

    pub fn peak_requests(&self) -> usize {
        self.global_count.peak.load(Ordering::SeqCst)
    }

    pub fn peak_org_requests(&self, org: OrgId) -> usize {
        match lock_slots(&self.org_slots).get(&org) {
            Some((_, count)) => count.peak.load(Ordering::SeqCst),
            None => 0,
        }
    }

    async fn admit(&self, org: OrgId) -> Result<Admission, RagError> {
        let (org_semaphore, org_count) = self.slots_for_org(org);
        let org_permit = org_semaphore
            .acquire_owned()
            .await
            .map_err(|_| RagError::QueueClosed)?;
        let total = Arc::clone(&self.global_slots)
            .acquire_owned()
            .await
            .map_err(|_| RagError::QueueClosed)?;
        org_count.enter();
        self.global_count.enter();
        Ok(Admission {
            _org: org_permit,
            _total: total,
            org_count,
            global_count: Arc::clone(&self.global_count),
        })
    }

    fn slots_for_org(&self, org: OrgId) -> (Arc<Semaphore>, Arc<RequestCount>) {
        let mut map = lock_slots(&self.org_slots);
        let (semaphore, count) = map.entry(org).or_insert_with(|| {
            (
                Arc::new(Semaphore::new(self.per_org_concurrency)),
                Arc::new(RequestCount::default()),
            )
        });
        (Arc::clone(semaphore), Arc::clone(count))
    }
}

impl Drop for EmbeddingQueue {
    fn drop(&mut self) {
        close(&self.shared);
    }
}

fn close(shared: &Shared) {
    let pending = {
        let mut guard = lock_pending(&shared.pending);
        shared.closed.store(true, Ordering::SeqCst);
        std::mem::take(&mut *guard)
    };
    drop(pending);
    shared.notification.notify_one();
}

struct WorkerGuard(Arc<Shared>);

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        close(&self.0);
    }
}

async fn run_queue(embedder: Arc<dyn Embedder>, shared: Arc<Shared>, config: QueueConfig) {
    let _guard = WorkerGuard(Arc::clone(&shared));
    loop {
        loop {
            if shared.closed.load(Ordering::SeqCst) {
                return;
            }
            if lock_pending(&shared.pending).total > 0 {
                break;
            }
            shared.notification.notified().await;
        }

        let deadline = Instant::now() + config.batch_window;
        loop {
            let total = lock_pending(&shared.pending).total;
            if total >= config.batch_size || shared.closed.load(Ordering::SeqCst) {
                break;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let _ = tokio::time::timeout(remaining, shared.notification.notified()).await;
        }

        let (batch, remaining_chunks) = {
            let mut pending = lock_pending(&shared.pending);
            let batch = pending.take_batch(config.batch_size);
            (batch, pending.total)
        };
        if let Some(metrics) = &shared.metrics {
            metrics.pending_chunks.set(remaining_chunks as i64);
        }
        if batch.is_empty() {
            continue;
        }
        process_batch(embedder.as_ref(), &shared, batch).await;
    }
}

async fn process_batch(embedder: &dyn Embedder, shared: &Shared, batch: Vec<Item>) {
    let start = Instant::now();
    let mut texts = Vec::with_capacity(batch.len());
    let mut recipients = Vec::with_capacity(batch.len());
    for item in batch {
        texts.push(item.text);
        recipients.push((item.position, item.reply));
    }
    let count = texts.len();
    let result = embedder.embed_batch(&texts).await;
    let seconds = start.elapsed().as_secs_f64();

    match result {
        Ok(vectors) if vectors.len() == count => {
            if let Some(metrics) = &shared.metrics {
                metrics.record_batch(count, seconds);
            }
            for ((position, reply), vector) in recipients.into_iter().zip(vectors) {
                let _ = reply.send((position, Ok(vector)));
            }
        }
        Ok(vectors) => {
            let error = RagError::Inference(format!(
                "esperava {count} vetores e recebi {}",
                vectors.len()
            ));
            reply_with_error(recipients, error, shared);
        }
        Err(error) => reply_with_error(recipients, error, shared),
    }
}

fn reply_with_error(recipients: Vec<(usize, Reply)>, error: RagError, shared: &Shared) {
    if let Some(metrics) = &shared.metrics {
        metrics.record_error();
    }
    for (position, reply) in recipients {
        let _ = reply.send((position, Err(error.clone())));
    }
}

fn lock_pending(pending: &Mutex<Pending>) -> MutexGuard<'_, Pending> {
    match pending.lock() {
        Ok(guard) => guard,
        Err(poison) => poison.into_inner(),
    }
}

fn lock_slots(slots: &Mutex<OrgSlots>) -> MutexGuard<'_, OrgSlots> {
    match slots.lock() {
        Ok(guard) => guard,
        Err(poison) => poison.into_inner(),
    }
}

pub struct QueuedEmbedder {
    queue: Arc<EmbeddingQueue>,
    org: OrgId,
    identity: ModelIdentity,
    _runtime: Option<Arc<tokio::runtime::Runtime>>,
}

impl QueuedEmbedder {
    pub fn new(queue: Arc<EmbeddingQueue>, org: OrgId, identity: ModelIdentity) -> Self {
        Self {
            queue,
            org,
            identity,
            _runtime: None,
        }
    }

    pub fn with_runtime(
        queue: Arc<EmbeddingQueue>,
        org: OrgId,
        identity: ModelIdentity,
        runtime: Arc<tokio::runtime::Runtime>,
    ) -> Self {
        Self {
            queue,
            org,
            identity,
            _runtime: Some(runtime),
        }
    }
}

impl Embedder for QueuedEmbedder {
    fn identity(&self) -> &ModelIdentity {
        &self.identity
    }

    fn embed_batch<'a>(&'a self, texts: &'a [String]) -> crate::embedder::BatchFuture<'a> {
        let queue = Arc::clone(&self.queue);
        let org = self.org;
        let texts = texts.to_vec();
        Box::pin(async move { queue.embed(org, texts).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(text: &str, position: usize, tx: &Reply) -> Item {
        Item {
            text: text.to_owned(),
            position,
            reply: tx.clone(),
        }
    }

    #[tokio::test]
    async fn adapter_embeds_through_queue_and_respects_org_slots() {
        use crate::fake::FakeEmbedder;

        let fake = Arc::new(FakeEmbedder::new().with_delay(Duration::from_millis(20)));
        let identity = fake.identity().clone();
        let embedder: Arc<dyn Embedder> = Arc::clone(&fake) as Arc<dyn Embedder>;
        let queue = Arc::new(
            EmbeddingQueue::start(
                Arc::clone(&embedder),
                QueueConfig {
                    per_org_concurrency: 1,
                    total_concurrency: 4,
                    ..QueueConfig::default()
                },
                None,
            )
            .expect("fila"),
        );
        let org = OrgId::generate();
        let adapter = Arc::new(QueuedEmbedder::new(
            Arc::clone(&queue),
            org,
            identity.clone(),
        ));
        assert_eq!(adapter.identity(), &identity, "identidade é a do modelo");

        let a = vec!["primeiro".to_owned()];
        let b = vec!["segundo".to_owned()];
        let c = vec!["terceiro".to_owned()];
        let (ra, rb, rc) = tokio::join!(
            adapter.embed_batch(&a),
            adapter.embed_batch(&b),
            adapter.embed_batch(&c)
        );
        let ra = ra.expect("vetores");
        let rb = rb.expect("vetores");
        let rc = rc.expect("vetores");
        assert_eq!(ra[0], crate::fake::fake_vector("primeiro"));
        assert_eq!(rb[0], crate::fake::fake_vector("segundo"));
        assert_eq!(rc[0], crate::fake::fake_vector("terceiro"));
        assert_eq!(
            queue.peak_org_requests(org),
            1,
            "a vaga da org é respeitada: um pedido por vez"
        );
        assert_eq!(queue.active_requests(), 0, "as vagas voltaram");
    }

    #[test]
    fn batch_rotates_between_orgs() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (a, b) = (OrgId::generate(), OrgId::generate());
        let mut pending = Pending::default();
        pending.add(a, (0..6).map(|i| item("a", i, &tx)).collect());
        pending.add(b, (0..2).map(|i| item("b", i, &tx)).collect());

        let batch = pending.take_batch(4);
        let from_a = batch.iter().filter(|i| i.text == "a").count();
        let from_b = batch.iter().filter(|i| i.text == "b").count();
        assert_eq!(batch.len(), 4);
        assert_eq!(
            (from_a, from_b),
            (2, 2),
            "rodízio dá metade do lote a cada org"
        );
        assert_eq!(pending.total, 4);
    }

    #[test]
    fn cancelled_request_is_excluded_from_batch() {
        let (tx, rx) = mpsc::unbounded_channel();
        let org = OrgId::generate();
        let mut pending = Pending::default();
        pending.add(org, vec![item("sai", 0, &tx), item("fica", 1, &tx)]);
        drop(rx);
        let batch = pending.take_batch(8);
        assert!(batch.is_empty(), "itens de pedido fechado são descartados");
        assert_eq!(pending.total, 0);
    }
}
