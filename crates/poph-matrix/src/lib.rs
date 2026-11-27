pub mod asset;
pub mod attendant_asset;
pub mod client;
pub mod config;
pub mod error;
pub mod events;
pub mod expiration;
pub mod format;
pub mod rooms;
pub mod spaces;
pub mod users;

pub use asset::{
    ASSET_BYTES_LIMIT, ATTACHMENT_BLOCK, Asset, AttachmentForAsset, MSGTYPE_FILE, MSGTYPE_IMAGE,
    PREFIX_NOTICE, RefusalReason, decide as decide_asset,
};
pub use attendant_asset::NotSendReason;
pub use client::{ClientConfig, CreateRoom, DEFAULT_TIMEOUT, MatrixClient, Profile, RoomMember};
pub use config::{
    MatrixError, MatrixModule, MatrixResult, MatrixSecrets, VAR_ADMIN_TOKEN, VAR_AS_TOKEN,
    VAR_HS_TOKEN,
};
pub use error::{AssetError, CallError, Classification};
pub use events::{
    Action, AssetKind, IgnoreReason, MSGTYPE_EMOTE, MSGTYPE_NOTICE, NEW_CONTENT_FIELD,
    REACTION_KIND, RELACAO_ANOTACAO, RELACAO_SUBSTITUICAO, REMOVAL_KIND, classify,
};
pub use rooms::{
    ConversationToRoom, METADATA_KIND, MIN_AGENT_POWER, NAME_KIND, NOTICE_WITHOUT_HISTORY,
    PortError, Provisioner, ROOM_KIND, RoomError, RoomMetadata, RoomReady, RoomStored, RoomsPort,
    StoredState,
};
pub use spaces::{
    CHILD_KIND, DEPARTMENT_KIND, DepartmentSpace, ENTRY_RULES_KIND, InviteLimits, PARENT_KIND,
    Position, RoomForSpace, SPACE_KIND, SpaceError, SpaceOrigin, Spaces, SpacesPort, StoredSpace,
    department_alias, localpart_do_alias, resolve_department,
};
pub use users::{
    CLIENT_NAME_SUFFIX, DEFAULT_CACHE_CAP, Ghost, GhostIds, GhostsError, LOCALPART_BODY_LIMIT,
    MARK_AGENT, MARK_CLIENT, MARK_DEVICE, SUFFIX_BOT, VirtualUsers,
};
