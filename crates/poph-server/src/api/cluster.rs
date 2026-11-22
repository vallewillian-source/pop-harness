//! Recarga do mapa de cluster (HR-50a): `POST /admin/v1/cluster/reload`.
//!
//! Relê o `cluster.toml` **no processo vivo** e troca o mapa em uso. Só token global: o mapa vale
//! para todas as orgs da instância, então token de org recebe `403` como nas outras ações globais.
//!
//! **Arquivo inválido mantém o mapa antigo:** a leitura e a validação acontecem antes da troca; o
//! operador não derruba o roteamento por um erro de digitação. Arquivo ausente volta para o modo
//! instância única (é o que a recarga reflete). A resposta traz a impressão digital antes e depois
//! e o que mudou — é o que a HR-50b usa para comparar instâncias.

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
pub async fn recarregar(
    State(estado): State<Estado>,
    ident: Identidade,
) -> Result<Json<Value>, ApiError> {
    ident.exigir_global()?;
    match cluster::recarregar(&estado) {
        Ok(relato) => Ok(Json(
            serde_json::to_value(relato).unwrap_or_else(|_| json!({})),
        )),
        Err(erro) => {
            let problemas = erro.problemas();
            let primeiro = problemas
                .first()
                .cloned()
                .unwrap_or_else(|| erro.to_string());
            Err(ApiError::novo(
                StatusCode::BAD_REQUEST,
                "cluster_invalido",
                format!("cluster.toml inválido: {primeiro}"),
            )
            .com_detalhes(json!({
                "arquivo": estado.cluster().caminho.display().to_string(),
                "problemas": problemas,
            })))
        }
    }
}
