use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::embedder::{
    BatchFuture, CHUNKER_VERSION, DIMENSION, Embedder, MODEL_NAME, ModelIdentity, TOKEN_WINDOW,
};

#[derive(Debug)]
pub struct FakeEmbedder {
    identity: ModelIdentity,
    delay: Duration,
    batches: AtomicUsize,
    largest_batch: AtomicUsize,
}

impl Default for FakeEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeEmbedder {
    pub fn new() -> Self {
        Self {
            identity: ModelIdentity {
                name: MODEL_NAME,
                quantization: "fake",
                dimension: DIMENSION,
                token_window: TOKEN_WINDOW,
                model_sha256: "fake".to_owned(),
                tokenizer_sha256: "fake".to_owned(),
                chunker_version: CHUNKER_VERSION,
            },
            delay: Duration::ZERO,
            batches: AtomicUsize::new(0),
            largest_batch: AtomicUsize::new(0),
        }
    }

    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn processed_batches(&self) -> usize {
        self.batches.load(Ordering::SeqCst)
    }

    pub fn largest_batch(&self) -> usize {
        self.largest_batch.load(Ordering::SeqCst)
    }
}

impl Embedder for FakeEmbedder {
    fn identity(&self) -> &ModelIdentity {
        &self.identity
    }

    fn embed_batch<'a>(&'a self, texts: &'a [String]) -> BatchFuture<'a> {
        Box::pin(async move {
            if texts.is_empty() {
                return Ok(Vec::new());
            }
            self.batches.fetch_add(1, Ordering::SeqCst);
            self.largest_batch.fetch_max(texts.len(), Ordering::SeqCst);
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            Ok(texts.iter().map(|text| fake_vector(text)).collect())
        })
    }
}

pub fn fake_vector(text: &str) -> Vec<f32> {
    let mut vector = vec![0f32; DIMENSION];
    for word in text.split_whitespace() {
        let lowercase = word.to_lowercase();
        let hash = fnv1a(lowercase.as_bytes());
        let position = (hash % DIMENSION as u64) as usize;
        let sign = if (hash >> 40) & 1 == 0 { 1.0 } else { -1.0 };
        vector[position] += sign;
    }
    let norm = vector
        .iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt();
    if norm == 0.0 {
        vector[0] = 1.0;
        return vector;
    }
    vector
        .into_iter()
        .map(|x| (f64::from(x) / norm) as f32)
        .collect()
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
