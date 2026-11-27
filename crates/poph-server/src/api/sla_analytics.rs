use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url as parse_path_id},
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
use std::time::{SystemTime, UNIX_EPOCH};
use utoipa::ToSchema;

#[derive(Deserialize)]
pub struct OrgPath {
    org_id: String,
}
#[derive(Deserialize)]
pub struct SlaQuery {
    to: Option<String>,
    now: Option<String>,
    limit: Option<String>,
    age_bounds_minutes: Option<String>,
}
#[derive(Serialize, ToSchema)]
pub struct AgeBucket {
    min_minutes: i64,
    max_minutes: Option<i64>,
    count: i64,
}
#[derive(Serialize, ToSchema)]
pub struct StaleItem {
    ficha_id: String,
    conversation_id: Option<String>,
    channel: String,
    peer: Option<String>,
    client_name: Option<String>,
    current_step_key: Option<String>,
    age_ms: i64,
    idle_ms: i64,
}
#[derive(Serialize, ToSchema)]
pub struct SlaReportJson {
    age_buckets: Vec<AgeBucket>,
    stale_items: Vec<StaleItem>,
}

#[utoipa::path(get, path = "/admin/v1/orgs/{org_id}/analytics/sla", tag = "analytics", operation_id = "sla_analytics", security(("bearer" = [])), params(("org_id" = String, Path), ("to" = i64, Query), ("now" = Option<i64>, Query), ("limit" = Option<i64>, Query), ("age_bounds_minutes" = Option<String>, Query)), responses((status = 200, body = SlaReportJson), (status = 422, body = crate::erro::RespostaErro), (status = 404, body = crate::erro::RespostaErro)))]
pub async fn report(
    State(state): State<Estado>,
    identity: Identidade,
    Path(path): Path<OrgPath>,
    Query(query): Query<SlaQuery>,
) -> Result<Json<SlaReportJson>, ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    identity.exigir_org(org)?;
    let to = parse("to", query.to.as_deref())?;
    let now = match query.now.as_deref() {
        Some(v) => parse("now", Some(v))?,
        None => SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0),
    };
    let limit = match query.limit.as_deref() {
        Some(v) => parse("limit", Some(v))?,
        None => 20,
    };
    if !(1..=100).contains(&limit) {
        return Err(invalid("limit", "must be between 1 and 100"));
    }
    let raw = query
        .age_bounds_minutes
        .as_deref()
        .unwrap_or("60,240,1440,2880,4320,10080");
    let bounds: Vec<i64> = raw
        .split(',')
        .map(|v| parse("age_bounds_minutes", Some(v)))
        .collect::<Result<_, _>>()?;
    if bounds.is_empty()
        || bounds.len() > 16
        || bounds.iter().any(|b| *b <= 0 || *b > i64::MAX / 60000)
        || bounds.windows(2).any(|w| w[0] >= w[1])
    {
        return Err(invalid(
            "age_bounds_minutes",
            "must be a strictly increasing list of at most 16 positive integers",
        ));
    }
    state.store.orgs().get(org)?;
    let result = state
        .store
        .sla_analytics()
        .report(org, to, now, limit as usize)?;
    let mut buckets: Vec<AgeBucket> = bounds
        .iter()
        .enumerate()
        .map(|(i, b)| AgeBucket {
            min_minutes: if i == 0 { 0 } else { bounds[i - 1] },
            max_minutes: Some(*b),
            count: 0,
        })
        .collect();
    buckets.push(AgeBucket {
        min_minutes: bounds[bounds.len() - 1],
        max_minutes: None,
        count: 0,
    });
    for age in result.ages {
        let index = bounds.partition_point(|b| age >= b * 60000);
        buckets[index].count += 1;
    }
    let stale_items = result
        .stale_items
        .into_iter()
        .map(|i| StaleItem {
            ficha_id: i.ficha_id,
            conversation_id: i.conversation_id,
            channel: i.channel,
            peer: i.peer,
            client_name: i.client_name,
            current_step_key: i.current_step_key,
            age_ms: i.age_ms,
            idle_ms: i.idle_ms,
        })
        .collect();
    Ok(Json(SlaReportJson {
        age_buckets: buckets,
        stale_items,
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
