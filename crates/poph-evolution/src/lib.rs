pub mod client;
pub mod error;
pub mod integration;
pub mod text;
pub mod webhook;

pub use client::{
    ATTACHMENT_CAP, ConfigEvolution, DEFAULT_BODY_CAP, DownloadedAttachment, EvolutionClient,
    Pairing,
};
pub use error::EvolutionError;
pub use integration::{AttachmentSource, ChannelConfig, DiskBlobs, WhatsAppEvolution};
pub use text::{TEXT_LIMIT, to_whatsapp, truncate};
pub use webhook::{
    AttachmentToDownload, Decision, IgnoreReason, Policy, ReceivedEntry, SECRET_HEADER,
    WebhookError, check_secret, embedded_attachment, interpret,
};
