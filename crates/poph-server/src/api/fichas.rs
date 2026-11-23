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
//! As rotas ainda não estão no OpenAPI: a documentação delas (inventário de `docs/API.md` e
//! `schema.d.ts`) é a FX-32.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use poph_store::{EtapaDaExecucao, EventoFicha, Execucao, Ficha, FiltroFichas, Fluxo};
use poph_types::{AgentId, FichaId, OrgId, StatusFicha};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{ConsultaPagina, ListaJson, proximo_cursor};
use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url},
    estado::Estado,
};

/// Quantas execuções o detalhe da Ficha traz. O repositório aceita até 500.
const LIMITE_EXECUCOES_NA_FICHA: u32 = 500;
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
pub async fn mostrar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoFicha>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let ficha_id = id_da_url::<FichaId>(&c.ficha_id)?;
    // Ficha de outra org ou inexistente vira 404 aqui; sem isso, a lista de execuções abaixo voltaria
    // vazia com 200.
    let ficha = e.store.fichas().obter(org, ficha_id)?;
    let execucoes = e
        .store
        .execucoes()
        .da_ficha(org, ficha_id, LIMITE_EXECUCOES_NA_FICHA)?;
    let mut lista_execucoes = Vec::with_capacity(execucoes.len());
    for execucao in &execucoes {
        let etapas = e.store.execucoes().etapas(org, execucao.id)?;
        lista_execucoes.push(execucao_json(execucao, &etapas));
    }
    let liberados: Vec<Value> = e
        .store
        .fluxos()
        .listar_do_agente(org, ficha.agent_id)?
        .iter()
        .map(fluxo_liberado_json)
        .collect();
    let mut corpo = ficha_json(&ficha);
    corpo["execucoes"] = Value::Array(lista_execucoes);
    corpo["permissoes"] = json!({ "fluxos_liberados_ao_agente": liberados });
    Ok(Json(corpo))
}

/// `GET /fichas/{ficha_id}/events`: a linha do tempo da Ficha, em ordem de `seq`.
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

/// Campos da Ficha que a lista e o detalhe trazem.
fn ficha_json(ficha: &Ficha) -> Value {
    json!({
        "id": ficha.id.to_string(),
        "conversation_id": ficha.conversation_id.as_ref().map(ToString::to_string),
        "agent_id": ficha.agent_id.to_string(),
        "canal": ficha.canal,
        "peer_externo": ficha.peer_externo,
        "status": ficha.status,
        "aberta_em": ficha.aberta_em,
        "fechada_em": ficha.fechada_em,
        "fechada_por": ficha.fechada_por,
        "fechada_motivo": ficha.fechada_motivo,
        "version": ficha.versao,
    })
}

/// Execução com as etapas percorridas. Cada etapa traz os campos respondidos nela (W6).
fn execucao_json(execucao: &Execucao, etapas: &[EtapaDaExecucao]) -> Value {
    json!({
        "id": execucao.id.to_string(),
        "fluxo_id": execucao.fluxo_id.to_string(),
        "revisao_id": execucao.revisao_id.to_string(),
        "status": execucao.status,
        "no_atual": execucao.no_atual,
        "resultado": execucao.resultado,
        "motivo_fim": execucao.motivo_fim,
        "iniciada_em": execucao.iniciada_em,
        "finalizada_em": execucao.finalizada_em,
        "version": execucao.versao,
        "etapas": etapas.iter().map(etapa_json).collect::<Vec<Value>>(),
    })
}

fn etapa_json(etapa: &EtapaDaExecucao) -> Value {
    json!({
        "no_id": etapa.no_id,
        "ordem": etapa.ordem,
        "status": etapa.status,
        "dados": dados_em_json(&etapa.dados_json),
        "atores_que_agiram": etapa.atores_que_agiram,
        "entrou_em": etapa.entrou_em,
        "concluiu_em": etapa.concluiu_em,
    })
}

/// Fluxo liberado ao agente (a permissão que vale para abrir execução na Ficha dele).
fn fluxo_liberado_json(fluxo: &Fluxo) -> Value {
    json!({
        "id": fluxo.id.to_string(),
        "slug": fluxo.slug,
        "name": fluxo.nome,
        "status": fluxo.status.como_texto(),
    })
}

/// Evento da linha do tempo. A chave de idempotência fica de fora: é detalhe interno de gravação.
fn evento_json(evento: &EventoFicha) -> Value {
    json!({
        "id": evento.id.to_string(),
        "seq": evento.seq,
        "tipo": evento.tipo,
        "ator_tipo": evento.ator_tipo,
        "ator_ref": evento.ator_ref,
        "execucao_id": evento.execucao_id.as_ref().map(ToString::to_string),
        "payload": dados_em_json(&evento.payload_json),
        "criado_em": evento.criado_em,
    })
}

/// JSON que o próprio servidor gravou. Se um dia não parsear, sai `null` em vez de derrubar a leitura.
fn dados_em_json(texto: &str) -> Value {
    serde_json::from_str(texto).unwrap_or(Value::Null)
}
