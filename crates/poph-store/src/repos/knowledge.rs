use poph_types::{AgentId, KnowledgeGroupId, KnowledgeItemId, OrgId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{
    IndexItem, KnowledgeGroup, KnowledgeGroupChange, KnowledgeItem, KnowledgeItemChange,
    MAX_ITEM_CONTENT, NewKnowledgeGroup, NewKnowledgeItem, Page, Status,
};
use crate::repos::{belongs, column_id, column_status, column_u64, conflict_or_missing};

const COLUMNS_GROUP: &str =
    "id, org_id, name, description, status, version, created_at, updated_at";
const FIELDS_ITEM: &[(&str, &str)] = &[
    ("title", "título do item"),
    ("content", "conteúdo do item"),
    ("status", "status do item"),
];

const COLUMNS_ITEM: &str = "id, org_id, group_id, title, content, tags_json, position, status,
    version, created_at, updated_at";

pub struct Knowledge<'a>(pub(crate) &'a Store);

impl Knowledge<'_> {
    pub fn create_group(&self, org: OrgId, new: NewKnowledgeGroup) -> Resultado<KnowledgeGroup> {
        let nome = validate_name("nome do grupo", &new.name, 120)?;
        let id = KnowledgeGroupId::generate();
        let now = self.0.now();
        let text_id = id.to_string();
        let text_org = org.to_string();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO knowledge_groups
                    (id, org_id, name, description, status, version, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'active', 1, ?5, ?5)",
                params![text_id, text_org, nome, new.description.trim(), now],
            )
            .map_err(|error| StoreError::from_write("nome do grupo", error))?;
            Ok(())
        })?;
        self.get_group(org, id)
    }

    pub fn get_group(&self, org: OrgId, id: KnowledgeGroupId) -> Resultado<KnowledgeGroup> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let located = self.0.read(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUMNS_GROUP} FROM knowledge_groups WHERE id = ?1 AND org_id = ?2"
                ),
                params![text_id, text_org],
                group_row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(located, "grupo de conhecimento", &id.to_string())
    }

    pub fn list_groups(&self, org: OrgId, page: Page) -> Resultado<Vec<KnowledgeGroup>> {
        let text_org = org.to_string();
        let after = page.after.map(|id| id.to_string());
        let limit = i64::from(page.limit);
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS_GROUP} FROM knowledge_groups
                  WHERE org_id = ?1 AND (?2 IS NULL OR id > ?2)
                  ORDER BY id LIMIT ?3"
            ))?;
            let rows = statement.query_map(params![text_org, after, limit], group_row)?;
            let mut groups = Vec::new();
            for row in rows {
                groups.push(row?);
            }
            Ok(groups)
        })
    }

    pub fn update_group(
        &self,
        org: OrgId,
        id: KnowledgeGroupId,
        expected: u64,
        change: KnowledgeGroupChange,
    ) -> Resultado<KnowledgeGroup> {
        let nome = match &change.name {
            Some(nome) => Some(validate_name("nome do grupo", nome, 120)?),
            None => None,
        };
        let now = self.0.now();
        let text_org = org.to_string();
        let text_id = id.to_string();
        let status = change.status.map(Status::as_text);
        let expected_i64 = expected as i64;
        let effect = self.0.write(move |conn| {
            conn.execute(
                "UPDATE knowledge_groups
                    SET name = COALESCE(?3, name),
                        description = COALESCE(?4, description),
                        status = COALESCE(?5, status),
                        version = version + 1,
                        updated_at = ?6
                  WHERE org_id = ?1 AND id = ?2 AND version = ?7",
                params![
                    text_org,
                    text_id,
                    nome,
                    change.description.map(|texto| texto.trim().to_owned()),
                    status,
                    now,
                    expected_i64
                ],
            )
            .map_err(|error| StoreError::from_write("nome do grupo", error))
        })?;
        if effect == 0 {
            return Err(self.conflict_group(org, id, expected));
        }
        self.get_group(org, id)
    }

    pub fn delete_group(&self, org: OrgId, id: KnowledgeGroupId) -> Resultado<()> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let deleted = self.0.write(move |conn| {
            conn.execute(
                "DELETE FROM knowledge_groups WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id],
            )
            .map_err(StoreError::from)
        })?;
        if deleted == 0 {
            return Err(not_found("grupo de conhecimento", &id.to_string()));
        }
        Ok(())
    }

    pub fn list_items_by_id(
        &self,
        org: OrgId,
        group: KnowledgeGroupId,
        page: Page,
    ) -> Resultado<Vec<KnowledgeItem>> {
        let text_org = org.to_string();
        let text_group = group.to_string();
        let after = page.after.map(|id| id.to_string());
        let limit = i64::from(page.limit);
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS_ITEM} FROM knowledge_items
                 WHERE org_id = ?1 AND group_id = ?2 AND (?3 IS NULL OR id > ?3)
                 ORDER BY id LIMIT ?4"
            ))?;
            let rows =
                statement.query_map(params![text_org, text_group, after, limit], item_row)?;
            let mut items = Vec::new();
            for row in rows {
                items.push(row?);
            }
            Ok(items)
        })
    }

    pub fn create_item(&self, org: OrgId, new: NewKnowledgeItem) -> Resultado<KnowledgeItem> {
        let title = validate_name("título do item", &new.title, 200)?;
        let content = validate_content(&new.content)?;
        let tags = serde_json::to_string(&new.tags).unwrap_or_else(|_| "[]".to_owned());
        let id = KnowledgeItemId::generate();
        let now = self.0.now();
        let text_id = id.to_string();
        let text_org = org.to_string();
        let group = new.group_id.to_string();
        self.0.write(move |conn| {
            belongs(
                conn,
                "knowledge_groups",
                "grupo de conhecimento",
                &text_org,
                &group,
            )?;
            conn.execute(
                "INSERT INTO knowledge_items
                    (id, org_id, group_id, title, content, tags_json, source_type, position, status,
                     version, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'manual', ?7, 'active', 1, ?8, ?8)",
                params![
                    text_id,
                    text_org,
                    group,
                    title,
                    content,
                    tags,
                    new.position,
                    now
                ],
            )
            .map_err(|error| StoreError::from_write_with("título do item", FIELDS_ITEM, error))?;
            Ok(())
        })?;
        self.get_item(org, id)
    }

    pub fn get_item(&self, org: OrgId, id: KnowledgeItemId) -> Resultado<KnowledgeItem> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let located = self.0.read(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUMNS_ITEM} FROM knowledge_items WHERE id = ?1 AND org_id = ?2"
                ),
                params![text_id, text_org],
                item_row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(located, "item de conhecimento", &id.to_string())
    }

    pub fn list_items(
        &self,
        org: OrgId,
        group: KnowledgeGroupId,
        page: Page,
    ) -> Resultado<Vec<KnowledgeItem>> {
        let text_org = org.to_string();
        let text_group = group.to_string();
        let after = page.after.map(|id| id.to_string());
        let limit = i64::from(page.limit);
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS_ITEM} FROM knowledge_items
                  WHERE org_id = ?1 AND group_id = ?2 AND (?3 IS NULL OR id > ?3)
                  ORDER BY position, id LIMIT ?4"
            ))?;
            let rows =
                statement.query_map(params![text_org, text_group, after, limit], item_row)?;
            let mut items = Vec::new();
            for row in rows {
                items.push(row?);
            }
            Ok(items)
        })
    }

    pub fn update_item(
        &self,
        org: OrgId,
        id: KnowledgeItemId,
        expected: u64,
        change: KnowledgeItemChange,
    ) -> Resultado<KnowledgeItem> {
        let title = match &change.title {
            Some(title) => Some(validate_name("título do item", title, 200)?),
            None => None,
        };
        let content = match &change.content {
            Some(content) => Some(validate_content(content)?),
            None => None,
        };
        let tags = change
            .tags
            .as_ref()
            .map(|tags| serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_owned()));
        let group = change.group_id.map(|group| group.to_string());
        let status = change.status.map(Status::as_text);
        let now = self.0.now();
        let text_org = org.to_string();
        let text_id = id.to_string();
        let expected_i64 = expected as i64;
        let effect = self.0.write(move |conn| {
            if let Some(group) = &group {
                belongs(
                    conn,
                    "knowledge_groups",
                    "grupo de conhecimento",
                    &text_org,
                    group,
                )?;
            }
            conn.execute(
                "UPDATE knowledge_items
                    SET group_id = COALESCE(?3, group_id),
                        title = COALESCE(?4, title),
                        content = COALESCE(?5, content),
                        tags_json = COALESCE(?6, tags_json),
                        position = COALESCE(?7, position),
                        status = COALESCE(?8, status),
                        version = version + 1,
                        updated_at = ?9
                  WHERE org_id = ?1 AND id = ?2 AND version = ?10",
                params![
                    text_org,
                    text_id,
                    group,
                    title,
                    content,
                    tags,
                    change.position,
                    status,
                    now,
                    expected_i64
                ],
            )
            .map_err(|error| StoreError::from_write_with("título do item", FIELDS_ITEM, error))
        })?;
        if effect == 0 {
            return Err(self.conflict_item(org, id, expected));
        }
        self.get_item(org, id)
    }

    pub fn delete_item(&self, org: OrgId, id: KnowledgeItemId) -> Resultado<()> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let deleted = self.0.write(move |conn| {
            conn.execute(
                "DELETE FROM knowledge_items WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id],
            )
            .map_err(StoreError::from)
        })?;
        if deleted == 0 {
            return Err(not_found("item de conhecimento", &id.to_string()));
        }
        Ok(())
    }

    pub fn link_agent(&self, org: OrgId, agent: AgentId, group: KnowledgeGroupId) -> Resultado<()> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        let text_group = group.to_string();
        self.0.write(move |conn| {
            belongs(conn, "agents", "agente", &text_org, &text_agent)?;
            belongs(
                conn,
                "knowledge_groups",
                "grupo de conhecimento",
                &text_org,
                &text_group,
            )?;
            conn.execute(
                "INSERT INTO agent_knowledge_groups (org_id, agent_id, group_id, version, created_at)
                 VALUES (?1, ?2, ?3, 1, ?4)
                 ON CONFLICT (agent_id, group_id) DO NOTHING",
                params![text_org, text_agent, text_group, now],
            )
            .map_err(StoreError::from)?;
            Ok(())
        })?;
        Ok(())
    }

    pub fn unlink_agent(
        &self,
        org: OrgId,
        agent: AgentId,
        group: KnowledgeGroupId,
    ) -> Resultado<bool> {
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        let text_group = group.to_string();
        let deleted = self.0.write(move |conn| {
            conn.execute(
                "DELETE FROM agent_knowledge_groups
                  WHERE org_id = ?1 AND agent_id = ?2 AND group_id = ?3",
                params![text_org, text_agent, text_group],
            )
            .map_err(StoreError::from)
        })?;
        Ok(deleted > 0)
    }

    pub fn agent_groups(&self, org: OrgId, agent: AgentId) -> Resultado<Vec<KnowledgeGroup>> {
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {} FROM knowledge_groups g
                   JOIN agent_knowledge_groups v
                     ON v.org_id = g.org_id AND v.group_id = g.id
                  WHERE g.org_id = ?1 AND v.agent_id = ?2
                  ORDER BY lower(g.name), g.id",
                prefixar("g", COLUMNS_GROUP)
            ))?;
            let rows = statement.query_map(params![text_org, text_agent], group_row)?;
            let mut groups = Vec::new();
            for row in rows {
                groups.push(row?);
            }
            Ok(groups)
        })
    }

    pub fn agents_from_group(
        &self,
        org: OrgId,
        group: KnowledgeGroupId,
    ) -> Resultado<Vec<AgentId>> {
        let text_org = org.to_string();
        let text_group = group.to_string();
        self.0.read(move |conn| {
            let mut statement = conn.prepare(
                "SELECT agent_id FROM agent_knowledge_groups
                  WHERE org_id = ?1 AND group_id = ?2 ORDER BY agent_id",
            )?;
            let rows = statement.query_map(params![text_org, text_group], |row| {
                column_id::<AgentId>(row, 0)
            })?;
            let mut agents = Vec::new();
            for row in rows {
                agents.push(row?);
            }
            Ok(agents)
        })
    }

    pub fn agent_items(&self, org: OrgId, agent: AgentId) -> Resultado<Vec<IndexItem>> {
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        self.0.read(move |conn| {
            let mut statement = conn.prepare(
                "SELECT i.id, i.group_id, g.name, i.title, i.content, i.version
                   FROM knowledge_items i
                   JOIN knowledge_groups g
                     ON g.org_id = i.org_id AND g.id = i.group_id
                   JOIN agent_knowledge_groups v
                     ON v.org_id = g.org_id AND v.group_id = g.id
                  WHERE i.org_id = ?1 AND v.agent_id = ?2
                    AND i.status = 'active' AND g.status = 'active'
                  ORDER BY lower(g.name), i.position, i.id",
            )?;
            let rows = statement.query_map(params![text_org, text_agent], |row| {
                Ok(IndexItem {
                    id: column_id(row, 0)?,
                    group_id: column_id(row, 1)?,
                    group: row.get(2)?,
                    title: row.get(3)?,
                    content: row.get(4)?,
                    version: column_u64(row, 5)?,
                })
            })?;
            let mut items = Vec::new();
            for row in rows {
                items.push(row?);
            }
            Ok(items)
        })
    }

    pub fn group_name_of_item(
        &self,
        org: OrgId,
        item: KnowledgeItemId,
    ) -> Resultado<Option<String>> {
        let text_org = org.to_string();
        let text_item = item.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT g.name FROM knowledge_items i
                   JOIN knowledge_groups g ON g.org_id = i.org_id AND g.id = i.group_id
                  WHERE i.org_id = ?1 AND i.id = ?2",
                params![text_org, text_item],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::from)
        })
    }

    fn conflict_group(&self, org: OrgId, id: KnowledgeGroupId, expected: u64) -> StoreError {
        self.0
            .read(move |conn| {
                Ok(conflict_or_missing(
                    conn,
                    "knowledge_groups",
                    "grupo de conhecimento",
                    &id.to_string(),
                    &org.to_string(),
                    expected,
                ))
            })
            .unwrap_or_else(|error| error)
    }

    fn conflict_item(&self, org: OrgId, id: KnowledgeItemId, expected: u64) -> StoreError {
        self.0
            .read(move |conn| {
                Ok(conflict_or_missing(
                    conn,
                    "knowledge_items",
                    "item de conhecimento",
                    &id.to_string(),
                    &org.to_string(),
                    expected,
                ))
            })
            .unwrap_or_else(|error| error)
    }
}

fn prefixar(apelido: &str, columns: &str) -> String {
    columns
        .split(',')
        .map(|column| format!("{apelido}.{}", column.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn validate_name(campo: &'static str, texto: &str, limit: usize) -> Resultado<String> {
    let clean_value = texto.trim();
    if clean_value.is_empty() {
        return Err(StoreError::Domain(poph_types::DomainError::Validation {
            field: campo,
            reason: "não pode ficar em branco".to_owned(),
        }));
    }
    if clean_value.chars().count() > limit {
        return Err(StoreError::Domain(poph_types::DomainError::Validation {
            field: campo,
            reason: format!(
                "até {limit} caracteres; vieram {}",
                clean_value.chars().count()
            ),
        }));
    }
    Ok(clean_value.to_owned())
}

fn validate_content(texto: &str) -> Resultado<String> {
    let clean_value = texto.trim();
    let characters = clean_value.chars().count();
    if characters == 0 {
        return Err(StoreError::Domain(poph_types::DomainError::Validation {
            field: "conteúdo do item",
            reason: "não pode ficar em branco".to_owned(),
        }));
    }
    if characters > MAX_ITEM_CONTENT {
        return Err(StoreError::Domain(poph_types::DomainError::Validation {
            field: "conteúdo do item",
            reason: format!("até {MAX_ITEM_CONTENT} caracteres; vieram {characters}"),
        }));
    }
    Ok(clean_value.to_owned())
}

fn not_found(entity: &'static str, id: &str) -> StoreError {
    StoreError::Domain(poph_types::DomainError::NotFound {
        entity,
        id: id.to_owned(),
    })
}

fn group_row(row: &Row<'_>) -> rusqlite::Result<KnowledgeGroup> {
    Ok(KnowledgeGroup {
        id: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        status: column_status(row, 4)?,
        version: column_u64(row, 5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn item_row(row: &Row<'_>) -> rusqlite::Result<KnowledgeItem> {
    let tags: String = row.get(5)?;
    Ok(KnowledgeItem {
        id: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        group_id: column_id(row, 2)?,
        title: row.get(3)?,
        content: row.get(4)?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        position: row.get(6)?,
        status: column_status(row, 7)?,
        version: column_u64(row, 8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}
