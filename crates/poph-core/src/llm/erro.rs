//! Erros do provedor de LLM. Mensagens nunca trazem a chave de API nem o conteúdo do cliente.

use thiserror::Error;

/// Falha numa chamada ao provedor de LLM.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ErroLlm {
    /// Configuração do cliente inválida (faltam valores, formato de endereço não aceito etc.).
    #[error("configuração do LLM inválida: {0}")]
    Configuracao(String),

    /// O pedido nem chegou a sair: formato recusado pela regra local (ex.: a última mensagem é do modelo).
    #[error("pedido ao LLM recusado antes do envio: {0}")]
    PedidoInvalido(String),

    /// O provedor recusou o pedido de forma definitiva (ex.: 400, 401). Não é retentado.
    #[error("o provedor recusou o pedido (HTTP {status}): {mensagem}")]
    Recusado { status: u16, mensagem: String },

    /// Falha transitória que persistiu depois de todas as retentativas.
    #[error("provedor indisponível depois de {tentativas} tentativa(s): {ultimo}")]
    Indisponivel { tentativas: u32, ultimo: String },

    /// Falha de TLS: certificado do servidor não é confiável, o nome não bate com o certificado ou
    /// o handshake foi recusado. Não é retentada — repetir não conserta certificado.
    #[error("falha de TLS ao falar com o provedor: {0}")]
    Tls(String),

    /// A chamada passou do tempo limite. Não é retentada: a espera já foi longa.
    #[error("a chamada ao provedor passou de {segundos} s")]
    TempoEsgotado { segundos: u64 },

    /// Falha de rede no meio de uma resposta já iniciada. Não é retentada: parte do texto já saiu.
    #[error("conexão com o provedor caiu durante a resposta: {0}")]
    ConexaoInterrompida(String),

    /// O provedor respondeu, mas fora do formato esperado (JSON inválido, resposta cortada etc.).
    #[error("resposta do provedor fora do formato esperado: {0}")]
    RespostaInvalida(String),

    /// O provedor avisou erro dentro do próprio stream.
    #[error("o provedor devolveu erro durante a resposta: {0}")]
    ErroNoStream(String),

    /// Quem chamou pediu o cancelamento.
    #[error("chamada ao LLM cancelada")]
    Cancelada,
}

impl ErroLlm {
    /// Se a falha merece nova tentativa (só antes de qualquer evento sair para quem chamou).
    pub fn transitorio(&self) -> bool {
        matches!(self, ErroLlm::Indisponivel { .. })
    }
}
