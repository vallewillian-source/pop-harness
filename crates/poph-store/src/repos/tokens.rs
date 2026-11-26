use poph_types::{AdminTokenId, OrgId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{Page, TokenAdmin};
use crate::repos::{column_id, id_optional};

const COLUMNS: &str = "id, org_id, label, token_hash, created_at, revoked_at";

pub struct Tokens<'a>(pub(crate) &'a Store);

impl Tokens<'_> {
    pub fn create(&self, org: Option<OrgId>, label: &str, hash: &str) -> Resultado<TokenAdmin> {
        let id = AdminTokenId::generate();
        let now = self.0.now();
        let text_id = id.to_string();
        let text_org = org.map(|org| org.to_string());
        let label = label.to_owned();
        let hash = hash.to_owned();
        let kept = label.clone();
        let stored_hash = hash.clone();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO admin_tokens (id, org_id, label, token_hash, created_at, revoked_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
                params![text_id, text_org, label, hash, now],
            )
            .map_err(|error| StoreError::from_write("token_hash", error))?;
            Ok(())
        })?;
        Ok(TokenAdmin {
            id,
            org_id: org,
            label: kept,
            hash: stored_hash,
            created_at: now,
            revoked_at: None,
        })
    }

    pub fn get(&self, org: OrgId, id: AdminTokenId) -> Resultado<TokenAdmin> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let located = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS} FROM admin_tokens WHERE org_id = ?1 AND id = ?2"),
                params![text_org, text_id],
                row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(located, "token de administração", &id.to_string())
    }

    pub fn by_hash(&self, hash: &str) -> Resultado<Option<TokenAdmin>> {
        let hash = hash.to_owned();
        self.0.read(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUMNS} FROM admin_tokens
                      WHERE token_hash = ?1 AND revoked_at IS NULL"
                ),
                params![hash],
                row,
            )
            .optional()
            .map_err(StoreError::from)
        })
    }

    pub fn by_id(&self, id: AdminTokenId) -> Resultado<Option<TokenAdmin>> {
        let text_id = id.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS} FROM admin_tokens WHERE id = ?1"),
                params![text_id],
                row,
            )
            .optional()
            .map_err(StoreError::from)
        })
    }

    pub fn list(&self, org: OrgId, page: Page) -> Resultado<Vec<TokenAdmin>> {
        let text_org = org.to_string();
        let after = page.after.map(|id| id.to_string());
        let limit = i64::from(page.limit);
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM admin_tokens
                  WHERE org_id = ?1 AND (?2 IS NULL OR id > ?2)
                  ORDER BY id LIMIT ?3"
            ))?;
            let rows = statement.query_map(params![text_org, after, limit], row)?;
            let mut tokens = Vec::new();
            for row in rows {
                tokens.push(row?);
            }
            Ok(tokens)
        })
    }

    pub fn revoke(&self, org: OrgId, id: AdminTokenId) -> Resultado<()> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_id = id.to_string();
        self.0.write(move |conn| {
            conn.execute(
                "UPDATE admin_tokens SET revoked_at = COALESCE(revoked_at, ?3)
                  WHERE id = ?1 AND org_id = ?2",
                params![text_id, text_org, now],
            )
            .map(|_| ())
            .map_err(StoreError::from)
        })?;
        Ok(())
    }
}

fn row(row: &Row<'_>) -> rusqlite::Result<TokenAdmin> {
    Ok(TokenAdmin {
        id: column_id(row, 0)?,
        org_id: id_optional(row, 1)?,
        label: row.get(2)?,
        hash: row.get(3)?,
        created_at: row.get(4)?,
        revoked_at: row.get(5)?,
    })
}
