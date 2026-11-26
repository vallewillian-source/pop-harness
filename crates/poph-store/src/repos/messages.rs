use poph_types::{AttachmentId, AuditId, ConversationId, MessageId, OrgId, OutboxId, TurnId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{
    ConversationStatus, Direction, HumanRecording, HumanResponse, HumanResponseWithFile, Mensagem,
    NewChannelAttachment, NewHumanResponse, NewMessage, NewOutboxItem, Origin, OutboxItem, Page,
    Role,
};
use crate::repos::boxes::{COLUMNS_OUTPUT, new_output_item, output_row};
use crate::repos::{column_id, id_optional};

const COLUMNS: &str = "id, org_id, conversation_id, direction, role, text, attachments_json,
    external_id, turn_id, created_at, origin, author_name";

pub struct Messages<'a>(pub(crate) &'a Store);

struct EventHuman<'a> {
    event_id: &'a str,
    text_raw: &'a str,
    author_id: &'a str,
    author_name: &'a str,
}

impl Messages<'_> {
    pub fn write(&self, org: OrgId, fresh: NewMessage) -> Resultado<Mensagem> {
        let now = self.0.now();
        let id = MessageId::generate();
        let text_org = org.to_string();
        self.0.write(move |conn| {
            let NewMessage {
                conversation_id,
                direction,
                role,
                origin,
                author_name,
                texto,
                attachments_json,
                external_id,
                turn_id,
            } = fresh;
            let conversation = conversation_id.to_string();
            let inseriu = conn
                .execute(
                    "INSERT OR IGNORE INTO messages
                        (id, org_id, conversation_id, direction, role, text, attachments_json,
                         external_id, turn_id, created_at, origin, author_name)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        id.to_string(),
                        &text_org,
                        &conversation,
                        direction.as_text(),
                        role.as_text(),
                        &texto,
                        &attachments_json,
                        external_id.as_deref(),
                        turn_id.map(|turno| turno.to_string()),
                        now,
                        origin.as_text(),
                        author_name.as_deref()
                    ],
                )
                .map_err(StoreError::from)?;

            if inseriu == 0 {
                let existing = conn
                    .query_row(
                        &format!(
                            "SELECT {COLUMNS} FROM messages
                              WHERE conversation_id = ?1 AND external_id = ?2"
                        ),
                        params![&conversation, external_id.as_deref()],
                        row,
                    )
                    .optional()
                    .map_err(StoreError::from)?;
                return existing.ok_or_else(|| {
                    StoreError::Internal(
                        "a inserção da mensagem foi ignorada, mas a original não foi encontrada"
                            .to_owned(),
                    )
                });
            }

            Ok(Mensagem {
                id,
                org_id: org,
                conversation_id,
                direction,
                role,
                origin,
                author_name,
                texto,
                attachments_json,
                external_id,
                turn_id,
                created_at: now,
            })
        })
    }

    pub fn write_with_attachments(
        &self,
        org: OrgId,
        fresh: NewMessage,
        attachments: &[NewChannelAttachment],
    ) -> Resultado<Mensagem> {
        let now = self.0.now();
        let id = MessageId::generate();
        let text_org = org.to_string();
        let attachments = attachments.to_vec();
        self.0.write(move |conn| {
            let NewMessage {
                conversation_id,
                direction,
                role,
                origin,
                author_name,
                texto,
                attachments_json,
                external_id,
                turn_id,
            } = fresh;
            let conversation = conversation_id.to_string();
            let inseriu = conn
                .execute(
                    "INSERT OR IGNORE INTO messages
                        (id, org_id, conversation_id, direction, role, text, attachments_json,
                         external_id, turn_id, created_at, origin, author_name)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        id.to_string(),
                        &text_org,
                        &conversation,
                        direction.as_text(),
                        role.as_text(),
                        &texto,
                        &attachments_json,
                        external_id.as_deref(),
                        turn_id.map(|turno| turno.to_string()),
                        now,
                        origin.as_text(),
                        author_name.as_deref()
                    ],
                )
                .map_err(StoreError::from)?;
            if inseriu == 0 {
                let existing = conn
                    .query_row(
                        &format!(
                            "SELECT {COLUMNS} FROM messages
                              WHERE conversation_id = ?1 AND external_id = ?2"
                        ),
                        params![&conversation, external_id.as_deref()],
                        row,
                    )
                    .optional()
                    .map_err(StoreError::from)?;
                return existing.ok_or_else(|| {
                    StoreError::Internal(
                        "a inserção da mensagem foi ignorada, mas a original não foi encontrada"
                            .to_owned(),
                    )
                });
            }

            let mut ids: Vec<String> = Vec::with_capacity(attachments.len());
            for attachment in &attachments {
                let attachment_id = AttachmentId::generate().to_string();
                conn.execute(
                    "INSERT INTO attachments
                        (id, org_id, message_id, name, mime, size_bytes, sha256, blob_path,
                         extracted_text, extraction_status, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        &attachment_id,
                        &text_org,
                        id.to_string(),
                        &attachment.name,
                        &attachment.mime,
                        attachment.size_bytes as i64,
                        &attachment.sha256,
                        &attachment.blob_path,
                        &attachment.extracted_text,
                        attachment.extraction_status,
                        now
                    ],
                )
                .map_err(StoreError::from)?;
                ids.push(attachment_id);
            }
            let attachments_json = if ids.is_empty() {
                "[]".to_owned()
            } else {
                serde_json::to_string(&ids).map_err(|error| {
                    StoreError::Internal(format!("lista de anexos não serializa: {error}"))
                })?
            };
            conn.execute(
                "UPDATE messages SET attachments_json = ?2 WHERE id = ?1",
                params![id.to_string(), &attachments_json],
            )
            .map_err(StoreError::from)?;
            conn.execute(
                "UPDATE conversations SET last_message_at = ?2, updated_at = ?2 WHERE id = ?1",
                params![&conversation, now],
            )
            .map_err(StoreError::from)?;

            Ok(Mensagem {
                id,
                org_id: org,
                conversation_id,
                direction,
                role,
                origin,
                author_name,
                texto,
                attachments_json,
                external_id,
                turn_id,
                created_at: now,
            })
        })
    }

    pub fn write_with_output(
        &self,
        org: OrgId,
        fresh: NewMessage,
        saida: NewOutboxItem,
    ) -> Resultado<(Mensagem, OutboxItem)> {
        let now = self.0.now();
        let id = MessageId::generate();
        let output_id = OutboxId::generate();
        let text_org = org.to_string();
        self.0.write(move |conn| {
            let NewMessage {
                conversation_id,
                direction,
                role,
                origin,
                author_name,
                texto,
                attachments_json,
                external_id,
                turn_id,
            } = fresh;
            let NewOutboxItem {
                conversation_id: conversation_of_output,
                turn_id: output_turn,
                seq,
                tipo,
                content_json,
            } = saida;
            if conversation_of_output != conversation_id || turn_id != Some(output_turn) {
                return Err(StoreError::Internal(
                    "a resposta e o item de saída precisam ser da mesma conversa e do mesmo turno"
                        .to_owned(),
                ));
            }
            let conversation = conversation_id.to_string();
            let text_turn = output_turn.to_string();
            conn.execute(
                "INSERT OR IGNORE INTO messages
                    (id, org_id, conversation_id, direction, role, text, attachments_json,
                     external_id, turn_id, created_at, origin, author_name)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id.to_string(),
                    &text_org,
                    &conversation,
                    direction.as_text(),
                    role.as_text(),
                    &texto,
                    &attachments_json,
                    external_id.as_deref(),
                    turn_id.map(|turno| turno.to_string()),
                    now,
                    origin.as_text(),
                    author_name.as_deref()
                ],
            )
            .map_err(StoreError::from)?;

            let inseriu = conn
                .execute(
                    "INSERT OR IGNORE INTO outbox
                        (id, org_id, conversation_id, turn_id, seq, kind, payload_json, state,
                         attempts, last_error, created_at, sent_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', 0, NULL, ?8, NULL)",
                    params![
                        output_id.to_string(),
                        &text_org,
                        &conversation,
                        &text_turn,
                        i64::from(seq),
                        &tipo,
                        &content_json,
                        now
                    ],
                )
                .map_err(StoreError::from)?;

            let item = if inseriu == 0 {
                conn.query_row(
                    &format!("SELECT {COLUMNS_OUTPUT} FROM outbox WHERE turn_id = ?1 AND seq = ?2"),
                    params![&text_turn, i64::from(seq)],
                    output_row,
                )
                .optional()
                .map_err(StoreError::from)?
                .ok_or_else(|| {
                    StoreError::Internal(
                        "inserção da saída ignorada, mas a original não foi encontrada".to_owned(),
                    )
                })?
            } else {
                new_output_item(
                    output_id,
                    org,
                    NewOutboxItem {
                        conversation_id,
                        turn_id: output_turn,
                        seq,
                        tipo,
                        content_json,
                    },
                    now,
                )
            };

            Ok((
                Mensagem {
                    id,
                    org_id: org,
                    conversation_id,
                    direction,
                    role,
                    origin,
                    author_name,
                    texto,
                    attachments_json,
                    external_id,
                    turn_id,
                    created_at: now,
                },
                item,
            ))
        })
    }

    pub fn register_response_human(
        &self,
        org: OrgId,
        conversation: ConversationId,
        fresh: NewHumanResponse<'_>,
    ) -> Resultado<HumanResponse> {
        let payload = serde_json::json!({ "texto": fresh.sent_text }).to_string();
        self.write_response_human(
            org,
            conversation,
            EventHuman {
                event_id: fresh.event_id,
                text_raw: fresh.raw_text,
                author_id: fresh.author_id,
                author_name: fresh.author_name,
            },
            "text",
            payload,
            None,
        )
    }

    pub fn record_human_response_with_file(
        &self,
        org: OrgId,
        conversation: ConversationId,
        fresh: HumanResponseWithFile<'_>,
    ) -> Resultado<HumanResponse> {
        let file = fresh.file;
        let attachment_id = AttachmentId::generate();
        let payload = serde_json::json!({
            "blob_ref": file.blob_path,
            "arquivo_id": attachment_id.to_string(),
            "nome": file.name,
            "mime": file.mime,
            "tamanho_bytes": file.size_bytes,
            "sha256": file.sha256,
            "legenda": file.caption,
        })
        .to_string();
        let attachment = NewChannelAttachment {
            name: file.name.to_owned(),
            mime: file.mime.to_owned(),
            size_bytes: file.size_bytes,
            sha256: file.sha256.to_owned(),
            blob_path: file.blob_path.to_owned(),
            extracted_text: None,
            extraction_status: file.extraction_status,
        };
        self.write_response_human(
            org,
            conversation,
            EventHuman {
                event_id: fresh.event_id,
                text_raw: "",
                author_id: fresh.author_id,
                author_name: fresh.author_name,
            },
            "file",
            payload,
            Some((attachment_id, attachment)),
        )
    }

    fn write_response_human(
        &self,
        org: OrgId,
        conversation: ConversationId,
        fresh: EventHuman<'_>,
        output_kind: &str,
        content_json: String,
        attachment: Option<(AttachmentId, NewChannelAttachment)>,
    ) -> Resultado<HumanResponse> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        let evento = fresh.event_id.to_owned();
        let raw = fresh.text_raw.to_owned();
        let actor = fresh.author_id.to_owned();
        let nome = fresh.author_name.to_owned();
        let output_kind = output_kind.to_owned();
        let attachments_json = match &attachment {
            Some((id, _)) => serde_json::json!([id.to_string()]).to_string(),
            None => "[]".to_owned(),
        };
        let id = MessageId::generate();
        let output_id = OutboxId::generate();
        let turno = TurnId::generate();
        self.0.write(move |conn| {
            let situation: Option<String> = conn
                .query_row(
                    "SELECT status FROM conversations WHERE org_id = ?1 AND id = ?2",
                    params![&text_org, &text_conversation],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            let Some(situation) = situation else {
                return Ok(HumanResponse::ConversationMissing);
            };
            let already_stored = conn
                .query_row(
                    "SELECT 1 FROM messages
                      WHERE org_id = ?1 AND conversation_id = ?2 AND external_id = ?3",
                    params![&text_org, &text_conversation, &evento],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(StoreError::from)?
                .is_some();
            if already_stored {
                return Ok(HumanResponse::Duplicate);
            }
            let situation = ConversationStatus::from_text(&situation)?;
            if situation == ConversationStatus::Closed {
                return Ok(HumanResponse::ConversationClosed);
            }
            let just_transferred = situation == ConversationStatus::Open;

            conn.execute(
                "INSERT INTO messages
                    (id, org_id, conversation_id, direction, role, text, attachments_json,
                     external_id, turn_id, created_at, origin, author_name)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id.to_string(),
                    &text_org,
                    &text_conversation,
                    Direction::Outbound.as_text(),
                    Role::Assistant.as_text(),
                    &raw,
                    &attachments_json,
                    &evento,
                    turno.to_string(),
                    now,
                    Origin::Human.as_text(),
                    &nome
                ],
            )
            .map_err(StoreError::from)?;

            if let Some((attachment_id, attachment)) = &attachment {
                conn.execute(
                    "INSERT INTO attachments
                        (id, org_id, message_id, name, mime, size_bytes, sha256, blob_path,
                         extracted_text, extraction_status, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        attachment_id.to_string(),
                        &text_org,
                        id.to_string(),
                        &attachment.name,
                        &attachment.mime,
                        attachment.size_bytes as i64,
                        &attachment.sha256,
                        &attachment.blob_path,
                        attachment.extracted_text.as_deref(),
                        attachment.extraction_status,
                        now
                    ],
                )
                .map_err(StoreError::from)?;
            }

            let new_output = NewOutboxItem {
                conversation_id: conversation,
                turn_id: turno,
                seq: 0,
                tipo: output_kind.clone(),
                content_json: content_json.clone(),
            };
            conn.execute(
                "INSERT INTO outbox
                    (id, org_id, conversation_id, turn_id, seq, kind, payload_json, state,
                     attempts, last_error, created_at, sent_at)
                 VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, 'pending', 0, NULL, ?7, NULL)",
                params![
                    output_id.to_string(),
                    &text_org,
                    &text_conversation,
                    turno.to_string(),
                    &output_kind,
                    &content_json,
                    now
                ],
            )
            .map_err(StoreError::from)?;
            let saida = new_output_item(output_id, org, new_output, now);

            if just_transferred {
                conn.execute(
                    "UPDATE conversations
                        SET status = 'transferred', version = version + 1,
                            last_message_at = ?3, updated_at = ?3
                      WHERE org_id = ?1 AND id = ?2 AND status = 'open'",
                    params![&text_org, &text_conversation, now],
                )
                .map_err(StoreError::from)?;
            } else {
                conn.execute(
                    "UPDATE conversations SET last_message_at = ?3, updated_at = ?3
                      WHERE org_id = ?1 AND id = ?2",
                    params![&text_org, &text_conversation, now],
                )
                .map_err(StoreError::from)?;
            }

            let detail = serde_json::json!({
                "transferiu": just_transferred,
                "mensagem_id": id.to_string(),
            });
            conn.execute(
                "INSERT INTO audit_log
                    (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json,
                     created_at)
                 VALUES (?1, ?2, 'matrix_user', ?3, 'human_takeover', 'conversation', ?4, ?5, ?6)",
                params![
                    AuditId::generate().to_string(),
                    &text_org,
                    &actor,
                    &text_conversation,
                    detail.to_string(),
                    now
                ],
            )
            .map_err(StoreError::from)?;

            Ok(HumanResponse::Stored(Box::new(HumanRecording {
                mensagem: Mensagem {
                    id,
                    org_id: org,
                    conversation_id: conversation,
                    direction: Direction::Outbound,
                    role: Role::Assistant,
                    origin: Origin::Human,
                    author_name: Some(nome),
                    texto: raw,
                    attachments_json,
                    external_id: Some(evento),
                    turn_id: Some(turno),
                    created_at: now,
                },
                saida,
                just_transferred,
            })))
        })
    }

    pub fn list(
        &self,
        org: OrgId,
        conversation: ConversationId,
        page: Page,
    ) -> Resultado<Vec<Mensagem>> {
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        let after = page.after.map(|id| id.to_string());
        let limit = i64::from(page.limit);
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM messages
                  WHERE org_id = ?1 AND conversation_id = ?2 AND (?3 IS NULL OR id > ?3)
                  ORDER BY id LIMIT ?4"
            ))?;
            let rows =
                statement.query_map(params![text_org, text_conversation, after, limit], row)?;
            let mut messages = Vec::new();
            for row in rows {
                messages.push(row?);
            }
            Ok(messages)
        })
    }

    pub fn last_set(
        &self,
        org: OrgId,
        conversation: ConversationId,
        limit: u32,
    ) -> Resultado<Vec<Mensagem>> {
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        let limit = i64::from(limit.clamp(1, 1000));
        let mut messages = self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM messages
                  WHERE org_id = ?1 AND conversation_id = ?2
                  ORDER BY id DESC LIMIT ?3"
            ))?;
            let rows = statement.query_map(params![text_org, text_conversation, limit], row)?;
            let mut messages = Vec::new();
            for row in rows {
                messages.push(row?);
            }
            Ok(messages)
        })?;
        messages.reverse();
        Ok(messages)
    }

    pub fn get(
        &self,
        org: OrgId,
        conversation: ConversationId,
        id: MessageId,
    ) -> Resultado<Mensagem> {
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        let text_id = id.to_string();
        let found = self.0.read(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUMNS} FROM messages
                      WHERE org_id = ?1 AND conversation_id = ?2 AND id = ?3"
                ),
                params![text_org, text_conversation, text_id],
                row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(found, "mensagem", &id.to_string())
    }

    pub fn had_messages_before(
        &self,
        org: OrgId,
        conversation: ConversationId,
        cutoff: i64,
    ) -> Resultado<bool> {
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM messages
                                 WHERE org_id = ?1 AND conversation_id = ?2 AND created_at < ?3)",
                params![text_org, text_conversation, cutoff],
                |row| row.get::<_, bool>(0),
            )
            .map_err(StoreError::from)
        })
    }

    pub fn last_rowid(&self) -> Resultado<Option<(u64, MessageId)>> {
        self.0.read(|conn| {
            let row: Option<(i64, String)> = conn
                .query_row(
                    "SELECT rowid, id FROM messages ORDER BY rowid DESC LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(StoreError::from)?;
            match row {
                None => Ok(None),
                Some((rowid, id)) => {
                    let id: MessageId = id.parse().map_err(|_| {
                        StoreError::Domain(poph_types::DomainError::Validation {
                            field: "messages.id",
                            reason: "id de mensagem inválido no banco".to_owned(),
                        })
                    })?;
                    Ok(Some((u64::try_from(rowid).unwrap_or(0), id)))
                }
            }
        })
    }

    pub fn count_rows(&self, org: OrgId, conversation: ConversationId) -> Resultado<u64> {
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT count(*) FROM messages WHERE org_id = ?1 AND conversation_id = ?2",
                params![text_org, text_conversation],
                |row| row.get::<_, i64>(0),
            )
            .map(|total| total.max(0) as u64)
            .map_err(StoreError::from)
        })
    }

    pub fn since(&self, after: u64, limit: u32) -> Resultado<Vec<MessageSince>> {
        let after = i64::try_from(after).unwrap_or(i64::MAX);
        let limit = i64::from(limit.clamp(1, LIMIT_SINCE));
        self.0.read(move |conn| {
            let mut statement = conn.prepare(
                "SELECT rowid, id, org_id, conversation_id, direction, role, origin, author_name,
                        text, attachments_json, external_id, created_at
                   FROM messages
                  WHERE rowid > ?1 AND role IN ('user', 'assistant')
                  ORDER BY rowid
                  LIMIT ?2",
            )?;
            let rows = statement.query_map(params![after, limit], since_row)?;
            let mut messages = Vec::new();
            for row in rows {
                messages.push(row?);
            }
            Ok(messages)
        })
    }
}

pub const LIMIT_SINCE: u32 = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageSince {
    pub rowid: u64,
    pub id: MessageId,
    pub org_id: OrgId,
    pub conversation_id: ConversationId,
    pub direction: Direction,
    pub role: Role,
    pub origin: Origin,
    pub author_name: Option<String>,
    pub texto: String,
    pub attachments_json: String,
    pub external_id: Option<String>,
    pub created_at: i64,
}

fn since_row(row: &Row<'_>) -> rusqlite::Result<MessageSince> {
    let direction = match row.get::<_, String>(4)?.as_str() {
        "in" => Direction::Inbound,
        "out" => Direction::Outbound,
        another => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                format!("direção desconhecida: `{another}`").into(),
            ));
        }
    };
    let role = Role::from_text(&row.get::<_, String>(5)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let origin = Origin::from_text(&row.get::<_, String>(6)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(MessageSince {
        rowid: u64::try_from(row.get::<_, i64>(0)?).unwrap_or(0),
        id: column_id(row, 1)?,
        org_id: column_id(row, 2)?,
        conversation_id: column_id(row, 3)?,
        direction,
        role,
        origin,
        author_name: row.get(7)?,
        texto: row.get(8)?,
        attachments_json: row.get(9)?,
        external_id: row.get(10)?,
        created_at: row.get(11)?,
    })
}

pub(crate) fn row(row: &Row<'_>) -> rusqlite::Result<Mensagem> {
    let direction = match row.get::<_, String>(3)?.as_str() {
        "in" => Direction::Inbound,
        "out" => Direction::Outbound,
        another => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                format!("direção desconhecida: `{another}`").into(),
            ));
        }
    };
    let role = Role::from_text(&row.get::<_, String>(4)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let origin = Origin::from_text(&row.get::<_, String>(10)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(10, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(Mensagem {
        id: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        conversation_id: column_id(row, 2)?,
        direction,
        role,
        origin,
        author_name: row.get(11)?,
        texto: row.get(5)?,
        attachments_json: row.get(6)?,
        external_id: row.get(7)?,
        turn_id: id_optional::<TurnId>(row, 8)?,
        created_at: row.get(9)?,
    })
}
