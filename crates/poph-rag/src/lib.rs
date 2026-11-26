pub mod embedder;
pub mod erro;
pub mod fake;
pub mod fila;
pub mod indice;
pub mod instalacao;
pub mod memory;
pub mod metrics;
pub mod onnx;
pub mod trechos;
pub mod vector;

mod database;

pub use embedder::{
    BatchFuture, CHUNK_SIZE_CHARS, CHUNKER_VERSION, DIMENSION, Embedder, MODEL_NAME, ModelIdentity,
    OVERLAP_CHARS, QUANTIZATION, TOKEN_WINDOW,
};
pub use erro::RagError;
pub use fake::FakeEmbedder;
pub use fila::{ConfigFila, EmbedderDaFila, FilaEmbedding};
pub use indice::{
    Achado, AchadoProduto, ConfigBusca, ConfigIndice, Encontrado, Indice, ItemRag, ProdutoRag,
    RelatorioAbertura, RelatorioIndexacao, Trecho,
};
pub use metrics::RagMetrics;
pub use onnx::{CargaModelo, ConfigModelo, EmbedderOrt};
