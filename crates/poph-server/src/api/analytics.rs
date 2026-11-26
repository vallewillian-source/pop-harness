use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

use poph_types::OrgId;

use crate::auth::Identidade;
use crate::erro::{ApiError, id_da_url};
use crate::estado::Estado;

#[derive(Deserialize)]
pub struct OrgPath {
    org_id: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct SummaryQuery {
    from: Option<String>,
    to: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct NodeCountJson {
    pub node_id: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OperationalSummaryJson {
    pub open_fichas: u64,
    pub closed_fichas: u64,
    pub open_conversations: u64,
    pub avg_resolution_ms: Option<f64>,
    pub active_flow_nodes: Vec<NodeCountJson>,
}

#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/analytics/operational-summary",
    tag = "analytics",
    operation_id = "analytics_resumo_operacional",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("from" = i64, Query, description = "Começo da janela, em milissegundos desde 1970"),
        ("to" = i64, Query, description = "Fim da janela (exclusivo), em milissegundos desde 1970"),
    ),
    responses(
        (status = 200, description = "Contadores e execuções ativas por nó", body = OperationalSummaryJson),
        (status = 422, description = "`from` ou `to` ausente ou inválido, ou `from` não anterior a `to`", body = crate::erro::RespostaErro),
        (status = 404, description = "Org inexistente", body = crate::erro::RespostaErro),
    )
)]
pub async fn operational_summary(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<OrgPath>,
    Query(query): Query<SummaryQuery>,
) -> Result<Json<OperationalSummaryJson>, ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    identidade.exigir_org(org)?;
    let from = parse_milliseconds("from", query.from.as_deref())?;
    let to = parse_milliseconds("to", query.to.as_deref())?;
    if from >= to {
        return Err(invalid_field("from", "from must be before to."));
    }
    estado.store.orgaos().obter(org)?;
    let summary = estado
        .store
        .analytics()
        .operational_summary(org, from, to)?;
    Ok(Json(OperationalSummaryJson {
        open_fichas: summary.open_fichas,
        closed_fichas: summary.closed_fichas,
        open_conversations: summary.open_conversations,
        avg_resolution_ms: summary.avg_resolution_ms,
        active_flow_nodes: summary
            .active_flow_nodes
            .into_iter()
            .map(|node| NodeCountJson {
                node_id: node.node_id,
                count: node.count,
            })
            .collect(),
    }))
}

fn parse_milliseconds(field: &'static str, value: Option<&str>) -> Result<i64, ApiError> {
    let text = value.ok_or_else(|| invalid_field(field, "Field required"))?;
    text.trim()
        .parse::<i64>()
        .map_err(|_| invalid_field(field, "Input should be a valid integer"))
}

fn invalid_field(field: &'static str, reason: &str) -> ApiError {
    ApiError::novo(
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_error",
        format!("campo `{field}` inválido"),
    )
    .com_detalhes(json!({ "field": field, "reason": reason }))
}
