use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use poph_matrix::spaces::{DepartmentSpace, SpaceError};
use poph_types::{DepartmentId, OrgId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::if_match_version;
use super::orgs::OrgPath;
use crate::auth::Identity;
use crate::error::{ApiError, id_from_url as parse_path_id};
use crate::estado::Estado;

#[doc = " Como o módulo está numa org."]
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = EstadoMatrixJson, description = "Como o módulo está numa org.")]
pub struct MatrixStateJson {
    #[doc = " Org dona."]
    pub org_id: String,
    #[doc = " Módulo ligado nesta org (`matrix_orgs.enabled`)."]
    pub enabled: bool,
    #[doc = " Corte do backfill (ms): só o que foi gravado depois disto é espelhado. `null` quando a org"]
    #[doc = " nunca ligou o módulo."]
    pub enabled_at: Option<i64>,
    #[doc = " Última alteração da linha (ms) — é o valor que vai no `If-Match` do `PUT`."]
    pub updated_at: i64,
    #[doc = " O módulo desta instância está falando com o Synapse? `null` = módulo não configurado aqui."]
    pub connected: Option<bool>,
    #[doc = " O que ainda falta espelhar nesta org."]
    pub backlog: DelayJson,
    #[doc = " Quantas mensagens estão isoladas em `matrix_falhas`."]
    pub pending_failures: u64,
    #[doc = " Motivo curto da falha mais recente (nunca texto de cliente), ou `null`."]
    pub last_failure: Option<String>,
}

#[doc = " Atraso do cursor de mensagens da org."]
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = AtrasoJson, description = "Atraso do cursor de mensagens da org.")]
pub struct DelayJson {
    #[doc = " Mensagens gravadas depois do cursor e ainda não espelhadas."]
    pub messages: u64,
    #[doc = " Idade, em segundos, da mais antiga delas (`0` quando não há nenhuma pendente)."]
    pub oldest_seconds: u64,
}

#[doc = " Espaço de um departamento."]
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = EspacoJson, description = "Espaço de um departamento.")]
pub struct SpaceJson {
    #[doc = " Departamento dono do espaço."]
    pub department_id: String,
    #[doc = " Nome do departamento (o mesmo que o espaço mostra), quando ele ainda existe."]
    pub department_name: Option<String>,
    #[doc = " Id do espaço no Synapse (`!…:servidor`)."]
    pub space_room_id: String,
    #[doc = " Alias do departamento no módulo (`#dep_<id>:<servidor>`). Num espaço ligado a um que já"]
    #[doc = " existia, é o nome do departamento para o módulo — quem deu alias no Element foi o operador."]
    pub alias: String,
    #[doc = " `auto` (o módulo criou) ou `linked` (já existia no Element)."]
    pub origin: String,
    #[doc = " Criação da linha (ms)."]
    pub created_at: i64,
    #[doc = " Última alteração da linha (ms)."]
    pub updated_at: i64,
}

#[doc = " Lista de espaços da org."]
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = EspacosJson, description = "Lista de espaços da org.")]
pub struct SpacesJson {
    #[doc = " Espaços, na ordem de criação."]
    pub items: Vec<SpaceJson>,
}

#[doc = " Corpo do `PUT …/matrix`."]
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[schema(as = LigarDesligar, description = "Corpo do `PUT …/matrix`.")]
pub struct EnableDisable {
    #[doc = " `true` liga o módulo na org (grava o corte do backfill); `false` desliga."]
    pub enabled: bool,
}

#[doc = " Corpo do `PUT …/matrix/spaces`."]
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[schema(as = LigarEspaco, description = "Corpo do `PUT …/matrix/spaces`.")]
pub struct EnableSpace {
    #[doc = " Departamento dono do espaço (id UUID v7 da própria org)."]
    pub department_id: String,
    #[doc = " Espaço que já existe no Element: id `!…` ou alias `#…`. Ausente = o módulo cria o espaço"]
    #[doc = " pelo alias do departamento."]
    pub space: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpacePath {
    org_id: String,
    department_id: String,
}

#[doc = " Resposta do `DELETE …/matrix/spaces/{department}`."]
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = EspacoDesligadoJson, description = "Resposta do `DELETE …/matrix/spaces/{department}`.")]
pub struct SpaceDisabledJson {
    #[doc = " Departamento do espaço desligado."]
    pub department_id: String,
    #[doc = " Havia espaço gravado e a linha saiu."]
    pub removed: bool,
    #[doc = " Salas que deixaram de estar registradas naquele espaço."]
    pub rooms_forgotten: u64,
}

#[doc = " `GET /admin/v1/orgs/{org_id}/matrix`."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/matrix",
    tag = "matrix",
    operation_id = "matrix_status",
    security(("bearer" = [])),
    params(("org_id" = String, Path, description = "Id da org")),
    responses(
        (status = 200, description = "Estado do módulo Matrix na org", body = MatrixStateJson),
        (status = 401, description = "Sem token", body = crate::error::ErrorResponse),
        (status = 403, description = "Token de outra org", body = crate::error::ErrorResponse),
        (status = 404, description = "Org inexistente", body = crate::error::ErrorResponse)
    )
,
    summary = "`GET /admin/v1/orgs/{org_id}/matrix`.",
)]
pub async fn show(
    State(state): State<Estado>,
    identity: Identity,
    Path(path): Path<OrgPath>,
) -> Result<Response, ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    identity.require_org(org)?;
    org_state(&state, org)
}

#[doc = " `PUT /admin/v1/orgs/{org_id}/matrix` (exige `If-Match` com o `updated_at` lido antes)."]
#[utoipa::path(
    put,
    path = "/admin/v1/orgs/{org_id}/matrix",
    tag = "matrix",
    operation_id = "matrix_atualizar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path, description = "Id da org"),
        ("If-Match" = String, Header, description = "`updated_at` lido antes no GET")
    ),
    request_body = EnableDisable,
    responses(
        (status = 200, description = "Estado do módulo depois da alteração", body = MatrixStateJson),
        (status = 401, description = "Sem token", body = crate::error::ErrorResponse),
        (status = 403, description = "Token de outra org", body = crate::error::ErrorResponse),
        (status = 404, description = "Org inexistente", body = crate::error::ErrorResponse),
        (status = 412, description = "`If-Match` desatualizado", body = crate::error::ErrorResponse),
        (status = 428, description = "Sem o cabeçalho `If-Match`", body = crate::error::ErrorResponse)
    )
,
    summary = "`PUT /admin/v1/orgs/{org_id}/matrix` (exige `If-Match` com o `updated_at` lido antes).",
)]
pub async fn update(
    State(state): State<Estado>,
    identity: Identity,
    Path(path): Path<OrgPath>,
    headers: HeaderMap,
    Json(body): Json<EnableDisable>,
) -> Result<Response, ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    identity.require_org(org)?;
    state.store.orgs().get(org)?;
    let expected = if_match_version(&headers, "módulo Matrix da org")?;
    let current = state.store.matrix().state(org)?;
    let current_etag = u64::try_from(current.updated_at).unwrap_or(0);
    if expected != current_etag {
        return Err(ApiError::version_conflict(
            "módulo Matrix da org",
            &org.to_string(),
            expected,
            current_etag,
        ));
    }
    if body.enabled {
        state.store.matrix().enable(org)?;
    } else {
        state.store.matrix().disable(org)?;
    }
    super::audit::register(
        &state,
        &identity,
        Some(org),
        "update",
        "matrix",
        org.to_string(),
    )?;
    org_state(&state, org)
}

fn org_state(state: &Estado, org: OrgId) -> Result<Response, ApiError> {
    state.store.orgs().get(org)?;
    let line = state.store.matrix().state(org)?;
    let cursors = state.store.matrix().read_cursors(org)?;
    let delay = state
        .store
        .matrix()
        .delay_of_messages(org, cursors.messages)?;
    let now = state.store.now();
    let age_second = delay
        .oldest_at
        .map(|created_at| u64::try_from((now - created_at).max(0) / 1000).unwrap_or(0))
        .unwrap_or(0);
    let failure = state.store.matrix().count_failures(org)?;
    let last_failure = state
        .store
        .matrix()
        .list_failures(org, 1)?
        .first()
        .map(|failure| failure.motivo.clone());
    let body = MatrixStateJson {
        org_id: org.to_string(),
        enabled: line.linked,
        enabled_at: line.linked_at,
        updated_at: line.updated_at,
        connected: state.matrix_conectado(),
        backlog: DelayJson {
            messages: delay.pending,
            oldest_seconds: age_second,
        },
        pending_failures: failure,
        last_failure,
    };
    with_etag(body, line.updated_at)
}

fn with_etag(body: MatrixStateJson, etag: i64) -> Result<Response, ApiError> {
    let value = HeaderValue::from_str(&format!("\"{etag}\"")).map_err(|_| ApiError::internal())?;
    let mut response = Json(body).into_response();
    response.headers_mut().insert(header::ETAG, value);
    Ok(response)
}

#[doc = " `GET /admin/v1/orgs/{org_id}/matrix/spaces`."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/matrix/spaces",
    tag = "matrix",
    operation_id = "matrix_espacos_listar",
    security(("bearer" = [])),
    params(("org_id" = String, Path, description = "Id da org")),
    responses(
        (status = 200, description = "Espaços por departamento", body = SpacesJson),
        (status = 401, description = "Sem token", body = crate::error::ErrorResponse),
        (status = 403, description = "Token de outra org", body = crate::error::ErrorResponse),
        (status = 404, description = "Org inexistente", body = crate::error::ErrorResponse)
    )
,
    summary = "`GET /admin/v1/orgs/{org_id}/matrix/spaces`.",
)]
pub async fn list_space(
    State(state): State<Estado>,
    identity: Identity,
    Path(path): Path<OrgPath>,
) -> Result<Json<SpacesJson>, ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    identity.require_org(org)?;
    state.store.orgs().get(org)?;
    let items = state
        .store
        .matrix()
        .list_spaces(org)?
        .iter()
        .map(|space| SpaceJson {
            department_id: space.departamento.to_string(),
            department_name: state
                .store
                .departments()
                .get(org, space.departamento)
                .ok()
                .map(|department| department.name),
            space_room_id: space.room_id.clone(),
            alias: space.alias.clone(),
            origin: space.origin.as_text().to_owned(),
            created_at: space.created_at,
            updated_at: space.updated_at,
        })
        .collect();
    Ok(Json(SpacesJson { items }))
}

#[doc = " `PUT /admin/v1/orgs/{org_id}/matrix/spaces` — cria o espaço do departamento ou liga um existente."]
#[utoipa::path(
    put,
    path = "/admin/v1/orgs/{org_id}/matrix/spaces",
    tag = "matrix",
    operation_id = "matrix_espacos_ligar",
    security(("bearer" = [])),
    params(("org_id" = String, Path, description = "Id da org")),
    request_body = EnableSpace,
    responses(
        (status = 200, description = "Espaço do departamento", body = SpaceJson),
        (status = 400, description = "Departamento ou espaço inválido", body = crate::error::ErrorResponse),
        (status = 401, description = "Sem token", body = crate::error::ErrorResponse),
        (status = 403, description = "Token de outra org", body = crate::error::ErrorResponse),
        (status = 404, description = "Org ou departamento inexistente", body = crate::error::ErrorResponse),
        (status = 409, description = "Sem permissão no espaço, ou módulo desligado nesta instância", body = crate::error::ErrorResponse),
        (status = 503, description = "Synapse ou armazenamento não respondeu; tente de novo", body = crate::error::ErrorResponse)
    )
,
    summary = "`PUT /admin/v1/orgs/{org_id}/matrix/spaces` — cria o espaço do departamento ou liga um existente.",
)]
pub async fn enable_space(
    State(state): State<Estado>,
    identity: Identity,
    Path(path): Path<OrgPath>,
    Json(body): Json<EnableSpace>,
) -> Result<Json<SpaceJson>, ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    identity.require_org(org)?;
    state.store.orgs().get(org)?;
    let department_id = parse_path_id::<DepartmentId>(&body.department_id)?;
    let department = state.store.departments().get(org, department_id)?;
    let spaces = state.matrix_espacos().ok_or_else(module_disabled)?;
    let target = DepartmentSpace {
        id: department.id,
        name: department.name.clone(),
    };
    let existing = body
        .space
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let stored = spaces
        .ensure_space(org, &target, existing)
        .await
        .map_err(space_error)?;
    super::audit::register(
        &state,
        &identity,
        Some(org),
        "update",
        "matrix_space",
        department.id.to_string(),
    )?;
    Ok(Json(SpaceJson {
        department_id: department.id.to_string(),
        department_name: Some(department.name),
        space_room_id: stored.room_id,
        alias: stored.alias,
        origin: match stored.origin {
            poph_matrix::spaces::SpaceOrigin::Automatic => "auto".to_owned(),
            poph_matrix::spaces::SpaceOrigin::Linked => "linked".to_owned(),
        },
        created_at: state.store.now(),
        updated_at: state.store.now(),
    }))
}

#[doc = " `DELETE /admin/v1/orgs/{org_id}/matrix/spaces/{department_id}`."]
#[utoipa::path(
    delete,
    path = "/admin/v1/orgs/{org_id}/matrix/spaces/{department_id}",
    tag = "matrix",
    operation_id = "matrix_espacos_desligar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path, description = "Id da org"),
        ("department_id" = String, Path, description = "Id do departamento")
    ),
    responses(
        (status = 200, description = "Espaço desligado", body = SpaceDisabledJson),
        (status = 401, description = "Sem token", body = crate::error::ErrorResponse),
        (status = 403, description = "Token de outra org", body = crate::error::ErrorResponse),
        (status = 404, description = "Org ou departamento inexistente", body = crate::error::ErrorResponse)
    )
,
    summary = "`DELETE /admin/v1/orgs/{org_id}/matrix/spaces/{department_id}`.",
)]
pub async fn disable_space(
    State(state): State<Estado>,
    identity: Identity,
    Path(path): Path<SpacePath>,
) -> Result<Json<SpaceDisabledJson>, ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    identity.require_org(org)?;
    state.store.orgs().get(org)?;
    let department = parse_path_id::<DepartmentId>(&path.department_id)?;
    state.store.departments().get(org, department)?;
    let disabled = state.store.matrix().disable_space(org, department)?;
    super::audit::register(
        &state,
        &identity,
        Some(org),
        "delete",
        "matrix_space",
        department.to_string(),
    )?;
    Ok(Json(SpaceDisabledJson {
        department_id: department.to_string(),
        removed: disabled.removed,
        rooms_forgotten: disabled.rooms_forgotten,
    }))
}

fn module_disabled() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "matrix_desligado",
        "o módulo Matrix não está ligado nesta instância: suba o `poph serve` com `[matrix] habilitado = true`",
    )
}

fn space_error(error: SpaceError) -> ApiError {
    match error {
        SpaceError::WithoutPermission { what_is_missing } => ApiError::conflict(what_is_missing),
        SpaceError::InvalidSpace { reason } => ApiError::validation("space", reason.to_owned()),
        SpaceError::Internal { .. } => {
            tracing::error!(
                target: "poph_server::api::matrix",
                reason = "espaco_interno",
                "falha interna ao garantir o espaço do departamento"
            );
            ApiError::internal()
        }
        SpaceError::Armazenamento(_) => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "matrix_indisponivel",
            "o armazenamento do módulo Matrix não respondeu: tente de novo",
        ),
        SpaceError::Call(call) => match call.classification() {
            poph_matrix::Classification::Transient => ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "synapse_indisponivel",
                "o Synapse não respondeu: tente de novo em instantes",
            ),
            poph_matrix::Classification::Definitive => ApiError::new(
                StatusCode::CONFLICT,
                "synapse_recusou",
                "o Synapse recusou a operação do espaço: confira o espaço informado no Element",
            ),
        },
    }
}
