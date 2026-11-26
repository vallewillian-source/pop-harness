//! Permissão de fluxos por agente (FX-28, PRD §6.2): quais fluxos da org cada agente pode usar.
//!
//! O fluxo é da organização; o agente só tem a permissão de usar. A lista é trocada inteira numa
//! escrita só (`substituir_lista`, FX-08), com a versão do agente no `If-Match`.
use super::{ListaJson, versao_do_if_match};
use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url},
    estado::Estado,
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

/// Caminho `/agents/{agent_id}` dentro da org.
#[derive(Deserialize)]
pub struct CaminhoAgente {
    org_id: String,
    agent_id: String,
}

/// Corpo do `PUT`: a lista inteira de fluxos do agente.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DadosFluxosAgente {
    /// Ids dos fluxos que o agente pode usar. O que não estiver na lista sai; máximo 20.
    flow_ids: Vec<String>,
}

fn json_fluxo(f: &Fluxo) -> Value {
    json!({
        "id": f.id.to_string(),
        "slug": f.slug,
        "name": f.nome,
        "status": f.status.como_texto(),
    })
}

#[utoipa::path(get,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/flows",tag="agents",params(("org_id"=String,Path),("agent_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Fluxos liberados ao agente, em ordem de slug",body=ListaJson<Value>),(status=404,description="Agente não encontrado nesta org",body=crate::erro::RespostaErro)))]
pub async fn listar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoAgente>,
) -> Result<Json<ListaJson<Value>>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let agente = id_da_url::<AgentId>(&c.agent_id)?;
    e.store.agentes().obter(org, agente)?;
    let items: Vec<Value> = e
        .store
        .fluxos()
        .listar_do_agente(org, agente)?
        .iter()
        .map(json_fluxo)
        .collect();
    Ok(Json(ListaJson {
        items,
        next_cursor: None,
    }))
}

#[utoipa::path(put,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/flows",tag="agents",params(("org_id"=String,Path),("agent_id"=String,Path),("If-Match"=String,Header)),security(("bearer"=[])),request_body=DadosFluxosAgente,responses((status=200,description="Lista de fluxos do agente substituída: o que entrou, o que saiu e a versão nova do agente",body=Value),(status=404,description="Agente ou fluxo não encontrado nesta org",body=crate::erro::RespostaErro),(status=409,description="Mais de 20 fluxos por agente (teto_de_fluxos_por_agente)",body=crate::erro::RespostaErro),(status=412,description="If-Match diferente da versão atual do agente",body=crate::erro::RespostaErro),(status=428,description="Falta o If-Match",body=crate::erro::RespostaErro)))]
pub async fn substituir(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoAgente>,
    h: HeaderMap,
    corpo: Result<Json<DadosFluxosAgente>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let agente = id_da_url::<AgentId>(&c.agent_id)?;
    let versao = versao_do_if_match(&h, "agente")?;
    let Json(d) = corpo.map_err(|_| {
        ApiError::invalido("corpo JSON inválido: use {\"flow_ids\": [ids de fluxo]}")
    })?;
    let ids = d
        .flow_ids
        .iter()
        .map(|texto| id_da_url::<FlowId>(texto))
        .collect::<Result<Vec<FlowId>, _>>()?;
    // Agente de outra org ou inexistente: 404 antes de mexer em qualquer permissão.
    e.store.agentes().obter(org, agente)?;
    let antes = e.store.fluxos().listar_do_agente(org, agente)?;
    e.store
        .fluxos()
        .substituir_lista(org, agente, versao, &ids)?;
    let depois = e.store.fluxos().listar_do_agente(org, agente)?;
    let do_antes: Vec<String> = antes.iter().map(|f| f.id.to_string()).collect();
    let do_depois: Vec<String> = depois.iter().map(|f| f.id.to_string()).collect();
    let adicionados: Vec<Value> = depois
        .iter()
        .filter(|f| !do_antes.contains(&f.id.to_string()))
        .map(json_fluxo)
        .collect();
    let retirados: Vec<Value> = antes
        .iter()
        .filter(|f| !do_depois.contains(&f.id.to_string()))
        .map(json_fluxo)
        .collect();
    let nova_versao = e.store.agentes().obter(org, agente)?.versao;
    super::auditoria::registrar(
        &e,
        &ident,
        Some(org),
        "agent_flows_update",
        "agent",
        agente.to_string(),
    )?;
    Ok(Json(json!({
        "agent_id": agente.to_string(),
        "version": nova_versao,
        "added": adicionados,
        "removed": retirados,
    })))
}
