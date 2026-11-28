use poph_types::{AgentId, ConversationId, MessageId, OrgId, RecordId, TurnId};
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

pub struct NewAdapterTurn {
    pub idempotency_key: String,
    pub text: String,
    pub author_name: Option<String>,
    pub external_message_id: Option<String>,
}

pub enum AdapterTurnSubmission {
    Accepted {
        message: MessageId,
        turn: TurnId,
    },
    Replayed {
        message: MessageId,
        turn: TurnId,
        status: String,
    },
    Conflict,
    Busy(TurnId),
}

#[derive(Clone, Copy, Debug)]
pub struct AdapterTurnKey {
    pub org: OrgId,
    pub conversation: ConversationId,
    pub turn: TurnId,
}

pub struct AdapterTurnClaim {
    pub agent: AgentId,
    pub message: MessageId,
}

pub enum AdapterTurnOutcome {
    Success {
        engine_turn: TurnId,
        input_tokens: u32,
        output_tokens: u32,
    },
    Failed(&'static str),
    Cancelled,
}

#[derive(Clone, Debug, Serialize)]
pub struct AdapterTurn {
    pub id: TurnId,
    pub conversation_id: ConversationId,
    pub status: String,
    pub reply_text: Option<String>,
    pub error_code: Option<String>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct AdapterEvent {
    pub seq: i64,
    #[serde(rename = "type")]
    pub kind: String,
    pub turn_id: TurnId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

pub enum AdapterTurnProgress {
    TextDelta(String),
    ToolCall(String),
    ToolResult {
        name: String,
        refusal: Option<&'static str>,
    },
}

enum Event<'a> {
    Start,
    Progress(AdapterTurnProgress),
    End,
    Failed(&'a str),
    Cancelled,
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
            let fingerprint = message_fingerprint(
                &fresh.text,
                fresh.origin.as_text(),
                fresh.author_name.as_deref(),
                fresh.external_message_id.as_deref(),
            )?;
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

fn message_fingerprint(
    text: &str,
    origin: &str,
    author_name: Option<&str>,
    external_message_id: Option<&str>,
) -> Resultado<String> {
    serde_json::to_string(&Fingerprint {
        text,
        origin,
        author_name,
        external_message_id,
    })
    .map_err(|error| StoreError::Internal(format!("invalid message fingerprint: {error}")))
}

const TURN_COLUMNS: &str = "t.id, t.conversation_id, t.status, m.text, t.error_code,
    t.input_tokens, t.output_tokens, t.created_at, t.started_at, t.finished_at";

impl AdapterChat<'_> {
    pub fn submit_turn(
        &self,
        org: OrgId,
        conversation: ConversationId,
        fresh: NewAdapterTurn,
    ) -> Resultado<AdapterTurnSubmission> {
        let turn = TurnId::generate();
        let message = MessageId::generate();
        let now = self.0.now();
        self.0.write(move |conn| {
            let text_org = org.to_string();
            let text_conversation = conversation.to_string();
            let found: Option<(String, String)> = conn
                .query_row(
                    "SELECT a.channel, c.status FROM adapter_conversations a
                       JOIN conversations c ON c.org_id = a.org_id AND c.id = a.id
                      WHERE a.org_id = ?1 AND a.id = ?2",
                    params![text_org, text_conversation],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((_channel, status)) = found else {
                return require::<AdapterTurnSubmission>(None, "conversa", &text_conversation);
            };
            if status != "open" {
                return Err(poph_types::DomainError::ConversationClosed {
                    id: text_conversation.clone(),
                }
                .into());
            }
            let fingerprint = message_fingerprint(
                &fresh.text,
                Origin::Agente.as_text(),
                fresh.author_name.as_deref(),
                fresh.external_message_id.as_deref(),
            )?;
            let previous: Option<(String, String, String, String)> = conn
                .query_row(
                    "SELECT id, message_id, fingerprint, status FROM adapter_turns
                      WHERE org_id = ?1 AND conversation_id = ?2 AND idempotency_key = ?3",
                    params![text_org, text_conversation, fresh.idempotency_key],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?;
            if let Some((stored_turn, stored_message, original, status)) = previous {
                if original != fingerprint {
                    return Ok(AdapterTurnSubmission::Conflict);
                }
                let stored_turn = stored_turn
                    .parse()
                    .map_err(|_| StoreError::Internal("invalid turn identifier".to_owned()))?;
                let stored_message = stored_message
                    .parse()
                    .map_err(|_| StoreError::Internal("invalid message identifier".to_owned()))?;
                return Ok(AdapterTurnSubmission::Replayed {
                    message: stored_message,
                    turn: stored_turn,
                    status,
                });
            }
            let active: Option<String> = conn
                .query_row(
                    "SELECT id FROM adapter_turns
                      WHERE org_id = ?1 AND conversation_id = ?2
                        AND status IN ('queued', 'running')
                      LIMIT 1",
                    params![text_org, text_conversation],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(active) = active {
                let active = active
                    .parse()
                    .map_err(|_| StoreError::Internal("invalid turn identifier".to_owned()))?;
                return Ok(AdapterTurnSubmission::Busy(active));
            }
            let taken: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM adapter_message_keys
                      WHERE org_id = ?1 AND conversation_id = ?2 AND idempotency_key = ?3",
                    params![text_org, text_conversation, fresh.idempotency_key],
                    |row| row.get(0),
                )
                .optional()?;
            if taken.is_some() {
                return Ok(AdapterTurnSubmission::Conflict);
            }
            conn.execute(
                "INSERT INTO messages
                    (id, org_id, conversation_id, direction, role, text, attachments_json,
                     external_id, turn_id, created_at, origin, author_name)
                 VALUES (?1, ?2, ?3, 'in', 'user', ?4, '[]', NULL, NULL, ?5, ?6, ?7)",
                params![
                    message.to_string(),
                    text_org,
                    text_conversation,
                    fresh.text,
                    now,
                    Origin::Agente.as_text(),
                    fresh.author_name.as_deref()
                ],
            )
            .map_err(StoreError::from)?;
            conn.execute(
                "INSERT INTO adapter_turns
                    (org_id, conversation_id, id, message_id, idempotency_key, fingerprint,
                     status, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'queued', ?7)",
                params![
                    text_org,
                    text_conversation,
                    turn.to_string(),
                    message.to_string(),
                    fresh.idempotency_key,
                    fingerprint,
                    now
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
            Ok(AdapterTurnSubmission::Accepted { message, turn })
        })
    }

    pub fn claim_turn(&self, key: AdapterTurnKey) -> Resultado<Option<AdapterTurnClaim>> {
        let now = self.0.now();
        self.0.write(move |conn| {
            let ids = (
                key.org.to_string(),
                key.conversation.to_string(),
                key.turn.to_string(),
            );
            let message: Option<MessageId> = conn
                .query_row(
                    "UPDATE adapter_turns SET status = 'running', started_at = ?4
                      WHERE org_id = ?1 AND conversation_id = ?2 AND id = ?3
                        AND status = 'queued'
                      RETURNING message_id",
                    params![ids.0, ids.1, ids.2, now],
                    |row| column_id(row, 0),
                )
                .optional()?;
            let Some(message) = message else {
                return Ok(None);
            };
            let agent: AgentId = conn.query_row(
                "SELECT agent_id FROM conversations WHERE org_id = ?1 AND id = ?2",
                params![ids.0, ids.1],
                |row| column_id(row, 0),
            )?;
            append_event(conn, key, Event::Start)?;
            Ok(Some(AdapterTurnClaim { agent, message }))
        })
    }

    pub fn record_progress(
        &self,
        key: AdapterTurnKey,
        progress: AdapterTurnProgress,
    ) -> Resultado<()> {
        self.0.write(move |conn| {
            let running = conn.query_row(
                "SELECT EXISTS (
                    SELECT 1 FROM adapter_turns
                    WHERE org_id = ?1 AND conversation_id = ?2 AND id = ?3 AND status = 'running'
                )",
                params![
                    key.org.to_string(),
                    key.conversation.to_string(),
                    key.turn.to_string()
                ],
                |row| row.get::<_, bool>(0),
            )?;
            if running {
                append_event(conn, key, Event::Progress(progress))?;
            }
            Ok(())
        })
    }

    pub fn events(
        &self,
        org: OrgId,
        conversation: ConversationId,
        after: i64,
        limit: u32,
    ) -> Resultado<Vec<AdapterEvent>> {
        self.0.read(|conn| {
            let mut statement = conn.prepare(
                "SELECT seq, kind, turn_id, text, tool_name, result, error_code FROM adapter_events
                  WHERE org_id = ?1 AND conversation_id = ?2 AND seq > ?3 ORDER BY seq LIMIT ?4",
            )?;
            let rows = statement.query_map(
                params![org.to_string(), conversation.to_string(), after, limit],
                |row| {
                    Ok(AdapterEvent {
                        seq: row.get(0)?,
                        kind: row.get(1)?,
                        turn_id: column_id(row, 2)?,
                        text: row.get(3)?,
                        tool_name: row.get(4)?,
                        result: row.get(5)?,
                        error_code: row.get(6)?,
                    })
                },
            )?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn finish_turn(
        &self,
        key: AdapterTurnKey,
        outcome: AdapterTurnOutcome,
    ) -> Resultado<AdapterTurn> {
        let now = self.0.now();
        self.0
            .write(move |conn| finish_turn(conn, key, outcome, now))
    }

    pub fn record_engine_turn(&self, key: AdapterTurnKey, engine: TurnId) -> Resultado<()> {
        self.0.write(move |conn| {
            conn.execute(
                "UPDATE adapter_turns SET engine_turn_id = ?4
                  WHERE org_id = ?1 AND conversation_id = ?2 AND id = ?3 AND status = 'running'",
                params![
                    key.org.to_string(),
                    key.conversation.to_string(),
                    key.turn.to_string(),
                    engine.to_string()
                ],
            )?;
            Ok(())
        })
    }

    pub fn recover_turns(&self) -> Resultado<Vec<AdapterTurnKey>> {
        let now = self.0.now();
        self.0.write(move |conn| {
            let interrupted = scan_turns(conn, "running")?;
            for key in interrupted {
                recover_turn(conn, key, now)?;
            }
            scan_turns(conn, "queued")
        })
    }

    pub fn turn(&self, key: AdapterTurnKey) -> Resultado<AdapterTurn> {
        self.0.read(|conn| load_turn(conn, key))
    }

    pub fn is_adapter(&self, org: OrgId, conversation: ConversationId) -> Resultado<bool> {
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        self.0.read(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT 1 FROM adapter_conversations WHERE org_id = ?1 AND id = ?2",
                    params![text_org, text_conversation],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .is_some())
        })
    }
}

fn recover_turn(conn: &Connection, key: AdapterTurnKey, now: i64) -> Resultado<()> {
    let ids = (
        key.org.to_string(),
        key.conversation.to_string(),
        key.turn.to_string(),
    );
    let engine: Option<Option<String>> = conn
        .query_row(
            "SELECT engine_turn_id FROM adapter_turns
              WHERE org_id = ?1 AND conversation_id = ?2 AND id = ?3",
            params![ids.0, ids.1, ids.2],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(Some(engine)) = engine {
        let reply: Option<String> = conn
            .query_row(
                "SELECT id FROM messages
                  WHERE org_id = ?1 AND conversation_id = ?2 AND turn_id = ?3
                    AND role = 'assistant'
                  ORDER BY created_at DESC, id DESC LIMIT 1",
                params![ids.0, ids.1, engine],
                |row| row.get(0),
            )
            .optional()?;
        if reply.is_some() {
            conn.execute(
                "UPDATE adapter_turns
                    SET status = 'succeeded', reply_message_id = ?4, error_code = NULL,
                        input_tokens = COALESCE((SELECT input_tokens FROM turns
                          WHERE org_id = ?1 AND conversation_id = ?2 AND id = ?5), 0),
                        output_tokens = COALESCE((SELECT output_tokens FROM turns
                          WHERE org_id = ?1 AND conversation_id = ?2 AND id = ?5), 0),
                        finished_at = ?6
                  WHERE org_id = ?1 AND conversation_id = ?2 AND id = ?3",
                params![ids.0, ids.1, ids.2, reply, engine, now],
            )?;
            return append_event(conn, key, Event::End);
        }
    }
    finish_turn(
        conn,
        key,
        AdapterTurnOutcome::Failed("server_restarted"),
        now,
    )?;
    Ok(())
}

fn scan_turns(conn: &Connection, status: &str) -> Resultado<Vec<AdapterTurnKey>> {
    let mut statement = conn.prepare(
        "SELECT org_id, conversation_id, id FROM adapter_turns WHERE status = ?1
          ORDER BY created_at, id",
    )?;
    let rows = statement.query_map([status], |row| {
        Ok(AdapterTurnKey {
            org: column_id(row, 0)?,
            conversation: column_id(row, 1)?,
            turn: column_id(row, 2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn load_turn(conn: &Connection, key: AdapterTurnKey) -> Resultado<AdapterTurn> {
    let found = conn
        .query_row(
            &format!(
                "SELECT {TURN_COLUMNS} FROM adapter_turns t
                   LEFT JOIN messages m ON m.org_id = t.org_id AND m.id = t.reply_message_id
                  WHERE t.org_id = ?1 AND t.conversation_id = ?2 AND t.id = ?3"
            ),
            params![
                key.org.to_string(),
                key.conversation.to_string(),
                key.turn.to_string()
            ],
            |row| {
                Ok(AdapterTurn {
                    id: column_id(row, 0)?,
                    conversation_id: column_id(row, 1)?,
                    status: row.get(2)?,
                    reply_text: row.get(3)?,
                    error_code: row.get(4)?,
                    input_tokens: u32::try_from(column_u64(row, 5)?).unwrap_or(u32::MAX),
                    output_tokens: u32::try_from(column_u64(row, 6)?).unwrap_or(u32::MAX),
                    created_at: row.get(7)?,
                    started_at: row.get(8)?,
                    finished_at: row.get(9)?,
                })
            },
        )
        .optional()?;
    require(found, "turno", &key.turn.to_string())
}

fn finish_turn(
    conn: &Connection,
    key: AdapterTurnKey,
    outcome: AdapterTurnOutcome,
    now: i64,
) -> Resultado<AdapterTurn> {
    let current = load_turn(conn, key)?;
    if !matches!(current.status.as_str(), "queued" | "running") {
        return Ok(current);
    }
    let ids = (
        key.org.to_string(),
        key.conversation.to_string(),
        key.turn.to_string(),
    );
    let stored_reply = |engine_turn: TurnId| -> Resultado<Option<String>> {
        Ok(conn
            .query_row(
                "SELECT id FROM messages
                  WHERE org_id = ?1 AND conversation_id = ?2 AND turn_id = ?3
                    AND role = 'assistant'
                  ORDER BY created_at DESC, id DESC LIMIT 1",
                params![ids.0, ids.1, engine_turn.to_string()],
                |row| row.get(0),
            )
            .optional()?)
    };
    let event = match outcome {
        AdapterTurnOutcome::Success {
            engine_turn,
            input_tokens,
            output_tokens,
        } => match stored_reply(engine_turn)? {
            Some(reply) => {
                conn.execute(
                    "UPDATE adapter_turns
                        SET status = 'succeeded', engine_turn_id = ?4, reply_message_id = ?5,
                            error_code = NULL, input_tokens = ?6, output_tokens = ?7,
                            finished_at = ?8
                      WHERE org_id = ?1 AND conversation_id = ?2 AND id = ?3",
                    params![
                        ids.0,
                        ids.1,
                        ids.2,
                        engine_turn.to_string(),
                        reply,
                        i64::from(input_tokens),
                        i64::from(output_tokens),
                        now
                    ],
                )?;
                Event::End
            }
            None => {
                conn.execute(
                    "UPDATE adapter_turns
                        SET status = 'failed', engine_turn_id = ?4, error_code = 'turn_failed',
                            finished_at = ?5
                      WHERE org_id = ?1 AND conversation_id = ?2 AND id = ?3",
                    params![ids.0, ids.1, ids.2, engine_turn.to_string(), now],
                )?;
                Event::Failed("turn_failed")
            }
        },
        AdapterTurnOutcome::Failed(code) => {
            conn.execute(
                "UPDATE adapter_turns
                    SET status = 'failed', error_code = ?4, finished_at = ?5
                  WHERE org_id = ?1 AND conversation_id = ?2 AND id = ?3",
                params![ids.0, ids.1, ids.2, code, now],
            )?;
            Event::Failed(code)
        }
        AdapterTurnOutcome::Cancelled => {
            conn.execute(
                "UPDATE adapter_turns
                    SET status = 'cancelled', error_code = NULL, finished_at = ?4
                  WHERE org_id = ?1 AND conversation_id = ?2 AND id = ?3",
                params![ids.0, ids.1, ids.2, now],
            )?;
            Event::Cancelled
        }
    };
    append_event(conn, key, event)?;
    load_turn(conn, key)
}

fn append_event(conn: &Connection, key: AdapterTurnKey, event: Event<'_>) -> Resultado<()> {
    let org = key.org.to_string();
    let conversation = key.conversation.to_string();
    let seq: i64 = conn.query_row(
        "UPDATE adapter_conversations SET last_event_seq = last_event_seq + 1
          WHERE org_id = ?1 AND id = ?2 RETURNING last_event_seq",
        params![org, conversation],
        |row| row.get(0),
    )?;
    let (kind, text, tool, result, error) = match &event {
        Event::Start => ("turn_start", None, None, None, None),
        Event::Progress(AdapterTurnProgress::TextDelta(text)) => {
            ("text_delta", Some(text.as_str()), None, None, None)
        }
        Event::Progress(AdapterTurnProgress::ToolCall(name)) => {
            ("tool_call", None, Some(name.as_str()), None, None)
        }
        Event::Progress(AdapterTurnProgress::ToolResult { name, refusal }) => {
            let result = if refusal.is_some() {
                "refused"
            } else {
                "executed"
            };
            (
                "tool_result",
                None,
                Some(name.as_str()),
                Some(result),
                *refusal,
            )
        }
        Event::End => ("turn_end", None, None, None, None),
        Event::Failed(code) => ("turn_failed", None, None, None, Some(*code)),
        Event::Cancelled => ("turn_cancelled", None, None, None, None),
    };
    conn.execute(
        "INSERT INTO adapter_events (org_id, conversation_id, seq, turn_id, kind, text, tool_name, result, error_code)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![org, conversation, seq, key.turn.to_string(), kind, text, tool, result, error],
    )?;
    Ok(())
}
