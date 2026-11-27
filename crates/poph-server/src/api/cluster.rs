use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::auth::Identity;
use crate::cluster;
use crate::error::ApiError;
use crate::estado::Estado;

#[utoipa::path(
    post,
    operation_id = "recarregar_cluster",
    path = "/admin/v1/cluster/reload",
    tag = "cluster",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Mapa recarregado: impressão digital antes e depois, resumo dos dois mapas e o que mudou (orgs que trocaram de instância, instâncias adicionadas e removidas)", body = Value),
        (status = 400, description = "cluster.toml inválido: o mapa anterior continua em uso (detalhe em `details.problemas`)", body = crate::error::ErrorResponse),
        (status = 403, description = "token de org (a recarga exige token global)", body = crate::error::ErrorResponse)
    )
)]
pub async fn reload(State(state): State<Estado>, ident: Identity) -> Result<Json<Value>, ApiError> {
    ident.require_global()?;
    match cluster::recarregar(&state) {
        Ok(report) => Ok(Json(
            serde_json::to_value(report).unwrap_or_else(|_| json!({})),
        )),
        Err(error) => {
            let problem = error.problemas();
            let first = problem
                .first()
                .cloned()
                .unwrap_or_else(|| error.to_string());
            Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "cluster_invalido",
                format!("cluster.toml inválido: {first}"),
            )
            .with_details(json!({
                "arquivo": state.cluster().caminho.display().to_string(),
                "problemas": problem,
            })))
        }
    }
}
