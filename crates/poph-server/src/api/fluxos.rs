//! API dos fluxos (FX-27, PRD §6.2 e §7.3): validar, rascunho, publicar, revisões, diff, preview e
//! formatar.
//!
//! Todas as rotas ficam sob `/admin/v1/orgs/{org_id}`, com token da org (`403` no caminho de outra
//! org, `401` sem token) e `If-Match` nas escritas:
//!
//! | Método | Rota | O que faz |
//! |---|---|---|
//! | `POST` | `/flows/validate` | compila o script, devolve a IR, os diagnósticos e as resoluções de departamento; não grava nada |
//! | `GET` | `/flows` | lista com filtro `status` e cursor |
//! | `POST` | `/flows` | cria o fluxo a partir do script (rascunho e, se pedido, a revisão 1) |
//! | `GET` | `/flows/{flow_id}` | detalhe com a revisão que vale hoje |
//! | `PATCH` | `/flows/{flow_id}` | muda nome, descrição, situação ou o texto do rascunho |
//! | `DELETE` | `/flows/{flow_id}?force=` | exclui; com execução ativa exige `force=true` |
//! | `POST` | `/flows/{flow_id}/publish` | publica o rascunho (ou o texto do corpo) como revisão nova |
//! | `GET` | `/flows/{flow_id}/revisions` | revisões, paginadas pelo número |
//! | `GET` | `/flows/{flow_id}/revisions/{n}` | uma revisão, com o texto e a estrutura |
//! | `GET` | `/flows/{flow_id}/diff?de=&para=` | o que mudou entre duas revisões, e quantos atendimentos estão em cada uma |
//! | `GET` | `/flows/{flow_id}/preview?etapa=` | bloco de prompt que o agente veria naquela etapa |
//! | `POST` | `/flows/format` | devolve o script na forma canônica |
//!
//! # O que a API faz e o que ela não faz
//!
//! A API é a **única** que compila e valida: o repositório de revisões (`FX-09`) recebe a IR já
//! pronta e só grava. Publicar recusa com `422 fluxo_invalido` (e a lista de diagnósticos, com
//! linha e coluna) qualquer script com erro do compilador, e ainda revalida a IR pela porta
//! semântica (`validar`) antes de entregá-la ao repositório: **nada entra sem passar nas duas**.
//!
//! As resoluções de departamento (nome citado em `encaminhar(...)` → id da org) são resolvidas
//! **na publicação**, com os departamentos que a org tem naquele instante, e vão gravadas em
//! `resolucoes_json`; o atendimento não resolve nome de novo.
//!
//! A leitura de uma revisão para o `diff` e o `preview` passa pelo cache de revisões (FX-19), que
//! recompila o texto e recusa (`409 ir_divergente`) quando a IR gravada não é a que o texto produz:
//! é a mesma porta que o turno usa, então a API não inventa um segundo caminho de carga.
//!
//! # Limites conhecidos
//!
//! - O `diff` compara sempre `de` < `para` no sentido em que foram publicadas; pedir `de` maior que
//!   `para` devolve a diferença invertida (o que a revisão antiga tem a mais), não erro.
//! - `de` e `para` ausentes viram `para = revisao_atual` e `de = para - 1` (na revisão 1, as duas
//!   são a mesma e a lista de mudanças sai vazia).
//! - O `preview` mostra a etapa pedida com os campos vazios: não há simulação de dados preenchidos.
//! - As rotas estão no OpenAPI (FX-32), em `docs/API.md` e no cliente gerado do Console.

use axum::{
    Json,
    extract::{Path, Query, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
};
use poph_fluxos::Diagnostico;
use poph_store::{FiltroFluxos, MudancaFluxo};
use poph_types::{FluxoId, OrgId};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

use super::{ConsultaPagina, ListaJson, proximo_cursor, status_pedido, versao_do_if_match};
use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url},
    estado::Estado,
    fluxos_servico::{
        Ator, FalhaFluxo, Operacoes, json_comparacao, json_fluxo, json_fluxo_com_rascunho,
        json_previa, json_revisao_completa, json_revisao_resumo, json_validacao,
    },
};

/// Revisões por página quando o cliente não pede `limit`.
const LIMITE_PADRAO_REVISOES: u32 = 50;
/// Teto de itens por página nas listas paginadas por cursor de id.
const LIMITE_MAXIMO: u32 = 200;

/// Caminho `/admin/v1/orgs/{org_id}`.
#[derive(Deserialize)]
pub struct CaminhoOrg {
    org_id: String,
}

/// Caminho `/admin/v1/orgs/{org_id}/flows/{flow_id}`.
#[derive(Deserialize)]
pub struct CaminhoFluxo {
    org_id: String,
    flow_id: String,
}

/// Caminho `/admin/v1/orgs/{org_id}/flows/{flow_id}/revisions/{number}`.
#[derive(Deserialize)]
pub struct CaminhoRevisao {
    org_id: String,
    flow_id: String,
    number: String,
}

/// Corpo com um script: `validate` e `format`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DadosScript {
    /// Texto do script de fluxo.
    source: String,
}

/// Corpo do `POST /flows`: o script e se a revisão 1 sai junto.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DadosCriacao {
    /// Texto do script. O `id`, o `nome` e a `descricao` do fluxo vêm dele.
    source: String,
    /// Publicar a revisão 1 na mesma chamada. Padrão: `false` (só o rascunho).
    #[serde(default)]
    publish: bool,
}

/// Corpo do `POST /flows/{flow_id}/publish`. Ausente, publica o rascunho.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DadosPublicacao {
    /// Texto a publicar em vez do rascunho gravado.
    source: Option<String>,
}

/// Corpo do `PATCH /flows/{flow_id}`: só o que veio muda.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DadosPatch {
    /// Novo nome (não muda o `id` do script).
    name: Option<String>,
    /// Nova descrição.
    description: Option<String>,
    /// Nova situação: `active` ou `inactive`.
    status: Option<String>,
    /// Novo texto do rascunho (pode estar inválido; publicar é que exige script bom).
    source: Option<String>,
}

/// Parâmetros da lista de fluxos.
#[derive(Debug, Default, Deserialize)]
pub struct ConsultaFluxos {
    limit: Option<u32>,
    cursor: Option<String>,
    status: Option<String>,
}

/// Paginação da lista de revisões: o cursor é o número da revisão, não um id.
#[derive(Debug, Default, Deserialize)]
pub struct ConsultaRevisoes {
    limit: Option<u32>,
    cursor: Option<String>,
}

/// Parâmetros do `diff`.
#[derive(Debug, Default, Deserialize)]
pub struct ConsultaDiff {
    de: Option<String>,
    para: Option<String>,
}

/// Parâmetros do `preview`.
#[derive(Debug, Default, Deserialize)]
pub struct ConsultaPreview {
    etapa: Option<String>,
}

/// Parâmetros do `DELETE`.
#[derive(Debug, Default, Deserialize)]
pub struct ConsultaExclusao {
    force: Option<String>,
}

// ───────────────────────────── Ponte entre a API e o serviço ─────────────────────────────
//
// As operações estão em `fluxos_servico` (a CLI usa as mesmas). Aqui só se traduz o erro de negócio
// para o status HTTP e o código que a API já devolvia.

/// Recusa `422` com a lista de diagnósticos (código, gravidade, mensagem, linha, coluna, caminho).
fn recusa_de_fluxo(diagnosticos: &[Diagnostico]) -> ApiError {
    let erros = diagnosticos.iter().filter(|d| d.e_erro()).count();
    ApiError::novo(
        StatusCode::UNPROCESSABLE_ENTITY,
        "fluxo_invalido",
        format!("o script não passa na validação: {erros} erro(s) a corrigir; nada foi gravado"),
    )
    .com_detalhes(json!({ "diagnostics": diagnosticos }))
}

impl From<FalhaFluxo> for ApiError {
    fn from(falha: FalhaFluxo) -> Self {
        match falha {
            FalhaFluxo::VersaoVelha {
                id,
                esperada,
                atual,
            } => ApiError::conflito_de_versao("fluxo", &id, esperada, atual),
            FalhaFluxo::ScriptVazio => {
                ApiError::validacao("source", "o script está vazio: não há o que publicar")
            }
            FalhaFluxo::ScriptInvalido(diagnosticos) => recusa_de_fluxo(&diagnosticos),
            // Publicação ainda não existe (`409`): o fluxo só tem rascunho.
            FalhaFluxo::SemRevisao => ApiError::novo(
                StatusCode::CONFLICT,
                "fluxo_nao_publicado",
                "o fluxo ainda não tem revisão publicada: publique o rascunho antes",
            ),
            FalhaFluxo::IrDivergente(codigo) => ApiError::novo(
                StatusCode::CONFLICT,
                codigo,
                "a revisão publicada não confere com o texto guardado: a leitura falha fechada",
            ),
            FalhaFluxo::Interno => ApiError::interno(),
            FalhaFluxo::TetoDeFluxos { atual, teto } => ApiError::novo(
                StatusCode::CONFLICT,
                "teto_de_fluxos_por_org",
                format!("a organização já tem {atual} fluxos e o teto é {teto}"),
            )
            .com_detalhes(json!({ "flows": atual, "limit": teto })),
            FalhaFluxo::Banco(erro) => ApiError::from(erro),
        }
    }
}

// ───────────────────────────────── rotas ─────────────────────────────────

/// `POST /flows/validate`: compila o script e devolve IR, diagnósticos e resoluções. Não grava.
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/flows/validate",
    tag = "flows",
    operation_id = "fluxos_validar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
    ),
    request_body = DadosScript,
    responses(
        (status = 200, description = "Compila o script e devolve `valid`, a IR, os diagnósticos (com linha e coluna) e as resoluções de departamento; não grava nada", body = Value),
        (status = 400, description = "Corpo sem `source`", body = crate::erro::RespostaErro),
    )
)]
pub async fn validar_script(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoOrg>,
    corpo: Result<Json<DadosScript>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let Json(d) = corpo.map_err(|_| {
        ApiError::invalido("corpo JSON inválido: use {\"source\": \"<script do fluxo>\"}")
    })?;
    if d.source.trim().is_empty() {
        return Err(ApiError::validacao(
            "source",
            "envie o texto do script em `source`",
        ));
    }
    let ops = Operacoes::novo(&e.store, &e.cache_fluxos, &e.config);
    let validacao = ops.validar(Some(org), &d.source)?;
    Ok(Json(json_validacao(&validacao)))
}

/// `GET /flows`: lista da org, com filtro de situação e cursor.
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
        (status = 200, description = "Fluxos da org: `items` e `next_cursor`", body = ListaJson<Value>),
        (status = 400, description = "`status` ou `cursor` inválido", body = crate::erro::RespostaErro),
    )
)]
pub async fn listar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoOrg>,
    Query(q): Query<ConsultaFluxos>,
) -> Result<Json<ListaJson<Value>>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let status = match q.status.as_deref() {
        None | Some("") => None,
        Some(texto) => Some(status_pedido(texto)?),
    };
    let pagina = ConsultaPagina {
        limit: q.limit,
        cursor: q.cursor,
    }
    .pagina()?;
    let limite = pagina.limite;
    let fluxos = e
        .store
        .fluxos()
        .listar(org, &FiltroFluxos { status }, pagina)?;
    let items: Vec<Value> = fluxos.iter().map(json_fluxo).collect();
    let next_cursor = proximo_cursor(&fluxos, limite, |f| f.id.to_string());
    Ok(Json(ListaJson { items, next_cursor }))
}

/// `POST /flows`: cria o fluxo a partir do script; `publish: true` publica a revisão 1 junto.
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/flows",
    tag = "flows",
    operation_id = "fluxos_criar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
    ),
    request_body = DadosCriacao,
    responses(
        (status = 201, description = "Fluxo criado com o rascunho; com `publish: true`, a revisão 1 sai junto", body = Value),
        (status = 409, description = "Teto de fluxos da org atingido (`teto_de_fluxos_por_org`), com `details.flows` e `details.limit`", body = crate::erro::RespostaErro),
        (status = 422, description = "Script não passa na validação (`fluxo_invalido`), com a lista de diagnósticos; nada é gravado", body = crate::erro::RespostaErro),
    )
)]
pub async fn criar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoOrg>,
    corpo: Result<Json<DadosCriacao>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let Json(d) = corpo.map_err(|_| {
        ApiError::invalido(
            "corpo JSON inválido: use {\"source\": \"<script do fluxo>\", \"publish\": false}",
        )
    })?;
    if d.source.trim().is_empty() {
        return Err(ApiError::validacao(
            "source",
            "envie o texto do script em `source`",
        ));
    }
    let ops = Operacoes::novo(&e.store, &e.cache_fluxos, &e.config);
    let criacao = ops.criar(org, &d.source, d.publish, &Ator::token(ident.ator_id()))?;
    let mut corpo = json_fluxo_com_rascunho(&criacao.fluxo);
    match criacao.publicacao {
        Some(feita) => {
            corpo["current_revision"] = json!(feita.revisao.numero);
            corpo["revision"] = json_revisao_resumo(&feita.revisao);
            corpo["warnings"] = json!(feita.avisos);
            corpo["version"] = json!(feita.versao_do_fluxo);
        }
        None => corpo["revision"] = Value::Null,
    }
    Ok((StatusCode::CREATED, Json(corpo)))
}

/// `GET /flows/{flow_id}`: detalhe com a revisão que vale hoje.
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
        (status = 404, description = "Fluxo inexistente ou de outra org", body = crate::erro::RespostaErro),
    )
)]
pub async fn mostrar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoFluxo>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let fluxo = e
        .store
        .fluxos()
        .obter(org, id_da_url::<FluxoId>(&c.flow_id)?)?;
    let atual = e.store.revisoes().atual(org, fluxo.id)?;
    let mut corpo = json_fluxo_com_rascunho(&fluxo);
    corpo["revision"] = atual.as_ref().map_or(Value::Null, json_revisao_resumo);
    Ok(Json(corpo))
}

/// `PATCH /flows/{flow_id}`: nome, descrição, situação e rascunho, com `If-Match`.
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
    request_body = DadosPatch,
    responses(
        (status = 200, description = "Fluxo alterado (só os campos enviados mudam)", body = Value),
        (status = 400, description = "Corpo ou `status` inválido", body = crate::erro::RespostaErro),
        (status = 404, description = "Fluxo inexistente ou de outra org", body = crate::erro::RespostaErro),
        (status = 412, description = "Versão do fluxo diferente da atual", body = crate::erro::RespostaErro),
        (status = 428, description = "Falta o `If-Match`", body = crate::erro::RespostaErro),
    )
)]
pub async fn atualizar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoFluxo>,
    h: HeaderMap,
    corpo: Result<Json<DadosPatch>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<FluxoId>(&c.flow_id)?;
    let versao = versao_do_if_match(&h, "fluxo")?;
    let Json(d) = corpo.map_err(|_| {
        ApiError::invalido(
            "corpo JSON inválido: use {\"name\", \"description\", \"status\", \"source\"}",
        )
    })?;
    let status = match d.status.as_deref() {
        None => None,
        Some(texto) => Some(status_pedido(texto)?),
    };
    let fluxo = e.store.fluxos().atualizar(
        org,
        id,
        versao,
        MudancaFluxo {
            nome: d.name,
            descricao: d.description,
            status,
            rascunho_fonte: d.source,
        },
    )?;
    super::auditoria::registrar(&e, &ident, Some(org), "flow_update", "flow", id.to_string())?;
    Ok(Json(json_fluxo_com_rascunho(&fluxo)))
}

/// `DELETE /flows/{flow_id}?force=`: exclui o fluxo; com execução ativa, só com `force=true`.
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
        (status = 400, description = "`force` que não é `true` nem `false`", body = crate::erro::RespostaErro),
        (status = 404, description = "Fluxo inexistente ou de outra org", body = crate::erro::RespostaErro),
        (status = 409, description = "Há execução ativa e `force` não veio; `details.active_runs` traz a contagem, nada é apagado", body = crate::erro::RespostaErro),
    )
)]
pub async fn excluir(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoFluxo>,
    Query(q): Query<ConsultaExclusao>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<FluxoId>(&c.flow_id)?;
    let force = match q.force.as_deref() {
        None | Some("") | Some("false") => false,
        Some("true") => true,
        Some(outro) => {
            return Err(
                ApiError::invalido(format!("`force` é true ou false, e veio `{outro}`"))
                    .com_detalhes(json!({ "field": "force", "value": outro })),
            );
        }
    };
    // O fluxo tem de existir nesta org: 404 antes de contar qualquer coisa.
    e.store.fluxos().obter(org, id)?;
    let ativas: u64 = e
        .store
        .execucoes()
        .ativas_por_revisao(org, id)?
        .iter()
        .map(|(_, quantas)| *quantas)
        .sum();
    if ativas > 0 && !force {
        return Err(ApiError::novo(
            StatusCode::CONFLICT,
            "conflict",
            format!(
                "o fluxo tem {ativas} execução(ões) ativa(s) de atendimento; envie `force=true` \
                 para abandoná-las e excluir"
            ),
        )
        .com_detalhes(json!({ "active_runs": ativas })));
    }
    let abandonadas = e.store.fluxos().excluir(org, id, force)?;
    super::auditoria::registrar(&e, &ident, Some(org), "flow_delete", "flow", id.to_string())?;
    Ok(Json(json!({
        "id": id.to_string(),
        "deleted": true,
        "abandoned_runs": abandonadas,
    })))
}

/// `POST /flows/{flow_id}/publish`: publica o rascunho (ou o texto do corpo) como revisão nova.
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
    request_body = Option<DadosPublicacao>,
    responses(
        (status = 201, description = "Revisão nova publicada: `revision`, `version` do fluxo e `warnings`", body = Value),
        (status = 400, description = "`source` em branco", body = crate::erro::RespostaErro),
        (status = 404, description = "Fluxo inexistente ou de outra org", body = crate::erro::RespostaErro),
        (status = 412, description = "Versão do fluxo diferente da atual; nada é gravado", body = crate::erro::RespostaErro),
        (status = 422, description = "Script não passa na validação (`fluxo_invalido`), com a lista de diagnósticos; nada é gravado", body = crate::erro::RespostaErro),
        (status = 428, description = "Falta o `If-Match`", body = crate::erro::RespostaErro),
    )
)]
pub async fn publicar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoFluxo>,
    h: HeaderMap,
    corpo: Result<Json<DadosPublicacao>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<FluxoId>(&c.flow_id)?;
    let versao = versao_do_if_match(&h, "fluxo")?;
    let dados = match corpo {
        Ok(Json(d)) => d,
        Err(_) => DadosPublicacao::default(),
    };
    let fluxo = e.store.fluxos().obter(org, id)?;
    let fonte = dados.source.unwrap_or_else(|| fluxo.rascunho_fonte.clone());
    let ops = Operacoes::novo(&e.store, &e.cache_fluxos, &e.config);
    let feita = ops.publicar(org, &fluxo, versao, &fonte, &Ator::token(ident.ator_id()))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "revision": json_revisao_resumo(&feita.revisao),
            "version": feita.versao_do_fluxo,
            "warnings": feita.avisos,
        })),
    ))
}

/// `GET /flows/{flow_id}/revisions`: revisões em ordem de número, com cursor por número.
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
        (status = 200, description = "Revisões em ordem de número: `items` e `next_cursor`", body = ListaJson<Value>),
        (status = 400, description = "`cursor` inválido", body = crate::erro::RespostaErro),
        (status = 404, description = "Fluxo inexistente ou de outra org", body = crate::erro::RespostaErro),
    )
)]
pub async fn listar_revisoes(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoFluxo>,
    Query(q): Query<ConsultaRevisoes>,
) -> Result<Json<ListaJson<Value>>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<FluxoId>(&c.flow_id)?;
    let limite = q
        .limit
        .unwrap_or(LIMITE_PADRAO_REVISOES)
        .clamp(1, LIMITE_MAXIMO);
    let apos = match q.cursor.as_deref() {
        None | Some("") => None,
        Some(texto) => Some(texto.parse::<u64>().map_err(|_| {
            ApiError::invalido("cursor inválido: use o `next_cursor` da página anterior")
                .com_detalhes(json!({ "field": "cursor", "value": texto }))
        })?),
    };
    let revisoes = e.store.revisoes().listar(org, id, apos, limite)?;
    let next_cursor = if revisoes.len() < limite as usize {
        None
    } else {
        revisoes.last().map(|r| r.numero.to_string())
    };
    let items: Vec<Value> = revisoes.iter().map(json_revisao_resumo).collect();
    Ok(Json(ListaJson { items, next_cursor }))
}

/// `GET /flows/{flow_id}/revisions/{number}`: uma revisão, com o texto e a IR.
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
        (status = 400, description = "Número que não é de revisão", body = crate::erro::RespostaErro),
        (status = 404, description = "Revisão inexistente ou fluxo de outra org", body = crate::erro::RespostaErro),
    )
)]
pub async fn mostrar_revisao(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoRevisao>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<FluxoId>(&c.flow_id)?;
    let numero = c.number.parse::<u64>().map_err(|_| {
        ApiError::invalido(format!("`{}` não é um número de revisão", c.number))
            .com_detalhes(json!({ "field": "revision", "value": c.number }))
    })?;
    let revisao = e.store.revisoes().obter(org, id, numero)?;
    Ok(Json(json_revisao_completa(&revisao)))
}

/// `GET /flows/{flow_id}/diff?de=&para=`: o que mudou, em português, e os atendimentos em cada
/// revisão.
///
/// A contagem é de execuções **ativas** por revisão (FX-20): é o que diz se publicar agora deixa
/// alguém no meio de um atendimento na versão antiga.
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
        (status = 400, description = "`de` ou `para` que não são números de revisão", body = crate::erro::RespostaErro),
        (status = 404, description = "Revisão inexistente ou fluxo de outra org", body = crate::erro::RespostaErro),
        (status = 409, description = "Fluxo sem revisão publicada (`fluxo_nao_publicado`) ou IR que não confere com o texto (`ir_divergente`)", body = crate::erro::RespostaErro),
    )
)]
pub async fn diff(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoFluxo>,
    Query(q): Query<ConsultaDiff>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<FluxoId>(&c.flow_id)?;
    let fluxo = e.store.fluxos().obter(org, id)?;
    let numero = |texto: &str, campo: &'static str| -> Result<u64, ApiError> {
        texto.parse::<u64>().map_err(|_| {
            ApiError::invalido(format!("`{texto}` não é um número de revisão"))
                .com_detalhes(json!({ "field": campo, "value": texto }))
        })
    };
    let para = match q.para.as_deref() {
        Some(texto) if !texto.is_empty() => Some(numero(texto, "para")?),
        _ => None,
    };
    let de = match q.de.as_deref() {
        Some(texto) if !texto.is_empty() => Some(numero(texto, "de")?),
        _ => None,
    };
    let ops = Operacoes::novo(&e.store, &e.cache_fluxos, &e.config);
    let comparacao = ops.comparar(org, &fluxo, de, para)?;
    Ok(Json(json_comparacao(id, &comparacao)))
}

/// `GET /flows/{flow_id}/preview?etapa=`: bloco de prompt que o agente veria naquela etapa.
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
        (status = 404, description = "Etapa inexistente no fluxo, ou fluxo de outra org", body = crate::erro::RespostaErro),
        (status = 409, description = "IR que não confere com o texto publicado (`ir_divergente`)", body = crate::erro::RespostaErro),
    )
)]
pub async fn preview(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoFluxo>,
    Query(q): Query<ConsultaPreview>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<FluxoId>(&c.flow_id)?;
    let fluxo = e.store.fluxos().obter(org, id)?;
    let ops = Operacoes::novo(&e.store, &e.cache_fluxos, &e.config);
    let previa = ops.preview(org, &fluxo, q.etapa.as_deref())?;
    Ok(Json(json_previa(&previa)))
}

/// `POST /flows/format`: devolve o script na forma canônica (ou os erros, se não compilar).
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/flows/format",
    tag = "flows",
    operation_id = "fluxos_formatar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
    ),
    request_body = DadosScript,
    responses(
        (status = 200, description = "Script na forma canônica (`source`)", body = Value),
        (status = 400, description = "Corpo sem `source`", body = crate::erro::RespostaErro),
        (status = 422, description = "Script não compila (`fluxo_invalido`), com a lista de diagnósticos", body = crate::erro::RespostaErro),
    )
)]
pub async fn formatar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoOrg>,
    corpo: Result<Json<DadosScript>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let Json(d) = corpo.map_err(|_| {
        ApiError::invalido("corpo JSON inválido: use {\"source\": \"<script do fluxo>\"}")
    })?;
    if d.source.trim().is_empty() {
        return Err(ApiError::validacao(
            "source",
            "envie o texto do script em `source`",
        ));
    }
    let ops = Operacoes::novo(&e.store, &e.cache_fluxos, &e.config);
    let texto = ops.formatar(&d.source)?;
    Ok(Json(json!({ "source": texto })))
}
