//! Contrato do embedding (`trait Embedder`) e a identidade do modelo gravada junto dos vetores.
//!
//! A HR-16 grava `versao_chunker`, o sha256 do modelo e o do tokenizador em `rag.db`, para
//! detectar quando a base foi gerada com outro modelo ou outro divisor de trechos.

use std::future::Future;
use std::pin::Pin;

use crate::erro::ErroRag;

/// Nome do modelo (o mesmo do POP Atende, D-07 do PRD).
pub const NOME_MODELO: &str = "paraphrase-multilingual-MiniLM-L12-v2";
/// Quantização usada na V1 (D-09). A troca exige reindexação.
pub const QUANTIZACAO: &str = "int8";
/// Dimensão dos vetores do modelo.
pub const DIMENSAO: usize = 384;
/// Janela do modelo em tokens; o tokenizador trunca nesse tamanho.
pub const JANELA_TOKENS: usize = 128;
/// Versão do divisor de trechos que a HR-16 implementa (D-09: ~350 caracteres, sobreposição 80).
pub const VERSAO_CHUNKER: &str = "trecho-350-sobreposicao-80-v1";
/// Tamanho-alvo de cada trecho, em caracteres (usado pela HR-16).
pub const TAMANHO_TRECHO_CARACTERES: usize = 350;
/// Sobreposição entre trechos vizinhos, em caracteres (usada pela HR-16).
pub const SOBREPOSICAO_CARACTERES: usize = 80;

/// O que identifica o modelo que gerou os vetores. Vai para `rag.db` na HR-16.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentidadeModelo {
    /// Nome do modelo.
    pub nome: &'static str,
    /// Quantização (`int8`, `fp32`, ou `fake` nos testes).
    pub quantizacao: &'static str,
    /// Dimensão dos vetores.
    pub dimensao: usize,
    /// Janela do modelo em tokens.
    pub janela_tokens: usize,
    /// sha256 do arquivo do modelo conferido na subida.
    pub sha256_modelo: String,
    /// sha256 do `tokenizer.json` conferido na subida (versão do tokenizador).
    pub sha256_tokenizador: String,
    /// Versão do divisor de trechos.
    pub versao_chunker: &'static str,
}

/// Resultado assíncrono de um lote: um vetor por texto, na mesma ordem.
pub type FuturoLote<'a> = Pin<Box<dyn Future<Output = Result<Vec<Vec<f32>>, ErroRag>> + Send + 'a>>;

/// Gera vetores de texto em lote. Uma instância serve a todas as orgs.
///
/// Usa futuros com caixa (`Pin<Box<..>>`) para continuar `dyn`-compatível, então a fila e os
/// testes podem guardar `Arc<dyn Embedder>`.
pub trait Embedder: Send + Sync {
    /// Identidade do modelo (vai para a base de vetores).
    fn identidade(&self) -> &IdentidadeModelo;

    /// Vetores de `textos`, um por texto e na mesma ordem. Lista vazia devolve lista vazia.
    fn embedir_lote<'a>(&'a self, textos: &'a [String]) -> FuturoLote<'a>;

    /// Dimensão dos vetores.
    fn dimensao(&self) -> usize {
        self.identidade().dimensao
    }
}
