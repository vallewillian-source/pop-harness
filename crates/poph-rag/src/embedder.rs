use std::future::Future;
use std::pin::Pin;

use crate::error::RagError;

pub const MODEL_NAME: &str = "paraphrase-multilingual-MiniLM-L12-v2";
pub const QUANTIZATION: &str = "int8";
pub const DIMENSION: usize = 384;
pub const TOKEN_WINDOW: usize = 128;
pub const CHUNKER_VERSION: &str = "trecho-350-sobreposicao-80-v1";
pub const CHUNK_SIZE_CHARS: usize = 350;
pub const OVERLAP_CHARS: usize = 80;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelIdentity {
    pub name: &'static str,
    pub quantization: &'static str,
    pub dimension: usize,
    pub token_window: usize,
    pub model_sha256: String,
    pub tokenizer_sha256: String,
    pub chunker_version: &'static str,
}

pub type BatchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<Vec<f32>>, RagError>> + Send + 'a>>;

pub trait Embedder: Send + Sync {
    fn identity(&self) -> &ModelIdentity;

    fn embed_batch<'a>(&'a self, texts: &'a [String]) -> BatchFuture<'a>;

    fn dimension(&self) -> usize {
        self.identity().dimension
    }
}
