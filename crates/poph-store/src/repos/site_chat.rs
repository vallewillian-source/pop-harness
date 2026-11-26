use poph_types::{AgentId, ConversationId, MessageId, OrgId, TurnId};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::repos::{belongs, column_id, id_optional, require};

pub struct SiteChat<'a>(pub(crate) &'a Store);

#[derive(Clone, Copy)]
pub struct SiteTurnKey {
    pub org: OrgId,
    pub conversation: ConversationId,
    pub turn: TurnId,
}

pub enum SiteSubmission {
    Accepted(MessageId, TurnId),
    Replayed(MessageId, TurnId),
    Conflict,
    Busy(TurnId),
}

#[derive(Clone, Debug, Serialize)]
pub struct SiteTurn {
    pub status: String,
    pub error_code: Option<String>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SiteEvent {
    pub seq: i64,
    #[serde(rename = "type")]
    pub kind: String,
    pub turn_id: TurnId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<MessageId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SiteMessage {
    pub id: MessageId,
    pub role: String,
    pub text: String,
    pub created_at: i64,
}

pub struct SiteReply {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

pub enum SiteOutcome {
    Success(SiteReply),
    Failed(&'static str),
    Cancelled,
}

impl SiteChat<'_> {
    pub fn create(
        &self,
        org: OrgId,
        agent: AgentId,
        reference: String,
    ) -> Resultado<(ConversationId, bool)> {
        let id = ConversationId::generate();
        let now = self.0.now();
        self.0.write(move |conn| {
            let org = org.to_string();
            let agent = agent.to_string();
            belongs(conn, "agents", "agente", &org, &agent)?;
            let created = conn.execute(
                "INSERT INTO conversations (id, org_id, agent_id, channel, external_peer, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'site', ?4, ?5, ?5)
                 ON CONFLICT (org_id, agent_id, external_peer) WHERE channel = 'site' DO NOTHING",
                params![id.to_string(), org, agent, reference, now],
            )? == 1;
            let actual = if created {
                id
            } else {
                conn.query_row(
                    "SELECT id FROM conversations WHERE org_id = ?1 AND agent_id = ?2
                     AND channel = 'site' AND external_peer = ?3",
                    params![org, agent, reference],
                    |row| column_id(row, 0),
                )?
            };
            conn.execute(
                "INSERT INTO site_conversation_cursors (org_id, conversation_id) VALUES (?1, ?2)
                 ON CONFLICT DO NOTHING",
                params![org, actual.to_string()],
            )?;
            Ok((actual, created))
        })
    }

    pub fn submit(
        &self,
        org: OrgId,
        conversation: ConversationId,
        key: String,
        text: String,
    ) -> Resultado<SiteSubmission> {
        let turn = TurnId::generate();
        let message = MessageId::generate();
        let now = self.0.now();
        let scope = SiteTurnKey {
            org,
            conversation,
            turn,
        };
        self.0.write(move |conn| {
            let org = org.to_string();
            let conversation = conversation.to_string();
            let existing: Option<(MessageId, TurnId, String)> = conn.query_row(
                "SELECT s.message_id, s.id, m.text FROM site_turns s
                 JOIN messages m ON m.org_id = s.org_id AND m.id = s.message_id
                 WHERE s.org_id = ?1 AND s.conversation_id = ?2 AND s.idempotency_key = ?3",
                params![org, conversation, key],
                |row| Ok((column_id(row, 0)?, column_id(row, 1)?, row.get(2)?)),
            ).optional()?;
            if let Some((message, turn, original)) = existing {
                return Ok(if original == text { SiteSubmission::Replayed(message, turn) }
                    else { SiteSubmission::Conflict });
            }
            if let Some(active) = conn.query_row(
                "SELECT id FROM site_turns WHERE org_id = ?1 AND conversation_id = ?2
                 AND status IN ('queued', 'running')",
                params![org, conversation],
                |row| column_id(row, 0),
            ).optional()? {
                return Ok(SiteSubmission::Busy(active));
            }
            let open = conn.query_row(
                "SELECT status = 'open' FROM conversations
                 WHERE org_id = ?1 AND id = ?2 AND channel = 'site'",
                params![org, conversation], |row| row.get::<_, bool>(0),
            ).optional()?;
            if open != Some(true) {
                return require(None, "conversa", &conversation);
            }
            conn.execute(
                "INSERT INTO messages (id, org_id, conversation_id, direction, role, text, turn_id, created_at)
                 VALUES (?1, ?2, ?3, 'in', 'user', ?4, ?5, ?6)",
                params![message.to_string(), org, conversation, text, turn.to_string(), now],
            )?;
            conn.execute(
                "INSERT INTO site_turns (org_id, conversation_id, id, message_id, idempotency_key, status, started_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'queued', ?6)",
                params![org, conversation, turn.to_string(), message.to_string(), key, now],
            )?;
            touch(conn, scope, now)?;
            event(conn, scope, "message", Some((message, "user")), Some(&text), None)?;
            Ok(SiteSubmission::Accepted(message, turn))
        })
    }

    pub fn claim(&self, key: SiteTurnKey) -> Resultado<Option<(AgentId, MessageId)>> {
        let now = self.0.now();
        self.0.write(move |conn| {
            let ids = (key.org.to_string(), key.conversation.to_string(), key.turn.to_string());
            let message = conn.query_row(
                "UPDATE site_turns SET status = 'running', started_at = ?4
                 WHERE org_id = ?1 AND conversation_id = ?2 AND id = ?3 AND status = 'queued'
                 RETURNING message_id",
                params![ids.0, ids.1, ids.2, now],
                |row| column_id(row, 0),
            ).optional()?;
            let Some(message) = message else { return Ok(None) };
            let agent: AgentId = conn.query_row(
                "SELECT agent_id FROM conversations WHERE org_id = ?1 AND id = ?2 AND channel = 'site'",
                params![ids.0, ids.1], |row| column_id(row, 0),
            )?;
            conn.execute(
                "INSERT INTO turns (id, org_id, conversation_id, agent_id, started_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![ids.2, ids.0, ids.1, agent.to_string(), now],
            )?;
            event(conn, key, "turn_start", None, None, None)?;
            Ok(Some((agent, message)))
        })
    }

    pub fn delta(&self, key: SiteTurnKey, text: String) -> Resultado<()> {
        self.0.write(move |conn| {
            if load_turn(conn, key)?.status == "running" {
                event(conn, key, "text_delta", None, Some(&text), None)?;
            }
            Ok(())
        })
    }

    pub fn finish(&self, key: SiteTurnKey, outcome: SiteOutcome) -> Resultado<SiteTurn> {
        let now = self.0.now();
        self.0.write(move |conn| finish(conn, key, outcome, now))
    }

    pub fn recover(&self) -> Resultado<Vec<SiteTurnKey>> {
        let now = self.0.now();
        self.0.write(move |conn| {
            let running = scan_turns(conn, "running")?;
            for key in running {
                finish(conn, key, SiteOutcome::Failed("server_restarted"), now)?;
            }
            scan_turns(conn, "queued")
        })
    }

    pub fn turn(&self, key: SiteTurnKey) -> Resultado<SiteTurn> {
        self.0.read(|conn| load_turn(conn, key))
    }

    pub fn events(
        &self,
        org: OrgId,
        conversation: ConversationId,
        after: i64,
        limit: u32,
    ) -> Resultado<Vec<SiteEvent>> {
        self.0.read(|conn| {
            let mut statement = conn.prepare(
                "SELECT seq, kind, turn_id, message_id, role, text, error_code FROM site_events
                 WHERE org_id = ?1 AND conversation_id = ?2 AND seq > ?3 ORDER BY seq LIMIT ?4",
            )?;
            let rows = statement.query_map(
                params![org.to_string(), conversation.to_string(), after, limit],
                |row| {
                    Ok(SiteEvent {
                        seq: row.get(0)?,
                        kind: row.get(1)?,
                        turn_id: column_id(row, 2)?,
                        message_id: id_optional(row, 3)?,
                        role: row.get(4)?,
                        text: row.get(5)?,
                        error_code: row.get(6)?,
                    })
                },
            )?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn history(
        &self,
        org: OrgId,
        conversation: ConversationId,
        before: Option<MessageId>,
        limit: u32,
    ) -> Resultado<Vec<SiteMessage>> {
        self.0.read(|conn| {
            let mut statement = conn.prepare(
                "SELECT id, role, text, created_at FROM messages
                 WHERE org_id = ?1 AND conversation_id = ?2
                 AND ((direction = 'in' AND role = 'user') OR (direction = 'out' AND role = 'assistant'))
                 AND origin = 'agent' AND (?3 IS NULL OR id < ?3) ORDER BY id DESC LIMIT ?4",
            )?;
            let rows = statement.query_map(params![org.to_string(), conversation.to_string(), before.map(|id| id.to_string()), limit], |row| {
                Ok(SiteMessage { id: column_id(row, 0)?, role: row.get(1)?, text: row.get(2)?, created_at: row.get(3)? })
            })?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }
}

fn scan_turns(conn: &Connection, status: &str) -> Resultado<Vec<SiteTurnKey>> {
    let mut statement =
        conn.prepare("SELECT org_id, conversation_id, id FROM site_turns WHERE status = ?1")?;
    let rows = statement.query_map([status], |row: &Row<'_>| {
        Ok(SiteTurnKey {
            org: column_id(row, 0)?,
            conversation: column_id(row, 1)?,
            turn: column_id(row, 2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn load_turn(conn: &Connection, key: SiteTurnKey) -> Resultado<SiteTurn> {
    let turn = conn
        .query_row(
            "SELECT status, error_code, started_at, finished_at FROM site_turns
         WHERE org_id = ?1 AND conversation_id = ?2 AND id = ?3",
            params![
                key.org.to_string(),
                key.conversation.to_string(),
                key.turn.to_string()
            ],
            |row| {
                Ok(SiteTurn {
                    status: row.get(0)?,
                    error_code: row.get(1)?,
                    started_at: row.get(2)?,
                    finished_at: row.get(3)?,
                })
            },
        )
        .optional()?;
    require(turn, "turno", &key.turn.to_string())
}

fn touch(conn: &Connection, key: SiteTurnKey, now: i64) -> Resultado<()> {
    conn.execute(
        "UPDATE conversations SET last_message_at = ?3, updated_at = ?3 WHERE org_id = ?1 AND id = ?2",
        params![key.org.to_string(), key.conversation.to_string(), now],
    )?;
    Ok(())
}

fn finish(
    conn: &Connection,
    key: SiteTurnKey,
    outcome: SiteOutcome,
    now: i64,
) -> Resultado<SiteTurn> {
    let mut turn = load_turn(conn, key)?;
    if !matches!(turn.status.as_str(), "queued" | "running") {
        return Ok(turn);
    }
    let ids = (
        key.org.to_string(),
        key.conversation.to_string(),
        key.turn.to_string(),
    );
    let (status, state, kind, error, input, output) = match outcome {
        SiteOutcome::Success(reply) => {
            if turn.status != "running" {
                return Err(StoreError::Internal(
                    "turno do site não iniciado".to_owned(),
                ));
            }
            let id = MessageId::generate();
            conn.execute(
                "INSERT INTO messages (id, org_id, conversation_id, direction, role, text, turn_id, created_at)
                 VALUES (?1, ?2, ?3, 'out', 'assistant', ?4, ?5, ?6)",
                params![id.to_string(), ids.0, ids.1, reply.text, ids.2, now],
            )?;
            touch(conn, key, now)?;
            event(
                conn,
                key,
                "message",
                Some((id, "assistant")),
                Some(&reply.text),
                None,
            )?;
            (
                "succeeded",
                "done",
                "turn_end",
                None,
                reply.input_tokens,
                reply.output_tokens,
            )
        }
        SiteOutcome::Failed(error) => ("failed", "failed", "turn_failed", Some(error), 0, 0),
        SiteOutcome::Cancelled => ("cancelled", "failed", "turn_cancelled", None, 0, 0),
    };
    conn.execute(
        "UPDATE turns SET state = ?4, error = ?5, finished_at = ?6, input_tokens = ?7, output_tokens = ?8
         WHERE org_id = ?1 AND conversation_id = ?2 AND id = ?3",
        params![ids.0, ids.1, ids.2, state, error, now, input, output],
    )?;
    conn.execute(
        "UPDATE site_turns SET status = ?4, error_code = ?5, finished_at = ?6
         WHERE org_id = ?1 AND conversation_id = ?2 AND id = ?3",
        params![ids.0, ids.1, ids.2, status, error, now],
    )?;
    event(conn, key, kind, None, None, error)?;
    turn.status = status.to_owned();
    turn.error_code = error.map(str::to_owned);
    turn.finished_at = Some(now);
    Ok(turn)
}

fn event(
    conn: &Connection,
    key: SiteTurnKey,
    kind: &str,
    message: Option<(MessageId, &str)>,
    text: Option<&str>,
    error: Option<&str>,
) -> Resultado<()> {
    let org = key.org.to_string();
    let conversation = key.conversation.to_string();
    let seq: i64 = conn.query_row(
        "UPDATE site_conversation_cursors SET last_seq = last_seq + 1
         WHERE org_id = ?1 AND conversation_id = ?2 RETURNING last_seq",
        params![org, conversation],
        |row| row.get(0),
    )?;
    let (message_id, role) = message.map_or((None, None), |(id, role)| {
        (Some(id.to_string()), Some(role))
    });
    conn.execute(
        "INSERT INTO site_events (org_id, conversation_id, seq, turn_id, kind, message_id, role, text, error_code)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![org, conversation, seq, key.turn.to_string(), kind, message_id, role, text, error],
    )?;
    Ok(())
}
