//! Métricas do embedding no registro que já existe em `poph-types` (sem HTTP; a HR-20 expõe).

use poph_types::{Contador, Histograma, Medidor, Registro};

use crate::erro::ErroRag;
use crate::onnx::CargaModelo;

/// Medidas do embedding: RAM do modelo, fila, tempo por lote e trechos por segundo.
#[derive(Debug, Clone)]
pub struct MetricasRag {
    /// Memória que o modelo ocupou ao carregar (variação do RSS), em bytes.
    pub memoria_modelo_bytes: Medidor,
    /// Trechos esperando na fila de embedding.
    pub trechos_pendentes: Medidor,
    /// Vazão do último lote, em trechos por segundo.
    pub trechos_por_segundo: Medidor,
    /// Trechos processados desde o início.
    pub trechos_total: Contador,
    /// Lotes processados desde o início.
    pub lotes_total: Contador,
    /// Lotes que falharam.
    pub erros_total: Contador,
    /// Tempo de cada lote, em segundos.
    pub lote_segundos: Histograma,
    /// Tempo de carregamento do modelo, em segundos.
    pub carga_segundos: Histograma,
}

impl MetricasRag {
    /// Registra o catálogo de embedding no registro dado. Não pode ser chamado duas vezes no mesmo registro.
    pub fn registrar(registro: &Registro) -> Result<Self, ErroRag> {
        let erro = |e: poph_types::DomainError| ErroRag::Configuracao(e.to_string());
        Ok(Self {
            memoria_modelo_bytes: registro
                .medidor(
                    "poph_embedding_memoria_modelo_bytes",
                    "Memória ocupada pelo modelo de embedding, em bytes.",
                )
                .map_err(erro)?,
            trechos_pendentes: registro
                .medidor(
                    "poph_embedding_trechos_pendentes",
                    "Trechos esperando na fila de embedding.",
                )
                .map_err(erro)?,
            trechos_por_segundo: registro
                .medidor(
                    "poph_embedding_trechos_por_segundo",
                    "Trechos por segundo no último lote.",
                )
                .map_err(erro)?,
            trechos_total: registro
                .contador(
                    "poph_embedding_trechos_total",
                    "Trechos com embedding gerado.",
                )
                .map_err(erro)?,
            lotes_total: registro
                .contador(
                    "poph_embedding_lotes_total",
                    "Lotes de embedding processados.",
                )
                .map_err(erro)?,
            erros_total: registro
                .contador(
                    "poph_embedding_erros_total",
                    "Lotes de embedding que falharam.",
                )
                .map_err(erro)?,
            lote_segundos: registro
                .histograma(
                    "poph_embedding_lote_segundos",
                    "Tempo de cada lote de embedding, em segundos.",
                    &[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0],
                )
                .map_err(erro)?,
            carga_segundos: registro
                .histograma(
                    "poph_embedding_carga_segundos",
                    "Tempo para carregar o modelo de embedding, em segundos.",
                    &[0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0],
                )
                .map_err(erro)?,
        })
    }

    pub(crate) fn registrar_carga(&self, carga: &CargaModelo) {
        self.carga_segundos.observar(carga.segundos);
        let delta = carga.delta_rss_bytes().unwrap_or(0);
        self.memoria_modelo_bytes
            .definir(i64::try_from(delta).unwrap_or(i64::MAX));
    }

    pub(crate) fn registrar_lote(&self, trechos: usize, segundos: f64) {
        self.lotes_total.incrementar(1);
        self.trechos_total.incrementar(trechos as u64);
        self.lote_segundos.observar(segundos);
        if segundos > 0.0 {
            let vazao = (trechos as f64 / segundos).round();
            self.trechos_por_segundo.definir(vazao as i64);
        }
    }

    pub(crate) fn registrar_erro(&self) {
        self.erros_total.incrementar(1);
    }
}
