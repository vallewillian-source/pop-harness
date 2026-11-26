//! API administrativa do módulo Matrix (MX-22): estado por org, liga/desliga e espaços por
//! departamento.
//!
//! Rotas (PRD §10), todas com token de org valendo só na própria org:
//!
//! - `GET /admin/v1/orgs/{org}/matrix` — estado: ligado, corte do backfill, "conectado" (sonda da
//!   MX-20), atraso do cursor (quantas mensagens faltam e a idade da mais antiga), falhas pendentes e
//!   o motivo curto da última;
//! - `PUT /admin/v1/orgs/{org}/matrix` — liga/desliga a org, com `If-Match` no `updated_at` da
//!   linha (`matrix_orgs`): `428` sem o cabeçalho, `412` com valor velho. Quem grava o corte do
//!   backfill é o repositório (`ligar`); desligar preserva `enabled_at` e não apaga sala nem espaço;
//! - `GET /admin/v1/orgs/{org}/matrix/spaces` — espaços por departamento;
//! - `PUT /admin/v1/orgs/{org}/matrix/spaces` — cria o espaço do departamento (sem `space`) ou liga
//!   um espaço que já existe no Element (`space` = id `!…` ou alias `#…`);
//! - `DELETE /admin/v1/orgs/{org}/matrix/spaces/{department}` — desliga o espaço do departamento.
//!
//! ## O que vale saber
//!
//! - **Nada aqui decide o que os laços decidem.** O `PUT` de espaço usa o mesmo serviço que o laço
//!   de estado usa (MX-12) e grava pela mesma porta; quem posiciona sala continua sendo a MX-15.
//! - **Módulo desligado nesta instância** (`[matrix]` ausente ou `habilitado = false`): o estado
//!   responde com `connected: null` e as rotas de espaço respondem `409`, sem falar com o Synapse.
//! - **Sem série de métrica nova** e sem coluna nova: o "conectado" vem de `Estado::matrix_conectado`
//!   e o ETag do `updated_at` de `matrix_orgs`.
//! - **Nada de segredo nem dado de cliente** na resposta: ids, estados, números e o motivo curto da
//!   falha (código de no máximo 64 caracteres, o mesmo que a MX-08 aceita).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use poph_matrix::espacos::{DepartamentoEspaco, ErroEspaco};
use poph_types::{DepartmentId, OrgId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::orgs::CaminhoOrg;
use super::versao_do_if_match;
use crate::auth::Identidade;
use crate::erro::{ApiError, id_da_url};
use crate::estado::Estado;

/// Como o módulo está numa org.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EstadoMatrixJson {
    /// Org dona.
    pub org_id: String,
    /// Módulo ligado nesta org (`matrix_orgs.enabled`).
    pub enabled: bool,
    /// Corte do backfill (ms): só o que foi gravado depois disto é espelhado. `null` quando a org
    /// nunca ligou o módulo.
    pub enabled_at: Option<i64>,
    /// Última alteração da linha (ms) — é o valor que vai no `If-Match` do `PUT`.
    pub updated_at: i64,
    /// O módulo desta instância está falando com o Synapse? `null` = módulo não configurado aqui.
    pub connected: Option<bool>,
    /// O que ainda falta espelhar nesta org.
    pub backlog: AtrasoJson,
    /// Quantas mensagens estão isoladas em `matrix_falhas`.
    pub pending_failures: u64,
    /// Motivo curto da falha mais recente (nunca texto de cliente), ou `null`.
    pub last_failure: Option<String>,
}

/// Atraso do cursor de mensagens da org.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AtrasoJson {
    /// Mensagens gravadas depois do cursor e ainda não espelhadas.
    pub messages: u64,
    /// Idade, em segundos, da mais antiga delas (`0` quando não há nenhuma pendente).
    pub oldest_seconds: u64,
}

/// Espaço de um departamento.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EspacoJson {
    /// Departamento dono do espaço.
    pub department_id: String,
    /// Nome do departamento (o mesmo que o espaço mostra), quando ele ainda existe.
    pub department_name: Option<String>,
    /// Id do espaço no Synapse (`!…:servidor`).
    pub space_room_id: String,
    /// Alias do departamento no módulo (`#dep_<id>:<servidor>`). Num espaço ligado a um que já
    /// existia, é o nome do departamento para o módulo — quem deu alias no Element foi o operador.
    pub alias: String,
    /// `auto` (o módulo criou) ou `linked` (já existia no Element).
    pub origin: String,
    /// Criação da linha (ms).
    pub created_at: i64,
    /// Última alteração da linha (ms).
    pub updated_at: i64,
}

/// Lista de espaços da org.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EspacosJson {
    /// Espaços, na ordem de criação.
    pub items: Vec<EspacoJson>,
}

/// Corpo do `PUT …/matrix`.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct LigarDesligar {
    /// `true` liga o módulo na org (grava o corte do backfill); `false` desliga.
    pub enabled: bool,
}

/// Corpo do `PUT …/matrix/spaces`.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct LigarEspaco {
    /// Departamento dono do espaço (id UUID v7 da própria org).
    pub department_id: String,
    /// Espaço que já existe no Element: id `!…` ou alias `#…`. Ausente = o módulo cria o espaço
    /// pelo alias do departamento.
    pub space: Option<String>,
}

/// Caminho do espaço de um departamento.
#[derive(Debug, Clone, Deserialize)]
pub struct CaminhoEspaco {
    org_id: String,
    department_id: String,
}

/// Resposta do `DELETE …/matrix/spaces/{department}`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EspacoDesligadoJson {
    /// Departamento do espaço desligado.
    pub department_id: String,
    /// Havia espaço gravado e a linha saiu.
    pub removed: bool,
    /// Salas que deixaram de estar registradas naquele espaço.
    pub rooms_forgotten: u64,
}

// ───────────────────────────── estado da org ─────────────────────────────

/// `GET /admin/v1/orgs/{org_id}/matrix`.
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/matrix",
    tag = "matrix",
    operation_id = "matrix_status",
    security(("bearer" = [])),
    params(("org_id" = String, Path, description = "Id da org")),
    responses(
        (status = 200, description = "Estado do módulo Matrix na org", body = EstadoMatrixJson),
        (status = 401, description = "Sem token", body = crate::erro::RespostaErro),
        (status = 403, description = "Token de outra org", body = crate::erro::RespostaErro),
        (status = 404, description = "Org inexistente", body = crate::erro::RespostaErro)
    )
)]
pub async fn mostrar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoOrg>,
) -> Result<Response, ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    identidade.exigir_org(org)?;
    estado_da_org(&estado, org)
}

/// `PUT /admin/v1/orgs/{org_id}/matrix` (exige `If-Match` com o `updated_at` lido antes).
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
    request_body = LigarDesligar,
    responses(
        (status = 200, description = "Estado do módulo depois da alteração", body = EstadoMatrixJson),
        (status = 401, description = "Sem token", body = crate::erro::RespostaErro),
        (status = 403, description = "Token de outra org", body = crate::erro::RespostaErro),
        (status = 404, description = "Org inexistente", body = crate::erro::RespostaErro),
        (status = 412, description = "`If-Match` desatualizado", body = crate::erro::RespostaErro),
        (status = 428, description = "Sem o cabeçalho `If-Match`", body = crate::erro::RespostaErro)
    )
)]
pub async fn atualizar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoOrg>,
    headers: HeaderMap,
    Json(corpo): Json<LigarDesligar>,
) -> Result<Response, ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    identidade.exigir_org(org)?;
    estado.store.orgs().get(org)?;
    let esperada = versao_do_if_match(&headers, "módulo Matrix da org")?;
    let atual = estado.store.matrix().estado(org)?;
    let atual_etag = u64::try_from(atual.atualizado_em).unwrap_or(0);
    if esperada != atual_etag {
        // `updated_at` é o ETag e também o que muda quando a org liga ou desliga (o `ligar`/`desligar`
        // só escreve quando o estado muda de fato): repetir o mesmo pedido com o mesmo `If-Match` é
        // aceito de novo, e não é conflito.
        return Err(ApiError::conflito_de_versao(
            "módulo Matrix da org",
            &org.to_string(),
            esperada,
            atual_etag,
        ));
    }
    if corpo.enabled {
        estado.store.matrix().ligar(org)?;
    } else {
        estado.store.matrix().desligar(org)?;
    }
    super::auditoria::registrar(
        &estado,
        &identidade,
        Some(org),
        "update",
        "matrix",
        org.to_string(),
    )?;
    estado_da_org(&estado, org)
}

/// Monta o estado da org e devolve com o `ETag` da linha.
fn estado_da_org(estado: &Estado, org: OrgId) -> Result<Response, ApiError> {
    estado.store.orgs().get(org)?;
    let linha = estado.store.matrix().estado(org)?;
    let cursores = estado.store.matrix().ler_cursores(org)?;
    let atraso = estado
        .store
        .matrix()
        .atraso_das_mensagens(org, cursores.mensagens)?;
    let agora = estado.store.now();
    let idade_segundos = atraso
        .mais_antiga_em
        .map(|criada_em| u64::try_from((agora - criada_em).max(0) / 1000).unwrap_or(0))
        .unwrap_or(0);
    let falhas = estado.store.matrix().contar_falhas(org)?;
    let ultima_falha = estado
        .store
        .matrix()
        .listar_falhas(org, 1)?
        .first()
        .map(|falha| falha.motivo.clone());
    let corpo = EstadoMatrixJson {
        org_id: org.to_string(),
        enabled: linha.ligado,
        enabled_at: linha.ligado_em,
        updated_at: linha.atualizado_em,
        connected: estado.matrix_conectado(),
        backlog: AtrasoJson {
            messages: atraso.pendentes,
            oldest_seconds: idade_segundos,
        },
        pending_failures: falhas,
        last_failure: ultima_falha,
    };
    com_etag(corpo, linha.atualizado_em)
}

/// Resposta com o `ETag` do estado, para o cliente guardar e mandar no `If-Match`.
fn com_etag(corpo: EstadoMatrixJson, etag: i64) -> Result<Response, ApiError> {
    let valor = HeaderValue::from_str(&format!("\"{etag}\"")).map_err(|_| ApiError::interno())?;
    let mut resposta = Json(corpo).into_response();
    resposta.headers_mut().insert(header::ETAG, valor);
    Ok(resposta)
}

// ───────────────────────────── espaços ─────────────────────────────

/// `GET /admin/v1/orgs/{org_id}/matrix/spaces`.
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/matrix/spaces",
    tag = "matrix",
    operation_id = "matrix_espacos_listar",
    security(("bearer" = [])),
    params(("org_id" = String, Path, description = "Id da org")),
    responses(
        (status = 200, description = "Espaços por departamento", body = EspacosJson),
        (status = 401, description = "Sem token", body = crate::erro::RespostaErro),
        (status = 403, description = "Token de outra org", body = crate::erro::RespostaErro),
        (status = 404, description = "Org inexistente", body = crate::erro::RespostaErro)
    )
)]
pub async fn listar_espacos(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoOrg>,
) -> Result<Json<EspacosJson>, ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    identidade.exigir_org(org)?;
    estado.store.orgs().get(org)?;
    let items = estado
        .store
        .matrix()
        .listar_espacos(org)?
        .iter()
        .map(|espaco| EspacoJson {
            department_id: espaco.departamento.to_string(),
            department_name: estado
                .store
                .departments()
                .obter(org, espaco.departamento)
                .ok()
                .map(|departamento| departamento.name),
            space_room_id: espaco.room_id.clone(),
            alias: espaco.alias.clone(),
            origin: espaco.origem.como_texto().to_owned(),
            created_at: espaco.criada_em,
            updated_at: espaco.atualizada_em,
        })
        .collect();
    Ok(Json(EspacosJson { items }))
}

/// `PUT /admin/v1/orgs/{org_id}/matrix/spaces` — cria o espaço do departamento ou liga um existente.
#[utoipa::path(
    put,
    path = "/admin/v1/orgs/{org_id}/matrix/spaces",
    tag = "matrix",
    operation_id = "matrix_espacos_ligar",
    security(("bearer" = [])),
    params(("org_id" = String, Path, description = "Id da org")),
    request_body = LigarEspaco,
    responses(
        (status = 200, description = "Espaço do departamento", body = EspacoJson),
        (status = 400, description = "Departamento ou espaço inválido", body = crate::erro::RespostaErro),
        (status = 401, description = "Sem token", body = crate::erro::RespostaErro),
        (status = 403, description = "Token de outra org", body = crate::erro::RespostaErro),
        (status = 404, description = "Org ou departamento inexistente", body = crate::erro::RespostaErro),
        (status = 409, description = "Sem permissão no espaço, ou módulo desligado nesta instância", body = crate::erro::RespostaErro),
        (status = 503, description = "Synapse ou armazenamento não respondeu; tente de novo", body = crate::erro::RespostaErro)
    )
)]
pub async fn ligar_espaco(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoOrg>,
    Json(corpo): Json<LigarEspaco>,
) -> Result<Json<EspacoJson>, ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    identidade.exigir_org(org)?;
    estado.store.orgs().get(org)?;
    let departamento_id = id_da_url::<DepartmentId>(&corpo.department_id)?;
    let departamento = estado.store.departments().obter(org, departamento_id)?;
    let espacos = estado.matrix_espacos().ok_or_else(modulo_desligado)?;
    let alvo = DepartamentoEspaco {
        id: departamento.id,
        nome: departamento.name.clone(),
    };
    let existente = corpo
        .space
        .as_deref()
        .map(str::trim)
        .filter(|texto| !texto.is_empty());
    let gravado = espacos
        .garantir_espaco(org, &alvo, existente)
        .await
        .map_err(erro_do_espaco)?;
    super::auditoria::registrar(
        &estado,
        &identidade,
        Some(org),
        "update",
        "matrix_space",
        departamento.id.to_string(),
    )?;
    Ok(Json(EspacoJson {
        department_id: departamento.id.to_string(),
        department_name: Some(departamento.name),
        space_room_id: gravado.room_id,
        alias: gravado.alias,
        origin: match gravado.origem {
            poph_matrix::espacos::OrigemEspaco::Automatico => "auto".to_owned(),
            poph_matrix::espacos::OrigemEspaco::Ligado => "linked".to_owned(),
        },
        created_at: estado.store.now(),
        updated_at: estado.store.now(),
    }))
}

/// `DELETE /admin/v1/orgs/{org_id}/matrix/spaces/{department_id}`.
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
        (status = 200, description = "Espaço desligado", body = EspacoDesligadoJson),
        (status = 401, description = "Sem token", body = crate::erro::RespostaErro),
        (status = 403, description = "Token de outra org", body = crate::erro::RespostaErro),
        (status = 404, description = "Org ou departamento inexistente", body = crate::erro::RespostaErro)
    )
)]
pub async fn desligar_espaco(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoEspaco>,
) -> Result<Json<EspacoDesligadoJson>, ApiError> {
    let org = id_da_url::<OrgId>(&caminho.org_id)?;
    identidade.exigir_org(org)?;
    estado.store.orgs().get(org)?;
    let departamento = id_da_url::<DepartmentId>(&caminho.department_id)?;
    // Departamento de outra org (ou inexistente) é 404 e nada muda.
    estado.store.departments().obter(org, departamento)?;
    let desligado = estado.store.matrix().desligar_espaco(org, departamento)?;
    super::auditoria::registrar(
        &estado,
        &identidade,
        Some(org),
        "delete",
        "matrix_space",
        departamento.to_string(),
    )?;
    Ok(Json(EspacoDesligadoJson {
        department_id: departamento.to_string(),
        removed: desligado.removido,
        rooms_forgotten: desligado.salas_esquecidas,
    }))
}

// ───────────────────────────── erros ─────────────────────────────

/// Módulo Matrix não montado nesta instância: não há como falar com o Synapse.
fn modulo_desligado() -> ApiError {
    ApiError::novo(
        StatusCode::CONFLICT,
        "matrix_desligado",
        "o módulo Matrix não está ligado nesta instância: suba o `poph serve` com `[matrix] habilitado = true`",
    )
}

/// Traduz a falha do serviço de espaços para a resposta da API.
///
/// Nada do corpo do Synapse nem id de cliente vai para a resposta: falta de permissão vira `409` com
/// o que o operador precisa fazer no Element; falha de rede/armazenamento vira `503` (tentar de novo
/// resolve); defeito do módulo vira `500`.
fn erro_do_espaco(erro: ErroEspaco) -> ApiError {
    match erro {
        ErroEspaco::SemPermissao { o_que_falta } => ApiError::conflito(o_que_falta),
        ErroEspaco::EspacoInvalido { motivo } => ApiError::validacao("space", motivo.to_owned()),
        ErroEspaco::Interno { .. } => {
            tracing::error!(
                target: "poph_server::api::matrix",
                motivo = "espaco_interno",
                "falha interna ao garantir o espaço do departamento"
            );
            ApiError::interno()
        }
        ErroEspaco::Armazenamento(_) => ApiError::novo(
            StatusCode::SERVICE_UNAVAILABLE,
            "matrix_indisponivel",
            "o armazenamento do módulo Matrix não respondeu: tente de novo",
        ),
        ErroEspaco::Chamada(chamada) => match chamada.classificacao() {
            poph_matrix::Classificacao::Transitoria => ApiError::novo(
                StatusCode::SERVICE_UNAVAILABLE,
                "synapse_indisponivel",
                "o Synapse não respondeu: tente de novo em instantes",
            ),
            poph_matrix::Classificacao::Definitiva => ApiError::novo(
                StatusCode::CONFLICT,
                "synapse_recusou",
                "o Synapse recusou a operação do espaço: confira o espaço informado no Element",
            ),
        },
    }
}
