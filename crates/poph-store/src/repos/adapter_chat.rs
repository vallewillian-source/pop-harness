use poph_types::{AgentId, ConversationId, MessageId, OrgId, RecordId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{Direction, Origin, Role};
use crate::repos::{belongs, column_id, column_u64, id_optional, require};

pub const RESERVED_CHANNELS: [&str; 5] = ["api", "matrix", "site", "test", "whatsapp"];

pub const LIMIT_EXTERNAL_REF: usize = 128;

pub const LIMIT_IDEMPOTENCY_KEY: usize = 128;

pub const LIMIT_AUTHOR_NAME: usize = 128;

pub const LIMIT_EXTERNAL_MESSAGE_ID: usize = 128;

#[derive(Clone, Debug, Serialize)]
pub struct AdapterConversation {
    pub id: ConversationId,
    pub org_id: OrgId,
    pub agent_id: AgentId,
    pub channel: String,
    pub external_ref: String,
    pub create_case: bool,
    pub status: String,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_message_at: Option<i64>,
    pub ficha_id: Option<RecordId>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AdapterMessage {
    pub id: MessageId,
    pub role: String,
    pub origin: String,
    pub author_name: Option<String>,
    pub text: String,
    pub external_message_id: Option<String>,
    pub created_at: i64,
}

pub struct NewAdapterMessage {
    pub text: String,
    pub direction: Direction,
    pub role: Role,
    pub origin: Origin,
    pub author_name: Option<String>,
    pub external_message_id: Option<String>,
    pub idempotency_key: String,
}

pub enum AdapterSubmission {
    Stored(AdapterMessage, bool),
    Conflict,
}

pub struct AdapterChat<'a>(pub(crate) &'a Store);

#[derive(Serialize)]
struct Fingerprint<'a> {
    text: &'a str,
    origin: &'a str,
    author_name: Option<&'a str>,
    external_message_id: Option<&'a str>,
}

const MESSAGE_COLUMNS: &str = "m.id, m.role, m.origin, m.author_name, m.text,
    k.external_message_id, m.created_at";

impl AdapterChat<'_> {
    pub fn create_or_resume(
        &self,
        org: OrgId,
        agent: AgentId,
        channel: String,
        reference: String,
        create_case: bool,
    ) -> Resultado<(ConversationId, bool)> {
        let id = ConversationId::generate();
        let now = self.0.now();
        self.0.write(move |conn| {
            let text_org = org.to_string();
            let text_agent = agent.to_string();
            belongs(conn, "agents", "agente", &text_org, &text_agent)?;
            let open: Option<String> = conn
                .query_row(
                    "SELECT c.id FROM adapter_conversations a
                       JOIN conversations c ON c.org_id = a.org_id AND c.id = a.id
                      WHERE a.org_id = ?1 AND c.agent_id = ?2 AND a.channel = ?3
                        AND a.external_ref = ?4 AND c.status <> 'closed'
                      ORDER BY c.created_at DESC, c.id DESC
                      LIMIT 1",
                    params![text_org, text_agent, channel, reference],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(existing) = open {
                let existing = existing.parse().map_err(|_| {
                    StoreError::Internal("identificador de conversa inválido".to_owned())
                })?;
                return Ok((existing, false));
            }
            let text_id = id.to_string();
            conn.execute(
                "INSERT INTO conversations
                    (id, org_id, agent_id, integration_id, channel, external_peer, status,
                     target_department_id, summary, version, created_at, updated_at,
                     last_message_at)
                 VALUES (?1, ?2, ?3, NULL, ?4, ?5, 'open', NULL, NULL, 1, ?6, ?6, NULL)",
                params![text_id, text_org, text_agent, channel, reference, now],
            )
            .map_err(StoreError::from)?;
            conn.execute(
                "INSERT INTO adapter_conversations
                    (org_id, id, channel, external_ref, create_case, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![text_org, text_id, channel, reference, create_case, now],
            )
            .map_err(StoreError::from)?;
            if create_case {
                super::cases::create_case_for_conversation(
                    conn,
                    &text_org,
                    &text_id,
                    &text_agent,
                    &channel,
                    Some(&reference),
                    now,
                )?;
            }
            Ok((id, true))
        })
    }

    pub fn get(&self, org: OrgId, id: ConversationId) -> Resultado<AdapterConversation> {
        self.0.read(|conn| load(conn, org, id))
    }

    pub fn write_message(
        &self,
        org: OrgId,
        conversation: ConversationId,
        fresh: NewAdapterMessage,
    ) -> Resultado<AdapterSubmission> {
        let now = self.0.now();
        let message = MessageId::generate();
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        self.0.write(move |conn| {
            let current: Option<String> = conn
                .query_row(
                    "SELECT c.status FROM adapter_conversations a
                       JOIN conversations c ON c.org_id = a.org_id AND c.id = a.id
                      WHERE a.org_id = ?1 AND a.id = ?2",
                    params![text_org, text_conversation],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(status) = current else {
                return require::<AdapterSubmission>(None, "conversa", &text_conversation);
            };
            if status == "closed" {
                return Err(poph_types::DomainError::ConversationClosed {
                    id: text_conversation.clone(),
                }
                .into());
            }
            let fingerprint = serde_json::to_string(&Fingerprint {
                text: &fresh.text,
                origin: fresh.origin.as_text(),
                author_name: fresh.author_name.as_deref(),
                external_message_id: fresh.external_message_id.as_deref(),
            })
            .map_err(|error| {
                StoreError::Internal(format!("impressão digital de mensagem inválida: {error}"))
            })?;
            let previous: Option<(String, String)> = conn
                .query_row(
                    "SELECT message_id, fingerprint FROM adapter_message_keys
                      WHERE org_id = ?1 AND conversation_id = ?2 AND idempotency_key = ?3",
                    params![text_org, text_conversation, fresh.idempotency_key],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if let Some((stored, original)) = previous {
                if original != fingerprint {
                    return Ok(AdapterSubmission::Conflict);
                }
                let stored = stored.parse().map_err(|_| {
                    StoreError::Internal("identificador de mensagem inválido".to_owned())
                })?;
                return Ok(AdapterSubmission::Stored(
                    load_message(conn, &text_org, &text_conversation, stored)?,
                    false,
                ));
            }
            conn.execute(
                "INSERT INTO messages
                    (id, org_id, conversation_id, direction, role, text, attachments_json,
                     external_id, turn_id, created_at, origin, author_name)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, '[]', NULL, NULL, ?7, ?8, ?9)",
                params![
                    message.to_string(),
                    text_org,
                    text_conversation,
                    fresh.direction.as_text(),
                    fresh.role.as_text(),
                    fresh.text,
                    now,
                    fresh.origin.as_text(),
                    fresh.author_name.as_deref()
                ],
            )
            .map_err(StoreError::from)?;
            conn.execute(
                "INSERT INTO adapter_message_keys
                    (org_id, conversation_id, idempotency_key, message_id, fingerprint,
                     external_message_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    text_org,
                    text_conversation,
                    fresh.idempotency_key,
                    message.to_string(),
                    fingerprint,
                    fresh.external_message_id.as_deref(),
                    now
                ],
            )
            .map_err(StoreError::from)?;
            conn.execute(
                "UPDATE conversations SET last_message_at = ?3, updated_at = ?3
                  WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_conversation, now],
            )
            .map_err(StoreError::from)?;
            Ok(AdapterSubmission::Stored(
                AdapterMessage {
                    id: message,
                    role: fresh.role.as_text().to_owned(),
                    origin: fresh.origin.as_text().to_owned(),
                    author_name: fresh.author_name,
                    text: fresh.text,
                    external_message_id: fresh.external_message_id,
                    created_at: now,
                },
                true,
            ))
        })
    }

    pub fn history(
        &self,
        org: OrgId,
        conversation: ConversationId,
        before: Option<MessageId>,
        limit: u32,
    ) -> Resultado<Vec<AdapterMessage>> {
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        let before = before.map(|id| id.to_string());
        let limit = i64::from(limit.clamp(1, 100));
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {MESSAGE_COLUMNS} FROM messages m
                   LEFT JOIN adapter_message_keys k
                     ON k.org_id = m.org_id AND k.message_id = m.id
                  WHERE m.org_id = ?1 AND m.conversation_id = ?2
                    AND m.role IN ('user', 'assistant')
                    AND (?3 IS NULL OR m.id < ?3)
                  ORDER BY m.id DESC LIMIT ?4"
            ))?;
            let rows = statement.query_map(
                params![text_org, text_conversation, before, limit],
                message_row,
            )?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }
}

fn load(conn: &Connection, org: OrgId, id: ConversationId) -> Resultado<AdapterConversation> {
    let found = conn
        .query_row(
            "SELECT c.id, c.org_id, c.agent_id, a.channel, a.external_ref, a.create_case,
                    c.status, c.version, c.created_at, c.updated_at, c.last_message_at, f.id
               FROM adapter_conversations a
               JOIN conversations c ON c.org_id = a.org_id AND c.id = a.id
               LEFT JOIN fichas f ON f.org_id = c.org_id AND f.conversation_id = c.id
              WHERE a.org_id = ?1 AND a.id = ?2",
            params![org.to_string(), id.to_string()],
            |row| {
                Ok(AdapterConversation {
                    id: column_id(row, 0)?,
                    org_id: column_id(row, 1)?,
                    agent_id: column_id(row, 2)?,
                    channel: row.get(3)?,
                    external_ref: row.get(4)?,
                    create_case: row.get(5)?,
                    status: row.get(6)?,
                    version: column_u64(row, 7)?,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    last_message_at: row.get(10)?,
                    ficha_id: id_optional(row, 11)?,
                })
            },
        )
        .optional()?;
    require(found, "conversa", &id.to_string())
}

fn load_message(
    conn: &Connection,
    org: &str,
    conversation: &str,
    id: MessageId,
) -> Resultado<AdapterMessage> {
    let found = conn
        .query_row(
            &format!(
                "SELECT {MESSAGE_COLUMNS} FROM messages m
                   LEFT JOIN adapter_message_keys k
                     ON k.org_id = m.org_id AND k.message_id = m.id
                  WHERE m.org_id = ?1 AND m.conversation_id = ?2 AND m.id = ?3"
            ),
            params![org, conversation, id.to_string()],
            message_row,
        )
        .optional()?;
    require(found, "mensagem", &id.to_string())
}

fn message_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AdapterMessage> {
    Ok(AdapterMessage {
        id: column_id(row, 0)?,
        role: row.get(1)?,
        origin: row.get(2)?,
        author_name: row.get(3)?,
        text: row.get(4)?,
        external_message_id: row.get(5)?,
        created_at: row.get(6)?,
    })
}
