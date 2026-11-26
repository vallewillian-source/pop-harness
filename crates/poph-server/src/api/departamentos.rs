//! Departamentos: CRUD em `/admin/v1/orgs/{org_id}/departments`.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use poph_store::{Departamento, DepartmentChange, NewDepartment};
use poph_types::{DepartmentId, OrgId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::orgs::CaminhoOrg;
use super::paginacao::{ConsultaPagina, ListaJson, proximo_cursor};
use super::{status_pedido, status_texto, versao_do_if_match};
use crate::auth::Identidade;
use crate::erro::{ApiError, id_da_url};
use crate::estado::Estado;

/// Departamento na resposta da API.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DepartamentoJson {
    /// Id (UUID v7).
    pub id: String,
    /// Org dona.
    pub org_id: String,
    /// Nome.
    pub name: String,
    /// Descrição.
    pub description: String,
    /// `active`, `inactive` ou `archived`.
    pub status: String,
    /// Versão para `If-Match`.
    pub version: u64,
    /// Criação (ms).
    pub created_at: i64,
    /// Última alteração (ms).
    pub updated_at: i64,
}

impl DepartamentoJson {
    fn de(departamento: &Departamento) -> Self {
        Self {
            id: departamento.id.to_string(),
            org_id: departamento.org_id.to_string(),
            name: departamento.name.clone(),
            description: departamento.description.clone(),
            status: status_texto(departamento.status).to_owned(),
            version: departamento.version,
            created_at: departamento.created_at,
            updated_at: departamento.updated_at,
        }
    }
}

/// Corpo de criação de departamento.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CriarDepartamento {
    /// Nome (único na org, sem diferenciar maiúsculas).
    pub name: String,
    /// Descrição.
    pub description: Option<String>,
}

/// Corpo de alteração de departamento.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct AtualizarDepartamento {
    /// Novo nome.
    pub name: Option<String>,
    /// Nova descrição.
    pub description: Option<String>,
    /// Nova situação.
    pub status: Option<String>,
}

/// Caminho de um departamento.
#[derive(Debug, Clone, Deserialize)]
pub struct CaminhoDepartamento {
    /// Id da org.
    pub org_id: String,
    /// Id do departamento.
    pub department_id: String,
}

/// `GET /admin/v1/orgs/{org_id}/departments`.
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
        (status = 200, description = "Página de departamentos", body = ListaJson<DepartamentoJson>),
        (status = 403, description = "Org de outro token", body = crate::erro::RespostaErro)
    )
)]
pub async fn listar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoOrg>,
    Query(consulta): Query<ConsultaPagina>,
) -> Result<Json<ListaJson<DepartamentoJson>>, ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    identidade.exigir_org(org)?;
    let pagina = consulta.pagina()?;
    let departamentos = estado.store.departments().listar(org, pagina)?;
    let items: Vec<DepartamentoJson> = departamentos.iter().map(DepartamentoJson::de).collect();
    let next_cursor = proximo_cursor(&items, pagina.limit, |item| item.id.clone());
    Ok(Json(ListaJson { items, next_cursor }))
}

/// `POST /admin/v1/orgs/{org_id}/departments`.
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/departments",
    tag = "departments",
    operation_id = "departamentos_criar",
    security(("bearer" = [])),
    params(("org_id" = String, Path, description = "Id da org")),
    request_body = CriarDepartamento,
    responses(
        (status = 201, description = "Departamento criado", body = DepartamentoJson),
        (status = 400, description = "Campo inválido", body = crate::erro::RespostaErro)
    )
)]
pub async fn criar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoOrg>,
    Json(corpo): Json<CriarDepartamento>,
) -> Result<(StatusCode, Json<DepartamentoJson>), ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    identidade.exigir_org(org)?;
    let nome = validar_nome(&corpo.name)?;
    let departamento = estado.store.departments().criar(
        org,
        NewDepartment {
            name: nome,
            description: corpo.description.unwrap_or_default(),
        },
    )?;
    super::auditoria::registrar(
        &estado,
        &identidade,
        Some(org),
        "create",
        "department",
        departamento.id.to_string(),
    )?;
    Ok((
        StatusCode::CREATED,
        Json(DepartamentoJson::de(&departamento)),
    ))
}

/// `GET /admin/v1/orgs/{org_id}/departments/{department_id}`.
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
        (status = 200, description = "Departamento", body = DepartamentoJson),
        (status = 404, description = "Departamento não encontrado", body = crate::erro::RespostaErro)
    )
)]
pub async fn mostrar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoDepartamento>,
) -> Result<Json<DepartamentoJson>, ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    let id = id_da_url::<DepartmentId>(&caminho.department_id)?;
    identidade.exigir_org(org)?;
    let departamento = estado.store.departments().obter(org, id)?;
    Ok(Json(DepartamentoJson::de(&departamento)))
}

/// `PATCH /admin/v1/orgs/{org_id}/departments/{department_id}` (exige `If-Match`).
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
    request_body = AtualizarDepartamento,
    responses(
        (status = 200, description = "Departamento alterado", body = DepartamentoJson),
        (status = 412, description = "Versão desatualizada", body = crate::erro::RespostaErro)
    )
)]
pub async fn atualizar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoDepartamento>,
    headers: HeaderMap,
    Json(corpo): Json<AtualizarDepartamento>,
) -> Result<Json<DepartamentoJson>, ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    let id = id_da_url::<DepartmentId>(&caminho.department_id)?;
    identidade.exigir_org(org)?;
    let esperada = versao_do_if_match(&headers, "departamento")?;
    let status = match &corpo.status {
        Some(texto) => Some(status_pedido(texto)?),
        None => None,
    };
    let mudanca = DepartmentChange {
        name: corpo.name.as_deref().map(validar_nome).transpose()?,
        description: corpo.description,
        status,
    };
    let departamento = estado
        .store
        .departments()
        .atualizar(org, id, esperada, mudanca)?;
    super::auditoria::registrar(
        &estado,
        &identidade,
        Some(org),
        "update",
        "department",
        id.to_string(),
    )?;
    Ok(Json(DepartamentoJson::de(&departamento)))
}

/// `DELETE /admin/v1/orgs/{org_id}/departments/{department_id}`.
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
pub async fn excluir(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoDepartamento>,
) -> Result<StatusCode, ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    let id = id_da_url::<DepartmentId>(&caminho.department_id)?;
    identidade.exigir_org(org)?;
    estado.store.departments().excluir(org, id)?;
    super::auditoria::registrar(
        &estado,
        &identidade,
        Some(org),
        "delete",
        "department",
        id.to_string(),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

fn validar_nome(texto: &str) -> Result<String, ApiError> {
    let limpo = texto.trim();
    if limpo.is_empty() || limpo.chars().count() > 120 {
        return Err(ApiError::validacao(
            "name",
            "deve ter de 1 a 120 caracteres",
        ));
    }
    Ok(limpo.to_owned())
}
