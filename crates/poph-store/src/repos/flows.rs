use poph_types::{
    AgentId, DomainError, EventActor, ExecutionId, ExecutionStatus, FlowId, OrgId, RecordEventKind,
};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::cases::register_event_conn;
use super::{belongs, column_id, column_status, column_u64, conflict_or_missing};
use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{FlowChange, FlowFilter, Fluxo, NewFlow, NewRecordEvent, Page, Status};

pub const CAP_FLOWS_BY_AGENT: u32 = 20;

pub const REASON_EXCLUSION_FLOW: &str = "fluxo_excluido";

const COLUMNS: &str = "id, org_id, slug, nome, descricao, status, rascunho_fonte, revisao_atual,
    version";

pub struct Fluxos<'a>(pub(crate) &'a Store);

impl Fluxos<'_> {
    pub fn create(&self, org: OrgId, new: NewFlow) -> Resultado<Fluxo> {
        validate_slug(&new.slug)?;
        validate_name(&new.name)?;
        let id = FlowId::generate();
        let text_id = id.to_string();
        let text_org = org.to_string();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO fluxos
                    (id, org_id, slug, nome, descricao, status, rascunho_fonte, revisao_atual, version)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, NULL, 1)",
                params![
                    text_id,
                    text_org,
                    new.slug,
                    new.name,
                    new.description,
                    new.rascunho_fonte
                ],
            )
            .map_err(|error| {
                StoreError::from_write_with(
                    "slug",
                    &[("nome", "nome"), ("descricao", "descricao")],
                    error,
                )
            })?;
            Ok(())
        })?;
        self.get(org, id)
    }

    pub fn get(&self, org: OrgId, id: FlowId) -> Resultado<Fluxo> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let located = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS} FROM fluxos WHERE org_id = ?1 AND id = ?2"),
                params![text_org, text_id],
                row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(located, "fluxo", &id.to_string())
    }

    pub fn by_slug(&self, org: OrgId, slug: &str) -> Resultado<Option<Fluxo>> {
        let text_org = org.to_string();
        let text_slug = slug.to_owned();
        self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS} FROM fluxos WHERE org_id = ?1 AND slug = ?2"),
                params![text_org, text_slug],
                row,
            )
            .optional()
            .map_err(StoreError::from)
        })
    }

    pub fn list(&self, org: OrgId, filter: &FlowFilter, page: Page) -> Resultado<Vec<Fluxo>> {
        let mut conditions = vec!["org_id = ?1".to_owned()];
        let mut values: Vec<rusqlite::types::Value> =
            vec![rusqlite::types::Value::Text(org.to_string())];
        if let Some(status) = filter.status {
            values.push(rusqlite::types::Value::Text(status.as_text().to_owned()));
            conditions.push(format!("status = ?{}", values.len()));
        }
        if let Some(after) = page.after {
            values.push(rusqlite::types::Value::Text(after.to_string()));
            conditions.push(format!("id > ?{}", values.len()));
        }
        values.push(rusqlite::types::Value::Integer(i64::from(page.limit)));
        let sql = format!(
            "SELECT {COLUMNS} FROM fluxos WHERE {} ORDER BY id LIMIT ?{}",
            conditions.join(" AND "),
            values.len()
        );
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&sql)?;
            let rows = statement.query_map(rusqlite::params_from_iter(values.iter()), row)?;
            let mut flows = Vec::new();
            for row in rows {
                flows.push(row?);
            }
            Ok(flows)
        })
    }

    pub fn count_rows(&self, org: OrgId) -> Resultado<u64> {
        let text_org = org.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM fluxos WHERE org_id = ?1",
                params![text_org],
                |row| column_u64(row, 0),
            )
            .map_err(StoreError::from)
        })
    }

    pub fn update(
        &self,
        org: OrgId,
        id: FlowId,
        expected: u64,
        change: FlowChange,
    ) -> Resultado<Fluxo> {
        if change.status == Some(Status::Archived) {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "status",
                reason: "o fluxo fica `active` ou `inactive`".to_owned(),
            }));
        }
        if let Some(nome) = &change.name {
            validate_name(nome)?;
        }
        let text_org = org.to_string();
        let text_id = id.to_string();
        let status = change.status.map(Status::as_text);
        let expected_i64 = expected as i64;
        let changed = self.0.write(move |conn| {
            conn.execute(
                "UPDATE fluxos
                    SET nome = COALESCE(?3, nome),
                        descricao = COALESCE(?4, descricao),
                        status = COALESCE(?5, status),
                        rascunho_fonte = COALESCE(?6, rascunho_fonte),
                        version = version + 1
                  WHERE org_id = ?1 AND id = ?2 AND version = ?7",
                params![
                    text_org,
                    text_id,
                    change.name,
                    change.description,
                    status,
                    change.rascunho_fonte,
                    expected_i64
                ],
            )
            .map_err(|error| StoreError::from_write("nome", error))
        })?;
        if changed == 0 {
            return Err(self.conflict(org, id, expected));
        }
        self.get(org, id)
    }

    pub fn delete(&self, org: OrgId, id: FlowId, force: bool) -> Resultado<u64> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_id = id.to_string();
        let metrics = self.0.execution_metrics();
        self.0.write(move |conn| {
            let exists: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM fluxos WHERE org_id = ?1 AND id = ?2",
                    params![text_org, text_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            if exists.is_none() {
                return Err(StoreError::Domain(DomainError::NotFound {
                    entity: "fluxo",
                    id: text_id.clone(),
                }));
            }
            let active_list = active_executions_of_flow(conn, &text_org, &text_id)?;
            if !active_list.is_empty() && !force {
                return Err(StoreError::Domain(DomainError::Conflict {
                    entity: "fluxo",
                    id: text_id.clone(),
                    reason: format!(
                        "tem {} execução(ões) ativa(s) de atendimento; envie force=true para \
                         abandoná-las e excluir",
                        active_list.len()
                    ),
                }));
            }
            for (execution, case) in &active_list {
                conn.execute(
                    "UPDATE fluxo_execucoes
                        SET status = 'abandonada', motivo_fim = ?3, finalizada_em = ?4,
                            version = version + 1
                      WHERE org_id = ?1 AND id = ?2",
                    params![text_org, execution, REASON_EXCLUSION_FLOW, now],
                )
                .map_err(StoreError::from)?;
                register_event_conn(
                    conn,
                    &text_org,
                    case,
                    &NewRecordEvent {
                        execucao_id: execution.parse::<ExecutionId>().ok(),
                        tipo: RecordEventKind::ExecutionAbandoned,
                        ator_tipo: EventActor::System,
                        ator_ref: None,
                        payload_json: format!("{{\"motivo\":\"{REASON_EXCLUSION_FLOW}\"}}"),
                        chave_idempotencia: None,
                    },
                    now,
                )?;
            }
            if let Some(metrics) = metrics.as_ref() {
                metrics.outcome(ExecutionStatus::Abandoned, active_list.len() as u64);
            }

            conn.execute(
                "DELETE FROM fluxos WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id],
            )
            .map_err(StoreError::from)?;
            Ok(active_list.len() as u64)
        })
    }

    pub fn release(&self, org: OrgId, agent: AgentId, flow: FlowId) -> Resultado<()> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        let text_flow = flow.to_string();
        self.0.write(move |conn| {
            belongs(conn, "fluxos", "fluxo", &text_org, &text_flow)?;
            belongs(conn, "agents", "agente", &text_org, &text_agent)?;
            if permission_exists(conn, &text_org, &text_agent, &text_flow)? {
                return Ok(());
            }
            let current_rows = count_permissions(conn, &text_org, &text_agent)?;
            if current_rows >= CAP_FLOWS_BY_AGENT {
                return Err(cap_error(agent, current_rows.saturating_add(1)));
            }
            conn.execute(
                "INSERT INTO agente_fluxos (org_id, agent_id, fluxo_id, liberado_em)
                 VALUES (?1, ?2, ?3, ?4)",
                params![text_org, text_agent, text_flow, now],
            )
            .map_err(StoreError::from)?;
            agent_version_advances(conn, &text_org, &text_agent, now)
        })
    }

    pub fn revoke(&self, org: OrgId, agent: AgentId, flow: FlowId) -> Resultado<()> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        let text_flow = flow.to_string();
        self.0.write(move |conn| {
            let deleted_rows = conn
                .execute(
                    "DELETE FROM agente_fluxos
                      WHERE org_id = ?1 AND agent_id = ?2 AND fluxo_id = ?3",
                    params![text_org, text_agent, text_flow],
                )
                .map_err(StoreError::from)?;
            if deleted_rows > 0 {
                agent_version_advances(conn, &text_org, &text_agent, now)?;
            }
            Ok(())
        })
    }

    pub fn list_from_agent(&self, org: OrgId, agent: AgentId) -> Resultado<Vec<Fluxo>> {
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM fluxos
                  WHERE org_id = ?1
                    AND id IN (SELECT fluxo_id FROM agente_fluxos
                                WHERE org_id = ?1 AND agent_id = ?2)
                  ORDER BY slug"
            ))?;
            let rows = statement.query_map(params![text_org, text_agent], row)?;
            let mut flows = Vec::new();
            for row in rows {
                flows.push(row?);
            }
            Ok(flows)
        })
    }

    pub fn list_agents_of_flow(&self, org: OrgId, flow: FlowId) -> Resultado<Vec<AgentId>> {
        self.get(org, flow)?;
        let text_org = org.to_string();
        let text_flow = flow.to_string();
        self.0.read(move |conn| {
            let mut statement = conn.prepare(
                "SELECT agent_id FROM agente_fluxos
                  WHERE org_id = ?1 AND fluxo_id = ?2
                  ORDER BY agent_id",
            )?;
            let rows = statement.query_map(params![text_org, text_flow], |row| {
                column_id::<AgentId>(row, 0)
            })?;
            let mut agents = Vec::new();
            for row in rows {
                agents.push(row?);
            }
            Ok(agents)
        })
    }

    pub fn replace_list(
        &self,
        org: OrgId,
        agent: AgentId,
        expected: u64,
        flows: &[FlowId],
    ) -> Resultado<()> {
        let mut unique: Vec<FlowId> = Vec::new();
        for flow in flows {
            if !unique.contains(flow) {
                unique.push(*flow);
            }
        }
        if u32::try_from(unique.len()).unwrap_or(u32::MAX) > CAP_FLOWS_BY_AGENT {
            return Err(cap_error(
                agent,
                u32::try_from(unique.len()).unwrap_or(u32::MAX),
            ));
        }
        let now = self.0.now();
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        let texts: Vec<String> = unique.iter().map(ToString::to_string).collect();
        let expected_i64 = expected as i64;
        self.0.write(move |conn| {
            let advanced = conn
                .execute(
                    "UPDATE agents SET version = version + 1, updated_at = ?4
                      WHERE org_id = ?1 AND id = ?2 AND version = ?3",
                    params![text_org, text_agent, expected_i64, now],
                )
                .map_err(StoreError::from)?;
            if advanced == 0 {
                return Err(conflict_or_missing(
                    conn,
                    "agents",
                    "agente",
                    &text_agent,
                    &text_org,
                    expected,
                ));
            }
            for text_flow in &texts {
                belongs(conn, "fluxos", "fluxo", &text_org, text_flow)?;
            }
            conn.execute(
                "DELETE FROM agente_fluxos WHERE org_id = ?1 AND agent_id = ?2",
                params![text_org, text_agent],
            )
            .map_err(StoreError::from)?;
            for text_flow in &texts {
                conn.execute(
                    "INSERT INTO agente_fluxos (org_id, agent_id, fluxo_id, liberado_em)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![text_org, text_agent, text_flow, now],
                )
                .map_err(StoreError::from)?;
            }
            Ok(())
        })
    }

    fn conflict(&self, org: OrgId, id: FlowId, expected: u64) -> StoreError {
        let text_org = org.to_string();
        let text_id = id.to_string();
        self.0
            .read(move |conn| {
                Ok(conflict_or_missing(
                    conn, "fluxos", "fluxo", &text_id, &text_org, expected,
                ))
            })
            .unwrap_or_else(|error| error)
    }
}

fn validate_slug(slug: &str) -> Resultado<()> {
    let size = slug.chars().count();
    let mut characters = slug.chars();
    let valid = (2..=64).contains(&size)
        && characters
            .next()
            .is_some_and(|first_value| first_value.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        });
    if valid {
        Ok(())
    } else {
        Err(StoreError::Domain(DomainError::Validation {
            field: "slug",
            reason: "use minúsculas, números e `_`, começando por letra, com 2 a 64 caracteres"
                .to_owned(),
        }))
    }
}

fn validate_name(nome: &str) -> Resultado<()> {
    if nome.trim().is_empty() {
        return Err(StoreError::Domain(DomainError::Validation {
            field: "nome",
            reason: "não pode ficar vazio".to_owned(),
        }));
    }
    Ok(())
}

fn cap_error(agent: AgentId, requests: u32) -> StoreError {
    StoreError::Domain(DomainError::FlowLimitPerAgent {
        agent_id: agent.to_string(),
        limit: CAP_FLOWS_BY_AGENT,
        requested: requests,
    })
}

fn permission_exists(conn: &Connection, org: &str, agent: &str, flow: &str) -> Resultado<bool> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM agente_fluxos WHERE org_id = ?1 AND agent_id = ?2 AND fluxo_id = ?3",
            params![org, agent, flow],
            |row| row.get(0),
        )
        .optional()
        .map_err(StoreError::from)?;
    Ok(exists.is_some())
}

fn count_permissions(conn: &Connection, org: &str, agent: &str) -> Resultado<u32> {
    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM agente_fluxos WHERE org_id = ?1 AND agent_id = ?2",
            params![org, agent],
            |row| row.get(0),
        )
        .map_err(StoreError::from)?;
    Ok(u32::try_from(total).unwrap_or(u32::MAX))
}

fn active_executions_of_flow(
    conn: &Connection,
    org: &str,
    flow: &str,
) -> Resultado<Vec<(String, String)>> {
    let mut statement = conn
        .prepare(
            "SELECT id, ficha_id FROM fluxo_execucoes
              WHERE org_id = ?1 AND fluxo_id = ?2 AND status = 'ativa'
              ORDER BY iniciada_em",
        )
        .map_err(StoreError::from)?;
    let rows = statement
        .query_map(params![org, flow], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(StoreError::from)?;
    let mut active_list = Vec::new();
    for row in rows {
        active_list.push(row.map_err(StoreError::from)?);
    }
    Ok(active_list)
}

fn agent_version_advances(conn: &Connection, org: &str, agent: &str, now: i64) -> Resultado<()> {
    conn.execute(
        "UPDATE agents SET version = version + 1, updated_at = ?3
          WHERE org_id = ?1 AND id = ?2",
        params![org, agent, now],
    )
    .map_err(StoreError::from)?;
    Ok(())
}

fn row(row: &Row<'_>) -> rusqlite::Result<Fluxo> {
    Ok(Fluxo {
        id: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        slug: row.get(2)?,
        name: row.get(3)?,
        description: row.get(4)?,
        status: column_status(row, 5)?,
        rascunho_fonte: row.get(6)?,
        revisao_atual: row
            .get::<_, Option<i64>>(7)?
            .map(|numero| numero.max(0) as u64),
        version: column_u64(row, 8)?,
    })
}
