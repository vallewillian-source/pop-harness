use poph_types::OrgId;
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{NewOrg, Org, OrgChange, Page, Status};
use crate::repos::{column_id, column_status, column_u64};

const COLUMNS: &str = "id, name, slug, status, config_json, version, created_at, updated_at";

pub struct Orgs<'a>(pub(crate) &'a Store);

impl Orgs<'_> {
    pub fn create(&self, fresh: NewOrg) -> Resultado<Org> {
        let id = OrgId::generate();
        let now = self.0.now();
        let config_json = if fresh.config_json.trim().is_empty() {
            "{}".to_owned()
        } else {
            fresh.config_json
        };
        let text_id = id.to_string();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO orgs (id, name, slug, status, config_json, version, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'active', ?4, 1, ?5, ?5)",
                params![text_id, fresh.name, fresh.slug, config_json, now],
            )
            .map_err(|error| StoreError::from_write("slug", error))?;
            Ok(())
        })?;
        self.get(id)
    }

    pub fn get(&self, id: OrgId) -> Resultado<Org> {
        let texto = id.to_string();
        let found = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS} FROM orgs WHERE id = ?1"),
                params![texto],
                row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(found, "org", &id.to_string())
    }

    pub fn by_slug(&self, slug: &str) -> Resultado<Org> {
        let procurado = slug.to_owned();
        let filter = procurado.clone();
        let found = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS} FROM orgs WHERE slug = ?1"),
                params![procurado],
                row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(found, "org", &slug_for_id(filter))
    }

    pub fn list(&self, page: Page) -> Resultado<Vec<Org>> {
        let after = page.after.map(|id| id.to_string());
        let limit = i64::from(page.limit);
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM orgs
                 WHERE (?1 IS NULL OR id > ?1)
                 ORDER BY id LIMIT ?2"
            ))?;
            let rows = statement.query_map(params![after, limit], row)?;
            let mut orgs = Vec::new();
            for row in rows {
                orgs.push(row?);
            }
            Ok(orgs)
        })
    }

    pub fn update(&self, id: OrgId, expected: u64, change: OrgChange) -> Resultado<Org> {
        let now = self.0.now();
        let text_id = id.to_string();
        let status = change.status.map(Status::as_text);
        let expected_i64 = expected as i64;
        let effect = self.0.write(move |conn| {
            conn.execute(
                "UPDATE orgs
                    SET name = COALESCE(?3, name),
                        status = COALESCE(?4, status),
                        config_json = COALESCE(?5, config_json),
                        version = version + 1,
                        updated_at = ?6
                  WHERE id = ?1 AND version = ?2",
                params![
                    text_id,
                    expected_i64,
                    change.name,
                    status,
                    change.config_json,
                    now
                ],
            )
            .map_err(|error| StoreError::from_write("slug", error))
        })?;
        if effect == 0 {
            return Err(org_conflict(self.0, id, expected));
        }
        self.get(id)
    }

    pub fn delete(&self, id: OrgId) -> Resultado<()> {
        let texto = id.to_string();
        let checked = texto.clone();
        let deleted = self.0.write(move |conn| {
            let agents: i64 = conn
                .query_row(
                    "SELECT count(*) FROM agents WHERE org_id = ?1",
                    params![checked],
                    |row| row.get(0),
                )
                .map_err(StoreError::from)?;
            if agents > 0 {
                return Err(StoreError::Domain(poph_types::DomainError::Conflict {
                    entity: "org",
                    id: checked.clone(),
                    reason: format!(
                        "{agents} agente(s) na org; exclua os agentes antes, ou confirme com --force"
                    ),
                }));
            }
            conn.execute("DELETE FROM orgs WHERE id = ?1", params![checked])
                .map_err(StoreError::from)
        })?;
        if deleted == 0 {
            return Err(StoreError::Domain(poph_types::DomainError::NotFound {
                entity: "org",
                id: id.to_string(),
            }));
        }
        Ok(())
    }

    pub fn delete_forced(&self, id: OrgId) -> Resultado<u64> {
        let texto = id.to_string();
        let checked = texto.clone();
        let (deleted, agents) = self.0.write(move |conn| {
            let agents: i64 = conn
                .query_row(
                    "SELECT count(*) FROM agents WHERE org_id = ?1",
                    params![checked],
                    |row| row.get(0),
                )
                .map_err(StoreError::from)?;
            let deleted = conn
                .execute("DELETE FROM orgs WHERE id = ?1", params![checked])
                .map_err(StoreError::from)?;
            Ok((deleted, agents))
        })?;
        if deleted == 0 {
            return Err(StoreError::Domain(poph_types::DomainError::NotFound {
                entity: "org",
                id: texto,
            }));
        }
        Ok(agents.max(0) as u64)
    }
}

fn org_conflict(store: &Store, id: OrgId, expected: u64) -> StoreError {
    let texto = id.to_string();
    let current: Result<Option<i64>, _> = store.read({
        let texto = texto.clone();
        move |conn| {
            conn.query_row(
                "SELECT version FROM orgs WHERE id = ?1",
                params![texto],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::from)
        }
    });
    match current {
        Ok(Some(current)) => StoreError::Domain(poph_types::DomainError::VersionConflict {
            entity: "org",
            id: texto,
            expected,
            current: current.max(0) as u64,
        }),
        _ => StoreError::Domain(poph_types::DomainError::NotFound {
            entity: "org",
            id: texto,
        }),
    }
}

fn slug_for_id(slug: String) -> String {
    format!("slug `{slug}`")
}

fn row(row: &Row<'_>) -> rusqlite::Result<Org> {
    Ok(Org {
        id: column_id(row, 0)?,
        name: row.get(1)?,
        slug: row.get(2)?,
        status: column_status(row, 3)?,
        config_json: row.get(4)?,
        version: column_u64(row, 5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}
