use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url},
    estado::Estado,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use poph_types::OrgId;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

#[derive(Deserialize)]
pub struct OrgPath {
    org_id: String,
}
#[derive(Deserialize)]
pub struct ConversationQuery {
    from: Option<String>,
    to: Option<String>,
    agent_id: Option<String>,
    channel: Option<String>,
    tz_offset_minutes: Option<String>,
}
#[derive(Serialize, ToSchema)]
pub struct SummaryJson {
    total: i64,
    open: i64,
    transferred: i64,
    closed: i64,
    avg_messages_per_conversation: Option<f64>,
    conversation_to_ficha_rate: Option<f64>,
}
#[derive(Serialize, ToSchema)]
pub struct DayJson {
    date: String,
    total: i64,
    open: i64,
    transferred: i64,
    closed: i64,
}
#[derive(Serialize, ToSchema)]
pub struct AgentJson {
    agent_id: String,
    agent_name: String,
    total: i64,
    open: i64,
    transferred: i64,
    closed: i64,
}
#[derive(Serialize, ToSchema)]
pub struct ChannelJson {
    channel: String,
    total: i64,
}
#[derive(Serialize, ToSchema)]
pub struct ConversationReportJson {
    summary: SummaryJson,
    by_day: Vec<DayJson>,
    by_agent: Vec<AgentJson>,
    by_channel: Vec<ChannelJson>,
}

#[utoipa::path(get, path = "/admin/v1/orgs/{org_id}/analytics/conversations", tag = "analytics", operation_id = "conversation_analytics", security(("bearer" = [])), params(("org_id" = String, Path), ("from" = i64, Query), ("to" = i64, Query), ("agent_id" = Option<String>, Query), ("channel" = Option<String>, Query), ("tz_offset_minutes" = Option<i64>, Query)), responses((status = 200, body = ConversationReportJson), (status = 422, body = crate::erro::RespostaErro), (status = 404, body = crate::erro::RespostaErro)))]
pub async fn report(
    State(state): State<Estado>,
    identity: Identidade,
    Path(path): Path<OrgPath>,
    Query(query): Query<ConversationQuery>,
) -> Result<Json<ConversationReportJson>, ApiError> {
    let org = id_da_url::<OrgId>(&path.org_id)?;
    identity.exigir_org(org)?;
    let from = parse("from", query.from.as_deref())?;
    let to = parse("to", query.to.as_deref())?;
    if from >= to {
        return Err(invalid("from", "from must be before to."));
    }
    let offset = match query.tz_offset_minutes.as_deref() {
        Some(value) => parse("tz_offset_minutes", Some(value))?,
        None => 0,
    };
    if !(-840..=840).contains(&offset) {
        return Err(invalid("tz_offset_minutes", "must be between -840 and 840"));
    }
    state.store.orgs().get(org)?;
    let optional = |value: Option<String>| value.filter(|s| !s.is_empty());
    let result = state.store.conversation_analytics().report(
        org,
        from,
        to,
        optional(query.agent_id),
        optional(query.channel),
        offset,
    )?;
    let counts = result.summary.counts;
    Ok(Json(ConversationReportJson {
        summary: SummaryJson {
            total: counts.total,
            open: counts.open,
            transferred: counts.transferred,
            closed: counts.closed,
            avg_messages_per_conversation: result.summary.avg_messages_per_conversation,
            conversation_to_ficha_rate: result.summary.conversation_to_ficha_rate,
        },
        by_day: result
            .by_day
            .into_iter()
            .map(|d| DayJson {
                date: d.date,
                total: d.counts.total,
                open: d.counts.open,
                transferred: d.counts.transferred,
                closed: d.counts.closed,
            })
            .collect(),
        by_agent: result
            .by_agent
            .into_iter()
            .map(|a| AgentJson {
                agent_id: a.agent_id,
                agent_name: a.agent_name,
                total: a.counts.total,
                open: a.counts.open,
                transferred: a.counts.transferred,
                closed: a.counts.closed,
            })
            .collect(),
        by_channel: result
            .by_channel
            .into_iter()
            .map(|c| ChannelJson {
                channel: c.channel,
                total: c.total,
            })
            .collect(),
    }))
}
fn parse(field: &'static str, value: Option<&str>) -> Result<i64, ApiError> {
    value
        .ok_or_else(|| invalid(field, "Field required"))?
        .trim()
        .parse()
        .map_err(|_| invalid(field, "Input should be a valid integer"))
}
fn invalid(field: &'static str, reason: &str) -> ApiError {
    ApiError::novo(
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_error",
        format!("campo `{field}` inválido"),
    )
    .com_detalhes(json!({"field":field,"reason":reason}))
}
