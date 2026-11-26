use poph_types::{AgentId, AuditId, ConversationId, OrgId, TurnId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{AuditEntry, NewAuditEntry, Page, TurnFinalization, TurnState, Turno};
use crate::repos::{belongs, column_id, id_optional};

const COLUMNS_TURN: &str = "id, org_id, conversation_id, agent_id, state, jev_json,
    retrieved_json, tools_json, timings_json, input_tokens, output_tokens, cost_micros, error,
    started_at, finished_at";

const COLUMNS_AUDIT: &str =
    "id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json, created_at";

pub struct Turns<'a>(pub(crate) &'a Store);

impl Turns<'_> {
    pub fn start(
        &self,
        org: OrgId,
        conversation: ConversationId,
        agent: Option<AgentId>,
    ) -> Resultado<Turno> {
        let id = TurnId::generate();
        let now = self.0.now();
        let text_org = org.to_string();
        self.0.write(move |conn| {
            if let Some(agent) = agent {
                belongs(conn, "agents", "agente", &text_org, &agent.to_string())?;
            }
            conn.execute(
                "INSERT INTO turns
                    (id, org_id, conversation_id, agent_id, state, jev_json, retrieved_json,
                     tools_json, timings_json, input_tokens, output_tokens, cost_micros, error,
                     started_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, 'running', '{}', '[]', '[]', '{}', 0, 0, 0, NULL, ?5, NULL)",
                params![
                    id.to_string(),
                    text_org,
                    conversation.to_string(),
                    agent.map(|agent| agent.to_string()),
                    now
                ],
            )
            .map_err(StoreError::from)?;
            Ok(())
        })?;
        self.get(org, id)
    }

    pub fn finalise(&self, org: OrgId, id: TurnId, fim: TurnFinalization) -> Resultado<Turno> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_id = id.to_string();
        let state = fim.state.map(TurnState::as_text);
        let effect = self.0.write(move |conn| {
            conn.execute(
                "UPDATE turns
                    SET state = COALESCE(?3, state),
                        jev_json = COALESCE(?4, jev_json),
                        retrieved_json = COALESCE(?5, retrieved_json),
                        tools_json = COALESCE(?6, tools_json),
                        timings_json = COALESCE(?7, timings_json),
                        input_tokens = COALESCE(?8, input_tokens),
                        output_tokens = COALESCE(?9, output_tokens),
                        cost_micros = COALESCE(?10, cost_micros),
                        error = ?11,
                        finished_at = ?12
                  WHERE org_id = ?1 AND id = ?2",
                params![
                    text_org,
                    text_id,
                    state,
                    fim.jev_json,
                    fim.passages_json,
                    fim.tools_json,
                    fim.times_json,
                    fim.input_tokens.map(i64::from),
                    fim.output_tokens.map(i64::from),
                    fim.cost_micros,
                    fim.error,
                    now
                ],
            )
            .map_err(StoreError::from)
        })?;
        if effect == 0 {
            return Err(StoreError::Domain(poph_types::DomainError::NotFound {
                entity: "turno",
                id: id.to_string(),
            }));
        }
        self.get(org, id)
    }

    pub fn fail_orphans(&self, org: OrgId, motivo: &str, before: i64) -> Resultado<u64> {
        let now = self.0.now();
        let text_org = org.to_string();
        let motivo = motivo.to_owned();
        let effect = self.0.write(move |conn| {
            conn.execute(
                "UPDATE turns
                    SET state = 'failed', error = ?3, finished_at = ?4
                  WHERE org_id = ?1 AND state = 'running' AND started_at <= ?2",
                params![text_org, before, motivo, now],
            )
            .map_err(StoreError::from)
        })?;
        Ok(u64::try_from(effect).unwrap_or(u64::MAX))
    }

    pub fn get(&self, org: OrgId, id: TurnId) -> Resultado<Turno> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let located = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS_TURN} FROM turns WHERE org_id = ?1 AND id = ?2"),
                params![text_org, text_id],
                turn_row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(located, "turno", &id.to_string())
    }

    pub fn in_progress(&self, org: OrgId, conversation: ConversationId) -> Resultado<bool> {
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT 1 FROM turns
                  WHERE org_id = ?1 AND conversation_id = ?2 AND state = 'running'
                  LIMIT 1",
                params![text_org, text_conversation],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map(|finding| finding.is_some())
            .map_err(StoreError::from)
        })
    }

    pub fn list(
        &self,
        org: OrgId,
        conversation: ConversationId,
        page: Page,
    ) -> Resultado<Vec<Turno>> {
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        let after = page.after.map(|id| id.to_string());
        let limit = i64::from(page.limit);
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS_TURN} FROM turns
                  WHERE org_id = ?1 AND conversation_id = ?2 AND (?3 IS NULL OR id > ?3)
                  ORDER BY id LIMIT ?4"
            ))?;
            let rows = statement
                .query_map(params![text_org, text_conversation, after, limit], turn_row)?;
            let mut turns = Vec::new();
            for row in rows {
                turns.push(row?);
            }
            Ok(turns)
        })
    }
}

pub struct Audit<'a>(pub(crate) &'a Store);

impl Audit<'_> {
    pub fn list_globals(&self, page: Page) -> Resultado<Vec<AuditEntry>> {
        let after = page.after.map(|id| id.to_string());
        let limit = i64::from(page.limit);
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS_AUDIT} FROM audit_log WHERE org_id IS NULL AND (?1 IS NULL OR id < ?1) ORDER BY id DESC LIMIT ?2"
            ))?;
            let rows = statement.query_map(params![after, limit], audit_row)?;
            let mut records = Vec::new();
            for row in rows { records.push(row?); }
            Ok(records)
        })
    }

    pub fn register(&self, org: Option<OrgId>, new: NewAuditEntry) -> Resultado<AuditEntry> {
        let registro = AuditEntry {
            id: AuditId::generate(),
            org_id: org,
            ator_tipo: new.ator_tipo,
            actor_id: new.actor_id,
            acao: new.acao,
            entity: new.entity,
            entity_id: new.entity_id,
            detail_json: new.detail_json,
            created_at: self.0.now(),
        };
        let for_write = registro.clone();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO audit_log
                    (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json,
                     created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    for_write.id.to_string(),
                    for_write.org_id.map(|org| org.to_string()),
                    for_write.ator_tipo,
                    for_write.actor_id,
                    for_write.acao,
                    for_write.entity,
                    for_write.entity_id,
                    for_write.detail_json,
                    for_write.created_at
                ],
            )
            .map_err(StoreError::from)?;
            Ok(())
        })?;
        Ok(registro)
    }

    pub fn list(&self, org: OrgId, page: Page) -> Resultado<Vec<AuditEntry>> {
        let text_org = org.to_string();
        let after = page.after.map(|id| id.to_string());
        let limit = i64::from(page.limit);
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS_AUDIT} FROM audit_log
                  WHERE org_id = ?1 AND (?2 IS NULL OR id < ?2)
                  ORDER BY id DESC LIMIT ?3"
            ))?;
            let rows = statement.query_map(params![text_org, after, limit], audit_row)?;
            let mut records = Vec::new();
            for row in rows {
                records.push(row?);
            }
            Ok(records)
        })
    }
}

fn turn_row(row: &Row<'_>) -> rusqlite::Result<Turno> {
    Ok(Turno {
        id: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        conversation_id: column_id(row, 2)?,
        agent_id: id_optional(row, 3)?,
        state: TurnState::from_text(&row.get::<_, String>(4)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        jev_json: row.get(5)?,
        passages_json: row.get(6)?,
        tools_json: row.get(7)?,
        times_json: row.get(8)?,
        input_tokens: row.get(9)?,
        output_tokens: row.get(10)?,
        cost_micros: row.get(11)?,
        error: row.get(12)?,
        started_at: row.get(13)?,
        finished_at: row.get(14)?,
    })
}

fn audit_row(row: &Row<'_>) -> rusqlite::Result<AuditEntry> {
    Ok(AuditEntry {
        id: column_id(row, 0)?,
        org_id: id_optional(row, 1)?,
        ator_tipo: row.get(2)?,
        actor_id: row.get(3)?,
        acao: row.get(4)?,
        entity: row.get(5)?,
        entity_id: row.get(6)?,
        detail_json: row.get(7)?,
        created_at: row.get(8)?,
    })
}
