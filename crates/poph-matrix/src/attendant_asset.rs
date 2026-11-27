use serde_json::{Value, json};

use crate::events::MSGTYPE_NOTICE;

pub use crate::asset::ASSET_BYTES_LIMIT;

pub const PREFIX_NOTICE: &str = "Não consegui enviar o arquivo ao cliente: ";

pub const NOTICE_TXN_PREFIX: &str = "poph-midia-atendente-";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotSendReason {
    Large,
    Empty,
    RefusedKind,
    WithoutFile,
    Unavailable,
    NotStored,
}

impl NotSendReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::Large => "arquivo_grande",
            Self::Empty => "arquivo_vazio",
            Self::RefusedKind => "tipo_recusado",
            Self::WithoutFile => "sem_arquivo_no_evento",
            Self::Unavailable => "arquivo_indisponivel",
            Self::NotStored => "arquivo_nao_gravado",
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Self::Large => "o arquivo passa de 16 MB",
            Self::Empty => "o arquivo está vazio",
            Self::RefusedKind => {
                "o tipo deste arquivo não é aceito (o cliente recebe imagem, PDF, Office, áudio e vídeo)"
            }
            Self::WithoutFile => "o evento não trouxe o arquivo",
            Self::Unavailable => "não consegui baixar o arquivo da sala",
            Self::NotStored => "não consegui guardar o arquivo para envio",
        }
    }
}

pub fn file_name(name: &str, default: &str) -> String {
    let clean = name.trim();
    if clean.is_empty() {
        default.to_owned()
    } else {
        clean.to_owned()
    }
}

pub fn notice_body(reason: NotSendReason) -> String {
    format!("{PREFIX_NOTICE}{}", reason.text())
}

pub fn notice_content(reason: NotSendReason) -> Value {
    json!({ "msgtype": MSGTYPE_NOTICE, "body": notice_body(reason) })
}

pub fn notice_txn(event_id: &str, reason: NotSendReason) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in event_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{NOTICE_TXN_PREFIX}{hash:016x}-{}", reason.code())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [NotSendReason; 6] = [
        NotSendReason::Large,
        NotSendReason::Empty,
        NotSendReason::RefusedKind,
        NotSendReason::WithoutFile,
        NotSendReason::Unavailable,
        NotSendReason::NotStored,
    ];

    #[test]
    fn each_reason_has_unique_code_and_own_text() {
        let mut codes = Vec::new();
        let mut texts = Vec::new();
        for reason in ALL {
            assert!(!codes.contains(&reason.code()), "{}", reason.code());
            assert!(!texts.contains(&reason.text()), "{}", reason.text());
            codes.push(reason.code());
            texts.push(reason.text());
        }
        assert_eq!(codes.len(), ALL.len());
    }

    #[test]
    fn notice_carries_reason_and_never_file_name() {
        for reason in ALL {
            let notice = notice_content(reason);
            assert_eq!(notice["msgtype"], json!(MSGTYPE_NOTICE));
            let body = notice["body"].as_str().expect("corpo é texto");
            assert!(body.starts_with(PREFIX_NOTICE), "corpo: {body}");
            assert!(body.ends_with(reason.text()), "corpo: {body}");
            assert!(!body.contains("laudo"), "o aviso não leva nome de arquivo");
        }
    }

    #[test]
    fn notice_txn_is_stable_per_event_and_differs_per_reason() {
        let event = "$evento-mx44:matrix-dev.local";
        assert_eq!(
            notice_txn(event, NotSendReason::Large),
            notice_txn(event, NotSendReason::Large)
        );
        assert_ne!(
            notice_txn(event, NotSendReason::Large),
            notice_txn("$outro:matrix-dev.local", NotSendReason::Large)
        );
        assert_ne!(
            notice_txn(event, NotSendReason::Large),
            notice_txn(event, NotSendReason::RefusedKind)
        );
        for reason in ALL {
            let txn = notice_txn(event, reason);
            assert!(
                txn.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~')),
                "txnId precisa ser seguro em URL: {txn}"
            );
        }
    }

    #[test]
    fn empty_name_gets_caller_default() {
        assert_eq!(file_name("  laudo.pdf ", "arquivo"), "laudo.pdf");
        assert_eq!(file_name("", "imagem"), "imagem");
        assert_eq!(file_name("   ", "arquivo"), "arquivo");
        assert_eq!(file_name("", "audio"), "audio");
        assert_eq!(file_name("", "video"), "video");
    }
}
