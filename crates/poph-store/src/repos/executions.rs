use poph_types::{
    ActionActor, ClosedBy, DomainError, EffectId, EffectState, ExecutionId, ExecutionStatus,
    FlowId, OrgId, RecordEventKind, RecordId, RecordStatus, RevisionId, StepStatus,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::{Map, Value as Json};

use super::cases::{
    close_case_by_id, event_by_key, last_seq, read_events_conn, register_event_conn,
};
use super::{column_id, column_u32, column_u64, conflict_or_missing};
use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{
    Execution, ExecutionStart, ExecutionState, ExecutionStep, FlowEffect, FlowTransition,
    NewRecordEvent, StoredExecution, TransitionEffect, TransitionEvent, TransitionExtras,
};

const COLUMNS: &str = "id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, dados_json,
    resultado, motivo_fim, iniciada_em, finalizada_em, version";

const COLUMNS_STEP: &str = "id, org_id, execucao_id, no_id, ordem, status, dados_json,
    atores_que_agiram_json, entrou_em, concluiu_em";

pub(crate) const COLUMNS_EFFECT: &str =
    "id, org_id, ficha_id, execucao_id, chave, tipo, payload_json, estado,
    tentativas, ultimo_erro, proxima_tentativa, enviado_em, created_at";

pub const LIMIT_EXECUTIONS: u32 = 50;

pub struct Executions<'a>(pub(crate) &'a Store);

impl Executions<'_> {
    pub fn start_execution(
        &self,
        org: OrgId,
        beginning: ExecutionStart<'_>,
    ) -> Resultado<StoredExecution> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_case = beginning.case.to_string();
        let text_flow = beginning.flow.to_string();
        let text_revision = beginning.revision.to_string();
        let actor = beginning.actor;
        let ator_ref = beginning.ator_ref.map(str::to_owned);
        let decisao = beginning
            .jev_decision
            .as_ref()
            .map(|decisao| (decisao.choice.to_owned(), decisao.confidence));
        let state_initial = beginning.initial_state;
        let effects_entry = beginning.inbound_effects;
        let start_key = match beginning.chave_idempotencia.map(str::trim) {
            None => None,
            Some("") => {
                return Err(StoreError::Domain(DomainError::Validation {
                    field: "chave_idempotencia",
                    reason: "a abertura precisa de uma chave: repetir a mesma tem de devolver a \
                             mesma execução"
                        .to_owned(),
                }));
            }
            Some(chave) => Some(format!("inicio:{chave}")),
        };

        let stored = self.0.write(move |conn| {
            if let Some(marker) = start_key.as_deref()
                && let Some(evento) = event_by_key(conn, &text_org, &text_case, marker)?
            {
                let Some(execucao_id) = evento.execucao_id else {
                    return Err(StoreError::Internal(
                        "a abertura marcada não tem execução ligada".to_owned(),
                    ));
                };
                let execution = read_execution(conn, &text_org, &execucao_id.to_string())?
                    .ok_or_else(|| {
                        StoreError::Internal("a execução da abertura marcada sumiu".to_owned())
                    })?;
                if execution.fluxo_id.to_string() != text_flow {
                    return Err(StoreError::Domain(DomainError::Conflict {
                        entity: "execução de fluxo",
                        id: execution.id.to_string(),
                        reason: "esta chave de idempotência já abriu outro fluxo nesta Ficha"
                            .to_owned(),
                    }));
                }
                return Ok(StoredExecution {
                    execution,
                    events: Vec::new(),
                    effects: Vec::new(),
                    closed_service: false,
                    repeated: true,
                });
            }

            let case_row: Option<(String, String)> = conn
                .query_row(
                    "SELECT status, agent_id FROM fichas WHERE org_id = ?1 AND id = ?2",
                    params![text_org, text_case],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(StoreError::from)?;
            let Some((situation_case, agent_id)) = case_row else {
                return Err(not_found("ficha", &text_case));
            };
            if RecordStatus::from_text(&situation_case).map_err(StoreError::from)?
                == RecordStatus::Closed
            {
                return Err(StoreError::Domain(DomainError::RecordClosed {
                    id: text_case.clone(),
                }));
            }

            let flow_row: Option<(String, String)> = conn
                .query_row(
                    "SELECT status, slug FROM fluxos WHERE org_id = ?1 AND id = ?2",
                    params![text_org, text_flow],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(StoreError::from)?;
            let Some((situation_flow, slug)) = flow_row else {
                return Err(not_found("fluxo", &text_flow));
            };
            if situation_flow != "active" {
                return Err(StoreError::Domain(DomainError::Conflict {
                    entity: "fluxo",
                    id: text_flow.clone(),
                    reason: "o fluxo está inativo: não inicia execução nova".to_owned(),
                }));
            }

            let released: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM agente_fluxos
                      WHERE org_id = ?1 AND agent_id = ?2 AND fluxo_id = ?3",
                    params![text_org, agent_id, text_flow],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            if released.is_none() {
                return Err(StoreError::Domain(DomainError::FlowNotAllowed {
                    fluxo_id: text_flow.clone(),
                    agent_id,
                }));
            }

            let active: Option<String> = conn
                .query_row(
                    "SELECT id FROM fluxo_execucoes
                      WHERE org_id = ?1 AND ficha_id = ?2 AND status = 'ativa'",
                    params![text_org, text_case],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            if let Some(active) = active {
                return Err(StoreError::Domain(DomainError::ExecutionAlreadyActive {
                    ficha_id: text_case.clone(),
                    execucao_id: active,
                }));
            }

            let revision_from_flow: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM fluxo_revisoes WHERE org_id = ?1 AND id = ?2 AND fluxo_id = ?3",
                    params![text_org, text_revision, text_flow],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            if revision_from_flow.is_none() {
                return Err(not_found("revisão de fluxo", &text_revision));
            }

            if state_initial.status != ExecutionStatus::Active {
                return Err(StoreError::Domain(DomainError::Validation {
                    field: "estado_inicial",
                    reason: "a execução nasce `ativa`; estado de abertura recusado".to_owned(),
                }));
            }

            let execution = ExecutionId::generate().to_string();
            conn.execute(
                "INSERT INTO fluxo_execucoes
                    (id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, dados_json,
                     resultado, motivo_fim, iniciada_em, finalizada_em, version)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'ativa', ?6, ?7, NULL, NULL, ?8, NULL, 1)",
                params![
                    execution,
                    text_org,
                    text_case,
                    text_flow,
                    text_revision,
                    state_initial.no_atual,
                    state_initial.dados_json,
                    now
                ],
            )
            .map_err(|error| StoreError::from_write("fluxo_execucoes", error))?;

            conn.execute(
                "INSERT INTO fluxo_execucao_etapas
                    (id, org_id, execucao_id, no_id, ordem, status, dados_json,
                     atores_que_agiram_json, entrou_em, concluiu_em)
                 VALUES (?1, ?2, ?3, ?4, 1, 'pendente', ?5, '[]', ?6, NULL)",
                params![
                    uuid::Uuid::now_v7().to_string(),
                    text_org,
                    execution,
                    state_initial.no_atual,
                    data_of_step(&state_initial.dados_json, &state_initial.no_atual),
                    now
                ],
            )
            .map_err(|error| StoreError::from_write("fluxo_execucao_etapas", error))?;

            let mut detail = Map::new();
            detail.insert("fluxo".to_owned(), Json::String(slug));
            detail.insert(
                "etapa".to_owned(),
                Json::String(state_initial.no_atual.clone()),
            );
            detail.insert("ator".to_owned(), Json::String(actor.as_text().to_owned()));
            if let Some((choice, confidence)) = decisao {
                let mut jev = Map::new();
                jev.insert("escolha".to_owned(), Json::String(choice));
                if let Some(confidence) = confidence {
                    jev.insert(
                        "confianca".to_owned(),
                        serde_json::Number::from_f64(confidence)
                            .map(Json::Number)
                            .unwrap_or(Json::Null),
                    );
                }
                detail.insert("jev".to_owned(), Json::Object(jev));
            }

            let seq_before = last_seq(conn, &text_org, &text_case)?;
            register_event_conn(
                conn,
                &text_org,
                &text_case,
                &NewRecordEvent {
                    execucao_id: Some(
                        execution
                            .parse()
                            .map_err(|error: DomainError| StoreError::Domain(error))?,
                    ),
                    tipo: RecordEventKind::FlowStarted,
                    ator_tipo: actor,
                    ator_ref: ator_ref.as_deref(),
                    payload_json: Json::Object(detail).to_string(),
                    chave_idempotencia: start_key.as_deref(),
                },
                now,
            )?;
            let events = read_events_conn(conn, &text_org, &text_case, seq_before)?;

            let effects =
                write_effects(conn, &text_org, &text_case, &execution, &effects_entry, now)?;

            let stored = read_execution(conn, &text_org, &execution)?.ok_or_else(|| {
                StoreError::Internal("a execução foi gravada e não voltou".to_owned())
            })?;
            Ok(StoredExecution {
                execution: stored,
                events,
                effects,
                closed_service: false,
                repeated: false,
            })
        })?;
        if !stored.repeated {
            self.0.count_opened_execution();
        }
        Ok(stored)
    }

    pub fn transition_already_applied(
        &self,
        org: OrgId,
        execution: ExecutionId,
        chave_idempotencia: &str,
    ) -> Resultado<bool> {
        let chave = chave_idempotencia.trim();
        if chave.is_empty() {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "chave_idempotencia",
                reason: "informe a chave de idempotência da operação".to_owned(),
            }));
        }
        let marker = event_key(&execution.to_string(), chave, 0);
        let text_org = org.to_string();
        self.0.read(move |conn| {
            let finding: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM ficha_eventos WHERE org_id = ?1 AND chave_idempotencia = ?2",
                    params![text_org, marker],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            Ok(finding.is_some())
        })
    }

    pub fn opening_already_done(
        &self,
        org: OrgId,
        case: RecordId,
        flow: FlowId,
        chave_idempotencia: &str,
    ) -> Resultado<Option<Execution>> {
        let chave = chave_idempotencia.trim();
        if chave.is_empty() {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "chave_idempotencia",
                reason: "a abertura precisa de uma chave".to_owned(),
            }));
        }
        let marker = format!("inicio:{chave}");
        let text_org = org.to_string();
        let text_case = case.to_string();
        let text_flow = flow.to_string();
        self.0.read(move |conn| {
            let Some(evento) = event_by_key(conn, &text_org, &text_case, &marker)? else {
                return Ok(None);
            };
            let Some(execucao_id) = evento.execucao_id else {
                return Err(StoreError::Internal(
                    "a abertura marcada não tem execução ligada".to_owned(),
                ));
            };
            let execution =
                read_execution(conn, &text_org, &execucao_id.to_string())?.ok_or_else(|| {
                    StoreError::Internal("a execução da abertura marcada sumiu".to_owned())
                })?;
            if execution.fluxo_id.to_string() != text_flow {
                return Err(StoreError::Domain(DomainError::Conflict {
                    entity: "execução de fluxo",
                    id: execution.id.to_string(),
                    reason: "esta chave de idempotência já abriu outro fluxo nesta Ficha"
                        .to_owned(),
                }));
            }
            Ok(Some(execution))
        })
    }

    pub fn apply_transition(
        &self,
        org: OrgId,
        execution: ExecutionId,
        expected: u64,
        transition: &FlowTransition,
        chave_idempotencia: &str,
        extras: TransitionExtras<'_>,
    ) -> Resultado<StoredExecution> {
        let chave = chave_idempotencia.trim();
        if chave.is_empty() {
            return Err(StoreError::Domain(DomainError::Validation {
                field: "chave_idempotencia",
                reason:
                    "a transição precisa de uma chave: repetir a mesma tem de devolver o mesmo \
                         desfecho, não gravar outra coisa"
                        .to_owned(),
            }));
        }
        let now = self.0.now();
        let text_org = org.to_string();
        let text_execution = execution.to_string();
        let error_of_rule = transition.rule_error;
        let transition = transition.clone();
        let metrics = self.0.execution_metrics();
        let chave = chave.to_owned();
        let ator_ref = extras.ator_ref.map(str::to_owned);
        let fechada_por = extras.fechada_por.unwrap_or(ClosedBy::Flow);
        let request_reason = extras.close_reason.map(str::to_owned);
        let expected_i64 = i64::try_from(expected).unwrap_or(i64::MAX);

        let stored = self.0.write(move |conn| {
            let Some(current) = read_execution(conn, &text_org, &text_execution)? else {
                return Err(not_found("execução de fluxo", &text_execution));
            };
            let text_case = current.ficha_id.to_string();
            let new = &transition.new_state;

            if transition.repeated {
                return Ok(StoredExecution {
                    execution: current,
                    events: Vec::new(),
                    effects: Vec::new(),
                    closed_service: false,
                    repeated: true,
                });
            }
            let marker = event_key(&text_execution, &chave, 0);
            let applied: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM ficha_eventos
                      WHERE org_id = ?1 AND ficha_id = ?2 AND chave_idempotencia = ?3",
                    params![text_org, text_case, marker],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            if applied.is_some() {
                return Ok(StoredExecution {
                    execution: current,
                    events: Vec::new(),
                    effects: Vec::new(),
                    closed_service: false,
                    repeated: true,
                });
            }

            if current.version != expected {
                return Err(StoreError::Domain(DomainError::VersionConflict {
                    entity: "execução de fluxo",
                    id: text_execution.clone(),
                    expected,
                    current: current.version,
                }));
            }

            if current.status != ExecutionStatus::Active {
                return Err(StoreError::Domain(DomainError::Conflict {
                    entity: "execução de fluxo",
                    id: text_execution.clone(),
                    reason: format!("a execução está {} e não aceita transição", current.status),
                }));
            }

            let case_row: Option<(String, Option<String>)> = conn
                .query_row(
                    "SELECT status, conversation_id FROM fichas WHERE org_id = ?1 AND id = ?2",
                    params![text_org, text_case],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(StoreError::from)?;
            let Some((situation_case, conversation)) = case_row else {
                return Err(not_found("ficha", &text_case));
            };
            if RecordStatus::from_text(&situation_case).map_err(StoreError::from)?
                == RecordStatus::Closed
            {
                return Err(StoreError::Domain(DomainError::RecordClosed {
                    id: text_case.clone(),
                }));
            }

            if transition.closes_service && new.status != ExecutionStatus::Completed {
                return Err(StoreError::Domain(DomainError::Validation {
                    field: "encerra_atendimento",
                    reason: "só execução concluída num fim pode encerrar o atendimento".to_owned(),
                }));
            }

            let finalised = match new.status {
                ExecutionStatus::Active => None,
                _ => Some(now),
            };
            let changed = conn
                .execute(
                    "UPDATE fluxo_execucoes
                        SET status = ?3, no_atual = ?4, dados_json = ?5, resultado = ?6,
                            motivo_fim = ?7, finalizada_em = ?8, version = version + 1
                      WHERE org_id = ?1 AND id = ?2 AND version = ?9",
                    params![
                        text_org,
                        text_execution,
                        new.status.as_text(),
                        new.no_atual,
                        new.dados_json,
                        new.result,
                        new.reason,
                        finalised,
                        expected_i64
                    ],
                )
                .map_err(StoreError::from)?;
            if changed == 0 {
                return Err(conflict_or_missing(
                    conn,
                    "fluxo_execucoes",
                    "execução de fluxo",
                    &text_execution,
                    &text_org,
                    expected,
                ));
            }

            write_steps(
                conn,
                &text_org,
                &text_execution,
                new,
                &transition.events,
                now,
            )?;

            let seq_before = last_seq(conn, &text_org, &text_case)?;
            for (index, evento) in transition.events.iter().enumerate() {
                register_event_conn(
                    conn,
                    &text_org,
                    &text_case,
                    &NewRecordEvent {
                        execucao_id: Some(execution),
                        tipo: evento.tipo,
                        ator_tipo: evento.ator_tipo,
                        ator_ref: ator_ref.as_deref(),
                        payload_json: evento.payload_json.clone(),
                        chave_idempotencia: Some(&event_key(&text_execution, &chave, index)),
                    },
                    now,
                )?;
            }
            let events = read_events_conn(conn, &text_org, &text_case, seq_before)?;

            let effects = write_effects(
                conn,
                &text_org,
                &text_case,
                &text_execution,
                &transition.effects,
                now,
            )?;

            let mut closed = false;
            if transition.closes_service {
                if let Some(conversation) = conversation {
                    conn.execute(
                        "UPDATE conversations
                            SET status = 'closed', version = version + 1, updated_at = ?3
                          WHERE org_id = ?1 AND id = ?2 AND status <> 'closed'",
                        params![text_org, conversation, now],
                    )
                    .map_err(|error| StoreError::from_write("conversa", error))?;
                }
                let motivo = request_reason
                    .or_else(|| new.result.clone())
                    .unwrap_or_else(|| "fim do fluxo".to_owned());
                closed = close_case_by_id(
                    conn,
                    &text_org,
                    &text_case,
                    fechada_por,
                    &motivo,
                    now,
                    metrics.as_ref(),
                )?;
            }

            let stored = read_execution(conn, &text_org, &text_execution)?.ok_or_else(|| {
                StoreError::Internal("a execução foi gravada e não voltou".to_owned())
            })?;
            Ok(StoredExecution {
                execution: stored,
                events,
                effects,
                closed_service: closed,
                repeated: false,
            })
        })?;
        if !stored.repeated {
            self.0.count_execution_outcome(stored.execution.status);
            if error_of_rule && stored.execution.status == ExecutionStatus::Failed {
                self.0.count_rule_error();
            }
        }
        Ok(stored)
    }

    pub fn get(&self, org: OrgId, id: ExecutionId) -> Resultado<Execution> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let found = self
            .0
            .read(move |conn| read_execution(conn, &text_org, &text_id))?;
        match found {
            Some(execution) => Ok(execution),
            None => Err(not_found("execução de fluxo", &id.to_string())),
        }
    }

    pub fn active_of_case(&self, org: OrgId, case: RecordId) -> Resultado<Option<Execution>> {
        let text_org = org.to_string();
        let text_case = case.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUMNS} FROM fluxo_execucoes
                      WHERE org_id = ?1 AND ficha_id = ?2 AND status = 'ativa'"
                ),
                params![text_org, text_case],
                execution_row,
            )
            .optional()
            .map_err(StoreError::from)
        })
    }

    pub fn from_case(&self, org: OrgId, case: RecordId, limit: u32) -> Resultado<Vec<Execution>> {
        let text_org = org.to_string();
        let text_case = case.to_string();
        let limit = i64::from(limit.clamp(1, 500));
        self.0.read(move |conn| {
            let mut statement = conn
                .prepare(&format!(
                    "SELECT {COLUMNS} FROM fluxo_execucoes
                      WHERE org_id = ?1 AND ficha_id = ?2
                      ORDER BY iniciada_em, id LIMIT ?3"
                ))
                .map_err(StoreError::from)?;
            let rows = statement
                .query_map(params![text_org, text_case, limit], execution_row)
                .map_err(StoreError::from)?;
            let mut executions = Vec::new();
            for row in rows {
                executions.push(row.map_err(StoreError::from)?);
            }
            Ok(executions)
        })
    }

    pub fn active_by_revision(
        &self,
        org: OrgId,
        flow: FlowId,
    ) -> Resultado<Vec<(RevisionId, u64)>> {
        let text_org = org.to_string();
        let text_flow = flow.to_string();
        self.0.read(move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT revisao_id, COUNT(*) FROM fluxo_execucoes
                      WHERE org_id = ?1 AND fluxo_id = ?2 AND status = 'ativa'
                      GROUP BY revisao_id ORDER BY revisao_id",
                )
                .map_err(StoreError::from)?;
            let rows = statement
                .query_map(params![text_org, text_flow], |row| {
                    Ok((column_id::<RevisionId>(row, 0)?, column_u64(row, 1)?))
                })
                .map_err(StoreError::from)?;
            let mut counts = Vec::new();
            for row in rows {
                counts.push(row.map_err(StoreError::from)?);
            }
            Ok(counts)
        })
    }

    pub fn stages(&self, org: OrgId, execution: ExecutionId) -> Resultado<Vec<ExecutionStep>> {
        self.get(org, execution)?;
        let text_org = org.to_string();
        let text_execution = execution.to_string();
        self.0.read(move |conn| {
            let mut statement = conn
                .prepare(&format!(
                    "SELECT {COLUMNS_STEP} FROM fluxo_execucao_etapas
                      WHERE org_id = ?1 AND execucao_id = ?2 ORDER BY ordem"
                ))
                .map_err(StoreError::from)?;
            let rows = statement
                .query_map(params![text_org, text_execution], step_row)
                .map_err(StoreError::from)?;
            let mut stages = Vec::new();
            for row in rows {
                stages.push(row.map_err(StoreError::from)?);
            }
            Ok(stages)
        })
    }

    pub fn effects(&self, org: OrgId, execution: ExecutionId) -> Resultado<Vec<FlowEffect>> {
        self.get(org, execution)?;
        let text_org = org.to_string();
        let text_execution = execution.to_string();
        self.0.read(move |conn| {
            let mut statement = conn
                .prepare(&format!(
                    "SELECT {COLUMNS_EFFECT} FROM fluxo_efeitos
                      WHERE org_id = ?1 AND execucao_id = ?2 ORDER BY created_at, id"
                ))
                .map_err(StoreError::from)?;
            let rows = statement
                .query_map(params![text_org, text_execution], effect_row)
                .map_err(StoreError::from)?;
            let mut effects = Vec::new();
            for row in rows {
                effects.push(row.map_err(StoreError::from)?);
            }
            Ok(effects)
        })
    }
}

fn event_key(execution: &str, chave: &str, index: usize) -> String {
    format!("fluxo:{execution}:{chave}#{index}")
}

fn not_found(entity: &'static str, id: &str) -> StoreError {
    StoreError::Domain(DomainError::NotFound {
        entity,
        id: id.to_owned(),
    })
}

fn write_steps(
    conn: &Connection,
    org: &str,
    execution: &str,
    new: &ExecutionState,
    events: &[TransitionEvent],
    now: i64,
) -> Resultado<()> {
    for evento in events {
        match evento.tipo {
            RecordEventKind::StepCompleted => {
                let Some(etapa) = step_from_payload(&evento.payload_json) else {
                    continue;
                };
                let actors_json: Option<String> = conn
                    .query_row(
                        "SELECT atores_que_agiram_json FROM fluxo_execucao_etapas
                      WHERE org_id = ?1 AND execucao_id = ?2 AND no_id = ?3
                        AND status <> 'concluida' ORDER BY ordem DESC LIMIT 1",
                        params![org, execution, etapa],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(StoreError::from)?;
                let mut actors: Vec<String> = actors_json
                    .as_deref()
                    .and_then(|json| serde_json::from_str(json).ok())
                    .unwrap_or_default();
                if let Some(acao) = events.iter().find(|acao| {
                    acao.tipo == RecordEventKind::ActionExecuted
                        && step_from_payload(&acao.payload_json).as_deref() == Some(&etapa)
                }) {
                    let actor = acao.ator_tipo.as_text();
                    if (actor == "agente" || actor == "atendente")
                        && !actors.iter().any(|a| a == actor)
                    {
                        actors.push(actor.to_owned());
                        actors.sort();
                    }
                }
                let actors = serde_json::to_string(&actors).unwrap_or_else(|_| "[]".to_owned());
                conn.execute(
                    "UPDATE fluxo_execucao_etapas
                        SET status = 'concluida', concluiu_em = ?4, dados_json = ?5, atores_que_agiram_json = ?6
                      WHERE org_id = ?1 AND execucao_id = ?2 AND no_id = ?3
                        AND status <> 'concluida'
                        AND ordem = (SELECT MAX(ordem) FROM fluxo_execucao_etapas
                                      WHERE org_id = ?1 AND execucao_id = ?2 AND no_id = ?3)",
                    params![org, execution, etapa, now, data_of_step(&new.dados_json, &etapa), actors],
                )
                .map_err(StoreError::from)?;
            }
            RecordEventKind::StepEntered => {
                let Some(etapa) = step_from_payload(&evento.payload_json) else {
                    continue;
                };
                let ordem: i64 = conn
                    .query_row(
                        "SELECT COALESCE(MAX(ordem), 0) + 1 FROM fluxo_execucao_etapas
                          WHERE org_id = ?1 AND execucao_id = ?2",
                        params![org, execution],
                        |row| row.get(0),
                    )
                    .map_err(StoreError::from)?;
                conn.execute(
                    "INSERT INTO fluxo_execucao_etapas
                        (id, org_id, execucao_id, no_id, ordem, status, dados_json,
                         atores_que_agiram_json, entrou_em, concluiu_em)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'pendente', ?6, '[]', ?7, NULL)",
                    params![
                        uuid::Uuid::now_v7().to_string(),
                        org,
                        execution,
                        etapa,
                        ordem,
                        data_of_step(&new.dados_json, &etapa),
                        now
                    ],
                )
                .map_err(|error| StoreError::from_write("fluxo_execucao_etapas", error))?;
            }
            _ => {}
        }
    }

    if new.status != ExecutionStatus::Active {
        return Ok(());
    }
    let dados = data_of_step(&new.dados_json, &new.no_atual);
    let actors = serde_json::to_string(&new.acting_actors).unwrap_or_else(|_| "[]".to_owned());
    let status = if dados == "{}" && new.acting_actors.is_empty() {
        StepStatus::Pending
    } else {
        StepStatus::Partial
    };
    conn.execute(
        "UPDATE fluxo_execucao_etapas
            SET dados_json = ?3, atores_que_agiram_json = ?4, status = ?5
          WHERE org_id = ?1 AND execucao_id = ?2 AND no_id = ?6 AND status <> 'concluida'
            AND ordem = (SELECT MAX(ordem) FROM fluxo_execucao_etapas
                          WHERE org_id = ?1 AND execucao_id = ?2)",
        params![
            org,
            execution,
            dados,
            actors,
            status.as_text(),
            new.no_atual
        ],
    )
    .map_err(StoreError::from)?;
    Ok(())
}

fn write_effects(
    conn: &Connection,
    org: &str,
    case: &str,
    execution: &str,
    effects: &[TransitionEffect],
    now: i64,
) -> Resultado<Vec<FlowEffect>> {
    let ficha_id: RecordId = case.parse().map_err(StoreError::Domain)?;
    let execucao_id: ExecutionId = execution.parse().map_err(StoreError::Domain)?;
    let org_id: OrgId = org.parse().map_err(StoreError::Domain)?;
    let mut stored_rows = Vec::new();
    for effect in effects {
        let id = EffectId::generate();
        let inserted = conn
            .execute(
                "INSERT INTO fluxo_efeitos
                    (id, org_id, ficha_id, execucao_id, chave, tipo, payload_json, estado,
                     tentativas, ultimo_erro, proxima_tentativa, enviado_em, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pendente', 0, NULL, NULL, NULL, ?8)
                 ON CONFLICT (org_id, execucao_id, chave) DO NOTHING",
                params![
                    id.to_string(),
                    org,
                    case,
                    execution,
                    effect.key,
                    effect.tipo,
                    effect.payload_json,
                    now
                ],
            )
            .map_err(|error| StoreError::from_write("fluxo_efeitos", error))?;
        if inserted == 1 {
            stored_rows.push(FlowEffect {
                id,
                org_id,
                ficha_id,
                execucao_id,
                key: effect.key.clone(),
                tipo: effect.tipo.clone(),
                payload_json: effect.payload_json.clone(),
                state: EffectState::Pending,
                attempts: 0,
                ultimo_erro: None,
                proxima_tentativa: None,
                enviado_em: None,
                created_at: now,
            });
        }
    }
    Ok(stored_rows)
}

fn read_execution(conn: &Connection, org: &str, execution: &str) -> Resultado<Option<Execution>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM fluxo_execucoes WHERE org_id = ?1 AND id = ?2"),
        params![org, execution],
        execution_row,
    )
    .optional()
    .map_err(StoreError::from)
}

fn data_of_step(dados_json: &str, no: &str) -> String {
    let Ok(Json::Object(map)) = serde_json::from_str::<Json>(dados_json) else {
        return "{}".to_owned();
    };
    match map.get(no) {
        Some(Json::Object(_)) => {
            serde_json::to_string(&map[no]).unwrap_or_else(|_| "{}".to_owned())
        }
        _ => "{}".to_owned(),
    }
}

fn step_from_payload(payload_json: &str) -> Option<String> {
    let value: Json = serde_json::from_str(payload_json).ok()?;
    value.get("etapa")?.as_str().map(str::to_owned)
}

fn execution_row(row: &Row<'_>) -> rusqlite::Result<Execution> {
    Ok(Execution {
        id: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        ficha_id: column_id(row, 2)?,
        fluxo_id: column_id(row, 3)?,
        revisao_id: column_id(row, 4)?,
        status: ExecutionStatus::from_text(&row.get::<_, String>(5)?)
            .map_err(|error| conversao(5, error))?,
        no_atual: row.get(6)?,
        dados_json: row.get(7)?,
        result: row.get(8)?,
        motivo_fim: row.get(9)?,
        iniciada_em: row.get(10)?,
        finalizada_em: row.get(11)?,
        version: column_u64(row, 12)?,
    })
}

fn step_row(row: &Row<'_>) -> rusqlite::Result<ExecutionStep> {
    let actors_json: String = row.get(7)?;
    let actors: Vec<ActionActor> = serde_json::from_str(&actors_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(ExecutionStep {
        id: row.get(0)?,
        org_id: column_id(row, 1)?,
        execucao_id: column_id(row, 2)?,
        no_id: row.get(3)?,
        ordem: row.get(4)?,
        status: StepStatus::from_text(&row.get::<_, String>(5)?)
            .map_err(|error| conversao(5, error))?,
        dados_json: row.get(6)?,
        acting_actors: actors,
        entrou_em: row.get(8)?,
        concluiu_em: row.get(9)?,
    })
}

pub(crate) fn effect_row(row: &Row<'_>) -> rusqlite::Result<FlowEffect> {
    Ok(FlowEffect {
        id: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        ficha_id: column_id(row, 2)?,
        execucao_id: column_id(row, 3)?,
        key: row.get(4)?,
        tipo: row.get(5)?,
        payload_json: row.get(6)?,
        state: EffectState::from_text(&row.get::<_, String>(7)?)
            .map_err(|error| conversao(7, error))?,
        attempts: column_u32(row, 8)?,
        ultimo_erro: row.get(9)?,
        proxima_tentativa: row.get(10)?,
        enviado_em: row.get(11)?,
        created_at: row.get(12)?,
    })
}

fn conversao(index: usize, error: DomainError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(error))
}
