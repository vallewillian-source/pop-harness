use poph_types::DomainError;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row};

use crate::error::StoreError;
use crate::model::Status;

mod adapter_chat;
mod agents;
mod analytics;
mod boxes;
mod cart;
mod cases;
mod categories;
mod conversations;
mod departments;
mod effects;
mod executions;
mod flows;
mod instructions;
mod knowledge;
mod library;
mod matrix;
mod matrix_asset;
mod messages;
mod orgs;
mod products;
mod retention;
mod revisions;
mod site_chat;
mod tokens;
mod turns;
mod usage;
pub use usage::{AgentUsage, DayUsage, TokenUsage, Usage};

pub use adapter_chat::{
    AdapterChat, AdapterConversation, AdapterEvent, AdapterMessage, AdapterSubmission, AdapterTurn,
    AdapterTurnClaim, AdapterTurnKey, AdapterTurnOutcome, AdapterTurnProgress,
    AdapterTurnSubmission, LIMIT_AUTHOR_NAME, LIMIT_EXTERNAL_MESSAGE_ID, LIMIT_EXTERNAL_REF,
    LIMIT_IDEMPOTENCY_KEY, NewAdapterMessage, NewAdapterTurn, RESERVED_CHANNELS,
};
pub use agents::{Agents, agents_by_provider_at};
pub use analytics::{Analytics, NodeCount, OperationalSummary};
pub use boxes::{Inbox, Outbox};
pub use cart::{CartItem, CartSummary, Carts, LIMIT_QUANTITY};
pub use cases::{Fichas, REASON_ABANDONMENT_SERVICE};
pub use categories::Categories;
pub use conversations::{
    Conversas, FilterConversations, LIMIT_NAME_CLIENT, OrderConversations, normalise_name_client,
};
pub use departments::Departments;
pub use effects::Effects;
pub use executions::{Executions, LIMIT_EXECUTIONS};
pub use flows::{CAP_FLOWS_BY_AGENT, Fluxos, REASON_EXCLUSION_FLOW};
pub use instructions::InstructionsRepo;
pub use knowledge::Knowledge;
pub use library::{LIMIT_DESCRIPTION_FILE, LIMIT_NAME_FILE, Library};
pub use matrix::{
    CursorConversations, CursorsMatrix, DelayMatrix, FailureMatrix, HumanResponseFailure,
    LIMIT_FAILURES, LIMIT_REASON_FAILURE, Matrix, OutcomeNotice, RoomMatrix, SpaceDisabled,
    SpaceMatrix, SpaceOrigin, StateMatrixOrg, StateRoom,
};
pub use matrix_asset::{
    CategoryAsset, LIMIT_ERROR_ASSET, LIMIT_PENDING, MatrixAsset, NewAsset, RoomAsset,
};
pub use messages::{LIMIT_SINCE, MessageSince, Messages};
pub use orgs::Orgs;
pub use products::{ProductReferences, Products};
pub use retention::{Retencao, ScanRetention};
pub use revisions::{ACTION_PUBLISH, ACTORS_OF_AUDIT, LIMIT_REVISIONS, Revisions};
pub use site_chat::{
    SiteChat, SiteEvent, SiteMessage, SiteOutcome, SiteReply, SiteSubmission, SiteTurn, SiteTurnKey,
};
pub use tokens::Tokens;
pub use turns::{Audit, Turns};

pub(crate) use crate::readers::require;

pub(crate) fn column_id<T>(row: &Row<'_>, index: usize) -> rusqlite::Result<T>
where
    T: std::str::FromStr<Err = DomainError>,
{
    let texto: String = row.get(index)?;
    texto.parse().map_err(|error: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
    })
}

pub(crate) fn id_optional<T>(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<T>>
where
    T: std::str::FromStr<Err = DomainError>,
{
    let texto: Option<String> = row.get(index)?;
    match texto {
        Some(texto) => texto.parse().map(Some).map_err(|error: DomainError| {
            rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
        }),
        None => Ok(None),
    }
}

pub(crate) fn column_status(row: &Row<'_>, index: usize) -> rusqlite::Result<Status> {
    let texto: String = row.get(index)?;
    Status::from_text(&texto).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
    })
}

pub(crate) fn column_u64(row: &Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    Ok(value.max(0) as u64)
}

pub(crate) fn belongs(
    conn: &Connection,
    table: &'static str,
    entity: &'static str,
    org_id: &str,
    id: &str,
) -> crate::error::Resultado<()> {
    let exists: Option<i64> = conn
        .query_row(
            &format!("SELECT 1 FROM {table} WHERE org_id = ?1 AND id = ?2"),
            rusqlite::params![org_id, id],
            |row| row.get(0),
        )
        .optional()
        .map_err(StoreError::from)?;
    match exists {
        Some(_) => Ok(()),
        None => Err(StoreError::Domain(DomainError::NotFound {
            entity,
            id: id.to_owned(),
        })),
    }
}

pub(crate) fn column_u32(row: &Row<'_>, index: usize) -> rusqlite::Result<u32> {
    let value: i64 = row.get(index)?;
    Ok(value.clamp(0, i64::from(u32::MAX)) as u32)
}

pub(crate) fn conflict_or_missing(
    conn: &Connection,
    table: &'static str,
    entity: &'static str,
    id: &str,
    org_id: &str,
    expected: u64,
) -> StoreError {
    let current: Result<Option<i64>, StoreError> = conn
        .query_row(
            &format!("SELECT version FROM {table} WHERE id = ?1 AND org_id = ?2"),
            rusqlite::params![id, org_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(StoreError::from);
    match current {
        Ok(Some(current)) => StoreError::Domain(DomainError::VersionConflict {
            entity,
            id: id.to_owned(),
            expected,
            current: current.max(0) as u64,
        }),
        Ok(None) | Err(_) => StoreError::Domain(DomainError::NotFound {
            entity,
            id: id.to_owned(),
        }),
    }
}

mod conversation_analytics;
pub use conversation_analytics::{
    ConversationAgent, ConversationAnalytics, ConversationChannel, ConversationCounts,
    ConversationDay, ConversationReport, ConversationSummary,
};

mod sla_analytics;
pub use sla_analytics::{SlaAnalytics, SlaItem, SlaReport};
