use super::{ListJson, if_match_version};
use crate::{
    auth::Identity,
    error::{ApiError, id_from_url as parse_path_id},
    state::ServerState,
};
use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::HeaderMap,
};
use poph_store::Fluxo;
use poph_types::{AgentId, FlowId, OrgId};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

#[derive(Deserialize)]
pub struct AgentPath {
    org_id: String,
    agent_id: String,
}

#[doc = " Corpo do `PUT`: a lista inteira de fluxos do agente."]
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = DadosFluxosAgente)]
pub struct FlowAgentData {
    #[doc = " Ids dos fluxos que o agente pode usar. O que não estiver na lista sai; máximo 20."]
    flow_ids: Vec<String>,
}

fn flow_json(f: &Fluxo) -> Value {
    json!({
        "id": f.id.to_string(),
        "slug": f.slug,
        "name": f.name,
        "status": f.status.as_text(),
    })
}

#[utoipa::path(operation_id = "listar", get,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/flows",tag="agents",params(("org_id"=String,Path),("agent_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Fluxos liberados ao agente, em ordem de slug",body=ListJson<Value>),(status=404,description="Agente não encontrado nesta org",body=crate::error::ErrorResponse)))]
pub async fn list(
    State(e): State<ServerState>,
    ident: Identity,
    Path(c): Path<AgentPath>,
) -> Result<Json<ListJson<Value>>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let agent_id = parse_path_id::<AgentId>(&c.agent_id)?;
    e.store.agents().get(org, agent_id)?;
    let items: Vec<Value> = e
        .store
        .flows()
        .list_from_agent(org, agent_id)?
        .iter()
        .map(flow_json)
        .collect();
    Ok(Json(ListJson {
        items,
        next_cursor: None,
    }))
}

#[utoipa::path(operation_id = "substituir", put,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/flows",tag="agents",params(("org_id"=String,Path),("agent_id"=String,Path),("If-Match"=String,Header)),security(("bearer"=[])),request_body=FlowAgentData,responses((status=200,description="Lista de fluxos do agente substituída: o que entrou, o que saiu e a versão nova do agente",body=Value),(status=404,description="Agente ou fluxo não encontrado nesta org",body=crate::error::ErrorResponse),(status=409,description="Mais de 20 fluxos por agente (teto_de_fluxos_por_agente)",body=crate::error::ErrorResponse),(status=412,description="If-Match diferente da versão atual do agente",body=crate::error::ErrorResponse),(status=428,description="Falta o If-Match",body=crate::error::ErrorResponse)))]
pub async fn replace(
    State(e): State<ServerState>,
    ident: Identity,
    Path(c): Path<AgentPath>,
    h: HeaderMap,
    body: Result<Json<FlowAgentData>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let agent_id = parse_path_id::<AgentId>(&c.agent_id)?;
    let version = if_match_version(&h, "agente")?;
    let Json(d) = body.map_err(|_| {
        ApiError::invalid("corpo JSON inválido: use {\"flow_ids\": [ids de fluxo]}")
    })?;
    let ids = d
        .flow_ids
        .iter()
        .map(|text| parse_path_id::<FlowId>(text))
        .collect::<Result<Vec<FlowId>, _>>()?;
    e.store.agents().get(org, agent_id)?;
    let before = e.store.flows().list_from_agent(org, agent_id)?;
    e.store.flows().replace_list(org, agent_id, version, &ids)?;
    let after = e.store.flows().list_from_agent(org, agent_id)?;
    let of_before: Vec<String> = before.iter().map(|f| f.id.to_string()).collect();
    let of_after: Vec<String> = after.iter().map(|f| f.id.to_string()).collect();
    let add: Vec<Value> = after
        .iter()
        .filter(|f| !of_before.contains(&f.id.to_string()))
        .map(flow_json)
        .collect();
    let withdraw: Vec<Value> = before
        .iter()
        .filter(|f| !of_after.contains(&f.id.to_string()))
        .map(flow_json)
        .collect();
    let new_version = e.store.agents().get(org, agent_id)?.version;
    super::audit::register(
        &e,
        &ident,
        Some(org),
        "agent_flows_update",
        "agent",
        agent_id.to_string(),
    )?;
    Ok(Json(json!({
        "agent_id": agent_id.to_string(),
        "version": new_version,
        "added": add,
        "removed": withdraw,
    })))
}
