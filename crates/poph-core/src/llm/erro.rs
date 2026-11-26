use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LlmError {
    #[error("configuração do LLM inválida: {0}")]
    Configuracao(String),

    #[error("pedido ao LLM recusado antes do envio: {0}")]
    InvalidRequest(String),

    #[error("o provedor recusou o pedido (HTTP {status}): {mensagem}")]
    Rejected { status: u16, mensagem: String },

    #[error("provedor indisponível depois de {tentativas} tentativa(s): {ultimo}")]
    Unavailable { tentativas: u32, ultimo: String },

    #[error("falha de TLS ao falar com o provedor: {0}")]
    Tls(String),

    #[error("a chamada ao provedor passou de {segundos} s")]
    TimedOut { segundos: u64 },

    #[error("conexão com o provedor caiu durante a resposta: {0}")]
    ConnectionInterrupted(String),

    #[error("resposta do provedor fora do formato esperado: {0}")]
    InvalidResponse(String),

    #[error("o provedor devolveu erro durante a resposta: {0}")]
    StreamError(String),

    #[error("chamada ao LLM cancelada")]
    Cancelled,
}

impl LlmError {
    pub fn is_transient(&self) -> bool {
        matches!(self, LlmError::Unavailable { .. })
    }
}
