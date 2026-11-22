//! Tokens de acesso: CRUD em `/admin/v1/orgs/{org_id}/tokens`.
//!
//! O valor em claro aparece **uma única vez**, na criação. Depois disso nem a listagem nem o
//! detalhe devolvem o valor (a API guarda só o hash argon2).

use std::fmt::Write as _;

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use poph_store::{Pagina, TokenAdmin};
use poph_types::{AdminTokenId, OrgId};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::paginacao::{ConsultaPagina, ListaJson, proximo_cursor};
use crate::auth::Identidade;
use crate::auth::PREFIXO_ORG;
use crate::erro::{ApiError, id_da_url};
use crate::estado::Estado;

/// Token na resposta da API (nunca traz o segredo).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TokenJson {
    /// Id (UUID v7).
    pub id: String,
    /// Org dona (`null` para token global).
    pub org_id: Option<String>,
    /// Rótulo.
    pub label: String,
    /// `active` ou `revoked`.
    pub status: String,
    /// Criação (ms).
    pub created_at: i64,
    /// Revogação (ms), quando houve.
    pub revoked_at: Option<i64>,
}

impl TokenJson {
    fn de(token: &TokenAdmin) -> Self {
        Self {
            id: token.id.to_string(),
            org_id: token.org_id.map(|org| org.to_string()),
            label: token.rotulo.clone(),
            status: if token.revogado_em.is_some() {
                "revoked".to_owned()
            } else {
                "active".to_owned()
            },
            created_at: token.criado_em,
            revoked_at: token.revogado_em,
        }
    }
}

/// Corpo de criação de token.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CriarToken {
    /// Rótulo para reconhecer o token (1 a 80 caracteres).
    pub label: String,
}

/// Resposta da criação: o token e o valor em claro (só aparece aqui).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TokenCriadoJson {
    /// O registro criado (sem segredo).
    pub token: TokenJson,
    /// Valor do token, no formato `poph_<uuid>_<segredo>`. **Só aparece nesta resposta.**
    pub value: String,
}

/// Caminho de um token: org + token.
#[derive(Debug, Clone, Deserialize)]
pub struct CaminhoToken {
    /// Id da org.
    pub org_id: String,
    /// Id do token.
    pub token_id: String,
}

/// `GET /admin/v1/orgs/{org_id}/tokens`.
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
        (status = 200, description = "Página de tokens", body = ListaJson<TokenJson>),
        (status = 403, description = "Org de outro token", body = crate::erro::RespostaErro)
    )
)]
pub async fn listar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<super::orgs::CaminhoOrg>,
    Query(consulta): Query<ConsultaPagina>,
) -> Result<Json<ListaJson<TokenJson>>, ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    identidade.exigir_org(org)?;
    let pagina = consulta.pagina()?;
    let tokens = estado.store.tokens().listar(org, pagina)?;
    let items: Vec<TokenJson> = tokens.iter().map(TokenJson::de).collect();
    let next_cursor = proximo_cursor(&items, pagina.limite, |item| item.id.clone());
    Ok(Json(ListaJson { items, next_cursor }))
}

/// `POST /admin/v1/orgs/{org_id}/tokens` (só token global).
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/tokens",
    tag = "tokens",
    operation_id = "tokens_criar",
    security(("bearer" = [])),
    params(("org_id" = String, Path, description = "Id da org")),
    request_body = CriarToken,
    responses(
        (status = 201, description = "Token criado (valor aparece só aqui)", body = TokenCriadoJson),
        (status = 403, description = "Só token global cria token", body = crate::erro::RespostaErro),
        (status = 404, description = "Org não encontrada", body = crate::erro::RespostaErro)
    )
)]
pub async fn criar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<super::orgs::CaminhoOrg>,
    Json(corpo): Json<CriarToken>,
) -> Result<(StatusCode, Json<TokenCriadoJson>), ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    identidade.exigir_global()?;
    let rotulo = corpo.label.trim();
    if rotulo.is_empty() || rotulo.chars().count() > 80 {
        return Err(ApiError::validacao(
            "label",
            "deve ter de 1 a 80 caracteres",
        ));
    }
    // A org precisa existir (erro 404 antes de qualquer gravação).
    estado.store.orgaos().obter(org)?;
    let segredo = gerar_segredo();
    let hash = gerar_hash(&segredo)?;
    let token = estado.store.tokens().criar(Some(org), rotulo, &hash)?;
    super::auditoria::registrar(
        &estado,
        &identidade,
        Some(org),
        "create",
        "token",
        token.id.to_string(),
    )?;
    let value = format!("{PREFIXO_ORG}{}_{segredo}", token.id);
    Ok((
        StatusCode::CREATED,
        Json(TokenCriadoJson {
            token: TokenJson::de(&token),
            value,
        }),
    ))
}

/// `GET /admin/v1/orgs/{org_id}/tokens/{token_id}`.
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
        (status = 404, description = "Token não encontrado", body = crate::erro::RespostaErro)
    )
)]
pub async fn mostrar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoToken>,
) -> Result<Json<TokenJson>, ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    let token = id_da_url::<AdminTokenId>(&caminho.token_id)?;
    identidade.exigir_org(org)?;
    let registro = estado.store.tokens().obter(org, token)?;
    Ok(Json(TokenJson::de(&registro)))
}

/// `DELETE /admin/v1/orgs/{org_id}/tokens/{token_id}` — revoga (só token global).
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
        (status = 403, description = "Só token global revoga", body = crate::erro::RespostaErro)
    )
)]
pub async fn revogar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoToken>,
) -> Result<StatusCode, ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    let token = id_da_url::<AdminTokenId>(&caminho.token_id)?;
    identidade.exigir_global()?;
    // Revogar token de outra org não pode acontecer por engano: confere a existência na org.
    estado.store.tokens().obter(org, token)?;
    estado.store.tokens().revogar(org, token)?;
    super::auditoria::registrar(
        &estado,
        &identidade,
        Some(org),
        "revoke",
        "token",
        token.to_string(),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

fn gerar_segredo() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let mut segredo = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(segredo, "{byte:02x}");
    }
    segredo
}

fn gerar_hash(valor: &str) -> Result<String, ApiError> {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(valor.as_bytes(), &sal)
        .map(|hash| hash.to_string())
        .map_err(|erro| {
            tracing::error!(erro = %erro, "falha ao gerar o hash do token");
            ApiError::interno()
        })
}

/// Página padrão usada em testes e em chamadas internas.
#[allow(dead_code)]
pub(crate) fn pagina_padrao() -> Pagina {
    Pagina::default()
}
