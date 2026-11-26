use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use std::sync::OnceLock;

use ort::environment::Environment;
use ort::session::Session;
use ort::value::Tensor;
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

use crate::embedder::{
    BatchFuture, CHUNKER_VERSION, DIMENSION, Embedder, MODEL_NAME, ModelIdentity, QUANTIZATION,
    TOKEN_WINDOW,
};
use crate::error::RagError;
use crate::installation::{self, MODEL_FILE, MODEL_SHA256, TOKENIZER_FILE, TOKENIZER_SHA256};
use crate::memory;
use crate::metrics::RagMetrics;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelConfig {
    pub diretorio: PathBuf,
    pub threads: usize,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            diretorio: PathBuf::from("data/modelos"),
            threads: 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelLoad {
    pub seconds: f64,
    pub rss_before_bytes: Option<u64>,
    pub rss_after_bytes: Option<u64>,
}

impl ModelLoad {
    pub fn delta_rss_bytes(&self) -> Option<u64> {
        match (self.rss_before_bytes, self.rss_after_bytes) {
            (Some(before), Some(after)) => Some(after.saturating_sub(before)),
            _ => None,
        }
    }
}

pub struct OrtEmbedder {
    identity: ModelIdentity,
    model_path: PathBuf,
    tokenizer_path: PathBuf,
    threads: usize,
    session: tokio::sync::Mutex<Option<Arc<LoadedSession>>>,
    load_report: Mutex<Option<ModelLoad>>,
    rag_metrics: Option<RagMetrics>,
}

struct LoadedSession {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
}

impl OrtEmbedder {
    pub fn open_model(
        config: &ModelConfig,
        rag_metrics: Option<RagMetrics>,
    ) -> Result<Self, RagError> {
        if config.threads == 0 {
            return Err(RagError::Configuration(
                "o número de threads do modelo deve ser maior que zero".to_owned(),
            ));
        }
        let model_sha = installation::verify_file(&config.diretorio, MODEL_FILE, MODEL_SHA256)?;
        let tokenizer_sha =
            installation::verify_file(&config.diretorio, TOKENIZER_FILE, TOKENIZER_SHA256)?;
        Ok(Self {
            identity: ModelIdentity {
                name: MODEL_NAME,
                quantization: QUANTIZATION,
                dimension: DIMENSION,
                token_window: TOKEN_WINDOW,
                model_sha256: model_sha,
                tokenizer_sha256: tokenizer_sha,
                chunker_version: CHUNKER_VERSION,
            },
            model_path: config.diretorio.join(MODEL_FILE),
            tokenizer_path: config.diretorio.join(TOKENIZER_FILE),
            threads: config.threads,
            session: tokio::sync::Mutex::new(None),
            load_report: Mutex::new(None),
            rag_metrics,
        })
    }

    pub async fn warm_up(&self) -> Result<ModelLoad, RagError> {
        self.get_session().await?;
        self.last_load()
            .ok_or_else(|| RagError::Loading("carga não registrada".to_owned()))
    }

    pub fn last_load(&self) -> Option<ModelLoad> {
        match self.load_report.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    async fn get_session(&self) -> Result<Arc<LoadedSession>, RagError> {
        let mut guard = self.session.lock().await;
        if let Some(session) = guard.as_ref() {
            return Ok(Arc::clone(session));
        }
        let modelo = self.model_path.clone();
        let tokenizer = self.tokenizer_path.clone();
        let threads = self.threads;
        let rss_before = memory::rss_bytes();
        let start = Instant::now();
        let loaded =
            tokio::task::spawn_blocking(move || open_session(&modelo, &tokenizer, threads))
                .await
                .map_err(|failure| {
                    RagError::Loading(format!("tarefa de carga interrompida: {failure}"))
                })??;
        let load = ModelLoad {
            seconds: start.elapsed().as_secs_f64(),
            rss_before_bytes: rss_before,
            rss_after_bytes: memory::rss_bytes(),
        };
        tracing::info!(
            seconds = load.seconds,
            model_memory_mb = load.delta_rss_bytes().unwrap_or(0) / 1_048_576,
            "modelo de embedding carregado"
        );
        if let Some(rag_metrics) = &self.rag_metrics {
            rag_metrics.record_load(&load);
        }
        match self.load_report.lock() {
            Ok(mut g) => *g = Some(load),
            Err(poisoned) => *poisoned.into_inner() = Some(load),
        }
        let session = Arc::new(loaded);
        *guard = Some(Arc::clone(&session));
        Ok(session)
    }
}

impl Embedder for OrtEmbedder {
    fn identity(&self) -> &ModelIdentity {
        &self.identity
    }

    fn embed_batch<'a>(&'a self, texts: &'a [String]) -> BatchFuture<'a> {
        Box::pin(async move {
            if texts.is_empty() {
                return Ok(Vec::new());
            }
            let session = self.get_session().await?;
            let texts = texts.to_vec();
            tokio::task::spawn_blocking(move || session.infer(&texts))
                .await
                .map_err(|failure| RagError::Inference(format!("tarefa interrompida: {failure}")))?
        })
    }
}

static ENVIRONMENT: OnceLock<Result<Environment, String>> = OnceLock::new();

fn process_environment() -> Result<&'static Environment, RagError> {
    ENVIRONMENT
        .get_or_init(|| ort::init().build().map_err(|failure| failure.to_string()))
        .as_ref()
        .map_err(|failure| RagError::Loading(format!("ambiente do ONNX Runtime: {failure}")))
}

fn open_session(
    modelo: &Path,
    tokenizer: &Path,
    threads: usize,
) -> Result<LoadedSession, RagError> {
    let tokenizer = load_tokenizer(tokenizer)?;
    let environment = process_environment()?;
    let mut builder = Session::builder(environment).map_err(session_error)?;
    builder = builder.with_intra_threads(threads).map_err(session_error)?;
    builder = builder.with_memory_pattern(false).map_err(session_error)?;
    let session = builder.commit_from_file(modelo).map_err(session_error)?;
    Ok(LoadedSession {
        session: Mutex::new(session),
        tokenizer,
    })
}

fn session_error<R>(failure: ort::Error<R>) -> RagError {
    RagError::Loading(format!("sessão do modelo: {failure}"))
}

fn load_tokenizer(path: &Path) -> Result<Tokenizer, RagError> {
    let mut tokenizer = Tokenizer::from_file(path)
        .map_err(|failure| RagError::Loading(format!("tokenizador: {failure}")))?;
    tokenizer
        .with_truncation(Some(TruncationParams {
            max_length: TOKEN_WINDOW,
            ..Default::default()
        }))
        .map_err(|failure| RagError::Loading(format!("truncamento: {failure}")))?;
    tokenizer.with_padding(Some(PaddingParams {
        strategy: PaddingStrategy::BatchLongest,
        ..Default::default()
    }));
    Ok(tokenizer)
}

struct Encoded {
    ids: Vec<i64>,
    mask: Vec<i64>,
    rows: usize,
    tokens: usize,
}

fn encode(tokenizer: &Tokenizer, texts: &[&str]) -> Result<Encoded, RagError> {
    let encoded = tokenizer
        .encode_batch(texts.to_vec(), true)
        .map_err(|failure| RagError::Inference(format!("tokenização: {failure}")))?;
    let rows = encoded.len();
    let tokens = encoded.iter().map(|e| e.get_ids().len()).max().unwrap_or(0);
    let mut ids = vec![0i64; rows * tokens];
    let mut mask = vec![0i64; rows * tokens];
    for (i, e) in encoded.iter().enumerate() {
        for (j, v) in e.get_ids().iter().enumerate() {
            ids[i * tokens + j] = i64::from(*v);
        }
        for (j, v) in e.get_attention_mask().iter().enumerate() {
            mask[i * tokens + j] = i64::from(*v);
        }
    }
    Ok(Encoded {
        ids,
        mask,
        rows,
        tokens,
    })
}

impl LoadedSession {
    fn infer(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, RagError> {
        texts.iter().map(|texto| self.infer_one(texto)).collect()
    }

    fn infer_one(&self, texto: &str) -> Result<Vec<f32>, RagError> {
        let input = encode(&self.tokenizer, &[texto])?;
        let (rows, tokens) = (input.rows, input.tokens);
        let ids = Tensor::from_array(([rows, tokens], input.ids))
            .map_err(|failure| RagError::Inference(format!("tensor de ids: {failure}")))?;
        let mask = Tensor::from_array(([rows, tokens], input.mask.clone()))
            .map_err(|failure| RagError::Inference(format!("tensor de máscara: {failure}")))?;
        let type_ids = Tensor::from_array(([rows, tokens], vec![0i64; rows * tokens]))
            .map_err(|failure| RagError::Inference(format!("tensor de tipos: {failure}")))?;

        let mut session = match self.session.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let outputs = session
            .run(ort::inputs![
                "input_ids" => ids,
                "attention_mask" => mask,
                "token_type_ids" => type_ids,
            ])
            .map_err(|failure| RagError::Inference(format!("execução do modelo: {failure}")))?;

        let nome = ["sentence_embedding", "last_hidden_state"]
            .into_iter()
            .find(|n| outputs.contains_key(*n))
            .ok_or_else(|| RagError::Inference("saída do modelo não reconhecida".to_owned()))?;
        let output_value = outputs
            .get(nome)
            .ok_or_else(|| RagError::Inference("saída do modelo ausente".to_owned()))?;
        let (shape, dados) = output_value
            .try_extract_tensor::<f32>()
            .map_err(|failure| RagError::Inference(format!("leitura da saída: {failure}")))?;
        let dimensions: Vec<usize> = shape.iter().map(|d| *d as usize).collect();

        let vector = match dimensions.as_slice() {
            [_, dim] if *dim == DIMENSION => normalize(dados[..*dim].to_vec()),
            [_, n, dim] if *dim == DIMENSION && *n == tokens => {
                masked_mean(dados, (rows, tokens, *dim), &input.mask)
                    .into_iter()
                    .next()
                    .ok_or_else(|| RagError::Inference("média sem vetor".to_owned()))?
            }
            other => {
                return Err(RagError::Inference(format!(
                    "dimensão de saída inesperada: {other:?}"
                )));
            }
        };
        Ok(vector)
    }
}

fn masked_mean(dados: &[f32], shape: (usize, usize, usize), mask: &[i64]) -> Vec<Vec<f32>> {
    let (rows, tokens, dim) = shape;
    let mut saida = Vec::with_capacity(rows);
    for i in 0..rows {
        let mut sum = vec![0f32; dim];
        let mut token_count = 0f32;
        for j in 0..tokens {
            if mask[i * tokens + j] == 0 {
                continue;
            }
            token_count += 1.0;
            let base = (i * tokens + j) * dim;
            for d in 0..dim {
                sum[d] += dados[base + d];
            }
        }
        if token_count > 0.0 {
            for value in sum.iter_mut() {
                *value /= token_count;
            }
        }
        saida.push(normalize(sum));
    }
    saida
}

fn normalize(mut vector: Vec<f32>) -> Vec<f32> {
    let norm = vector
        .iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt();
    if norm > 0.0 {
        for value in vector.iter_mut() {
            *value = (f64::from(*value) / norm) as f32;
        }
    }
    vector
}
