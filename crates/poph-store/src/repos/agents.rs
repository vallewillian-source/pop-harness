use std::collections::HashMap;
use std::path::Path;

use poph_types::{AgentId, OrgId, ProductCategoryId};
use rusqlite::{OptionalExtension, Row, named_params, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{AgentChange, Agente, NewAgent, Page, Status};
use crate::repos::{
    belongs, column_id, column_status, column_u32, column_u64, conflict_or_missing, id_optional,
};

pub fn agents_by_provider_at(path: &Path) -> Resultado<Vec<(String, u64)>> {
    let conn = crate::connection::open_for_read(path)?;
    let mut statement =
        conn.prepare("SELECT provider, COUNT(*) FROM agents GROUP BY provider ORDER BY provider")?;
    let rows = statement.query_map([], |row| {
        let nome: String = row.get(0)?;
        let total: i64 = row.get(1)?;
        Ok((nome, total.max(0) as u64))
    })?;
    let mut count = Vec::new();
    for row in rows {
        count.push(row?);
    }
    Ok(count)
}

const FIELDS_AGENT: &[(&str, &str)] = &[
    ("name", "name"),
    ("status", "status"),
    ("agent_type", "type"),
    ("category", "category"),
    ("human_handoff_enabled", "handoff"),
    ("provider", "provider"),
    ("model", "model"),
    ("temperature", "temperature"),
    ("max_output_tokens", "max_output_tokens"),
    ("max_context_tokens", "max_context_tokens"),
    ("grouping_window_ms", "grouping_window_ms"),
    ("max_tool_rounds", "max_tool_rounds"),
    ("max_tool_calls_per_message", "max_tool_calls"),
    ("rag_top_k", "rag_top_k"),
    ("rag_min_score", "rag_min_score"),
];

const COLUMNS: &str = "id, org_id, name, slug, description, status, agent_type, category,
    default_department_id, human_handoff_enabled, human_handoff_department_id,
    provider, model, temperature, max_output_tokens,
    max_context_messages, max_context_tokens, grouping_window_ms,
    max_tool_rounds, max_tool_calls_per_message,
    rag_top_k, rag_min_score, jev_json, version, created_at, updated_at";

pub struct Agents<'a>(pub(crate) &'a Store);

impl Agents<'_> {
    pub fn create(&self, org: OrgId, new: NewAgent) -> Resultado<Agente> {
        let id = AgentId::generate();
        let now = self.0.now();
        let text_id = id.to_string();
        let text_org = org.to_string();
        self.0.write(move |conn| {
            if let Some(departamento) = new.default_department {
                belongs(
                    conn,
                    "departments",
                    "departamento",
                    &text_org,
                    &departamento.to_string(),
                )?;
            }
            conn.execute(
                "INSERT INTO agents (
                    id, org_id, name, slug, description, status, agent_type, category,
                    default_department_id, human_handoff_enabled, human_handoff_department_id,
                    provider, model, temperature, max_output_tokens,
                    max_context_messages, max_context_tokens, grouping_window_ms,
                    max_tool_rounds, max_tool_calls_per_message,
                    rag_top_k, rag_min_score, jev_json, version, created_at, updated_at
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7,
                    ?8, 0, NULL,
                    ?9, ?10, ?11, ?12,
                    30, ?13, ?14,
                    ?15, ?16,
                    ?17, ?18, '{}', 1, ?19, ?19
                 )",
                params![
                    text_id,
                    text_org,
                    new.name,
                    new.slug,
                    new.description,
                    new.agent_type,
                    new.category,
                    new.default_department.map(|dep| dep.to_string()),
                    new.provedor,
                    new.modelo,
                    new.temperatura,
                    new.max_tokens_saida,
                    new.max_context_tokens,
                    new.grouping_window_ms,
                    new.max_tool_rounds,
                    new.max_tools_per_message,
                    new.rag_top_k,
                    new.rag_cutoff,
                    now
                ],
            )
            .map_err(|error| StoreError::from_write_with("slug", FIELDS_AGENT, error))?;
            Ok(())
        })?;
        self.get(org, id)
    }

    pub fn get(&self, org: OrgId, id: AgentId) -> Resultado<Agente> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let located = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS} FROM agents WHERE id = ?1 AND org_id = ?2"),
                params![text_id, text_org],
                row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(located, "agente", &id.to_string())
    }

    pub fn by_slug(&self, org: OrgId, slug: &str) -> Resultado<Agente> {
        let text_org = org.to_string();
        let procurado = slug.to_owned();
        let label = slug.to_owned();
        let located = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS} FROM agents WHERE org_id = ?1 AND slug = ?2"),
                params![text_org, procurado],
                row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(located, "agente", &format!("slug `{label}`"))
    }

    pub fn version(&self, org: OrgId, id: AgentId) -> Resultado<u64> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let found = self.0.read(move |conn| {
            conn.query_row(
                "SELECT version FROM agents WHERE id = ?1 AND org_id = ?2",
                params![text_id, text_org],
                |row| column_u64(row, 0),
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(found, "agente", &id.to_string())
    }

    pub fn list(&self, org: OrgId, page: Page) -> Resultado<Vec<Agente>> {
        let text_org = org.to_string();
        let after = page.after.map(|id| id.to_string());
        let limit = i64::from(page.limit);
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM agents
                  WHERE org_id = ?1 AND (?2 IS NULL OR id > ?2)
                  ORDER BY id LIMIT ?3"
            ))?;
            let rows = statement.query_map(params![text_org, after, limit], row)?;
            let mut agents = Vec::new();
            for row in rows {
                agents.push(row?);
            }
            Ok(agents)
        })
    }

    pub fn names(
        &self,
        org: OrgId,
        ids: &[AgentId],
    ) -> Resultado<std::collections::HashMap<AgentId, String>> {
        if ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let text_org = org.to_string();
        let list = serde_json::to_string(&ids.iter().map(ToString::to_string).collect::<Vec<_>>())
            .unwrap_or_else(|_| "[]".to_owned());
        self.0.read(move |conn| {
            let mut statement = conn.prepare(
                "SELECT id, name FROM agents
                  WHERE org_id = ?1 AND id IN (SELECT value FROM json_each(?2))",
            )?;
            let rows = statement.query_map(params![text_org, list], |row| {
                Ok((column_id::<AgentId>(row, 0)?, row.get::<_, String>(1)?))
            })?;
            let mut names = std::collections::HashMap::new();
            for row in rows {
                let (id, nome) = row?;
                names.insert(id, nome);
            }
            Ok(names)
        })
    }

    pub fn count_active_by_org(&self, orgs: &[OrgId]) -> Resultado<HashMap<OrgId, u32>> {
        if orgs.is_empty() {
            return Ok(HashMap::new());
        }
        let list = serde_json::to_string(&orgs.iter().map(ToString::to_string).collect::<Vec<_>>())
            .unwrap_or_else(|_| "[]".to_owned());
        self.0.read(move |conn| {
            let mut statement = conn.prepare(
                "SELECT org_id, count(*) FROM agents
                  WHERE status = 'active' AND org_id IN (SELECT value FROM json_each(?1))
                  GROUP BY org_id",
            )?;
            let rows = statement.query_map(params![list], |row| {
                Ok((column_id::<OrgId>(row, 0)?, column_u32(row, 1)?))
            })?;
            let mut count = HashMap::new();
            for row in rows {
                let (org, total) = row?;
                count.insert(org, total);
            }
            Ok(count)
        })
    }

    pub fn get_for_turn(&self, org: OrgId, id: AgentId) -> Resultado<Agente> {
        let agent = self.get(org, id)?;
        if !agent.can_open_turn() {
            return Err(StoreError::Domain(poph_types::DomainError::AgentInactive {
                id: agent.id.to_string(),
                status: agent.status.as_text().to_owned(),
            }));
        }
        Ok(agent)
    }

    pub fn list_active(&self, org: OrgId) -> Resultado<Vec<Agente>> {
        let text_org = org.to_string();
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM agents WHERE org_id = ?1 AND status = 'active' ORDER BY id"
            ))?;
            let rows = statement.query_map(params![text_org], row)?;
            let mut agents = Vec::new();
            for row in rows {
                agents.push(row?);
            }
            Ok(agents)
        })
    }

    pub fn update(
        &self,
        org: OrgId,
        id: AgentId,
        expected: u64,
        change: AgentChange,
    ) -> Resultado<Agente> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_id = id.to_string();
        let status = change.status.map(Status::as_text);
        let handoff_active = change.handoff_active.map(i64::from);
        let expected_i64 = expected as i64;
        let effect = self.0.write(move |conn| {
            for departamento in [change.default_department, change.handoff_department]
                .into_iter()
                .flatten()
            {
                belongs(
                    conn,
                    "departments",
                    "departamento",
                    &text_org,
                    &departamento.to_string(),
                )?;
            }
            conn.execute(
                "UPDATE agents SET
                    name = COALESCE(:nome, name),
                    description = COALESCE(:descricao, description),
                    status = COALESCE(:status, status),
                    default_department_id = COALESCE(:departamento_padrao, default_department_id),
                    human_handoff_enabled = COALESCE(:handoff_ativo, human_handoff_enabled),
                    human_handoff_department_id = COALESCE(:handoff_departamento, human_handoff_department_id),
                    provider = COALESCE(:provedor, provider),
                    model = COALESCE(:modelo, model),
                    temperature = COALESCE(:temperatura, temperature),
                    max_output_tokens = COALESCE(:max_tokens_saida, max_output_tokens),
                    max_context_tokens = COALESCE(:max_context_tokens, max_context_tokens),
                    grouping_window_ms = COALESCE(:janela, grouping_window_ms),
                    max_tool_rounds = COALESCE(:rodadas, max_tool_rounds),
                    max_tool_calls_per_message = COALESCE(:calls, max_tool_calls_per_message),
                    rag_top_k = COALESCE(:top_k, rag_top_k),
                    rag_min_score = COALESCE(:corte, rag_min_score),
                    jev_json = COALESCE(:jev, jev_json),
                    version = version + 1,
                    updated_at = :agora
                  WHERE id = :id AND org_id = :org AND version = :esperada",
                named_params! {
                    ":nome": change.name,
                    ":descricao": change.description,
                    ":status": status,
                    ":departamento_padrao": change.default_department.map(|dep| dep.to_string()),
                    ":handoff_ativo": handoff_active,
                    ":handoff_departamento": change.handoff_department.map(|dep| dep.to_string()),
                    ":provedor": change.provedor,
                    ":modelo": change.modelo,
                    ":temperatura": change.temperatura,
                    ":max_tokens_saida": change.max_tokens_saida,
                    ":max_context_tokens": change.max_context_tokens.map(i64::from),
                    ":janela": change.grouping_window_ms.map(i64::from),
                    ":rodadas": change.max_tool_rounds.map(i64::from),
                    ":calls": change.max_tools_per_message.map(i64::from),
                    ":top_k": change.rag_top_k.map(i64::from),
                    ":corte": change.rag_cutoff,
                    ":jev": change.jev_json,
                    ":agora": now,
                    ":id": text_id,
                    ":org": text_org,
                    ":esperada": expected_i64,
                },
            )
            .map_err(|error| StoreError::from_write_with("slug", FIELDS_AGENT, error))
        })?;
        if effect == 0 {
            return Err(self.conflict(org, id, expected));
        }
        self.get(org, id)
    }

    pub fn delete(&self, org: OrgId, id: AgentId) -> Resultado<()> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let deleted = self.0.write(move |conn| {
            conn.execute(
                "DELETE FROM agents WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id],
            )
            .map_err(StoreError::from)
        })?;
        if deleted == 0 {
            return Err(not_found(id));
        }
        Ok(())
    }

    pub fn define_categories(
        &self,
        org: OrgId,
        id: AgentId,
        expected: u64,
        categories: &[ProductCategoryId],
    ) -> Resultado<Agente> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_id = id.to_string();
        let categories: Vec<String> = categories.iter().map(ToString::to_string).collect();
        let expected_i64 = expected as i64;
        self.0.write(move |conn| {
            for category in &categories {
                belongs(
                    conn,
                    "product_categories",
                    "categoria de produto",
                    &text_org,
                    category,
                )?;
            }
            let effect = conn
                .execute(
                    "UPDATE agents SET version = version + 1, updated_at = ?3
                      WHERE id = ?1 AND org_id = ?2 AND version = ?4",
                    params![text_id, text_org, now, expected_i64],
                )
                .map_err(StoreError::from)?;
            if effect == 0 {
                return Err(conflict_or_missing(
                    conn, "agents", "agente", &text_id, &text_org, expected,
                ));
            }
            conn.execute(
                "DELETE FROM agent_product_categories WHERE org_id = ?1 AND agent_id = ?2",
                params![text_org, text_id],
            )
            .map_err(StoreError::from)?;
            for category in &categories {
                conn.execute(
                    "INSERT INTO agent_product_categories (org_id, agent_id, category_id)
                     VALUES (?1, ?2, ?3)",
                    params![text_org, text_id, category],
                )
                .map_err(StoreError::from)?;
            }
            Ok(())
        })?;
        self.get(org, id)
    }

    pub fn categories(&self, org: OrgId, id: AgentId) -> Resultado<Vec<ProductCategoryId>> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        self.0.read(move |conn| {
            let mut statement = conn.prepare(
                "SELECT category_id FROM agent_product_categories
                  WHERE org_id = ?1 AND agent_id = ?2 ORDER BY category_id",
            )?;
            let rows = statement.query_map(params![text_org, text_id], |row| {
                column_id::<ProductCategoryId>(row, 0)
            })?;
            let mut categories = Vec::new();
            for row in rows {
                categories.push(row?);
            }
            Ok(categories)
        })
    }

    fn conflict(&self, org: OrgId, id: AgentId, expected: u64) -> StoreError {
        let text_org = org.to_string();
        let text_id = id.to_string();
        self.0
            .read(move |conn| {
                Ok(conflict_or_missing(
                    conn, "agents", "agente", &text_id, &text_org, expected,
                ))
            })
            .unwrap_or_else(|error| error)
    }
}

fn not_found(id: AgentId) -> StoreError {
    StoreError::Domain(poph_types::DomainError::NotFound {
        entity: "agente",
        id: id.to_string(),
    })
}

fn row(row: &Row<'_>) -> rusqlite::Result<Agente> {
    Ok(Agente {
        id: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        name: row.get(2)?,
        slug: row.get(3)?,
        description: row.get(4)?,
        status: column_status(row, 5)?,
        agent_type: row.get(6)?,
        category: row.get(7)?,
        default_department: id_optional(row, 8)?,
        handoff_active: row.get::<_, i64>(9)? != 0,
        handoff_department: id_optional(row, 10)?,
        provedor: row.get(11)?,
        modelo: row.get(12)?,
        temperatura: row.get(13)?,
        max_tokens_saida: row.get(14)?,
        max_context_messages: column_u32(row, 15)?,
        max_context_tokens: column_u32(row, 16)?,
        grouping_window_ms: column_u32(row, 17)?,
        max_tool_rounds: column_u32(row, 18)?,
        max_tools_per_message: column_u32(row, 19)?,
        rag_top_k: column_u32(row, 20)?,
        rag_cutoff: row.get(21)?,
        jev_json: row.get(22)?,
        version: column_u64(row, 23)?,
        created_at: row.get(24)?,
        updated_at: row.get(25)?,
    })
}
