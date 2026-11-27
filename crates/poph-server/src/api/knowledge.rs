use super::{ListJson, PageQuery, if_match_version, next_cursor, request_status};
use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url as parse_path_id},
    estado::Estado,
};
use axum::{
    Json,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode},
};
use poph_store::{KnowledgeGroup, KnowledgeGroupChange, NewKnowledgeGroup};
use poph_types::{KnowledgeGroupId, OrgId};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

#[derive(Deserialize)]
pub struct PathParams {
    org_id: String,
    group_id: Option<String>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Dados {
    name: Option<String>,
    description: Option<String>,
    status: Option<String>,
}
fn ids(c: PathParams, ident: &Identidade) -> Result<(OrgId, Option<KnowledgeGroupId>), ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    Ok((
        org,
        c.group_id
            .as_deref()
            .map(|s| {
                s.parse::<KnowledgeGroupId>()
                    .map_err(|_| ApiError::invalido("id de grupo inválido"))
            })
            .transpose()?,
    ))
}
fn group(g: &KnowledgeGroup) -> Value {
    json!({"id":g.id.to_string(),"org_id":g.org_id.to_string(),"name":g.name,"description":g.description,"status":g.status.as_text(),"version":g.version,"created_at":g.created_at,"updated_at":g.updated_at})
}
#[utoipa::path(operation_id = "listar", get,path="/admin/v1/orgs/{org_id}/knowledge",tag="knowledge",params(("org_id"=String,Path),("limit"=Option<u32>,Query),("cursor"=Option<String>,Query)),security(("bearer"=[])),responses((status=200,description="Grupos",body=ListJson<Value>)))]
pub async fn list(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<PathParams>,
    q: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Json<ListJson<Value>>, ApiError> {
    let (org, _) = ids(c, &ident)?;
    let Query(q) = q.map_err(|_| ApiError::invalido("parâmetros de paginação inválidos"))?;
    let p = q.page()?;
    let items: Vec<_> = e
        .store
        .knowledge()
        .list_groups(org, p)?
        .iter()
        .map(group)
        .collect();
    let next_cursor = next_cursor(&items, p.limit, |v| {
        v["id"].as_str().unwrap_or_default().to_owned()
    });
    Ok(Json(ListJson { items, next_cursor }))
}
#[utoipa::path(operation_id = "mostrar", get,path="/admin/v1/orgs/{org_id}/knowledge/{group_id}",tag="knowledge",params(("org_id"=String,Path),("group_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Grupo",body=Value)))]
pub async fn show(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<PathParams>,
) -> Result<Json<Value>, ApiError> {
    let (org, id) = ids(c, &ident)?;
    let id = id.ok_or_else(|| ApiError::invalido("grupo ausente"))?;
    let mut value = group(&e.store.knowledge().get_group(org, id)?);
    let agents = e.store.knowledge().agents_from_group(org, id)?;
    value["agents"] = json!(agents.iter().map(ToString::to_string).collect::<Vec<_>>());
    Ok(Json(value))
}
#[utoipa::path(operation_id = "criar", post,path="/admin/v1/orgs/{org_id}/knowledge",tag="knowledge",params(("org_id"=String,Path)),security(("bearer"=[])),request_body=Dados,responses((status=201,description="Criado",body=Value)))]
pub async fn create(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<PathParams>,
    body: Result<Json<Dados>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let (org, _) = ids(c, &ident)?;
    let Json(d) = body.map_err(|_| ApiError::invalido("corpo JSON inválido"))?;
    if d.status.is_some() {
        return Err(ApiError::invalido("status não é aceito na criação"));
    }
    let g = e.store.knowledge().create_group(
        org,
        NewKnowledgeGroup {
            name: d
                .name
                .ok_or_else(|| ApiError::invalido("name é obrigatório"))?,
            description: d.description.unwrap_or_default(),
        },
    )?;
    super::audit::register(
        &e,
        &ident,
        Some(org),
        "create",
        "knowledge_group",
        g.id.to_string(),
    )?;
    Ok((StatusCode::CREATED, Json(group(&g))))
}
#[utoipa::path(operation_id = "atualizar", patch,path="/admin/v1/orgs/{org_id}/knowledge/{group_id}",tag="knowledge",params(("org_id"=String,Path),("group_id"=String,Path),("If-Match"=String,Header)),security(("bearer"=[])),request_body=Dados,responses((status=200,description="Alterado",body=Value),(status=412,description="Conflito",body=crate::erro::RespostaErro)))]
pub async fn update(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<PathParams>,
    h: HeaderMap,
    body: Result<Json<Dados>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let (org, id) = ids(c, &ident)?;
    let id = id.ok_or_else(|| ApiError::invalido("grupo ausente"))?;
    let version = if_match_version(&h, "grupo")?;
    let Json(d) = body.map_err(|_| ApiError::invalido("corpo JSON inválido"))?;
    let g = e.store.knowledge().update_group(
        org,
        id,
        version,
        KnowledgeGroupChange {
            name: d.name,
            description: d.description,
            status: d.status.as_deref().map(request_status).transpose()?,
        },
    )?;
    super::audit::register(
        &e,
        &ident,
        Some(org),
        "update",
        "knowledge_group",
        id.to_string(),
    )?;
    let notice = group_notice(&e, org, id).await;
    let mut value = group(&g);
    value["index_warning"] = json!(notice);
    Ok(Json(value))
}
#[utoipa::path(operation_id = "excluir", delete,path="/admin/v1/orgs/{org_id}/knowledge/{group_id}",tag="knowledge",params(("org_id"=String,Path),("group_id"=String,Path),("force"=Option<bool>,Query)),security(("bearer"=[])),responses((status=200,description="Excluído",body=Value)))]
pub async fn delete(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<PathParams>,
    q: Result<Query<ConfirmExclusion>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let (org, id) = ids(c, &ident)?;
    let id = id.ok_or_else(|| ApiError::invalido("grupo ausente"))?;
    let Query(q) = q.map_err(|_| ApiError::invalido("parâmetro force inválido"))?;
    if !q.force.unwrap_or(false) {
        return Err(ApiError::conflito(
            "grupo pode conter itens e vínculos; confirme com force=true",
        ));
    }
    let agents = e.store.knowledge().agents_from_group(org, id)?;
    e.store.knowledge().delete_group(org, id)?;
    super::audit::register(
        &e,
        &ident,
        Some(org),
        "delete",
        "knowledge_group",
        id.to_string(),
    )?;
    let notice = sync_notice(&e, org, agents).await;
    Ok(Json(
        json!({"removido":true,"id":id.to_string(),"index_warning":notice}),
    ))
}

#[derive(Deserialize)]
pub struct ConfirmExclusion {
    force: Option<bool>,
}

#[derive(Deserialize)]
pub struct ItemPath {
    org_id: String,
    group_id: String,
    item_id: Option<String>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = DadosItem)]
pub struct ItemData {
    group_id: Option<String>,
    title: Option<String>,
    content: Option<String>,
    tags: Option<Vec<String>>,
    position: Option<i64>,
    status: Option<String>,
}
fn ids_item(
    c: ItemPath,
    ident: &Identidade,
) -> Result<(OrgId, KnowledgeGroupId, Option<poph_types::KnowledgeItemId>), ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let group = c
        .group_id
        .parse()
        .map_err(|_| ApiError::invalido("id de grupo inválido"))?;
    let item = c
        .item_id
        .map(|s| {
            s.parse()
                .map_err(|_| ApiError::invalido("id de item inválido"))
        })
        .transpose()?;
    Ok((org, group, item))
}
fn item_json(i: &poph_store::KnowledgeItem) -> Value {
    json!({"id":i.id.to_string(),"org_id":i.org_id.to_string(),"group_id":i.group_id.to_string(),"title":i.title,"content":i.content,"tags":i.tags,"position":i.position,"status":i.status.as_text(),"version":i.version,"created_at":i.created_at,"updated_at":i.updated_at})
}
fn check_item(
    e: &Estado,
    org: OrgId,
    group: KnowledgeGroupId,
    id: poph_types::KnowledgeItemId,
) -> Result<poph_store::KnowledgeItem, ApiError> {
    let i = e.store.knowledge().get_item(org, id)?;
    if i.group_id != group {
        return Err(ApiError::novo(
            StatusCode::NOT_FOUND,
            "not_found",
            "item não pertence ao grupo",
        ));
    }
    Ok(i)
}
#[utoipa::path(operation_id = "listar_itens", get,path="/admin/v1/orgs/{org_id}/knowledge/{group_id}/items",tag="knowledge",params(("org_id"=String,Path),("group_id"=String,Path),("limit"=Option<u32>,Query),("cursor"=Option<String>,Query)),security(("bearer"=[])),responses((status=200,description="Itens",body=ListJson<Value>)))]
pub async fn list_items(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<ItemPath>,
    q: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Json<ListJson<Value>>, ApiError> {
    let (org, g, _) = ids_item(c, &ident)?;
    e.store.knowledge().get_group(org, g)?;
    let Query(q) = q.map_err(|_| ApiError::invalido("parâmetros de paginação inválidos"))?;
    let p = q.page()?;
    let items: Vec<_> = e
        .store
        .knowledge()
        .list_items_by_id(org, g, p)?
        .iter()
        .map(item_json)
        .collect();
    let next_cursor = next_cursor(&items, p.limit, |v| {
        v["id"].as_str().unwrap_or_default().to_owned()
    });
    Ok(Json(ListJson { items, next_cursor }))
}
#[utoipa::path(operation_id = "mostrar_item", get,path="/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}",tag="knowledge",params(("org_id"=String,Path),("group_id"=String,Path),("item_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Item",body=Value)))]
pub async fn show_item(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<ItemPath>,
) -> Result<Json<Value>, ApiError> {
    let (org, g, id) = ids_item(c, &ident)?;
    Ok(Json(item_json(&check_item(
        &e,
        org,
        g,
        id.ok_or_else(|| ApiError::invalido("item ausente"))?,
    )?)))
}
#[utoipa::path(operation_id = "criar_item", post,path="/admin/v1/orgs/{org_id}/knowledge/{group_id}/items",tag="knowledge",params(("org_id"=String,Path),("group_id"=String,Path)),security(("bearer"=[])),request_body=ItemData,responses((status=201,description="Criado",body=Value)))]
pub async fn create_item(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<ItemPath>,
    body: Result<Json<ItemData>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let (org, g, _) = ids_item(c, &ident)?;
    let Json(d) = body.map_err(|_| ApiError::invalido("corpo JSON inválido"))?;
    if d.status.is_some() || d.group_id.is_some() {
        return Err(ApiError::invalido(
            "status e group_id não são aceitos na criação",
        ));
    }
    let i = e.store.knowledge().create_item(
        org,
        poph_store::NewKnowledgeItem {
            group_id: g,
            title: d
                .title
                .ok_or_else(|| ApiError::invalido("title é obrigatório"))?,
            content: d
                .content
                .ok_or_else(|| ApiError::invalido("content é obrigatório"))?,
            tags: d.tags.unwrap_or_default(),
            position: d.position.unwrap_or_default(),
        },
    )?;
    super::audit::register(
        &e,
        &ident,
        Some(org),
        "create",
        "knowledge_item",
        i.id.to_string(),
    )?;
    let notice = group_notice(&e, org, g).await;
    let mut value = item_json(&i);
    value["index_warning"] = json!(notice);
    Ok((StatusCode::CREATED, Json(value)))
}
#[utoipa::path(operation_id = "atualizar_item", patch,path="/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}",tag="knowledge",params(("org_id"=String,Path),("group_id"=String,Path),("item_id"=String,Path),("If-Match"=String,Header)),security(("bearer"=[])),request_body=ItemData,responses((status=200,description="Alterado",body=Value),(status=412,description="Conflito",body=crate::erro::RespostaErro)))]
pub async fn update_item(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<ItemPath>,
    h: HeaderMap,
    body: Result<Json<ItemData>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let (org, g, id) = ids_item(c, &ident)?;
    let id = id.ok_or_else(|| ApiError::invalido("item ausente"))?;
    check_item(&e, org, g, id)?;
    let version = if_match_version(&h, "item")?;
    let Json(d) = body.map_err(|_| ApiError::invalido("corpo JSON inválido"))?;
    let new_group = d
        .group_id
        .as_deref()
        .map(|v| {
            v.parse::<KnowledgeGroupId>()
                .map_err(|_| ApiError::invalido("id de grupo inválido"))
        })
        .transpose()?;
    let i = e.store.knowledge().update_item(
        org,
        id,
        version,
        poph_store::KnowledgeItemChange {
            group_id: new_group,
            title: d.title,
            content: d.content,
            tags: d.tags,
            position: d.position,
            status: d.status.as_deref().map(request_status).transpose()?,
        },
    )?;
    super::audit::register(
        &e,
        &ident,
        Some(org),
        "update",
        "knowledge_item",
        id.to_string(),
    )?;
    let notice = group_notice(&e, org, g).await;
    let notice_new = match new_group {
        Some(other) if other != g => group_notice(&e, org, other).await,
        _ => None,
    };
    let mut value = item_json(&i);
    value["index_warning"] = json!(notice.or(notice_new));
    Ok(Json(value))
}
#[utoipa::path(operation_id = "excluir_item", delete,path="/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}",tag="knowledge",params(("org_id"=String,Path),("group_id"=String,Path),("item_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Excluído",body=Value)))]
pub async fn delete_item(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<ItemPath>,
) -> Result<Json<Value>, ApiError> {
    let (org, g, id) = ids_item(c, &ident)?;
    let id = id.ok_or_else(|| ApiError::invalido("item ausente"))?;
    check_item(&e, org, g, id)?;
    e.store.knowledge().delete_item(org, id)?;
    super::audit::register(
        &e,
        &ident,
        Some(org),
        "delete",
        "knowledge_item",
        id.to_string(),
    )?;
    let notice = group_notice(&e, org, g).await;
    Ok(Json(
        json!({"removido":true,"id":id.to_string(),"index_warning":notice}),
    ))
}

#[derive(Deserialize)]
pub struct AgentPath {
    org_id: String,
    agent_id: String,
    group_id: Option<String>,
}
fn agent_ids(
    c: AgentPath,
    ident: &Identidade,
) -> Result<(OrgId, poph_types::AgentId, Option<KnowledgeGroupId>), ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let agent_id = parse_path_id::<poph_types::AgentId>(&c.agent_id)?;
    let group = c
        .group_id
        .as_deref()
        .map(|s| {
            s.parse::<KnowledgeGroupId>()
                .map_err(|_| ApiError::invalido("id de grupo inválido"))
        })
        .transpose()?;
    Ok((org, agent_id, group))
}

#[utoipa::path(operation_id = "vincular", post,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/{group_id}",tag="knowledge",params(("org_id"=String,Path),("agent_id"=String,Path),("group_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Vínculo criado",body=Value)))]
pub async fn link(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<AgentPath>,
) -> Result<Json<Value>, ApiError> {
    let (org, agent_id, group) = agent_ids(c, &ident)?;
    let group = group.ok_or_else(|| ApiError::invalido("grupo ausente"))?;
    e.store.knowledge().link_agent(org, agent_id, group)?;
    super::audit::register(
        &e,
        &ident,
        Some(org),
        "link",
        "knowledge_group",
        group.to_string(),
    )?;
    let notice = sync_notice(&e, org, vec![agent_id]).await;
    Ok(Json(
        json!({"agent":agent_id.to_string(),"group":group.to_string(),"linked":true,"index_warning":notice}),
    ))
}
#[utoipa::path(operation_id = "desvincular", delete,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/{group_id}",tag="knowledge",params(("org_id"=String,Path),("agent_id"=String,Path),("group_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Vínculo removido",body=Value)))]
pub async fn unlink(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<AgentPath>,
) -> Result<Json<Value>, ApiError> {
    let (org, agent_id, group) = agent_ids(c, &ident)?;
    let group = group.ok_or_else(|| ApiError::invalido("grupo ausente"))?;
    e.store.agents().get(org, agent_id)?;
    e.store.knowledge().get_group(org, group)?;
    let change = e.store.knowledge().unlink_agent(org, agent_id, group)?;
    super::audit::register(
        &e,
        &ident,
        Some(org),
        "unlink",
        "knowledge_group",
        group.to_string(),
    )?;
    let notice = sync_notice(&e, org, vec![agent_id]).await;
    Ok(Json(
        json!({"agent":agent_id.to_string(),"group":group.to_string(),"linked":false,"changed":change,"index_warning":notice}),
    ))
}
#[utoipa::path(operation_id = "reindexar", post,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/reindex",tag="knowledge",params(("org_id"=String,Path),("agent_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Índice sincronizado",body=Value)))]
pub async fn reindex(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<AgentPath>,
) -> Result<Json<Value>, ApiError> {
    let (org, agent_id, _) = agent_ids(c, &ident)?;
    e.store.agents().get(org, agent_id)?;
    let r = synchronise(&e, org, agent_id).await?;
    super::audit::register(
        &e,
        &ident,
        Some(org),
        "reindex",
        "agent",
        agent_id.to_string(),
    )?;
    Ok(Json(
        json!({"agent":agent_id.to_string(),"indexed":{"itens_novos":r.new_items,"itens_alterados":r.changed_items,"itens_inalterados":r.unchanged_items,"itens_removidos":r.removed_items,"embeddings_gerados":r.generated_embeddings,"trechos_gravados":r.stored_chunks}}),
    ))
}

async fn synchronise(
    e: &Estado,
    org: OrgId,
    agent_id: poph_types::AgentId,
) -> Result<poph_rag::IndexReport, ApiError> {
    use poph_rag::{
        Embedder, EmbeddingQueue, Index, IndexConfig, ModelConfig, OrtEmbedder, QueueConfig,
        QueuedEmbedder,
    };
    use std::sync::Arc;
    static SINCRONIZACAO: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = SINCRONIZACAO.lock().await;
    let config = ModelConfig {
        diretorio: e.config.models(),
        ..ModelConfig::default()
    };
    let model = tokio::task::spawn_blocking(move || OrtEmbedder::open_model(&config, None))
        .await
        .map_err(|_| ApiError::interno())?
        .map_err(|_| {
            ApiError::novo(
                StatusCode::SERVICE_UNAVAILABLE,
                "embedding_unavailable",
                "modelo de embedding indisponível; instale o modelo",
            )
        })?;
    let identity = model.identity().clone();
    let model: Arc<dyn Embedder> = Arc::new(model);
    let queue = Arc::new(
        EmbeddingQueue::start(model, QueueConfig::default(), None)
            .map_err(|_| ApiError::interno())?,
    );
    let adapter = Arc::new(QueuedEmbedder::new(queue, org, identity));
    let (index, _) = Index::open(e.store.rag_path(), adapter, IndexConfig::default())
        .map_err(|_| ApiError::interno())?;
    let base = poph_agent::KnowledgeBase::new(Arc::clone(&e.store), Arc::new(index));
    base.synchronise_agent(org, agent_id)
        .await
        .map_err(|_| ApiError::interno())
}

async fn sync_notice(
    e: &Estado,
    org: OrgId,
    agents: Vec<poph_types::AgentId>,
) -> Option<&'static str> {
    let mut fail = false;
    for agent_id in agents {
        if synchronise(e, org, agent_id).await.is_err() {
            fail = true;
        }
    }
    fail.then_some("índice não atualizado; execute reindex para os agentes afetados")
}
async fn group_notice(e: &Estado, org: OrgId, group: KnowledgeGroupId) -> Option<&'static str> {
    match e.store.knowledge().agents_from_group(org, group) {
        Ok(agents) => sync_notice(e, org, agents).await,
        Err(_) => Some("índice não atualizado; execute reindex para o agente"),
    }
}
