//! Instruções: um registro por agente.
use super::{ListaJson, versao_do_if_match};
use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url},
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
pub struct Caminho {
    org_id: String,
    agent_id: String,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Dados {
    structured: Option<Value>,
    markdown: Option<String>,
}

fn ids(c: Caminho, ident: &Identidade) -> Result<(OrgId, AgentId), ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    Ok((org, id_da_url::<AgentId>(&c.agent_id)?))
}
fn representar(i: &Instructions) -> Value {
    json!({"agent_id":i.agent_id.to_string(),"org_id":i.org_id.to_string(),
        "structured":serde_json::from_str::<Value>(&i.structured_json).unwrap_or(json!(i.structured_json)),
        "markdown":i.markdown,"derived_from_json":i.derived_from_json,"version":i.version,
        "created_at":i.created_at,"updated_at":i.updated_at})
}
fn existente(e: &Estado, org: OrgId, id: AgentId) -> Result<Option<Instructions>, ApiError> {
    match e.store.instructions().get(org, id) {
        Ok(i) => Ok(Some(i)),
        Err(poph_store::StoreError::Domain(poph_types::DomainError::NotFound { .. })) => Ok(None),
        Err(erro) => Err(erro.into()),
    }
}
#[utoipa::path(get,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions",tag="instructions",params(("org_id"=String,Path),("agent_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Instruções do agente",body=ListaJson<Value>)))]
pub async fn listar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<Caminho>,
) -> Result<Json<ListaJson<Value>>, ApiError> {
    let (org, id) = ids(c, &ident)?;
    e.store.agents().get(org, id)?;
    let items = existente(&e, org, id)?.iter().map(representar).collect();
    Ok(Json(ListaJson {
        items,
        next_cursor: None,
    }))
}
#[utoipa::path(get,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",tag="instructions",params(("org_id"=String,Path),("agent_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Instruções",body=Value)))]
pub async fn mostrar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<Caminho>,
) -> Result<Json<Value>, ApiError> {
    let (org, id) = ids(c, &ident)?;
    Ok(Json(representar(&e.store.instructions().get(org, id)?)))
}
fn preparar(
    e: &Estado,
    org: OrgId,
    id: AgentId,
    d: Dados,
) -> Result<(String, String, bool), ApiError> {
    match (d.structured, d.markdown) {
        (Some(valor), None) => {
            let dados = StructuredInstructions::from_tolerant_json(&valor.to_string())
                .map_err(|err| ApiError::validacao("structured", err.to_string()))?;
            dados
                .validar()
                .map_err(|err| ApiError::validacao("structured", err.to_string()))?;
            Ok((dados.to_json(), poph_agent::compor_instrucoes(&dados), true))
        }
        (None, Some(markdown)) => {
            let estruturado =
                existente(e, org, id)?.map_or_else(|| "{}".to_owned(), |i| i.structured_json);
            Ok((estruturado, markdown, false))
        }
        _ => Err(ApiError::invalido(
            "informe structured ou markdown, exclusivamente",
        )),
    }
}
async fn gravar(
    e: Estado,
    ident: Identidade,
    c: Caminho,
    d: Dados,
    versao: u64,
    acao: &str,
) -> Result<Json<Value>, ApiError> {
    let (org, id) = ids(c, &ident)?;
    e.store.agents().get(org, id)?;
    let (estruturado, markdown, derivado) = preparar(&e, org, id, d)?;
    let i = e
        .store
        .instructions()
        .save(org, id, versao, &estruturado, &markdown, derivado)?;
    super::auditoria::registrar(
        &e,
        &ident,
        Some(org),
        acao,
        "agent_instructions",
        id.to_string(),
    )?;
    Ok(Json(representar(&i)))
}
#[utoipa::path(post,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions",tag="instructions",params(("org_id"=String,Path),("agent_id"=String,Path)),security(("bearer"=[])),request_body=Dados,responses((status=201,description="Criadas",body=Value)))]
pub async fn criar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<Caminho>,
    corpo: Result<Json<Dados>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let Json(d) = corpo.map_err(|_| ApiError::invalido("corpo JSON de instruções inválido"))?;
    Ok((
        StatusCode::CREATED,
        gravar(e, ident, c, d, 0, "create").await?,
    ))
}
#[utoipa::path(patch,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",tag="instructions",params(("org_id"=String,Path),("agent_id"=String,Path),("If-Match"=String,Header)),security(("bearer"=[])),request_body=Dados,responses((status=200,description="Alteradas",body=Value),(status=412,description="Conflito",body=crate::erro::RespostaErro)))]
pub async fn atualizar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<Caminho>,
    h: HeaderMap,
    corpo: Result<Json<Dados>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let versao = versao_do_if_match(&h, "instruções")?;
    if versao == 0 {
        return Err(ApiError::invalido("PATCH exige versão maior que zero"));
    }
    let Json(d) = corpo.map_err(|_| ApiError::invalido("corpo JSON de instruções inválido"))?;
    gravar(e, ident, c, d, versao, "update").await
}
#[utoipa::path(delete,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",tag="instructions",params(("org_id"=String,Path),("agent_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Excluídas",body=Value)))]
pub async fn excluir(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<Caminho>,
) -> Result<Json<Value>, ApiError> {
    let (org, id) = ids(c, &ident)?;
    e.store.instructions().delete(org, id)?;
    super::auditoria::registrar(
        &e,
        &ident,
        Some(org),
        "delete",
        "agent_instructions",
        id.to_string(),
    )?;
    Ok(Json(json!({"removido":true,"agent_id":id.to_string()})))
}
