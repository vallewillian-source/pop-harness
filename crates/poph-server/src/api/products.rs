use super::orgs::OrgPath;
use super::{ListJson, PageQuery, next_cursor};
use crate::{
    auth::Identity,
    error::{ApiError, id_from_url as parse_path_id},
    state::ServerState,
};
use axum::{
    Json,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
};
use poph_types::{OrgId, ProductId};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct ProductsQuery {
    #[serde(flatten)]
    #[serde(rename = "pagina")]
    page: PageQuery,
    all: Option<bool>,
}

#[derive(Deserialize)]
pub struct ProductPath {
    org_id: String,
    product_id: String,
}

fn product_json(p: &poph_store::Product) -> Value {
    json!({"id":p.id.to_string(), "org_id":p.org_id.to_string(), "category_id":p.category_id.to_string(),
        "name":p.name, "slug":p.slug, "aliases":p.aliases, "description":p.description,
        "price_text":poph_agent::price_text(p), "price_cents":p.price_cents, "unit":p.unit,
        "includes":p.includes, "excludes":p.exclui, "conditions":p.conditions, "tags":p.tags,
        "status":p.status.as_text(), "version":p.version, "created_at":p.created_at, "updated_at":p.updated_at})
}

#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/products", tag="products", operation_id="produtos_listar", security(("bearer"=[])), params(("org_id"=String, Path), ("limit"=Option<u32>, Query), ("cursor"=Option<String>, Query), ("all"=Option<bool>, Query)), responses((status=200, description="Produtos", body=ListJson<Value>)))]
pub async fn list(
    State(e): State<ServerState>,
    ident: Identity,
    Path(c): Path<OrgPath>,
    query: Result<Query<ProductsQuery>, QueryRejection>,
) -> Result<Json<ListJson<Value>>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let Query(q) = query.map_err(|_| ApiError::invalid("parâmetros de paginação inválidos"))?;
    let page = q.page.page()?;
    let products = if q.all.unwrap_or(false) {
        e.store.products().list(org, page)?
    } else {
        e.store.products().list_active(org, page)?
    };
    let items: Vec<_> = products.iter().map(product_json).collect();
    let next_cursor = next_cursor(&items, page.limit, |v| {
        v["id"].as_str().unwrap_or_default().to_owned()
    });
    Ok(Json(ListJson { items, next_cursor }))
}

#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/products/{product_id}", tag="products", operation_id="produtos_mostrar", security(("bearer"=[])), params(("org_id"=String, Path), ("product_id"=String, Path)), responses((status=200, description="Produto", body=Value), (status=404, description="Não encontrado", body=crate::error::ErrorResponse)))]
pub async fn show(
    State(e): State<ServerState>,
    ident: Identity,
    Path(c): Path<ProductPath>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let id = parse_path_id::<ProductId>(&c.product_id)?;
    Ok(Json(product_json(&e.store.products().get(org, id)?)))
}

#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = DadosProduto)]
pub struct ProductData {
    category_id: Option<String>,
    name: Option<String>,
    slug: Option<String>,
    aliases: Option<Vec<String>>,
    description: Option<String>,
    price_text: Option<String>,
    price_cents: Option<i64>,
    unit: Option<String>,
    includes: Option<String>,
    excludes: Option<String>,
    conditions: Option<String>,
    tags: Option<Vec<String>>,
    status: Option<String>,
}

#[utoipa::path(operation_id = "criar", post,path="/admin/v1/orgs/{org_id}/products",tag="products",security(("bearer"=[])),params(("org_id"=String,Path)),request_body=ProductData,responses((status=201,description="Produto criado",body=Value)))]
pub async fn create(
    State(e): State<ServerState>,
    ident: Identity,
    Path(c): Path<OrgPath>,
    body: Result<Json<ProductData>, JsonRejection>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let Json(d) = body.map_err(|_| ApiError::invalid("corpo JSON de produto inválido"))?;
    let new_product = poph_store::NewProduct {
        category_id: d
            .category_id
            .as_deref()
            .map(parse_path_id::<poph_types::ProductCategoryId>)
            .transpose()?,
        name: d
            .name
            .ok_or_else(|| ApiError::validation("name", "campo obrigatório"))?,
        slug: d.slug,
        aliases: d.aliases.unwrap_or_default(),
        description: d.description.unwrap_or_default(),
        price_text: d.price_text.unwrap_or_default(),
        price_cents: d.price_cents.unwrap_or_default(),
        unit: d.unit.unwrap_or_default(),
        includes: d.includes.unwrap_or_default(),
        exclui: d.excludes.unwrap_or_default(),
        conditions: d.conditions.unwrap_or_default(),
        tags: d.tags.unwrap_or_default(),
    };
    if d.status.is_some() {
        return Err(ApiError::validation("status", "não permitido na criação"));
    }
    let p = e.store.products().create(org, new_product)?;
    super::audit::register(&e, &ident, Some(org), "create", "product", p.id.to_string())?;
    let mut response = product_json(&p);
    response["index_warning"] = json!(synchronise_catalog(&e, org).await);
    Ok((axum::http::StatusCode::CREATED, Json(response)))
}

#[utoipa::path(operation_id = "atualizar", patch,path="/admin/v1/orgs/{org_id}/products/{product_id}",tag="products",security(("bearer"=[])),params(("org_id"=String,Path),("product_id"=String,Path),("If-Match"=String,Header)),request_body=ProductData,responses((status=200,description="Produto alterado",body=Value),(status=412,description="Conflito",body=crate::error::ErrorResponse)))]
pub async fn update(
    State(e): State<ServerState>,
    ident: Identity,
    Path(c): Path<ProductPath>,
    headers: axum::http::HeaderMap,
    body: Result<Json<ProductData>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let id = parse_path_id::<ProductId>(&c.product_id)?;
    let version = super::if_match_version(&headers, "produto")?;
    let Json(d) = body.map_err(|_| ApiError::invalid("corpo JSON de produto inválido"))?;
    let change = poph_store::ProductChange {
        category_id: d
            .category_id
            .as_deref()
            .map(parse_path_id::<poph_types::ProductCategoryId>)
            .transpose()?,
        name: d.name,
        slug: d.slug,
        aliases: d.aliases,
        description: d.description,
        price_text: d.price_text,
        price_cents: d.price_cents,
        unit: d.unit,
        includes: d.includes,
        exclui: d.excludes,
        conditions: d.conditions,
        tags: d.tags,
        status: d.status.as_deref().map(super::request_status).transpose()?,
    };
    let p = e.store.products().update(org, id, version, change)?;
    super::audit::register(&e, &ident, Some(org), "update", "product", id.to_string())?;
    let mut response = product_json(&p);
    response["index_warning"] = json!(synchronise_catalog(&e, org).await);
    Ok(Json(response))
}

#[doc = " Parâmetros de exclusão de produto."]
#[derive(Debug, Clone, Default, Deserialize, utoipa::ToSchema)]
#[schema(as = ProductDeletion)]
pub struct ExclusionProduct {
    #[doc = " Confirma a exclusão de um produto que está em itens de carrinho."]
    pub force: Option<bool>,
}

#[doc = " `DELETE /admin/v1/orgs/{org_id}/products/{product_id}`."]
#[doc = ""]
#[doc = " Sem vínculo com carrinho, apaga direto (como a categoria sem vínculo). Com itens de carrinho,"]
#[doc = " sem `force=true` responde 409 com a contagem e nada é apagado. Com `force=true`, apaga: os itens"]
#[doc = " de carrinho continuam, com o preço gravado, mas sem o produto (aparecem como \"produto removido"]
#[doc = " do catálogo\")."]
#[utoipa::path(
    delete,
    path = "/admin/v1/orgs/{org_id}/products/{product_id}",
    tag = "products",
    operation_id = "produtos_excluir",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path, description = "Id da org"),
        ("product_id" = String, Path, description = "Id do produto"),
        ("force" = Option<bool>, Query, description = "Confirma a exclusão de produto que está em itens de carrinho")
    ),
    responses(
        (status = 200, description = "Produto excluído", body = Value),
        (status = 404, description = "Produto não encontrado", body = crate::error::ErrorResponse),
        (status = 409, description = "Produto está em itens de carrinho e faltou force", body = crate::error::ErrorResponse)
    )
)]
pub async fn delete(
    State(e): State<ServerState>,
    ident: Identity,
    Path(c): Path<ProductPath>,
    Query(deletion): Query<ExclusionProduct>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let id = parse_path_id::<ProductId>(&c.product_id)?;
    let forced = deletion.force.unwrap_or(false);
    if !forced {
        let refs = e.store.products().references(org, id)?;
        if refs.cart_items > 0 {
            return Err(ApiError::conflict(format!(
                "o produto está em {} item(ns) de {} carrinho(s); excluir faz esses itens ficarem \
                 como \"produto removido do catálogo\" (o preço gravado continua). \
                 Para confirmar, repita com force=true",
                refs.cart_items, refs.carts
            ))
            .with_details(json!({
                "itens_de_carrinho": refs.cart_items,
                "carrinhos": refs.carts
            })));
        }
    }
    e.store.products().delete(org, id)?;
    super::audit::register(
        &e,
        &ident,
        Some(org),
        if forced { "delete_force" } else { "delete" },
        "product",
        id.to_string(),
    )?;
    let notice = synchronise_catalog(&e, org).await;
    Ok(Json(json!({"deleted": true, "index_warning": notice})))
}

async fn synchronise_catalog(e: &ServerState, org: OrgId) -> Option<&'static str> {
    use poph_rag::{Embedder, Index, IndexConfig, ModelConfig, OrtEmbedder};
    use std::sync::Arc;
    static SINCRONIZACAO: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = SINCRONIZACAO.lock().await;
    let config = ModelConfig {
        diretorio: e.config.models(),
        ..ModelConfig::default()
    };
    let model =
        match tokio::task::spawn_blocking(move || OrtEmbedder::open_model(&config, None)).await {
            Ok(Ok(model)) => model,
            _ => return Some("índice de produtos não atualizado; execute product reindex"),
        };
    let embedder: Arc<dyn Embedder> = Arc::new(model);
    let (index, _) = match Index::open(e.store.rag_path(), embedder, IndexConfig::default()) {
        Ok(index) => index,
        Err(_) => return Some("índice de produtos não atualizado; execute product reindex"),
    };
    let catalog = poph_agent::ProductCatalog::new(Arc::clone(&e.store), Arc::new(index));
    if catalog.synchronise_org(org).await.is_err() {
        return Some("índice de produtos não atualizado; execute product reindex");
    }
    None
}
