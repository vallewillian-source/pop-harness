use poph_types::{AgentId, FileId, OrgId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{LibraryFile, NewLibraryFile, Page};
use crate::repos::{belongs, column_id, require};

const COLUMNS: &str =
    "id, org_id, agent_id, name, description, mime, size_bytes, sha256, blob_path, created_at";

const ENTITY: &str = "arquivo da biblioteca do agente";

pub const LIMIT_NAME_FILE: usize = 200;
pub const LIMIT_DESCRIPTION_FILE: usize = 500;

pub struct Library<'a>(pub(crate) &'a Store);

impl Library<'_> {
    pub fn delete_audited(
        &self,
        org: OrgId,
        agent: AgentId,
        id: FileId,
        actor: String,
    ) -> Resultado<()> {
        let org = org.to_string();
        let agent = agent.to_string();
        let id = id.to_string();
        let audit_id = poph_types::AuditId::generate().to_string();
        let now = self.0.now();
        self.0.write(move |conn| {
            let deleted = conn.execute(
                "DELETE FROM file_library WHERE org_id = ?1 AND agent_id = ?2 AND id = ?3",
                params![org, agent, id],
            )?;
            if deleted == 0 {
                return Err(StoreError::Domain(poph_types::DomainError::NotFound {
                    entity: ENTITY,
                    id,
                }));
            }
            conn.execute(
                "INSERT INTO audit_log (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json, created_at)
                 VALUES (?1, ?2, 'admin_token', ?3, 'delete', 'file_library', ?4, '{}', ?5)",
                params![audit_id, org, actor, id, now],
            )?;
            Ok(())
        })
    }

    pub fn create(&self, org: OrgId, new: NewLibraryFile) -> Resultado<LibraryFile> {
        let nome = validate_text("nome", &new.name, LIMIT_NAME_FILE)?;
        let descricao = new.description.trim().to_owned();
        if descricao.chars().count() > LIMIT_DESCRIPTION_FILE {
            return Err(StoreError::Domain(poph_types::DomainError::Validation {
                field: "descrição",
                reason: format!("use no máximo {LIMIT_DESCRIPTION_FILE} caracteres"),
            }));
        }
        let mime = new.mime.trim().to_owned();
        if mime.is_empty() {
            return Err(StoreError::Domain(poph_types::DomainError::Validation {
                field: "mime",
                reason: "informe o tipo real do arquivo".to_owned(),
            }));
        }
        let sha256 = validate_sha256(&new.sha256)?;
        let blob_path = new.blob_path.trim().to_owned();
        if blob_path.is_empty() {
            return Err(StoreError::Domain(poph_types::DomainError::Validation {
                field: "blob_path",
                reason: "informe onde o conteúdo foi gravado".to_owned(),
            }));
        }

        let id = FileId::generate();
        let now = self.0.now();
        let text_id = id.to_string();
        let text_org = org.to_string();
        let text_agent = new.agent_id.to_string();
        let size = i64::try_from(new.size_bytes).unwrap_or(i64::MAX);
        self.0.write(move |conn| {
            belongs(conn, "agents", "agente", &text_org, &text_agent)?;
            conn.execute(
                "INSERT INTO file_library
                    (id, org_id, agent_id, name, description, mime, size_bytes, sha256, blob_path,
                     created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    text_id, text_org, text_agent, nome, descricao, mime, size, sha256, blob_path,
                    now
                ],
            )
            .map_err(|error| StoreError::from_write("nome", error))?;
            Ok(())
        })?;
        self.get(org, new.agent_id, id)
    }

    pub fn get(&self, org: OrgId, agent: AgentId, id: FileId) -> Resultado<LibraryFile> {
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        let text_id = id.to_string();
        let located = self.0.read(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUMNS} FROM file_library
                      WHERE org_id = ?1 AND agent_id = ?2 AND id = ?3"
                ),
                params![text_org, text_agent, text_id],
                row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        require(located, ENTITY, &id.to_string())
    }

    pub fn list(&self, org: OrgId, agent: AgentId, page: Page) -> Resultado<Vec<LibraryFile>> {
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        let after = page.after.map(|id| id.to_string());
        let limit = i64::from(page.limit);
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM file_library
                  WHERE org_id = ?1 AND agent_id = ?2 AND (?3 IS NULL OR id > ?3)
                  ORDER BY id LIMIT ?4"
            ))?;
            let rows = statement.query_map(params![text_org, text_agent, after, limit], row)?;
            let mut files = Vec::new();
            for row in rows {
                files.push(row?);
            }
            Ok(files)
        })
    }

    pub fn count_rows(&self, org: OrgId, agent: AgentId) -> Resultado<u64> {
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        self.0.read(move |conn| {
            let total: i64 = conn.query_row(
                "SELECT COUNT(*) FROM file_library WHERE org_id = ?1 AND agent_id = ?2",
                params![text_org, text_agent],
                |row| row.get(0),
            )?;
            Ok(total.max(0) as u64)
        })
    }

    pub fn delete(&self, org: OrgId, agent: AgentId, id: FileId) -> Resultado<()> {
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        let text_id = id.to_string();
        let deleted = self.0.write(move |conn| {
            conn.execute(
                "DELETE FROM file_library WHERE org_id = ?1 AND agent_id = ?2 AND id = ?3",
                params![text_org, text_agent, text_id],
            )
            .map_err(StoreError::from)
        })?;
        if deleted == 0 {
            return Err(StoreError::Domain(poph_types::DomainError::NotFound {
                entity: ENTITY,
                id: id.to_string(),
            }));
        }
        Ok(())
    }
}

fn row(row: &Row<'_>) -> rusqlite::Result<LibraryFile> {
    Ok(LibraryFile {
        id: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        agent_id: column_id(row, 2)?,
        name: row.get(3)?,
        description: row.get(4)?,
        mime: row.get(5)?,
        size_bytes: crate::repos::column_u64(row, 6)?,
        sha256: row.get(7)?,
        blob_path: row.get(8)?,
        created_at: row.get(9)?,
    })
}

fn validate_text(campo: &'static str, value: &str, limit: usize) -> Resultado<String> {
    let clean_value = value.trim().to_owned();
    if clean_value.is_empty() {
        return Err(StoreError::Domain(poph_types::DomainError::Validation {
            field: campo,
            reason: "não pode ficar vazio".to_owned(),
        }));
    }
    if clean_value.chars().count() > limit {
        return Err(StoreError::Domain(poph_types::DomainError::Validation {
            field: campo,
            reason: format!("use no máximo {limit} caracteres"),
        }));
    }
    Ok(clean_value)
}

fn validate_sha256(value: &str) -> Resultado<String> {
    let texto = value.trim().to_lowercase();
    let valid = texto.len() == 64 && texto.chars().all(|c| c.is_ascii_hexdigit());
    if !valid {
        return Err(StoreError::Domain(poph_types::DomainError::Validation {
            field: "sha256",
            reason: "precisa ser o hash em hexadecimal (64 caracteres)".to_owned(),
        }));
    }
    Ok(texto)
}
