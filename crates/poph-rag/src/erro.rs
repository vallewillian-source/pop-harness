//! Erros do crate `poph-rag`. Mensagens em português, prontas para a CLI e para o log.
//! Nenhuma mensagem traz caminho absoluto do modelo: só o nome do arquivo.

use thiserror::Error;

/// Erros do embedding e da instalação do modelo.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ErroRag {
    /// O arquivo do modelo (ou do tokenizador) não está na pasta de modelos.
    #[error("modelo de embedding não instalado ({arquivo}); rode `poph model install`")]
    ModeloAusente { arquivo: String },

    /// O arquivo existe, mas o sha256 não é o esperado: o modelo não sobe.
    #[error(
        "o arquivo {arquivo} do modelo não confere com o sha256 esperado (esperado {esperado}, obtido {obtido}); rode `poph model install` de novo"
    )]
    ChecksumNaoConfere {
        arquivo: String,
        esperado: String,
        obtido: String,
    },

    /// A sessão do ONNX Runtime ou o tokenizador não abriram.
    #[error("não consegui carregar o modelo de embedding: {0}")]
    Carregamento(String),

    /// A inferência falhou (tensor, saída inesperada, tarefa interrompida).
    #[error("falha ao gerar o embedding: {0}")]
    Inferencia(String),

    /// A fila foi encerrada antes de responder ao pedido.
    #[error("a fila de embedding foi encerrada antes de responder")]
    FilaEncerrada,

    /// Parâmetro inválido (zero em um limite, por exemplo).
    #[error("configuração de embedding inválida: {0}")]
    Configuracao(String),

    /// Falha de rede ou de resposta ao baixar um arquivo do modelo.
    #[error("falha ao baixar {arquivo}: {motivo}")]
    Download { arquivo: String, motivo: String },

    /// Falha de leitura ou escrita na pasta de modelos.
    #[error("falha de arquivo na pasta de modelos: {0}")]
    Arquivo(String),

    /// Falha do `rag.db` (SQLite): abrir, gravar, consultar ou dado gravado que não confere.
    #[error("falha no índice de busca (rag.db): {0}")]
    Banco(String),

    /// Item recusado antes de indexar (id repetido na lista, por exemplo).
    #[error("item de conhecimento inválido: {0}")]
    Item(String),
}
