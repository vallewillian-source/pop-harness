//! Arquivos do POP Harness (HR-23): recebimento, validação, armazenamento e extração.
//!
//! - [`receber`]: ponto de entrada. Confere tamanho (16 MB), tipo real pelo conteúdo, lê documento
//!   em processo isolado e grava o blob por hash da org. Devolve metadados e as partes para o LLM.
//! - [`tipo`]: tipo real pela assinatura; recusa programa, script, áudio e formato antigo do Office;
//!   pega extensão enganosa.
//! - [`envio`]: tipo real do arquivo que **vai** ao cliente (MX-48). Lista fechada e separada da de
//!   entrada: aceita imagem, documento, áudio e vídeo, porque o arquivo do atendente é entregue, não
//!   lido.
//! - [`extracao`]: leitura do documento num processo filho (`poph-extrator`), com prazo de tempo e
//!   teto de memória. PDF malicioso ou bomba de descompactação não derruba o processo principal.
//! - [`envelope`]: texto extraído sempre dentro do bloco não confiável.
//! - [`blob`]: `data/blobs/<org>/<sha256>`, deduplicado por hash.
//!
//! Sem HTTP, sem LLM de verdade e sem tabela `attachments`: isso é da HR-24 e do motor do turno.
//! Nenhum `unsafe` neste crate.

pub mod blob;
pub mod config;
pub mod envelope;
pub mod envio;
pub mod erro;
pub mod extracao;
pub mod isolamento;
pub mod leitores;
pub mod recebimento;
pub mod tipo;

pub use config::ConfigArquivos;
pub use envio::{FamiliaEnvio, TipoEnvio};
pub use erro::ErroArquivo;
pub use recebimento::{ArquivoRecebido, Conteudo, MetadadosArquivo, ParteArquivo, receber};
pub use tipo::{Tipo, classificar};
