//! Métricas do embedding no registro que já existe em `poph-types` (sem HTTP; a HR-20 expõe).

use poph_types::{Counter, Gauge, Histogram, Registry};

use crate::erro::ErroRag;
use crate::onnx::CargaModelo;

/// Medidas do embedding: RAM do modelo, fila, tempo por lote e trechos por segundo.
#[derive(Debug, Clone)]
pub struct MetricasRag {
    /// Memória que o modelo ocupou ao carregar (variação do RSS), em bytes.
    pub memoria_modelo_bytes: Gauge,
    /// Trechos esperando na fila de embedding.
    pub trechos_pendentes: Gauge,
    /// Vazão do último lote, em trechos por segundo.
    pub trechos_por_segundo: Gauge,
    /// Trechos processados desde o início.
    pub trechos_total: Counter,
    /// Lotes processados desde o início.
    pub lotes_total: Counter,
    /// Lotes que falharam.
    pub erros_total: Counter,
    /// Tempo de cada lote, em segundos.
    pub lote_segundos: Histogram,
    /// Tempo de carregamento do modelo, em segundos.
    pub carga_segundos: Histogram,
}

impl MetricasRag {
    /// Registra o catálogo de embedding no registro dado. Não pode ser chamado duas vezes no mesmo registro.
    pub fn registrar(registro: &Registry) -> Result<Self, ErroRag> {
        let erro = |e: poph_types::DomainError| ErroRag::Configuracao(e.to_string());
        Ok(Self {
            memoria_modelo_bytes: registro
                .gauge(
                    "poph_embedding_memoria_modelo_bytes",
                    "Memória ocupada pelo modelo de embedding, em bytes.",
                )
                .map_err(erro)?,
            trechos_pendentes: registro
                .gauge(
                    "poph_embedding_trechos_pendentes",
                    "Trechos esperando na fila de embedding.",
                )
                .map_err(erro)?,
            trechos_por_segundo: registro
                .gauge(
                    "poph_embedding_trechos_por_segundo",
                    "Trechos por segundo no último lote.",
                )
                .map_err(erro)?,
            trechos_total: registro
                .counter(
                    "poph_embedding_trechos_total",
                    "Trechos com embedding gerado.",
                )
                .map_err(erro)?,
            lotes_total: registro
                .counter(
                    "poph_embedding_lotes_total",
                    "Lotes de embedding processados.",
                )
                .map_err(erro)?,
            erros_total: registro
                .counter(
                    "poph_embedding_erros_total",
                    "Lotes de embedding que falharam.",
                )
                .map_err(erro)?,
            lote_segundos: registro
                .histogram(
                    "poph_embedding_lote_segundos",
                    "Tempo de cada lote de embedding, em segundos.",
                    &[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0],
                )
                .map_err(erro)?,
            carga_segundos: registro
                .histogram(
                    "poph_embedding_carga_segundos",
                    "Tempo para carregar o modelo de embedding, em segundos.",
                    &[0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0],
                )
                .map_err(erro)?,
        })
    }

    pub(crate) fn registrar_carga(&self, carga: &CargaModelo) {
        self.carga_segundos.observe(carga.segundos);
        let delta = carga.delta_rss_bytes().unwrap_or(0);
        self.memoria_modelo_bytes
            .set(i64::try_from(delta).unwrap_or(i64::MAX));
    }

    pub(crate) fn registrar_lote(&self, trechos: usize, segundos: f64) {
        self.lotes_total.increment(1);
        self.trechos_total.increment(trechos as u64);
        self.lote_segundos.observe(segundos);
        if segundos > 0.0 {
            let vazao = (trechos as f64 / segundos).round();
            self.trechos_por_segundo.set(vazao as i64);
        }
    }

    pub(crate) fn registrar_erro(&self) {
        self.erros_total.increment(1);
    }
}
