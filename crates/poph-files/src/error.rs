use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum FileError {
    #[error("o arquivo está vazio")]
    Vazio,

    #[error("o arquivo tem {size_mb} MB e o limite é de {limit_mb} MB; não foi gravado")]
    Large { size_mb: f64, limit_mb: f64 },

    #[error("arquivo recusado: {0}")]
    UnsupportedType(String),

    #[error("arquivo recusado: o nome termina em .{declared}, mas o conteúdo é {real}")]
    MisleadingExtension { declared: String, real: String },

    #[error("não consegui ler o arquivo: {0}")]
    Reading(String),

    #[error("a leitura do arquivo passou de {limit_seconds} s e foi interrompida")]
    Time { limit_seconds: f64 },

    #[error("programa de leitura de arquivos indisponível: {0}")]
    Extrator(String),

    #[error("falha ao gravar o arquivo no disco: {0}")]
    WriteOutcome(String),

    #[error("configuração de arquivos inválida: {0}")]
    Configuracao(String),
}
