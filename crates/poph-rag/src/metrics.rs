use poph_types::{Counter, Gauge, Histogram, Registry};

use crate::error::RagError;
use crate::onnx::ModelLoad;

#[derive(Debug, Clone)]
pub struct RagMetrics {
    pub model_memory_bytes: Gauge,
    pub pending_chunks: Gauge,
    pub chunks_per_second: Gauge,
    pub chunks_total: Counter,
    pub batches_total: Counter,
    pub errors_total: Counter,
    pub batch_seconds: Histogram,
    pub load_seconds: Histogram,
}

impl RagMetrics {
    pub fn register(registry: &Registry) -> Result<Self, RagError> {
        let error = |e: poph_types::DomainError| RagError::Configuration(e.to_string());
        Ok(Self {
            model_memory_bytes: registry
                .gauge(
                    "poph_embedding_memoria_modelo_bytes",
                    "Memória ocupada pelo modelo de embedding, em bytes.",
                )
                .map_err(error)?,
            pending_chunks: registry
                .gauge(
                    "poph_embedding_trechos_pendentes",
                    "Trechos esperando na fila de embedding.",
                )
                .map_err(error)?,
            chunks_per_second: registry
                .gauge(
                    "poph_embedding_trechos_por_segundo",
                    "Trechos por segundo no último lote.",
                )
                .map_err(error)?,
            chunks_total: registry
                .counter(
                    "poph_embedding_trechos_total",
                    "Trechos com embedding gerado.",
                )
                .map_err(error)?,
            batches_total: registry
                .counter(
                    "poph_embedding_lotes_total",
                    "Lotes de embedding processados.",
                )
                .map_err(error)?,
            errors_total: registry
                .counter(
                    "poph_embedding_erros_total",
                    "Lotes de embedding que falharam.",
                )
                .map_err(error)?,
            batch_seconds: registry
                .histogram(
                    "poph_embedding_lote_segundos",
                    "Tempo de cada lote de embedding, em segundos.",
                    &[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0],
                )
                .map_err(error)?,
            load_seconds: registry
                .histogram(
                    "poph_embedding_carga_segundos",
                    "Tempo para carregar o modelo de embedding, em segundos.",
                    &[0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0],
                )
                .map_err(error)?,
        })
    }

    pub(crate) fn record_load(&self, load: &ModelLoad) {
        self.load_seconds.observe(load.seconds);
        let delta = load.delta_rss_bytes().unwrap_or(0);
        self.model_memory_bytes
            .set(i64::try_from(delta).unwrap_or(i64::MAX));
    }

    pub(crate) fn record_batch(&self, chunks: usize, seconds: f64) {
        self.batches_total.increment(1);
        self.chunks_total.increment(chunks as u64);
        self.batch_seconds.observe(seconds);
        if seconds > 0.0 {
            let throughput = (chunks as f64 / seconds).round();
            self.chunks_per_second.set(throughput as i64);
        }
    }

    pub(crate) fn record_error(&self) {
        self.errors_total.increment(1);
    }
}
