use serde_json::{Value, json};

use crate::client::MatrixClient;
use crate::error::CallError;
use crate::events::MSGTYPE_NOTICE;
use crate::format::EVENT_KIND;
use crate::users::Ghost;

pub const ASSET_BYTES_LIMIT: u64 = 16 * 1024 * 1024;

pub const ATTACHMENT_BLOCK: &str = "com.sofacoding.attachment";

pub const ID_FIELD: &str = "id";

pub const SOURCE_FIELD: &str = "source";

pub const MSGTYPE_IMAGE: &str = "m.image";

pub const MSGTYPE_FILE: &str = "m.file";

pub const ASSET_TXN_PREFIX: &str = "poph-midia-";

pub const NOTICE_TXN_PREFIX: &str = "poph-midia-aviso-";

pub const PREFIX_NOTICE: &str = "Não consegui mostrar na sala o arquivo que o cliente enviou: ";

#[derive(Debug, Clone, Copy)]
pub struct AttachmentForAsset<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub mime: &'a str,
    pub size_bytes: u64,
    pub source: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalReason {
    Large,
    Empty,
    Unreadable,
    Rejeitada,
}

impl RefusalReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::Large => "arquivo_grande",
            Self::Empty => "arquivo_vazio",
            Self::Unreadable => "arquivo_ilegivel",
            Self::Rejeitada => "midia_rejeitada",
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Self::Large => "o arquivo passa de 16 MB",
            Self::Empty => "o arquivo está vazio",
            Self::Unreadable => "o arquivo não pôde ser lido",
            Self::Rejeitada => "o serviço de atendimento recusou o arquivo",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub id_attachment: String,
    pub msgtype: &'static str,
    pub name: String,
    pub mime: String,
    pub size_bytes: u64,
    pub source: String,
    pub txn_id: String,
}

pub fn decide(attachment: &AttachmentForAsset<'_>) -> Result<Asset, RefusalReason> {
    if attachment.size_bytes == 0 {
        return Err(RefusalReason::Empty);
    }
    if attachment.size_bytes > ASSET_BYTES_LIMIT {
        return Err(RefusalReason::Large);
    }
    let image = attachment
        .mime
        .trim()
        .to_ascii_lowercase()
        .starts_with("image/");
    let msgtype = if image { MSGTYPE_IMAGE } else { MSGTYPE_FILE };
    let name = visible_name(attachment.name, image);
    Ok(Asset {
        id_attachment: attachment.id.to_owned(),
        msgtype,
        name,
        mime: attachment.mime.trim().to_owned(),
        size_bytes: attachment.size_bytes,
        source: attachment.source.trim().to_owned(),
        txn_id: asset_txn(attachment.id),
    })
}

fn visible_name(name: &str, image: bool) -> String {
    let clean = name.trim();
    if !clean.is_empty() {
        return clean.to_owned();
    }
    if image {
        "imagem".to_owned()
    } else {
        "arquivo".to_owned()
    }
    .to_owned()
}

pub fn asset_txn(id_attachment: &str) -> String {
    format!("{ASSET_TXN_PREFIX}{id_attachment}")
}

pub fn notice_txn(id_attachment: &str, reason: RefusalReason) -> String {
    format!("{NOTICE_TXN_PREFIX}{id_attachment}-{}", reason.code())
}

pub fn notice_body(reason: RefusalReason) -> String {
    format!("{PREFIX_NOTICE}{}", reason.text())
}

pub fn event_content(mxc: &str, asset: &Asset) -> Value {
    json!({
        "msgtype": asset.msgtype,
        "body": asset.name,
        "filename": asset.name,
        "url": mxc,
        "info": { "mimetype": asset.mime, "size": asset.size_bytes },
        ATTACHMENT_BLOCK: { ID_FIELD: asset.id_attachment, SOURCE_FIELD: asset.source },
    })
}

pub fn notice_content(reason: RefusalReason) -> Value {
    json!({ "msgtype": MSGTYPE_NOTICE, "body": notice_body(reason) })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentAsset {
    pub event: String,
    pub mxc: String,
}

pub async fn send(
    client: &MatrixClient,
    sender: &Ghost,
    room: &str,
    asset: &Asset,
    content: &[u8],
) -> Result<SentAsset, CallError> {
    let mxc = client
        .send_asset(sender.user_id(), &asset.name, &asset.mime, content)
        .await?;
    let event = client
        .send_event(
            sender.user_id(),
            room,
            EVENT_KIND,
            &asset.txn_id,
            &event_content(&mxc, asset),
        )
        .await?;
    Ok(SentAsset { event, mxc })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment<'a>(
        id: &'a str,
        name: &'a str,
        mime: &'a str,
        size: u64,
    ) -> AttachmentForAsset<'a> {
        AttachmentForAsset {
            id,
            name,
            mime,
            size_bytes: size,
            source: "whatsapp",
        }
    }

    #[test]
    fn image_becomes_m_image_and_rest_becomes_m_file() {
        for (mime, expected) in [
            ("image/jpeg", MSGTYPE_IMAGE),
            ("IMAGE/PNG", MSGTYPE_IMAGE),
            ("application/pdf", MSGTYPE_FILE),
            (
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                MSGTYPE_FILE,
            ),
            ("application/octet-stream", MSGTYPE_FILE),
        ] {
            let asset = decide(&attachment("a1", "arquivo.bin", mime, 10)).expect("decide");
            assert_eq!(asset.msgtype, expected, "mime {mime}");
        }
    }

    #[test]
    fn sixteen_megabyte_limit_is_of_attachment_not_event() {
        let at_limit = decide(&attachment(
            "a1",
            "foto.jpg",
            "image/jpeg",
            ASSET_BYTES_LIMIT,
        ))
        .expect("no limite entra");
        assert_eq!(at_limit.size_bytes, ASSET_BYTES_LIMIT);

        let above = decide(&attachment(
            "a2",
            "foto.jpg",
            "image/jpeg",
            ASSET_BYTES_LIMIT + 1,
        ));
        assert_eq!(above, Err(RefusalReason::Large));

        assert_eq!(
            decide(&attachment("a3", "vazio.pdf", "application/pdf", 0)),
            Err(RefusalReason::Empty)
        );
    }

    #[test]
    fn empty_name_gets_kind_default() {
        let image = decide(&attachment("a1", "  ", "image/png", 5)).expect("decide");
        assert_eq!(image.name, "imagem");
        let file = decide(&attachment("a2", "", "application/pdf", 5)).expect("decide");
        assert_eq!(file.name, "arquivo");
        let with_space =
            decide(&attachment("a3", "  laudo.pdf ", "application/pdf", 5)).expect("decide");
        assert_eq!(with_space.name, "laudo.pdf");
    }

    #[test]
    fn txn_is_stable_and_differs_per_attachment_and_reason() {
        assert_eq!(asset_txn("anx-1"), asset_txn("anx-1"));
        assert_ne!(asset_txn("anx-1"), asset_txn("anx-2"));
        assert_ne!(
            notice_txn("anx-1", RefusalReason::Large),
            notice_txn("anx-1", RefusalReason::Unreadable)
        );
        for txn in [
            asset_txn("018f-0000-7000-8000-000000000001"),
            notice_txn("018f-0000-7000-8000-000000000001", RefusalReason::Rejeitada),
        ] {
            assert!(
                txn.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~')),
                "txnId precisa ser seguro em URL: {txn}"
            );
        }
    }

    #[test]
    fn event_carries_frontend_block_with_id_and_source() {
        let asset =
            decide(&attachment("anx-9", "laudo.pdf", "application/pdf", 2_048)).expect("decide");
        let event = event_content("mxc://matrix-dev.local/abc123", &asset);

        assert_eq!(event["msgtype"], json!("m.file"));
        assert_eq!(event["body"], json!("laudo.pdf"));
        assert_eq!(event["filename"], json!("laudo.pdf"));
        assert_eq!(event["url"], json!("mxc://matrix-dev.local/abc123"));
        assert_eq!(event["info"]["mimetype"], json!("application/pdf"));
        assert_eq!(event["info"]["size"], json!(2_048));
        assert_eq!(
            event[ATTACHMENT_BLOCK],
            json!({ "id": "anx-9", "source": "whatsapp" })
        );
    }

    #[test]
    fn image_also_carries_block() {
        let asset = decide(&attachment("anx-1", "foto.jpg", "image/jpeg", 10)).expect("decide");
        let event = event_content("mxc://s/x", &asset);
        assert_eq!(event["msgtype"], json!("m.image"));
        assert_eq!(event[ATTACHMENT_BLOCK]["id"], json!("anx-1"));
    }

    #[test]
    fn notice_gives_reason_without_file_name() {
        for reason in [
            RefusalReason::Large,
            RefusalReason::Empty,
            RefusalReason::Unreadable,
            RefusalReason::Rejeitada,
        ] {
            let notice = notice_content(reason);
            assert_eq!(notice["msgtype"], json!("m.notice"));
            let body = notice["body"].as_str().expect("corpo é texto");
            assert!(body.starts_with(PREFIX_NOTICE), "corpo: {body}");
            assert!(body.ends_with(reason.text()), "corpo: {body}");
            assert!(!body.contains("laudo"), "o aviso não leva nome de arquivo");
        }
    }
}
