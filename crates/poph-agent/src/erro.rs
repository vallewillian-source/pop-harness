//! Erro do estado quente e da concorrência (`poph-agent`).
//!
//! Os erros de domínio e de banco são reaproveitados de `poph-types`/`poph-store`: quem chama o
//! turno trata "agente inativo", "não encontrado" ou conflito de versão do mesmo jeito, venha de
//! onde vier. As variantes daqui cobrem o que é só desta camada: fila cheia, ator encerrado e
//! espera por vaga estourada.

use poph_types::DomainError;
use thiserror::Error;

/// Resultado padrão do crate.
pub type Resultado<T> = Result<T, ErroAgente>;

/// Tudo o que pode dar errado ao carregar a config quente, enfileirar uma conversa ou pedir vaga.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ErroAgente {
    /// Regra de domínio (agente inativo, id inválido, registro ausente).
    #[error(transparent)]
    Dominio(#[from] DomainError),

    /// Falha do banco ao recarregar do disco.
    #[error(transparent)]
    Store(#[from] poph_store::ErroStore),

    /// A fila da conversa está cheia: há mais eventos esperando do que o teto por conversa.
    #[error(
        "fila da conversa cheia ({capacidade} eventos): o evento não foi aceito; reenvie depois"
    )]
    FilaCheia { capacidade: usize },

    /// O ator da conversa encerrou entre o envio e a entrega (o canal fechou).
    #[error("o ator da conversa foi encerrado antes de receber o evento")]
    AtorEncerrado,

    /// Acabou o tempo de espera por uma vaga de simultaneidade.
    #[error("sem vaga de {escopo} em {espera_ms} ms: a instância está ocupada")]
    SemVaga { escopo: String, espera_ms: u64 },

    /// Acabou o tempo de espera por uma vaga de simultaneidade do LLM.
    #[error("sem vaga de chamada ao provedor `{provedor}` em {espera_ms} ms")]
    SemVagaProvedor { provedor: String, espera_ms: u64 },

    /// A cota de mensagens por minuto da org acabou.
    #[error("cota de {limite} mensagens por minuto da org esgotada: tente em {esperar_ms} ms")]
    CotaEsgotada { limite: u32, esperar_ms: u64 },

    /// O `payload_json` de um item da `inbox` não é um evento de entrada legível.
    #[error("evento de entrada inválido: {motivo}")]
    EventoInvalido {
        /// O que o parser reclamou.
        motivo: String,
    },

    /// Falha da busca semântica ou do índice de trechos (HR-17).
    #[error("busca na base de conhecimento: {0}")]
    Rag(#[from] poph_rag::ErroRag),

    /// Falha que não deveria acontecer (estado inconsistente, tarefa sem runtime).
    #[error("erro interno do estado quente: {0}")]
    Interno(String),
}

impl ErroAgente {
    /// `true` quando o erro é só "está ocupado, tente de novo" — quem chama pode repetir.
    pub fn eh_transitorio(&self) -> bool {
        matches!(
            self,
            ErroAgente::FilaCheia { .. }
                | ErroAgente::SemVaga { .. }
                | ErroAgente::SemVagaProvedor { .. }
                | ErroAgente::CotaEsgotada { .. }
        )
    }
}
