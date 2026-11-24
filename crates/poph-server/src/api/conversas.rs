//! Consultas administrativas de conversas e seu histórico.
use super::orgs::CaminhoOrg;
use super::{ConsultaPagina, ListaJson, proximo_cursor};
use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url},
    estado::Estado,
};
use axum::{
    Json,
    extract::{Path, Query, State, rejection::JsonRejection},
};
use poph_store::{FiltroConversas, OrdemConversas, StatusConversa};
use poph_types::{AgentId, ConversationId, DepartmentId, OrgId};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use utoipa::ToSchema;

#[derive(Deserialize)]
pub struct CaminhoConversa {
    org_id: String,
    conversation_id: String,
}

/// Dados que o Element lê da conversa além dos campos do Harness (RP-01): o nome do agente e a
/// sala do Matrix. Montados em lote por resposta: a lista inteira usa as mesmas duas consultas.
#[derive(Default)]
struct Complementos {
    /// Nome do agente de cada conversa (`agent_name`).
    agentes: HashMap<AgentId, String>,
    /// Sala `ready` de cada conversa (`matrix_room_id`). Vazio quando o módulo está desligado na org.
    salas: HashMap<ConversationId, String>,
}

/// Lê os complementos de um conjunto de conversas da mesma org. São no máximo quatro leituras,
/// qualquer que seja o tamanho do conjunto: nomes de agentes, situação do módulo e, só com o
/// módulo ligado, as salas prontas.
fn complementos_de(
    e: &Estado,
    org: OrgId,
    conversas: &[poph_store::Conversa],
) -> Result<Complementos, ApiError> {
    let mut agentes_ids: Vec<AgentId> = conversas.iter().map(|c| c.agent_id).collect();
    agentes_ids.sort();
    agentes_ids.dedup();
    let agentes = e.store.agentes().nomes(org, &agentes_ids)?;
    let salas = if e.store.matrix().estado(org)?.ligado {
        let ids: Vec<ConversationId> = conversas.iter().map(|c| c.id).collect();
        e.store.matrix().salas_prontas(org, &ids)?
    } else {
        HashMap::new()
    };
    Ok(Complementos { agentes, salas })
}

/// Forma da conversa na API. Os campos de `id` até `last_message_at` vêm do Harness desde o
/// começo. `client_name`, `agent_name` e `matrix_room_id` (RP-01) são a parte que o Element usa
/// para mostrar a conversa: nome do cliente (`null` quando vazio), nome do agente e id da sala do
/// Matrix (só com a sala `ready` e o módulo ligado na org; `null` nos outros casos).
fn conversa_json(c: &poph_store::Conversa, complementos: &Complementos) -> Value {
    json!({"id":c.id.to_string(), "org_id":c.org_id.to_string(), "agent_id":c.agent_id.to_string(),
      "integration_id":c.integration_id.map(|x| x.to_string()), "channel":c.canal, "external_peer":c.peer_externo,
      "status":c.status.como_texto(), "target_department_id":c.departamento_alvo.map(|x| x.to_string()),
      "summary":c.resumo, "version":c.versao, "created_at":c.criado_em, "updated_at":c.atualizado_em,
      "last_message_at":c.ultima_mensagem_em,
      "client_name": c.nome_cliente.as_deref().filter(|nome| !nome.trim().is_empty()),
      "agent_name": complementos.agentes.get(&c.agent_id),
      "matrix_room_id": complementos.salas.get(&c.id)})
}

/// Parâmetros da listagem de conversas: a paginação de sempre mais os filtros e a ordem (RP-02).
/// Sem `sort` e sem filtros, a listagem é a de antes.
#[derive(Debug, Default, Deserialize)]
pub struct ConsultaConversas {
    /// Quantos trazer. Sem `sort`, o valor fora de 1 a 200 é ajustado (como nas outras listas);
    /// com `sort=recent`, valor fora da faixa devolve 400.
    limit: Option<u32>,
    /// Último id da página anterior. Não se usa com `sort=recent`.
    cursor: Option<String>,
    /// Identificador do cliente no canal (`external_peer`), exato.
    external_peer: Option<String>,
    /// Situação: `open`, `transferred` ou `closed`.
    status: Option<String>,
    /// `recent`: da atividade mais recente para a mais antiga, sem cursor.
    sort: Option<String>,
    last_activity_before: Option<String>,
}

/// Lê os filtros da consulta. Valor presente mas vazio, ou desconhecido, devolve 400 sem repetir o
/// valor recebido.
fn filtro_da_consulta(q: &ConsultaConversas) -> Result<FiltroConversas, ApiError> {
    let status = match q.status.as_deref() {
        None => None,
        Some("open") => Some(StatusConversa::Aberta),
        Some("transferred") => Some(StatusConversa::Transferida),
        Some("closed") => Some(StatusConversa::Fechada),
        Some(_) => {
            return Err(ApiError::invalido(
                "status inválido: use open, transferred ou closed",
            ));
        }
    };
    let external_peer = match q.external_peer.as_deref() {
        None => None,
        Some(peer) => {
            let limpo = peer.trim();
            if limpo.is_empty() {
                return Err(ApiError::invalido(
                    "external_peer não pode ser vazio: informe o identificador do cliente",
                ));
            }
            Some(limpo.to_owned())
        }
    };
    let last_activity_before = match q.last_activity_before.as_deref() {
        None => None,
        Some(texto) => Some(texto.trim().parse::<i64>().map_err(|_| {
            ApiError::invalido(
                "last_activity_before inválido: informe o instante em milissegundos desde 1970",
            )
        })?),
    };
    Ok(FiltroConversas {
        status,
        external_peer,
        last_activity_before,
    })
}

/// Lê a ordem pedida. Sem `sort` vale a ordem de id com cursor.
fn ordem_da_consulta(q: &ConsultaConversas) -> Result<OrdemConversas, ApiError> {
    match q.sort.as_deref() {
        None => Ok(OrdemConversas::Id),
        Some("recent") => Ok(OrdemConversas::Recente),
        Some(_) => Err(ApiError::invalido(
            "sort inválido: use recent ou não envie sort",
        )),
    }
}

#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/conversations", tag="conversations", operation_id="conversas_listar", security(("bearer"=[])), params(("org_id"=String, Path), ("limit"=Option<u32>, Query), ("cursor"=Option<String>, Query), ("external_peer"=Option<String>, Query), ("status"=Option<String>, Query), ("sort"=Option<String>, Query), ("last_activity_before"=Option<i64>, Query)), responses((status=200, description="Conversas", body=ListaJson<Value>), (status=400, description="Parâmetro inválido", body=crate::erro::RespostaErro)))]
pub async fn listar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoOrg>,
    Query(q): Query<ConsultaConversas>,
) -> Result<Json<ListaJson<Value>>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let filtro = filtro_da_consulta(&q)?;
    let ordem = ordem_da_consulta(&q)?;
    let (conversas, proximo) = match ordem {
        OrdemConversas::Id => {
            let p = ConsultaPagina {
                limit: q.limit,
                cursor: q.cursor.clone(),
            }
            .pagina()?;
            let conversas = e.store.conversas().listar_filtrado(org, &filtro, p)?;
            (conversas, Some(p))
        }
        OrdemConversas::Recente => {
            if q.cursor.as_deref().is_some_and(|cursor| !cursor.is_empty()) {
                return Err(ApiError::invalido(
                    "sort=recent não usa cursor: peça só limit",
                ));
            }
            let limite = q.limit.unwrap_or(50);
            if !(1..=200).contains(&limite) {
                return Err(ApiError::invalido("limit deve ficar entre 1 e 200"));
            }
            let conversas = e.store.conversas().listar_recentes(org, &filtro, limite)?;
            (conversas, None)
        }
    };
    let complementos = complementos_de(&e, org, &conversas)?;
    let items: Vec<_> = conversas
        .iter()
        .map(|c| conversa_json(c, &complementos))
        .collect();
    let next_cursor = match proximo {
        Some(p) => proximo_cursor(&items, p.limite, |v| {
            v["id"].as_str().unwrap_or_default().to_owned()
        }),
        None => None,
    };
    Ok(Json(ListaJson { items, next_cursor }))
}
#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/conversations/{conversation_id}", tag="conversations", operation_id="conversas_mostrar", security(("bearer"=[])), params(("org_id"=String, Path), ("conversation_id"=String, Path)), responses((status=200, description="Conversa", body=Value)))]
pub async fn mostrar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoConversa>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<ConversationId>(&c.conversation_id)?;
    let conversa = e.store.conversas().obter(org, id)?;
    let complementos = complementos_de(&e, org, std::slice::from_ref(&conversa))?;
    Ok(Json(conversa_json(&conversa, &complementos)))
}
#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/conversations/{conversation_id}/messages", tag="conversations", operation_id="mensagens_listar", security(("bearer"=[])), params(("org_id"=String, Path), ("conversation_id"=String, Path), ("limit"=Option<u32>, Query), ("cursor"=Option<String>, Query)), responses((status=200, description="Mensagens", body=ListaJson<Value>)))]
pub async fn mensagens(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoConversa>,
    Query(q): Query<ConsultaPagina>,
) -> Result<Json<ListaJson<Value>>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<ConversationId>(&c.conversation_id)?;
    e.store.conversas().obter(org, id)?;
    let p = q.pagina()?;
    let items:Vec<_>=e.store.mensagens().listar(org,id,p)?.iter().map(|m|json!({"id":m.id.to_string(),"org_id":m.org_id.to_string(),"conversation_id":m.conversation_id.to_string(),"direction":m.direcao.como_texto(),"role":m.papel.como_texto(),"text":m.texto,"attachments":serde_json::from_str::<Value>(&m.anexos_json).unwrap_or(Value::Null),"turn_id":m.turn_id.map(|v|v.to_string()),"created_at":m.criado_em})).collect();
    let next_cursor = proximo_cursor(&items, p.limite, |v| {
        v["id"].as_str().unwrap_or_default().to_owned()
    });
    Ok(Json(ListaJson { items, next_cursor }))
}
#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/conversations/{conversation_id}/turns", tag="conversations", operation_id="turnos_listar", security(("bearer"=[])), params(("org_id"=String, Path), ("conversation_id"=String, Path), ("limit"=Option<u32>, Query), ("cursor"=Option<String>, Query)), responses((status=200, description="Turnos", body=ListaJson<Value>)))]
pub async fn turnos(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoConversa>,
    Query(q): Query<ConsultaPagina>,
) -> Result<Json<ListaJson<Value>>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<ConversationId>(&c.conversation_id)?;
    e.store.conversas().obter(org, id)?;
    let p = q.pagina()?;
    let items: Vec<_> = e
        .store
        .turnos()
        .listar(org, id, p)?
        .iter()
        .map(|t| {
            // HR-47b: nomes em `tools`, detalhe por chamada em `tool_calls`.
            let (nomes, chamadas) = super::partes_das_ferramentas(&t.ferramentas_json);
            json!({
                "id": t.id.to_string(),
                "org_id": t.org_id.to_string(),
                "conversation_id": t.conversation_id.to_string(),
                "state": t.estado.como_texto(),
                "retrieved": serde_json::from_str::<Value>(&t.trechos_json).unwrap_or(Value::Null),
                "tools": nomes,
                "tool_calls": chamadas,
                "timings": serde_json::from_str::<Value>(&t.tempos_json).unwrap_or(Value::Null),
                "input_tokens": t.tokens_entrada,
                "output_tokens": t.tokens_saida,
                "cost_micros": t.custo_micros,
                "error": t.erro,
                "started_at": t.iniciado_em,
                "finished_at": t.finalizado_em,
            })
        })
        .collect();
    let next_cursor = proximo_cursor(&items, p.limite, |v| {
        v["id"].as_str().unwrap_or_default().to_owned()
    });
    Ok(Json(ListaJson { items, next_cursor }))
}

#[utoipa::path(post, path="/admin/v1/orgs/{org_id}/conversations/{conversation_id}/reopen", tag="conversations", operation_id="conversas_reabrir", security(("bearer"=[])), params(("org_id"=String, Path), ("conversation_id"=String, Path), ("If-Match"=String, Header)), responses((status=200, description="Conversa reaberta", body=Value), (status=409, description="Conversa encerrada (`conversa_encerrada`): não reabre; nada muda", body=crate::erro::RespostaErro), (status=412, description="Versão desatualizada", body=crate::erro::RespostaErro)))]
pub async fn reabrir(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoConversa>,
    headers: axum::http::HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<ConversationId>(&c.conversation_id)?;
    let versao = super::versao_do_if_match(&headers, "conversa")?;
    let conversa = e
        .store
        .conversas()
        .reabrir_auditado(org, id, versao, ident.ator_id())?;
    let complementos = complementos_de(&e, org, std::slice::from_ref(&conversa))?;
    Ok(Json(conversa_json(&conversa, &complementos)))
}

#[utoipa::path(post, path="/admin/v1/orgs/{org_id}/conversations/{conversation_id}/close", tag="conversations", operation_id="conversas_fechar", security(("bearer"=[])), params(("org_id"=String, Path), ("conversation_id"=String, Path), ("If-Match"=String, Header)), responses((status=200, description="Conversa fechada", body=Value), (status=404, description="Conversa não encontrada na org", body=crate::erro::RespostaErro), (status=409, description="Conversa já fechada", body=crate::erro::RespostaErro), (status=412, description="Versão desatualizada", body=crate::erro::RespostaErro), (status=428, description="Faltou If-Match", body=crate::erro::RespostaErro)))]
pub async fn fechar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoConversa>,
    headers: axum::http::HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<ConversationId>(&c.conversation_id)?;
    let versao = super::versao_do_if_match(&headers, "conversa")?;
    let conversa = e
        .store
        .conversas()
        .fechar_auditado(org, id, versao, ident.ator_id())?;
    let complementos = complementos_de(&e, org, std::slice::from_ref(&conversa))?;
    Ok(Json(conversa_json(&conversa, &complementos)))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CorpoTransferencia {
    department_id: Option<String>,
    reason: Option<String>,
}

#[utoipa::path(post, path="/admin/v1/orgs/{org_id}/conversations/{conversation_id}/transfer", tag="conversations", operation_id="conversas_transferir", security(("bearer"=[])), params(("org_id"=String, Path), ("conversation_id"=String, Path)), request_body = CorpoTransferencia, responses((status=200, description="Conversa transferida para o departamento", body=Value), (status=400, description="Corpo inválido, motivo vazio ou departamento malformado", body=crate::erro::RespostaErro), (status=404, description="Conversa ou departamento fora da org", body=crate::erro::RespostaErro), (status=409, description="Conversa encerrada (`conversa_encerrada`): não transfere; nada muda", body=crate::erro::RespostaErro)))]
pub async fn transferir(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoConversa>,
    corpo: Result<Json<CorpoTransferencia>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<ConversationId>(&c.conversation_id)?;
    let Json(dados) = corpo.map_err(|_| {
        ApiError::invalido("corpo JSON inválido: use {\"department_id\": ..., \"reason\": ...}")
    })?;
    let motivo = dados
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|motivo| !motivo.is_empty())
        .ok_or_else(|| ApiError::validacao("reason", "informe o motivo da transferência"))?
        .to_owned();
    let departamento =
        match dados
            .department_id
            .as_deref()
            .map(str::trim)
            .filter(|texto| !texto.is_empty())
        {
            Some(texto) => Some(texto.parse::<DepartmentId>().map_err(|_| {
                ApiError::validacao("department_id", "informe um id de departamento")
            })?),
            None => None,
        };
    let detalhe = json!({
        "motivo": motivo,
        "ator": "administrador",
        "department_id": departamento.map(|destino| destino.to_string()),
    })
    .to_string();
    let conversa = e.store.conversas().transferir_auditado(
        org,
        id,
        departamento,
        Some(ident.ator_id()),
        detalhe,
    )?;
    let complementos = complementos_de(&e, org, std::slice::from_ref(&conversa))?;
    Ok(Json(conversa_json(&conversa, &complementos)))
}
