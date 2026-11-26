use poph_types::IntegrationError;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EvolutionError {
    #[error("configuração da integração inválida: {0}")]
    InvalidConfiguration(String),

    #[error("a Evolution respondeu {status}: {message}")]
    Response { status: u16, message: String },

    #[error("falha de rede ao falar com a Evolution: {0}")]
    Network(String),

    #[error("mídia de {bytes} bytes passa do limite de {cap} bytes")]
    AttachmentTooLarge { bytes: u64, cap: u64 },

    #[error("mídia expirada na Evolution: {0}")]
    AttachmentExpired(String),

    #[error("resposta inesperada da Evolution: {0}")]
    InvalidResponse(String),

    #[error("a resposta da Evolution passou de {cap} bytes")]
    ResponseTooLarge { cap: u64 },

    #[error("não consegui ler o `{blob}` da biblioteca: {reason}")]
    BlobUnreadable { blob: String, reason: String },
}

impl EvolutionError {
    pub fn is_transient(&self) -> bool {
        match self {
            EvolutionError::Network(_) => true,
            EvolutionError::Response { status, .. } => *status == 429 || *status >= 500,
            _ => false,
        }
    }

    pub fn to_integration(&self, operation: &str) -> IntegrationError {
        match self {
            EvolutionError::Network(reason) => IntegrationError::Unavailable {
                reason: format!("{operation}: {reason}"),
            },
            EvolutionError::Response { status, message } => IntegrationError::Refused {
                operation: operation.to_owned(),
                code: status.to_string(),
                message: message.clone(),
                transient: *status == 429 || *status >= 500,
            },
            EvolutionError::AttachmentTooLarge { bytes, cap } => {
                IntegrationError::MessageTooLarge {
                    bytes: usize::try_from(*bytes).unwrap_or(usize::MAX),
                    cap: usize::try_from(*cap).unwrap_or(usize::MAX),
                }
            }
            EvolutionError::AttachmentExpired(reason) => IntegrationError::Refused {
                operation: operation.to_owned(),
                code: "midia_expirada".to_owned(),
                message: reason.clone(),
                transient: false,
            },
            EvolutionError::InvalidConfiguration(reason)
            | EvolutionError::InvalidResponse(reason)
            | EvolutionError::BlobUnreadable { reason, .. } => IntegrationError::Refused {
                operation: operation.to_owned(),
                code: "integracao_invalida".to_owned(),
                message: reason.clone(),
                transient: false,
            },
            EvolutionError::ResponseTooLarge { cap } => IntegrationError::Refused {
                operation: operation.to_owned(),
                code: "resposta_grande".to_owned(),
                message: format!("a resposta passou de {cap} bytes"),
                transient: false,
            },
        }
    }
}
