//! Embedder determinístico para o resto do sistema e para os testes (HR-16 e HR-17 usam).
//!
//! Não carrega modelo nem usa rede. Cada palavra vira uma contribuição fixa num vetor de 384
//! posições, então textos com palavras em comum ficam próximos. Mesmo texto, mesmo vetor, sempre.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::embedder::{
    DIMENSAO, Embedder, FuturoLote, IdentidadeModelo, JANELA_TOKENS, NOME_MODELO, VERSAO_CHUNKER,
};

/// Embedder falso. `atraso` simula o tempo de um lote (para testes de concorrência).
#[derive(Debug)]
pub struct EmbedderFake {
    identidade: IdentidadeModelo,
    atraso: Duration,
    lotes: AtomicUsize,
    maior_lote: AtomicUsize,
}

impl Default for EmbedderFake {
    fn default() -> Self {
        Self::new()
    }
}

impl EmbedderFake {
    /// Embedder sem atraso.
    pub fn new() -> Self {
        Self {
            identidade: IdentidadeModelo {
                nome: NOME_MODELO,
                quantizacao: "fake",
                dimensao: DIMENSAO,
                janela_tokens: JANELA_TOKENS,
                sha256_modelo: "fake".to_owned(),
                sha256_tokenizador: "fake".to_owned(),
                versao_chunker: VERSAO_CHUNKER,
            },
            atraso: Duration::ZERO,
            lotes: AtomicUsize::new(0),
            maior_lote: AtomicUsize::new(0),
        }
    }

    /// Define o tempo que cada lote leva.
    pub fn com_atraso(mut self, atraso: Duration) -> Self {
        self.atraso = atraso;
        self
    }

    /// Quantos lotes o embedder recebeu.
    pub fn lotes_processados(&self) -> usize {
        self.lotes.load(Ordering::SeqCst)
    }

    /// Tamanho do maior lote recebido.
    pub fn maior_lote(&self) -> usize {
        self.maior_lote.load(Ordering::SeqCst)
    }
}

impl Embedder for EmbedderFake {
    fn identidade(&self) -> &IdentidadeModelo {
        &self.identidade
    }

    fn embedir_lote<'a>(&'a self, textos: &'a [String]) -> FuturoLote<'a> {
        Box::pin(async move {
            if textos.is_empty() {
                return Ok(Vec::new());
            }
            self.lotes.fetch_add(1, Ordering::SeqCst);
            self.maior_lote.fetch_max(textos.len(), Ordering::SeqCst);
            if !self.atraso.is_zero() {
                tokio::time::sleep(self.atraso).await;
            }
            Ok(textos.iter().map(|texto| vetor_fake(texto)).collect())
        })
    }
}

/// Vetor determinístico de um texto: cada palavra (minúscula) soma +1 ou -1 numa posição fixa.
/// Normalizado para comprimento 1; texto sem palavras vira o vetor unitário da primeira posição.
pub fn vetor_fake(texto: &str) -> Vec<f32> {
    let mut vetor = vec![0f32; DIMENSAO];
    for palavra in texto.split_whitespace() {
        let minuscula = palavra.to_lowercase();
        let hash = fnv1a(minuscula.as_bytes());
        let posicao = (hash % DIMENSAO as u64) as usize;
        let sinal = if (hash >> 40) & 1 == 0 { 1.0 } else { -1.0 };
        vetor[posicao] += sinal;
    }
    let norma = vetor
        .iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt();
    if norma == 0.0 {
        vetor[0] = 1.0;
        return vetor;
    }
    vetor
        .into_iter()
        .map(|x| (f64::from(x) / norma) as f32)
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
