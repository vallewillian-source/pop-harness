use poph_types::{AuditId, ConversationId, InboxId, IntegrationId, OrgId, OutboxId, TurnId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{
    AttachmentRecord, InboxItem, Mensagem, NewChannelInboxItem, NewDeviceMessage, NewInboxItem,
    NewInboxItemWithMessage, NewOutboxItem, Origin, OutboxItem, OutboxState, Received,
    ReceivedFromDevice, ReceivedWithMessage, Role,
};
use crate::repos::column_id;
use crate::repos::conversations::write_name_client;
use crate::repos::messages::Messages;
use poph_types::MessageId;

const COLUMNS_ENTRY: &str = "id, org_id, integration_id, external_message_id, payload_json,
    received_at, processed_at, attempts, last_error";

pub(crate) const COLUMNS_OUTPUT: &str =
    "id, org_id, conversation_id, turn_id, seq, kind, payload_json, state, attempts, last_error,
     created_at, sent_at";

pub(crate) fn new_output_item(
    id: OutboxId,
    org: OrgId,
    fresh: NewOutboxItem,
    created_at: i64,
) -> OutboxItem {
    OutboxItem {
        id,
        org_id: org,
        conversation_id: fresh.conversation_id,
        turn_id: fresh.turn_id,
        seq: fresh.seq,
        tipo: fresh.tipo,
        content_json: fresh.content_json,
        state: OutboxState::Pending,
        attempts: 0,
        ultimo_erro: None,
        created_at,
        enviado_em: None,
    }
}

pub struct Inbox<'a>(pub(crate) &'a Store);

impl Inbox<'_> {
    pub fn receive(&self, org: OrgId, fresh: NewInboxItem) -> Resultado<(Received, InboxItem)> {
        let id = InboxId::generate();
        let now = self.0.now();
        let text_org = org.to_string();
        let NewInboxItem {
            integration_id,
            external_id,
            event_json,
        } = fresh;
        let text_integration = integration_id.to_string();
        self.0.write(move |conn| {
            let inseriu = conn
                .execute(
                    "INSERT OR IGNORE INTO inbox
                        (id, org_id, integration_id, external_message_id, payload_json,
                         received_at, processed_at, attempts, last_error)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 0, NULL)",
                    params![
                        id.to_string(),
                        &text_org,
                        &text_integration,
                        &external_id,
                        &event_json,
                        now
                    ],
                )
                .map_err(StoreError::from)?;

            if inseriu == 0 {
                let existing = conn
                    .query_row(
                        &format!(
                            "SELECT {COLUMNS_ENTRY} FROM inbox
                              WHERE integration_id = ?1 AND external_message_id = ?2"
                        ),
                        params![&text_integration, &external_id],
                        entry_row,
                    )
                    .optional()
                    .map_err(StoreError::from)?;
                return match existing {
                    Some(item) => Ok((Received::Duplicate, item)),
                    None => Err(StoreError::Internal(
                        "inserção da entrada ignorada, mas a original não foi encontrada"
                            .to_owned(),
                    )),
                };
            }

            Ok((
                Received::New,
                InboxItem {
                    id,
                    org_id: org,
                    integration_id,
                    external_id,
                    event_json,
                    received_at: now,
                    processed_at: None,
                    attempts: 0,
                    ultimo_erro: None,
                },
            ))
        })
    }

    pub fn mark_processed(&self, org: OrgId, id: InboxId, error: Option<&str>) -> Resultado<()> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_id = id.to_string();
        let error = error.map(str::to_owned);
        self.0.write(move |conn| {
            conn.execute(
                "UPDATE inbox
                    SET processed_at = ?3, attempts = attempts + 1, last_error = ?4
                  WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id, now, error],
            )
            .map(|_| ())
            .map_err(StoreError::from)
        })
    }

    pub fn receive_with_message(
        &self,
        org: OrgId,
        fresh: NewInboxItemWithMessage,
    ) -> Resultado<ReceivedWithMessage> {
        let id = InboxId::generate();
        let now = self.0.now();
        let text_org = org.to_string();
        let NewInboxItemWithMessage {
            integration_id,
            external_id,
            event_json,
            conversation_id,
            message_id,
            texto,
            attachments_json,
        } = fresh;
        let text_integration = integration_id.to_string();
        let text_conversation = conversation_id.to_string();
        self.0.write(move |conn| {
            let inseriu = conn
                .execute(
                    "INSERT OR IGNORE INTO inbox
                        (id, org_id, integration_id, external_message_id, payload_json,
                         received_at, processed_at, attempts, last_error)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 0, NULL)",
                    params![
                        id.to_string(),
                        &text_org,
                        &text_integration,
                        &external_id,
                        &event_json,
                        now
                    ],
                )
                .map_err(StoreError::from)?;

            conn.execute(
                "INSERT OR IGNORE INTO messages
                    (id, org_id, conversation_id, direction, role, text, attachments_json,
                     external_id, turn_id, created_at)
                 VALUES (?1, ?2, ?3, 'in', ?4, ?5, ?6, ?7, NULL, ?8)",
                params![
                    message_id.to_string(),
                    &text_org,
                    &text_conversation,
                    Role::User.as_text(),
                    &texto,
                    &attachments_json,
                    &external_id,
                    now
                ],
            )
            .map_err(StoreError::from)?;

            let mensagem = conn
                .query_row(
                    "SELECT id, org_id, conversation_id, direction, role, text, attachments_json,
                            external_id, turn_id, created_at, origin, author_name
                       FROM messages
                      WHERE org_id = ?1 AND conversation_id = ?2 AND external_id = ?3",
                    params![&text_org, &text_conversation, &external_id],
                    crate::repos::messages::row,
                )
                .optional()
                .map_err(StoreError::from)?
                .ok_or_else(|| {
                    StoreError::Internal(
                        "mensagem recém-gravada não encontrada na mesma transação".to_owned(),
                    )
                })?;

            if inseriu == 0 {
                let existing = conn
                    .query_row(
                        &format!(
                            "SELECT {COLUMNS_ENTRY} FROM inbox
                              WHERE integration_id = ?1 AND external_message_id = ?2"
                        ),
                        params![&text_integration, &external_id],
                        entry_row,
                    )
                    .optional()
                    .map_err(StoreError::from)?
                    .ok_or_else(|| {
                        StoreError::Internal(
                            "inserção da entrada ignorada, mas a original não foi encontrada"
                                .to_owned(),
                        )
                    })?;
                return Ok(ReceivedWithMessage {
                    situation: Received::Duplicate,
                    item: existing,
                    mensagem,
                });
            }

            Ok(ReceivedWithMessage {
                situation: Received::New,
                item: InboxItem {
                    id,
                    org_id: org,
                    integration_id,
                    external_id,
                    event_json,
                    received_at: now,
                    processed_at: None,
                    attempts: 0,
                    ultimo_erro: None,
                },
                mensagem,
            })
        })
    }

    pub fn count_pending(&self, org: OrgId) -> Resultado<u64> {
        let text_org = org.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM inbox WHERE org_id = ?1 AND processed_at IS NULL",
                params![text_org],
                |row| row.get::<_, i64>(0),
            )
            .map(|total| total.max(0) as u64)
            .map_err(StoreError::from)
        })
    }

    pub fn list_pending(&self, org: OrgId, limit: u32) -> Resultado<Vec<InboxItem>> {
        self.list(org, limit, true)
    }

    pub fn list_opens(&self, org: OrgId, limit: u32) -> Resultado<Vec<InboxItem>> {
        self.list(org, limit, false)
    }

    fn list(&self, org: OrgId, limit: u32, so_vencidos: bool) -> Resultado<Vec<InboxItem>> {
        let text_org = org.to_string();
        let limit = i64::from(limit.clamp(1, 1000));
        let now = self.0.now();
        let filter_by = i64::from(so_vencidos);
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS_ENTRY} FROM inbox
                  WHERE org_id = ?1 AND processed_at IS NULL
                    AND (?3 = 0 OR next_attempt_at IS NULL OR next_attempt_at <= ?4)
                  ORDER BY received_at, id LIMIT ?2"
            ))?;
            let rows = statement.query_map(params![text_org, limit, filter_by, now], entry_row)?;
            let mut items = Vec::new();
            for row in rows {
                items.push(row?);
            }
            Ok(items)
        })
    }

    pub fn adiar(&self, org: OrgId, ids: &[InboxId], next_at: i64, error: &str) -> Resultado<()> {
        let text_org = org.to_string();
        let texts: Vec<String> = ids.iter().map(ToString::to_string).collect();
        let error = error.to_owned();
        self.0.write(move |conn| {
            for text_id in &texts {
                conn.execute(
                    "UPDATE inbox
                        SET attempts = attempts + 1, next_attempt_at = ?3, last_error = ?4
                      WHERE org_id = ?1 AND id = ?2 AND processed_at IS NULL",
                    params![text_org, text_id, next_at, error],
                )
                .map_err(StoreError::from)?;
            }
            Ok(())
        })
    }

    pub fn attempts(&self, org: OrgId, ids: &[InboxId]) -> Resultado<Vec<u32>> {
        let mut total = Vec::with_capacity(ids.len());
        for id in ids {
            total.push(self.get(org, *id)?.attempts);
        }
        Ok(total)
    }

    pub fn get(&self, org: OrgId, id: InboxId) -> Resultado<InboxItem> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let located = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS_ENTRY} FROM inbox WHERE org_id = ?1 AND id = ?2"),
                params![text_org, text_id],
                entry_row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(located, "item de entrada", &id.to_string())
    }
}

pub struct Outbox<'a>(pub(crate) &'a Store);

impl Outbox<'_> {
    pub fn count_turn(&self, org: OrgId, turno: TurnId) -> Resultado<u64> {
        let text_org = org.to_string();
        let text_turn = turno.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM outbox WHERE org_id = ?1 AND turn_id = ?2",
                params![text_org, text_turn],
                |row| row.get::<_, i64>(0),
            )
            .map(|total| total.max(0) as u64)
            .map_err(StoreError::from)
        })
    }

    pub fn enqueue(&self, org: OrgId, fresh: NewOutboxItem) -> Resultado<(bool, OutboxItem)> {
        let id = OutboxId::generate();
        let now = self.0.now();
        let text_org = org.to_string();
        let NewOutboxItem {
            conversation_id,
            turn_id,
            seq,
            tipo,
            content_json,
        } = fresh;
        let text_turn = turn_id.to_string();
        self.0.write(move |conn| {
            let inseriu = conn
                .execute(
                    "INSERT OR IGNORE INTO outbox
                        (id, org_id, conversation_id, turn_id, seq, kind, payload_json, state,
                         attempts, last_error, created_at, sent_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', 0, NULL, ?8, NULL)",
                    params![
                        id.to_string(),
                        &text_org,
                        conversation_id.to_string(),
                        &text_turn,
                        i64::from(seq),
                        tipo,
                        &content_json,
                        now
                    ],
                )
                .map_err(StoreError::from)?;

            if inseriu == 0 {
                let existing = conn
                    .query_row(
                        &format!(
                            "SELECT {COLUMNS_OUTPUT} FROM outbox WHERE turn_id = ?1 AND seq = ?2"
                        ),
                        params![&text_turn, i64::from(seq)],
                        output_row,
                    )
                    .optional()
                    .map_err(StoreError::from)?;
                return match existing {
                    Some(item) => Ok((false, item)),
                    None => Err(StoreError::Internal(
                        "inserção da saída ignorada, mas a original não foi encontrada".to_owned(),
                    )),
                };
            }

            Ok((
                true,
                new_output_item(
                    id,
                    org,
                    NewOutboxItem {
                        conversation_id,
                        turn_id,
                        seq,
                        tipo,
                        content_json,
                    },
                    now,
                ),
            ))
        })
    }

    pub fn change_state(
        &self,
        org: OrgId,
        id: OutboxId,
        state: OutboxState,
        error: Option<&str>,
    ) -> Resultado<()> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_id = id.to_string();
        let error = error.map(str::to_owned);
        let sent = matches!(state, OutboxState::Sent).then_some(now);
        self.0.write(move |conn| {
            conn.execute(
                "UPDATE outbox
                    SET state = ?3, attempts = attempts + 1, last_error = ?4,
                        sent_at = COALESCE(?5, sent_at)
                  WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id, state.as_text(), error, sent],
            )
            .map(|_| ())
            .map_err(StoreError::from)
        })
    }

    pub fn start_send(&self, org: OrgId, id: OutboxId) -> Resultado<u32> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        self.0.write(move |conn| {
            let attempts = conn
                .query_row(
                    "UPDATE outbox
                        SET state = 'sending', attempts = attempts + 1, last_error = NULL
                      WHERE org_id = ?1 AND id = ?2 AND state IN ('pending', 'sending')
                      RETURNING attempts",
                    params![text_org, text_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(StoreError::from)?
                .ok_or_else(|| {
                    StoreError::Internal(format!(
                        "item de saída `{text_id}` não estava aguardando envio"
                    ))
                })?;
            Ok(attempts.max(0) as u32)
        })
    }

    pub fn confirm_send(&self, org: OrgId, id: OutboxId) -> Resultado<()> {
        self.confirm_send_with_id(org, id, None)
    }

    pub fn confirm_send_with_id(
        &self,
        org: OrgId,
        id: OutboxId,
        external_id: Option<&str>,
    ) -> Resultado<()> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_id = id.to_string();
        let external_id = external_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        self.0.write(move |conn| {
            conn.execute(
                "UPDATE outbox
                    SET state = 'sent', sent_at = ?3, last_error = NULL,
                        external_message_id = COALESCE(?4, external_message_id)
                  WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id, now, external_id],
            )
            .map(|_| ())
            .map_err(StoreError::from)
        })
    }

    pub fn fail_send(
        &self,
        org: OrgId,
        id: OutboxId,
        motivo: &str,
        definitivo: bool,
    ) -> Resultado<()> {
        let state = if definitivo {
            OutboxState::Failed
        } else {
            OutboxState::Sending
        };
        let text_org = org.to_string();
        let text_id = id.to_string();
        let motivo = motivo.to_owned();
        self.0.write(move |conn| {
            conn.execute(
                "UPDATE outbox SET state = ?3, last_error = ?4 WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id, state.as_text(), motivo],
            )
            .map(|_| ())
            .map_err(StoreError::from)
        })
    }

    pub fn count_by_state(&self, org: OrgId, state: OutboxState) -> Resultado<u64> {
        let text_org = org.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM outbox WHERE org_id = ?1 AND state = ?2",
                params![text_org, state.as_text()],
                |row| row.get::<_, i64>(0),
            )
            .map(|total| total.max(0) as u64)
            .map_err(StoreError::from)
        })
    }

    pub fn list_pending(&self, org: OrgId, limit: u32) -> Resultado<Vec<OutboxItem>> {
        let text_org = org.to_string();
        let limit = i64::from(limit.clamp(1, 1000));
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS_OUTPUT} FROM outbox
                  WHERE org_id = ?1 AND state IN ('pending', 'sending')
                  ORDER BY created_at, id LIMIT ?2"
            ))?;
            let rows = statement.query_map(params![text_org, limit], output_row)?;
            let mut items = Vec::new();
            for row in rows {
                items.push(row?);
            }
            Ok(items)
        })
    }

    pub fn external_id(&self, org: OrgId, id: OutboxId) -> Resultado<Option<String>> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT external_message_id FROM outbox WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map(|value| value.flatten())
            .map_err(StoreError::from)
        })
    }

    pub fn sent_with_external_id(&self, org: OrgId, external_id: &str) -> Resultado<bool> {
        let text_org = org.to_string();
        let external_id = external_id.trim().to_owned();
        self.0.read(move |conn| {
            let finding: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM outbox
                      WHERE org_id = ?1 AND external_message_id = ?2
                      LIMIT 1",
                    params![text_org, external_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            Ok(finding.is_some())
        })
    }

    pub fn list_files_of_conversation(
        &self,
        org: OrgId,
        conversation: ConversationId,
        limit: u32,
    ) -> Resultado<Vec<OutboxItem>> {
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        let limit = i64::from(limit.clamp(1, 1000));
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS_OUTPUT} FROM outbox
                  WHERE org_id = ?1 AND conversation_id = ?2 AND kind = 'file'
                  ORDER BY created_at, id LIMIT ?3"
            ))?;
            let rows =
                statement.query_map(params![text_org, text_conversation, limit], output_row)?;
            let mut items = Vec::new();
            for row in rows {
                items.push(row?);
            }
            Ok(items)
        })
    }

    pub fn get(&self, org: OrgId, id: OutboxId) -> Resultado<OutboxItem> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let located = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS_OUTPUT} FROM outbox WHERE org_id = ?1 AND id = ?2"),
                params![text_org, text_id],
                output_row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(located, "item de saída", &id.to_string())
    }
}

fn entry_row(row: &Row<'_>) -> rusqlite::Result<InboxItem> {
    Ok(InboxItem {
        id: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        integration_id: column_id::<IntegrationId>(row, 2)?,
        external_id: row.get(3)?,
        event_json: row.get(4)?,
        received_at: row.get(5)?,
        processed_at: row.get(6)?,
        attempts: row.get::<_, u32>(7)?,
        ultimo_erro: row.get(8)?,
    })
}

pub(crate) fn output_row(row: &Row<'_>) -> rusqlite::Result<OutboxItem> {
    Ok(OutboxItem {
        id: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        conversation_id: column_id::<ConversationId>(row, 2)?,
        turn_id: column_id::<TurnId>(row, 3)?,
        seq: row.get::<_, u32>(4)?,
        tipo: row.get(5)?,
        content_json: row.get(6)?,
        state: OutboxState::from_text(&row.get::<_, String>(7)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        attempts: row.get(8)?,
        ultimo_erro: row.get(9)?,
        created_at: row.get(10)?,
        enviado_em: row.get(11)?,
    })
}

impl Inbox<'_> {
    pub fn delivery_received(
        &self,
        integration: poph_types::IntegrationId,
        external_id: &str,
    ) -> Resultado<Option<ReceivedWithMessage>> {
        let text_integration = integration.to_string();
        let external_id = external_id.to_owned();
        self.0.read(move |conn| {
            let item = conn
                .query_row(
                    &format!(
                        "SELECT {COLUMNS_ENTRY} FROM inbox
                          WHERE integration_id = ?1 AND external_message_id = ?2"
                    ),
                    params![&text_integration, &external_id],
                    entry_row,
                )
                .optional()
                .map_err(StoreError::from)?;
            let Some(item) = item else {
                return Ok(None);
            };
            let Some(mensagem) = delivery_message(conn, &text_integration, &external_id)? else {
                return Ok(None);
            };
            Ok(Some(ReceivedWithMessage {
                situation: Received::Duplicate,
                item,
                mensagem,
            }))
        })
    }

    pub fn receive_from_channel(
        &self,
        org: OrgId,
        fresh: NewChannelInboxItem,
    ) -> Resultado<ReceivedWithMessage> {
        let id_entry = InboxId::generate();
        let now = self.0.now();
        let text_org = org.to_string();
        let NewChannelInboxItem {
            integration_id,
            agent_id,
            canal,
            external_id,
            peer,
            event_json,
            texto,
            attachments,
            customer_name: name_client,
        } = fresh;
        let text_integration = integration_id.to_string();
        self.0.write(move |conn| {
            let inseriu = conn
                .execute(
                    "INSERT OR IGNORE INTO inbox
                        (id, org_id, integration_id, external_message_id, payload_json,
                         received_at, processed_at, attempts, last_error)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 0, NULL)",
                    params![
                        id_entry.to_string(),
                        &text_org,
                        &text_integration,
                        &external_id,
                        &event_json,
                        now
                    ],
                )
                .map_err(StoreError::from)?;

            if inseriu == 0 {
                let item = conn
                    .query_row(
                        &format!(
                            "SELECT {COLUMNS_ENTRY} FROM inbox
                              WHERE integration_id = ?1 AND external_message_id = ?2"
                        ),
                        params![&text_integration, &external_id],
                        entry_row,
                    )
                    .map_err(StoreError::from)?;
                let mensagem = delivery_message(conn, &text_integration, &external_id)?
                    .ok_or_else(|| {
                        StoreError::Internal(
                            "entrada já gravada sem a mensagem correspondente".to_owned(),
                        )
                    })?;
                return Ok(ReceivedWithMessage {
                    situation: Received::Duplicate,
                    item,
                    mensagem,
                });
            }

            let conversation_id = conversation_from_contact(
                conn,
                &text_org,
                &text_integration,
                &agent_id.to_string(),
                &canal,
                &peer,
                now,
            )?;

            if let Some(nome) = name_client.as_deref() {
                write_name_without_dropping(conn, &text_org, &conversation_id, nome, now);
            }

            let message_id = poph_types::MessageId::generate().to_string();
            conn.execute(
                "INSERT INTO messages
                    (id, org_id, conversation_id, direction, role, text, attachments_json,
                     external_id, turn_id, created_at)
                 VALUES (?1, ?2, ?3, 'in', ?4, ?5, '[]', ?6, NULL, ?7)",
                params![
                    &message_id,
                    &text_org,
                    &conversation_id,
                    Role::User.as_text(),
                    &texto,
                    &external_id,
                    now
                ],
            )
            .map_err(StoreError::from)?;

            let turn_payload =
                payload_of_channel_entry(&conversation_id, &message_id, &event_json)?;
            conn.execute(
                "UPDATE inbox SET payload_json = ?2 WHERE id = ?1",
                params![id_entry.to_string(), turn_payload],
            )
            .map_err(StoreError::from)?;

            let mut attachment_ids: Vec<String> = Vec::with_capacity(attachments.len());
            for attachment in &attachments {
                let attachment_id = poph_types::AttachmentId::generate().to_string();
                conn.execute(
                    "INSERT INTO attachments
                        (id, org_id, message_id, name, mime, size_bytes, sha256, blob_path,
                         extracted_text, extraction_status, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        &attachment_id,
                        &text_org,
                        &message_id,
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
                attachment_ids.push(attachment_id);
            }
            let attachments_json = serde_json::to_string(&attachment_ids).map_err(|error| {
                StoreError::Internal(format!("lista de anexos não serializa: {error}"))
            })?;
            conn.execute(
                "UPDATE messages SET attachments_json = ?2 WHERE id = ?1",
                params![&message_id, attachments_json],
            )
            .map_err(StoreError::from)?;
            conn.execute(
                "UPDATE conversations SET last_message_at = ?2, updated_at = ?2 WHERE id = ?1",
                params![&conversation_id, now],
            )
            .map_err(StoreError::from)?;

            let mensagem = conn
                .query_row(
                    "SELECT id, org_id, conversation_id, direction, role, text, attachments_json,
                            external_id, turn_id, created_at, origin, author_name
                       FROM messages WHERE id = ?1",
                    params![&message_id],
                    crate::repos::messages::row,
                )
                .map_err(StoreError::from)?;

            Ok(ReceivedWithMessage {
                situation: Received::New,
                item: InboxItem {
                    id: id_entry,
                    org_id: org,
                    integration_id,
                    external_id,
                    event_json,
                    received_at: now,
                    processed_at: None,
                    attempts: 0,
                    ultimo_erro: None,
                },
                mensagem,
            })
        })
    }

    pub fn receive_from_device(
        &self,
        org: OrgId,
        fresh: NewDeviceMessage,
    ) -> Resultado<ReceivedFromDevice> {
        let now = self.0.now();
        let NewDeviceMessage {
            integration_id,
            agent_id,
            canal,
            peer,
            external_id,
            texto,
        } = fresh;
        let text_org = org.to_string();
        let text_integration = integration_id.to_string();
        let text_agent = agent_id.to_string();
        self.0.write(move |conn| {
            let already_stored: Option<(String, String)> = conn
                .query_row(
                    "SELECT m.id, m.conversation_id
                       FROM messages m
                       JOIN conversations c ON c.org_id = m.org_id AND c.id = m.conversation_id
                      WHERE m.org_id = ?1 AND m.external_id = ?2 AND c.integration_id = ?3
                      LIMIT 1",
                    params![&text_org, &external_id, &text_integration],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(StoreError::from)?;
            if let Some((message_id, conversation_id)) = already_stored {
                let mensagem = conn
                    .query_row(
                        "SELECT id, org_id, conversation_id, direction, role, text,
                                attachments_json, external_id, turn_id, created_at, origin,
                                author_name
                           FROM messages WHERE id = ?1",
                        params![&message_id],
                        crate::repos::messages::row,
                    )
                    .map_err(StoreError::from)?;
                return Ok(ReceivedFromDevice {
                    situation: Received::Duplicate,
                    conversation: conversation_of_text(&conversation_id)?,
                    mensagem,
                    just_transferred: false,
                });
            }

            let conversation = conversation_from_contact(
                conn,
                &text_org,
                &text_integration,
                &text_agent,
                &canal,
                &peer,
                now,
            )?;

            let message_id = poph_types::MessageId::generate().to_string();
            conn.execute(
                "INSERT INTO messages
                    (id, org_id, conversation_id, direction, role, text, attachments_json,
                     external_id, turn_id, created_at, origin, author_name)
                 VALUES (?1, ?2, ?3, 'out', ?4, ?5, '[]', ?6, NULL, ?7, ?8, NULL)",
                params![
                    &message_id,
                    &text_org,
                    &conversation,
                    Role::Assistant.as_text(),
                    &texto,
                    &external_id,
                    now,
                    Origin::Device.as_text()
                ],
            )
            .map_err(StoreError::from)?;

            let just_transferred = conn
                .execute(
                    "UPDATE conversations
                        SET status = 'transferred', version = version + 1,
                            last_message_at = ?3, updated_at = ?3
                      WHERE org_id = ?1 AND id = ?2 AND status = 'open'",
                    params![&text_org, &conversation, now],
                )
                .map_err(StoreError::from)?
                > 0;
            if !just_transferred {
                conn.execute(
                    "UPDATE conversations SET last_message_at = ?3, updated_at = ?3
                      WHERE org_id = ?1 AND id = ?2",
                    params![&text_org, &conversation, now],
                )
                .map_err(StoreError::from)?;
            }

            let detail = serde_json::json!({
                "transferiu": just_transferred,
                "mensagem_id": message_id,
            });
            conn.execute(
                "INSERT INTO audit_log
                    (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json,
                     created_at)
                 VALUES (?1, ?2, 'system', NULL, 'device_takeover', 'conversation', ?3, ?4, ?5)",
                params![
                    AuditId::generate().to_string(),
                    &text_org,
                    &conversation,
                    detail.to_string(),
                    now
                ],
            )
            .map_err(StoreError::from)?;

            let mensagem = conn
                .query_row(
                    "SELECT id, org_id, conversation_id, direction, role, text, attachments_json,
                            external_id, turn_id, created_at, origin, author_name
                       FROM messages WHERE id = ?1",
                    params![&message_id],
                    crate::repos::messages::row,
                )
                .map_err(StoreError::from)?;

            Ok(ReceivedFromDevice {
                situation: Received::New,
                conversation: conversation_of_text(&conversation)?,
                mensagem,
                just_transferred,
            })
        })
    }
}

fn conversation_from_contact(
    conn: &rusqlite::Connection,
    org: &str,
    integration: &str,
    agent: &str,
    canal: &str,
    peer: &str,
    now: i64,
) -> Resultado<String> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM conversations
              WHERE org_id = ?1 AND integration_id = ?2 AND external_peer = ?3
                AND status IN ('open', 'transferred')
              ORDER BY updated_at DESC, id LIMIT 1",
            params![org, integration, peer],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(StoreError::from)?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let id = ConversationId::generate().to_string();
    conn.execute(
        "INSERT INTO conversations
            (id, org_id, agent_id, integration_id, channel, external_peer, status,
             target_department_id, summary, version, created_at, updated_at,
             last_message_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'open',
             (SELECT default_department_id FROM agents
               WHERE org_id = ?2 AND id = ?3),
             NULL, 1, ?7, ?7, NULL)",
        params![&id, org, agent, integration, canal, peer, now],
    )
    .map_err(StoreError::from)?;
    super::cases::create_case_for_conversation(conn, org, &id, agent, canal, Some(peer), now)?;
    Ok(id)
}

fn conversation_of_text(value: &str) -> Resultado<ConversationId> {
    value
        .parse()
        .map_err(|_| StoreError::Internal("conversa gravada sem id válido".to_owned()))
}

fn write_name_without_dropping(
    conn: &rusqlite::Connection,
    org: &str,
    conversation: &str,
    nome: &str,
    now: i64,
) {
    if let Err(error) = conn.execute_batch("SAVEPOINT nome_cliente") {
        let code = error_code_name(&StoreError::Sqlite(error));
        tracing::warn!(target: "poph_store", org, conversation, code, "não consegui abrir o savepoint do nome do cliente; a mensagem segue");
        return;
    }
    match write_name_client(conn, org, conversation, nome, now) {
        Ok(_) => {
            if let Err(error) = conn.execute_batch("RELEASE nome_cliente") {
                let code = error_code_name(&StoreError::Sqlite(error));
                tracing::warn!(target: "poph_store", org, conversation, code, "não consegui fechar o savepoint do nome do cliente; a mensagem segue");
            }
        }
        Err(error) => {
            let code = error_code_name(&error);
            let _ = conn.execute_batch("ROLLBACK TO nome_cliente; RELEASE nome_cliente");
            tracing::warn!(target: "poph_store", org, conversation, code, "não consegui gravar o nome do cliente; a mensagem segue");
        }
    }
}

fn error_code_name(error: &StoreError) -> &'static str {
    match error {
        StoreError::Domain(_) => "dominio",
        StoreError::Sqlite(_) => "banco",
        StoreError::WriterClosed => "escritor_encerrado",
        _ => "armazenamento",
    }
}

fn payload_of_channel_entry(
    conversation: &str,
    mensagem: &str,
    event_json: &str,
) -> Resultado<String> {
    let canal: serde_json::Value = serde_json::from_str(event_json).map_err(|error| {
        StoreError::Internal(format!("evento do canal não lê como JSON: {error}"))
    })?;
    serde_json::to_string(&serde_json::json!({
        "conversa": conversation,
        "mensagem": mensagem,
        "canal": canal,
    }))
    .map_err(|error| StoreError::Internal(format!("payload da entrada não serializa: {error}")))
}

fn delivery_message(
    conn: &rusqlite::Connection,
    integration: &str,
    external_id: &str,
) -> Resultado<Option<Mensagem>> {
    conn.query_row(
        "SELECT m.id, m.org_id, m.conversation_id, m.direction, m.role, m.text, m.attachments_json,
                m.external_id, m.turn_id, m.created_at, m.origin, m.author_name
           FROM messages m
           JOIN conversations c ON c.id = m.conversation_id
          WHERE c.integration_id = ?1 AND m.external_id = ?2 AND m.direction = 'in'
          LIMIT 1",
        params![integration, external_id],
        crate::repos::messages::row,
    )
    .optional()
    .map_err(StoreError::from)
}

impl Messages<'_> {
    pub fn attachments(&self, org: OrgId, mensagem: MessageId) -> Resultado<Vec<AttachmentRecord>> {
        let text_org = org.to_string();
        let text_message = mensagem.to_string();
        self.0.read(move |conn| {
            let mut statement = conn.prepare(
                "SELECT id, message_id, name, mime, size_bytes, sha256, blob_path,
                        extraction_status, extracted_text
                   FROM attachments
                  WHERE org_id = ?1 AND message_id = ?2
                  ORDER BY created_at, id",
            )?;
            let rows = statement.query_map(params![text_org, text_message], |row| {
                Ok(AttachmentRecord {
                    id: column_id(row, 0)?,
                    message_id: column_id(row, 1)?,
                    name: row.get(2)?,
                    mime: row.get(3)?,
                    size_bytes: row.get::<_, i64>(4)?.max(0) as u64,
                    sha256: row.get(5)?,
                    blob_path: row.get(6)?,
                    extraction_status: row.get(7)?,
                    extracted_text: row.get(8)?,
                })
            })?;
            let mut attachments = Vec::new();
            for row in rows {
                attachments.push(row?);
            }
            Ok(attachments)
        })
    }
}
