//! Erros da integração nativa do WhatsApp.
//!
//! Duas regras:
//!
//! - **Nada de segredo no erro.** A mensagem da Evolution é lida só do campo `message`/`error` do
//!   corpo (nunca o corpo inteiro, que pode ecoar o `apikey` enviado no cabeçalho) e o `Debug` deste
//!   tipo não carrega credencial nenhuma.
//! - **Transitório é explícito.** Queda de rede, prazo e 429/5xx são marcados como transitórios;
//!   400/401/404 e mídia expirada não. A `outbox` (D11) usa isso para decidir se tenta de novo
//!   mais tarde. O cliente, sozinho, só repete leitura (ver `cliente`): envio nunca repete dentro
//!   dele.

use poph_types::IntegrationError;
use thiserror::Error;

/// O que pode dar errado ao falar com a Evolution ou ao ler o que ela entrega.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ErroEvolution {
    /// Configuração incompleta ou inválida (endereço, instância, chave).
    #[error("configuração da integração inválida: {0}")]
    Configuracao(String),

    /// A Evolution respondeu erro HTTP. `mensagem` é o texto dela, nunca o corpo inteiro.
    #[error("a Evolution respondeu {status}: {mensagem}")]
    Resposta {
        /// Código HTTP.
        status: u16,
        /// Mensagem legível (do campo `message`/`error`).
        mensagem: String,
    },

    /// Não deu para falar com a Evolution (conexão, prazo, handshake TLS).
    #[error("falha de rede ao falar com a Evolution: {0}")]
    Rede(String),

    /// A mídia passa do teto do canal.
    #[error("mídia de {bytes} bytes passa do limite de {teto} bytes")]
    MidiaGrandeDemais {
        /// Tamanho tentado.
        bytes: u64,
        /// Teto do canal.
        teto: u64,
    },

    /// A Evolution não guarda mais a mídia: o download tem prazo de 4 dias
    /// (`docs/WHATSAPP_MEDIA.md`).
    #[error("mídia expirada na Evolution: {0}")]
    MidiaExpirada(String),

    /// A resposta não tem o formato que o contrato da Evolution promete.
    #[error("resposta inesperada da Evolution: {0}")]
    RespostaInvalida(String),

    /// A resposta de sucesso passou do teto de tamanho; a leitura parou ali, sem o corpo inteiro.
    #[error("a resposta da Evolution passou de {teto} bytes")]
    RespostaGrande {
        /// Teto aplicado à leitura.
        teto: u64,
    },

    /// O blob da biblioteca do agente não está em disco.
    #[error("não consegui ler o `{blob}` da biblioteca: {motivo}")]
    BlobIlegivel {
        /// Referência do blob (`<org>/<sha256>`).
        blob: String,
        /// O que o sistema de arquivos disse.
        motivo: String,
    },
}

impl ErroEvolution {
    /// `true` quando repetir mais tarde pode dar certo.
    pub fn transitorio(&self) -> bool {
        match self {
            ErroEvolution::Rede(_) => true,
            ErroEvolution::Resposta { status, .. } => *status == 429 || *status >= 500,
            _ => false,
        }
    }

    /// Converte para o erro do contrato de integração, que é o que a `outbox` e o turno entendem.
    pub fn para_integracao(&self, operacao: &str) -> IntegrationError {
        match self {
            ErroEvolution::Rede(motivo) => IntegrationError::Unavailable {
                reason: format!("{operacao}: {motivo}"),
            },
            ErroEvolution::Resposta { status, mensagem } => IntegrationError::Refused {
                operation: operacao.to_owned(),
                code: status.to_string(),
                message: mensagem.clone(),
                transient: *status == 429 || *status >= 500,
            },
            ErroEvolution::MidiaGrandeDemais { bytes, teto } => IntegrationError::MessageTooLarge {
                bytes: usize::try_from(*bytes).unwrap_or(usize::MAX),
                cap: usize::try_from(*teto).unwrap_or(usize::MAX),
            },
            ErroEvolution::MidiaExpirada(motivo) => IntegrationError::Refused {
                operation: operacao.to_owned(),
                code: "midia_expirada".to_owned(),
                message: motivo.clone(),
                transient: false,
            },
            ErroEvolution::Configuracao(motivo)
            | ErroEvolution::RespostaInvalida(motivo)
            | ErroEvolution::BlobIlegivel { motivo, .. } => IntegrationError::Refused {
                operation: operacao.to_owned(),
                code: "integracao_invalida".to_owned(),
                message: motivo.clone(),
                transient: false,
            },
            ErroEvolution::RespostaGrande { teto } => IntegrationError::Refused {
                operation: operacao.to_owned(),
                code: "resposta_grande".to_owned(),
                message: format!("a resposta passou de {teto} bytes"),
                transient: false,
            },
        }
    }
}
