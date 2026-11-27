use serde_json::{Value, json};

use crate::client::MatrixClient;
use crate::error::{AssetError, CallError};
use crate::users::Ghost;
use poph_types::Secret;

pub const EXPIRED_BLOCK: &str = "com.sofacoding.attachment_expired";

pub const NAME_FIELD: &str = "filename";

pub const KIND_FIELD: &str = "mimetype";

pub const SIZE_FIELD: &str = "size";

pub const SENT_AT_FIELD: &str = "sent_at";

pub const CATEGORY_FIELD: &str = "kind";

pub const REDACTION_REASON: &str = "arquivo expirado: o prazo de retenção terminou";

pub const REDACTION_TXN_PREFIX: &str = "poph-expira-";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetToExpire<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub mime: &'a str,
    pub category: &'a str,
    pub size_bytes: u64,
    pub sent_at: i64,
}

pub fn redaction_txn(id_attachment: &str) -> String {
    format!("{REDACTION_TXN_PREFIX}{id_attachment}")
}

pub fn redaction_content(asset: &AssetToExpire<'_>) -> Value {
    json!({
        "reason": REDACTION_REASON,
        EXPIRED_BLOCK: {
            NAME_FIELD: asset.name,
            KIND_FIELD: asset.mime,
            SIZE_FIELD: asset.size_bytes,
            SENT_AT_FIELD: iso_instant(asset.sent_at),
            CATEGORY_FIELD: asset.category,
        },
    })
}

pub fn iso_instant(ms: i64) -> String {
    let seconds = ms.div_euclid(1_000);
    let daies = seconds.div_euclid(86_400);
    let in_day = seconds.rem_euclid(86_400);
    let (year, month, day) = data_civil(daies);
    let hour = in_day / 3_600;
    let minuto = (in_day % 3_600) / 60;
    let second = in_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minuto:02}:{second:02}Z")
}

fn data_civil(daies: i64) -> (i64, u32, u32) {
    let z = daies + 719_468;
    let era = z.div_euclid(146_097);
    let epoch_day = z.rem_euclid(146_097);
    let epoch_year =
        (epoch_day - epoch_day / 1_460 + epoch_day / 36_524 - epoch_day / 146_096) / 365;
    let year = epoch_year + era * 400;
    let day_of_year = epoch_day - (365 * epoch_year + epoch_year / 4 - epoch_year / 100);
    let adjusted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * adjusted_month + 2) / 5 + 1;
    let month = if adjusted_month < 10 {
        adjusted_month + 3
    } else {
        adjusted_month - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month as u32, day as u32)
}

pub async fn redact(
    client: &MatrixClient,
    sender: &Ghost,
    room: &str,
    event: &str,
    asset: &AssetToExpire<'_>,
) -> Result<String, CallError> {
    client
        .redact(
            sender.user_id(),
            room,
            event,
            &redaction_txn(asset.id),
            &redaction_content(asset),
        )
        .await
}

pub async fn delete(
    client: &MatrixClient,
    token_admin: &Secret,
    mxc: &str,
) -> Result<bool, AssetError> {
    client.delete_asset(token_admin, mxc).await
}

pub fn error_code(error: &AssetError) -> String {
    match error {
        AssetError::Call(CallError::Response {
            errcode: Some(code),
            ..
        }) if safe_code(code) => code.clone(),
        AssetError::Call(CallError::Network { .. }) => "rede".to_owned(),
        AssetError::Call(CallError::Deadline { .. }) => "prazo".to_owned(),
        AssetError::Call(CallError::Configuration(_)) => "configuracao".to_owned(),
        AssetError::Call(CallError::InvalidResponse { .. }) => "resposta_invalida".to_owned(),
        AssetError::Call(CallError::Response { status, .. }) => {
            format!("http_{status}")
        }
        AssetError::Address(_) => "endereco_da_midia".to_owned(),
        AssetError::Large { .. } => "midia_grande".to_owned(),
        AssetError::Empty => "midia_vazia".to_owned(),
    }
}

fn safe_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset() -> AssetToExpire<'static> {
        AssetToExpire {
            id: "anx-9",
            name: "laudo.pdf",
            mime: "application/pdf",
            category: "document",
            size_bytes: 2_048,
            sent_at: 1_797_900_000_000,
        }
    }

    #[test]
    fn redaction_carries_frontend_block_with_five_fields() {
        let content = redaction_content(&asset());
        let block = &content[EXPIRED_BLOCK];
        assert_eq!(content["reason"], json!(REDACTION_REASON));
        assert_eq!(block[NAME_FIELD], json!("laudo.pdf"));
        assert_eq!(block[KIND_FIELD], json!("application/pdf"));
        assert_eq!(block[SIZE_FIELD], json!(2_048));
        assert_eq!(block[CATEGORY_FIELD], json!("document"));
        assert_eq!(block[SENT_AT_FIELD], json!("2026-12-22T00:40:00Z"));
        assert_eq!(
            block.as_object().map(|object| object.len()),
            Some(5),
            "o bloco não leva campo a mais: {block}"
        );
    }

    #[test]
    fn instant_in_iso_utc_does_not_depend_on_locale() {
        for (ms, expected) in [
            (0, "1970-01-01T00:00:00Z"),
            (1_000, "1970-01-01T00:00:01Z"),
            (1_797_900_000_000, "2026-12-22T00:40:00Z"),
            (1_767_225_599_000, "2025-12-31T23:59:59Z"),
            (951_782_400_000, "2000-02-29T00:00:00Z"),
        ] {
            assert_eq!(iso_instant(ms), expected, "ms {ms}");
        }
        assert_eq!(iso_instant(-86_400_000), "1969-12-31T00:00:00Z");
    }

    #[test]
    fn redaction_txn_is_stable_and_url_safe() {
        assert_eq!(redaction_txn("anx-1"), redaction_txn("anx-1"));
        assert_ne!(redaction_txn("anx-1"), redaction_txn("anx-2"));
        let txn = redaction_txn("018f-0000-7000-8000-000000000001");
        assert!(txn.starts_with(REDACTION_TXN_PREFIX));
        assert!(
            txn.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~')),
            "txnId precisa ser seguro em URL: {txn}"
        );
    }

    #[test]
    fn error_code_is_short_and_carries_no_message_or_address() {
        let cases = [
            (
                AssetError::Call(CallError::Response {
                    operation: "media (administração)",
                    status: 403,
                    errcode: Some("M_FORBIDDEN".to_owned()),
                    message: "nao posso apagar mxc://matrix-dev.local/abc".to_owned(),
                    retry_after_ms: None,
                }),
                "M_FORBIDDEN",
            ),
            (
                AssetError::Call(CallError::Response {
                    operation: "media (administração)",
                    status: 500,
                    errcode: None,
                    message: "erro interno".to_owned(),
                    retry_after_ms: None,
                }),
                "http_500",
            ),
            (
                AssetError::Call(CallError::Network {
                    operation: "media (administração)",
                    reason: "não consegui conectar".to_owned(),
                }),
                "rede",
            ),
            (
                AssetError::Address("sem servidor".to_owned()),
                "endereco_da_midia",
            ),
        ];
        for (error, expected) in cases {
            let code = error_code(&error);
            assert_eq!(code, expected);
            assert!(!code.contains("mxc://"), "código não leva endereço: {code}");
            assert!(code.len() <= 64);
        }

        let estranho = AssetError::Call(CallError::Response {
            operation: "media (administração)",
            status: 400,
            errcode: Some("M_ERRADO; DROP TABLE".to_owned()),
            message: String::new(),
            retry_after_ms: None,
        });
        assert_eq!(error_code(&estranho), "http_400");
    }
}
