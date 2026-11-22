//! Erros do cliente da API do Matrix (MX-09).
//!
//! Duas regras, as mesmas do cliente da Evolution:
//!
//! - **Nada de segredo no erro.** O `as_token` viaja só no cabeçalho `Authorization` e nenhuma
//!   variante deste enum carrega token, URL com consulta ou corpo inteiro de resposta. A mensagem
//!   que o Synapse manda é recortada e limpa antes de virar texto de erro.
//! - **Transitório é explícito.** Quem decide repetir é o laço do módulo, não o cliente: cada erro
//!   responde [`ErroChamada::classificacao`], e o cliente só repete sozinho o caso de
//!   `429 M_LIMIT_EXCEEDED` com `retry_after_ms` (uma vez, respeitando a espera pedida).
//!
//! O `Debug` é derivado de propósito: todos os campos são seguros para log (código HTTP,
//! `errcode`, mensagem recortada do Synapse e operação). Nenhum deles é o token.

use std::fmt;

/// Como o laço deve tratar a falha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classificacao {
    /// Vale tentar de novo mais tarde: rede, prazo, `429` e `5xx`.
    Transitoria,
    /// Repetir não resolve sem mudar alguma coisa: `4xx` de contrato, resposta malformada ou
    /// configuração inválida.
    Definitiva,
}

impl fmt::Display for Classificacao {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Classificacao::Transitoria => f.write_str("transitória"),
            Classificacao::Definitiva => f.write_str("definitiva"),
        }
    }
}

/// Falha ao falar com a API do Matrix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErroChamada {
    /// Configuração inválida (URL, `server_name` ou token vazio). A mensagem nomeia o campo,
    /// nunca repete o valor.
    Configuracao(String),
    /// Não deu para falar com o Synapse (conexão, handshake TLS, conexão fechada no meio).
    Rede {
        /// Operação do cliente que falhou (ex.: `createRoom`).
        operacao: &'static str,
        /// Motivo curto, sem o endereço da requisição.
        motivo: String,
    },
    /// O Synapse não respondeu dentro do prazo configurado.
    Prazo {
        /// Operação do cliente que estourou o prazo.
        operacao: &'static str,
        /// Prazo aplicado, em milissegundos.
        ms: u64,
    },
    /// O Synapse respondeu um status de erro HTTP.
    Resposta {
        /// Operação do cliente que recebeu o erro.
        operacao: &'static str,
        /// Código HTTP.
        status: u16,
        /// `errcode` do Matrix, quando veio (ex.: `M_LIMIT_EXCEEDED`).
        errcode: Option<String>,
        /// Mensagem curta do Synapse, já recortada. Nunca o corpo inteiro.
        mensagem: String,
        /// `retry_after_ms` pedido pelo Synapse em um `429`, quando veio.
        retry_after_ms: Option<u64>,
    },
    /// A resposta tem status de sucesso mas não é o JSON que o contrato promete.
    RespostaInvalida {
        /// Operação do cliente que recebeu a resposta.
        operacao: &'static str,
        /// O que não bateu (campo ausente, corpo que não é JSON, corpo grande demais).
        motivo: String,
    },
}

impl ErroChamada {
    /// Como o laço deve tratar esta falha. Não repete nada aqui: só informa.
    pub fn classificacao(&self) -> Classificacao {
        match self {
            ErroChamada::Rede { .. } | ErroChamada::Prazo { .. } => Classificacao::Transitoria,
            ErroChamada::Resposta { status, .. } => {
                if *status == 429 || *status >= 500 {
                    Classificacao::Transitoria
                } else {
                    Classificacao::Definitiva
                }
            }
            ErroChamada::Configuracao(_) | ErroChamada::RespostaInvalida { .. } => {
                Classificacao::Definitiva
            }
        }
    }

    /// `true` quando repetir mais tarde pode dar certo.
    pub fn transitorio(&self) -> bool {
        self.classificacao() == Classificacao::Transitoria
    }

    /// Espera que o Synapse pediu (`retry_after_ms` de um `429`), quando pediu.
    pub fn retry_after_ms(&self) -> Option<u64> {
        match self {
            ErroChamada::Resposta { retry_after_ms, .. } => *retry_after_ms,
            _ => None,
        }
    }
}

impl fmt::Display for ErroChamada {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErroChamada::Configuracao(motivo) => {
                write!(f, "configuração do cliente Matrix inválida: {motivo}")
            }
            ErroChamada::Rede { operacao, motivo } => {
                write!(
                    f,
                    "falha de rede em `{operacao}` ao falar com o Synapse: {motivo}"
                )
            }
            ErroChamada::Prazo { operacao, ms } => {
                write!(f, "o Synapse não respondeu `{operacao}` em {ms} ms")
            }
            ErroChamada::Resposta {
                operacao,
                status,
                errcode,
                mensagem,
                retry_after_ms,
            } => {
                write!(f, "o Synapse respondeu {status} em `{operacao}`")?;
                if let Some(errcode) = errcode {
                    write!(f, " ({errcode})")?;
                }
                write!(f, ": {mensagem}")?;
                if let Some(ms) = retry_after_ms {
                    write!(f, " [pediu espera de {ms} ms]")?;
                }
                Ok(())
            }
            ErroChamada::RespostaInvalida { operacao, motivo } => {
                write!(
                    f,
                    "resposta inesperada do Synapse em `{operacao}`: {motivo}"
                )
            }
        }
    }
}

impl std::error::Error for ErroChamada {}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn rede_e_prazo_sao_transitorios_e_5xx_tambem() {
        for erro in [
            ErroChamada::Rede {
                operacao: "createRoom",
                motivo: "não consegui conectar".to_owned(),
            },
            ErroChamada::Prazo {
                operacao: "send",
                ms: 10,
            },
            ErroChamada::Resposta {
                operacao: "createRoom",
                status: 503,
                errcode: None,
                mensagem: "indisponível".to_owned(),
                retry_after_ms: None,
            },
            ErroChamada::Resposta {
                operacao: "createRoom",
                status: 429,
                errcode: Some("M_LIMIT_EXCEEDED".to_owned()),
                mensagem: "calma".to_owned(),
                retry_after_ms: Some(500),
            },
        ] {
            assert!(erro.transitorio(), "deveria ser transitório: {erro}");
        }
    }

    #[test]
    fn contrato_e_configuracao_sao_definitivos() {
        for erro in [
            ErroChamada::Configuracao("a URL deve começar com http:// ou https://".to_owned()),
            ErroChamada::Resposta {
                operacao: "invite",
                status: 403,
                errcode: Some("M_FORBIDDEN".to_owned()),
                mensagem: "sem permissão".to_owned(),
                retry_after_ms: None,
            },
            ErroChamada::RespostaInvalida {
                operacao: "createRoom",
                motivo: "campo `room_id` ausente".to_owned(),
            },
        ] {
            assert!(!erro.transitorio(), "não deveria ser transitório: {erro}");
            assert_eq!(erro.classificacao(), Classificacao::Definitiva);
        }
    }

    #[test]
    fn display_mostra_status_errcode_e_espera_sem_valor_de_token() {
        let erro = ErroChamada::Resposta {
            operacao: "send",
            status: 429,
            errcode: Some("M_LIMIT_EXCEEDED".to_owned()),
            mensagem: "Too Many Requests".to_owned(),
            retry_after_ms: Some(1500),
        };
        let texto = erro.to_string();
        assert!(texto.contains("429"), "{texto}");
        assert!(texto.contains("M_LIMIT_EXCEEDED"), "{texto}");
        assert!(texto.contains("1500 ms"), "{texto}");
        assert_eq!(erro.retry_after_ms(), Some(1500));
    }
}
