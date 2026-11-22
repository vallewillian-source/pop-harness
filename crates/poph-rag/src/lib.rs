//! RAG do POP Harness (HR-15): embedding local, fila com lote e instalação do modelo.
//!
//! - [`Embedder`]: contrato de geração de vetores em lote, dyn-compatível.
//! - [`EmbedderOrt`]: implementação com ONNX Runtime (`ort` 2.0.0-rc.15) e o modelo MiniLM int8.
//!   Uma instância serve todas as orgs; a sessão é carregada no primeiro uso (ou com `aquecer`).
//! - [`FilaEmbedding`]: lote por tamanho/janela, rodízio entre orgs e limites de simultaneidade.
//! - [`EmbedderFake`]: embedding determinístico para o resto do sistema e para os testes.
//! - [`instalacao`]: baixa o modelo e confere o sha256 (usado por `poph model install`).
//! - [`MetricasRag`]: medidas no registro de métricas, sem HTTP.
//!
//! Único crate do Harness com FFI do embedding (D-08). Este código não escreve `unsafe`; o que
//! é nativo fica dentro das bibliotecas `ort` e `tokenizers`.

//! Índice de trechos (HR-16): `rag.db` com vetores int8 e FTS5, indexação incremental por hash
//! de origem, cache por agente com LRU e busca exata isolada por org.
//!
//! - [`Indice`]: abre o `rag.db`, indexa a lista de itens de um agente e faz as buscas.
//! - [`trechos`]: divisão do texto em trechos e hash de origem.
//! - [`vetor`]: quantização int8 e produto interno exato.

pub mod embedder;
pub mod erro;
pub mod fake;
pub mod fila;
pub mod indice;
pub mod instalacao;
pub mod memoria;
pub mod metricas;
pub mod onnx;
pub mod trechos;
pub mod vetor;

mod banco;

pub use embedder::{
    DIMENSAO, Embedder, FuturoLote, IdentidadeModelo, JANELA_TOKENS, NOME_MODELO, QUANTIZACAO,
    SOBREPOSICAO_CARACTERES, TAMANHO_TRECHO_CARACTERES, VERSAO_CHUNKER,
};
pub use erro::ErroRag;
pub use fake::EmbedderFake;
pub use fila::{ConfigFila, EmbedderDaFila, FilaEmbedding};
pub use indice::{
    Achado, AchadoProduto, ConfigBusca, ConfigIndice, Encontrado, Indice, ItemRag, ProdutoRag,
    RelatorioAbertura, RelatorioIndexacao, Trecho,
};
pub use metricas::MetricasRag;
pub use onnx::{CargaModelo, ConfigModelo, EmbedderOrt};
