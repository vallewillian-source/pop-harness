use axum::{
    Json,
    extract::{Path, Query, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
};
use poph_flows::Diagnostic;
use poph_store::{FlowChange, FlowFilter};
use poph_types::{FlowId, OrgId};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

use super::{ListJson, PageQuery, if_match_version, next_cursor, request_status};
use crate::{
    auth::Identity,
    error::{ApiError, id_from_url as parse_path_id},
    estado::Estado,
    fluxos_servico::{
        Ator, FalhaFluxo, Operacoes, json_comparacao, json_fluxo, json_fluxo_com_rascunho,
        json_previa, json_revisao_completa, json_revisao_resumo, json_validacao,
    },
};

const LIMIT_DEFAULT_REVISION: u32 = 50;
const LIMIT_MAXIMUM: u32 = 200;

#[derive(Deserialize)]
pub struct OrgPath {
    org_id: String,
}

#[derive(Deserialize)]
pub struct FlowPath {
    org_id: String,
    flow_id: String,
}

#[derive(Deserialize)]
pub struct RevisionPath {
    org_id: String,
    flow_id: String,
    number: String,
}

#[doc = " Corpo com um script: `validate` e `format`."]
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = DadosScript)]
pub struct ScriptData {
    #[doc = " Texto do script de fluxo."]
    source: String,
}

#[doc = " Corpo do `POST /flows`: o script e se a revisão 1 sai junto."]
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = DadosCriacao)]
pub struct CreationData {
    #[doc = " Texto do script. O `id`, o `nome` e a `descricao` do fluxo vêm dele."]
    source: String,
    #[doc = " Publicar a revisão 1 na mesma chamada. Padrão: `false` (só o rascunho)."]
    #[serde(default)]
    publish: bool,
}

#[doc = " Corpo do `POST /flows/{flow_id}/publish`. Ausente, publica o rascunho."]
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = DadosPublicacao)]
pub struct PublicationData {
    #[doc = " Texto a publicar em vez do rascunho gravado."]
    source: Option<String>,
}

#[doc = " Corpo do `PATCH /flows/{flow_id}`: só o que veio muda."]
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = DadosPatch)]
pub struct PatchData {
    #[doc = " Novo nome (não muda o `id` do script)."]
    name: Option<String>,
    #[doc = " Nova descrição."]
    description: Option<String>,
    #[doc = " Nova situação: `active` ou `inactive`."]
    status: Option<String>,
    #[doc = " Novo texto do rascunho (pode estar inválido; publicar é que exige script bom)."]
    source: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct FlowsQuery {
    limit: Option<u32>,
    cursor: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RevisionsQuery {
    limit: Option<u32>,
    cursor: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct DiffQuery {
    #[serde(rename = "de")]
    from: Option<String>,
    #[serde(rename = "para")]
    to: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct PreviewQuery {
    #[serde(rename = "etapa")]
    step: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ExclusionQuery {
    force: Option<String>,
}

fn flow_refusal(diagnostic: &[Diagnostic]) -> ApiError {
    let errors = diagnostic.iter().filter(|d| d.is_error()).count();
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "fluxo_invalido",
        format!("o script não passa na validação: {errors} erro(s) a corrigir; nada foi gravado"),
    )
    .with_details(json!({ "diagnostics": diagnostic }))
}

impl From<FalhaFluxo> for ApiError {
    fn from(failure: FalhaFluxo) -> Self {
        match failure {
            FalhaFluxo::VersaoVelha {
                id,
                esperada: expected_version,
                atual: current_version,
            } => ApiError::version_conflict("fluxo", &id, expected_version, current_version),
            FalhaFluxo::ScriptVazio => {
                ApiError::validation("source", "o script está vazio: não há o que publicar")
            }
            FalhaFluxo::ScriptInvalido(diagnostic) => flow_refusal(&diagnostic),
            FalhaFluxo::SemRevisao => ApiError::new(
                StatusCode::CONFLICT,
                "fluxo_nao_publicado",
                "o fluxo ainda não tem revisão publicada: publique o rascunho antes",
            ),
            FalhaFluxo::IrDivergente(code) => ApiError::new(
                StatusCode::CONFLICT,
                code,
                "a revisão publicada não confere com o texto guardado: a leitura falha fechada",
            ),
            FalhaFluxo::Interno => ApiError::internal(),
            FalhaFluxo::TetoDeFluxos {
                atual: count,
                teto: limit,
            } => ApiError::new(
                StatusCode::CONFLICT,
                "teto_de_fluxos_por_org",
                format!("a organização já tem {count} fluxos e o teto é {limit}"),
            )
            .with_details(json!({ "flows": count, "limit": limit })),
            FalhaFluxo::Banco(error) => ApiError::from(error),
        }
    }
}

#[doc = " `POST /flows/validate`: compila o script e devolve IR, diagnósticos e resoluções. Não grava."]
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/flows/validate",
    tag = "flows",
    operation_id = "fluxos_validar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
    ),
    request_body = ScriptData,
    responses(
        (status = 200, description = "Compila o script e devolve `valid`, a IR, os diagnósticos (com linha e coluna) e as resoluções de departamento; não grava nada", body = Value),
        (status = 400, description = "Corpo sem `source`", body = crate::error::ErrorResponse),
    )
)]
pub async fn validate_script(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<OrgPath>,
    body: Result<Json<ScriptData>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let Json(d) = body.map_err(|_| {
        ApiError::invalid("corpo JSON inválido: use {\"source\": \"<script do fluxo>\"}")
    })?;
    if d.source.trim().is_empty() {
        return Err(ApiError::validation(
            "source",
            "envie o texto do script em `source`",
        ));
    }
    let ops = Operacoes::novo(&e.store, &e.cache_fluxos, &e.config);
    let validation = ops.validar(Some(org), &d.source)?;
    Ok(Json(json_validacao(&validation)))
}

#[doc = " `GET /flows`: lista da org, com filtro de situação e cursor."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/flows",
    tag = "flows",
    operation_id = "fluxos_listar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("status" = Option<String>, Query, description = "`active` ou `inactive`"),
        ("limit" = Option<u32>, Query, description = "De 1 a 200; padrão 50"),
        ("cursor" = Option<String>, Query, description = "`next_cursor` da página anterior"),
    ),
    responses(
        (status = 200, description = "Fluxos da org: `items` e `next_cursor`", body = ListJson<Value>),
        (status = 400, description = "`status` ou `cursor` inválido", body = crate::error::ErrorResponse),
    )
)]
pub async fn list(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<OrgPath>,
    Query(q): Query<FlowsQuery>,
) -> Result<Json<ListJson<Value>>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let status = match q.status.as_deref() {
        None | Some("") => None,
        Some(text) => Some(request_status(text)?),
    };
    let page = PageQuery {
        limit: q.limit,
        cursor: q.cursor,
    }
    .page()?;
    let limit = page.limit;
    let flows = e.store.flows().list(org, &FlowFilter { status }, page)?;
    let items: Vec<Value> = flows.iter().map(json_fluxo).collect();
    let next_cursor = next_cursor(&flows, limit, |f| f.id.to_string());
    Ok(Json(ListJson { items, next_cursor }))
}

#[doc = " `POST /flows`: cria o fluxo a partir do script; `publish: true` publica a revisão 1 junto."]
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/flows",
    tag = "flows",
    operation_id = "fluxos_criar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
    ),
    request_body = CreationData,
    responses(
        (status = 201, description = "Fluxo criado com o rascunho; com `publish: true`, a revisão 1 sai junto", body = Value),
        (status = 409, description = "Teto de fluxos da org atingido (`teto_de_fluxos_por_org`), com `details.flows` e `details.limit`", body = crate::error::ErrorResponse),
        (status = 422, description = "Script não passa na validação (`fluxo_invalido`), com a lista de diagnósticos; nada é gravado", body = crate::error::ErrorResponse),
    )
)]
pub async fn create(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<OrgPath>,
    body: Result<Json<CreationData>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let Json(d) = body.map_err(|_| {
        ApiError::invalid(
            "corpo JSON inválido: use {\"source\": \"<script do fluxo>\", \"publish\": false}",
        )
    })?;
    if d.source.trim().is_empty() {
        return Err(ApiError::validation(
            "source",
            "envie o texto do script em `source`",
        ));
    }
    let ops = Operacoes::novo(&e.store, &e.cache_fluxos, &e.config);
    let creation = ops.criar(org, &d.source, d.publish, &Ator::token(ident.actor_id()))?;
    let mut body = json_fluxo_com_rascunho(&creation.fluxo);
    match creation.publicacao {
        Some(done) => {
            body["current_revision"] = json!(done.revisao.numero);
            body["revision"] = json_revisao_resumo(&done.revisao);
            body["warnings"] = json!(done.avisos);
            body["version"] = json!(done.versao_do_fluxo);
        }
        None => body["revision"] = Value::Null,
    }
    Ok((StatusCode::CREATED, Json(body)))
}

#[doc = " `GET /flows/{flow_id}`: detalhe com a revisão que vale hoje."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/flows/{flow_id}",
    tag = "flows",
    operation_id = "fluxos_mostrar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("flow_id" = String, Path),
    ),
    responses(
        (status = 200, description = "Fluxo com o rascunho e a revisão que vale hoje (`null` se nunca publicou)", body = Value),
        (status = 404, description = "Fluxo inexistente ou de outra org", body = crate::error::ErrorResponse),
    )
)]
pub async fn show(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<FlowPath>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let flow = e
        .store
        .flows()
        .get(org, parse_path_id::<FlowId>(&c.flow_id)?)?;
    let current = e.store.revisions().current(org, flow.id)?;
    let mut body = json_fluxo_com_rascunho(&flow);
    body["revision"] = current.as_ref().map_or(Value::Null, json_revisao_resumo);
    Ok(Json(body))
}

#[doc = " `PATCH /flows/{flow_id}`: nome, descrição, situação e rascunho, com `If-Match`."]
#[utoipa::path(
    patch,
    path = "/admin/v1/orgs/{org_id}/flows/{flow_id}",
    tag = "flows",
    operation_id = "fluxos_atualizar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("flow_id" = String, Path),
        ("If-Match" = String, Header, description = "Versão atual da Ficha ou da execução, como lida antes"),
    ),
    request_body = PatchData,
    responses(
        (status = 200, description = "Fluxo alterado (só os campos enviados mudam)", body = Value),
        (status = 400, description = "Corpo ou `status` inválido", body = crate::error::ErrorResponse),
        (status = 404, description = "Fluxo inexistente ou de outra org", body = crate::error::ErrorResponse),
        (status = 412, description = "Versão do fluxo diferente da atual", body = crate::error::ErrorResponse),
        (status = 428, description = "Falta o `If-Match`", body = crate::error::ErrorResponse),
    )
)]
pub async fn update(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<FlowPath>,
    h: HeaderMap,
    body: Result<Json<PatchData>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let id = parse_path_id::<FlowId>(&c.flow_id)?;
    let version = if_match_version(&h, "fluxo")?;
    let Json(d) = body.map_err(|_| {
        ApiError::invalid(
            "corpo JSON inválido: use {\"name\", \"description\", \"status\", \"source\"}",
        )
    })?;
    let status = match d.status.as_deref() {
        None => None,
        Some(text) => Some(request_status(text)?),
    };
    let flow = e.store.flows().update(
        org,
        id,
        version,
        FlowChange {
            name: d.name,
            description: d.description,
            status,
            rascunho_fonte: d.source,
        },
    )?;
    super::audit::register(&e, &ident, Some(org), "flow_update", "flow", id.to_string())?;
    Ok(Json(json_fluxo_com_rascunho(&flow)))
}

#[doc = " `DELETE /flows/{flow_id}?force=`: exclui o fluxo; com execução ativa, só com `force=true`."]
#[utoipa::path(
    delete,
    path = "/admin/v1/orgs/{org_id}/flows/{flow_id}",
    tag = "flows",
    operation_id = "fluxos_excluir",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("flow_id" = String, Path),
        ("force" = Option<String>, Query, description = "`true` abandona as execuções ativas e exclui; padrão `false`"),
    ),
    responses(
        (status = 200, description = "Fluxo excluído: `id`, `deleted` e `abandoned_runs`", body = Value),
        (status = 400, description = "`force` que não é `true` nem `false`", body = crate::error::ErrorResponse),
        (status = 404, description = "Fluxo inexistente ou de outra org", body = crate::error::ErrorResponse),
        (status = 409, description = "Há execução ativa e `force` não veio; `details.active_runs` traz a contagem, nada é apagado", body = crate::error::ErrorResponse),
    )
)]
pub async fn delete(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<FlowPath>,
    Query(q): Query<ExclusionQuery>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let id = parse_path_id::<FlowId>(&c.flow_id)?;
    let force = match q.force.as_deref() {
        None | Some("") | Some("false") => false,
        Some("true") => true,
        Some(other) => {
            return Err(
                ApiError::invalid(format!("`force` é true ou false, e veio `{other}`"))
                    .with_details(json!({ "field": "force", "value": other })),
            );
        }
    };
    e.store.flows().get(org, id)?;
    let active: u64 = e
        .store
        .executions()
        .active_by_revision(org, id)?
        .iter()
        .map(|(_, count)| *count)
        .sum();
    if active > 0 && !force {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "conflict",
            format!(
                "o fluxo tem {active} execução(ões) ativa(s) de atendimento; envie `force=true` \
                 para abandoná-las e excluir"
            ),
        )
        .with_details(json!({ "active_runs": active })));
    }
    let abandoned = e.store.flows().delete(org, id, force)?;
    super::audit::register(&e, &ident, Some(org), "flow_delete", "flow", id.to_string())?;
    Ok(Json(json!({
        "id": id.to_string(),
        "deleted": true,
        "abandoned_runs": abandoned,
    })))
}

#[doc = " `POST /flows/{flow_id}/publish`: publica o rascunho (ou o texto do corpo) como revisão nova."]
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/flows/{flow_id}/publish",
    tag = "flows",
    operation_id = "fluxos_publicar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("flow_id" = String, Path),
        ("If-Match" = String, Header, description = "Versão atual da Ficha ou da execução, como lida antes"),
    ),
    request_body = Option<PublicationData>,
    responses(
        (status = 201, description = "Revisão nova publicada: `revision`, `version` do fluxo e `warnings`", body = Value),
        (status = 400, description = "`source` em branco", body = crate::error::ErrorResponse),
        (status = 404, description = "Fluxo inexistente ou de outra org", body = crate::error::ErrorResponse),
        (status = 412, description = "Versão do fluxo diferente da atual; nada é gravado", body = crate::error::ErrorResponse),
        (status = 422, description = "Script não passa na validação (`fluxo_invalido`), com a lista de diagnósticos; nada é gravado", body = crate::error::ErrorResponse),
        (status = 428, description = "Falta o `If-Match`", body = crate::error::ErrorResponse),
    )
)]
pub async fn publish(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<FlowPath>,
    h: HeaderMap,
    body: Result<Json<PublicationData>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let id = parse_path_id::<FlowId>(&c.flow_id)?;
    let version = if_match_version(&h, "fluxo")?;
    let data = match body {
        Ok(Json(d)) => d,
        Err(_) => PublicationData::default(),
    };
    let flow = e.store.flows().get(org, id)?;
    let source = data.source.unwrap_or_else(|| flow.rascunho_fonte.clone());
    let ops = Operacoes::novo(&e.store, &e.cache_fluxos, &e.config);
    let done = ops.publicar(org, &flow, version, &source, &Ator::token(ident.actor_id()))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "revision": json_revisao_resumo(&done.revisao),
            "version": done.versao_do_fluxo,
            "warnings": done.avisos,
        })),
    ))
}

#[doc = " `GET /flows/{flow_id}/revisions`: revisões em ordem de número, com cursor por número."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/flows/{flow_id}/revisions",
    tag = "flows",
    operation_id = "fluxos_listar_revisoes",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("flow_id" = String, Path),
        ("limit" = Option<u32>, Query, description = "De 1 a 200; padrão 50"),
        ("cursor" = Option<String>, Query, description = "Número da última revisão da página anterior"),
    ),
    responses(
        (status = 200, description = "Revisões em ordem de número: `items` e `next_cursor`", body = ListJson<Value>),
        (status = 400, description = "`cursor` inválido", body = crate::error::ErrorResponse),
        (status = 404, description = "Fluxo inexistente ou de outra org", body = crate::error::ErrorResponse),
    )
)]
pub async fn list_revisions(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<FlowPath>,
    Query(q): Query<RevisionsQuery>,
) -> Result<Json<ListJson<Value>>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let id = parse_path_id::<FlowId>(&c.flow_id)?;
    let limit = q
        .limit
        .unwrap_or(LIMIT_DEFAULT_REVISION)
        .clamp(1, LIMIT_MAXIMUM);
    let after = match q.cursor.as_deref() {
        None | Some("") => None,
        Some(text) => Some(text.parse::<u64>().map_err(|_| {
            ApiError::invalid("cursor inválido: use o `next_cursor` da página anterior")
                .with_details(json!({ "field": "cursor", "value": text }))
        })?),
    };
    let revision = e.store.revisions().list(org, id, after, limit)?;
    let next_cursor = if revision.len() < limit as usize {
        None
    } else {
        revision.last().map(|r| r.numero.to_string())
    };
    let items: Vec<Value> = revision.iter().map(json_revisao_resumo).collect();
    Ok(Json(ListJson { items, next_cursor }))
}

#[doc = " `GET /flows/{flow_id}/revisions/{number}`: uma revisão, com o texto e a IR."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/flows/{flow_id}/revisions/{number}",
    tag = "flows",
    operation_id = "fluxos_mostrar_revisao",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("flow_id" = String, Path),
        ("number" = String, Path, description = "Número da revisão"),
    ),
    responses(
        (status = 200, description = "Revisão com o texto publicado, a IR, o hash e as resoluções", body = Value),
        (status = 400, description = "Número que não é de revisão", body = crate::error::ErrorResponse),
        (status = 404, description = "Revisão inexistente ou fluxo de outra org", body = crate::error::ErrorResponse),
    )
)]
pub async fn show_revision(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<RevisionPath>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let id = parse_path_id::<FlowId>(&c.flow_id)?;
    let number = c.number.parse::<u64>().map_err(|_| {
        ApiError::invalid(format!("`{}` não é um número de revisão", c.number))
            .with_details(json!({ "field": "revision", "value": c.number }))
    })?;
    let revision = e.store.revisions().get(org, id, number)?;
    Ok(Json(json_revisao_completa(&revision)))
}

#[doc = " `GET /flows/{flow_id}/diff?de=&para=`: o que mudou, em português, e os atendimentos em cada"]
#[doc = " revisão."]
#[doc = ""]
#[doc = " A contagem é de execuções **ativas** por revisão (FX-20): é o que diz se publicar agora deixa"]
#[doc = " alguém no meio de um atendimento na versão antiga."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/flows/{flow_id}/diff",
    tag = "flows",
    operation_id = "fluxos_diff",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("flow_id" = String, Path),
        ("de" = Option<String>, Query, description = "Número da revisão antiga (padrão: a atual menos 1)"),
        ("para" = Option<String>, Query, description = "Número da revisão nova (padrão: a atual)"),
    ),
    responses(
        (status = 200, description = "Frases do que mudou e, para cada revisão, quantas execuções ativas estão nela", body = Value),
        (status = 400, description = "`de` ou `para` que não são números de revisão", body = crate::error::ErrorResponse),
        (status = 404, description = "Revisão inexistente ou fluxo de outra org", body = crate::error::ErrorResponse),
        (status = 409, description = "Fluxo sem revisão publicada (`fluxo_nao_publicado`) ou IR que não confere com o texto (`ir_divergente`)", body = crate::error::ErrorResponse),
    )
)]
pub async fn diff(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<FlowPath>,
    Query(q): Query<DiffQuery>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let id = parse_path_id::<FlowId>(&c.flow_id)?;
    let flow = e.store.flows().get(org, id)?;
    let number = |text: &str, field: &'static str| -> Result<u64, ApiError> {
        text.parse::<u64>().map_err(|_| {
            ApiError::invalid(format!("`{text}` não é um número de revisão"))
                .with_details(json!({ "field": field, "value": text }))
        })
    };
    let to = match q.to.as_deref() {
        Some(text) if !text.is_empty() => Some(number(text, "para")?),
        _ => None,
    };
    let from = match q.from.as_deref() {
        Some(text) if !text.is_empty() => Some(number(text, "de")?),
        _ => None,
    };
    let ops = Operacoes::novo(&e.store, &e.cache_fluxos, &e.config);
    let comparison = ops.comparar(org, &flow, from, to)?;
    Ok(Json(json_comparacao(id, &comparison)))
}

#[doc = " `GET /flows/{flow_id}/preview?etapa=`: bloco de prompt que o agente veria naquela etapa."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/flows/{flow_id}/preview",
    tag = "flows",
    operation_id = "fluxos_preview",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("flow_id" = String, Path),
        ("etapa" = Option<String>, Query, description = "Id da etapa; padrão: a etapa de início"),
    ),
    responses(
        (status = 200, description = "Bloco do prompt que o agente veria naquela etapa, com a revisão usada", body = Value),
        (status = 404, description = "Etapa inexistente no fluxo, ou fluxo de outra org", body = crate::error::ErrorResponse),
        (status = 409, description = "IR que não confere com o texto publicado (`ir_divergente`)", body = crate::error::ErrorResponse),
    )
)]
pub async fn preview(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<FlowPath>,
    Query(q): Query<PreviewQuery>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let id = parse_path_id::<FlowId>(&c.flow_id)?;
    let flow = e.store.flows().get(org, id)?;
    let ops = Operacoes::novo(&e.store, &e.cache_fluxos, &e.config);
    let preview = ops.preview(org, &flow, q.step.as_deref())?;
    Ok(Json(json_previa(&preview)))
}

#[doc = " `POST /flows/format`: devolve o script na forma canônica (ou os erros, se não compilar)."]
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/flows/format",
    tag = "flows",
    operation_id = "fluxos_formatar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
    ),
    request_body = ScriptData,
    responses(
        (status = 200, description = "Script na forma canônica (`source`)", body = Value),
        (status = 400, description = "Corpo sem `source`", body = crate::error::ErrorResponse),
        (status = 422, description = "Script não compila (`fluxo_invalido`), com a lista de diagnósticos", body = crate::error::ErrorResponse),
    )
)]
pub async fn format(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<OrgPath>,
    body: Result<Json<ScriptData>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.require_org(org)?;
    let Json(d) = body.map_err(|_| {
        ApiError::invalid("corpo JSON inválido: use {\"source\": \"<script do fluxo>\"}")
    })?;
    if d.source.trim().is_empty() {
        return Err(ApiError::validation(
            "source",
            "envie o texto do script em `source`",
        ));
    }
    let ops = Operacoes::novo(&e.store, &e.cache_fluxos, &e.config);
    let text = ops.formatar(&d.source)?;
    Ok(Json(json!({ "source": text })))
}
