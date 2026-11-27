use std::fmt::Write as _;

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use poph_store::{Page, TokenAdmin};
use poph_types::{AdminTokenId, OrgId};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::pagination::{ListJson, PageQuery, next_cursor};
use crate::auth::Identity;
use crate::auth::ORG_PREFIX;
use crate::error::{ApiError, id_from_url as parse_path_id};
use crate::estado::Estado;

#[doc = " Token na resposta da API (nunca traz o segredo)."]
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TokenJson {
    #[doc = " Id (UUID v7)."]
    pub id: String,
    #[doc = " Org dona (`null` para token global)."]
    pub org_id: Option<String>,
    #[doc = " Rótulo."]
    pub label: String,
    #[doc = " `active` ou `revoked`."]
    pub status: String,
    #[doc = " Criação (ms)."]
    pub created_at: i64,
    #[doc = " Revogação (ms), quando houve."]
    pub revoked_at: Option<i64>,
}

impl TokenJson {
    fn de(token: &TokenAdmin) -> Self {
        Self {
            id: token.id.to_string(),
            org_id: token.org_id.map(|org| org.to_string()),
            label: token.label.clone(),
            status: if token.revoked_at.is_some() {
                "revoked".to_owned()
            } else {
                "active".to_owned()
            },
            created_at: token.created_at,
            revoked_at: token.revoked_at,
        }
    }
}

#[doc = " Corpo de criação de token."]
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[schema(as = CriarToken)]
pub struct CreateToken {
    #[doc = " Rótulo para reconhecer o token (1 a 80 caracteres)."]
    pub label: String,
}

#[doc = " Resposta da criação: o token e o valor em claro (só aparece aqui)."]
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = TokenCriadoJson)]
pub struct TokenCreatedJson {
    #[doc = " O registro criado (sem segredo)."]
    pub token: TokenJson,
    #[doc = " Valor do token, no formato `poph_<uuid>_<segredo>`. **Só aparece nesta resposta.**"]
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenPath {
    pub org_id: String,
    pub token_id: String,
}

#[doc = " `GET /admin/v1/orgs/{org_id}/tokens`."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/tokens",
    tag = "tokens",
    operation_id = "tokens_listar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path, description = "Id da org"),
        ("limit" = Option<u32>, Query, description = "Registros por página (1 a 200)"),
        ("cursor" = Option<String>, Query, description = "Último id da página anterior")
    ),
    responses(
        (status = 200, description = "Página de tokens", body = ListJson<TokenJson>),
        (status = 403, description = "Org de outro token", body = crate::error::ErrorResponse)
    )
)]
pub async fn list(
    State(state): State<Estado>,
    identity: Identity,
    Path(path): Path<super::orgs::OrgPath>,
    Query(query): Query<PageQuery>,
) -> Result<Json<ListJson<TokenJson>>, ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    identity.require_org(org)?;
    let page = query.page()?;
    let tokens = state.store.tokens().list(org, page)?;
    let items: Vec<TokenJson> = tokens.iter().map(TokenJson::de).collect();
    let next_cursor = next_cursor(&items, page.limit, |item| item.id.clone());
    Ok(Json(ListJson { items, next_cursor }))
}

#[doc = " `POST /admin/v1/orgs/{org_id}/tokens` (só token global)."]
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/tokens",
    tag = "tokens",
    operation_id = "tokens_criar",
    security(("bearer" = [])),
    params(("org_id" = String, Path, description = "Id da org")),
    request_body = CreateToken,
    responses(
        (status = 201, description = "Token criado (valor aparece só aqui)", body = TokenCreatedJson),
        (status = 403, description = "Só token global cria token", body = crate::error::ErrorResponse),
        (status = 404, description = "Org não encontrada", body = crate::error::ErrorResponse)
    )
)]
pub async fn create(
    State(state): State<Estado>,
    identity: Identity,
    Path(path): Path<super::orgs::OrgPath>,
    Json(body): Json<CreateToken>,
) -> Result<(StatusCode, Json<TokenCreatedJson>), ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    identity.require_global()?;
    let label = body.label.trim();
    if label.is_empty() || label.chars().count() > 80 {
        return Err(ApiError::validation(
            "label",
            "deve ter de 1 a 80 caracteres",
        ));
    }
    state.store.orgs().get(org)?;
    let secret = generate_secret();
    let hash = generate_hash(&secret)?;
    let token = state.store.tokens().create(Some(org), label, &hash)?;
    super::audit::register(
        &state,
        &identity,
        Some(org),
        "create",
        "token",
        token.id.to_string(),
    )?;
    let value = format!("{ORG_PREFIX}{}_{secret}", token.id);
    Ok((
        StatusCode::CREATED,
        Json(TokenCreatedJson {
            token: TokenJson::de(&token),
            value,
        }),
    ))
}

#[doc = " `GET /admin/v1/orgs/{org_id}/tokens/{token_id}`."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/tokens/{token_id}",
    tag = "tokens",
    operation_id = "tokens_mostrar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path, description = "Id da org"),
        ("token_id" = String, Path, description = "Id do token")
    ),
    responses(
        (status = 200, description = "Token", body = TokenJson),
        (status = 404, description = "Token não encontrado", body = crate::error::ErrorResponse)
    )
)]
pub async fn show(
    State(state): State<Estado>,
    identity: Identity,
    Path(path): Path<TokenPath>,
) -> Result<Json<TokenJson>, ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    let token = parse_path_id::<AdminTokenId>(&path.token_id)?;
    identity.require_org(org)?;
    let record = state.store.tokens().get(org, token)?;
    Ok(Json(TokenJson::de(&record)))
}

#[doc = " `DELETE /admin/v1/orgs/{org_id}/tokens/{token_id}` — revoga (só token global)."]
#[utoipa::path(
    delete,
    path = "/admin/v1/orgs/{org_id}/tokens/{token_id}",
    tag = "tokens",
    operation_id = "tokens_revogar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path, description = "Id da org"),
        ("token_id" = String, Path, description = "Id do token")
    ),
    responses(
        (status = 204, description = "Token revogado"),
        (status = 403, description = "Só token global revoga", body = crate::error::ErrorResponse)
    )
)]
pub async fn revoke(
    State(state): State<Estado>,
    identity: Identity,
    Path(path): Path<TokenPath>,
) -> Result<StatusCode, ApiError> {
    let org = parse_path_id::<OrgId>(&path.org_id)?;
    let token = parse_path_id::<AdminTokenId>(&path.token_id)?;
    identity.require_global()?;
    state.store.tokens().get(org, token)?;
    state.store.tokens().revoke(org, token)?;
    super::audit::register(
        &state,
        &identity,
        Some(org),
        "revoke",
        "token",
        token.to_string(),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

fn generate_secret() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let mut secret = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(secret, "{byte:02x}");
    }
    secret
}

fn generate_hash(value: &str) -> Result<String, ApiError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| {
            tracing::error!(error = %error, "falha ao gerar o hash do token");
            ApiError::internal()
        })
}

#[allow(dead_code)]
pub(crate) fn default_page() -> Page {
    Page::default()
}
