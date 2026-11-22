//! Erros da retenção. Mensagens legíveis: aparecem no terminal e no relatório.

/// Falha na retenção de uma org ou de um passo dela.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErroRetencao {
    /// A configuração de retenção da org não é válida. A org é pulada inteira: nada é apagado.
    #[error("retenção da org {org} inválida: {motivo}. Nada foi apagado nessa org.")]
    ConfigInvalida {
        /// Id da org.
        org: String,
        /// O que está errado, em uma frase.
        motivo: String,
    },
    /// A org pedida não existe no banco. Nada é apagado.
    #[error("não existe org com o id {0}. Nada foi apagado.")]
    OrgInexistente(String),
    /// Falha no banco (repositório). Não há apagamento de arquivos depois de uma falha aqui.
    #[error("falha no banco: {0}")]
    Banco(String),
    /// Falha ao ler ou apagar arquivos da pasta de blobs.
    #[error("falha nos arquivos em {caminho}: {motivo}")]
    Arquivos {
        /// Pasta ou arquivo afetado.
        caminho: String,
        /// Motivo do sistema de arquivos.
        motivo: String,
    },
}
