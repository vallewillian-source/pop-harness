use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url as parse_path_id},
    estado::Estado,
};
use axum::{Json, extract::Path, extract::State, http::StatusCode};
use poph_types::OrgId;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct PathParams {
    org_id: String,
    kind: String,
}

#[derive(Deserialize)]
pub struct OrgPath {
    org_id: String,
}

#[utoipa::path(get,operation_id="listar_manifests",path="/admin/v1/orgs/{org_id}/manifests",tag="manifests",params(("org_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Plugins declarados na configuração da instância: um item por tipo (`kind`) com nome, versão e situação (`ok`/`indisponivel`); plugin que não responde sai marcado e não derruba a lista",body=Value)))]
pub async fn list(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<OrgPath>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let items = e.manifestos.listar().await;
    Ok(Json(json!({ "items": items })))
}

#[utoipa::path(operation_id = "mostrar", get,path="/admin/v1/orgs/{org_id}/manifests/{kind}",tag="manifests",params(("org_id"=String,Path),("kind"=String,Path)),security(("bearer"=[])),responses((status=200,description="Manifesto do plugin (nome, versão, capacidades, limites e JSON Schema de configuração e segredos)",body=Value),(status=404,description="Tipo sem plugin declarado",body=crate::erro::RespostaErro),(status=503,description="Plugin declarado que não respondeu ao handshake",body=crate::erro::RespostaErro)))]
pub async fn show(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<PathParams>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let kind = c.kind.trim();
    if kind.is_empty() {
        return Err(ApiError::validacao("kind", "obrigatório"));
    }
    match e.manifestos.manifesto(kind).await {
        Ok(manifesto) => Ok(Json(
            serde_json::to_value(&manifesto).unwrap_or_else(|_| json!({ "kind": kind })),
        )),
        Err(failure @ crate::manifestos::FalhaManifesto::NaoDeclarado) => Err(ApiError::novo(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("manifesto `{kind}` não encontrado: {}", failure.mensagem()),
        )
        .com_detalhes(json!({ "kind": kind }))),
        Err(failure @ crate::manifestos::FalhaManifesto::Indisponivel { .. }) => {
            Err(ApiError::novo(
                StatusCode::SERVICE_UNAVAILABLE,
                "manifesto_indisponivel",
                failure.mensagem(),
            )
            .com_detalhes(json!({ "kind": kind })))
        }
    }
}
