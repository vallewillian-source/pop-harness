use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poph_rag::fake::fake_vector;
use poph_rag::{
    BatchFuture, Embedder, EmbeddingQueue, FakeEmbedder, ModelIdentity, QueueConfig, RagError,
};
use poph_types::{OrgId, Registry};

fn queue_with(embedder: Arc<FakeEmbedder>, config: QueueConfig) -> Arc<EmbeddingQueue> {
    let dyn_embedder: Arc<dyn Embedder> = embedder;
    Arc::new(EmbeddingQueue::start(dyn_embedder, config, None).expect("fila sobe"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hundred_requests_across_three_orgs_obey_limits_and_return_expected_vectors() {
    let embedder = Arc::new(FakeEmbedder::new());
    let queue = queue_with(
        Arc::clone(&embedder),
        QueueConfig {
            batch_size: 32,
            batch_window: Duration::from_millis(2),
            per_org_concurrency: 3,
            total_concurrency: 8,
        },
    );
    let orgs = [OrgId::generate(), OrgId::generate(), OrgId::generate()];

    let mut tasks = Vec::new();
    for i in 0..100usize {
        let queue = Arc::clone(&queue);
        let org = orgs[i % 3];
        tasks.push(tokio::spawn(async move {
            let texts = vec![
                format!("pedido {i} primeiro trecho"),
                format!("pedido {i} segundo"),
            ];
            let expected: Vec<Vec<f32>> = texts.iter().map(|t| fake_vector(t)).collect();
            let actual = queue.embed(org, texts).await.expect("embedding");
            actual == expected
        }));
    }
    for task in tasks {
        assert!(
            task.await.expect("tarefa"),
            "vetores trocados entre pedidos"
        );
    }

    assert!(
        queue.peak_requests() <= 8,
        "passou do limite total: {}",
        queue.peak_requests()
    );
    assert!(queue.peak_requests() >= 2, "a concorrência não aconteceu");
    for org in orgs {
        assert!(
            queue.peak_org_requests(org) <= 3,
            "org passou do próprio limite"
        );
    }
    assert_eq!(queue.pending_chunks(), 0, "nada pode ficar parado na fila");
    assert_eq!(queue.active_requests(), 0, "vagas devolvidas ao fim");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn saturated_org_does_not_delay_another_org() {
    let embedder = Arc::new(FakeEmbedder::new().with_delay(Duration::from_millis(15)));
    let queue = queue_with(
        Arc::clone(&embedder),
        QueueConfig {
            batch_size: 32,
            batch_window: Duration::from_millis(1),
            per_org_concurrency: 2,
            total_concurrency: 8,
        },
    );
    let saturada = OrgId::generate();
    let other_org = OrgId::generate();

    let saturated_completions = Arc::new(Mutex::new(0usize));
    let mut tasks = Vec::new();
    for i in 0..40usize {
        let queue = Arc::clone(&queue);
        let completions = Arc::clone(&saturated_completions);
        tasks.push(tokio::spawn(async move {
            queue
                .embed(saturada, vec![format!("org saturada {i}")])
                .await
                .expect("embedding");
            *completions.lock().expect("trava") += 1;
        }));
    }
    tokio::time::sleep(Duration::from_millis(5)).await;

    let start = Instant::now();
    let vectors = queue
        .embed(other_org, vec!["org livre".to_owned()])
        .await
        .expect("a outra org é atendida");
    let other_org_elapsed = start.elapsed();
    let completions_when_other_org_finished = *saturated_completions.lock().expect("trava");

    assert_eq!(vectors, vec![fake_vector("org livre")]);
    assert!(
        other_org_elapsed < Duration::from_millis(150),
        "a outra org esperou {other_org_elapsed:?}, como se a saturada travasse a fila"
    );
    assert!(
        completions_when_other_org_finished < 40,
        "a outra org só terminou depois de toda a saturada ({completions_when_other_org_finished}/40)"
    );
    for task in tasks {
        task.await.expect("tarefa da saturada");
    }
    assert!(queue.peak_org_requests(saturada) <= 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn batch_never_exceeds_configured_size() {
    let embedder = Arc::new(FakeEmbedder::new());
    let queue = queue_with(
        Arc::clone(&embedder),
        QueueConfig {
            batch_size: 4,
            batch_window: Duration::from_millis(50),
            per_org_concurrency: 4,
            total_concurrency: 4,
        },
    );
    let texts: Vec<String> = (0..10).map(|i| format!("trecho {i}")).collect();

    let vectors = queue
        .embed(OrgId::generate(), texts.clone())
        .await
        .expect("embedding");

    assert_eq!(vectors.len(), 10);
    assert!(
        embedder.largest_batch() <= 4,
        "lote de {} trechos",
        embedder.largest_batch()
    );
    assert!(
        embedder.processed_batches() >= 3,
        "10 trechos em lotes de 4 pedem ao menos 3 lotes"
    );
}

#[tokio::test]
async fn empty_input_returns_empty_without_calling_embedder() {
    let embedder = Arc::new(FakeEmbedder::new());
    let queue = queue_with(Arc::clone(&embedder), QueueConfig::default());

    let vectors = queue
        .embed(OrgId::generate(), Vec::new())
        .await
        .expect("vazio");

    assert!(vectors.is_empty());
    assert_eq!(embedder.processed_batches(), 0);
}

#[derive(Debug)]
struct FailingEmbedder {
    model_identity: ModelIdentity,
}

impl Embedder for FailingEmbedder {
    fn identity(&self) -> &ModelIdentity {
        &self.model_identity
    }

    fn embed_batch<'a>(&'a self, _texts: &'a [String]) -> BatchFuture<'a> {
        Box::pin(async { Err(RagError::Inference("falha simulada".to_owned())) })
    }
}

#[tokio::test]
async fn model_failure_reaches_caller() {
    let embedder: Arc<dyn Embedder> = Arc::new(FailingEmbedder {
        model_identity: FakeEmbedder::new().identity().clone(),
    });
    let queue = EmbeddingQueue::start(embedder, QueueConfig::default(), None).expect("fila sobe");

    let error = queue
        .embed(OrgId::generate(), vec!["a".to_owned(), "b".to_owned()])
        .await
        .expect_err("a falha precisa aparecer");

    assert_eq!(error, RagError::Inference("falha simulada".to_owned()));
}

#[tokio::test]
async fn metrics_count_chunks_batches_and_errors() {
    let registro = Registry::new();
    let metrics = poph_rag::RagMetrics::register(&registro).expect("registra");
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new());
    let queue =
        EmbeddingQueue::start(embedder, QueueConfig::default(), Some(metrics)).expect("fila sobe");

    queue
        .embed(
            OrgId::generate(),
            vec!["um".to_owned(), "dois".to_owned(), "três".to_owned()],
        )
        .await
        .expect("embedding");

    let texto = registro.render();
    assert!(
        texto.contains("poph_embedding_trechos_total 3\n"),
        "{texto}"
    );
    assert!(texto.contains("poph_embedding_lotes_total 1\n"), "{texto}");
    assert!(texto.contains("poph_embedding_erros_total 0\n"), "{texto}");
    assert!(
        texto.contains("# TYPE poph_embedding_lote_segundos histogram"),
        "{texto}"
    );
}

#[tokio::test]
async fn zero_configuration_is_rejected() {
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new());
    let rejected = EmbeddingQueue::start(
        embedder,
        QueueConfig {
            batch_size: 0,
            ..QueueConfig::default()
        },
        None,
    );
    assert!(matches!(rejected, Err(RagError::Configuration(_))));
}
