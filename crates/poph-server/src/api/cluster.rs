use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::auth::Identidade;
use crate::cluster;
use crate::erro::ApiError;
use crate::estado::Estado;

#[utoipa::path(
    post,
    operation_id = "recarregar_cluster",
    path = "/admin/v1/cluster/reload",
    tag = "cluster",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Mapa recarregado: impressão digital antes e depois, resumo dos dois mapas e o que mudou (orgs que trocaram de instância, instâncias adicionadas e removidas)", body = Value),
        (status = 400, description = "cluster.toml inválido: o mapa anterior continua em uso (detalhe em `details.problemas`)", body = crate::erro::RespostaErro),
        (status = 403, description = "token de org (a recarga exige token global)", body = crate::erro::RespostaErro)
    )
)]
pub async fn reload(
    State(state): State<Estado>,
    ident: Identidade,
) -> Result<Json<Value>, ApiError> {
    ident.exigir_global()?;
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
            Err(ApiError::novo(
                StatusCode::BAD_REQUEST,
                "cluster_invalido",
                format!("cluster.toml inválido: {first}"),
            )
            .com_detalhes(json!({
                "arquivo": state.cluster().caminho.display().to_string(),
                "problemas": problem,
            })))
        }
    }
}
