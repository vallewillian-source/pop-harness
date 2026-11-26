//! Fila de embedding com o embedder falso (sem modelo, sem rede): lote, limites e concorrência.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poph_rag::fake::fake_vector;
use poph_rag::{
    BatchFuture, Embedder, EmbeddingQueue, FakeEmbedder, ModelIdentity, QueueConfig, RagError,
};
use poph_types::{OrgId, Registry};

fn fila_com(embedder: Arc<FakeEmbedder>, config: QueueConfig) -> Arc<EmbeddingQueue> {
    let dyn_embedder: Arc<dyn Embedder> = embedder;
    Arc::new(EmbeddingQueue::start(dyn_embedder, config, None).expect("fila sobe"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cem_pedidos_de_tres_orgs_respeitam_os_limites_e_voltam_certos() {
    let embedder = Arc::new(FakeEmbedder::new());
    let fila = fila_com(
        Arc::clone(&embedder),
        QueueConfig {
            batch_size: 32,
            batch_window: Duration::from_millis(2),
            per_org_concurrency: 3,
            total_concurrency: 8,
        },
    );
    let orgs = [OrgId::generate(), OrgId::generate(), OrgId::generate()];

    let mut tarefas = Vec::new();
    for i in 0..100usize {
        let fila = Arc::clone(&fila);
        let org = orgs[i % 3];
        tarefas.push(tokio::spawn(async move {
            let textos = vec![
                format!("pedido {i} primeiro trecho"),
                format!("pedido {i} segundo"),
            ];
            let esperado: Vec<Vec<f32>> = textos.iter().map(|t| fake_vector(t)).collect();
            let obtido = fila.embed(org, textos).await.expect("embedding");
            obtido == esperado
        }));
    }
    for tarefa in tarefas {
        assert!(
            tarefa.await.expect("tarefa"),
            "vetores trocados entre pedidos"
        );
    }

    assert!(
        fila.peak_requests() <= 8,
        "passou do limite total: {}",
        fila.peak_requests()
    );
    assert!(fila.peak_requests() >= 2, "a concorrência não aconteceu");
    for org in orgs {
        assert!(
            fila.peak_org_requests(org) <= 3,
            "org passou do próprio limite"
        );
    }
    assert_eq!(fila.pending_chunks(), 0, "nada pode ficar parado na fila");
    assert_eq!(fila.active_requests(), 0, "vagas devolvidas ao fim");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn org_saturada_nao_atrasa_outra_org() {
    let embedder = Arc::new(FakeEmbedder::new().with_delay(Duration::from_millis(15)));
    let fila = fila_com(
        Arc::clone(&embedder),
        QueueConfig {
            batch_size: 32,
            batch_window: Duration::from_millis(1),
            per_org_concurrency: 2,
            total_concurrency: 8,
        },
    );
    let saturada = OrgId::generate();
    let outra = OrgId::generate();

    let concluidos_da_saturada = Arc::new(Mutex::new(0usize));
    let mut tarefas = Vec::new();
    for i in 0..40usize {
        let fila = Arc::clone(&fila);
        let concluidos = Arc::clone(&concluidos_da_saturada);
        tarefas.push(tokio::spawn(async move {
            fila.embed(saturada, vec![format!("org saturada {i}")])
                .await
                .expect("embedding");
            *concluidos.lock().expect("trava") += 1;
        }));
    }
    // Dá tempo para a org saturada encher as próprias vagas e formar fila.
    tokio::time::sleep(Duration::from_millis(5)).await;

    let inicio = Instant::now();
    let vetores = fila
        .embed(outra, vec!["org livre".to_owned()])
        .await
        .expect("a outra org é atendida");
    let tempo_outra = inicio.elapsed();
    let concluidos_quando_outra_terminou = *concluidos_da_saturada.lock().expect("trava");

    assert_eq!(vetores, vec![fake_vector("org livre")]);
    assert!(
        tempo_outra < Duration::from_millis(150),
        "a outra org esperou {tempo_outra:?}, como se a saturada travasse a fila"
    );
    assert!(
        concluidos_quando_outra_terminou < 40,
        "a outra org só terminou depois de toda a saturada ({concluidos_quando_outra_terminou}/40)"
    );
    for tarefa in tarefas {
        tarefa.await.expect("tarefa da saturada");
    }
    assert!(fila.peak_org_requests(saturada) <= 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lote_nunca_passa_do_tamanho_configurado() {
    let embedder = Arc::new(FakeEmbedder::new());
    let fila = fila_com(
        Arc::clone(&embedder),
        QueueConfig {
            batch_size: 4,
            batch_window: Duration::from_millis(50),
            per_org_concurrency: 4,
            total_concurrency: 4,
        },
    );
    let textos: Vec<String> = (0..10).map(|i| format!("trecho {i}")).collect();

    let vetores = fila
        .embed(OrgId::generate(), textos.clone())
        .await
        .expect("embedding");

    assert_eq!(vetores.len(), 10);
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
async fn lista_vazia_volta_vazia_sem_tocar_o_embedder() {
    let embedder = Arc::new(FakeEmbedder::new());
    let fila = fila_com(Arc::clone(&embedder), QueueConfig::default());

    let vetores = fila
        .embed(OrgId::generate(), Vec::new())
        .await
        .expect("vazio");

    assert!(vetores.is_empty());
    assert_eq!(embedder.processed_batches(), 0);
}

/// Embedder que sempre falha, para ver o erro chegar a quem pediu.
#[derive(Debug)]
struct EmbedderQueFalha {
    identidade: ModelIdentity,
}

impl Embedder for EmbedderQueFalha {
    fn identity(&self) -> &ModelIdentity {
        &self.identidade
    }

    fn embed_batch<'a>(&'a self, _textos: &'a [String]) -> BatchFuture<'a> {
        Box::pin(async { Err(RagError::Inference("falha simulada".to_owned())) })
    }
}

#[tokio::test]
async fn falha_do_modelo_chega_a_quem_pediu() {
    let embedder: Arc<dyn Embedder> = Arc::new(EmbedderQueFalha {
        identidade: FakeEmbedder::new().identity().clone(),
    });
    let fila = EmbeddingQueue::start(embedder, QueueConfig::default(), None).expect("fila sobe");

    let erro = fila
        .embed(OrgId::generate(), vec!["a".to_owned(), "b".to_owned()])
        .await
        .expect_err("a falha precisa aparecer");

    assert_eq!(erro, RagError::Inference("falha simulada".to_owned()));
}

#[tokio::test]
async fn metricas_contam_trechos_lotes_e_erros() {
    let registro = Registry::new();
    let metricas = poph_rag::RagMetrics::register(&registro).expect("registra");
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new());
    let fila =
        EmbeddingQueue::start(embedder, QueueConfig::default(), Some(metricas)).expect("fila sobe");

    fila.embed(
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
async fn configuracao_com_zero_e_recusada() {
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new());
    let recusada = EmbeddingQueue::start(
        embedder,
        QueueConfig {
            batch_size: 0,
            ..QueueConfig::default()
        },
        None,
    );
    assert!(matches!(recusada, Err(RagError::Configuration(_))));
}
