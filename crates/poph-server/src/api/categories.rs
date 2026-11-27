use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use poph_store::{CategoryChange, NewCategory, Page, ProductCategory};
use poph_types::{OrgId, ProductCategoryId};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

use super::orgs::OrgPath;
use super::pagination::{ListJson, PageQuery, next_cursor};
use super::{if_match_version, request_status, status_text};
use crate::auth::Identity;
use crate::error::{ApiError, id_from_url as parse_path_id};
use crate::state::ServerState;

#[doc = " Categoria na resposta da API."]
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CategoriaJson)]
pub struct CategoryJson {
    #[doc = " Id (UUID v7)."]
    pub id: String,
    #[doc = " Org dona."]
    pub org_id: String,
    #[doc = " Nome."]
    pub name: String,
    #[doc = " Descrição."]
    pub description: String,
    #[doc = " `active`, `inactive` ou `archived`."]
    pub status: String,
    #[doc = " Versão para `If-Match`."]
    pub version: u64,
    #[doc = " Criação (ms)."]
    pub created_at: i64,
    #[doc = " Última alteração (ms)."]
    pub updated_at: i64,
}

impl CategoryJson {
    fn from_category(category: &ProductCategory) -> Self {
        Self {
            id: category.id.to_string(),
            org_id: category.org_id.to_string(),
            name: category.name.clone(),
            description: category.description.clone(),
            status: status_text(category.status).to_owned(),
            version: category.version,
            created_at: category.created_at,
            updated_at: category.updated_at,
        }
    }
}

#[doc = " Corpo de criação de categoria."]
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[schema(as = CriarCategoria)]
pub struct CreateCategory {
    #[doc = " Nome (único na org, sem diferenciar maiúsculas)."]
    pub name: String,
    #[doc = " Descrição."]
    pub description: Option<String>,
}

#[doc = " Corpo de alteração de categoria."]
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[schema(as = AtualizarCategoria)]
pub struct UpdateCategory {
    #[doc = " Novo nome."]
    pub name: Option<String>,
    #[doc = " Nova descrição."]
    pub description: Option<String>,
    #[doc = " Nova situação."]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CategoryPath {
    pub org_id: String,
    pub category_id: String,
}

#[doc = " `GET /admin/v1/orgs/{org_id}/categories`."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/categories",
    tag = "categories",
    operation_id = "categorias_listar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path, description = "Id da org"),
        ("limit" = Option<u32>, Query, description = "Registros por página (1 a 200)"),
        ("cursor" = Option<String>, Query, description = "Último id da página anterior")
    ),
    responses(
        (status = 200, description = "Página de categorias", body = ListJson<CategoryJson>),
        (status = 403, description = "Org de outro token", body = crate::error::ErrorResponse)
    )
)]
pub async fn list(
    State(state): State<ServerState>,
    identity: Identity,
    Path(path): Path<OrgPath>,
    Query(query): Query<PageQuery>,
) -> Result<Json<ListJson<CategoryJson>>, ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    identity.require_org(org)?;
    let page = query.page()?;
    let categories = state.store.categories().list(org, page)?;
    let items: Vec<CategoryJson> = categories.iter().map(CategoryJson::from_category).collect();
    let next_cursor = next_cursor(&items, page.limit, |item| item.id.clone());
    Ok(Json(ListJson { items, next_cursor }))
}

#[doc = " `POST /admin/v1/orgs/{org_id}/categories`."]
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/categories",
    tag = "categories",
    operation_id = "categorias_criar",
    security(("bearer" = [])),
    params(("org_id" = String, Path, description = "Id da org")),
    request_body = CreateCategory,
    responses(
        (status = 201, description = "Categoria criada", body = CategoryJson),
        (status = 400, description = "Campo inválido", body = crate::error::ErrorResponse),
        (status = 404, description = "Org não encontrada", body = crate::error::ErrorResponse)
    )
)]
pub async fn create(
    State(state): State<ServerState>,
    identity: Identity,
    Path(path): Path<OrgPath>,
    Json(body): Json<CreateCategory>,
) -> Result<(StatusCode, Json<CategoryJson>), ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    identity.require_org(org)?;
    let name = validate_name(&body.name)?;
    let description = body.description.unwrap_or_default();
    let category = state
        .store
        .categories()
        .create(org, NewCategory { name, description })?;
    super::audit::register(
        &state,
        &identity,
        Some(org),
        "create",
        "category",
        category.id.to_string(),
    )?;
    Ok((
        StatusCode::CREATED,
        Json(CategoryJson::from_category(&category)),
    ))
}

#[doc = " `GET /admin/v1/orgs/{org_id}/categories/{category_id}`."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/categories/{category_id}",
    tag = "categories",
    operation_id = "categorias_mostrar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path, description = "Id da org"),
        ("category_id" = String, Path, description = "Id da categoria")
    ),
    responses(
        (status = 200, description = "Categoria", body = CategoryJson),
        (status = 404, description = "Categoria não encontrada", body = crate::error::ErrorResponse)
    )
)]
pub async fn show(
    State(state): State<ServerState>,
    identity: Identity,
    Path(path): Path<CategoryPath>,
) -> Result<Json<CategoryJson>, ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    let id = parse_path_id::<ProductCategoryId>(&path.category_id)?;
    identity.require_org(org)?;
    let category = state.store.categories().get(org, id)?;
    Ok(Json(CategoryJson::from_category(&category)))
}

#[doc = " `PATCH /admin/v1/orgs/{org_id}/categories/{category_id}` (exige `If-Match`)."]
#[utoipa::path(
    patch,
    path = "/admin/v1/orgs/{org_id}/categories/{category_id}",
    tag = "categories",
    operation_id = "categorias_atualizar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path, description = "Id da org"),
        ("category_id" = String, Path, description = "Id da categoria"),
        ("If-Match" = String, Header, description = "Versão lida antes")
    ),
    request_body = UpdateCategory,
    responses(
        (status = 200, description = "Categoria alterada", body = CategoryJson),
        (status = 412, description = "Versão desatualizada", body = crate::error::ErrorResponse),
        (status = 428, description = "Faltou If-Match", body = crate::error::ErrorResponse)
    )
)]
pub async fn update(
    State(state): State<ServerState>,
    identity: Identity,
    Path(path): Path<CategoryPath>,
    headers: HeaderMap,
    Json(body): Json<UpdateCategory>,
) -> Result<Json<CategoryJson>, ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    let id = parse_path_id::<ProductCategoryId>(&path.category_id)?;
    identity.require_org(org)?;
    let expected = if_match_version(&headers, "categoria")?;
    let status = match &body.status {
        Some(text) => Some(request_status(text)?),
        None => None,
    };
    let change = CategoryChange {
        name: body.name.as_deref().map(validate_name).transpose()?,
        description: body.description,
        status,
    };
    let category = state.store.categories().update(org, id, expected, change)?;
    super::audit::register(
        &state,
        &identity,
        Some(org),
        "update",
        "category",
        id.to_string(),
    )?;
    Ok(Json(CategoryJson::from_category(&category)))
}

#[doc = " Parâmetros de exclusão de categoria."]
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[schema(as = ExclusaoCategoria)]
pub struct ExclusionCategory {
    #[doc = " Confirma a exclusão de uma categoria com produtos ou agentes vinculados."]
    pub force: Option<bool>,
}

fn count_links(
    state: &ServerState,
    org: OrgId,
    id: ProductCategoryId,
) -> Result<(u64, u64), ApiError> {
    const BATCH: u32 = 200;
    let mut products = 0u64;
    let mut after = None;
    loop {
        let batch = state.store.products().list(
            org,
            Page {
                after,
                limit: BATCH,
            },
        )?;
        products += batch.iter().filter(|p| p.category_id == id).count() as u64;
        if batch.len() < BATCH as usize {
            break;
        }
        after = batch.last().map(|p| p.id.as_uuid());
    }
    let mut agents = 0u64;
    let mut after = None;
    loop {
        let batch = state.store.agents().list(
            org,
            Page {
                after,
                limit: BATCH,
            },
        )?;
        for agent in &batch {
            let used = state.store.agents().categories(org, agent.id)?;
            if used.contains(&id) {
                agents += 1;
            }
        }
        if batch.len() < BATCH as usize {
            break;
        }
        after = batch.last().map(|a| a.id.as_uuid());
    }
    Ok((products, agents))
}

#[doc = " `DELETE /admin/v1/orgs/{org_id}/categories/{category_id}`."]
#[doc = ""]
#[doc = " Sem `force`, uma categoria com produtos ou agentes vinculados responde 409 e nada é apagado."]
#[doc = " Sem vínculo, apaga como antes. Com `force=true`, apaga a categoria, os produtos dela e os"]
#[doc = " vínculos com agentes."]
#[utoipa::path(
    delete,
    path = "/admin/v1/orgs/{org_id}/categories/{category_id}",
    tag = "categories",
    operation_id = "categorias_excluir",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path, description = "Id da org"),
        ("category_id" = String, Path, description = "Id da categoria"),
        ("force" = Option<bool>, Query, description = "Confirma a exclusão de categoria com produtos ou agentes vinculados (apaga os produtos dela)")
    ),
    responses(
        (status = 204, description = "Categoria excluída"),
        (status = 404, description = "Categoria não encontrada", body = crate::error::ErrorResponse),
        (status = 409, description = "Categoria tem produtos ou agentes vinculados e faltou force", body = crate::error::ErrorResponse)
    )
)]
pub async fn delete(
    State(state): State<ServerState>,
    identity: Identity,
    Path(path): Path<CategoryPath>,
    Query(deletion): Query<ExclusionCategory>,
) -> Result<StatusCode, ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    let id = parse_path_id::<ProductCategoryId>(&path.category_id)?;
    identity.require_org(org)?;
    let forced = deletion.force.unwrap_or(false);
    if !forced {
        let (products, agents) = count_links(&state, org, id)?;
        if products > 0 || agents > 0 {
            return Err(ApiError::conflict(format!(
                "a categoria tem {products} produto(s) e {agents} agente(s) vinculados; \
                 excluir apaga os produtos dela e tira a categoria dos agentes. \
                 Para confirmar, repita com force=true"
            ))
            .with_details(json!({ "produtos": products, "agentes": agents })));
        }
    }
    state.store.categories().delete(org, id)?;
    super::audit::register(
        &state,
        &identity,
        Some(org),
        if forced { "delete_force" } else { "delete" },
        "category",
        id.to_string(),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

fn validate_name(text: &str) -> Result<String, ApiError> {
    let clean = text.trim();
    if clean.is_empty() || clean.chars().count() > 120 {
        return Err(ApiError::validation(
            "name",
            "deve ter de 1 a 120 caracteres",
        ));
    }
    Ok(clean.to_owned())
}
