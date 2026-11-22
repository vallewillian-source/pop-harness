//! Categorias de produto: CRUD em `/admin/v1/orgs/{org_id}/categories`.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use poph_store::{CategoriaProduto, MudancaCategoria, NovaCategoria, Pagina};
use poph_types::{OrgId, ProductCategoryId};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

use super::orgs::CaminhoOrg;
use super::paginacao::{ConsultaPagina, ListaJson, proximo_cursor};
use super::{status_pedido, status_texto, versao_do_if_match};
use crate::auth::Identidade;
use crate::erro::{ApiError, id_da_url};
use crate::estado::Estado;

/// Categoria na resposta da API.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CategoriaJson {
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

impl CategoriaJson {
    fn de(categoria: &CategoriaProduto) -> Self {
        Self {
            id: categoria.id.to_string(),
            org_id: categoria.org_id.to_string(),
            name: categoria.nome.clone(),
            description: categoria.descricao.clone(),
            status: status_texto(categoria.status).to_owned(),
            version: categoria.versao,
            created_at: categoria.criado_em,
            updated_at: categoria.atualizado_em,
        }
    }
}

/// Corpo de criação de categoria.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CriarCategoria {
    /// Nome (único na org, sem diferenciar maiúsculas).
    pub name: String,
    /// Descrição.
    pub description: Option<String>,
}

/// Corpo de alteração de categoria.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct AtualizarCategoria {
    /// Novo nome.
    pub name: Option<String>,
    /// Nova descrição.
    pub description: Option<String>,
    /// Nova situação.
    pub status: Option<String>,
}

/// Caminho de uma categoria.
#[derive(Debug, Clone, Deserialize)]
pub struct CaminhoCategoria {
    /// Id da org.
    pub org_id: String,
    /// Id da categoria.
    pub category_id: String,
}

/// `GET /admin/v1/orgs/{org_id}/categories`.
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
        (status = 200, description = "Página de categorias", body = ListaJson<CategoriaJson>),
        (status = 403, description = "Org de outro token", body = crate::erro::RespostaErro)
    )
)]
pub async fn listar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoOrg>,
    Query(consulta): Query<ConsultaPagina>,
) -> Result<Json<ListaJson<CategoriaJson>>, ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    identidade.exigir_org(org)?;
    let pagina = consulta.pagina()?;
    let categorias = estado.store.categorias().listar(org, pagina)?;
    let items: Vec<CategoriaJson> = categorias.iter().map(CategoriaJson::de).collect();
    let next_cursor = proximo_cursor(&items, pagina.limite, |item| item.id.clone());
    Ok(Json(ListaJson { items, next_cursor }))
}

/// `POST /admin/v1/orgs/{org_id}/categories`.
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/categories",
    tag = "categories",
    operation_id = "categorias_criar",
    security(("bearer" = [])),
    params(("org_id" = String, Path, description = "Id da org")),
    request_body = CriarCategoria,
    responses(
        (status = 201, description = "Categoria criada", body = CategoriaJson),
        (status = 400, description = "Campo inválido", body = crate::erro::RespostaErro),
        (status = 404, description = "Org não encontrada", body = crate::erro::RespostaErro)
    )
)]
pub async fn criar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoOrg>,
    Json(corpo): Json<CriarCategoria>,
) -> Result<(StatusCode, Json<CategoriaJson>), ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    identidade.exigir_org(org)?;
    let nome = validar_nome(&corpo.name)?;
    let descricao = corpo.description.unwrap_or_default();
    let categoria = estado
        .store
        .categorias()
        .criar(org, NovaCategoria { nome, descricao })?;
    super::auditoria::registrar(
        &estado,
        &identidade,
        Some(org),
        "create",
        "category",
        categoria.id.to_string(),
    )?;
    Ok((StatusCode::CREATED, Json(CategoriaJson::de(&categoria))))
}

/// `GET /admin/v1/orgs/{org_id}/categories/{category_id}`.
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
        (status = 200, description = "Categoria", body = CategoriaJson),
        (status = 404, description = "Categoria não encontrada", body = crate::erro::RespostaErro)
    )
)]
pub async fn mostrar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoCategoria>,
) -> Result<Json<CategoriaJson>, ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    let id = id_da_url::<ProductCategoryId>(&caminho.category_id)?;
    identidade.exigir_org(org)?;
    let categoria = estado.store.categorias().obter(org, id)?;
    Ok(Json(CategoriaJson::de(&categoria)))
}

/// `PATCH /admin/v1/orgs/{org_id}/categories/{category_id}` (exige `If-Match`).
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
    request_body = AtualizarCategoria,
    responses(
        (status = 200, description = "Categoria alterada", body = CategoriaJson),
        (status = 412, description = "Versão desatualizada", body = crate::erro::RespostaErro),
        (status = 428, description = "Faltou If-Match", body = crate::erro::RespostaErro)
    )
)]
pub async fn atualizar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoCategoria>,
    headers: HeaderMap,
    Json(corpo): Json<AtualizarCategoria>,
) -> Result<Json<CategoriaJson>, ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    let id = id_da_url::<ProductCategoryId>(&caminho.category_id)?;
    identidade.exigir_org(org)?;
    let esperada = versao_do_if_match(&headers, "categoria")?;
    let status = match &corpo.status {
        Some(texto) => Some(status_pedido(texto)?),
        None => None,
    };
    let mudanca = MudancaCategoria {
        nome: corpo.name.as_deref().map(validar_nome).transpose()?,
        descricao: corpo.description,
        status,
    };
    let categoria = estado
        .store
        .categorias()
        .atualizar(org, id, esperada, mudanca)?;
    super::auditoria::registrar(
        &estado,
        &identidade,
        Some(org),
        "update",
        "category",
        id.to_string(),
    )?;
    Ok(Json(CategoriaJson::de(&categoria)))
}

/// Parâmetros de exclusão de categoria.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct ExclusaoCategoria {
    /// Confirma a exclusão de uma categoria com produtos ou agentes vinculados.
    pub force: Option<bool>,
}

/// Conta os produtos (de qualquer situação) e os agentes que usam a categoria.
///
/// Lê a org inteira em lotes de 200: é operação administrativa e rara, então o custo de
/// ler os agentes um a um é aceitável.
fn contar_vinculos(
    estado: &Estado,
    org: OrgId,
    id: ProductCategoryId,
) -> Result<(u64, u64), ApiError> {
    const LOTE: u32 = 200;
    let mut produtos = 0u64;
    let mut apos = None;
    loop {
        let lote = estado
            .store
            .produtos()
            .listar(org, Pagina { apos, limite: LOTE })?;
        produtos += lote.iter().filter(|p| p.categoria_id == id).count() as u64;
        if lote.len() < LOTE as usize {
            break;
        }
        apos = lote.last().map(|p| p.id.as_uuid());
    }
    let mut agentes = 0u64;
    let mut apos = None;
    loop {
        let lote = estado
            .store
            .agentes()
            .listar(org, Pagina { apos, limite: LOTE })?;
        for agente in &lote {
            let usadas = estado.store.agentes().categorias(org, agente.id)?;
            if usadas.contains(&id) {
                agentes += 1;
            }
        }
        if lote.len() < LOTE as usize {
            break;
        }
        apos = lote.last().map(|a| a.id.as_uuid());
    }
    Ok((produtos, agentes))
}

/// `DELETE /admin/v1/orgs/{org_id}/categories/{category_id}`.
///
/// Sem `force`, uma categoria com produtos ou agentes vinculados responde 409 e nada é apagado.
/// Sem vínculo, apaga como antes. Com `force=true`, apaga a categoria, os produtos dela e os
/// vínculos com agentes.
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
        (status = 404, description = "Categoria não encontrada", body = crate::erro::RespostaErro),
        (status = 409, description = "Categoria tem produtos ou agentes vinculados e faltou force", body = crate::erro::RespostaErro)
    )
)]
pub async fn excluir(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoCategoria>,
    Query(exclusao): Query<ExclusaoCategoria>,
) -> Result<StatusCode, ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    let id = id_da_url::<ProductCategoryId>(&caminho.category_id)?;
    identidade.exigir_org(org)?;
    let forcado = exclusao.force.unwrap_or(false);
    if !forcado {
        let (produtos, agentes) = contar_vinculos(&estado, org, id)?;
        if produtos > 0 || agentes > 0 {
            return Err(ApiError::conflito(format!(
                "a categoria tem {produtos} produto(s) e {agentes} agente(s) vinculados; \
                 excluir apaga os produtos dela e tira a categoria dos agentes. \
                 Para confirmar, repita com force=true"
            ))
            .com_detalhes(json!({ "produtos": produtos, "agentes": agentes })));
        }
    }
    estado.store.categorias().excluir(org, id)?;
    super::auditoria::registrar(
        &estado,
        &identidade,
        Some(org),
        if forcado { "delete_force" } else { "delete" },
        "category",
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
