use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum RagError {
    #[error("modelo de embedding não instalado ({file}); rode `poph model install`")]
    ModelMissing { file: String },

    #[error(
        "o arquivo {file} do modelo não confere com o sha256 esperado (esperado {expected}, obtido {actual}); rode `poph model install` de novo"
    )]
    ChecksumMismatch {
        file: String,
        expected: String,
        actual: String,
    },

    #[error("não consegui carregar o modelo de embedding: {0}")]
    Loading(String),

    #[error("falha ao gerar o embedding: {0}")]
    Inference(String),

    #[error("a fila de embedding foi encerrada antes de responder")]
    QueueClosed,

    #[error("configuração de embedding inválida: {0}")]
    Configuration(String),

    #[error("falha ao baixar {file}: {reason}")]
    Download { file: String, reason: String },

    #[error("falha de arquivo na pasta de modelos: {0}")]
    File(String),

    #[error("falha no índice de busca (rag.db): {0}")]
    Database(String),

    #[error("item de conhecimento inválido: {0}")]
    Item(String),
}
