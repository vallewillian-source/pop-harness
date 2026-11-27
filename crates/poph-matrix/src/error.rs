use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    Transient,
    Definitive,
}

impl fmt::Display for Classification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Classification::Transient => f.write_str("transitória"),
            Classification::Definitive => f.write_str("definitiva"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    Configuration(String),
    Network {
        operation: &'static str,
        reason: String,
    },
    Deadline {
        operation: &'static str,
        ms: u64,
    },
    Response {
        operation: &'static str,
        status: u16,
        errcode: Option<String>,
        message: String,
        retry_after_ms: Option<u64>,
    },
    InvalidResponse {
        operation: &'static str,
        reason: String,
    },
}

impl CallError {
    pub fn classification(&self) -> Classification {
        match self {
            CallError::Network { .. } | CallError::Deadline { .. } => Classification::Transient,
            CallError::Response { status, .. } => {
                if *status == 429 || *status >= 500 {
                    Classification::Transient
                } else {
                    Classification::Definitive
                }
            }
            CallError::Configuration(_) | CallError::InvalidResponse { .. } => {
                Classification::Definitive
            }
        }
    }

    pub fn transient(&self) -> bool {
        self.classification() == Classification::Transient
    }

    pub fn retry_after_ms(&self) -> Option<u64> {
        match self {
            CallError::Response { retry_after_ms, .. } => *retry_after_ms,
            _ => None,
        }
    }
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Configuration(reason) => {
                write!(f, "configuração do cliente Matrix inválida: {reason}")
            }
            CallError::Network { operation, reason } => {
                write!(
                    f,
                    "falha de rede em `{operation}` ao falar com o Synapse: {reason}"
                )
            }
            CallError::Deadline { operation, ms } => {
                write!(f, "o Synapse não respondeu `{operation}` em {ms} ms")
            }
            CallError::Response {
                operation,
                status,
                errcode,
                message,
                retry_after_ms,
            } => {
                write!(f, "o Synapse respondeu {status} em `{operation}`")?;
                if let Some(errcode) = errcode {
                    write!(f, " ({errcode})")?;
                }
                write!(f, ": {message}")?;
                if let Some(ms) = retry_after_ms {
                    write!(f, " [pediu espera de {ms} ms]")?;
                }
                Ok(())
            }
            CallError::InvalidResponse { operation, reason } => {
                write!(
                    f,
                    "resposta inesperada do Synapse em `{operation}`: {reason}"
                )
            }
        }
    }
}

impl std::error::Error for CallError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetError {
    Call(CallError),
    Large { bytes: u64, cap: u64 },
    Empty,
    Address(String),
}

impl AssetError {
    pub fn classification(&self) -> Classification {
        match self {
            AssetError::Call(error) => error.classification(),
            AssetError::Large { .. } | AssetError::Empty | AssetError::Address(_) => {
                Classification::Definitive
            }
        }
    }

    pub fn transient(&self) -> bool {
        self.classification() == Classification::Transient
    }
}

impl From<CallError> for AssetError {
    fn from(error: CallError) -> Self {
        AssetError::Call(error)
    }
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetError::Call(error) => write!(f, "{error}"),
            AssetError::Large { bytes, cap } => {
                write!(
                    f,
                    "a mídia tem {bytes} bytes e passa do teto de {cap} bytes"
                )
            }
            AssetError::Empty => f.write_str("a mídia veio vazia"),
            AssetError::Address(reason) => {
                write!(f, "o endereço da mídia não serve: {reason}")
            }
        }
    }
}

impl std::error::Error for AssetError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_and_deadline_are_transient_so_is_5xx() {
        for error in [
            CallError::Network {
                operation: "createRoom",
                reason: "não consegui conectar".to_owned(),
            },
            CallError::Deadline {
                operation: "send",
                ms: 10,
            },
            CallError::Response {
                operation: "createRoom",
                status: 503,
                errcode: None,
                message: "indisponível".to_owned(),
                retry_after_ms: None,
            },
            CallError::Response {
                operation: "createRoom",
                status: 429,
                errcode: Some("M_LIMIT_EXCEEDED".to_owned()),
                message: "calma".to_owned(),
                retry_after_ms: Some(500),
            },
        ] {
            assert!(error.transient(), "deveria ser transitório: {error}");
        }
    }

    #[test]
    fn contract_and_configuration_are_definitive() {
        for error in [
            CallError::Configuration("a URL deve começar com http:// ou https://".to_owned()),
            CallError::Response {
                operation: "invite",
                status: 403,
                errcode: Some("M_FORBIDDEN".to_owned()),
                message: "sem permissão".to_owned(),
                retry_after_ms: None,
            },
            CallError::InvalidResponse {
                operation: "createRoom",
                reason: "campo `room_id` ausente".to_owned(),
            },
        ] {
            assert!(!error.transient(), "não deveria ser transitório: {error}");
            assert_eq!(error.classification(), Classification::Definitive);
        }
    }

    #[test]
    fn display_shows_status_errcode_and_wait_without_token_value() {
        let error = CallError::Response {
            operation: "send",
            status: 429,
            errcode: Some("M_LIMIT_EXCEEDED".to_owned()),
            message: "Too Many Requests".to_owned(),
            retry_after_ms: Some(1500),
        };
        let text = error.to_string();
        assert!(text.contains("429"), "{text}");
        assert!(text.contains("M_LIMIT_EXCEEDED"), "{text}");
        assert!(text.contains("1500 ms"), "{text}");
        assert_eq!(error.retry_after_ms(), Some(1500));
    }

    #[test]
    fn attachment_error_separates_what_retry_solves() {
        let transient = AssetError::Call(CallError::Network {
            operation: "download",
            reason: "não consegui conectar".to_owned(),
        });
        assert!(transient.transient(), "{transient}");

        for definitivo in [
            AssetError::Call(CallError::Response {
                operation: "download",
                status: 404,
                errcode: Some("M_NOT_FOUND".to_owned()),
                message: "não achei".to_owned(),
                retry_after_ms: None,
            }),
            AssetError::Large { bytes: 20, cap: 16 },
            AssetError::Empty,
            AssetError::Address("o evento não trouxe url".to_owned()),
        ] {
            assert!(
                !definitivo.transient(),
                "deveria ser definitivo: {definitivo}"
            );
            assert_eq!(definitivo.classification(), Classification::Definitive);
        }

        let text = AssetError::Address("falta o identificador".to_owned()).to_string();
        assert!(text.contains("endereço da mídia"), "{text}");
        assert!(!text.contains("mxc://"), "{text}");
    }
}
