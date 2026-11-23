//! Leitura das Fichas pela API (FX-29, PRD §7.3 e W6).
//!
//! Três rotas de leitura, sob `/admin/v1/orgs/{org_id}`:
//!
//! - `GET /fichas`: lista com os filtros `status`, `agente`, `canal` e `desde` (instante em
//!   milissegundos), paginada por cursor (`limit` e `cursor`, o último id da página anterior);
//! - `GET /fichas/{ficha_id}`: a Ficha com as execuções de fluxo, as etapas de cada execução com os
//!   campos respondidos em cada etapa (W6) e os fluxos liberados ao agente da Ficha;
//! - `GET /fichas/{ficha_id}/events`: a linha do tempo em ordem de `seq`, paginada por `seq`
//!   (`cursor` é o `seq` do último evento da página anterior).
//!
//! Isolamento: Ficha de outra org ou inexistente responde `404`, sem dizer qual dos dois. Token de
//! org usando o caminho de outra org responde `403`, como no resto da API.
//! Ler não depende do status: Ficha fechada continua legível.
//! Só sai o que é da Ficha: nenhum texto de mensagem da conversa entra nas respostas, e o payload dos
//! eventos é só metadado do motor (chave, tipo, nó).
//!
//! Documentadas no OpenAPI (FX-32): aparecem em `/admin/v1/openapi.json`, em `docs/API.md` e no
//! cliente gerado do Console.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
};
use poph_fluxos::cache::ChaveRevisao;
use poph_fluxos::ir::Destino;
use poph_fluxos::motor::{
    Campos, Contexto, Estado as EstadoMotor, abandonar as abandonar_motor, efeitos_de_entrada,
    executar_acao, iniciar,
};
use poph_fluxos::{Avaliador, Dados, Ir, Limites, LimitesRegra};
use poph_store::{
    ErroStore, EtapaDaExecucao, EventoFicha, Execucao, ExtrasDaTransicao, FiltroFichas,
    InicioDeExecucao, RevisaoDeFluxo,
};
use poph_types::{
    AgentId, AtorAcao, AtorEvento, DomainError, ExecucaoId, FechadaPor, FichaId, FluxoId, OrgId,
    StatusFicha,
};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

use super::{ConsultaPagina, ListaJson, proximo_cursor, versao_do_if_match};
use crate::fichas_servico::{detalhe, evento_json, execucao_json, ficha_json};
use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url},
    estado::Estado,
    fluxos_efeitos::efeito_da_transicao,
    fluxos_transicao::{estado_para_banco, transicao_do_motor},
};

/// Motivo gravado quando a Ficha é fechada sem conversa: o mesmo texto do fechamento pela conversa.
const MOTIVO_FECHAMENTO_PELA_API: &str = "encerrada pelo administrador";

/// Quantas execuções o detalhe da Ficha traz. O repositório aceita até 500.
/// Eventos por página quando o cliente não pede `limit`.
const LIMITE_PADRAO_EVENTOS: u32 = 50;

/// Caminho `/admin/v1/orgs/{org_id}`.
#[derive(Deserialize)]
pub struct CaminhoOrg {
    org_id: String,
}

/// Caminho `/admin/v1/orgs/{org_id}/fichas/{ficha_id}`.
#[derive(Deserialize)]
pub struct CaminhoFicha {
    org_id: String,
    ficha_id: String,
}

/// Filtros e paginação da lista de Fichas. Todos chegam como texto para que o erro saia no formato
/// da API (`validation_error` com o campo), e não como a resposta padrão do extrator.
#[derive(Debug, Default, Deserialize)]
pub struct ConsultaFichas {
    limit: Option<u32>,
    cursor: Option<String>,
    status: Option<String>,
    agente: Option<String>,
    canal: Option<String>,
    desde: Option<String>,
}

/// Paginação da linha do tempo: o cursor é um `seq`, não um id.
#[derive(Debug, Default, Deserialize)]
pub struct ConsultaEventos {
    limit: Option<u32>,
    cursor: Option<String>,
}

/// `GET /fichas`: lista da org, com filtros e cursor.
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/fichas",
    tag = "fichas",
    operation_id = "fichas_listar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("status" = Option<String>, Query, description = "`open` ou `closed`"),
        ("agente" = Option<String>, Query, description = "Id do agente da Ficha (UUID v7)"),
        ("canal" = Option<String>, Query, description = "Canal da conversa, por exemplo `whatsapp`"),
        ("desde" = Option<i64>, Query, description = "Só Fichas abertas a partir deste instante, em milissegundos desde 1970"),
        ("limit" = Option<u32>, Query, description = "De 1 a 200; padrão 50"),
        ("cursor" = Option<String>, Query, description = "`next_cursor` da página anterior"),
    ),
    responses(
        (status = 200, description = "Página de Fichas: `items` e `next_cursor`", body = ListaJson<Value>),
        (status = 400, description = "Filtro ou cursor inválido (`status`, `agente`, `canal`, `desde`, `limit` ou `cursor`)", body = crate::erro::RespostaErro),
    )
)]
pub async fn listar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoOrg>,
    Query(q): Query<ConsultaFichas>,
) -> Result<Json<ListaJson<Value>>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let filtro = filtro_da_consulta(&q)?;
    let pagina = ConsultaPagina {
        limit: q.limit,
        cursor: q.cursor.clone(),
    }
    .pagina()?;
    let fichas = e.store.fichas().listar(org, &filtro, pagina)?;
    let next_cursor = proximo_cursor(&fichas, pagina.limite, |ficha| ficha.id.to_string());
    let items: Vec<Value> = fichas.iter().map(ficha_json).collect();
    Ok(Json(ListaJson { items, next_cursor }))
}

/// `GET /fichas/{ficha_id}`: a Ficha, as execuções com as etapas e os campos respondidos, e os
/// fluxos liberados ao agente da Ficha.
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/fichas/{ficha_id}",
    tag = "fichas",
    operation_id = "fichas_mostrar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("ficha_id" = String, Path),
    ),
    responses(
        (status = 200, description = "A Ficha com as execuções, as etapas com os campos respondidos (W6) e os fluxos liberados ao agente", body = Value),
        (status = 404, description = "Ficha inexistente ou de outra org", body = crate::erro::RespostaErro),
    )
)]
pub async fn mostrar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoFicha>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let ficha_id = id_da_url::<FichaId>(&c.ficha_id)?;
    // `detalhe` lê a Ficha antes de qualquer outra coisa: Ficha de outra org ou inexistente vira 404,
    // e não uma lista de execuções vazia com 200. A CLI (`poph ficha mostrar`) usa a mesma função.
    let corpo = detalhe(&e.store, org, ficha_id)?;
    Ok(Json(corpo))
}

/// `GET /fichas/{ficha_id}/events`: a linha do tempo da Ficha, em ordem de `seq`.
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/events",
    tag = "fichas",
    operation_id = "fichas_eventos",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("ficha_id" = String, Path),
        ("limit" = Option<u32>, Query, description = "De 1 a 200; padrão 50"),
        ("cursor" = Option<String>, Query, description = "`next_cursor` da página anterior"),
    ),
    responses(
        (status = 200, description = "Linha do tempo da Ficha em ordem de `seq`: `items` e `next_cursor` (um `seq`)", body = ListaJson<Value>),
        (status = 400, description = "Cursor inválido", body = crate::erro::RespostaErro),
        (status = 404, description = "Ficha inexistente ou de outra org", body = crate::erro::RespostaErro),
    )
)]
pub async fn eventos(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoFicha>,
    Query(q): Query<ConsultaEventos>,
) -> Result<Json<ListaJson<Value>>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let ficha_id = id_da_url::<FichaId>(&c.ficha_id)?;
    let limite = q.limit.unwrap_or(LIMITE_PADRAO_EVENTOS).clamp(1, 200);
    let apos = cursor_de_seq(q.cursor.as_deref())?;
    // O repositório confere a Ficha antes de ler: Ficha de outra org sai como 404.
    let eventos = e.store.fichas().eventos(org, ficha_id, apos, limite)?;
    let next_cursor = proximo_cursor(&eventos, limite, |evento| evento.seq.to_string());
    let items: Vec<Value> = eventos.iter().map(evento_json).collect();
    Ok(Json(ListaJson { items, next_cursor }))
}

/// Lê os filtros da lista. Valor presente mas vazio, ou fora do vocabulário, devolve 400 com o
/// campo; nenhum filtro é ignorado em silêncio.
fn filtro_da_consulta(q: &ConsultaFichas) -> Result<FiltroFichas, ApiError> {
    let status = match q.status.as_deref() {
        None => None,
        Some(texto) => Some(
            StatusFicha::de_texto(texto)
                .map_err(|_| ApiError::validacao("status", "use open ou closed"))?,
        ),
    };
    let agente = match q.agente.as_deref() {
        None => None,
        Some(texto) => Some(id_da_url::<AgentId>(texto)?),
    };
    let canal = match q.canal.as_deref() {
        None => None,
        Some(texto) if texto.trim().is_empty() => {
            return Err(ApiError::validacao(
                "canal",
                "informe o canal, por exemplo whatsapp",
            ));
        }
        Some(texto) => Some(texto.trim().to_owned()),
    };
    let desde = match q.desde.as_deref() {
        None => None,
        Some(texto) => Some(texto.trim().parse::<i64>().map_err(|_| {
            ApiError::validacao("desde", "use milissegundos desde 1970, como número inteiro")
        })?),
    };
    Ok(FiltroFichas {
        status,
        agente,
        canal,
        desde,
    })
}

/// Cursor da linha do tempo: vazio começa do início; senão, um `seq` não negativo.
fn cursor_de_seq(cursor: Option<&str>) -> Result<Option<i64>, ApiError> {
    match cursor.map(str::trim) {
        None | Some("") => Ok(None),
        Some(texto) => texto
            .parse::<i64>()
            .ok()
            .filter(|seq| *seq >= 0)
            .map(Some)
            .ok_or_else(|| {
                ApiError::invalido("cursor inválido: use o `next_cursor` da resposta anterior")
                    .com_detalhes(json!({ "field": "cursor", "value": texto }))
            }),
    }
}

// ───────────────────── Ações humanas do atendente (FX-30, PRD §7.3 e L5/L6) ─────────────────────
//
// Quatro escritas sobre a Ficha. Nenhuma reimplementa transição: o motor (`poph-fluxos`) diz o que
// acontece, e a operação atômica da FX-20 grava (`Store::execucoes`). Ordem comum a todas:
// `If-Match` obrigatório, chave de idempotência obrigatória, ficha de outra org `404`, Ficha fechada
// `409`, versão velha `412`. Repetir a chave devolve o que a primeira chamada fez (sem evento nem
// efeito novo) e vem **antes** da versão, como no store.

/// Caminho `…/fichas/{ficha_id}/flow-runs/{run_id}`.
#[derive(Deserialize)]
pub struct CaminhoExecucao {
    org_id: String,
    ficha_id: String,
    run_id: String,
}

/// Corpo de `POST …/flow-runs`: o fluxo a abrir e a chave que torna a abertura repetível.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DadosInicio {
    fluxo_id: String,
    chave_idempotencia: String,
}

/// Corpo de `POST …/actions`: a ação, os campos que ela grava e a chave.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DadosAcao {
    acao: String,
    #[serde(default)]
    campos: Option<Value>,
    chave_idempotencia: String,
}

/// Corpo de `POST …/abandon`: o motivo (fica no histórico) e a chave.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DadosAbandono {
    motivo: String,
    chave_idempotencia: String,
}

/// `POST /fichas/{ficha_id}/flow-runs`: o atendente abre um fluxo na Ficha, sem o Jev.
///
/// `If-Match` é a versão da Ficha. Abertura nova responde `201`; repetir a chave responde `200` com a
/// execução que a primeira chamada abriu. Fluxo não liberado ao agente da Ficha: `403`
/// (`fluxo_nao_liberado`, a permissão é conferida pelo store).
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs",
    tag = "fichas",
    operation_id = "fichas_iniciar_fluxo",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("ficha_id" = String, Path),
        ("If-Match" = String, Header, description = "Versão atual da Ficha ou da execução, como lida antes"),
    ),
    request_body = DadosInicio,
    responses(
        (status = 201, description = "Execução aberta na etapa de início do fluxo", body = Value),
        (status = 200, description = "Chave já usada: devolve a execução que a primeira chamada abriu, com `repetida: true`", body = Value),
        (status = 400, description = "Chave de idempotência vazia ou corpo inválido", body = crate::erro::RespostaErro),
        (status = 403, description = "Fluxo não liberado ao agente da Ficha (`fluxo_nao_liberado`); nada é aberto", body = crate::erro::RespostaErro),
        (status = 404, description = "Ficha ou fluxo inexistente na org", body = crate::erro::RespostaErro),
        (status = 409, description = "Ficha fechada (`ficha_encerrada`) ou fluxo sem revisão publicada (`conflict`)", body = crate::erro::RespostaErro),
        (status = 412, description = "Versão da Ficha diferente da atual", body = crate::erro::RespostaErro),
        (status = 428, description = "Falta o `If-Match`", body = crate::erro::RespostaErro),
    )
)]
pub async fn iniciar_fluxo(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoFicha>,
    h: HeaderMap,
    corpo: Result<Json<DadosInicio>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let ficha_id = id_da_url::<FichaId>(&c.ficha_id)?;
    let versao = versao_do_if_match(&h, "ficha")?;
    let Json(dados) = corpo.map_err(|_| {
        ApiError::invalido(
            "corpo JSON inválido: use {\"fluxo_id\": ..., \"chave_idempotencia\": ...}",
        )
    })?;
    let fluxo = id_da_url::<FluxoId>(&dados.fluxo_id)?;
    let chave = chave_obrigatoria(&dados.chave_idempotencia)?;
    // Repetição vem antes da Ficha fechada e da versão: o reenvio devolve a execução da primeira vez.
    if let Some(ja_aberta) = e
        .store
        .execucoes()
        .abertura_ja_feita(org, ficha_id, fluxo, &chave)?
    {
        return Ok((
            StatusCode::OK,
            Json(resposta_execucao(&e, org, &ja_aberta, true, &[])?),
        ));
    }
    let ficha = e.store.fichas().obter(org, ficha_id)?;
    if ficha.status == StatusFicha::Fechada {
        return Err(dominio(DomainError::FichaFechada {
            id: ficha_id.to_string(),
        }));
    }
    if ficha.versao != versao {
        return Err(ApiError::conflito_de_versao(
            "ficha",
            &ficha_id.to_string(),
            versao,
            ficha.versao,
        ));
    }
    let revisao = e
        .store
        .revisoes()
        .atual(org, fluxo)?
        .ok_or_else(|| ApiError::conflito("o fluxo ainda não tem revisão publicada"))?;
    let ir = ir_da_revisao(&e, &revisao)?;
    let avaliador = avaliador_da_revisao(&e, &revisao, &ir)?;
    let ficha_texto = ficha_id.to_string();
    let peer = ficha.peer_externo.clone().unwrap_or_default();
    let contexto = Contexto::novo(&avaliador)
        .com_ficha(&ficha_texto)
        .com_cliente(&peer);
    let estado = iniciar(&ir).map_err(dominio)?;
    let efeitos =
        efeitos_de_entrada(&ir, estado.no_atual.id(), &estado.dados, &contexto).map_err(dominio)?;
    let token = ident.ator_id();
    let gravada = e.store.execucoes().iniciar_execucao(
        org,
        InicioDeExecucao {
            ficha: ficha_id,
            fluxo,
            revisao: revisao.id,
            ator: AtorEvento::Atendente,
            ator_ref: Some(&token),
            decisao_jev: None,
            estado_inicial: estado_para_banco(&estado),
            efeitos_de_entrada: efeitos.iter().map(efeito_da_transicao).collect(),
            chave_idempotencia: Some(&chave),
        },
    )?;
    if !gravada.repetida {
        super::auditoria::registrar(
            &e,
            &ident,
            Some(org),
            "flow_run_start",
            "ficha",
            ficha_texto,
        )?;
    }
    let status = if gravada.repetida {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((
        status,
        Json(resposta_execucao(
            &e,
            org,
            &gravada.execucao,
            gravada.repetida,
            &gravada.eventos,
        )?),
    ))
}

/// `POST /fichas/{ficha_id}/flow-runs/{run_id}/actions`: o atendente executa uma ação da etapa atual.
///
/// `If-Match` é a versão da **execução**. Se o agente andou por cima (outro escrito primeiro), a
/// versão não bate e a resposta é `412`: é a corrida atendente × agente. Ação que só o agente pode
/// fazer: `409` (`acao_invalida`, a recusa do motor). Campo inválido: `400`.
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs/{run_id}/actions",
    tag = "fichas",
    operation_id = "fichas_acao",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("ficha_id" = String, Path),
        ("run_id" = String, Path),
        ("If-Match" = String, Header, description = "Versão atual da Ficha ou da execução, como lida antes"),
    ),
    request_body = DadosAcao,
    responses(
        (status = 200, description = "Execução depois da ação; chave repetida devolve a execução atual, sem evento nem efeito novo", body = Value),
        (status = 400, description = "Chave vazia ou campos inválidos (`campos_invalidos`)", body = crate::erro::RespostaErro),
        (status = 404, description = "Execução fora da Ficha ou da org", body = crate::erro::RespostaErro),
        (status = 409, description = "Ação que só o agente pode fazer (`acao_invalida`) ou Ficha fechada (`ficha_encerrada`)", body = crate::erro::RespostaErro),
        (status = 412, description = "Versão da execução diferente da atual (corrida com o agente)", body = crate::erro::RespostaErro),
        (status = 428, description = "Falta o `If-Match`", body = crate::erro::RespostaErro),
    )
)]
pub async fn acao(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoExecucao>,
    h: HeaderMap,
    corpo: Result<Json<DadosAcao>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let ficha_id = id_da_url::<FichaId>(&c.ficha_id)?;
    let run = id_da_execucao(&c.run_id)?;
    let versao = versao_do_if_match(&h, "execução de fluxo")?;
    let Json(dados) = corpo.map_err(|_| {
        ApiError::invalido(
            "corpo JSON inválido: use {\"acao\": ..., \"campos\": {...}, \"chave_idempotencia\": ...}",
        )
    })?;
    let chave = chave_obrigatoria(&dados.chave_idempotencia)?;
    let campos = campos_do_corpo(dados.campos)?;
    let atual = execucao_da_ficha(&e, org, ficha_id, run)?;
    if e.store
        .execucoes()
        .transicao_ja_aplicada(org, run, &chave)?
    {
        return Ok(Json(resposta_execucao(&e, org, &atual, true, &[])?));
    }
    let ficha = e.store.fichas().obter(org, ficha_id)?;
    if ficha.status == StatusFicha::Fechada {
        return Err(dominio(DomainError::FichaFechada {
            id: ficha_id.to_string(),
        }));
    }
    if atual.versao != versao {
        return Err(ApiError::conflito_de_versao(
            "execução de fluxo",
            &run.to_string(),
            versao,
            atual.versao,
        ));
    }
    let (ir, avaliador, estado) = estado_para_agir(&e, org, &atual)?;
    let ficha_texto = ficha_id.to_string();
    let peer = ficha.peer_externo.clone().unwrap_or_default();
    let contexto = Contexto::novo(&avaliador)
        .com_ficha(&ficha_texto)
        .com_cliente(&peer);
    let resultado = executar_acao(
        &ir,
        &estado,
        &dados.acao,
        &campos,
        AtorAcao::Atendente,
        Some(&chave),
        &contexto,
    )
    .map_err(dominio)?;
    let token = ident.ator_id();
    let gravada = e.store.execucoes().aplicar_transicao(
        org,
        run,
        versao,
        &transicao_do_motor(&resultado),
        &chave,
        ExtrasDaTransicao {
            ator_ref: Some(&token),
            fechada_por: Some(FechadaPor::Atendente),
            motivo_encerramento: None,
        },
    )?;
    if !gravada.repetida {
        super::auditoria::registrar(
            &e,
            &ident,
            Some(org),
            "flow_run_action",
            "execucao_fluxo",
            run.to_string(),
        )?;
    }
    Ok(Json(resposta_execucao(
        &e,
        org,
        &gravada.execucao,
        gravada.repetida,
        &gravada.eventos,
    )?))
}

/// `POST /fichas/{ficha_id}/flow-runs/{run_id}/abandon`: o atendente para a execução onde ela está.
///
/// Não gera efeito. Execução que já não está ativa: `409`. `If-Match` é a versão da execução.
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs/{run_id}/abandon",
    tag = "fichas",
    operation_id = "fichas_abandonar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("ficha_id" = String, Path),
        ("run_id" = String, Path),
        ("If-Match" = String, Header, description = "Versão atual da Ficha ou da execução, como lida antes"),
    ),
    request_body = DadosAbandono,
    responses(
        (status = 200, description = "Execução abandonada; não gera efeito", body = Value),
        (status = 400, description = "Chave vazia ou corpo inválido", body = crate::erro::RespostaErro),
        (status = 404, description = "Execução fora da Ficha ou da org", body = crate::erro::RespostaErro),
        (status = 409, description = "Execução que já não está ativa, ou Ficha fechada (`ficha_encerrada`)", body = crate::erro::RespostaErro),
        (status = 412, description = "Versão da execução diferente da atual", body = crate::erro::RespostaErro),
        (status = 428, description = "Falta o `If-Match`", body = crate::erro::RespostaErro),
    )
)]
pub async fn abandonar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoExecucao>,
    h: HeaderMap,
    corpo: Result<Json<DadosAbandono>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let ficha_id = id_da_url::<FichaId>(&c.ficha_id)?;
    let run = id_da_execucao(&c.run_id)?;
    let versao = versao_do_if_match(&h, "execução de fluxo")?;
    let Json(dados) = corpo.map_err(|_| {
        ApiError::invalido(
            "corpo JSON inválido: use {\"motivo\": ..., \"chave_idempotencia\": ...}",
        )
    })?;
    let chave = chave_obrigatoria(&dados.chave_idempotencia)?;
    let atual = execucao_da_ficha(&e, org, ficha_id, run)?;
    if e.store
        .execucoes()
        .transicao_ja_aplicada(org, run, &chave)?
    {
        return Ok(Json(resposta_execucao(&e, org, &atual, true, &[])?));
    }
    let ficha = e.store.fichas().obter(org, ficha_id)?;
    if ficha.status == StatusFicha::Fechada {
        return Err(dominio(DomainError::FichaFechada {
            id: ficha_id.to_string(),
        }));
    }
    if atual.versao != versao {
        return Err(ApiError::conflito_de_versao(
            "execução de fluxo",
            &run.to_string(),
            versao,
            atual.versao,
        ));
    }
    let (_ir, _avaliador, estado) = estado_para_agir(&e, org, &atual)?;
    let resultado = abandonar_motor(&estado, &dados.motivo, AtorAcao::Atendente, Some(&chave))
        .map_err(dominio)?;
    let token = ident.ator_id();
    let gravada = e.store.execucoes().aplicar_transicao(
        org,
        run,
        versao,
        &transicao_do_motor(&resultado),
        &chave,
        ExtrasDaTransicao {
            ator_ref: Some(&token),
            fechada_por: None,
            motivo_encerramento: None,
        },
    )?;
    if !gravada.repetida {
        super::auditoria::registrar(
            &e,
            &ident,
            Some(org),
            "flow_run_abandon",
            "execucao_fluxo",
            run.to_string(),
        )?;
    }
    Ok(Json(resposta_execucao(
        &e,
        org,
        &gravada.execucao,
        gravada.repetida,
        &gravada.eventos,
    )?))
}

/// `POST /fichas/{ficha_id}/close`: encerra o atendimento. Fecha a conversa e a Ficha juntas e
/// abandona a execução ativa (motivo `atendimento_encerrado`), como o `close` da conversa.
///
/// `If-Match` é a versão da Ficha. Sem conversa ligada (apagada pela retenção), fecha só a Ficha.
/// Ficha já fechada: `409`. Não tem `chave_idempotencia`: repetir depois de fechado responde `409`
/// e não gera nada novo.
#[utoipa::path(
    post,
    path = "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/close",
    tag = "fichas",
    operation_id = "fichas_fechar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path),
        ("ficha_id" = String, Path),
        ("If-Match" = String, Header, description = "Versão atual da Ficha ou da execução, como lida antes"),
    ),
    responses(
        (status = 200, description = "Ficha fechada (e a conversa, quando há); a execução ativa vai para `abandonada`", body = Value),
        (status = 404, description = "Ficha inexistente ou de outra org", body = crate::erro::RespostaErro),
        (status = 409, description = "Ficha já fechada (`ficha_encerrada`)", body = crate::erro::RespostaErro),
        (status = 412, description = "Versão da Ficha diferente da atual", body = crate::erro::RespostaErro),
        (status = 428, description = "Falta o `If-Match`", body = crate::erro::RespostaErro),
    )
)]
pub async fn fechar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoFicha>,
    h: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let ficha_id = id_da_url::<FichaId>(&c.ficha_id)?;
    let esperada = versao_do_if_match(&h, "ficha")?;
    let ficha = e.store.fichas().obter(org, ficha_id)?;
    if ficha.status == StatusFicha::Fechada {
        return Err(dominio(DomainError::FichaFechada {
            id: ficha_id.to_string(),
        }));
    }
    if ficha.versao != esperada {
        return Err(ApiError::conflito_de_versao(
            "ficha",
            &ficha_id.to_string(),
            esperada,
            ficha.versao,
        ));
    }
    match ficha.conversation_id {
        Some(conversa) => {
            // A conversa confere a própria versão na escrita: se mudou entre as duas leituras, `412`.
            let registro = e.store.conversas().obter(org, conversa)?;
            e.store
                .conversas()
                .fechar_auditado(org, conversa, registro.versao, ident.ator_id())?;
        }
        None => {
            e.store.fichas().fechar_sem_conversa(
                org,
                ficha_id,
                esperada,
                MOTIVO_FECHAMENTO_PELA_API,
            )?;
        }
    }
    let atual = e.store.fichas().obter(org, ficha_id)?;
    Ok(Json(ficha_json(&atual)))
}

/// Erro de domínio do motor ou do store, no formato da API.
fn dominio(erro: DomainError) -> ApiError {
    ApiError::from(ErroStore::Dominio(erro))
}

/// A chave de idempotência, sem espaços. Vazia é `400`: sem chave, duas operações diferentes
/// colidiriam no mesmo marcador.
fn chave_obrigatoria(texto: &str) -> Result<String, ApiError> {
    let limpa = texto.trim();
    if limpa.is_empty() {
        return Err(ApiError::validacao(
            "chave_idempotencia",
            "informe uma chave: repetir a mesma tem de devolver o mesmo resultado",
        ));
    }
    Ok(limpa.to_owned())
}

/// Os campos da ação: ausentes viram vazio; se vierem, têm de ser um objeto.
fn campos_do_corpo(campos: Option<Value>) -> Result<Campos, ApiError> {
    match campos {
        None | Some(Value::Null) => Ok(Campos::new()),
        Some(Value::Object(mapa)) => Ok(mapa),
        Some(_) => Err(ApiError::validacao(
            "campos",
            "use um objeto: {\"nome_do_campo\": valor}",
        )),
    }
}

/// Execução pelo id, conferindo que ela é desta Ficha. Execução de outra org, ou de outra Ficha da
/// mesma org, sai como `404`.
fn execucao_da_ficha(
    e: &Estado,
    org: OrgId,
    ficha: FichaId,
    run: ExecucaoId,
) -> Result<Execucao, ApiError> {
    let execucao = e.store.execucoes().obter(org, run)?;
    if execucao.ficha_id != ficha {
        return Err(ApiError::nao_encontrado(
            "execução de fluxo",
            &run.to_string(),
        ));
    }
    Ok(execucao)
}

/// A IR publicada da revisão, pelo cache de revisões (FX-19). A carga recompila o texto e confere
/// que dá a IR gravada; se não der, a revisão não entra e a resposta é `500` (a revisão está ruim).
fn ir_da_revisao(e: &Estado, revisao: &RevisaoDeFluxo) -> Result<Arc<Ir>, ApiError> {
    let gravada = Ir::de_json(&revisao.definicao_json).map_err(|erro| {
        tracing::error!(erro = ?erro, "IR gravada de revisão de fluxo não lê");
        ApiError::interno()
    })?;
    let chave = ChaveRevisao::com_hash(
        revisao.fluxo_id.to_string(),
        u32::try_from(revisao.numero).unwrap_or(u32::MAX),
        revisao.fonte_sha256.clone(),
    );
    e.cache_fluxos
        .carregar(
            &chave,
            &gravada,
            &revisao.fonte,
            &Limites::da_config(&e.config.fluxos),
        )
        .map_err(|erro| {
            tracing::error!(erro = ?erro, "revisão de fluxo publicada não carrega");
            ApiError::interno()
        })
}

/// O avaliador das regras da revisão. Sem `def` no fluxo, basta o de combinadores (não compila
/// script nenhum); com `def`, carrega o módulo do texto da revisão.
fn avaliador_da_revisao(
    e: &Estado,
    revisao: &RevisaoDeFluxo,
    ir: &Ir,
) -> Result<Avaliador, ApiError> {
    if ir.defs.is_empty() {
        return Ok(Avaliador::sem_funcoes());
    }
    Avaliador::novo_com(
        &revisao.fonte,
        &Limites::da_config(&e.config.fluxos),
        LimitesRegra::default(),
    )
    .map_err(|erro| {
        tracing::error!(erro = ?erro, "módulo de regras de revisão não carrega");
        ApiError::interno()
    })
}

/// Tudo o que o motor precisa para agir numa execução: a IR da revisão que ela segue, o avaliador e
/// o estado como está no banco (o nó, os dados e os atores que já agiram na etapa).
fn estado_para_agir(
    e: &Estado,
    org: OrgId,
    execucao: &Execucao,
) -> Result<(Arc<Ir>, Avaliador, EstadoMotor), ApiError> {
    let revisao = e.store.revisoes().obter_por_id(org, execucao.revisao_id)?;
    let ir = ir_da_revisao(e, &revisao)?;
    let avaliador = avaliador_da_revisao(e, &revisao, &ir)?;
    let etapas = e.store.execucoes().etapas(org, execucao.id)?;
    let atores_que_agiram = etapas
        .iter()
        .rev()
        .find(|etapa| etapa.no_id == execucao.no_atual)
        .map(|etapa| etapa.atores_que_agiram.clone())
        .unwrap_or_default();
    let dados = serde_json::from_str::<Dados>(&execucao.dados_json).map_err(|erro| {
        tracing::error!(erro = ?erro, "dados de execução de fluxo não leem");
        ApiError::interno()
    })?;
    let estado = EstadoMotor {
        fluxo_id: ir.fluxo.id.clone(),
        status: execucao.status,
        no_atual: destino_do_id(&ir, &execucao.no_atual)?,
        dados,
        atores_que_agiram,
        resultado: execucao.resultado.clone(),
        motivo: execucao.motivo_fim.clone(),
        chaves: BTreeMap::new(),
    };
    Ok((ir, avaliador, estado))
}

/// Reconstrói o destino tipado do motor a partir do id gravado. Id que não existe na IR da revisão
/// é divergência entre o banco e a publicação: `500`, sem chutar.
fn destino_do_id(ir: &Ir, id: &str) -> Result<Destino, ApiError> {
    if ir.etapas.iter().any(|etapa| etapa.id == id) {
        return Ok(Destino::Etapa { id: id.to_owned() });
    }
    if ir.decisoes.iter().any(|decisao| decisao.id == id) {
        return Ok(Destino::Decisao { id: id.to_owned() });
    }
    if let Some(fim) = ir.fins.iter().find(|fim| fim.id == id) {
        return Ok(Destino::Fim {
            id: fim.id.clone(),
            resultado: fim.resultado.clone(),
            encerra_atendimento: fim.encerra_atendimento,
        });
    }
    tracing::error!(no = id, "nó da execução não existe na IR da revisão");
    Err(ApiError::interno())
}

/// A execução com as etapas e, quando a operação já tinha sido aplicada, a marca de repetição. Os
/// eventos são só os que esta chamada gravou (vazio quando repetida).
fn resposta_execucao(
    e: &Estado,
    org: OrgId,
    execucao: &Execucao,
    repetida: bool,
    eventos: &[EventoFicha],
) -> Result<Value, ApiError> {
    let etapas: Vec<EtapaDaExecucao> = e.store.execucoes().etapas(org, execucao.id)?;
    let mut corpo = execucao_json(execucao, &etapas);
    corpo["repetida"] = json!(repetida);
    corpo["eventos"] = Value::Array(eventos.iter().map(evento_json).collect());
    Ok(corpo)
}

/// Id de execução da URL. Não usa `id_da_url`: o id de execução não tem o nome de entidade que ele
/// exige, e a mensagem de erro é a mesma.
fn id_da_execucao(texto: &str) -> Result<ExecucaoId, ApiError> {
    texto.parse::<ExecucaoId>().map_err(|_| {
        ApiError::id_invalido(
            "execução de fluxo",
            texto,
            "use o id de uma execução (UUID v7)",
        )
    })
}
