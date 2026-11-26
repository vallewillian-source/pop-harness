pub mod chunks;
pub mod embedder;
pub mod erro;
pub mod fake;
pub mod index;
pub mod installation;
pub mod memory;
pub mod metrics;
pub mod onnx;
pub mod queue;
pub mod vector;

mod database;

pub use embedder::{
    BatchFuture, CHUNK_SIZE_CHARS, CHUNKER_VERSION, DIMENSION, Embedder, MODEL_NAME, ModelIdentity,
    OVERLAP_CHARS, QUANTIZATION, TOKEN_WINDOW,
};
pub use erro::RagError;
pub use fake::FakeEmbedder;
pub use index::{
    Chunk, Found, Hit, Index, IndexConfig, IndexReport, ItemRag, OpenReport, ProductHit,
    ProductRag, SearchConfig,
};
pub use metrics::RagMetrics;
pub use onnx::{CargaModelo, ConfigModelo, EmbedderOrt};
pub use queue::{EmbeddingQueue, QueueConfig, QueuedEmbedder};
