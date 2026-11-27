use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use poph_store::{Departamento, DepartmentChange, NewDepartment};
use poph_types::{DepartmentId, OrgId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::orgs::OrgPath;
use super::pagination::{ListJson, PageQuery, next_cursor};
use super::{if_match_version, request_status, status_text};
use crate::auth::Identidade;
use crate::erro::{ApiError, id_da_url as parse_path_id};
use crate::estado::Estado;

#[doc = " Departamento na resposta da API."]
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = DepartamentoJson)]
pub struct DepartmentJson {
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

impl DepartmentJson {
    fn from_department(department: &Departamento) -> Self {
        Self {
            id: department.id.to_string(),
            org_id: department.org_id.to_string(),
            name: department.name.clone(),
            description: department.description.clone(),
            status: status_text(department.status).to_owned(),
            version: department.version,
            created_at: department.created_at,
            updated_at: department.updated_at,
        }
    }
}

#[doc = " Corpo de criação de departamento."]
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[schema(as = CriarDepartamento)]
pub struct CreateDepartment {
    #[doc = " Nome (único na org, sem diferenciar maiúsculas)."]
    pub name: String,
    #[doc = " Descrição."]
    pub description: Option<String>,
}

#[doc = " Corpo de alteração de departamento."]
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[schema(as = AtualizarDepartamento)]
pub struct UpdateDepartment {
    #[doc = " Novo nome."]
    pub name: Option<String>,
    #[doc = " Nova descrição."]
    pub description: Option<String>,
    #[doc = " Nova situação."]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DepartmentPath {
    pub org_id: String,
    pub department_id: String,
}

#[doc = " `GET /admin/v1/orgs/{org_id}/departments`."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/departments",
    tag = "departments",
    operation_id = "departamentos_listar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path, description = "Id da org"),
        ("limit" = Option<u32>, Query, description = "Registros por página (1 a 200)"),
        ("cursor" = Option<String>, Query, description = "Último id da página anterior")
    ),
    responses(
        (status = 200, description = "Página de departamentos", body = ListJson<DepartmentJson>),
        (status = 403, description = "Org de outro token", body = crate::erro::RespostaErro)
    )
)]
pub async fn list(
    State(state): State<Estado>,
    identity: Identidade,
    Path(path): Path<OrgPath>,
    Query(query): Query<PageQuery>,
) -> Result<Json<ListJson<DepartmentJson>>, ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    identity.exigir_org(org)?;
    let page = query.page()?;
    let departments = state.store.departments().list(org, page)?;
    let items: Vec<DepartmentJson> = departments
        .iter()
        .map(DepartmentJson::from_department)
        .collect();
    let next_cursor = next_cursor(&items, page.limit, |item| item.id.clone());
    Ok(Json(ListJson { items, next_cursor }))
}

#[doc = " `POST /admin/v1/orgs/{org_id}/departments`."]
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/departments",
    tag = "departments",
    operation_id = "departamentos_criar",
    security(("bearer" = [])),
    params(("org_id" = String, Path, description = "Id da org")),
    request_body = CreateDepartment,
    responses(
        (status = 201, description = "Departamento criado", body = DepartmentJson),
        (status = 400, description = "Campo inválido", body = crate::erro::RespostaErro)
    )
)]
pub async fn create(
    State(state): State<Estado>,
    identity: Identidade,
    Path(path): Path<OrgPath>,
    Json(body): Json<CreateDepartment>,
) -> Result<(StatusCode, Json<DepartmentJson>), ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    identity.exigir_org(org)?;
    let name = validate_name(&body.name)?;
    let department = state.store.departments().create(
        org,
        NewDepartment {
            name,
            description: body.description.unwrap_or_default(),
        },
    )?;
    super::audit::register(
        &state,
        &identity,
        Some(org),
        "create",
        "department",
        department.id.to_string(),
    )?;
    Ok((
        StatusCode::CREATED,
        Json(DepartmentJson::from_department(&department)),
    ))
}

#[doc = " `GET /admin/v1/orgs/{org_id}/departments/{department_id}`."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/departments/{department_id}",
    tag = "departments",
    operation_id = "departamentos_mostrar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path, description = "Id da org"),
        ("department_id" = String, Path, description = "Id do departamento")
    ),
    responses(
        (status = 200, description = "Departamento", body = DepartmentJson),
        (status = 404, description = "Departamento não encontrado", body = crate::erro::RespostaErro)
    )
)]
pub async fn show(
    State(state): State<Estado>,
    identity: Identidade,
    Path(path): Path<DepartmentPath>,
) -> Result<Json<DepartmentJson>, ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    let id = parse_path_id::<DepartmentId>(&path.department_id)?;
    identity.exigir_org(org)?;
    let department = state.store.departments().get(org, id)?;
    Ok(Json(DepartmentJson::from_department(&department)))
}

#[doc = " `PATCH /admin/v1/orgs/{org_id}/departments/{department_id}` (exige `If-Match`)."]
#[utoipa::path(
    patch,
    path = "/admin/v1/orgs/{org_id}/departments/{department_id}",
    tag = "departments",
    operation_id = "departamentos_atualizar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path, description = "Id da org"),
        ("department_id" = String, Path, description = "Id do departamento"),
        ("If-Match" = String, Header, description = "Versão lida antes")
    ),
    request_body = UpdateDepartment,
    responses(
        (status = 200, description = "Departamento alterado", body = DepartmentJson),
        (status = 412, description = "Versão desatualizada", body = crate::erro::RespostaErro)
    )
)]
pub async fn update(
    State(state): State<Estado>,
    identity: Identidade,
    Path(path): Path<DepartmentPath>,
    headers: HeaderMap,
    Json(body): Json<UpdateDepartment>,
) -> Result<Json<DepartmentJson>, ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    let id = parse_path_id::<DepartmentId>(&path.department_id)?;
    identity.exigir_org(org)?;
    let expected = if_match_version(&headers, "departamento")?;
    let status = match &body.status {
        Some(text) => Some(request_status(text)?),
        None => None,
    };
    let change = DepartmentChange {
        name: body.name.as_deref().map(validate_name).transpose()?,
        description: body.description,
        status,
    };
    let department = state
        .store
        .departments()
        .update(org, id, expected, change)?;
    super::audit::register(
        &state,
        &identity,
        Some(org),
        "update",
        "department",
        id.to_string(),
    )?;
    Ok(Json(DepartmentJson::from_department(&department)))
}

#[doc = " `DELETE /admin/v1/orgs/{org_id}/departments/{department_id}`."]
#[utoipa::path(
    delete,
    path = "/admin/v1/orgs/{org_id}/departments/{department_id}",
    tag = "departments",
    operation_id = "departamentos_excluir",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path, description = "Id da org"),
        ("department_id" = String, Path, description = "Id do departamento")
    ),
    responses(
        (status = 204, description = "Departamento excluído"),
        (status = 404, description = "Departamento não encontrado", body = crate::erro::RespostaErro)
    )
)]
pub async fn delete(
    State(state): State<Estado>,
    identity: Identidade,
    Path(path): Path<DepartmentPath>,
) -> Result<StatusCode, ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    let id = parse_path_id::<DepartmentId>(&path.department_id)?;
    identity.exigir_org(org)?;
    state.store.departments().delete(org, id)?;
    super::audit::register(
        &state,
        &identity,
        Some(org),
        "delete",
        "department",
        id.to_string(),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

fn validate_name(text: &str) -> Result<String, ApiError> {
    let clean = text.trim();
    if clean.is_empty() || clean.chars().count() > 120 {
        return Err(ApiError::validacao(
            "name",
            "deve ter de 1 a 120 caracteres",
        ));
    }
    Ok(clean.to_owned())
}
