use poph_types::{AttachmentId, ConversationId, DomainError, OrgId};
use rusqlite::{OptionalExtension, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::repos::{column_id, column_u32, column_u64};

pub const LIMIT_ERROR_ASSET: usize = 64;

pub const LIMIT_PENDING: u32 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryAsset {
    Image,
    Document,
}

impl CategoryAsset {
    pub fn as_text(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Document => "document",
        }
    }

    pub fn from_text(value: &str) -> rusqlite::Result<Self> {
        match value {
            "image" => Ok(Self::Image),
            "document" => Ok(Self::Document),
            another => Err(rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::other(format!(
                    "categoria de mídia do Matrix desconhecida no banco: `{another}`"
                ))),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAsset {
    pub id: AttachmentId,
    pub conversation: ConversationId,
    pub agent: String,
    pub room: String,
    pub evento: String,
    pub mxc: String,
    pub nome: String,
    pub mime: String,
    pub category: CategoryAsset,
    pub size_bytes: u64,
    pub sent_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomAsset {
    pub id: AttachmentId,
    pub org_id: OrgId,
    pub conversation: ConversationId,
    pub agent: String,
    pub room: String,
    pub evento: String,
    pub mxc: String,
    pub nome: String,
    pub mime: String,
    pub category: CategoryAsset,
    pub size_bytes: u64,
    pub sent_at: i64,
    pub draft_at: Option<i64>,
    pub deleted_at: Option<i64>,
    pub attempts: u32,
    pub next_attempt_at: i64,
    pub ultimo_erro: Option<String>,
}

impl RoomAsset {
    pub fn still_to_draft(&self) -> bool {
        self.draft_at.is_none()
    }

    pub fn still_to_delete(&self) -> bool {
        self.deleted_at.is_none()
    }
}

pub struct MatrixAsset<'a>(pub(crate) &'a Store);

impl MatrixAsset<'_> {
    pub fn register(&self, org: OrgId, asset: &NewAsset) -> Resultado<bool> {
        require_text("mídia da sala", "sala", &asset.room, 255)?;
        require_text("mídia da sala", "evento", &asset.evento, 255)?;
        require_text("mídia da sala", "agente", &asset.agent, 64)?;
        require_text("mídia da sala", "mxc", &asset.mxc, 512)?;
        let now = self.0.now();
        let text_org = org.to_string();
        let id = asset.id.to_string();
        let conversation = asset.conversation.to_string();
        let agent = asset.agent.clone();
        let room = asset.room.clone();
        let evento = asset.evento.clone();
        let mxc = asset.mxc.clone();
        let nome = asset.nome.clone();
        let mime = asset.mime.clone();
        let category = asset.category.as_text();
        let size = i64::try_from(asset.size_bytes).unwrap_or(i64::MAX);
        let sent_at = asset.sent_at;
        self.0.write(move |conn| {
            let inserted = conn
                .execute(
                    "INSERT INTO matrix_midia (
                         id, org_id, conversation_id, agent_id, room_id, event_id, mxc, name, mime,
                         kind, size_bytes, sent_at, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)
                     ON CONFLICT (id) DO NOTHING",
                    params![
                        id,
                        text_org,
                        conversation,
                        agent,
                        room,
                        evento,
                        mxc,
                        nome,
                        mime,
                        category,
                        size,
                        sent_at,
                        now,
                    ],
                )
                .map_err(|error| StoreError::from_write("mídia da sala", error))?;
            Ok(inserted == 1)
        })
    }

    pub fn pending(
        &self,
        org: OrgId,
        cutoff: i64,
        now: i64,
        limit: u32,
    ) -> Resultado<Vec<RoomAsset>> {
        let limit = limit.clamp(1, LIMIT_PENDING) as i64;
        let text_org = org.to_string();
        self.0.read(move |conn| {
            let mut statement = conn.prepare(
                "SELECT id, org_id, conversation_id, agent_id, room_id, event_id, mxc, name, mime,
                        kind, size_bytes, sent_at, redacted_at, deleted_at, attempts,
                        next_attempt_at, last_error
                   FROM matrix_midia
                  WHERE org_id = ?1
                    AND (redacted_at IS NULL OR deleted_at IS NULL)
                    AND sent_at <= ?2
                    AND next_attempt_at <= ?3
                  ORDER BY sent_at, id
                  LIMIT ?4",
            )?;
            let rows = statement.query_map(params![text_org, cutoff, now, limit], asset_row)?;
            let mut assets = Vec::new();
            for row in rows {
                assets.push(row?);
            }
            Ok(assets)
        })
    }

    pub fn mark_draft(&self, org: OrgId, id: AttachmentId, now: i64) -> Resultado<()> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        self.0.write(move |conn| {
            conn.execute(
                "UPDATE matrix_midia
                    SET redacted_at = COALESCE(redacted_at, ?3),
                        attempts = 0,
                        next_attempt_at = 0,
                        last_error = NULL,
                        updated_at = ?3
                  WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id, now],
            )
            .map(|_| ())
            .map_err(|error| StoreError::from_write("mídia da sala", error))
        })
    }

    pub fn mark_deleted(&self, org: OrgId, id: AttachmentId, now: i64) -> Resultado<()> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        self.0.write(move |conn| {
            conn.execute(
                "UPDATE matrix_midia
                    SET deleted_at = COALESCE(deleted_at, ?3),
                        attempts = 0,
                        next_attempt_at = 0,
                        last_error = NULL,
                        updated_at = ?3
                  WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id, now],
            )
            .map(|_| ())
            .map_err(|error| StoreError::from_write("mídia da sala", error))
        })
    }

    pub fn adiar(
        &self,
        org: OrgId,
        id: AttachmentId,
        next_attempt_at: i64,
        code: &str,
        now: i64,
    ) -> Resultado<u32> {
        let code: String = code.chars().take(LIMIT_ERROR_ASSET).collect();
        let text_org = org.to_string();
        let text_id = id.to_string();
        self.0.write(move |conn| {
            conn.execute(
                "UPDATE matrix_midia
                    SET attempts = attempts + 1,
                        next_attempt_at = ?3,
                        last_error = ?4,
                        updated_at = ?5
                  WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id, next_attempt_at, code, now],
            )
            .map_err(|error| StoreError::from_write("mídia da sala", error))?;
            let attempts: i64 = conn
                .query_row(
                    "SELECT attempts FROM matrix_midia WHERE org_id = ?1 AND id = ?2",
                    params![text_org, text_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StoreError::from)?
                .unwrap_or(0);
            Ok(u32::try_from(attempts).unwrap_or(u32::MAX))
        })
    }

    pub fn org_pending(&self, org: OrgId) -> Resultado<u32> {
        let text_org = org.to_string();
        self.0.read(move |conn| {
            let total: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM matrix_midia
                      WHERE org_id = ?1 AND (redacted_at IS NULL OR deleted_at IS NULL)",
                    params![text_org],
                    |row| row.get(0),
                )
                .map_err(StoreError::from)?;
            Ok(u32::try_from(total).unwrap_or(u32::MAX))
        })
    }
}

fn asset_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RoomAsset> {
    Ok(RoomAsset {
        id: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        conversation: column_id(row, 2)?,
        agent: row.get(3)?,
        room: row.get(4)?,
        evento: row.get(5)?,
        mxc: row.get(6)?,
        nome: row.get(7)?,
        mime: row.get(8)?,
        category: CategoryAsset::from_text(&row.get::<_, String>(9)?)?,
        size_bytes: column_u64(row, 10)?,
        sent_at: row.get(11)?,
        draft_at: row.get(12)?,
        deleted_at: row.get(13)?,
        attempts: column_u32(row, 14)?,
        next_attempt_at: row.get(15)?,
        ultimo_erro: row.get(16)?,
    })
}

fn require_text(
    entity: &'static str,
    campo: &'static str,
    value: &str,
    cap: usize,
) -> Resultado<()> {
    if value.trim().is_empty() {
        return Err(StoreError::Domain(DomainError::Validation {
            field: campo,
            reason: format!("{entity} não pode ficar vazia"),
        }));
    }
    let size = value.chars().count();
    if size > cap {
        return Err(StoreError::Domain(DomainError::Validation {
            field: campo,
            reason: format!("{entity} passa de {cap} caracteres ({size})"),
        }));
    }
    Ok(())
}
