use poph_types::{DepartmentId, OrgId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{Departamento, DepartmentChange, NewDepartment, Page, Status};
use crate::repos::{column_id, column_status, column_u64, conflict_or_missing};

const COLUMNS: &str = "id, org_id, name, description, status, version, created_at, updated_at";

pub struct Departments<'a>(pub(crate) &'a Store);

impl Departments<'_> {
    pub fn create(&self, org: OrgId, new: NewDepartment) -> Resultado<Departamento> {
        let id = DepartmentId::generate();
        let now = self.0.now();
        let text_id = id.to_string();
        let text_org = org.to_string();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO departments
                    (id, org_id, name, description, status, version, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'active', 1, ?5, ?5)",
                params![text_id, text_org, new.name, new.description, now],
            )
            .map_err(|error| StoreError::from_write("nome", error))?;
            Ok(())
        })?;
        self.get(org, id)
    }

    pub fn get(&self, org: OrgId, id: DepartmentId) -> Resultado<Departamento> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let located = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS} FROM departments WHERE id = ?1 AND org_id = ?2"),
                params![text_id, text_org],
                row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(located, "departamento", &id.to_string())
    }

    pub fn by_name(&self, org: OrgId, nome: &str) -> Resultado<Option<Departamento>> {
        let text_org = org.to_string();
        let nome = nome.trim().to_lowercase();
        self.0.read(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUMNS} FROM departments
                      WHERE org_id = ?1 AND lower(name) = ?2"
                ),
                params![text_org, nome],
                row,
            )
            .optional()
            .map_err(StoreError::from)
        })
    }

    pub fn list(&self, org: OrgId, page: Page) -> Resultado<Vec<Departamento>> {
        let text_org = org.to_string();
        let after = page.after.map(|id| id.to_string());
        let limit = i64::from(page.limit);
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM departments
                  WHERE org_id = ?1 AND (?2 IS NULL OR id > ?2)
                  ORDER BY id LIMIT ?3"
            ))?;
            let rows = statement.query_map(params![text_org, after, limit], row)?;
            let mut departments = Vec::new();
            for row in rows {
                departments.push(row?);
            }
            Ok(departments)
        })
    }

    pub fn update(
        &self,
        org: OrgId,
        id: DepartmentId,
        expected: u64,
        change: DepartmentChange,
    ) -> Resultado<Departamento> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_id = id.to_string();
        let status = change.status.map(Status::as_text);
        let expected_i64 = expected as i64;
        let effect = self.0.write(move |conn| {
            conn.execute(
                "UPDATE departments
                    SET name = COALESCE(?3, name),
                        description = COALESCE(?4, description),
                        status = COALESCE(?5, status),
                        version = version + 1,
                        updated_at = ?6
                  WHERE org_id = ?1 AND id = ?2 AND version = ?7",
                params![
                    text_org,
                    text_id,
                    change.name,
                    change.description,
                    status,
                    now,
                    expected_i64
                ],
            )
            .map_err(|error| StoreError::from_write("nome", error))
        })?;
        if effect == 0 {
            return Err(self.conflict(org, id, expected));
        }
        self.get(org, id)
    }

    pub fn delete(&self, org: OrgId, id: DepartmentId) -> Resultado<()> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let deleted = self.0.write(move |conn| {
            conn.execute(
                "DELETE FROM departments WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id],
            )
            .map_err(StoreError::from)
        })?;
        if deleted == 0 {
            return Err(StoreError::Domain(poph_types::DomainError::NotFound {
                entity: "departamento",
                id: id.to_string(),
            }));
        }
        Ok(())
    }

    fn conflict(&self, org: OrgId, id: DepartmentId, expected: u64) -> StoreError {
        let text_org = org.to_string();
        let text_id = id.to_string();
        self.0
            .read(move |conn| {
                Ok(conflict_or_missing(
                    conn,
                    "departments",
                    "departamento",
                    &text_id,
                    &text_org,
                    expected,
                ))
            })
            .unwrap_or_else(|error| error)
    }
}

fn row(row: &Row<'_>) -> rusqlite::Result<Departamento> {
    Ok(Departamento {
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
