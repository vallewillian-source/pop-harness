use super::{ListJson, if_match_version};
use crate::{
    auth::Identity,
    error::{ApiError, id_from_url as parse_path_id},
    estado::Estado,
};
use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
};
use poph_store::Instructions;
use poph_types::{AgentId, OrgId, StructuredInstructions};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

#[derive(Deserialize)]
pub struct PathParams {
    org_id: String,
    agent_id: String,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Dados {
    structured: Option<Value>,
    markdown: Option<String>,
}

fn ids(c: PathParams, ident: &Identity) -> Result<(OrgId, AgentId), ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    Ok((org, parse_path_id::<AgentId>(&c.agent_id)?))
}
fn instructions_json(i: &Instructions) -> Value {
    json!({"agent_id":i.agent_id.to_string(),"org_id":i.org_id.to_string(),
        "structured":serde_json::from_str::<Value>(&i.structured_json).unwrap_or(json!(i.structured_json)),
        "markdown":i.markdown,"derived_from_json":i.derived_from_json,"version":i.version,
        "created_at":i.created_at,"updated_at":i.updated_at})
}
fn existing(e: &Estado, org: OrgId, id: AgentId) -> Result<Option<Instructions>, ApiError> {
    match e.store.instructions().get(org, id) {
        Ok(i) => Ok(Some(i)),
        Err(poph_store::StoreError::Domain(poph_types::DomainError::NotFound { .. })) => Ok(None),
        Err(error) => Err(error.into()),
    }
}
#[utoipa::path(operation_id = "listar", get,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions",tag="instructions",params(("org_id"=String,Path),("agent_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Instruções do agente",body=ListJson<Value>)))]
pub async fn list(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<PathParams>,
) -> Result<Json<ListJson<Value>>, ApiError> {
    let (org, id) = ids(c, &ident)?;
    e.store.agents().get(org, id)?;
    let items = existing(&e, org, id)?
        .iter()
        .map(instructions_json)
        .collect();
    Ok(Json(ListJson {
        items,
        next_cursor: None,
    }))
}
#[utoipa::path(operation_id = "mostrar", get,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",tag="instructions",params(("org_id"=String,Path),("agent_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Instruções",body=Value)))]
pub async fn show(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<PathParams>,
) -> Result<Json<Value>, ApiError> {
    let (org, id) = ids(c, &ident)?;
    Ok(Json(instructions_json(
        &e.store.instructions().get(org, id)?,
    )))
}
fn prepare(
    e: &Estado,
    org: OrgId,
    id: AgentId,
    d: Dados,
) -> Result<(String, String, bool), ApiError> {
    match (d.structured, d.markdown) {
        (Some(value), None) => {
            let data = StructuredInstructions::from_tolerant_json(&value.to_string())
                .map_err(|err| ApiError::validation("structured", err.to_string()))?;
            data.validar()
                .map_err(|err| ApiError::validation("structured", err.to_string()))?;
            Ok((
                data.to_json(),
                poph_agent::compose_instructions(&data),
                true,
            ))
        }
        (None, Some(markdown)) => {
            let structured =
                existing(e, org, id)?.map_or_else(|| "{}".to_owned(), |i| i.structured_json);
            Ok((structured, markdown, false))
        }
        _ => Err(ApiError::invalid(
            "informe structured ou markdown, exclusivamente",
        )),
    }
}
async fn write(
    e: Estado,
    ident: Identity,
    c: PathParams,
    d: Dados,
    version: u64,
    acao: &str,
) -> Result<Json<Value>, ApiError> {
    let (org, id) = ids(c, &ident)?;
    e.store.agents().get(org, id)?;
    let (structured, markdown, derived) = prepare(&e, org, id, d)?;
    let i = e
        .store
        .instructions()
        .save(org, id, version, &structured, &markdown, derived)?;
    super::audit::register(
        &e,
        &ident,
        Some(org),
        acao,
        "agent_instructions",
        id.to_string(),
    )?;
    Ok(Json(instructions_json(&i)))
}
#[utoipa::path(operation_id = "criar", post,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions",tag="instructions",params(("org_id"=String,Path),("agent_id"=String,Path)),security(("bearer"=[])),request_body=Dados,responses((status=201,description="Criadas",body=Value)))]
pub async fn create(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<PathParams>,
    body: Result<Json<Dados>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let Json(d) = body.map_err(|_| ApiError::invalid("corpo JSON de instruções inválido"))?;
    Ok((
        StatusCode::CREATED,
        write(e, ident, c, d, 0, "create").await?,
    ))
}
#[utoipa::path(operation_id = "atualizar", patch,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",tag="instructions",params(("org_id"=String,Path),("agent_id"=String,Path),("If-Match"=String,Header)),security(("bearer"=[])),request_body=Dados,responses((status=200,description="Alteradas",body=Value),(status=412,description="Conflito",body=crate::error::ErrorResponse)))]
pub async fn update(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<PathParams>,
    h: HeaderMap,
    body: Result<Json<Dados>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let version = if_match_version(&h, "instruções")?;
    if version == 0 {
        return Err(ApiError::invalid("PATCH exige versão maior que zero"));
    }
    let Json(d) = body.map_err(|_| ApiError::invalid("corpo JSON de instruções inválido"))?;
    write(e, ident, c, d, version, "update").await
}
#[utoipa::path(operation_id = "excluir", delete,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",tag="instructions",params(("org_id"=String,Path),("agent_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Excluídas",body=Value)))]
pub async fn delete(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<PathParams>,
) -> Result<Json<Value>, ApiError> {
    let (org, id) = ids(c, &ident)?;
    e.store.instructions().delete(org, id)?;
    super::audit::register(
        &e,
        &ident,
        Some(org),
        "delete",
        "agent_instructions",
        id.to_string(),
    )?;
    Ok(Json(json!({"removido":true,"agent_id":id.to_string()})))
}
