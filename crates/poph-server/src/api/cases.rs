use std::collections::BTreeMap;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
};
use poph_fluxos::cache::RevisionKey;
use poph_fluxos::ir::Destination;
use poph_fluxos::motor::{
    Campos, Contexto, Estado as EngineState, abandonar as abandon_run, efeitos_de_entrada,
    executar_acao, iniciar,
};
use poph_fluxos::{Dados, Ir, Limites, RuleEvaluator, RuleLimits};
use poph_store::{
    Execution, ExecutionStart, ExecutionStep, FlowRevision, RecordEvent, RecordFilter, StoreError,
    TransitionExtras,
};
use poph_types::{
    ActionActor, AgentId, ClosedBy, DomainError, EventActor, ExecutionId, FlowId, OrgId, RecordId,
    RecordStatus,
};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

use super::{ListJson, PageQuery, if_match_version, next_cursor};
use crate::fichas_servico::{detalhe as detail, evento_json, execucao_json, ficha_json};
use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url as parse_path_id},
    estado::Estado,
    fluxos_efeitos::efeito_da_transicao,
    fluxos_transicao::{estado_para_banco, transicao_do_motor},
};

const REASON_CLOSING_BY_API: &str = "encerrada pelo administrador";

const LIMIT_DEFAULT_EVENT: u32 = 50;

#[derive(Deserialize)]
pub struct OrgPath {
    org_id: String,
}

#[derive(Deserialize)]
pub struct CasePath {
    org_id: String,
    #[serde(rename = "ficha_id")]
    case_id: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct CasesQuery {
    limit: Option<u32>,
    cursor: Option<String>,
    status: Option<String>,
    #[serde(rename = "agente")]
    agent: Option<String>,
    #[serde(rename = "canal")]
    channel: Option<String>,
    #[serde(rename = "desde")]
    since: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct EventsQuery {
    limit: Option<u32>,
    cursor: Option<String>,
}

#[doc = " `GET /fichas`: lista da org, com filtros e cursor."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/fichas",
    tag = "fichas",
    operation_id = "fichas_listar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("status" = Option<String>, Query, description = "`open` ou `closed`"),
        ("agente" = Option<String>, Query, description = "Id do agente da Ficha (UUID v7)"),
        ("canal" = Option<String>, Query, description = "Canal da conversa, por exemplo `whatsapp`"),
        ("desde" = Option<i64>, Query, description = "Só Fichas abertas a partir deste instante, em milissegundos desde 1970"),
        ("limit" = Option<u32>, Query, description = "De 1 a 200; padrão 50"),
        ("cursor" = Option<String>, Query, description = "`next_cursor` da página anterior"),
    ),
    responses(
        (status = 200, description = "Página de Fichas: `items` e `next_cursor`", body = ListJson<Value>),
        (status = 400, description = "Filtro ou cursor inválido (`status`, `agente`, `canal`, `desde`, `limit` ou `cursor`)", body = crate::erro::RespostaErro),
    )
)]
pub async fn list(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<OrgPath>,
    Query(q): Query<CasesQuery>,
) -> Result<Json<ListJson<Value>>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let filter = query_filter(&q)?;
    let page = PageQuery {
        limit: q.limit,
        cursor: q.cursor.clone(),
    }
    .page()?;
    let cases = e.store.cases().list(org, &filter, page)?;
    let next_cursor = next_cursor(&cases, page.limit, |case| case.id.to_string());
    let items: Vec<Value> = cases.iter().map(ficha_json).collect();
    Ok(Json(ListJson { items, next_cursor }))
}

#[doc = " `GET /fichas/{ficha_id}`: a Ficha, as execuções com as etapas e os campos respondidos, e os"]
#[doc = " fluxos liberados ao agente da Ficha."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/fichas/{ficha_id}",
    tag = "fichas",
    operation_id = "fichas_mostrar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("ficha_id" = String, Path),
    ),
    responses(
        (status = 200, description = "A Ficha com as execuções, as etapas com os campos respondidos (W6) e os fluxos liberados ao agente", body = Value),
        (status = 404, description = "Ficha inexistente ou de outra org", body = crate::erro::RespostaErro),
    )
)]
pub async fn show(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CasePath>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let case_id = parse_path_id::<RecordId>(&c.case_id)?;
    let body = detail(&e.store, org, case_id)?;
    Ok(Json(body))
}

#[doc = " `GET /fichas/{ficha_id}/events`: a linha do tempo da Ficha, em ordem de `seq`."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/events",
    tag = "fichas",
    operation_id = "fichas_eventos",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("ficha_id" = String, Path),
        ("limit" = Option<u32>, Query, description = "De 1 a 200; padrão 50"),
        ("cursor" = Option<String>, Query, description = "`next_cursor` da página anterior"),
    ),
    responses(
        (status = 200, description = "Linha do tempo da Ficha em ordem de `seq`: `items` e `next_cursor` (um `seq`)", body = ListJson<Value>),
        (status = 400, description = "Cursor inválido", body = crate::erro::RespostaErro),
        (status = 404, description = "Ficha inexistente ou de outra org", body = crate::erro::RespostaErro),
    )
)]
pub async fn events(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CasePath>,
    Query(q): Query<EventsQuery>,
) -> Result<Json<ListJson<Value>>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let case_id = parse_path_id::<RecordId>(&c.case_id)?;
    let limit = q.limit.unwrap_or(LIMIT_DEFAULT_EVENT).clamp(1, 200);
    let after = seq_cursor(q.cursor.as_deref())?;
    let events = e.store.cases().events(org, case_id, after, limit)?;
    let next_cursor = next_cursor(&events, limit, |event| event.seq.to_string());
    let items: Vec<Value> = events.iter().map(evento_json).collect();
    Ok(Json(ListJson { items, next_cursor }))
}

fn query_filter(q: &CasesQuery) -> Result<RecordFilter, ApiError> {
    let status = match q.status.as_deref() {
        None => None,
        Some(text) => Some(
            RecordStatus::from_text(text)
                .map_err(|_| ApiError::validacao("status", "use open ou closed"))?,
        ),
    };
    let agent = match q.agent.as_deref() {
        None => None,
        Some(text) => Some(parse_path_id::<AgentId>(text)?),
    };
    let channel = match q.channel.as_deref() {
        None => None,
        Some(text) if text.trim().is_empty() => {
            return Err(ApiError::validacao(
                "canal",
                "informe o canal, por exemplo whatsapp",
            ));
        }
        Some(text) => Some(text.trim().to_owned()),
    };
    let since = match q.since.as_deref() {
        None => None,
        Some(text) => Some(text.trim().parse::<i64>().map_err(|_| {
            ApiError::validacao("desde", "use milissegundos desde 1970, como número inteiro")
        })?),
    };
    Ok(RecordFilter {
        status,
        agent,
        canal: channel,
        since,
    })
}

fn seq_cursor(cursor: Option<&str>) -> Result<Option<i64>, ApiError> {
    match cursor.map(str::trim) {
        None | Some("") => Ok(None),
        Some(text) => text
            .parse::<i64>()
            .ok()
            .filter(|seq| *seq >= 0)
            .map(Some)
            .ok_or_else(|| {
                ApiError::invalido("cursor inválido: use o `next_cursor` da resposta anterior")
                    .com_detalhes(json!({ "field": "cursor", "value": text }))
            }),
    }
}

#[derive(Deserialize)]
pub struct ExecutionPath {
    org_id: String,
    #[serde(rename = "ficha_id")]
    case_id: String,
    run_id: String,
}

#[doc = " Corpo de `POST …/flow-runs`: o fluxo a abrir e a chave que torna a abertura repetível."]
#[derive(Debug, Deserialize, ToSchema)]
#[schema(as = DadosInicio)]
pub struct StartData {
    #[serde(rename = "fluxo_id")]
    flow_id: String,
    #[serde(rename = "chave_idempotencia")]
    idempotency_key: String,
}

#[doc = " Corpo de `POST …/actions`: a ação, os campos que ela grava e a chave."]
#[derive(Debug, Deserialize, ToSchema)]
#[schema(as = DadosAcao)]
pub struct ActionData {
    #[serde(rename = "acao")]
    action: String,
    #[serde(default, rename = "campos")]
    fields: Option<Value>,
    #[serde(rename = "chave_idempotencia")]
    idempotency_key: String,
}

#[doc = " Corpo de `POST …/abandon`: o motivo (fica no histórico) e a chave."]
#[derive(Debug, Deserialize, ToSchema)]
#[schema(as = DadosAbandono)]
pub struct AbandonmentData {
    #[serde(rename = "motivo")]
    reason: String,
    #[serde(rename = "chave_idempotencia")]
    idempotency_key: String,
}

#[doc = " `POST /fichas/{ficha_id}/flow-runs`: o atendente abre um fluxo na Ficha, sem o Jev."]
#[doc = " "]
#[doc = " `If-Match` é a versão da Ficha. Abertura nova responde `201`; repetir a chave responde `200` com a"]
#[doc = " execução que a primeira chamada abriu. Fluxo não liberado ao agente da Ficha: `403`"]
#[doc = " (`fluxo_nao_liberado`, a permissão é conferida pelo store)."]
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs",
    tag = "fichas",
    operation_id = "fichas_iniciar_fluxo",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("ficha_id" = String, Path),
        ("If-Match" = String, Header, description = "Versão atual da Ficha ou da execução, como lida antes"),
    ),
    request_body = StartData,
    responses(
        (status = 201, description = "Execução aberta na etapa de início do fluxo", body = Value),
        (status = 200, description = "Chave já usada: devolve a execução que a primeira chamada abriu, com `repetida: true`", body = Value),
        (status = 400, description = "Chave de idempotência vazia ou corpo inválido", body = crate::erro::RespostaErro),
        (status = 403, description = "Fluxo não liberado ao agente da Ficha (`fluxo_nao_liberado`); nada é aberto", body = crate::erro::RespostaErro),
        (status = 404, description = "Ficha ou fluxo inexistente na org", body = crate::erro::RespostaErro),
        (status = 409, description = "Ficha fechada (`ficha_encerrada`) ou fluxo sem revisão publicada (`conflict`)", body = crate::erro::RespostaErro),
        (status = 412, description = "Versão da Ficha diferente da atual", body = crate::erro::RespostaErro),
        (status = 428, description = "Falta o `If-Match`", body = crate::erro::RespostaErro),
    )
)]
pub async fn start_flow(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CasePath>,
    h: HeaderMap,
    body: Result<Json<StartData>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let case_id = parse_path_id::<RecordId>(&c.case_id)?;
    let version = if_match_version(&h, "ficha")?;
    let Json(data) = body.map_err(|_| {
        ApiError::invalido(
            "corpo JSON inválido: use {\"fluxo_id\": ..., \"chave_idempotencia\": ...}",
        )
    })?;
    let flow = parse_path_id::<FlowId>(&data.flow_id)?;
    let key = key_required(&data.idempotency_key)?;
    if let Some(already_open) = e
        .store
        .executions()
        .opening_already_done(org, case_id, flow, &key)?
    {
        return Ok((
            StatusCode::OK,
            Json(execution_response(&e, org, &already_open, true, &[])?),
        ));
    }
    let case = e.store.cases().get(org, case_id)?;
    if case.status == RecordStatus::Closed {
        return Err(domain(DomainError::RecordClosed {
            id: case_id.to_string(),
        }));
    }
    if case.version != version {
        return Err(ApiError::conflito_de_versao(
            "ficha",
            &case_id.to_string(),
            version,
            case.version,
        ));
    }
    let revision = e
        .store
        .revisions()
        .current(org, flow)?
        .ok_or_else(|| ApiError::conflito("o fluxo ainda não tem revisão publicada"))?;
    let ir = revision_ir(&e, &revision)?;
    let evaluator = revision_evaluator(&e, &revision, &ir)?;
    let case_text = case_id.to_string();
    let peer = case.peer_externo.clone().unwrap_or_default();
    let context = Contexto::novo(&evaluator)
        .com_ficha(&case_text)
        .com_cliente(&peer);
    let state = iniciar(&ir).map_err(domain)?;
    let effect =
        efeitos_de_entrada(&ir, state.no_atual.id(), &state.dados, &context).map_err(domain)?;
    let token = ident.ator_id();
    let stored = e.store.executions().start_execution(
        org,
        ExecutionStart {
            case: case_id,
            flow,
            revision: revision.id,
            actor: EventActor::Attendant,
            ator_ref: Some(&token),
            jev_decision: None,
            initial_state: estado_para_banco(&state),
            inbound_effects: effect.iter().map(efeito_da_transicao).collect(),
            chave_idempotencia: Some(&key),
        },
    )?;
    if !stored.repeated {
        super::audit::register(&e, &ident, Some(org), "flow_run_start", "ficha", case_text)?;
    }
    let status = if stored.repeated {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((
        status,
        Json(execution_response(
            &e,
            org,
            &stored.execution,
            stored.repeated,
            &stored.events,
        )?),
    ))
}

#[doc = " `POST /fichas/{ficha_id}/flow-runs/{run_id}/actions`: o atendente executa uma ação da etapa atual."]
#[doc = " "]
#[doc = " `If-Match` é a versão da **execução**. Se o agente andou por cima (outro escrito primeiro), a"]
#[doc = " versão não bate e a resposta é `412`: é a corrida atendente × agente. Ação que só o agente pode"]
#[doc = " fazer: `409` (`acao_invalida`, a recusa do motor). Campo inválido: `400`."]
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs/{run_id}/actions",
    tag = "fichas",
    operation_id = "fichas_acao",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("ficha_id" = String, Path),
        ("run_id" = String, Path),
        ("If-Match" = String, Header, description = "Versão atual da Ficha ou da execução, como lida antes"),
    ),
    request_body = ActionData,
    responses(
        (status = 200, description = "Execução depois da ação; chave repetida devolve a execução atual, sem evento nem efeito novo", body = Value),
        (status = 400, description = "Chave vazia ou campos inválidos (`campos_invalidos`)", body = crate::erro::RespostaErro),
        (status = 404, description = "Execução fora da Ficha ou da org", body = crate::erro::RespostaErro),
        (status = 409, description = "Ação que só o agente pode fazer (`acao_invalida`) ou Ficha fechada (`ficha_encerrada`)", body = crate::erro::RespostaErro),
        (status = 412, description = "Versão da execução diferente da atual (corrida com o agente)", body = crate::erro::RespostaErro),
        (status = 428, description = "Falta o `If-Match`", body = crate::erro::RespostaErro),
    )
)]
pub async fn apply_action(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<ExecutionPath>,
    h: HeaderMap,
    body: Result<Json<ActionData>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let case_id = parse_path_id::<RecordId>(&c.case_id)?;
    let run = execution_id(&c.run_id)?;
    let version = if_match_version(&h, "execução de fluxo")?;
    let Json(data) = body.map_err(|_| {
        ApiError::invalido(
            "corpo JSON inválido: use {\"acao\": ..., \"campos\": {...}, \"chave_idempotencia\": ...}",
        )
    })?;
    let key = key_required(&data.idempotency_key)?;
    let fields = body_fields(data.fields)?;
    let current = case_execution(&e, org, case_id, run)?;
    if e.store
        .executions()
        .transition_already_applied(org, run, &key)?
    {
        return Ok(Json(execution_response(&e, org, &current, true, &[])?));
    }
    let case = e.store.cases().get(org, case_id)?;
    if case.status == RecordStatus::Closed {
        return Err(domain(DomainError::RecordClosed {
            id: case_id.to_string(),
        }));
    }
    if current.version != version {
        return Err(ApiError::conflito_de_versao(
            "execução de fluxo",
            &run.to_string(),
            version,
            current.version,
        ));
    }
    let (ir, evaluator, state) = action_state(&e, org, &current)?;
    let case_text = case_id.to_string();
    let peer = case.peer_externo.clone().unwrap_or_default();
    let context = Contexto::novo(&evaluator)
        .com_ficha(&case_text)
        .com_cliente(&peer);
    let result = executar_acao(
        &ir,
        &state,
        &data.action,
        &fields,
        ActionActor::Attendant,
        Some(&key),
        &context,
    )
    .map_err(domain)?;
    let token = ident.ator_id();
    let stored = e.store.executions().apply_transition(
        org,
        run,
        version,
        &transicao_do_motor(&result),
        &key,
        TransitionExtras {
            ator_ref: Some(&token),
            fechada_por: Some(ClosedBy::Attendant),
            close_reason: None,
        },
    )?;
    if !stored.repeated {
        super::audit::register(
            &e,
            &ident,
            Some(org),
            "flow_run_action",
            "execucao_fluxo",
            run.to_string(),
        )?;
    }
    Ok(Json(execution_response(
        &e,
        org,
        &stored.execution,
        stored.repeated,
        &stored.events,
    )?))
}

#[doc = " `POST /fichas/{ficha_id}/flow-runs/{run_id}/abandon`: o atendente para a execução onde ela está."]
#[doc = " "]
#[doc = " Não gera efeito. Execução que já não está ativa: `409`. `If-Match` é a versão da execução."]
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs/{run_id}/abandon",
    tag = "fichas",
    operation_id = "fichas_abandonar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("ficha_id" = String, Path),
        ("run_id" = String, Path),
        ("If-Match" = String, Header, description = "Versão atual da Ficha ou da execução, como lida antes"),
    ),
    request_body = AbandonmentData,
    responses(
        (status = 200, description = "Execução abandonada; não gera efeito", body = Value),
        (status = 400, description = "Chave vazia ou corpo inválido", body = crate::erro::RespostaErro),
        (status = 404, description = "Execução fora da Ficha ou da org", body = crate::erro::RespostaErro),
        (status = 409, description = "Execução que já não está ativa, ou Ficha fechada (`ficha_encerrada`)", body = crate::erro::RespostaErro),
        (status = 412, description = "Versão da execução diferente da atual", body = crate::erro::RespostaErro),
        (status = 428, description = "Falta o `If-Match`", body = crate::erro::RespostaErro),
    )
)]
pub async fn abandon(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<ExecutionPath>,
    h: HeaderMap,
    body: Result<Json<AbandonmentData>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let case_id = parse_path_id::<RecordId>(&c.case_id)?;
    let run = execution_id(&c.run_id)?;
    let version = if_match_version(&h, "execução de fluxo")?;
    let Json(data) = body.map_err(|_| {
        ApiError::invalido(
            "corpo JSON inválido: use {\"motivo\": ..., \"chave_idempotencia\": ...}",
        )
    })?;
    let key = key_required(&data.idempotency_key)?;
    let current = case_execution(&e, org, case_id, run)?;
    if e.store
        .executions()
        .transition_already_applied(org, run, &key)?
    {
        return Ok(Json(execution_response(&e, org, &current, true, &[])?));
    }
    let case = e.store.cases().get(org, case_id)?;
    if case.status == RecordStatus::Closed {
        return Err(domain(DomainError::RecordClosed {
            id: case_id.to_string(),
        }));
    }
    if current.version != version {
        return Err(ApiError::conflito_de_versao(
            "execução de fluxo",
            &run.to_string(),
            version,
            current.version,
        ));
    }
    let (_ir, _evaluator, state) = action_state(&e, org, &current)?;
    let result =
        abandon_run(&state, &data.reason, ActionActor::Attendant, Some(&key)).map_err(domain)?;
    let token = ident.ator_id();
    let stored = e.store.executions().apply_transition(
        org,
        run,
        version,
        &transicao_do_motor(&result),
        &key,
        TransitionExtras {
            ator_ref: Some(&token),
            fechada_por: None,
            close_reason: None,
        },
    )?;
    if !stored.repeated {
        super::audit::register(
            &e,
            &ident,
            Some(org),
            "flow_run_abandon",
            "execucao_fluxo",
            run.to_string(),
        )?;
    }
    Ok(Json(execution_response(
        &e,
        org,
        &stored.execution,
        stored.repeated,
        &stored.events,
    )?))
}

#[doc = " `POST /fichas/{ficha_id}/close`: encerra o atendimento. Fecha a conversa e a Ficha juntas e"]
#[doc = " abandona a execução ativa (motivo `atendimento_encerrado`), como o `close` da conversa."]
#[doc = " "]
#[doc = " `If-Match` é a versão da Ficha. Sem conversa ligada (apagada pela retenção), fecha só a Ficha."]
#[doc = " Ficha já fechada: `409`. Não tem `chave_idempotencia`: repetir depois de fechado responde `409`"]
#[doc = " e não gera nada novo."]
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/close",
    tag = "fichas",
    operation_id = "fichas_fechar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("ficha_id" = String, Path),
        ("If-Match" = String, Header, description = "Versão atual da Ficha ou da execução, como lida antes"),
    ),
    responses(
        (status = 200, description = "Ficha fechada (e a conversa, quando há); a execução ativa vai para `abandonada`", body = Value),
        (status = 404, description = "Ficha inexistente ou de outra org", body = crate::erro::RespostaErro),
        (status = 409, description = "Ficha já fechada (`ficha_encerrada`)", body = crate::erro::RespostaErro),
        (status = 412, description = "Versão da Ficha diferente da atual", body = crate::erro::RespostaErro),
        (status = 428, description = "Falta o `If-Match`", body = crate::erro::RespostaErro),
    )
)]
pub async fn close(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CasePath>,
    h: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let case_id = parse_path_id::<RecordId>(&c.case_id)?;
    let expected = if_match_version(&h, "ficha")?;
    let case = e.store.cases().get(org, case_id)?;
    if case.status == RecordStatus::Closed {
        return Err(domain(DomainError::RecordClosed {
            id: case_id.to_string(),
        }));
    }
    if case.version != expected {
        return Err(ApiError::conflito_de_versao(
            "ficha",
            &case_id.to_string(),
            expected,
            case.version,
        ));
    }
    match case.conversation_id {
        Some(conversation) => {
            let record = e.store.conversations().get(org, conversation)?;
            e.store.conversations().close_audited(
                org,
                conversation,
                record.version,
                ident.ator_id(),
            )?;
        }
        None => {
            e.store.cases().close_without_conversation(
                org,
                case_id,
                expected,
                REASON_CLOSING_BY_API,
            )?;
        }
    }
    let current = e.store.cases().get(org, case_id)?;
    Ok(Json(ficha_json(&current)))
}

fn domain(error: DomainError) -> ApiError {
    ApiError::from(StoreError::Domain(error))
}

fn key_required(text: &str) -> Result<String, ApiError> {
    let clean = text.trim();
    if clean.is_empty() {
        return Err(ApiError::validacao(
            "chave_idempotencia",
            "informe uma chave: repetir a mesma tem de devolver o mesmo resultado",
        ));
    }
    Ok(clean.to_owned())
}

fn body_fields(fields: Option<Value>) -> Result<Campos, ApiError> {
    match fields {
        None | Some(Value::Null) => Ok(Campos::new()),
        Some(Value::Object(map)) => Ok(map),
        Some(_) => Err(ApiError::validacao(
            "campos",
            "use um objeto: {\"nome_do_campo\": valor}",
        )),
    }
}

fn case_execution(
    e: &Estado,
    org: OrgId,
    case: RecordId,
    run: ExecutionId,
) -> Result<Execution, ApiError> {
    let execution = e.store.executions().get(org, run)?;
    if execution.ficha_id != case {
        return Err(ApiError::nao_encontrado(
            "execução de fluxo",
            &run.to_string(),
        ));
    }
    Ok(execution)
}

fn revision_ir(e: &Estado, revision: &FlowRevision) -> Result<Arc<Ir>, ApiError> {
    let stored = Ir::from_json(&revision.definicao_json).map_err(|error| {
        tracing::error!(error = ?error, "IR gravada de revisão de fluxo não lê");
        ApiError::interno()
    })?;
    let key = RevisionKey::with_hash(
        revision.fluxo_id.to_string(),
        u32::try_from(revision.numero).unwrap_or(u32::MAX),
        revision.fonte_sha256.clone(),
    );
    e.cache_fluxos
        .load(
            &key,
            &stored,
            &revision.fonte,
            &Limites::da_config(&e.config.flows),
        )
        .map_err(|error| {
            tracing::error!(error = ?error, "revisão de fluxo publicada não carrega");
            ApiError::interno()
        })
}

fn revision_evaluator(
    e: &Estado,
    revision: &FlowRevision,
    ir: &Ir,
) -> Result<RuleEvaluator, ApiError> {
    if ir.defs.is_empty() {
        return Ok(RuleEvaluator::without_functions());
    }
    RuleEvaluator::new_with(
        &revision.fonte,
        &Limites::da_config(&e.config.flows),
        RuleLimits::default(),
    )
    .map_err(|error| {
        tracing::error!(error = ?error, "módulo de regras de revisão não carrega");
        ApiError::interno()
    })
}

fn action_state(
    e: &Estado,
    org: OrgId,
    execution: &Execution,
) -> Result<(Arc<Ir>, RuleEvaluator, EngineState), ApiError> {
    let revision = e.store.revisions().get_by_id(org, execution.revisao_id)?;
    let ir = revision_ir(e, &revision)?;
    let evaluator = revision_evaluator(e, &revision, &ir)?;
    let step = e.store.executions().stages(org, execution.id)?;
    let acting_actors = step
        .iter()
        .rev()
        .find(|etapa| etapa.no_id == execution.no_atual)
        .map(|etapa| etapa.acting_actors.clone())
        .unwrap_or_default();
    let data = serde_json::from_str::<Dados>(&execution.dados_json).map_err(|error| {
        tracing::error!(error = ?error, "dados de execução de fluxo não leem");
        ApiError::interno()
    })?;
    let state = EngineState {
        fluxo_id: ir.fluxo.id.clone(),
        status: execution.status,
        no_atual: destination_id(&ir, &execution.no_atual)?,
        dados: data,
        atores_que_agiram: acting_actors,
        resultado: execution.result.clone(),
        motivo: execution.motivo_fim.clone(),
        keys: BTreeMap::new(),
    };
    Ok((ir, evaluator, state))
}

fn destination_id(ir: &Ir, id: &str) -> Result<Destination, ApiError> {
    if ir.stages.iter().any(|etapa| etapa.id == id) {
        return Ok(Destination::Etapa { id: id.to_owned() });
    }
    if ir.decisions.iter().any(|decisao| decisao.id == id) {
        return Ok(Destination::Decisao { id: id.to_owned() });
    }
    if let Some(fim) = ir.ends.iter().find(|fim| fim.id == id) {
        return Ok(Destination::Fim {
            id: fim.id.clone(),
            resultado: fim.resultado.clone(),
            ends_service: fim.ends_service,
        });
    }
    tracing::error!(no = id, "nó da execução não existe na IR da revisão");
    Err(ApiError::interno())
}

fn execution_response(
    e: &Estado,
    org: OrgId,
    execution: &Execution,
    repeated: bool,
    events: &[RecordEvent],
) -> Result<Value, ApiError> {
    let step: Vec<ExecutionStep> = e.store.executions().stages(org, execution.id)?;
    let mut body = execucao_json(execution, &step);
    body["repetida"] = json!(repeated);
    body["eventos"] = Value::Array(events.iter().map(evento_json).collect());
    Ok(body)
}

fn execution_id(text: &str) -> Result<ExecutionId, ApiError> {
    text.parse::<ExecutionId>().map_err(|_| {
        ApiError::id_invalido(
            "execução de fluxo",
            text,
            "use o id de uma execução (UUID v7)",
        )
    })
}
