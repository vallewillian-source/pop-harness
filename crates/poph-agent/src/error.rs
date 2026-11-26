use poph_types::DomainError;
use thiserror::Error;

pub type Resultado<T> = Result<T, AgentError>;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AgentError {
    #[error(transparent)]
    Domain(#[from] DomainError),

    #[error(transparent)]
    Store(#[from] poph_store::StoreError),

    #[error(
        "fila da conversa cheia ({capacidade} eventos): o evento não foi aceito; reenvie depois"
    )]
    QueueCheia { capacidade: usize },

    #[error("o ator da conversa foi encerrado antes de receber o evento")]
    ActorClosed,

    #[error("sem vaga de {scope} em {wait_ms} ms: a instância está ocupada")]
    WithoutSlot { scope: String, wait_ms: u64 },

    #[error("sem vaga de chamada ao provedor `{provedor}` em {wait_ms} ms")]
    WithoutSlotProvider { provedor: String, wait_ms: u64 },

    #[error("cota de {limit} mensagens por minuto da org esgotada: tente em {wait_ms} ms")]
    CotaExhausted { limit: u32, wait_ms: u64 },

    #[error("evento de entrada inválido: {motivo}")]
    EventInvalid { motivo: String },

    #[error("busca na base de conhecimento: {0}")]
    Rag(#[from] poph_rag::RagError),

    #[error("erro interno do estado quente: {0}")]
    Internal(String),
}

impl AgentError {
    pub fn eh_transient(&self) -> bool {
        matches!(
            self,
            AgentError::QueueCheia { .. }
                | AgentError::WithoutSlot { .. }
                | AgentError::WithoutSlotProvider { .. }
                | AgentError::CotaExhausted { .. }
        )
    }
}
