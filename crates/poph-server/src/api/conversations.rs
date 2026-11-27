use super::orgs::OrgPath;
use super::{ListJson, PageQuery, next_cursor};
use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url as parse_path_id},
    estado::Estado,
};
use axum::{
    Json,
    extract::{Path, Query, State, rejection::JsonRejection},
};
use poph_store::{ConversationStatus, FilterConversations, OrderConversations};
use poph_types::{AgentId, ConversationId, DepartmentId, OrgId};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use utoipa::ToSchema;

#[derive(Deserialize)]
pub struct ConversationPath {
    org_id: String,
    conversation_id: String,
}

#[derive(Default)]
struct Complements {
    agents: HashMap<AgentId, String>,
    room: HashMap<ConversationId, String>,
}

fn complements_of(
    e: &Estado,
    org: OrgId,
    conversations: &[poph_store::Conversa],
) -> Result<Complements, ApiError> {
    let mut agent_ids: Vec<AgentId> = conversations.iter().map(|c| c.agent_id).collect();
    agent_ids.sort();
    agent_ids.dedup();
    let agents = e.store.agents().names(org, &agent_ids)?;
    let room = if e.store.matrix().state(org)?.linked {
        let ids: Vec<ConversationId> = conversations.iter().map(|c| c.id).collect();
        e.store.matrix().rooms_ready(org, &ids)?
    } else {
        HashMap::new()
    };
    Ok(Complements { agents, room })
}

fn conversation_json(c: &poph_store::Conversa, complements: &Complements) -> Value {
    json!({"id":c.id.to_string(), "org_id":c.org_id.to_string(), "agent_id":c.agent_id.to_string(),
      "integration_id":c.integration_id.map(|x| x.to_string()), "channel":c.canal, "external_peer":c.peer_externo,
      "status":c.status.as_text(), "target_department_id":c.target_department.map(|x| x.to_string()),
      "summary":c.summary, "version":c.version, "created_at":c.created_at, "updated_at":c.updated_at,
      "last_message_at":c.last_message_at,
      "client_name": c.customer_name.as_deref().filter(|name| !name.trim().is_empty()),
      "agent_name": complements.agents.get(&c.agent_id),
      "matrix_room_id": complements.room.get(&c.id)})
}

#[derive(Debug, Default, Deserialize)]
pub struct ConversationsQuery {
    limit: Option<u32>,
    cursor: Option<String>,
    external_peer: Option<String>,
    status: Option<String>,
    sort: Option<String>,
    last_activity_before: Option<String>,
}

fn query_filter(q: &ConversationsQuery) -> Result<FilterConversations, ApiError> {
    let status = match q.status.as_deref() {
        None => None,
        Some("open") => Some(ConversationStatus::Open),
        Some("transferred") => Some(ConversationStatus::Transferred),
        Some("closed") => Some(ConversationStatus::Closed),
        Some(_) => {
            return Err(ApiError::invalido(
                "status inválido: use open, transferred ou closed",
            ));
        }
    };
    let external_peer = match q.external_peer.as_deref() {
        None => None,
        Some(peer) => {
            let clean = peer.trim();
            if clean.is_empty() {
                return Err(ApiError::invalido(
                    "external_peer não pode ser vazio: informe o identificador do cliente",
                ));
            }
            Some(clean.to_owned())
        }
    };
    let last_activity_before = match q.last_activity_before.as_deref() {
        None => None,
        Some(text) => Some(text.trim().parse::<i64>().map_err(|_| {
            ApiError::invalido(
                "last_activity_before inválido: informe o instante em milissegundos desde 1970",
            )
        })?),
    };
    Ok(FilterConversations {
        status,
        external_peer,
        last_activity_before,
    })
}

fn query_order(q: &ConversationsQuery) -> Result<OrderConversations, ApiError> {
    match q.sort.as_deref() {
        None => Ok(OrderConversations::Id),
        Some("recent") => Ok(OrderConversations::Recent),
        Some(_) => Err(ApiError::invalido(
            "sort inválido: use recent ou não envie sort",
        )),
    }
}

#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/conversations", tag="conversations", operation_id="conversas_listar", security(("bearer"=[])), params(("org_id"=String, Path), ("limit"=Option<u32>, Query), ("cursor"=Option<String>, Query), ("external_peer"=Option<String>, Query), ("status"=Option<String>, Query), ("sort"=Option<String>, Query), ("last_activity_before"=Option<i64>, Query)), responses((status=200, description="Conversas", body=ListJson<Value>), (status=400, description="Parâmetro inválido", body=crate::erro::RespostaErro)))]
pub async fn list(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<OrgPath>,
    Query(q): Query<ConversationsQuery>,
) -> Result<Json<ListJson<Value>>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let filter = query_filter(&q)?;
    let order = query_order(&q)?;
    let (conversations, next_page) = match order {
        OrderConversations::Id => {
            let p = PageQuery {
                limit: q.limit,
                cursor: q.cursor.clone(),
            }
            .page()?;
            let conversations = e.store.conversations().list_filter(org, &filter, p)?;
            (conversations, Some(p))
        }
        OrderConversations::Recent => {
            if q.cursor.as_deref().is_some_and(|cursor| !cursor.is_empty()) {
                return Err(ApiError::invalido(
                    "sort=recent não usa cursor: peça só limit",
                ));
            }
            let limit = q.limit.unwrap_or(50);
            if !(1..=200).contains(&limit) {
                return Err(ApiError::invalido("limit deve ficar entre 1 e 200"));
            }
            let conversations = e.store.conversations().list_recents(org, &filter, limit)?;
            (conversations, None)
        }
    };
    let complements = complements_of(&e, org, &conversations)?;
    let items: Vec<_> = conversations
        .iter()
        .map(|c| conversation_json(c, &complements))
        .collect();
    let next_cursor = match next_page {
        Some(p) => next_cursor(&items, p.limit, |v| {
            v["id"].as_str().unwrap_or_default().to_owned()
        }),
        None => None,
    };
    Ok(Json(ListJson { items, next_cursor }))
}
#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/conversations/{conversation_id}", tag="conversations", operation_id="conversas_mostrar", security(("bearer"=[])), params(("org_id"=String, Path), ("conversation_id"=String, Path)), responses((status=200, description="Conversa", body=Value)))]
pub async fn show(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<ConversationPath>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = parse_path_id::<ConversationId>(&c.conversation_id)?;
    let conversation = e.store.conversations().get(org, id)?;
    let complements = complements_of(&e, org, std::slice::from_ref(&conversation))?;
    Ok(Json(conversation_json(&conversation, &complements)))
}
#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/conversations/{conversation_id}/messages", tag="conversations", operation_id="mensagens_listar", security(("bearer"=[])), params(("org_id"=String, Path), ("conversation_id"=String, Path), ("limit"=Option<u32>, Query), ("cursor"=Option<String>, Query)), responses((status=200, description="Mensagens", body=ListJson<Value>)))]
pub async fn messages(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<ConversationPath>,
    Query(q): Query<PageQuery>,
) -> Result<Json<ListJson<Value>>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = parse_path_id::<ConversationId>(&c.conversation_id)?;
    e.store.conversations().get(org, id)?;
    let p = q.page()?;
    let items:Vec<_>=e.store.messages().list(org,id,p)?.iter().map(|m|json!({"id":m.id.to_string(),"org_id":m.org_id.to_string(),"conversation_id":m.conversation_id.to_string(),"direction":m.direction.as_text(),"role":m.role.as_text(),"text":m.texto,"attachments":serde_json::from_str::<Value>(&m.attachments_json).unwrap_or(Value::Null),"turn_id":m.turn_id.map(|v|v.to_string()),"created_at":m.created_at})).collect();
    let next_cursor = next_cursor(&items, p.limit, |v| {
        v["id"].as_str().unwrap_or_default().to_owned()
    });
    Ok(Json(ListJson { items, next_cursor }))
}
#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/conversations/{conversation_id}/turns", tag="conversations", operation_id="turnos_listar", security(("bearer"=[])), params(("org_id"=String, Path), ("conversation_id"=String, Path), ("limit"=Option<u32>, Query), ("cursor"=Option<String>, Query)), responses((status=200, description="Turnos", body=ListJson<Value>)))]
pub async fn turns(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<ConversationPath>,
    Query(q): Query<PageQuery>,
) -> Result<Json<ListJson<Value>>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = parse_path_id::<ConversationId>(&c.conversation_id)?;
    e.store.conversations().get(org, id)?;
    let p = q.page()?;
    let items: Vec<_> = e
        .store
        .turns()
        .list(org, id, p)?
        .iter()
        .map(|t| {
            let (names, call) = super::tool_parts(&t.tools_json);
            json!({
                "id": t.id.to_string(),
                "org_id": t.org_id.to_string(),
                "conversation_id": t.conversation_id.to_string(),
                "state": t.state.as_text(),
                "retrieved": serde_json::from_str::<Value>(&t.passages_json).unwrap_or(Value::Null),
                "tools": names,
                "tool_calls": call,
                "timings": serde_json::from_str::<Value>(&t.times_json).unwrap_or(Value::Null),
                "input_tokens": t.input_tokens,
                "output_tokens": t.output_tokens,
                "cost_micros": t.cost_micros,
                "error": t.error,
                "started_at": t.started_at,
                "finished_at": t.finished_at,
            })
        })
        .collect();
    let next_cursor = next_cursor(&items, p.limit, |v| {
        v["id"].as_str().unwrap_or_default().to_owned()
    });
    Ok(Json(ListJson { items, next_cursor }))
}

#[utoipa::path(post, path="/admin/v1/orgs/{org_id}/conversations/{conversation_id}/reopen", tag="conversations", operation_id="conversas_reabrir", security(("bearer"=[])), params(("org_id"=String, Path), ("conversation_id"=String, Path), ("If-Match"=String, Header)), responses((status=200, description="Conversa reaberta", body=Value), (status=409, description="Conversa encerrada (`conversa_encerrada`): não reabre; nada muda", body=crate::erro::RespostaErro), (status=412, description="Versão desatualizada", body=crate::erro::RespostaErro)))]
pub async fn reopen(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<ConversationPath>,
    headers: axum::http::HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = parse_path_id::<ConversationId>(&c.conversation_id)?;
    let version = super::if_match_version(&headers, "conversa")?;
    let conversation = e
        .store
        .conversations()
        .reopen_audited(org, id, version, ident.ator_id())?;
    let complements = complements_of(&e, org, std::slice::from_ref(&conversation))?;
    Ok(Json(conversation_json(&conversation, &complements)))
}

#[utoipa::path(post, path="/admin/v1/orgs/{org_id}/conversations/{conversation_id}/close", tag="conversations", operation_id="conversas_fechar", security(("bearer"=[])), params(("org_id"=String, Path), ("conversation_id"=String, Path), ("If-Match"=String, Header)), responses((status=200, description="Conversa fechada", body=Value), (status=404, description="Conversa não encontrada na org", body=crate::erro::RespostaErro), (status=409, description="Conversa já fechada", body=crate::erro::RespostaErro), (status=412, description="Versão desatualizada", body=crate::erro::RespostaErro), (status=428, description="Faltou If-Match", body=crate::erro::RespostaErro)))]
pub async fn close(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<ConversationPath>,
    headers: axum::http::HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = parse_path_id::<ConversationId>(&c.conversation_id)?;
    let version = super::if_match_version(&headers, "conversa")?;
    let conversation = e
        .store
        .conversations()
        .close_audited(org, id, version, ident.ator_id())?;
    let complements = complements_of(&e, org, std::slice::from_ref(&conversation))?;
    Ok(Json(conversation_json(&conversation, &complements)))
}

#[derive(Debug, Deserialize, ToSchema)]
#[schema(as = CorpoTransferencia)]
pub struct BodyTransfer {
    department_id: Option<String>,
    reason: Option<String>,
}

#[utoipa::path(post, path="/admin/v1/orgs/{org_id}/conversations/{conversation_id}/transfer", tag="conversations", operation_id="conversas_transferir", security(("bearer"=[])), params(("org_id"=String, Path), ("conversation_id"=String, Path)), request_body = BodyTransfer, responses((status=200, description="Conversa transferida para o departamento", body=Value), (status=400, description="Corpo inválido, motivo vazio ou departamento malformado", body=crate::erro::RespostaErro), (status=404, description="Conversa ou departamento fora da org", body=crate::erro::RespostaErro), (status=409, description="Conversa encerrada (`conversa_encerrada`): não transfere; nada muda", body=crate::erro::RespostaErro)))]
pub async fn transfer(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<ConversationPath>,
    body: Result<Json<BodyTransfer>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = parse_path_id::<ConversationId>(&c.conversation_id)?;
    let Json(data) = body.map_err(|_| {
        ApiError::invalido("corpo JSON inválido: use {\"department_id\": ..., \"reason\": ...}")
    })?;
    let reason = data
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .ok_or_else(|| ApiError::validacao("reason", "informe o motivo da transferência"))?
        .to_owned();
    let department =
        match data
            .department_id
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            Some(text) => Some(text.parse::<DepartmentId>().map_err(|_| {
                ApiError::validacao("department_id", "informe um id de departamento")
            })?),
            None => None,
        };
    let detail = json!({
        "motivo": reason,
        "ator": "administrador",
        "department_id": department.map(|destination| destination.to_string()),
    })
    .to_string();
    let conversation = e.store.conversations().transfer_audited(
        org,
        id,
        department,
        Some(ident.ator_id()),
        detail,
    )?;
    let complements = complements_of(&e, org, std::slice::from_ref(&conversation))?;
    Ok(Json(conversation_json(&conversation, &complements)))
}
