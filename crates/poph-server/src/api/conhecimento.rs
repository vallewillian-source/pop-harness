//! Grupos da base de conhecimento.
use super::{ConsultaPagina, ListaJson, proximo_cursor, status_pedido, versao_do_if_match};
use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url},
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
pub struct Caminho {
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
fn ids(c: Caminho, ident: &Identidade) -> Result<(OrgId, Option<KnowledgeGroupId>), ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
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
fn grupo(g: &KnowledgeGroup) -> Value {
    json!({"id":g.id.to_string(),"org_id":g.org_id.to_string(),"name":g.name,"description":g.description,"status":g.status.as_text(),"version":g.version,"created_at":g.created_at,"updated_at":g.updated_at})
}
#[utoipa::path(get,path="/admin/v1/orgs/{org_id}/knowledge",tag="knowledge",params(("org_id"=String,Path),("limit"=Option<u32>,Query),("cursor"=Option<String>,Query)),security(("bearer"=[])),responses((status=200,description="Grupos",body=ListaJson<Value>)))]
pub async fn listar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<Caminho>,
    q: Result<Query<ConsultaPagina>, QueryRejection>,
) -> Result<Json<ListaJson<Value>>, ApiError> {
    let (org, _) = ids(c, &ident)?;
    let Query(q) = q.map_err(|_| ApiError::invalido("parâmetros de paginação inválidos"))?;
    let p = q.pagina()?;
    let items: Vec<_> = e
        .store
        .knowledge()
        .list_groups(org, p)?
        .iter()
        .map(grupo)
        .collect();
    let next_cursor = proximo_cursor(&items, p.limit, |v| {
        v["id"].as_str().unwrap_or_default().to_owned()
    });
    Ok(Json(ListaJson { items, next_cursor }))
}
#[utoipa::path(get,path="/admin/v1/orgs/{org_id}/knowledge/{group_id}",tag="knowledge",params(("org_id"=String,Path),("group_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Grupo",body=Value)))]
pub async fn mostrar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<Caminho>,
) -> Result<Json<Value>, ApiError> {
    let (org, id) = ids(c, &ident)?;
    let id = id.ok_or_else(|| ApiError::invalido("grupo ausente"))?;
    let mut valor = grupo(&e.store.knowledge().get_group(org, id)?);
    let agentes = e.store.knowledge().agents_from_group(org, id)?;
    valor["agents"] = json!(agentes.iter().map(ToString::to_string).collect::<Vec<_>>());
    Ok(Json(valor))
}
#[utoipa::path(post,path="/admin/v1/orgs/{org_id}/knowledge",tag="knowledge",params(("org_id"=String,Path)),security(("bearer"=[])),request_body=Dados,responses((status=201,description="Criado",body=Value)))]
pub async fn criar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<Caminho>,
    corpo: Result<Json<Dados>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let (org, _) = ids(c, &ident)?;
    let Json(d) = corpo.map_err(|_| ApiError::invalido("corpo JSON inválido"))?;
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
    super::auditoria::registrar(
        &e,
        &ident,
        Some(org),
        "create",
        "knowledge_group",
        g.id.to_string(),
    )?;
    Ok((StatusCode::CREATED, Json(grupo(&g))))
}
#[utoipa::path(patch,path="/admin/v1/orgs/{org_id}/knowledge/{group_id}",tag="knowledge",params(("org_id"=String,Path),("group_id"=String,Path),("If-Match"=String,Header)),security(("bearer"=[])),request_body=Dados,responses((status=200,description="Alterado",body=Value),(status=412,description="Conflito",body=crate::erro::RespostaErro)))]
pub async fn atualizar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<Caminho>,
    h: HeaderMap,
    corpo: Result<Json<Dados>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let (org, id) = ids(c, &ident)?;
    let id = id.ok_or_else(|| ApiError::invalido("grupo ausente"))?;
    let versao = versao_do_if_match(&h, "grupo")?;
    let Json(d) = corpo.map_err(|_| ApiError::invalido("corpo JSON inválido"))?;
    let g = e.store.knowledge().update_group(
        org,
        id,
        versao,
        KnowledgeGroupChange {
            name: d.name,
            description: d.description,
            status: d.status.as_deref().map(status_pedido).transpose()?,
        },
    )?;
    super::auditoria::registrar(
        &e,
        &ident,
        Some(org),
        "update",
        "knowledge_group",
        id.to_string(),
    )?;
    let aviso = aviso_grupo(&e, org, id).await;
    let mut valor = grupo(&g);
    valor["index_warning"] = json!(aviso);
    Ok(Json(valor))
}
#[utoipa::path(delete,path="/admin/v1/orgs/{org_id}/knowledge/{group_id}",tag="knowledge",params(("org_id"=String,Path),("group_id"=String,Path),("force"=Option<bool>,Query)),security(("bearer"=[])),responses((status=200,description="Excluído",body=Value)))]
pub async fn excluir(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<Caminho>,
    q: Result<Query<ConfirmarExclusao>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let (org, id) = ids(c, &ident)?;
    let id = id.ok_or_else(|| ApiError::invalido("grupo ausente"))?;
    let Query(q) = q.map_err(|_| ApiError::invalido("parâmetro force inválido"))?;
    if !q.force.unwrap_or(false) {
        return Err(ApiError::conflito(
            "grupo pode conter itens e vínculos; confirme com force=true",
        ));
    }
    let agentes = e.store.knowledge().agents_from_group(org, id)?;
    e.store.knowledge().delete_group(org, id)?;
    super::auditoria::registrar(
        &e,
        &ident,
        Some(org),
        "delete",
        "knowledge_group",
        id.to_string(),
    )?;
    let aviso = aviso_sincronizar(&e, org, agentes).await;
    Ok(Json(
        json!({"removido":true,"id":id.to_string(),"index_warning":aviso}),
    ))
}

#[derive(Deserialize)]
pub struct ConfirmarExclusao {
    force: Option<bool>,
}

#[derive(Deserialize)]
pub struct CaminhoItem {
    org_id: String,
    group_id: String,
    item_id: Option<String>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DadosItem {
    group_id: Option<String>,
    title: Option<String>,
    content: Option<String>,
    tags: Option<Vec<String>>,
    position: Option<i64>,
    status: Option<String>,
}
fn ids_item(
    c: CaminhoItem,
    ident: &Identidade,
) -> Result<(OrgId, KnowledgeGroupId, Option<poph_types::KnowledgeItemId>), ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
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
fn conferir_item(
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
#[utoipa::path(get,path="/admin/v1/orgs/{org_id}/knowledge/{group_id}/items",tag="knowledge",params(("org_id"=String,Path),("group_id"=String,Path),("limit"=Option<u32>,Query),("cursor"=Option<String>,Query)),security(("bearer"=[])),responses((status=200,description="Itens",body=ListaJson<Value>)))]
pub async fn listar_itens(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoItem>,
    q: Result<Query<ConsultaPagina>, QueryRejection>,
) -> Result<Json<ListaJson<Value>>, ApiError> {
    let (org, g, _) = ids_item(c, &ident)?;
    e.store.knowledge().get_group(org, g)?;
    let Query(q) = q.map_err(|_| ApiError::invalido("parâmetros de paginação inválidos"))?;
    let p = q.pagina()?;
    let items: Vec<_> = e
        .store
        .knowledge()
        .list_items_by_id(org, g, p)?
        .iter()
        .map(item_json)
        .collect();
    let next_cursor = proximo_cursor(&items, p.limit, |v| {
        v["id"].as_str().unwrap_or_default().to_owned()
    });
    Ok(Json(ListaJson { items, next_cursor }))
}
#[utoipa::path(get,path="/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}",tag="knowledge",params(("org_id"=String,Path),("group_id"=String,Path),("item_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Item",body=Value)))]
pub async fn mostrar_item(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoItem>,
) -> Result<Json<Value>, ApiError> {
    let (org, g, id) = ids_item(c, &ident)?;
    Ok(Json(item_json(&conferir_item(
        &e,
        org,
        g,
        id.ok_or_else(|| ApiError::invalido("item ausente"))?,
    )?)))
}
#[utoipa::path(post,path="/admin/v1/orgs/{org_id}/knowledge/{group_id}/items",tag="knowledge",params(("org_id"=String,Path),("group_id"=String,Path)),security(("bearer"=[])),request_body=DadosItem,responses((status=201,description="Criado",body=Value)))]
pub async fn criar_item(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoItem>,
    corpo: Result<Json<DadosItem>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let (org, g, _) = ids_item(c, &ident)?;
    let Json(d) = corpo.map_err(|_| ApiError::invalido("corpo JSON inválido"))?;
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
    super::auditoria::registrar(
        &e,
        &ident,
        Some(org),
        "create",
        "knowledge_item",
        i.id.to_string(),
    )?;
    let aviso = aviso_grupo(&e, org, g).await;
    let mut valor = item_json(&i);
    valor["index_warning"] = json!(aviso);
    Ok((StatusCode::CREATED, Json(valor)))
}
#[utoipa::path(patch,path="/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}",tag="knowledge",params(("org_id"=String,Path),("group_id"=String,Path),("item_id"=String,Path),("If-Match"=String,Header)),security(("bearer"=[])),request_body=DadosItem,responses((status=200,description="Alterado",body=Value),(status=412,description="Conflito",body=crate::erro::RespostaErro)))]
pub async fn atualizar_item(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoItem>,
    h: HeaderMap,
    corpo: Result<Json<DadosItem>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let (org, g, id) = ids_item(c, &ident)?;
    let id = id.ok_or_else(|| ApiError::invalido("item ausente"))?;
    conferir_item(&e, org, g, id)?;
    let versao = versao_do_if_match(&h, "item")?;
    let Json(d) = corpo.map_err(|_| ApiError::invalido("corpo JSON inválido"))?;
    let novo_grupo = d
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
        versao,
        poph_store::KnowledgeItemChange {
            group_id: novo_grupo,
            title: d.title,
            content: d.content,
            tags: d.tags,
            position: d.position,
            status: d.status.as_deref().map(status_pedido).transpose()?,
        },
    )?;
    super::auditoria::registrar(
        &e,
        &ident,
        Some(org),
        "update",
        "knowledge_item",
        id.to_string(),
    )?;
    let aviso = aviso_grupo(&e, org, g).await;
    let aviso_novo = match novo_grupo {
        Some(outro) if outro != g => aviso_grupo(&e, org, outro).await,
        _ => None,
    };
    let mut valor = item_json(&i);
    valor["index_warning"] = json!(aviso.or(aviso_novo));
    Ok(Json(valor))
}
#[utoipa::path(delete,path="/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}",tag="knowledge",params(("org_id"=String,Path),("group_id"=String,Path),("item_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Excluído",body=Value)))]
pub async fn excluir_item(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoItem>,
) -> Result<Json<Value>, ApiError> {
    let (org, g, id) = ids_item(c, &ident)?;
    let id = id.ok_or_else(|| ApiError::invalido("item ausente"))?;
    conferir_item(&e, org, g, id)?;
    e.store.knowledge().delete_item(org, id)?;
    super::auditoria::registrar(
        &e,
        &ident,
        Some(org),
        "delete",
        "knowledge_item",
        id.to_string(),
    )?;
    let aviso = aviso_grupo(&e, org, g).await;
    Ok(Json(
        json!({"removido":true,"id":id.to_string(),"index_warning":aviso}),
    ))
}

#[derive(Deserialize)]
pub struct CaminhoAgente {
    org_id: String,
    agent_id: String,
    group_id: Option<String>,
}
fn ids_agente(
    c: CaminhoAgente,
    ident: &Identidade,
) -> Result<(OrgId, poph_types::AgentId, Option<KnowledgeGroupId>), ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let agente = id_da_url::<poph_types::AgentId>(&c.agent_id)?;
    let grupo = c
        .group_id
        .as_deref()
        .map(|s| {
            s.parse::<KnowledgeGroupId>()
                .map_err(|_| ApiError::invalido("id de grupo inválido"))
        })
        .transpose()?;
    Ok((org, agente, grupo))
}

#[utoipa::path(post,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/{group_id}",tag="knowledge",params(("org_id"=String,Path),("agent_id"=String,Path),("group_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Vínculo criado",body=Value)))]
pub async fn vincular(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoAgente>,
) -> Result<Json<Value>, ApiError> {
    let (org, agente, grupo) = ids_agente(c, &ident)?;
    let grupo = grupo.ok_or_else(|| ApiError::invalido("grupo ausente"))?;
    e.store.knowledge().link_agent(org, agente, grupo)?;
    super::auditoria::registrar(
        &e,
        &ident,
        Some(org),
        "link",
        "knowledge_group",
        grupo.to_string(),
    )?;
    let aviso = aviso_sincronizar(&e, org, vec![agente]).await;
    Ok(Json(
        json!({"agent":agente.to_string(),"group":grupo.to_string(),"linked":true,"index_warning":aviso}),
    ))
}
#[utoipa::path(delete,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/{group_id}",tag="knowledge",params(("org_id"=String,Path),("agent_id"=String,Path),("group_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Vínculo removido",body=Value)))]
pub async fn desvincular(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoAgente>,
) -> Result<Json<Value>, ApiError> {
    let (org, agente, grupo) = ids_agente(c, &ident)?;
    let grupo = grupo.ok_or_else(|| ApiError::invalido("grupo ausente"))?;
    e.store.agents().get(org, agente)?;
    e.store.knowledge().get_group(org, grupo)?;
    let mudou = e.store.knowledge().unlink_agent(org, agente, grupo)?;
    super::auditoria::registrar(
        &e,
        &ident,
        Some(org),
        "unlink",
        "knowledge_group",
        grupo.to_string(),
    )?;
    let aviso = aviso_sincronizar(&e, org, vec![agente]).await;
    Ok(Json(
        json!({"agent":agente.to_string(),"group":grupo.to_string(),"linked":false,"changed":mudou,"index_warning":aviso}),
    ))
}
#[utoipa::path(post,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/reindex",tag="knowledge",params(("org_id"=String,Path),("agent_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Índice sincronizado",body=Value)))]
pub async fn reindexar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoAgente>,
) -> Result<Json<Value>, ApiError> {
    let (org, agente, _) = ids_agente(c, &ident)?;
    e.store.agents().get(org, agente)?;
    let r = sincronizar(&e, org, agente).await?;
    super::auditoria::registrar(
        &e,
        &ident,
        Some(org),
        "reindex",
        "agent",
        agente.to_string(),
    )?;
    Ok(Json(
        json!({"agent":agente.to_string(),"indexed":{"itens_novos":r.itens_novos,"itens_alterados":r.itens_alterados,"itens_inalterados":r.itens_inalterados,"itens_removidos":r.itens_removidos,"embeddings_gerados":r.embeddings_gerados,"trechos_gravados":r.trechos_gravados}}),
    ))
}

async fn sincronizar(
    e: &Estado,
    org: OrgId,
    agente: poph_types::AgentId,
) -> Result<poph_rag::RelatorioIndexacao, ApiError> {
    use poph_rag::{
        ConfigFila, ConfigIndice, ConfigModelo, Embedder, EmbedderDaFila, EmbedderOrt,
        FilaEmbedding, Indice,
    };
    use std::sync::Arc;
    // O índice e a sessão de embedding são compartilhados pelo processo; serializar evita
    // sincronizações concorrentes fora de ordem e várias cargas simultâneas do modelo.
    static SINCRONIZACAO: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guarda = SINCRONIZACAO.lock().await;
    let config = ConfigModelo {
        diretorio: e.config.models(),
        ..ConfigModelo::default()
    };
    let modelo = tokio::task::spawn_blocking(move || EmbedderOrt::abrir(&config, None))
        .await
        .map_err(|_| ApiError::interno())?
        .map_err(|_| {
            ApiError::novo(
                StatusCode::SERVICE_UNAVAILABLE,
                "embedding_unavailable",
                "modelo de embedding indisponível; instale o modelo",
            )
        })?;
    let identidade = modelo.identidade().clone();
    let modelo: Arc<dyn Embedder> = Arc::new(modelo);
    let fila = Arc::new(
        FilaEmbedding::iniciar(modelo, ConfigFila::default(), None)
            .map_err(|_| ApiError::interno())?,
    );
    let adaptador = Arc::new(EmbedderDaFila::novo(fila, org, identidade));
    let (indice, _) = Indice::abrir(e.store.rag_path(), adaptador, ConfigIndice::default())
        .map_err(|_| ApiError::interno())?;
    let base = poph_agent::BaseConhecimento::nova(Arc::clone(&e.store), Arc::new(indice));
    base.sincronizar_agente(org, agente)
        .await
        .map_err(|_| ApiError::interno())
}

async fn aviso_sincronizar(
    e: &Estado,
    org: OrgId,
    agentes: Vec<poph_types::AgentId>,
) -> Option<&'static str> {
    let mut falhou = false;
    for agente in agentes {
        if sincronizar(e, org, agente).await.is_err() {
            falhou = true;
        }
    }
    falhou.then_some("índice não atualizado; execute reindex para os agentes afetados")
}
async fn aviso_grupo(e: &Estado, org: OrgId, grupo: KnowledgeGroupId) -> Option<&'static str> {
    match e.store.knowledge().agents_from_group(org, grupo) {
        Ok(agentes) => aviso_sincronizar(e, org, agentes).await,
        Err(_) => Some("índice não atualizado; execute reindex para o agente"),
    }
}
