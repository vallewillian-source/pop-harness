use poph_types::{AgentId, ConversationId, DepartmentId, OrgId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{Conversa, ConversationStatus, NewConversation, Page};
use crate::repos::{belongs, column_id, column_u64, conflict_or_missing, id_optional};

const COLUMNS: &str = "id, org_id, agent_id, integration_id, channel, external_peer, status,
    target_department_id, summary, version, created_at, updated_at, last_message_at, client_name";

pub const LIMIT_NAME_CLIENT: usize = 100;

pub const REASON_CLOSING_ADMIN: &str = "encerrada pelo administrador";

pub const REASON_CLOSING_SITUATION: &str = "conversa fechada por mudança de situação";

pub fn normalise_name_client(raw: &str) -> Option<String> {
    let mut clean_value = String::with_capacity(raw.len());
    let mut last_was_space = true;
    for character in raw.chars() {
        if character.is_whitespace() {
            if !last_was_space {
                clean_value.push(' ');
                last_was_space = true;
            }
            continue;
        }
        if character.is_control() || is_mark_invisible(character) {
            continue;
        }
        clean_value.push(character);
        last_was_space = false;
    }
    let cut: String = clean_value
        .trim_end()
        .chars()
        .take(LIMIT_NAME_CLIENT)
        .collect::<String>();
    let cut = cut.trim_end().to_owned();
    (!cut.is_empty()).then_some(cut)
}

fn is_mark_invisible(character: char) -> bool {
    matches!(
        character,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

pub(crate) fn write_name_client(
    conn: &rusqlite::Connection,
    text_org: &str,
    text_id: &str,
    nome: &str,
    now: i64,
) -> Resultado<bool> {
    let Some(nome) = normalise_name_client(nome) else {
        return Ok(false);
    };
    let changed = conn
        .execute(
            "UPDATE conversations
                SET client_name = ?3, updated_at = ?4
              WHERE org_id = ?1 AND id = ?2 AND client_name IS NOT ?3",
            params![text_org, text_id, nome, now],
        )
        .map_err(StoreError::from)?;
    Ok(changed > 0)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterConversations {
    pub status: Option<ConversationStatus>,
    pub external_peer: Option<String>,
    pub last_activity_before: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OrderConversations {
    #[default]
    Id,
    Recent,
}

impl Conversa {
    pub fn identifier_client(&self) -> String {
        self.peer_externo
            .as_deref()
            .map(str::trim)
            .filter(|peer| !peer.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| self.id.to_string())
    }
}

type ValueSql = rusqlite::types::Value;

fn build_listing(
    org: String,
    filter: &FilterConversations,
    ordem: OrderConversations,
    after: Option<String>,
    limit: i64,
) -> (String, Vec<ValueSql>) {
    let mut conditions = vec!["org_id = ?1".to_owned()];
    let mut values = vec![ValueSql::Text(org)];
    if let Some(status) = filter.status {
        values.push(ValueSql::Text(status.as_text().to_owned()));
        conditions.push(format!("status = ?{}", values.len()));
    }
    if let Some(peer) = &filter.external_peer {
        values.push(ValueSql::Text(peer.clone()));
        conditions.push(format!("external_peer = ?{}", values.len()));
    }
    if let Some(limit) = filter.last_activity_before {
        values.push(ValueSql::Integer(limit));
        conditions.push(format!(
            "COALESCE(last_message_at, created_at) < ?{}",
            values.len()
        ));
    }
    let ordenacao = match ordem {
        OrderConversations::Id => {
            if let Some(after) = after {
                values.push(ValueSql::Text(after));
                conditions.push(format!("id > ?{}", values.len()));
            }
            "id"
        }
        OrderConversations::Recent => "COALESCE(last_message_at, created_at) DESC, id DESC",
    };
    values.push(ValueSql::Integer(limit));
    let origin = if filter.external_peer.is_some() {
        "conversations INDEXED BY ix_conversations_org_peer_id"
    } else {
        "conversations"
    };
    let sql = format!(
        "SELECT {COLUMNS} FROM {origin} WHERE {} ORDER BY {ordenacao} LIMIT ?{}",
        conditions.join(" AND "),
        values.len()
    );
    (sql, values)
}

pub struct Conversas<'a>(pub(crate) &'a Store);

impl Conversas<'_> {
    pub fn reopen_audited(
        &self,
        org: OrgId,
        id: ConversationId,
        expected: u64,
        actor: String,
    ) -> Resultado<Conversa> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let now = self.0.now();
        let audit_id = poph_types::AuditId::generate().to_string();
        let expected_i64 = i64::try_from(expected).unwrap_or(i64::MAX);
        let closed = ConversationStatus::Closed.as_text();
        self.0.write(move |conn| {
            let current: Option<(i64, String)> = conn
                .query_row(
                    "SELECT version, status FROM conversations WHERE org_id = ?1 AND id = ?2",
                    params![text_org, text_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(StoreError::from)?;
            let Some((version_current, situation)) = current else {
                return super::require::<()>(None, "conversa", &text_id);
            };
            if version_current != expected_i64 {
                return Err(StoreError::Domain(
                    poph_types::DomainError::VersionConflict {
                        entity: "conversa",
                        id: text_id.clone(),
                        expected,
                        current: u64::try_from(version_current).unwrap_or(0),
                    },
                ));
            }
            if situation == closed {
                return Err(StoreError::Domain(poph_types::DomainError::ConversationClosed {
                    id: text_id.clone(),
                }));
            }
            let changed = conn.execute("UPDATE conversations SET status='open', version=version+1, updated_at=?3 WHERE org_id=?1 AND id=?2 AND version=?4", params![text_org, text_id, now, expected_i64])?;
            if changed == 0 {
                return Err(conflict_or_missing(conn, "conversations", "conversa", &text_id, &text_org, expected));
            }
            conn.execute("INSERT INTO audit_log (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json, created_at) VALUES (?1, ?2, 'admin_token', ?3, 'reopen', 'conversation', ?4, '{}', ?5)", params![audit_id, text_org, actor, text_id, now])?;
            Ok(())
        })?;
        self.get(org, id)
    }

    pub fn close_audited(
        &self,
        org: OrgId,
        id: ConversationId,
        expected: u64,
        actor: String,
    ) -> Resultado<Conversa> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let now = self.0.now();
        let audit_id = poph_types::AuditId::generate().to_string();
        let expected_i64 = i64::try_from(expected).unwrap_or(i64::MAX);
        let closed = ConversationStatus::Closed.as_text();
        let metrics = self.0.execution_metrics();
        self.0.write(move |conn| {
            let current: Option<(i64, String)> = conn
                .query_row(
                    "SELECT version, status FROM conversations WHERE org_id = ?1 AND id = ?2",
                    params![text_org, text_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(StoreError::from)?;
            let Some((version_current, situation)) = current else {
                return super::require::<()>(None, "conversa", &text_id);
            };
            if version_current != expected_i64 {
                return Err(StoreError::Domain(
                    poph_types::DomainError::VersionConflict {
                        entity: "conversa",
                        id: text_id.clone(),
                        expected,
                        current: u64::try_from(version_current).unwrap_or(0),
                    },
                ));
            }
            if situation == closed {
                return Err(StoreError::Domain(poph_types::DomainError::Conflict {
                    entity: "conversa",
                    id: text_id.clone(),
                    reason: "a conversa já está fechada".to_owned(),
                }));
            }
            let changed = conn
                .execute(
                    "UPDATE conversations
                        SET status = 'closed', version = version + 1, updated_at = ?3
                      WHERE org_id = ?1 AND id = ?2 AND version = ?4",
                    params![text_org, text_id, now, expected_i64],
                )
                .map_err(StoreError::from)?;
            if changed == 0 {
                return Err(conflict_or_missing(
                    conn,
                    "conversations",
                    "conversa",
                    &text_id,
                    &text_org,
                    expected,
                ));
            }
            conn.execute(
                "INSERT INTO audit_log
                    (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json,
                     created_at)
                 VALUES (?1, ?2, 'admin_token', ?3, 'close', 'conversation', ?4, '{}', ?5)",
                params![audit_id, text_org, actor, text_id, now],
            )
            .map_err(StoreError::from)?;
            super::cases::close_case_for_conversation(
                conn,
                &text_org,
                &text_id,
                poph_types::ClosedBy::Admin,
                REASON_CLOSING_ADMIN,
                now,
                metrics.as_ref(),
            )?;
            Ok(())
        })?;
        self.get(org, id)
    }

    pub fn close_by_agent(
        &self,
        org: OrgId,
        id: ConversationId,
        agent: AgentId,
        motivo: String,
    ) -> Resultado<bool> {
        if motivo.trim().is_empty() {
            return Err(StoreError::Domain(poph_types::DomainError::Validation {
                field: "motivo",
                reason: "informe o motivo do encerramento".to_owned(),
            }));
        }
        let text_org = org.to_string();
        let text_id = id.to_string();
        let text_agent = agent.to_string();
        let now = self.0.now();
        let audit_id = poph_types::AuditId::generate().to_string();
        let metrics = self.0.execution_metrics();
        self.0.write(move |conn| {
            let situation: Option<String> = conn.query_row(
                "SELECT status FROM conversations WHERE org_id=?1 AND id=?2 AND agent_id=?3",
                params![text_org, text_id, text_agent], |row| row.get(0),
            ).optional()?;
            let Some(situation) = situation else {
                return super::require::<bool>(None, "conversa", &text_id);
            };
            if situation == "closed" { return Ok(false); }
            conn.execute(
                "UPDATE conversations SET status='closed', version=version+1, updated_at=?3 WHERE org_id=?1 AND id=?2",
                params![text_org, text_id, now],
            )?;
            super::cases::close_case_for_conversation(
                conn, &text_org, &text_id, poph_types::ClosedBy::Agent, &motivo, now,
                metrics.as_ref(),
            )?;
            let detail = serde_json::json!({"motivo": motivo, "ator": "agente"}).to_string();
            conn.execute(
                "INSERT INTO audit_log (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json, created_at) VALUES (?1, ?2, 'system', ?3, 'close', 'conversation', ?4, ?5, ?6)",
                params![audit_id, text_org, text_agent, text_id, detail, now],
            )?;
            Ok(true)
        })
    }

    pub fn transfer(
        &self,
        org: OrgId,
        id: ConversationId,
        department_target: Option<DepartmentId>,
        actor_id: Option<String>,
        detail_json: String,
    ) -> Resultado<Conversa> {
        self.transfer_internal(org, id, department_target, actor_id, detail_json, false)
    }

    pub fn transfer_audited(
        &self,
        org: OrgId,
        id: ConversationId,
        department_target: Option<DepartmentId>,
        actor_id: Option<String>,
        detail_json: String,
    ) -> Resultado<Conversa> {
        self.transfer_internal(org, id, department_target, actor_id, detail_json, true)
    }

    fn transfer_internal(
        &self,
        org: OrgId,
        id: ConversationId,
        department_target: Option<DepartmentId>,
        actor_id: Option<String>,
        detail_json: String,
        refuse_closed: bool,
    ) -> Resultado<Conversa> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_id = id.to_string();
        let audit_id = poph_types::AuditId::generate().to_string();
        let closed = ConversationStatus::Closed.as_text();
        self.0.write(move |conn| {
            if let Some(departamento) = department_target {
                belongs(
                    conn,
                    "departments",
                    "departamento",
                    &text_org,
                    &departamento.to_string(),
                )?;
            }
            let situation: Option<(String, bool)> = conn
                .query_row(
                    "SELECT status, channel = 'site' FROM conversations WHERE org_id = ?1 AND id = ?2",
                    params![text_org, text_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((situation, is_site)) = situation else {
                return super::require::<()>(None, "conversa", &text_id);
            };
            if is_site {
                return Err(poph_types::DomainError::Conflict {
                    entity: "conversa",
                    id: text_id.clone(),
                    reason: "conversa do site não permite transferência".to_owned(),
                }
                .into());
            }
            if refuse_closed && situation == closed {
                return Err(StoreError::Domain(
                    poph_types::DomainError::ConversationClosed {
                        id: text_id.clone(),
                    },
                ));
            }
            let changed = conn.execute(
                "UPDATE conversations
                    SET status = 'transferred',
                        target_department_id = COALESCE(?3, target_department_id),
                        version = version + 1,
                        updated_at = ?4
                  WHERE org_id = ?1 AND id = ?2",
                params![
                    text_org,
                    text_id,
                    department_target.map(|dep| dep.to_string()),
                    now
                ],
            )?;
            if changed == 0 {
                return super::require::<()>(None, "conversa", &text_id);
            }
            conn.execute(
                "INSERT INTO audit_log
                    (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json,
                     created_at)
                 VALUES (?1, ?2, 'system', ?3, 'transfer', 'conversation', ?4, ?5, ?6)",
                params![audit_id, text_org, actor_id, text_id, detail_json, now],
            )
            .map_err(StoreError::from)?;
            Ok(())
        })?;
        self.get(org, id)
    }

    pub fn create(&self, org: OrgId, fresh: NewConversation) -> Resultado<Conversa> {
        let id = ConversationId::generate();
        let now = self.0.now();
        let text_id = id.to_string();
        let text_org = org.to_string();
        let text_agent = fresh.agent_id.to_string();
        let canal = fresh.canal;
        let peer = fresh.peer_externo;
        self.0.write(move |conn| {
            if let Some(departamento) = fresh.target_department {
                belongs(
                    conn,
                    "departments",
                    "departamento",
                    &text_org,
                    &departamento.to_string(),
                )?;
            }
            conn.execute(
                "INSERT INTO conversations
                    (id, org_id, agent_id, integration_id, channel, external_peer, status,
                     target_department_id, summary, version, created_at, updated_at, last_message_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'open', ?7, NULL, 1, ?8, ?8, NULL)",
                params![
                    text_id,
                    text_org,
                    &text_agent,
                    fresh.integration_id.map(|id| id.to_string()),
                    &canal,
                    peer.as_deref(),
                    fresh.target_department.map(|dep| dep.to_string()),
                    now
                ],
            )
            .map_err(StoreError::from)?;
            super::cases::create_case_for_conversation(
                conn,
                &text_org,
                &text_id,
                &text_agent,
                &canal,
                peer.as_deref(),
                now,
            )?;
            Ok(())
        })?;
        self.get(org, id)
    }

    pub fn changed_since(
        &self,
        after: Option<(i64, ConversationId)>,
        limit: u32,
    ) -> Resultado<Vec<Conversa>> {
        let (instant, id) = match after {
            Some((instant, id)) => (instant, id.to_string()),
            None => (i64::MIN, String::new()),
        };
        let limit = i64::from(limit.clamp(1, 1000));
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM conversations
                  WHERE (updated_at, id) > (?1, ?2)
                  ORDER BY updated_at, id
                  LIMIT ?3"
            ))?;
            let rows = statement.query_map(params![instant, id, limit], row)?;
            let mut conversations = Vec::new();
            for row in rows {
                conversations.push(row?);
            }
            Ok(conversations)
        })
    }

    pub fn get(&self, org: OrgId, id: ConversationId) -> Resultado<Conversa> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let found = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS} FROM conversations WHERE id = ?1 AND org_id = ?2"),
                params![text_id, text_org],
                row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(found, "conversa", &id.to_string())
    }

    pub fn list(
        &self,
        org: OrgId,
        status: Option<ConversationStatus>,
        page: Page,
    ) -> Resultado<Vec<Conversa>> {
        let text_org = org.to_string();
        let status = status.map(ConversationStatus::as_text);
        let after = page.after.map(|id| id.to_string());
        let limit = i64::from(page.limit);
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM conversations
                  WHERE org_id = ?1
                    AND (?2 IS NULL OR status = ?2)
                    AND (?3 IS NULL OR id > ?3)
                  ORDER BY id LIMIT ?4"
            ))?;
            let rows = statement.query_map(params![text_org, status, after, limit], row)?;
            let mut conversations = Vec::new();
            for row in rows {
                conversations.push(row?);
            }
            Ok(conversations)
        })
    }

    pub fn list_filter(
        &self,
        org: OrgId,
        filter: &FilterConversations,
        page: Page,
    ) -> Resultado<Vec<Conversa>> {
        let after = page.after.map(|id| id.to_string());
        let (sql, values) = build_listing(
            org.to_string(),
            filter,
            OrderConversations::Id,
            after,
            i64::from(page.limit),
        );
        self.query(sql, values)
    }

    pub fn list_recents(
        &self,
        org: OrgId,
        filter: &FilterConversations,
        limit: u32,
    ) -> Resultado<Vec<Conversa>> {
        let (sql, values) = build_listing(
            org.to_string(),
            filter,
            OrderConversations::Recent,
            None,
            i64::from(limit.clamp(1, 200)),
        );
        self.query(sql, values)
    }

    pub fn listing_plan(
        &self,
        org: OrgId,
        filter: &FilterConversations,
        ordem: OrderConversations,
    ) -> Resultado<Vec<String>> {
        let (sql, values) = build_listing(org.to_string(), filter, ordem, None, 50);
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
            let rows = statement.query_map(rusqlite::params_from_iter(values.iter()), |row| {
                row.get::<_, String>(3)
            })?;
            let mut steps = Vec::new();
            for row in rows {
                steps.push(row?);
            }
            Ok(steps)
        })
    }

    fn query(&self, sql: String, values: Vec<ValueSql>) -> Resultado<Vec<Conversa>> {
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&sql)?;
            let rows = statement.query_map(rusqlite::params_from_iter(values.iter()), row)?;
            let mut conversations = Vec::new();
            for row in rows {
                conversations.push(row?);
            }
            Ok(conversations)
        })
    }

    pub fn list_by_agent_channel(
        &self,
        org: OrgId,
        agent: AgentId,
        canal: &str,
        after: Option<ConversationId>,
        limit: u32,
    ) -> Resultado<Vec<Conversa>> {
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        let canal = canal.to_owned();
        let after = after.map(|id| id.to_string());
        let limit = i64::from(limit.clamp(1, 200));
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM conversations
                  WHERE org_id = ?1 AND agent_id = ?2 AND channel = ?3
                    AND (?4 IS NULL OR id < ?4)
                  ORDER BY id DESC LIMIT ?5"
            ))?;
            let rows =
                statement.query_map(params![text_org, text_agent, canal, after, limit], row)?;
            let mut conversations = Vec::new();
            for row in rows {
                conversations.push(row?);
            }
            Ok(conversations)
        })
    }

    pub fn change_status(
        &self,
        org: OrgId,
        id: ConversationId,
        expected: u64,
        status: ConversationStatus,
        department_target: Option<DepartmentId>,
    ) -> Resultado<Conversa> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_id = id.to_string();
        let expected_i64 = expected as i64;
        let closed = ConversationStatus::Closed.as_text();
        let metrics = self.0.execution_metrics();
        self.0.write(move |conn| {
            if let Some(departamento) = department_target {
                belongs(
                    conn,
                    "departments",
                    "departamento",
                    &text_org,
                    &departamento.to_string(),
                )?;
            }
            let situation_current: Option<String> = conn
                .query_row(
                    "SELECT status FROM conversations WHERE org_id = ?1 AND id = ?2",
                    params![text_org, text_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            if status == ConversationStatus::Open && situation_current.as_deref() == Some(closed) {
                return Err(StoreError::Domain(
                    poph_types::DomainError::ConversationClosed {
                        id: text_id.clone(),
                    },
                ));
            }
            let changed = conn
                .execute(
                    "UPDATE conversations
                        SET status = ?3,
                            target_department_id = COALESCE(?4, target_department_id),
                            version = version + 1,
                            updated_at = ?5
                      WHERE org_id = ?1 AND id = ?2 AND version = ?6",
                    params![
                        text_org,
                        text_id,
                        status.as_text(),
                        department_target.map(|dep| dep.to_string()),
                        now,
                        expected_i64
                    ],
                )
                .map_err(StoreError::from)?;
            if changed == 0 {
                return Err(conflict_or_missing(
                    conn,
                    "conversations",
                    "conversa",
                    &text_id,
                    &text_org,
                    expected,
                ));
            }
            if status == ConversationStatus::Closed {
                super::cases::close_case_for_conversation(
                    conn,
                    &text_org,
                    &text_id,
                    poph_types::ClosedBy::System,
                    REASON_CLOSING_SITUATION,
                    now,
                    metrics.as_ref(),
                )?;
            }
            Ok(())
        })?;
        self.get(org, id)
    }

    pub fn touch(&self, org: OrgId, id: ConversationId, when: i64) -> Resultado<()> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        self.0.write(move |conn| {
            conn.execute(
                "UPDATE conversations
                    SET last_message_at = ?3, updated_at = ?3
                  WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id, when],
            )
            .map(|_| ())
            .map_err(StoreError::from)
        })
    }

    pub fn define_summary(&self, org: OrgId, id: ConversationId, summary: &str) -> Resultado<()> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let summary = summary.to_owned();
        self.0.write(move |conn| {
            conn.execute(
                "UPDATE conversations SET summary = ?3 WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id, summary],
            )
            .map(|_| ())
            .map_err(StoreError::from)
        })
    }

    pub fn set_customer_name(&self, org: OrgId, id: ConversationId, nome: &str) -> Resultado<bool> {
        let nome = nome.to_owned();
        let now = self.0.now();
        let text_org = org.to_string();
        let text_id = id.to_string();
        self.0
            .write(move |conn| write_name_client(conn, &text_org, &text_id, &nome, now))
    }

    pub fn agent(&self, org: OrgId, id: ConversationId) -> Resultado<AgentId> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let agent = self.0.read(move |conn| {
            conn.query_row(
                "SELECT agent_id FROM conversations WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id],
                |row| column_id::<AgentId>(row, 0),
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(agent, "conversa", &id.to_string())
    }
}

fn row(row: &Row<'_>) -> rusqlite::Result<Conversa> {
    Ok(Conversa {
        id: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        agent_id: column_id(row, 2)?,
        integration_id: id_optional(row, 3)?,
        canal: row.get(4)?,
        peer_externo: row.get(5)?,
        status: ConversationStatus::from_text(&row.get::<_, String>(6)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        target_department: id_optional(row, 7)?,
        summary: row.get(8)?,
        version: column_u64(row, 9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        last_message_at: row.get(12)?,
        customer_name: row.get(13)?,
    })
}
