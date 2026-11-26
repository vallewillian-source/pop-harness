use std::collections::VecDeque;
use std::time::Duration;

use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use futures::stream;
use poph_store::{SiteEvent, SiteOutcome, SiteSubmission, SiteTurnKey};
use poph_types::{AgentId, ConversationId, MessageId, OrgId, StructuredInstructions, TurnId};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use utoipa::ToSchema;

use crate::auth::Identidade;
use crate::erro::id_da_url;
use crate::{ApiError, Estado};

pub fn routes() -> Router<Estado> {
    use axum::routing::{get, post};
    Router::new()
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/site-conversations",
            post(create),
        )
        .route(
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/messages",
            get(history).post(submit),
        )
        .route(
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/turns/{turn_id}",
            get(turn),
        )
        .route(
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/events",
            get(events),
        )
        .route(
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/events/stream",
            get(events_stream),
        )
        .route(
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/turns/{turn_id}/cancel",
            post(cancel),
        )
}

#[derive(Deserialize)]
pub struct AgentPath {
    org_id: String,
    agent_id: String,
}

#[derive(Deserialize)]
pub struct ConversationPath {
    org_id: String,
    id: String,
}

#[derive(Deserialize)]
pub struct TurnPath {
    org_id: String,
    id: String,
    turn_id: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateBody {
    #[schema(min_length = 1, max_length = 128)]
    external_ref: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MessageBody {
    text: String,
    #[schema(min_length = 1, max_length = 128)]
    idempotency_key: String,
}

#[derive(Deserialize)]
pub struct EventsQuery {
    after: Option<String>,
    limit: Option<String>,
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    before: Option<String>,
    limit: Option<String>,
}

fn validation(field: &str, reason: &str) -> ApiError {
    ApiError::novo(
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_error",
        format!("campo `{field}` inválido"),
    )
    .com_detalhes(json!({ "field": field, "reason": reason }))
}

fn body<T>(value: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    value
        .map(|Json(data)| data)
        .map_err(|_| validation("body", "envie um objeto JSON com os campos da rota"))
}

fn query<T>(value: Result<Query<T>, QueryRejection>) -> Result<T, ApiError> {
    value
        .map(|Query(data)| data)
        .map_err(|_| validation("query", "parâmetros de consulta inválidos"))
}

fn scoped_org(identity: &Identidade, raw: &str) -> Result<OrgId, ApiError> {
    let org = id_da_url::<OrgId>(raw)?;
    identity.exigir_org(org)?;
    Ok(org)
}

fn allowed_agent(state: &Estado, org: OrgId, agent: AgentId) -> Result<(), ApiError> {
    state.store.agents().get(org, agent)?;
    let configured = match state.store.instructions().get(org, agent) {
        Ok(instructions) => {
            StructuredInstructions::from_tolerant_json(&instructions.structured_json).ok()
        }
        Err(error) if error.is_not_found() => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if configured.is_some_and(|instructions| {
        instructions
            .enabled_tools()
            .iter()
            .all(|tool| state.config.site_chat.permits_tool(&tool.name))
    }) {
        return Ok(());
    }
    Err(ApiError::novo(
        StatusCode::UNPROCESSABLE_ENTITY,
        "agent_not_public",
        "este agente tem ferramentas indisponíveis para conversas do site",
    ))
}

fn scoped_conversation(
    state: &Estado,
    identity: &Identidade,
    raw_org: &str,
    raw_id: &str,
) -> Result<(OrgId, ConversationId), ApiError> {
    let org = scoped_org(identity, raw_org)?;
    let id = id_da_url::<ConversationId>(raw_id)?;
    let conversation = state.store.conversations().get(org, id)?;
    if conversation.canal != "site" {
        return Err(ApiError::nao_encontrado("conversa", raw_id));
    }
    allowed_agent(state, org, conversation.agent_id)?;
    Ok((org, id))
}

fn scoped_turn(
    state: &Estado,
    identity: &Identidade,
    path: &TurnPath,
) -> Result<SiteTurnKey, ApiError> {
    let (org, conversation) = scoped_conversation(state, identity, &path.org_id, &path.id)?;
    let turn = path
        .turn_id
        .parse::<TurnId>()
        .map_err(|_| validation("turn_id", "informe um id de turno"))?;
    Ok(SiteTurnKey {
        org,
        conversation,
        turn,
    })
}

fn cursor(raw: Option<&str>, field: &str) -> Result<i64, ApiError> {
    raw.map_or(Ok(0), |raw| {
        raw.parse::<i64>()
            .ok()
            .filter(|value| *value >= 0)
            .ok_or_else(|| validation(field, "informe um inteiro não negativo"))
    })
}

fn limit(raw: Option<&str>, max: u32, default: u32) -> Result<u32, ApiError> {
    raw.map_or(Ok(default), |raw| {
        raw.parse::<u32>()
            .ok()
            .filter(|value| (1..=max).contains(value))
            .ok_or_else(|| validation("limit", &format!("informe um inteiro de 1 a {max}")))
    })
}

#[utoipa::path(post, path="/admin/v1/orgs/{org_id}/agents/{agent_id}/site-conversations", tag="site-conversations", security(("bearer"=[])), params(("org_id"=String, Path), ("agent_id"=String, Path)), request_body=CreateBody, responses((status=201, description="Conversa criada", body=Value), (status=200, description="Conversa existente", body=Value), (status=422, description="Agente ou dados recusados", body=crate::RespostaErro)))]
pub async fn create(
    State(state): State<Estado>,
    identity: Identidade,
    Path(path): Path<AgentPath>,
    json: Result<Json<CreateBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    let org = scoped_org(&identity, &path.org_id)?;
    let agent = id_da_url::<AgentId>(&path.agent_id)?;
    allowed_agent(&state, org, agent)?;
    let data = body(json)?;
    if !(1..=128).contains(&data.external_ref.chars().count()) {
        return Err(validation("external_ref", "informe de 1 a 128 caracteres"));
    }
    let (conversation, created) = state
        .store
        .site_chat()
        .create(org, agent, data.external_ref)?;
    if created {
        super::auditoria::registrar(
            &state,
            &identity,
            Some(org),
            "site_chat_open",
            "conversation",
            conversation.to_string(),
        )?;
    }
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(json!({"conversation_id": conversation}))).into_response())
}

#[utoipa::path(post, path="/admin/v1/orgs/{org_id}/site-conversations/{id}/messages", tag="site-conversations", security(("bearer"=[])), params(("org_id"=String, Path), ("id"=String, Path)), request_body=MessageBody, responses((status=202, description="Mensagem aceita", body=Value), (status=200, description="Repetição da mensagem", body=Value), (status=409, description="Turno ativo ou chave já usada para outro texto", body=crate::RespostaErro), (status=422, description="Agente ou dados recusados", body=crate::RespostaErro)))]
pub async fn submit(
    State(state): State<Estado>,
    identity: Identidade,
    Path(path): Path<ConversationPath>,
    json: Result<Json<MessageBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    let (org, conversation) = scoped_conversation(&state, &identity, &path.org_id, &path.id)?;
    let data = body(json)?;
    let max = state.config.site_chat.max_text_bytes;
    if !(1..=max).contains(&data.text.len()) {
        return Err(validation("text", &format!("informe de 1 a {max} bytes")));
    }
    if !(1..=128).contains(&data.idempotency_key.chars().count()) {
        return Err(validation(
            "idempotency_key",
            "informe de 1 a 128 caracteres",
        ));
    }
    let (status, message_id, turn_id) =
        match state
            .store
            .site_chat()
            .submit(org, conversation, data.idempotency_key, data.text)?
        {
            SiteSubmission::Accepted(message, turn) => {
                super::auditoria::registrar(
                    &state,
                    &identity,
                    Some(org),
                    "site_chat_message",
                    "message",
                    message.to_string(),
                )?;
                crate::site_chat::launch(
                    state,
                    SiteTurnKey {
                        org,
                        conversation,
                        turn,
                    },
                );
                (StatusCode::ACCEPTED, message, turn)
            }
            SiteSubmission::Replayed(message, turn) => (StatusCode::OK, message, turn),
            SiteSubmission::Conflict => {
                return Err(ApiError::novo(
                    StatusCode::CONFLICT,
                    "idempotency_conflict",
                    "a chave já foi usada para outro texto",
                ));
            }
            SiteSubmission::Busy(turn) => {
                return Err(ApiError::novo(
                    StatusCode::CONFLICT,
                    "turn_in_progress",
                    "esta conversa já tem um turno em andamento",
                )
                .com_detalhes(json!({"turn_id": turn})));
            }
        };
    Ok((
        status,
        Json(json!({"message_id": message_id, "turn_id": turn_id, "status": "accepted"})),
    )
        .into_response())
}

#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/site-conversations/{id}/turns/{turn_id}", tag="site-conversations", security(("bearer"=[])), params(("org_id"=String, Path), ("id"=String, Path), ("turn_id"=String, Path)), responses((status=200, description="Estado do turno", body=Value), (status=404, description="Turno não encontrado", body=crate::RespostaErro)))]
pub async fn turn(
    State(state): State<Estado>,
    identity: Identidade,
    Path(path): Path<TurnPath>,
) -> Result<Json<Value>, ApiError> {
    let key = scoped_turn(&state, &identity, &path)?;
    Ok(Json(json!(state.store.site_chat().turn(key)?)))
}

#[utoipa::path(post, path="/admin/v1/orgs/{org_id}/site-conversations/{id}/turns/{turn_id}/cancel", tag="site-conversations", security(("bearer"=[])), params(("org_id"=String, Path), ("id"=String, Path), ("turn_id"=String, Path)), responses((status=200, description="Estado final do turno", body=Value), (status=404, description="Turno não encontrado", body=crate::RespostaErro)))]
pub async fn cancel(
    State(state): State<Estado>,
    identity: Identidade,
    Path(path): Path<TurnPath>,
) -> Result<Json<Value>, ApiError> {
    let key = scoped_turn(&state, &identity, &path)?;
    let previous = state.store.site_chat().turn(key.clone())?;
    let result = state
        .store
        .site_chat()
        .finish(key.clone(), SiteOutcome::Cancelled)?;
    state.site_workers.cancel(key.clone());
    if previous.status != "cancelled" && result.status == "cancelled" {
        super::auditoria::registrar(
            &state,
            &identity,
            Some(key.org),
            "site_chat_cancel",
            "turn",
            key.turn.to_string(),
        )?;
    }
    Ok(Json(json!(result)))
}

#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/site-conversations/{id}/messages", tag="site-conversations", security(("bearer"=[])), params(("org_id"=String, Path), ("id"=String, Path), ("before"=Option<String>, Query), ("limit"=Option<u32>, Query)), responses((status=200, description="Mensagens de visitante e agente, da mais nova para a mais antiga", body=Value), (status=404, description="Conversa não encontrada", body=crate::RespostaErro)))]
pub async fn history(
    State(state): State<Estado>,
    identity: Identidade,
    Path(path): Path<ConversationPath>,
    parsed: Result<Query<HistoryQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let (org, conversation) = scoped_conversation(&state, &identity, &path.org_id, &path.id)?;
    let options = query(parsed)?;
    let before = options
        .before
        .map(|raw| {
            raw.parse::<MessageId>()
                .map_err(|_| validation("before", "informe um id de mensagem"))
        })
        .transpose()?;
    let limit = limit(options.limit.as_deref(), 100, 50)?;
    Ok(Json(
        json!({"messages": state.store.site_chat().history(org, conversation, before, limit)?}),
    ))
}

#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/site-conversations/{id}/events", tag="site-conversations", security(("bearer"=[])), params(("org_id"=String, Path), ("id"=String, Path), ("after"=Option<i64>, Query), ("limit"=Option<u32>, Query)), responses((status=200, description="Eventos ordenados após o cursor", body=Value), (status=404, description="Conversa não encontrada", body=crate::RespostaErro)))]
pub async fn events(
    State(state): State<Estado>,
    identity: Identidade,
    Path(path): Path<ConversationPath>,
    parsed: Result<Query<EventsQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let (org, conversation) = scoped_conversation(&state, &identity, &path.org_id, &path.id)?;
    let options = query(parsed)?;
    let after = cursor(options.after.as_deref(), "after")?;
    let limit = limit(options.limit.as_deref(), 200, 100)?;
    Ok(Json(
        json!({"events": state.store.site_chat().events(org, conversation, after, limit)?}),
    ))
}

#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/site-conversations/{id}/events/stream", tag="site-conversations", security(("bearer"=[])), params(("org_id"=String, Path), ("id"=String, Path), ("after"=Option<i64>, Query), ("Last-Event-ID"=Option<String>, Header)), responses((status=200, description="Eventos do site em SSE", body=String, content_type="text/event-stream"), (status=404, description="Conversa não encontrada", body=crate::RespostaErro)))]
pub async fn events_stream(
    State(state): State<Estado>,
    identity: Identidade,
    Path(path): Path<ConversationPath>,
    headers: HeaderMap,
    parsed: Result<Query<EventsQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let (org, conversation) = scoped_conversation(&state, &identity, &path.org_id, &path.id)?;
    let options = query(parsed)?;
    let after = cursor(options.after.as_deref(), "after")?;
    let header = headers
        .get("last-event-id")
        .map(|header| {
            header.to_str().map_err(|_| {
                validation("Last-Event-ID", "use o identificador de um evento anterior")
            })
        })
        .transpose()?;
    let after = after.max(cursor(header, "Last-Event-ID")?);
    let changes = state.site_workers.changes.subscribe();
    let initial = state
        .store
        .site_chat()
        .events(org, conversation, after, 200)?;
    let stream = stream::unfold(
        (state, changes, VecDeque::from(initial), after),
        move |(state, mut changes, mut buffer, mut after)| async move {
            loop {
                if let Some(item) = buffer.pop_front() {
                    after = item.seq;
                    let event = encode(item);
                    return Some((event, (state, changes, buffer, after)));
                }
                let page = match state
                    .store
                    .site_chat()
                    .events(org, conversation, after, 200)
                {
                    Ok(page) => page,
                    Err(error) => {
                        tracing::error!(%error, "falha ao ler eventos do site");
                        return None;
                    }
                };
                if !page.is_empty() {
                    buffer.extend(page);
                    continue;
                }
                loop {
                    match changes.recv().await {
                        Ok(scope) if scope != (org, conversation) => {}
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => break,
                        Err(broadcast::error::RecvError::Closed) => return None,
                    }
                }
            }
        },
    );
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response())
}

fn encode(item: SiteEvent) -> Result<Event, axum::Error> {
    Event::default()
        .id(item.seq.to_string())
        .event(&item.kind)
        .json_data(&item)
}
