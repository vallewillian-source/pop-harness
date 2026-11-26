//! Consultas do catálogo de produtos, escopadas por organização.
use super::orgs::CaminhoOrg;
use super::{ConsultaPagina, ListaJson, proximo_cursor};
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
};
use poph_types::{OrgId, ProductId};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct ConsultaProdutos {
    #[serde(flatten)]
    pagina: ConsultaPagina,
    all: Option<bool>,
}

#[derive(Deserialize)]
pub struct CaminhoProduto {
    org_id: String,
    product_id: String,
}

fn produto_json(p: &poph_store::Product) -> Value {
    json!({"id":p.id.to_string(), "org_id":p.org_id.to_string(), "category_id":p.category_id.to_string(),
        "name":p.name, "slug":p.slug, "aliases":p.aliases, "description":p.description,
        "price_text":poph_agent::preco_texto(p), "price_cents":p.price_cents, "unit":p.unit,
        "includes":p.includes, "excludes":p.exclui, "conditions":p.conditions, "tags":p.tags,
        "status":p.status.as_text(), "version":p.version, "created_at":p.created_at, "updated_at":p.updated_at})
}

#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/products", tag="products", operation_id="produtos_listar", security(("bearer"=[])), params(("org_id"=String, Path), ("limit"=Option<u32>, Query), ("cursor"=Option<String>, Query), ("all"=Option<bool>, Query)), responses((status=200, description="Produtos", body=ListaJson<Value>)))]
pub async fn listar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoOrg>,
    query: Result<Query<ConsultaProdutos>, QueryRejection>,
) -> Result<Json<ListaJson<Value>>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let Query(q) = query.map_err(|_| ApiError::invalido("parâmetros de paginação inválidos"))?;
    let pagina = q.pagina.pagina()?;
    let produtos = if q.all.unwrap_or(false) {
        e.store.products().listar(org, pagina)?
    } else {
        e.store.products().listar_ativos(org, pagina)?
    };
    let items: Vec<_> = produtos.iter().map(produto_json).collect();
    let next_cursor = proximo_cursor(&items, pagina.limit, |v| {
        v["id"].as_str().unwrap_or_default().to_owned()
    });
    Ok(Json(ListaJson { items, next_cursor }))
}

#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/products/{product_id}", tag="products", operation_id="produtos_mostrar", security(("bearer"=[])), params(("org_id"=String, Path), ("product_id"=String, Path)), responses((status=200, description="Produto", body=Value), (status=404, description="Não encontrado", body=crate::erro::RespostaErro)))]
pub async fn mostrar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoProduto>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<ProductId>(&c.product_id)?;
    Ok(Json(produto_json(&e.store.products().obter(org, id)?)))
}

#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DadosProduto {
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

#[utoipa::path(post,path="/admin/v1/orgs/{org_id}/products",tag="products",security(("bearer"=[])),params(("org_id"=String,Path)),request_body=DadosProduto,responses((status=201,description="Produto criado",body=Value)))]
pub async fn criar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoOrg>,
    corpo: Result<Json<DadosProduto>, JsonRejection>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let Json(d) = corpo.map_err(|_| ApiError::invalido("corpo JSON de produto inválido"))?;
    let novo = poph_store::NewProduct {
        category_id: d
            .category_id
            .as_deref()
            .map(id_da_url::<poph_types::ProductCategoryId>)
            .transpose()?,
        name: d
            .name
            .ok_or_else(|| ApiError::validacao("name", "campo obrigatório"))?,
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
        return Err(ApiError::validacao("status", "não permitido na criação"));
    }
    let p = e.store.products().criar(org, novo)?;
    super::auditoria::registrar(&e, &ident, Some(org), "create", "product", p.id.to_string())?;
    let mut resposta = produto_json(&p);
    resposta["index_warning"] = json!(sincronizar_catalogo(&e, org).await);
    Ok((axum::http::StatusCode::CREATED, Json(resposta)))
}

#[utoipa::path(patch,path="/admin/v1/orgs/{org_id}/products/{product_id}",tag="products",security(("bearer"=[])),params(("org_id"=String,Path),("product_id"=String,Path),("If-Match"=String,Header)),request_body=DadosProduto,responses((status=200,description="Produto alterado",body=Value),(status=412,description="Conflito",body=crate::erro::RespostaErro)))]
pub async fn atualizar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoProduto>,
    headers: axum::http::HeaderMap,
    corpo: Result<Json<DadosProduto>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<ProductId>(&c.product_id)?;
    let versao = super::versao_do_if_match(&headers, "produto")?;
    let Json(d) = corpo.map_err(|_| ApiError::invalido("corpo JSON de produto inválido"))?;
    let mudanca = poph_store::ProductChange {
        category_id: d
            .category_id
            .as_deref()
            .map(id_da_url::<poph_types::ProductCategoryId>)
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
        status: d.status.as_deref().map(super::status_pedido).transpose()?,
    };
    let p = e.store.products().atualizar(org, id, versao, mudanca)?;
    super::auditoria::registrar(&e, &ident, Some(org), "update", "product", id.to_string())?;
    let mut resposta = produto_json(&p);
    resposta["index_warning"] = json!(sincronizar_catalogo(&e, org).await);
    Ok(Json(resposta))
}

/// Parâmetros de exclusão de produto.
#[derive(Debug, Clone, Default, Deserialize, utoipa::ToSchema)]
pub struct ExclusaoProduto {
    /// Confirma a exclusão de um produto que está em itens de carrinho.
    pub force: Option<bool>,
}

/// `DELETE /admin/v1/orgs/{org_id}/products/{product_id}`.
///
/// Sem vínculo com carrinho, apaga direto (como a categoria sem vínculo). Com itens de carrinho,
/// sem `force=true` responde 409 com a contagem e nada é apagado. Com `force=true`, apaga: os itens
/// de carrinho continuam, com o preço gravado, mas sem o produto (aparecem como "produto removido
/// do catálogo").
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
        (status = 404, description = "Produto não encontrado", body = crate::erro::RespostaErro),
        (status = 409, description = "Produto está em itens de carrinho e faltou force", body = crate::erro::RespostaErro)
    )
)]
pub async fn excluir(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoProduto>,
    Query(exclusao): Query<ExclusaoProduto>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<ProductId>(&c.product_id)?;
    let forcado = exclusao.force.unwrap_or(false);
    if !forcado {
        let refs = e.store.products().referencias(org, id)?;
        if refs.itens_de_carrinho > 0 {
            return Err(ApiError::conflito(format!(
                "o produto está em {} item(ns) de {} carrinho(s); excluir faz esses itens ficarem \
                 como \"produto removido do catálogo\" (o preço gravado continua). \
                 Para confirmar, repita com force=true",
                refs.itens_de_carrinho, refs.carrinhos
            ))
            .com_detalhes(json!({
                "itens_de_carrinho": refs.itens_de_carrinho,
                "carrinhos": refs.carrinhos
            })));
        }
    }
    e.store.products().excluir(org, id)?;
    super::auditoria::registrar(
        &e,
        &ident,
        Some(org),
        if forcado { "delete_force" } else { "delete" },
        "product",
        id.to_string(),
    )?;
    let aviso = sincronizar_catalogo(&e, org).await;
    Ok(Json(json!({"deleted": true, "index_warning": aviso})))
}

// Como na CLI, a escrita permanece válida se o modelo ou o índice estiverem indisponíveis.
async fn sincronizar_catalogo(e: &Estado, org: OrgId) -> Option<&'static str> {
    use poph_rag::{ConfigIndice, ConfigModelo, Embedder, EmbedderOrt, Indice};
    use std::sync::Arc;
    static SINCRONIZACAO: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guarda = SINCRONIZACAO.lock().await;
    let config = ConfigModelo {
        diretorio: e.config.models(),
        ..ConfigModelo::default()
    };
    let modelo = match tokio::task::spawn_blocking(move || EmbedderOrt::abrir(&config, None)).await
    {
        Ok(Ok(modelo)) => modelo,
        _ => return Some("índice de produtos não atualizado; execute product reindex"),
    };
    let embedder: Arc<dyn Embedder> = Arc::new(modelo);
    let (indice, _) = match Indice::abrir(e.store.rag_path(), embedder, ConfigIndice::default()) {
        Ok(indice) => indice,
        Err(_) => return Some("índice de produtos não atualizado; execute product reindex"),
    };
    let catalogo = poph_agent::CatalogoProdutos::novo(Arc::clone(&e.store), Arc::new(indice));
    if catalogo.sincronizar_org(org).await.is_err() {
        return Some("índice de produtos não atualizado; execute product reindex");
    }
    None
}
