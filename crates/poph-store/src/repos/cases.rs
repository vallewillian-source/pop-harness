use poph_types::{
    ClosedBy, ConversationId, EventActor, ExecutionId, ExecutionStatus, OrgId, RecordEventId,
    RecordEventKind, RecordId, RecordStatus,
};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::metrics::ExecutionMetrics;
use crate::model::{Ficha, NewRecord, NewRecordEvent, Page, RecordEvent, RecordFilter};
use crate::repos::{column_id, column_u64, id_optional, require};

const COLUMNS: &str =
    "id, org_id, conversation_id, agent_id, canal, peer_externo, status, aberta_em,
    fechada_em, fechada_por, fechada_motivo, version";

const COLUMNS_EVENT: &str = "id, org_id, ficha_id, execucao_id, seq, tipo, ator_tipo, ator_ref,
    payload_json, chave_idempotencia, created_at";

pub const REASON_ABANDONMENT_SERVICE: &str = "atendimento_encerrado";

pub struct Fichas<'a>(pub(crate) &'a Store);

impl Fichas<'_> {
    pub fn create(&self, org: OrgId, fresh: NewRecord) -> Resultado<Ficha> {
        let id = RecordId::generate();
        let now = self.0.now();
        let text_org = org.to_string();
        let text_id = id.to_string();
        let conversation = fresh
            .conversation_id
            .map(|conversation| conversation.to_string());
        let agent = fresh.agent_id.to_string();
        let canal = fresh.canal;
        let peer = fresh.peer_externo;
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO fichas
                    (id, org_id, conversation_id, agent_id, canal, peer_externo, status, aberta_em,
                     fechada_em, fechada_por, fechada_motivo, version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'open', ?7, NULL, NULL, NULL, 1)",
                params![text_id, text_org, conversation, agent, canal, peer, now],
            )
            .map_err(|error| StoreError::from_write("conversa", error))?;
            register_event_conn(
                conn,
                &text_org,
                &text_id,
                &NewRecordEvent {
                    execucao_id: None,
                    tipo: RecordEventKind::RecordOpened,
                    ator_tipo: EventActor::System,
                    ator_ref: None,
                    payload_json: "{}".to_owned(),
                    chave_idempotencia: None,
                },
                now,
            )?;
            Ok(())
        })?;
        self.get(org, id)
    }

    pub fn get(&self, org: OrgId, id: RecordId) -> Resultado<Ficha> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let found = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS} FROM fichas WHERE org_id = ?1 AND id = ?2"),
                params![text_org, text_id],
                row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        require(found, "ficha", &id.to_string())
    }

    pub fn by_conversation(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Resultado<Option<Ficha>> {
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS} FROM fichas WHERE org_id = ?1 AND conversation_id = ?2"),
                params![text_org, text_conversation],
                row,
            )
            .optional()
            .map_err(StoreError::from)
        })
    }

    pub fn list(&self, org: OrgId, filter: &RecordFilter, page: Page) -> Resultado<Vec<Ficha>> {
        let mut conditions = vec!["org_id = ?1".to_owned()];
        let mut values: Vec<rusqlite::types::Value> =
            vec![rusqlite::types::Value::Text(org.to_string())];
        if let Some(status) = filter.status {
            values.push(rusqlite::types::Value::Text(status.as_text().to_owned()));
            conditions.push(format!("status = ?{}", values.len()));
        }
        if let Some(agent) = filter.agent {
            values.push(rusqlite::types::Value::Text(agent.to_string()));
            conditions.push(format!("agent_id = ?{}", values.len()));
        }
        if let Some(canal) = &filter.canal {
            values.push(rusqlite::types::Value::Text(canal.clone()));
            conditions.push(format!("canal = ?{}", values.len()));
        }
        if let Some(since) = filter.since {
            values.push(rusqlite::types::Value::Integer(since));
            conditions.push(format!("aberta_em >= ?{}", values.len()));
        }
        if let Some(after) = page.after {
            values.push(rusqlite::types::Value::Text(after.to_string()));
            conditions.push(format!("id > ?{}", values.len()));
        }
        values.push(rusqlite::types::Value::Integer(i64::from(page.limit)));
        let sql = format!(
            "SELECT {COLUMNS} FROM fichas WHERE {} ORDER BY id LIMIT ?{}",
            conditions.join(" AND "),
            values.len()
        );
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&sql)?;
            let rows = statement.query_map(rusqlite::params_from_iter(values.iter()), row)?;
            let mut cases = Vec::new();
            for row in rows {
                cases.push(row?);
            }
            Ok(cases)
        })
    }

    pub fn events(
        &self,
        org: OrgId,
        case: RecordId,
        after: Option<i64>,
        limit: u32,
    ) -> Resultado<Vec<RecordEvent>> {
        self.get(org, case)?;
        let text_org = org.to_string();
        let text_case = case.to_string();
        let limit = i64::from(limit.clamp(1, 500));
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS_EVENT} FROM ficha_eventos
                  WHERE org_id = ?1 AND ficha_id = ?2 AND (?3 IS NULL OR seq > ?3)
                  ORDER BY seq LIMIT ?4"
            ))?;
            let rows =
                statement.query_map(params![text_org, text_case, after, limit], event_row)?;
            let mut events = Vec::new();
            for row in rows {
                events.push(row?);
            }
            Ok(events)
        })
    }

    pub fn close_without_conversation(
        &self,
        org: OrgId,
        case: RecordId,
        expected: u64,
        motivo: &str,
    ) -> Resultado<()> {
        let text_org = org.to_string();
        let text_case = case.to_string();
        let now = self.0.now();
        let motivo = motivo.to_owned();
        let metrics = self.0.execution_metrics();
        self.0.write(move |conn| {
            let row: Option<(String, Option<String>, i64)> = conn
                .query_row(
                    "SELECT status, conversation_id, version FROM fichas
                      WHERE org_id = ?1 AND id = ?2",
                    params![text_org, text_case],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(StoreError::from)?;
            let Some((status, conversation, version)) = row else {
                return Err(StoreError::Domain(poph_types::DomainError::NotFound {
                    entity: "ficha",
                    id: text_case.clone(),
                }));
            };
            if RecordStatus::from_text(&status).map_err(StoreError::from)? == RecordStatus::Closed {
                return Err(StoreError::Domain(poph_types::DomainError::RecordClosed {
                    id: text_case.clone(),
                }));
            }
            if conversation.is_some() {
                return Err(StoreError::Domain(poph_types::DomainError::Conflict {
                    entity: "ficha",
                    id: text_case.clone(),
                    reason: "a Ficha tem conversa ligada: feche o atendimento pela conversa"
                        .to_owned(),
                }));
            }
            let current = u64::try_from(version).unwrap_or(0);
            if current != expected {
                return Err(StoreError::Domain(
                    poph_types::DomainError::VersionConflict {
                        entity: "ficha",
                        id: text_case.clone(),
                        expected,
                        current,
                    },
                ));
            }
            close_case_by_id(
                conn,
                &text_org,
                &text_case,
                ClosedBy::Admin,
                &motivo,
                now,
                metrics.as_ref(),
            )?;
            Ok(())
        })
    }

    pub fn register_event(
        &self,
        org: OrgId,
        case: RecordId,
        new: NewRecordEvent<'_>,
    ) -> Resultado<RecordEvent> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_case = case.to_string();
        let execution = new.execucao_id;
        let tipo = new.tipo;
        let ator_tipo = new.ator_tipo;
        let ator_ref = new.ator_ref.map(str::to_owned);
        let payload_json = new.payload_json;
        let chave = new.chave_idempotencia.map(str::to_owned);
        let id = self.0.write(move |conn| {
            let situation = case_situation(conn, &text_org, &text_case)?;
            if situation.is_none() {
                return Err(StoreError::Domain(poph_types::DomainError::NotFound {
                    entity: "ficha",
                    id: text_case.clone(),
                }));
            }
            if let Some(chave) = chave.as_deref()
                && let Some(evento) = event_by_key(conn, &text_org, &text_case, chave)?
            {
                return Ok(evento.id.to_string());
            }
            if situation == Some(RecordStatus::Closed) {
                return Err(StoreError::Domain(poph_types::DomainError::RecordClosed {
                    id: text_case.clone(),
                }));
            }
            register_event_conn(
                conn,
                &text_org,
                &text_case,
                &NewRecordEvent {
                    execucao_id: execution,
                    tipo,
                    ator_tipo,
                    ator_ref: ator_ref.as_deref(),
                    payload_json,
                    chave_idempotencia: chave.as_deref(),
                },
                now,
            )
        })?;
        let text_org = org.to_string();
        let event_id: RecordEventId = id
            .parse()
            .map_err(|error: poph_types::DomainError| StoreError::Domain(error))?;
        self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS_EVENT} FROM ficha_eventos WHERE org_id = ?1 AND id = ?2"),
                params![text_org, event_id.to_string()],
                event_row,
            )
            .optional()
            .map_err(StoreError::from)?
            .ok_or_else(|| {
                StoreError::Internal("o evento foi gravado mas não voltou na leitura".to_owned())
            })
        })
    }
}

fn case_situation(conn: &Connection, org: &str, case: &str) -> Resultado<Option<RecordStatus>> {
    let texto: Option<String> = conn
        .query_row(
            "SELECT status FROM fichas WHERE org_id = ?1 AND id = ?2",
            params![org, case],
            |row| row.get(0),
        )
        .optional()
        .map_err(StoreError::from)?;
    texto
        .map(|texto| RecordStatus::from_text(&texto).map_err(StoreError::from))
        .transpose()
}

pub(crate) fn event_by_key(
    conn: &Connection,
    org: &str,
    case: &str,
    chave: &str,
) -> Resultado<Option<RecordEvent>> {
    conn.query_row(
        &format!(
            "SELECT {COLUMNS_EVENT} FROM ficha_eventos
              WHERE org_id = ?1 AND ficha_id = ?2 AND chave_idempotencia = ?3"
        ),
        params![org, case, chave],
        event_row,
    )
    .optional()
    .map_err(StoreError::from)
}

pub(crate) fn read_events_conn(
    conn: &Connection,
    org: &str,
    case: &str,
    after: i64,
) -> Resultado<Vec<RecordEvent>> {
    let mut statement = conn
        .prepare(&format!(
            "SELECT {COLUMNS_EVENT} FROM ficha_eventos
              WHERE org_id = ?1 AND ficha_id = ?2 AND seq > ?3
              ORDER BY seq"
        ))
        .map_err(StoreError::from)?;
    let rows = statement
        .query_map(params![org, case, after], event_row)
        .map_err(StoreError::from)?;
    let mut events = Vec::new();
    for row in rows {
        events.push(row.map_err(StoreError::from)?);
    }
    Ok(events)
}

pub(crate) fn last_seq(conn: &Connection, org: &str, case: &str) -> Resultado<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) FROM ficha_eventos WHERE org_id = ?1 AND ficha_id = ?2",
        params![org, case],
        |row| row.get(0),
    )
    .map_err(StoreError::from)
}

pub(crate) fn create_case_for_conversation(
    conn: &Connection,
    org: &str,
    conversation: &str,
    agent_id: &str,
    canal: &str,
    peer: Option<&str>,
    now: i64,
) -> Resultado<String> {
    let case = RecordId::generate().to_string();
    conn.execute(
        "INSERT INTO fichas
            (id, org_id, conversation_id, agent_id, canal, peer_externo, status, aberta_em,
             fechada_em, fechada_por, fechada_motivo, version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'open', ?7, NULL, NULL, NULL, 1)",
        params![case, org, conversation, agent_id, canal, peer, now],
    )
    .map_err(|error| StoreError::from_write("conversa", error))?;
    register_event_conn(
        conn,
        org,
        &case,
        &NewRecordEvent {
            execucao_id: None,
            tipo: RecordEventKind::RecordOpened,
            ator_tipo: EventActor::System,
            ator_ref: None,
            payload_json: "{}".to_owned(),
            chave_idempotencia: None,
        },
        now,
    )?;
    Ok(case)
}

pub(crate) fn close_case_for_conversation(
    conn: &Connection,
    org: &str,
    conversation: &str,
    by: ClosedBy,
    motivo: &str,
    now: i64,
    metrics: Option<&ExecutionMetrics>,
) -> Resultado<bool> {
    let situation: Option<(String, String)> = conn
        .query_row(
            "SELECT id, status FROM fichas WHERE org_id = ?1 AND conversation_id = ?2",
            params![org, conversation],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(StoreError::from)?;
    let case = match situation {
        Some((id, status)) => {
            if RecordStatus::from_text(&status).map_err(StoreError::from)? == RecordStatus::Closed {
                return Ok(false);
            }
            id
        }
        None => {
            let without_case: bool = conn
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM adapter_conversations
                                     WHERE org_id = ?1 AND id = ?2 AND create_case = 0)",
                    params![org, conversation],
                    |row| row.get(0),
                )
                .map_err(StoreError::from)?;
            if without_case {
                return Ok(false);
            }
            let case = RecordId::generate().to_string();
            let inserted = conn
                .execute(
                    "INSERT INTO fichas
                        (id, org_id, conversation_id, agent_id, canal, peer_externo, status,
                         aberta_em, fechada_em, fechada_por, fechada_motivo, version)
                     SELECT ?1, org_id, id, agent_id, channel, external_peer, 'closed', created_at,
                            ?3, ?4, ?5, 1
                       FROM conversations WHERE org_id = ?2 AND id = ?6",
                    params![case, org, now, by.as_text(), motivo, conversation],
                )
                .map_err(|error| StoreError::from_write("conversa", error))?;
            if inserted == 0 {
                return Ok(false);
            }
            register_event_conn(conn, org, &case, &event_of_closing(by, motivo), now)?;
            return Ok(true);
        }
    };

    close_case_by_id(conn, org, &case, by, motivo, now, metrics)
}

pub(crate) fn close_case_by_id(
    conn: &Connection,
    org: &str,
    case: &str,
    by: ClosedBy,
    motivo: &str,
    now: i64,
    metrics: Option<&ExecutionMetrics>,
) -> Resultado<bool> {
    let situation: Option<String> = conn
        .query_row(
            "SELECT status FROM fichas WHERE org_id = ?1 AND id = ?2",
            params![org, case],
            |row| row.get(0),
        )
        .optional()
        .map_err(StoreError::from)?;
    let Some(situation) = situation else {
        return Err(StoreError::Domain(poph_types::DomainError::NotFound {
            entity: "ficha",
            id: case.to_owned(),
        }));
    };
    if RecordStatus::from_text(&situation).map_err(StoreError::from)? == RecordStatus::Closed {
        return Ok(false);
    }

    let active_list = executions_active(conn, org, case)?;
    let abandoned = ExecutionStatus::Abandoned.as_text();
    for execution in &active_list {
        conn.execute(
            "UPDATE fluxo_execucoes
                SET status = ?3, motivo_fim = ?4, finalizada_em = ?5, version = version + 1
              WHERE org_id = ?1 AND id = ?2",
            params![org, execution, abandoned, REASON_ABANDONMENT_SERVICE, now],
        )
        .map_err(StoreError::from)?;
        register_event_conn(
            conn,
            org,
            case,
            &NewRecordEvent {
                execucao_id: execution.parse::<ExecutionId>().ok(),
                tipo: RecordEventKind::ExecutionAbandoned,
                ator_tipo: EventActor::System,
                ator_ref: None,
                payload_json: format!("{{\"motivo\":{}}}", json_string(REASON_ABANDONMENT_SERVICE)),
                chave_idempotencia: None,
            },
            now,
        )?;
    }
    if let Some(metrics) = metrics {
        metrics.outcome(ExecutionStatus::Abandoned, active_list.len() as u64);
    }

    register_event_conn(conn, org, case, &event_of_closing(by, motivo), now)?;

    let changed = conn
        .execute(
            "UPDATE fichas
                SET status = 'closed', fechada_em = ?3, fechada_por = ?4, fechada_motivo = ?5,
                    version = version + 1
              WHERE org_id = ?1 AND id = ?2 AND status = 'open'",
            params![org, case, now, by.as_text(), motivo],
        )
        .map_err(|error| StoreError::from_write("fechada_por", error))?;
    Ok(changed > 0)
}

fn event_of_closing(by: ClosedBy, motivo: &str) -> NewRecordEvent<'static> {
    NewRecordEvent {
        execucao_id: None,
        tipo: RecordEventKind::RecordClosed,
        ator_tipo: match by {
            ClosedBy::Admin => EventActor::Admin,
            ClosedBy::Agent => EventActor::Agent,
            ClosedBy::Attendant => EventActor::Attendant,
            ClosedBy::Customer | ClosedBy::Flow | ClosedBy::System | ClosedBy::Migration => {
                EventActor::System
            }
        },
        ator_ref: None,
        payload_json: format!("{{\"motivo\":{}}}", json_string(motivo)),
        chave_idempotencia: None,
    }
}

fn executions_active(conn: &Connection, org: &str, case: &str) -> Resultado<Vec<String>> {
    let mut statement = conn
        .prepare(
            "SELECT id FROM fluxo_execucoes
              WHERE org_id = ?1 AND ficha_id = ?2 AND status = 'ativa'",
        )
        .map_err(StoreError::from)?;
    let rows = statement
        .query_map(params![org, case], |row| row.get::<_, String>(0))
        .map_err(StoreError::from)?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row.map_err(StoreError::from)?);
    }
    Ok(ids)
}

fn json_string(texto: &str) -> String {
    let mut saida = String::with_capacity(texto.len() + 2);
    saida.push('"');
    for character in texto.chars() {
        match character {
            '"' => saida.push_str("\\\""),
            '\\' => saida.push_str("\\\\"),
            '\n' => saida.push_str("\\n"),
            '\r' => saida.push_str("\\r"),
            '\t' => saida.push_str("\\t"),
            another => saida.push(another),
        }
    }
    saida.push('"');
    saida
}

pub(crate) fn register_event_conn(
    conn: &Connection,
    org: &str,
    case: &str,
    new: &NewRecordEvent<'_>,
    now: i64,
) -> Resultado<String> {
    if let Some(execution) = new.execucao_id {
        belongs_to_case(conn, org, case, &execution.to_string())?;
    }
    let id = RecordEventId::generate().to_string();
    conn.execute(
        "INSERT INTO ficha_eventos
            (id, org_id, ficha_id, execucao_id, seq, tipo, ator_tipo, ator_ref, payload_json,
             chave_idempotencia, created_at)
         VALUES (?1, ?2, ?3, ?4,
                 (SELECT COALESCE(MAX(seq), 0) + 1 FROM ficha_eventos
                   WHERE org_id = ?2 AND ficha_id = ?3),
                 ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            org,
            case,
            new.execucao_id.map(|execution| execution.to_string()),
            new.tipo.as_text(),
            new.ator_tipo.as_text(),
            new.ator_ref,
            new.payload_json,
            new.chave_idempotencia,
            now
        ],
    )
    .map_err(|error| StoreError::from_write("tipo", error))?;
    Ok(id)
}

fn belongs_to_case(conn: &Connection, org: &str, case: &str, execution: &str) -> Resultado<()> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM fluxo_execucoes WHERE org_id = ?1 AND ficha_id = ?2 AND id = ?3",
            params![org, case, execution],
            |row| row.get(0),
        )
        .optional()
        .map_err(StoreError::from)?;
    match exists {
        Some(_) => Ok(()),
        None => Err(StoreError::Domain(poph_types::DomainError::NotFound {
            entity: "execução de fluxo",
            id: execution.to_owned(),
        })),
    }
}

fn row(row: &Row<'_>) -> rusqlite::Result<Ficha> {
    Ok(Ficha {
        id: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        conversation_id: id_optional(row, 2)?,
        agent_id: column_id(row, 3)?,
        canal: row.get(4)?,
        peer_externo: row.get(5)?,
        status: RecordStatus::from_text(&row.get::<_, String>(6)?)
            .map_err(|error| conversao(6, error))?,
        aberta_em: row.get(7)?,
        fechada_em: row.get(8)?,
        fechada_por: match row.get::<_, Option<String>>(9)? {
            Some(texto) => Some(ClosedBy::from_text(&texto).map_err(|error| conversao(9, error))?),
            None => None,
        },
        fechada_motivo: row.get(10)?,
        version: column_u64(row, 11)?,
    })
}

fn event_row(row: &Row<'_>) -> rusqlite::Result<RecordEvent> {
    Ok(RecordEvent {
        id: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        ficha_id: column_id(row, 2)?,
        execucao_id: id_optional(row, 3)?,
        seq: row.get(4)?,
        tipo: RecordEventKind::from_text(&row.get::<_, String>(5)?)
            .map_err(|error| conversao(5, error))?,
        ator_tipo: EventActor::from_text(&row.get::<_, String>(6)?)
            .map_err(|error| conversao(6, error))?,
        ator_ref: row.get(7)?,
        payload_json: row.get(8)?,
        chave_idempotencia: row.get(9)?,
        created_at: row.get(10)?,
    })
}

fn conversao(index: usize, error: poph_types::DomainError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(error))
}
