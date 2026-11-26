use std::collections::HashMap;

use poph_types::{ConversationId, DepartmentId, DomainError, MessageId, OrgId, OutboxId};
use rusqlite::{Connection, OptionalExtension, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::ConversationStatus;
use crate::repos::{LIMIT_NAME_CLIENT, column_id, column_u32, column_u64};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateRoom {
    Creating,
    Ready,
}

impl StateRoom {
    pub fn as_text(self) -> &'static str {
        match self {
            StateRoom::Creating => "creating",
            StateRoom::Ready => "ready",
        }
    }

    fn from_text(value: &str) -> rusqlite::Result<Self> {
        match value {
            "creating" => Ok(StateRoom::Creating),
            "ready" => Ok(StateRoom::Ready),
            another => Err(rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::other(format!(
                    "estado de sala do Matrix desconhecido no banco: `{another}`"
                ))),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceOrigin {
    Automatic,
    Linked,
}

impl SpaceOrigin {
    pub fn as_text(self) -> &'static str {
        match self {
            SpaceOrigin::Automatic => "auto",
            SpaceOrigin::Linked => "linked",
        }
    }

    fn from_text(value: &str) -> rusqlite::Result<Self> {
        match value {
            "auto" => Ok(SpaceOrigin::Automatic),
            "linked" => Ok(SpaceOrigin::Linked),
            another => Err(rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::other(format!(
                    "origem de espaço do Matrix desconhecida no banco: `{another}`"
                ))),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateMatrixOrg {
    pub org_id: OrgId,
    pub linked: bool,
    pub linked_at: Option<i64>,
    pub updated_at: i64,
}

impl StateMatrixOrg {
    fn switched_off(org: OrgId) -> Self {
        Self {
            org_id: org,
            linked: false,
            linked_at: None,
            updated_at: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomMatrix {
    pub conversation: ConversationId,
    pub org_id: OrgId,
    pub room_id: String,
    pub space: Option<String>,
    pub last_status: Option<ConversationStatus>,
    pub name_client_mirrored: Option<String>,
    pub state: StateRoom,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceMatrix {
    pub id: uuid::Uuid,
    pub org_id: OrgId,
    pub departamento: DepartmentId,
    pub room_id: String,
    pub alias: String,
    pub origin: SpaceOrigin,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpaceDisabled {
    pub removed: bool,
    pub rooms_forgotten: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DelayMatrix {
    pub pending: u64,
    pub oldest_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CursorConversations {
    pub updated_at: i64,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorsMatrix {
    pub org_id: OrgId,
    pub messages: u64,
    pub message_ids: String,
    pub conversations: CursorConversations,
    pub version: u64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureMatrix {
    pub id: uuid::Uuid,
    pub org_id: OrgId,
    pub message_id: MessageId,
    pub motivo: String,
    pub attempts: u32,
    pub created_at: i64,
    pub updated_at: i64,
}

pub const LIMIT_REASON_FAILURE: usize = 64;

pub const LIMIT_FAILURES: u32 = 200;

pub struct Matrix<'a>(pub(crate) &'a Store);

impl Matrix<'_> {
    pub fn enable(&self, org: OrgId) -> Resultado<StateMatrixOrg> {
        let now = self.0.now();
        let text_org = org.to_string();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO matrix_orgs (org_id, enabled, enabled_at, updated_at)
                 VALUES (?1, 1, ?2, ?2)
                 ON CONFLICT (org_id) DO UPDATE SET
                     enabled = 1,
                     enabled_at = excluded.enabled_at,
                     updated_at = excluded.updated_at
                 WHERE matrix_orgs.enabled = 0",
                params![text_org, now],
            )
            .map(|_| ())
            .map_err(|error| StoreError::from_write("org", error))
        })?;
        self.state(org)
    }

    pub fn disable(&self, org: OrgId) -> Resultado<StateMatrixOrg> {
        let now = self.0.now();
        let text_org = org.to_string();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO matrix_orgs (org_id, enabled, enabled_at, updated_at)
                 VALUES (?1, 0, NULL, ?2)
                 ON CONFLICT (org_id) DO UPDATE SET
                     enabled = 0,
                     updated_at = excluded.updated_at
                 WHERE matrix_orgs.enabled = 1",
                params![text_org, now],
            )
            .map(|_| ())
            .map_err(|error| StoreError::from_write("org", error))
        })?;
        self.state(org)
    }

    pub fn state(&self, org: OrgId) -> Resultado<StateMatrixOrg> {
        let text_org = org.to_string();
        let located = self.0.read(move |conn| {
            conn.query_row(
                "SELECT org_id, enabled, enabled_at, updated_at FROM matrix_orgs WHERE org_id = ?1",
                params![text_org],
                org_state_row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        Ok(located.unwrap_or_else(|| StateMatrixOrg::switched_off(org)))
    }

    pub fn linked_orgs(&self) -> Resultado<Vec<StateMatrixOrg>> {
        self.0.read(|conn| {
            let mut statement = conn.prepare(
                "SELECT org_id, enabled, enabled_at, updated_at FROM matrix_orgs
                  WHERE enabled = 1 ORDER BY org_id",
            )?;
            let rows = statement.query_map([], org_state_row)?;
            let mut orgs = Vec::new();
            for row in rows {
                orgs.push(row?);
            }
            Ok(orgs)
        })
    }

    pub fn ensure_room(
        &self,
        org: OrgId,
        conversation: ConversationId,
        room_id: &str,
        space: Option<&str>,
    ) -> Resultado<RoomMatrix> {
        require_text("sala", "room_id", room_id, 255)?;
        if let Some(space) = space {
            require_text("espaço", "space_room_id", space, 255)?;
        }
        let now = self.0.now();
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        let room = room_id.to_owned();
        let space = space.map(str::to_owned);
        let stored = self.0.write(move |conn| {
            let fem_owner: Option<String> = conn
                .query_row(
                    "SELECT conversation_id FROM matrix_rooms WHERE room_id = ?1",
                    params![room],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            if let Some(fem_owner) = fem_owner.filter(|fem_owner| *fem_owner != text_conversation) {
                return Err(StoreError::Domain(DomainError::Conflict {
                    entity: "sala do Matrix",
                    id: room.clone(),
                    reason: format!("esta sala já está ligada à conversa {fem_owner}"),
                }));
            }
            conn.execute(
                "INSERT INTO matrix_rooms
                    (conversation_id, org_id, room_id, space_room_id, state, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'creating', ?5, ?5)
                 ON CONFLICT (conversation_id) DO NOTHING",
                params![text_conversation, text_org, room, space, now],
            )
            .map_err(|error| StoreError::from_write("sala", error))?;
            room_at(conn, &text_org, &text_conversation)
        })?;
        stored.ok_or_else(|| room_missing(&conversation.to_string()))
    }

    pub fn mark_room_ready(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Resultado<RoomMatrix> {
        self.mark_room_ready_with_name(org, conversation, None)
    }

    pub fn mark_room_ready_with_name(
        &self,
        org: OrgId,
        conversation: ConversationId,
        name_mirrored: Option<&str>,
    ) -> Resultado<RoomMatrix> {
        if let Some(nome) = name_mirrored {
            require_text(
                "nome do cliente",
                "client_name_espelhado",
                nome,
                LIMIT_NAME_CLIENT,
            )?;
        }
        let now = self.0.now();
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        let nome = name_mirrored.map(str::to_owned);
        let stored = self.0.write(move |conn| {
            conn.execute(
                "UPDATE matrix_rooms SET state = 'ready', client_name_espelhado = ?3, updated_at = ?4
                  WHERE org_id = ?1 AND conversation_id = ?2 AND state = 'creating'",
                params![text_org, text_conversation, nome, now],
            )
            .map_err(|error| StoreError::from_write("sala", error))?;
            room_at(conn, &text_org, &text_conversation)
        })?;
        stored.ok_or_else(|| room_missing(&conversation.to_string()))
    }

    pub fn set_mirrored_customer_name(
        &self,
        org: OrgId,
        conversation: ConversationId,
        nome: &str,
    ) -> Resultado<RoomMatrix> {
        require_text(
            "nome do cliente",
            "client_name_espelhado",
            nome,
            LIMIT_NAME_CLIENT,
        )?;
        let now = self.0.now();
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        let nome = nome.to_owned();
        let stored = self.0.write(move |conn| {
            conn.execute(
                "UPDATE matrix_rooms SET client_name_espelhado = ?3, updated_at = ?4
                  WHERE org_id = ?1 AND conversation_id = ?2 AND client_name_espelhado IS NOT ?3",
                params![text_org, text_conversation, nome, now],
            )
            .map_err(|error| StoreError::from_write("nome da sala", error))?;
            room_at(conn, &text_org, &text_conversation)
        })?;
        stored.ok_or_else(|| room_missing(&conversation.to_string()))
    }

    pub fn room(&self, org: OrgId, conversation: ConversationId) -> Resultado<Option<RoomMatrix>> {
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        self.0
            .read(move |conn| room_at(conn, &text_org, &text_conversation))
    }

    pub fn rooms_ready(
        &self,
        org: OrgId,
        conversations: &[ConversationId],
    ) -> Resultado<HashMap<ConversationId, String>> {
        if conversations.is_empty() {
            return Ok(HashMap::new());
        }
        let text_org = org.to_string();
        let list = serde_json::to_string(
            &conversations
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_owned());
        self.0.read(move |conn| {
            let mut statement = conn.prepare(
                "SELECT conversation_id, room_id FROM matrix_rooms
                  WHERE org_id = ?1 AND state = 'ready'
                    AND conversation_id IN (SELECT value FROM json_each(?2))",
            )?;
            let rows = statement.query_map(params![text_org, list], |row| {
                Ok((
                    column_id::<ConversationId>(row, 0)?,
                    row.get::<_, String>(1)?,
                ))
            })?;
            let mut rooms = HashMap::new();
            for row in rows {
                let (conversation, room) = row?;
                rooms.insert(conversation, room);
            }
            Ok(rooms)
        })
    }

    pub fn room_by_room_id(&self, org: OrgId, room_id: &str) -> Resultado<Option<RoomMatrix>> {
        let text_org = org.to_string();
        let room = room_id.to_owned();
        self.0.read(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUMNS_ROOM} FROM matrix_rooms WHERE org_id = ?1 AND room_id = ?2"
                ),
                params![text_org, room],
                room_row,
            )
            .optional()
            .map_err(StoreError::from)
        })
    }

    pub fn define_last_status(
        &self,
        org: OrgId,
        conversation: ConversationId,
        status: ConversationStatus,
    ) -> Resultado<RoomMatrix> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        let stored = self.0.write(move |conn| {
            conn.execute(
                "UPDATE matrix_rooms SET last_status = ?3, updated_at = ?4
                  WHERE org_id = ?1 AND conversation_id = ?2",
                params![text_org, text_conversation, status.as_text(), now],
            )
            .map_err(|error| StoreError::from_write("sala", error))?;
            room_at(conn, &text_org, &text_conversation)
        })?;
        stored.ok_or_else(|| room_missing(&conversation.to_string()))
    }

    pub fn set_space_of_room(
        &self,
        org: OrgId,
        conversation: ConversationId,
        space: Option<&str>,
    ) -> Resultado<RoomMatrix> {
        if let Some(space) = space {
            require_text("espaço", "space_room_id", space, 255)?;
        }
        let now = self.0.now();
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        let space = space.map(str::to_owned);
        let stored = self.0.write(move |conn| {
            conn.execute(
                "UPDATE matrix_rooms SET space_room_id = ?3, updated_at = ?4
                  WHERE org_id = ?1 AND conversation_id = ?2 AND space_room_id IS NOT ?3",
                params![text_org, text_conversation, space, now],
            )
            .map_err(|error| StoreError::from_write("espaço da sala", error))?;
            room_at(conn, &text_org, &text_conversation)
        })?;
        stored.ok_or_else(|| room_missing(&conversation.to_string()))
    }

    pub fn ensure_space(
        &self,
        org: OrgId,
        departamento: DepartmentId,
        room_id: &str,
        alias: &str,
        origin: SpaceOrigin,
    ) -> Resultado<SpaceMatrix> {
        require_text("espaço", "space_room_id", room_id, 255)?;
        require_text("espaço", "alias", alias, 255)?;
        let now = self.0.now();
        let text_org = org.to_string();
        let text_department = departamento.to_string();
        let room = room_id.to_owned();
        let alias = alias.to_owned();
        let recorded = self.0.write(move |conn| {
            let owner: Option<String> = conn
                .query_row(
                    "SELECT department_id FROM matrix_spaces
                      WHERE org_id = ?1 AND (space_room_id = ?2 OR alias = ?3)",
                    params![text_org, room, alias],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            if let Some(owner) = owner.filter(|owner| *owner != text_department) {
                return Err(StoreError::Domain(DomainError::Conflict {
                    entity: "espaço do Matrix",
                    id: room.clone(),
                    reason: format!("este espaço já está ligado ao departamento {owner}"),
                }));
            }
            conn.execute(
                "INSERT INTO matrix_spaces
                    (id, org_id, department_id, space_room_id, alias, origin, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
                 ON CONFLICT (org_id, department_id) DO NOTHING",
                params![
                    uuid::Uuid::now_v7().to_string(),
                    text_org,
                    text_department,
                    room,
                    alias,
                    origin.as_text(),
                    now
                ],
            )
            .map_err(|error| StoreError::from_write("espaço", error))?;
            space_at(conn, &text_org, &text_department)
        })?;
        recorded.ok_or_else(|| {
            StoreError::Domain(DomainError::NotFound {
                entity: "espaço do Matrix",
                id: departamento.to_string(),
            })
        })
    }

    pub fn space(&self, org: OrgId, departamento: DepartmentId) -> Resultado<Option<SpaceMatrix>> {
        let text_org = org.to_string();
        let text_department = departamento.to_string();
        self.0
            .read(move |conn| space_at(conn, &text_org, &text_department))
    }

    pub fn list_spaces(&self, org: OrgId) -> Resultado<Vec<SpaceMatrix>> {
        let text_org = org.to_string();
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS_SPACE} FROM matrix_spaces WHERE org_id = ?1 ORDER BY id"
            ))?;
            let rows = statement.query_map(params![text_org], space_row)?;
            let mut spaces = Vec::new();
            for row in rows {
                spaces.push(row?);
            }
            Ok(spaces)
        })
    }

    pub fn disable_space(
        &self,
        org: OrgId,
        departamento: DepartmentId,
    ) -> Resultado<SpaceDisabled> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_department = departamento.to_string();
        self.0.write(move |conn| {
            let recorded: Option<String> = conn
                .query_row(
                    "SELECT space_room_id FROM matrix_spaces
                      WHERE org_id = ?1 AND department_id = ?2",
                    params![text_org, text_department],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            let Some(space) = recorded else {
                return Ok(SpaceDisabled::default());
            };
            conn.execute(
                "DELETE FROM matrix_spaces WHERE org_id = ?1 AND department_id = ?2",
                params![text_org, text_department],
            )
            .map_err(|error| StoreError::from_write("espaço", error))?;
            let esquecidas = conn
                .execute(
                    "UPDATE matrix_rooms SET space_room_id = NULL, updated_at = ?3
                      WHERE org_id = ?1 AND space_room_id = ?2",
                    params![text_org, space, now],
                )
                .map_err(|error| StoreError::from_write("espaço da sala", error))?;
            Ok(SpaceDisabled {
                removed: true,
                rooms_forgotten: u64::try_from(esquecidas).unwrap_or(u64::MAX),
            })
        })
    }

    pub fn read_cursors(&self, org: OrgId) -> Resultado<CursorsMatrix> {
        let text_org = org.to_string();
        let located = self.0.read(move |conn| cursors_at(conn, &text_org))?;
        Ok(located.unwrap_or_else(|| CursorsMatrix {
            org_id: org,
            messages: 0,
            message_ids: String::new(),
            conversations: CursorConversations::default(),
            version: 0,
            updated_at: 0,
        }))
    }

    pub fn delay_of_messages(&self, org: OrgId, cursor: u64) -> Resultado<DelayMatrix> {
        let text_org = org.to_string();
        let cursor = i64::try_from(cursor).unwrap_or(i64::MAX);
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT count(*), min(created_at) FROM messages
                  WHERE org_id = ?1 AND rowid > ?2 AND role IN ('user', 'assistant')",
                params![text_org, cursor],
                |row| {
                    Ok(DelayMatrix {
                        pending: column_u64(row, 0)?,
                        oldest_at: row.get::<_, Option<i64>>(1)?,
                    })
                },
            )
            .map_err(StoreError::from)
        })
    }

    pub fn advance_messages(&self, org: OrgId, rowid: u64) -> Resultado<CursorsMatrix> {
        if rowid > i64::MAX as u64 {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "messages_cursor",
                reason: "cursor de mensagens fora da faixa".to_owned(),
            }));
        }
        let now = self.0.now();
        let text_org = org.to_string();
        let cursor = rowid as i64;
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO matrix_estado (org_id, messages_cursor, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (org_id) DO UPDATE SET
                     messages_cursor = excluded.messages_cursor,
                     version = matrix_estado.version + 1,
                     updated_at = excluded.updated_at
                 WHERE matrix_estado.messages_cursor < excluded.messages_cursor",
                params![text_org, cursor, now],
            )
            .map(|_| ())
            .map_err(|error| StoreError::from_write("messages_cursor", error))
        })?;
        self.read_cursors(org)
    }

    pub fn advance_messages_with_id(
        &self,
        org: OrgId,
        rowid: u64,
        mensagem: &str,
    ) -> Resultado<CursorsMatrix> {
        if rowid > i64::MAX as u64 {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "messages_cursor",
                reason: "cursor de mensagens fora da faixa".to_owned(),
            }));
        }
        let now = self.0.now();
        let text_org = org.to_string();
        let cursor = rowid as i64;
        let id = mensagem.to_owned();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO matrix_estado (org_id, messages_cursor, messages_cursor_id, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (org_id) DO UPDATE SET
                     messages_cursor = excluded.messages_cursor,
                     messages_cursor_id = excluded.messages_cursor_id,
                     version = matrix_estado.version + 1,
                     updated_at = excluded.updated_at
                 WHERE matrix_estado.messages_cursor < excluded.messages_cursor",
                params![text_org, cursor, id, now],
            )
            .map(|_| ())
            .map_err(|error| StoreError::from_write("messages_cursor", error))
        })?;
        self.read_cursors(org)
    }

    pub fn rewind_cursor_messages(
        &self,
        org: OrgId,
        rowid: u64,
        mensagem: &str,
    ) -> Resultado<CursorsMatrix> {
        if rowid > i64::MAX as u64 {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "messages_cursor",
                reason: "cursor de mensagens fora da faixa".to_owned(),
            }));
        }
        let now = self.0.now();
        let text_org = org.to_string();
        let cursor = rowid as i64;
        let id = mensagem.to_owned();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO matrix_estado (org_id, messages_cursor, messages_cursor_id, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (org_id) DO UPDATE SET
                     messages_cursor = excluded.messages_cursor,
                     messages_cursor_id = excluded.messages_cursor_id,
                     version = matrix_estado.version + 1,
                     updated_at = excluded.updated_at",
                params![text_org, cursor, id, now],
            )
            .map(|_| ())
            .map_err(|error| StoreError::from_write("messages_cursor", error))
        })?;
        self.read_cursors(org)
    }

    pub fn advance_conversations(
        &self,
        org: OrgId,
        updated_at: i64,
        id: &str,
    ) -> Resultado<CursorsMatrix> {
        if updated_at < 0 {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "conversations_cursor_at",
                reason: "instante do cursor de conversas não pode ser negativo".to_owned(),
            }));
        }
        let now = self.0.now();
        let text_org = org.to_string();
        let conversation = id.to_owned();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO matrix_estado
                    (org_id, conversations_cursor_at, conversations_cursor_id, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (org_id) DO UPDATE SET
                     conversations_cursor_at = excluded.conversations_cursor_at,
                     conversations_cursor_id = excluded.conversations_cursor_id,
                     version = matrix_estado.version + 1,
                     updated_at = excluded.updated_at
                 WHERE matrix_estado.conversations_cursor_at < excluded.conversations_cursor_at
                    OR (matrix_estado.conversations_cursor_at = excluded.conversations_cursor_at
                        AND matrix_estado.conversations_cursor_id < excluded.conversations_cursor_id)",
                params![text_org, updated_at, conversation, now],
            )
            .map(|_| ())
            .map_err(|error| StoreError::from_write("conversations_cursor_at", error))
        })?;
        self.read_cursors(org)
    }

    pub fn register_failure(
        &self,
        org: OrgId,
        mensagem: MessageId,
        motivo: &str,
        attempts: u32,
    ) -> Resultado<FailureMatrix> {
        require_text("motivo", "motivo", motivo, LIMIT_REASON_FAILURE)?;
        let now = self.0.now();
        let text_org = org.to_string();
        let text_message = mensagem.to_string();
        let code = motivo.to_owned();
        let attempts = i64::from(attempts.max(1));
        let stored = self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO matrix_falhas
                    (id, org_id, message_id, motivo, tentativas, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                 ON CONFLICT (org_id, message_id) DO UPDATE SET
                     motivo = excluded.motivo,
                     tentativas = excluded.tentativas,
                     updated_at = excluded.updated_at",
                params![
                    uuid::Uuid::now_v7().to_string(),
                    text_org,
                    text_message,
                    code,
                    attempts,
                    now
                ],
            )
            .map_err(|error| StoreError::from_write("motivo", error))?;
            failure_at(conn, &text_org, &text_message)
        })?;
        stored.ok_or_else(|| {
            StoreError::Domain(DomainError::NotFound {
                entity: "falha do Matrix",
                id: mensagem.to_string(),
            })
        })
    }

    pub fn count_failures(&self, org: OrgId) -> Resultado<u64> {
        let text_org = org.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT count(*) FROM matrix_falhas WHERE org_id = ?1",
                params![text_org],
                |row| column_u64(row, 0),
            )
            .map_err(StoreError::from)
        })
    }

    pub fn list_failures(&self, org: OrgId, limit: u32) -> Resultado<Vec<FailureMatrix>> {
        let text_org = org.to_string();
        let limit = i64::from(limit.clamp(1, LIMIT_FAILURES));
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS_FAILURE} FROM matrix_falhas WHERE org_id = ?1
                  ORDER BY updated_at DESC, message_id LIMIT ?2"
            ))?;
            let rows = statement.query_map(params![text_org, limit], failure_row)?;
            let mut failures = Vec::new();
            for row in rows {
                failures.push(row?);
            }
            Ok(failures)
        })
    }

    pub fn remove_failure(&self, org: OrgId, mensagem: MessageId) -> Resultado<bool> {
        let text_org = org.to_string();
        let text_message = mensagem.to_string();
        let deleted = self.0.write(move |conn| {
            conn.execute(
                "DELETE FROM matrix_falhas WHERE org_id = ?1 AND message_id = ?2",
                params![text_org, text_message],
            )
            .map_err(StoreError::from)
        })?;
        Ok(deleted > 0)
    }
}

const COLUMNS_ROOM: &str = "conversation_id, org_id, room_id, space_room_id, last_status, \
     client_name_espelhado, state, created_at, updated_at";
const COLUMNS_SPACE: &str =
    "id, org_id, department_id, space_room_id, alias, origin, created_at, updated_at";
const COLUMNS_FAILURE: &str = "id, org_id, message_id, motivo, tentativas, created_at, updated_at";

fn column_uuid(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<uuid::Uuid> {
    let texto: String = row.get(index)?;
    texto.parse().map_err(|error: uuid::Error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn org_state_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StateMatrixOrg> {
    Ok(StateMatrixOrg {
        org_id: column_id(row, 0)?,
        linked: row.get::<_, i64>(1)? != 0,
        linked_at: row.get(2)?,
        updated_at: row.get(3)?,
    })
}

fn room_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RoomMatrix> {
    let last_status: Option<String> = row.get(4)?;
    let last_status = match last_status {
        Some(texto) => Some(ConversationStatus::from_text(&texto).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?),
        None => None,
    };
    Ok(RoomMatrix {
        conversation: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        room_id: row.get(2)?,
        space: row.get(3)?,
        last_status,
        name_client_mirrored: row.get(5)?,
        state: StateRoom::from_text(&row.get::<_, String>(6)?)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn space_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SpaceMatrix> {
    Ok(SpaceMatrix {
        id: column_uuid(row, 0)?,
        org_id: column_id(row, 1)?,
        departamento: column_id(row, 2)?,
        room_id: row.get(3)?,
        alias: row.get(4)?,
        origin: SpaceOrigin::from_text(&row.get::<_, String>(5)?)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn failure_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FailureMatrix> {
    Ok(FailureMatrix {
        id: column_uuid(row, 0)?,
        org_id: column_id(row, 1)?,
        message_id: column_id(row, 2)?,
        motivo: row.get(3)?,
        attempts: column_u32(row, 4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn room_at(
    conn: &Connection,
    text_org: &str,
    text_conversation: &str,
) -> Resultado<Option<RoomMatrix>> {
    conn.query_row(
        &format!(
            "SELECT {COLUMNS_ROOM} FROM matrix_rooms
              WHERE org_id = ?1 AND conversation_id = ?2"
        ),
        params![text_org, text_conversation],
        room_row,
    )
    .optional()
    .map_err(StoreError::from)
}

fn space_at(
    conn: &Connection,
    text_org: &str,
    text_department: &str,
) -> Resultado<Option<SpaceMatrix>> {
    conn.query_row(
        &format!(
            "SELECT {COLUMNS_SPACE} FROM matrix_spaces
              WHERE org_id = ?1 AND department_id = ?2"
        ),
        params![text_org, text_department],
        space_row,
    )
    .optional()
    .map_err(StoreError::from)
}

fn failure_at(
    conn: &Connection,
    text_org: &str,
    text_message: &str,
) -> Resultado<Option<FailureMatrix>> {
    conn.query_row(
        &format!(
            "SELECT {COLUMNS_FAILURE} FROM matrix_falhas
              WHERE org_id = ?1 AND message_id = ?2"
        ),
        params![text_org, text_message],
        failure_row,
    )
    .optional()
    .map_err(StoreError::from)
}

fn cursors_at(conn: &Connection, text_org: &str) -> Resultado<Option<CursorsMatrix>> {
    conn.query_row(
        "SELECT org_id, messages_cursor, conversations_cursor_at, conversations_cursor_id,
                version, updated_at, messages_cursor_id
           FROM matrix_estado WHERE org_id = ?1",
        params![text_org],
        |row| {
            Ok(CursorsMatrix {
                org_id: column_id(row, 0)?,
                messages: column_u64(row, 1)?,
                message_ids: row.get(6)?,
                conversations: CursorConversations {
                    updated_at: row.get(2)?,
                    id: row.get(3)?,
                },
                version: column_u64(row, 4)?,
                updated_at: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(StoreError::from)
}

fn room_missing(text_conversation: &str) -> StoreError {
    StoreError::Domain(DomainError::NotFound {
        entity: "sala do Matrix",
        id: text_conversation.to_owned(),
    })
}

fn require_text(
    entity: &'static str,
    campo: &'static str,
    value: &str,
    cap: usize,
) -> Resultado<()> {
    let size = value.chars().count();
    if value.trim().is_empty() {
        return Err(StoreError::Domain(DomainError::Validation {
            field: campo,
            reason: format!("{entity} não pode ficar vazia"),
        }));
    }
    if size > cap {
        return Err(StoreError::Domain(DomainError::Validation {
            field: campo,
            reason: format!("{entity} passa de {cap} caracteres ({size})"),
        }));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeNotice {
    Notify,
    WithoutRoom,
    Refused,
}

impl OutcomeNotice {
    pub fn as_text(self) -> &'static str {
        match self {
            OutcomeNotice::Notify => "avisado",
            OutcomeNotice::WithoutRoom => "sem_sala",
            OutcomeNotice::Refused => "recusado",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanResponseFailure {
    pub org_id: OrgId,
    pub outbox_id: OutboxId,
    pub conversation: ConversationId,
    pub ultimo_erro: Option<String>,
}

impl Matrix<'_> {
    pub fn human_response_failures(
        &self,
        org: OrgId,
        limit: u32,
    ) -> Resultado<Vec<HumanResponseFailure>> {
        let text_org = org.to_string();
        let limit = i64::from(limit.clamp(1, 500));
        self.0.read(move |conn| {
            let mut statement = conn.prepare(
                "SELECT o.id, o.conversation_id, o.last_error FROM outbox o
                  WHERE o.org_id = ?1 AND o.state = 'failed'
                    AND EXISTS (SELECT 1 FROM messages m
                                 WHERE m.org_id = o.org_id AND m.turn_id = o.turn_id
                                   AND m.origin = 'human')
                    AND NOT EXISTS (SELECT 1 FROM matrix_avisos_falha a
                                     WHERE a.org_id = o.org_id AND a.outbox_id = o.id)
                  ORDER BY o.created_at, o.id LIMIT ?2",
            )?;
            let rows = statement.query_map(params![text_org, limit], |row| {
                Ok(HumanResponseFailure {
                    org_id: org,
                    outbox_id: column_id(row, 0)?,
                    conversation: column_id(row, 1)?,
                    ultimo_erro: row.get(2)?,
                })
            })?;
            let mut failures = Vec::new();
            for row in rows {
                failures.push(row?);
            }
            Ok(failures)
        })
    }

    pub fn record_notice_outcome(
        &self,
        org: OrgId,
        failure: OutboxId,
        outcome: OutcomeNotice,
        motivo: &str,
    ) -> Resultado<bool> {
        require_text("motivo do aviso", "motivo", motivo, 64)?;
        let now = self.0.now();
        let text_org = org.to_string();
        let text_failure = failure.to_string();
        let text_outcome = outcome.as_text();
        let motivo = motivo.to_owned();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO matrix_avisos_falha
                    (org_id, outbox_id, desfecho, motivo, registrado_em)
                 SELECT ?1, ?2, ?3, ?4, ?5 FROM outbox WHERE id = ?2 AND org_id = ?1
                 ON CONFLICT (org_id, outbox_id) DO NOTHING",
                params![text_org, text_failure, text_outcome, motivo, now],
            )
            .map(|rows| rows > 0)
            .map_err(StoreError::from)
        })
    }
}
