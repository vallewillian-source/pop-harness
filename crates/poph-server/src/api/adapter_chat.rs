use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use poph_store::{
    AdapterSubmission, AdapterTurnKey, AdapterTurnOutcome, AdapterTurnSubmission, Direction,
    LIMIT_AUTHOR_NAME, LIMIT_EXTERNAL_MESSAGE_ID, LIMIT_EXTERNAL_REF, LIMIT_IDEMPOTENCY_KEY,
    NewAdapterMessage, NewAdapterTurn, Origin, RESERVED_CHANNELS, Role,
};
use poph_types::{AgentId, ConversationId, MessageId, OrgId, TurnId};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

use crate::auth::Identity;
use crate::error::id_from_url as parse_path_id;
use crate::{ApiError, ServerState};

const LIMIT_CHANNEL: usize = 32;

pub fn routes() -> Router<ServerState> {
    use axum::routing::{get, post};
    Router::new()
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/adapter-conversations",
            post(create),
        )
        .route(
            "/admin/v1/orgs/{org_id}/adapter-conversations/{id}",
            get(show),
        )
        .route(
            "/admin/v1/orgs/{org_id}/adapter-conversations/{id}/messages",
            get(history).post(write_message),
        )
        .route(
            "/admin/v1/orgs/{org_id}/adapter-conversations/{id}/turns",
            post(submit_turn),
        )
        .route(
            "/admin/v1/orgs/{org_id}/adapter-conversations/{id}/turns/{turn_id}",
            get(show_turn),
        )
        .route(
            "/admin/v1/orgs/{org_id}/adapter-conversations/{id}/turns/{turn_id}/cancel",
            post(cancel_turn),
        )
        .route(
            "/admin/v1/orgs/{org_id}/adapter-conversations/{id}/events",
            get(events),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/adapter-conversations/{id}/events",
            get(agent_events),
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

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateBody {
    #[schema(min_length = 1, max_length = 32)]
    channel: String,
    #[schema(min_length = 1, max_length = 128)]
    external_ref: String,
    create_case: Option<bool>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MessageBody {
    text: String,
    origin: String,
    #[schema(max_length = 128)]
    author_name: Option<String>,
    #[schema(max_length = 128)]
    external_message_id: Option<String>,
    #[schema(min_length = 1, max_length = 128)]
    idempotency_key: String,
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    before: Option<String>,
    limit: Option<String>,
}

#[derive(Deserialize)]
pub struct EventsQuery {
    after: Option<String>,
    limit: Option<String>,
}

#[derive(Deserialize)]
pub struct AgentConversationPath {
    org_id: String,
    agent_id: String,
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
pub struct TurnBody {
    text: String,
    #[schema(min_length = 1, max_length = 128)]
    idempotency_key: String,
    #[schema(max_length = 128)]
    author_name: Option<String>,
    #[schema(max_length = 128)]
    external_message_id: Option<String>,
}

fn validation(field: &str, reason: &str) -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_error",
        format!("campo `{field}` inválido"),
    )
    .with_details(json!({ "field": field, "reason": reason }))
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

fn scoped_org(identity: &Identity, raw: &str) -> Result<OrgId, ApiError> {
    let org = parse_path_id::<OrgId>(raw)?;
    identity.require_org(org)?;
    Ok(org)
}

fn channel(raw: &str) -> Result<String, ApiError> {
    let value = raw.trim();
    let valid = (1..=LIMIT_CHANNEL).contains(&value.chars().count())
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if !valid {
        return Err(validation(
            "channel",
            "informe de 1 a 32 caracteres entre a-z, 0-9, _ e -",
        ));
    }
    if RESERVED_CHANNELS.contains(&value) {
        return Err(validation(
            "channel",
            "este canal tem regra própria e não aceita conversa de adaptador",
        ));
    }
    Ok(value.to_owned())
}

fn reference(raw: &str) -> Result<String, ApiError> {
    let value = raw.trim();
    if !(1..=LIMIT_EXTERNAL_REF).contains(&value.chars().count()) {
        return Err(validation("external_ref", "informe de 1 a 128 caracteres"));
    }
    Ok(value.to_owned())
}

fn optional_name(raw: Option<String>, field: &str, max: usize) -> Result<Option<String>, ApiError> {
    raw.map(|value| {
        let value = value.trim().to_owned();
        if value.chars().count() > max {
            return Err(validation(field, &format!("informe até {max} caracteres")));
        }
        Ok((!value.is_empty()).then_some(value))
    })
    .transpose()
    .map(Option::flatten)
}

fn origin(raw: &str) -> Result<(Origin, Direction, Role), ApiError> {
    match raw.trim() {
        "customer" => Ok((Origin::Agente, Direction::Inbound, Role::User)),
        "human" => Ok((Origin::Human, Direction::Outbound, Role::Assistant)),
        "agent" => Ok((Origin::Agente, Direction::Outbound, Role::Assistant)),
        _ => Err(validation("origin", "use customer, human ou agent")),
    }
}

fn scoped_conversation(
    state: &ServerState,
    identity: &Identity,
    raw_org: &str,
    raw_id: &str,
) -> Result<(OrgId, ConversationId, poph_store::AdapterConversation), ApiError> {
    let org = scoped_org(identity, raw_org)?;
    let id = parse_path_id::<ConversationId>(raw_id)?;
    let conversation = state.store.adapter_chat().get(org, id)?;
    Ok((org, id, conversation))
}

fn scoped_turn(
    state: &ServerState,
    identity: &Identity,
    path: &TurnPath,
) -> Result<AdapterTurnKey, ApiError> {
    let (org, conversation, _) = scoped_conversation(state, identity, &path.org_id, &path.id)?;
    let turn = path
        .turn_id
        .parse::<TurnId>()
        .map_err(|_| validation("turn_id", "informe um id de turno"))?;
    let key = AdapterTurnKey {
        org,
        conversation,
        turn,
    };
    state.store.adapter_chat().turn(key)?;
    Ok(key)
}

fn limit(raw: Option<&str>, max: u32, default: u32) -> Result<u32, ApiError> {
    raw.map_or(Ok(default), |raw| {
        raw.parse::<u32>()
            .ok()
            .filter(|value| (1..=max).contains(value))
            .ok_or_else(|| validation("limit", &format!("informe um inteiro de 1 a {max}")))
    })
}

#[utoipa::path(operation_id = "adapter_conversation_create", post, path="/admin/v1/orgs/{org_id}/agents/{agent_id}/adapter-conversations", tag="adapter-conversations", security(("bearer"=[])), params(("org_id"=String, Path), ("agent_id"=String, Path)), request_body=CreateBody, responses((status=201, description="Conversa criada", body=Value), (status=200, description="Conversa existente", body=Value), (status=404, description="Agente fora da organização", body=crate::ErrorResponse), (status=422, description="Canal ou dados recusados", body=crate::ErrorResponse)))]
pub async fn create(
    State(state): State<ServerState>,
    identity: Identity,
    Path(path): Path<AgentPath>,
    json: Result<Json<CreateBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    let org = scoped_org(&identity, &path.org_id)?;
    let agent = parse_path_id::<AgentId>(&path.agent_id)?;
    let data = body(json)?;
    let channel = channel(&data.channel)?;
    let reference = reference(&data.external_ref)?;
    let create_case = data.create_case.unwrap_or(channel != "popbot");
    let (conversation, created) =
        state
            .store
            .adapter_chat()
            .create_or_resume(org, agent, channel, reference, create_case)?;
    if created {
        super::audit::register(
            &state,
            &identity,
            Some(org),
            "adapter_chat_open",
            "conversation",
            conversation.to_string(),
        )?;
    }
    let detail = state.store.adapter_chat().get(org, conversation)?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(json!(detail))).into_response())
}

#[utoipa::path(operation_id = "adapter_conversation_show", get, path="/admin/v1/orgs/{org_id}/adapter-conversations/{id}", tag="adapter-conversations", security(("bearer"=[])), params(("org_id"=String, Path), ("id"=String, Path)), responses((status=200, description="Conversa de adaptador", body=Value), (status=404, description="Conversa não encontrada", body=crate::ErrorResponse)))]
pub async fn show(
    State(state): State<ServerState>,
    identity: Identity,
    Path(path): Path<ConversationPath>,
) -> Result<Json<Value>, ApiError> {
    let (_, _, conversation) = scoped_conversation(&state, &identity, &path.org_id, &path.id)?;
    Ok(Json(json!(conversation)))
}

#[utoipa::path(operation_id = "adapter_message_create", post, path="/admin/v1/orgs/{org_id}/adapter-conversations/{id}/messages", tag="adapter-conversations", security(("bearer"=[])), params(("org_id"=String, Path), ("id"=String, Path)), request_body=MessageBody, responses((status=201, description="Mensagem gravada", body=Value), (status=200, description="Repetição idêntica", body=Value), (status=404, description="Conversa não encontrada", body=crate::ErrorResponse), (status=409, description="Chave repetida com outro conteúdo ou conversa encerrada", body=crate::ErrorResponse), (status=422, description="Dados recusados", body=crate::ErrorResponse)))]
pub async fn write_message(
    State(state): State<ServerState>,
    identity: Identity,
    Path(path): Path<ConversationPath>,
    json: Result<Json<MessageBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    let (org, conversation, _) = scoped_conversation(&state, &identity, &path.org_id, &path.id)?;
    let data = body(json)?;
    let max = state.config.adapter_chat.max_text_bytes;
    if !(1..=max).contains(&data.text.len()) {
        return Err(validation("text", &format!("informe de 1 a {max} bytes")));
    }
    if !(1..=LIMIT_IDEMPOTENCY_KEY).contains(&data.idempotency_key.chars().count()) {
        return Err(validation(
            "idempotency_key",
            "informe de 1 a 128 caracteres",
        ));
    }
    let author_name = optional_name(data.author_name, "author_name", LIMIT_AUTHOR_NAME)?;
    let external_message_id = optional_name(
        data.external_message_id,
        "external_message_id",
        LIMIT_EXTERNAL_MESSAGE_ID,
    )?;
    let (origin, direction, role) = origin(&data.origin)?;
    let (status, message) = match state.store.adapter_chat().write_message(
        org,
        conversation,
        NewAdapterMessage {
            text: data.text,
            direction,
            role,
            origin,
            author_name,
            external_message_id,
            idempotency_key: data.idempotency_key,
        },
    )? {
        AdapterSubmission::Stored(message, true) => {
            super::audit::register(
                &state,
                &identity,
                Some(org),
                "adapter_chat_message",
                "message",
                message.id.to_string(),
            )?;
            (StatusCode::CREATED, message)
        }
        AdapterSubmission::Stored(message, false) => (StatusCode::OK, message),
        AdapterSubmission::Conflict => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "idempotency_conflict",
                "a chave já foi usada para outro conteúdo",
            ));
        }
    };
    Ok((status, Json(json!(message))).into_response())
}

#[utoipa::path(operation_id = "adapter_message_history", get, path="/admin/v1/orgs/{org_id}/adapter-conversations/{id}/messages", tag="adapter-conversations", security(("bearer"=[])), params(("org_id"=String, Path), ("id"=String, Path), ("before"=Option<String>, Query), ("limit"=Option<u32>, Query)), responses((status=200, description="Histórico da conversa, da mais nova para a mais antiga", body=Value), (status=404, description="Conversa não encontrada", body=crate::ErrorResponse)))]
pub async fn history(
    State(state): State<ServerState>,
    identity: Identity,
    Path(path): Path<ConversationPath>,
    parsed: Result<Query<HistoryQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let (org, conversation, _) = scoped_conversation(&state, &identity, &path.org_id, &path.id)?;
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
        json!({"messages": state.store.adapter_chat().history(org, conversation, before, limit)?}),
    ))
}

#[utoipa::path(operation_id = "adapter_turn_create", post, path="/admin/v1/orgs/{org_id}/adapter-conversations/{id}/turns", tag="adapter-conversations", security(("bearer"=[])), params(("org_id"=String, Path), ("id"=String, Path)), request_body=TurnBody, responses((status=202, description="Turno aceito", body=Value), (status=200, description="Repetição da chave", body=Value), (status=404, description="Conversa não encontrada", body=crate::ErrorResponse), (status=409, description="Turno em andamento, chave repetida com outro conteúdo ou conversa encerrada", body=crate::ErrorResponse), (status=422, description="Dados recusados", body=crate::ErrorResponse)))]
pub async fn submit_turn(
    State(state): State<ServerState>,
    identity: Identity,
    Path(path): Path<ConversationPath>,
    json: Result<Json<TurnBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    let (org, conversation, _) = scoped_conversation(&state, &identity, &path.org_id, &path.id)?;
    let data = body(json)?;
    let max = state.config.adapter_chat.max_text_bytes;
    if !(1..=max).contains(&data.text.len()) {
        return Err(validation("text", &format!("informe de 1 a {max} bytes")));
    }
    if !(1..=LIMIT_IDEMPOTENCY_KEY).contains(&data.idempotency_key.chars().count()) {
        return Err(validation(
            "idempotency_key",
            "informe de 1 a 128 caracteres",
        ));
    }
    let author_name = optional_name(data.author_name, "author_name", LIMIT_AUTHOR_NAME)?;
    let external_message_id = optional_name(
        data.external_message_id,
        "external_message_id",
        LIMIT_EXTERNAL_MESSAGE_ID,
    )?;
    let (status, message, turn, state_label) = match state.store.adapter_chat().submit_turn(
        org,
        conversation,
        NewAdapterTurn {
            idempotency_key: data.idempotency_key,
            text: data.text,
            author_name,
            external_message_id,
        },
    )? {
        AdapterTurnSubmission::Accepted { message, turn } => {
            super::audit::register(
                &state,
                &identity,
                Some(org),
                "adapter_chat_turn",
                "turn",
                turn.to_string(),
            )?;
            crate::adapter_chat::launch(
                state.clone(),
                AdapterTurnKey {
                    org,
                    conversation,
                    turn,
                },
            );
            (StatusCode::ACCEPTED, message, turn, "queued".to_owned())
        }
        AdapterTurnSubmission::Replayed {
            message,
            turn,
            status,
        } => (StatusCode::OK, message, turn, status),
        AdapterTurnSubmission::Conflict => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "idempotency_conflict",
                "a chave já foi usada para outro conteúdo",
            ));
        }
        AdapterTurnSubmission::Busy(turn) => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "turn_in_progress",
                "esta conversa já tem um turno em andamento",
            )
            .with_details(json!({"turn_id": turn})));
        }
    };
    Ok((
        status,
        Json(json!({"turn_id": turn, "message_id": message, "status": state_label})),
    )
        .into_response())
}

#[utoipa::path(operation_id = "adapter_turn_show", get, path="/admin/v1/orgs/{org_id}/adapter-conversations/{id}/turns/{turn_id}", tag="adapter-conversations", security(("bearer"=[])), params(("org_id"=String, Path), ("id"=String, Path), ("turn_id"=String, Path)), responses((status=200, description="Estado do turno", body=Value), (status=404, description="Turno não encontrado", body=crate::ErrorResponse)))]
pub async fn show_turn(
    State(state): State<ServerState>,
    identity: Identity,
    Path(path): Path<TurnPath>,
) -> Result<Json<Value>, ApiError> {
    let key = scoped_turn(&state, &identity, &path)?;
    Ok(Json(json!(state.store.adapter_chat().turn(key)?)))
}

#[utoipa::path(operation_id = "adapter_turn_cancel", post, path="/admin/v1/orgs/{org_id}/adapter-conversations/{id}/turns/{turn_id}/cancel", tag="adapter-conversations", security(("bearer"=[])), params(("org_id"=String, Path), ("id"=String, Path), ("turn_id"=String, Path)), responses((status=200, description="Estado final do turno", body=Value), (status=404, description="Turno não encontrado", body=crate::ErrorResponse)))]
pub async fn cancel_turn(
    State(state): State<ServerState>,
    identity: Identity,
    Path(path): Path<TurnPath>,
) -> Result<Json<Value>, ApiError> {
    let key = scoped_turn(&state, &identity, &path)?;
    let previous = state.store.adapter_chat().turn(key)?;
    let result = state
        .store
        .adapter_chat()
        .finish_turn(key, AdapterTurnOutcome::Cancelled)?;
    state.adapter_workers.cancel(key);
    if previous.status != "cancelled" && result.status == "cancelled" {
        super::audit::register(
            &state,
            &identity,
            Some(key.org),
            "adapter_chat_turn_cancel",
            "turn",
            key.turn.to_string(),
        )?;
    }
    Ok(Json(json!(result)))
}

#[utoipa::path(operation_id = "adapter_conversation_events", get, path="/admin/v1/orgs/{org_id}/adapter-conversations/{id}/events", tag="adapter-conversations", security(("bearer"=[])), params(("org_id"=String, Path), ("id"=String, Path), ("after"=Option<i64>, Query), ("limit"=Option<u32>, Query)), responses((status=200, description="Eventos ordenados após o cursor", body=Value), (status=404, description="Conversa não encontrada", body=crate::ErrorResponse), (status=422, description="Cursor ou limite inválido", body=crate::ErrorResponse)))]
pub async fn events(
    State(state): State<ServerState>,
    identity: Identity,
    Path(path): Path<ConversationPath>,
    parsed: Result<Query<EventsQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let (org, id, _) = scoped_conversation(&state, &identity, &path.org_id, &path.id)?;
    read_events(&state, org, id, query(parsed)?)
}

#[utoipa::path(operation_id = "adapter_agent_conversation_events", get, path="/admin/v1/orgs/{org_id}/agents/{agent_id}/adapter-conversations/{id}/events", tag="adapter-conversations", security(("bearer"=[])), params(("org_id"=String, Path), ("agent_id"=String, Path), ("id"=String, Path), ("after"=Option<i64>, Query), ("limit"=Option<u32>, Query)), responses((status=200, description="Eventos ordenados após o cursor", body=Value), (status=404, description="Conversa não encontrada", body=crate::ErrorResponse), (status=422, description="Cursor ou limite inválido", body=crate::ErrorResponse)))]
pub async fn agent_events(
    State(state): State<ServerState>,
    identity: Identity,
    Path(path): Path<AgentConversationPath>,
    parsed: Result<Query<EventsQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let (org, id, conversation) = scoped_conversation(&state, &identity, &path.org_id, &path.id)?;
    if parse_path_id::<AgentId>(&path.agent_id)? != conversation.agent_id {
        return Err(ApiError::not_found("conversa", &path.id));
    }
    read_events(&state, org, id, query(parsed)?)
}

fn read_events(
    state: &ServerState,
    org: OrgId,
    id: ConversationId,
    options: EventsQuery,
) -> Result<Json<Value>, ApiError> {
    let after = options.after.as_deref().map_or(Ok(0), |raw| {
        raw.parse::<i64>()
            .ok()
            .filter(|value| *value >= 0)
            .ok_or_else(|| validation("after", "informe um inteiro não negativo"))
    })?;
    let limit = limit(options.limit.as_deref(), 200, 100)?;
    let events = state.store.adapter_chat().events(org, id, after, limit)?;
    Ok(Json(json!({"events": events})))
}
