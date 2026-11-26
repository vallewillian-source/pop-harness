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
pub struct UsageQuery {
    from: Option<String>,
    to: Option<String>,
}
#[derive(Serialize, ToSchema)]
pub struct AgentUsageJson {
    agent_id: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
    turn_count: i64,
}
#[derive(Serialize, ToSchema)]
pub struct DayUsageJson {
    date: String,
    output_tokens: i64,
    turn_count: i64,
}
#[derive(Serialize, ToSchema)]
pub struct TokenUsageJson {
    total_input_tokens: i64,
    total_output_tokens: i64,
    total_cost_micros: i64,
    turn_count: i64,
    by_agent: Vec<AgentUsageJson>,
    by_day: Vec<DayUsageJson>,
}

#[utoipa::path(get, path = "/admin/v1/orgs/{org_id}/usage/tokens", tag = "usage", operation_id = "usage_tokens", security(("bearer" = [])), params(("org_id" = String, Path), ("from" = i64, Query), ("to" = i64, Query)), responses((status = 200, body = TokenUsageJson), (status = 422, body = crate::erro::RespostaErro), (status = 404, body = crate::erro::RespostaErro)))]
pub async fn tokens(
    State(state): State<Estado>,
    identity: Identidade,
    Path(path): Path<OrgPath>,
    Query(query): Query<UsageQuery>,
) -> Result<Json<TokenUsageJson>, ApiError> {
    let org = id_da_url::<OrgId>(&path.org_id)?;
    identity.exigir_org(org)?;
    let from = parse("from", query.from.as_deref())?;
    let to = parse("to", query.to.as_deref())?;
    if from >= to {
        return Err(invalid("from", "from must be before to."));
    }
    state.store.orgs().get(org)?;
    let result = state.store.usage().tokens(org, from, to)?;
    Ok(Json(TokenUsageJson {
        total_input_tokens: result.total_input_tokens,
        total_output_tokens: result.total_output_tokens,
        total_cost_micros: result.total_cost_micros,
        turn_count: result.turn_count,
        by_agent: result
            .by_agent
            .into_iter()
            .map(|a| AgentUsageJson {
                agent_id: a.agent_id,
                input_tokens: a.input_tokens,
                output_tokens: a.output_tokens,
                turn_count: a.turn_count,
            })
            .collect(),
        by_day: result
            .by_day
            .into_iter()
            .map(|d| DayUsageJson {
                date: d.date,
                output_tokens: d.output_tokens,
                turn_count: d.turn_count,
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
