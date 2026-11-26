//! Embedder com ONNX Runtime (`ort`) e o modelo MiniLM int8 (D-09).
//!
//! - Na abertura ([`EmbedderOrt::abrir`]) confere o sha256 do modelo e do tokenizador. Arquivo
//!   ausente ou diferente impede a subida, com erro tipado.
//! - A sessão do ONNX Runtime só é criada no primeiro uso (carga preguiçosa), ou antes, com
//!   [`EmbedderOrt::aquecer`]. Ela é uma só para todas as orgs.
//! - Tokenizador e truncamento em 128 tokens, como no spike HR-02; o vetor sai com média
//!   ponderada pela máscara e normalização L2.

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
use crate::erro::RagError;
use crate::installation::{self, MODEL_FILE, MODEL_SHA256, TOKENIZER_FILE, TOKENIZER_SHA256};
use crate::memory;
use crate::metrics::RagMetrics;

/// Configuração de abertura do modelo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigModelo {
    /// Pasta onde estão `model_int8.onnx` e `tokenizer.json`.
    pub diretorio: PathBuf,
    /// Threads internas do ONNX Runtime. O spike mediu 4 como o ponto bom (RELATORIO_HR02 §3.1).
    pub threads: usize,
}

impl Default for ConfigModelo {
    fn default() -> Self {
        Self {
            diretorio: PathBuf::from("data/modelos"),
            threads: 4,
        }
    }
}

/// Medida da carga da sessão: tempo e memória residente antes e depois.
#[derive(Debug, Clone, PartialEq)]
pub struct CargaModelo {
    /// Tempo de carga, em segundos.
    pub segundos: f64,
    /// Memória residente do processo antes da carga, em bytes (quando dá para ler).
    pub rss_antes_bytes: Option<u64>,
    /// Memória residente do processo depois da carga, em bytes (quando dá para ler).
    pub rss_depois_bytes: Option<u64>,
}

impl CargaModelo {
    /// Quanto a memória residente cresceu com a carga, em bytes.
    pub fn delta_rss_bytes(&self) -> Option<u64> {
        match (self.rss_antes_bytes, self.rss_depois_bytes) {
            (Some(antes), Some(depois)) => Some(depois.saturating_sub(antes)),
            _ => None,
        }
    }
}

/// Embedder ONNX. Guarde em `Arc` e compartilhe entre todas as orgs.
pub struct EmbedderOrt {
    identidade: ModelIdentity,
    caminho_modelo: PathBuf,
    caminho_tokenizador: PathBuf,
    threads: usize,
    sessao: tokio::sync::Mutex<Option<Arc<Sessao>>>,
    carga: Mutex<Option<CargaModelo>>,
    metricas: Option<RagMetrics>,
}

/// Sessão carregada: runtime, tokenizador e o ambiente que precisa ficar vivo junto.
struct Sessao {
    sessao: Mutex<Session>,
    tokenizador: Tokenizer,
}

impl EmbedderOrt {
    /// Abre o modelo da pasta dada: confere os sha256 e prepara a abertura da sessão.
    ///
    /// Bloqueante (lê cerca de 128 MB para o hash). Chame antes de servir, não dentro de um turno.
    pub fn abrir(config: &ConfigModelo, metricas: Option<RagMetrics>) -> Result<Self, RagError> {
        if config.threads == 0 {
            return Err(RagError::Configuration(
                "o número de threads do modelo deve ser maior que zero".to_owned(),
            ));
        }
        let sha_modelo = installation::verify_file(&config.diretorio, MODEL_FILE, MODEL_SHA256)?;
        let sha_tokenizador =
            installation::verify_file(&config.diretorio, TOKENIZER_FILE, TOKENIZER_SHA256)?;
        Ok(Self {
            identidade: ModelIdentity {
                name: MODEL_NAME,
                quantization: QUANTIZATION,
                dimension: DIMENSION,
                token_window: TOKEN_WINDOW,
                model_sha256: sha_modelo,
                tokenizer_sha256: sha_tokenizador,
                chunker_version: CHUNKER_VERSION,
            },
            caminho_modelo: config.diretorio.join(MODEL_FILE),
            caminho_tokenizador: config.diretorio.join(TOKENIZER_FILE),
            threads: config.threads,
            sessao: tokio::sync::Mutex::new(None),
            carga: Mutex::new(None),
            metricas,
        })
    }

    /// Carrega a sessão agora (em vez de no primeiro pedido) e devolve a medida da carga.
    pub async fn aquecer(&self) -> Result<CargaModelo, RagError> {
        self.sessao().await?;
        self.carga()
            .ok_or_else(|| RagError::Loading("carga não registrada".to_owned()))
    }

    /// Medida da última carga da sessão, se ela já aconteceu.
    pub fn carga(&self) -> Option<CargaModelo> {
        match self.carga.lock() {
            Ok(guarda) => guarda.clone(),
            Err(veneno) => veneno.into_inner().clone(),
        }
    }

    async fn sessao(&self) -> Result<Arc<Sessao>, RagError> {
        let mut guarda = self.sessao.lock().await;
        if let Some(sessao) = guarda.as_ref() {
            return Ok(Arc::clone(sessao));
        }
        let modelo = self.caminho_modelo.clone();
        let tokenizador = self.caminho_tokenizador.clone();
        let threads = self.threads;
        let rss_antes = memory::rss_bytes();
        let inicio = Instant::now();
        let sessao =
            tokio::task::spawn_blocking(move || abrir_sessao(&modelo, &tokenizador, threads))
                .await
                .map_err(|erro| {
                    RagError::Loading(format!("tarefa de carga interrompida: {erro}"))
                })??;
        let carga = CargaModelo {
            segundos: inicio.elapsed().as_secs_f64(),
            rss_antes_bytes: rss_antes,
            rss_depois_bytes: memory::rss_bytes(),
        };
        // Log sem caminho do modelo: só tempo e memória.
        tracing::info!(
            segundos = carga.segundos,
            memoria_modelo_mb = carga.delta_rss_bytes().unwrap_or(0) / 1_048_576,
            "modelo de embedding carregado"
        );
        if let Some(metricas) = &self.metricas {
            metricas.record_load(&carga);
        }
        match self.carga.lock() {
            Ok(mut g) => *g = Some(carga),
            Err(veneno) => *veneno.into_inner() = Some(carga),
        }
        let sessao = Arc::new(sessao);
        *guarda = Some(Arc::clone(&sessao));
        Ok(sessao)
    }
}

impl Embedder for EmbedderOrt {
    fn identity(&self) -> &ModelIdentity {
        &self.identidade
    }

    fn embed_batch<'a>(&'a self, textos: &'a [String]) -> BatchFuture<'a> {
        Box::pin(async move {
            if textos.is_empty() {
                return Ok(Vec::new());
            }
            let sessao = self.sessao().await?;
            let textos = textos.to_vec();
            tokio::task::spawn_blocking(move || sessao.inferir(&textos))
                .await
                .map_err(|erro| RagError::Inference(format!("tarefa interrompida: {erro}")))?
        })
    }
}

/// Ambiente do ONNX Runtime, único no processo. O `ort` só aceita uma criação por processo, mesmo
/// depois de soltar a primeira; por isso ele fica guardado para sempre aqui.
static AMBIENTE: OnceLock<Result<Environment, String>> = OnceLock::new();

fn ambiente_do_processo() -> Result<&'static Environment, RagError> {
    AMBIENTE
        .get_or_init(|| ort::init().build().map_err(|erro| erro.to_string()))
        .as_ref()
        .map_err(|erro| RagError::Loading(format!("ambiente do ONNX Runtime: {erro}")))
}

fn abrir_sessao(modelo: &Path, tokenizador: &Path, threads: usize) -> Result<Sessao, RagError> {
    let tokenizador = carregar_tokenizador(tokenizador)?;
    let ambiente = ambiente_do_processo()?;
    let mut construtor = Session::builder(ambiente).map_err(erro_sessao)?;
    construtor = construtor
        .with_intra_threads(threads)
        .map_err(erro_sessao)?;
    construtor = construtor.with_memory_pattern(false).map_err(erro_sessao)?;
    let sessao = construtor.commit_from_file(modelo).map_err(erro_sessao)?;
    Ok(Sessao {
        sessao: Mutex::new(sessao),
        tokenizador,
    })
}

fn erro_sessao<R>(erro: ort::Error<R>) -> RagError {
    RagError::Loading(format!("sessão do modelo: {erro}"))
}

fn carregar_tokenizador(caminho: &Path) -> Result<Tokenizer, RagError> {
    let mut tokenizador = Tokenizer::from_file(caminho)
        .map_err(|erro| RagError::Loading(format!("tokenizador: {erro}")))?;
    tokenizador
        .with_truncation(Some(TruncationParams {
            max_length: TOKEN_WINDOW,
            ..Default::default()
        }))
        .map_err(|erro| RagError::Loading(format!("truncamento: {erro}")))?;
    tokenizador.with_padding(Some(PaddingParams {
        strategy: PaddingStrategy::BatchLongest,
        ..Default::default()
    }));
    Ok(tokenizador)
}

/// Entradas do modelo para um lote: ids, máscara de atenção e dimensões.
struct Codificado {
    ids: Vec<i64>,
    mascara: Vec<i64>,
    lotes: usize,
    tokens: usize,
}

fn codificar(tokenizador: &Tokenizer, textos: &[&str]) -> Result<Codificado, RagError> {
    let codificados = tokenizador
        .encode_batch(textos.to_vec(), true)
        .map_err(|erro| RagError::Inference(format!("tokenização: {erro}")))?;
    let lotes = codificados.len();
    let tokens = codificados
        .iter()
        .map(|e| e.get_ids().len())
        .max()
        .unwrap_or(0);
    let mut ids = vec![0i64; lotes * tokens];
    let mut mascara = vec![0i64; lotes * tokens];
    for (i, e) in codificados.iter().enumerate() {
        for (j, v) in e.get_ids().iter().enumerate() {
            ids[i * tokens + j] = i64::from(*v);
        }
        for (j, v) in e.get_attention_mask().iter().enumerate() {
            mascara[i * tokens + j] = i64::from(*v);
        }
    }
    Ok(Codificado {
        ids,
        mascara,
        lotes,
        tokens,
    })
}

impl Sessao {
    /// Um vetor por texto. Cada trecho roda sozinho: na int8 (quantização dinâmica) a escala dos
    /// valores é calculada sobre o lote inteiro, então um trecho mudaria de vetor conforme os
    /// vizinhos (cosseno de 0,991 medido com lote de 9). Rodar um por vez custa o mesmo no spike.
    fn inferir(&self, textos: &[String]) -> Result<Vec<Vec<f32>>, RagError> {
        textos.iter().map(|texto| self.inferir_um(texto)).collect()
    }

    fn inferir_um(&self, texto: &str) -> Result<Vec<f32>, RagError> {
        let entrada = codificar(&self.tokenizador, &[texto])?;
        let (lotes, tokens) = (entrada.lotes, entrada.tokens);
        let ids = Tensor::from_array(([lotes, tokens], entrada.ids))
            .map_err(|erro| RagError::Inference(format!("tensor de ids: {erro}")))?;
        let mascara = Tensor::from_array(([lotes, tokens], entrada.mascara.clone()))
            .map_err(|erro| RagError::Inference(format!("tensor de máscara: {erro}")))?;
        let tipos = Tensor::from_array(([lotes, tokens], vec![0i64; lotes * tokens]))
            .map_err(|erro| RagError::Inference(format!("tensor de tipos: {erro}")))?;

        let mut sessao = match self.sessao.lock() {
            Ok(guarda) => guarda,
            Err(veneno) => veneno.into_inner(),
        };
        let saidas = sessao
            .run(ort::inputs![
                "input_ids" => ids,
                "attention_mask" => mascara,
                "token_type_ids" => tipos,
            ])
            .map_err(|erro| RagError::Inference(format!("execução do modelo: {erro}")))?;

        // Prefere o vetor já agrupado (sentence_embedding); senão, faz a média dos tokens.
        let nome = ["sentence_embedding", "last_hidden_state"]
            .into_iter()
            .find(|n| saidas.contains_key(*n))
            .ok_or_else(|| RagError::Inference("saída do modelo não reconhecida".to_owned()))?;
        let valor = saidas
            .get(nome)
            .ok_or_else(|| RagError::Inference("saída do modelo ausente".to_owned()))?;
        let (forma, dados) = valor
            .try_extract_tensor::<f32>()
            .map_err(|erro| RagError::Inference(format!("leitura da saída: {erro}")))?;
        let dimensoes: Vec<usize> = forma.iter().map(|d| *d as usize).collect();

        let vetor = match dimensoes.as_slice() {
            [_, dim] if *dim == DIMENSION => normalizar(dados[..*dim].to_vec()),
            [_, n, dim] if *dim == DIMENSION && *n == tokens => {
                media_com_mascara(dados, (lotes, tokens, *dim), &entrada.mascara)
                    .into_iter()
                    .next()
                    .ok_or_else(|| RagError::Inference("média sem vetor".to_owned()))?
            }
            outra => {
                return Err(RagError::Inference(format!(
                    "dimensão de saída inesperada: {outra:?}"
                )));
            }
        };
        Ok(vetor)
    }
}

/// Média dos tokens de cada texto, só com as posições da máscara; depois normaliza.
fn media_com_mascara(
    dados: &[f32],
    forma: (usize, usize, usize),
    mascara: &[i64],
) -> Vec<Vec<f32>> {
    let (lotes, tokens, dim) = forma;
    let mut saida = Vec::with_capacity(lotes);
    for i in 0..lotes {
        let mut soma = vec![0f32; dim];
        let mut contagem = 0f32;
        for j in 0..tokens {
            if mascara[i * tokens + j] == 0 {
                continue;
            }
            contagem += 1.0;
            let base = (i * tokens + j) * dim;
            for d in 0..dim {
                soma[d] += dados[base + d];
            }
        }
        if contagem > 0.0 {
            for valor in soma.iter_mut() {
                *valor /= contagem;
            }
        }
        saida.push(normalizar(soma));
    }
    saida
}

/// Normalização L2, com acumulação em f64 (como no spike, para os vetores baterem).
fn normalizar(mut vetor: Vec<f32>) -> Vec<f32> {
    let norma = vetor
        .iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt();
    if norma > 0.0 {
        for valor in vetor.iter_mut() {
            *valor = (f64::from(*valor) / norma) as f32;
        }
    }
    vetor
}
