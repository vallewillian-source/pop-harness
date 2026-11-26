use poph_types::{AgentId, OrgId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{Instructions, Page};
use crate::repos::{column_id, column_u64, conflict_or_missing};

const COLUMNS: &str = "agent_id, org_id, structured_json, markdown, derived_from_json, version, created_at, updated_at";

pub struct InstructionsRepo<'a>(pub(crate) &'a Store);

impl InstructionsRepo<'_> {
    pub fn get(&self, org: OrgId, agent: AgentId) -> Resultado<Instructions> {
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        let found_many = self.0.read(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUMNS} FROM agent_instructions WHERE org_id = ?1 AND agent_id = ?2"
                ),
                params![text_org, text_agent],
                row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(found_many, "instruções do agente", &agent.to_string())
    }

    pub fn version(&self, org: OrgId, agent: AgentId) -> Resultado<Option<u64>> {
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT version FROM agent_instructions WHERE org_id = ?1 AND agent_id = ?2",
                params![text_org, text_agent],
                |row| column_u64(row, 0),
            )
            .optional()
            .map_err(StoreError::from)
        })
    }

    pub fn save(
        &self,
        org: OrgId,
        agent: AgentId,
        expected: u64,
        structured_json: &str,
        markdown: &str,
        derived_from_json: bool,
    ) -> Resultado<Instructions> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        let structured_json = structured_json.to_owned();
        let markdown = markdown.to_owned();
        let derived = i64::from(derived_from_json);
        self.0.write(move |conn| {
            let current: Option<i64> = conn
                .query_row(
                    "SELECT version FROM agent_instructions WHERE org_id = ?1 AND agent_id = ?2",
                    params![text_org, text_agent],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            match current {
                Some(current) => {
                    if current.max(0) as u64 != expected {
                        return Err(StoreError::Domain(
                            poph_types::DomainError::VersionConflict {
                                entity: "instruções do agente",
                                id: text_agent,
                                expected,
                                current: current.max(0) as u64,
                            },
                        ));
                    }
                    conn.execute(
                        "UPDATE agent_instructions
                            SET structured_json = ?3, markdown = ?4, derived_from_json = ?5,
                                version = version + 1, updated_at = ?6
                          WHERE org_id = ?1 AND agent_id = ?2",
                        params![
                            text_org,
                            text_agent,
                            structured_json,
                            markdown,
                            derived,
                            now
                        ],
                    )
                    .map_err(StoreError::from)?;
                }
                None => {
                    if expected != 0 {
                        return Err(StoreError::Domain(
                            poph_types::DomainError::VersionConflict {
                                entity: "instruções do agente",
                                id: text_agent,
                                expected,
                                current: 0,
                            },
                        ));
                    }
                    let exists_agent: Option<i64> = conn
                        .query_row(
                            "SELECT 1 FROM agents WHERE org_id = ?1 AND id = ?2",
                            params![text_org, text_agent],
                            |row| row.get(0),
                        )
                        .optional()
                        .map_err(StoreError::from)?;
                    if exists_agent.is_none() {
                        return Err(conflict_or_missing(
                            conn,
                            "agents",
                            "agente",
                            &text_agent,
                            &text_org,
                            0,
                        ));
                    }
                    conn.execute(
                        "INSERT INTO agent_instructions
                            (agent_id, org_id, structured_json, markdown, derived_from_json,
                             version, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?6)",
                        params![
                            text_agent,
                            text_org,
                            structured_json,
                            markdown,
                            derived,
                            now
                        ],
                    )
                    .map_err(StoreError::from)?;
                }
            }
            Ok(())
        })?;
        self.get(org, agent)
    }

    pub fn list(&self, org: OrgId, page: Page) -> Resultado<Vec<Instructions>> {
        let text_org = org.to_string();
        let after = page.after.map(|id| id.to_string());
        let limit = i64::from(page.limit);
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM agent_instructions
                  WHERE org_id = ?1 AND (?2 IS NULL OR agent_id > ?2)
                  ORDER BY agent_id LIMIT ?3"
            ))?;
            let rows = statement.query_map(params![text_org, after, limit], row)?;
            let mut instructions = Vec::new();
            for item in rows {
                instructions.push(item?);
            }
            Ok(instructions)
        })
    }

    pub fn delete(&self, org: OrgId, agent: AgentId) -> Resultado<()> {
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        let deleted = self.0.write(move |conn| {
            conn.execute(
                "DELETE FROM agent_instructions WHERE org_id = ?1 AND agent_id = ?2",
                params![text_org, text_agent],
            )
            .map_err(StoreError::from)
        })?;
        if deleted == 0 {
            return Err(StoreError::Domain(poph_types::DomainError::NotFound {
                entity: "instruções do agente",
                id: agent.to_string(),
            }));
        }
        Ok(())
    }
}

fn row(row: &Row<'_>) -> rusqlite::Result<Instructions> {
    Ok(Instructions {
        agent_id: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        structured_json: row.get(2)?,
        markdown: row.get(3)?,
        derived_from_json: row.get::<_, i64>(4)? != 0,
        version: column_u64(row, 5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}
