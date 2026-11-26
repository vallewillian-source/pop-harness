//! Leitura das Fichas sem HTTP (FX-31): o que a API (`api/fichas.rs`) e a CLI (`poph ficha`)
//! mostram de uma Ficha. Uma função só monta o JSON, então as duas saídas não divergem.
//!
//! Só sai o que é da Ficha: nenhum texto de mensagem da conversa entra no JSON. O payload dos eventos
//! é metadado do motor, e a chave de idempotência fica de fora.

use poph_store::{Execution, ExecutionStep, Ficha, Fluxo, RecordEvent, Store, StoreError};
use poph_types::{OrgId, RecordId};
use serde_json::{Value, json};

/// Teto de execuções que o detalhe devolve (sem paginação própria; a Ficha tem poucas).
pub const LIMITE_EXECUCOES_NA_FICHA: u32 = 500;

/// O detalhe de uma Ficha: a Ficha, as execuções com as etapas e os campos respondidos (W6), e os
/// fluxos liberados ao agente da Ficha. Ficha de outra org ou inexistente vira o erro do repositório
/// (`NaoEncontrado`), que a API traduz para `404`.
pub fn detalhe(store: &Store, org: OrgId, ficha_id: RecordId) -> Result<Value, StoreError> {
    let ficha = store.fichas().obter(org, ficha_id)?;
    let execucoes = store
        .executions()
        .da_ficha(org, ficha_id, LIMITE_EXECUCOES_NA_FICHA)?;
    let mut lista_execucoes = Vec::with_capacity(execucoes.len());
    for execucao in &execucoes {
        let etapas = store.executions().etapas(org, execucao.id)?;
        lista_execucoes.push(execucao_json(execucao, &etapas));
    }
    let liberados: Vec<Value> = store
        .fluxos()
        .listar_do_agente(org, ficha.agent_id)?
        .iter()
        .map(fluxo_liberado_json)
        .collect();
    let mut corpo = ficha_json(&ficha);
    corpo["execucoes"] = Value::Array(lista_execucoes);
    corpo["permissoes"] = json!({ "fluxos_liberados_ao_agente": liberados });
    Ok(corpo)
}

/// Campos da Ficha que a lista e o detalhe trazem.
pub fn ficha_json(ficha: &Ficha) -> Value {
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
        "version": ficha.version,
    })
}

/// Execução com as etapas percorridas. Cada etapa traz os campos respondidos nela (W6).
pub fn execucao_json(execucao: &Execution, etapas: &[ExecutionStep]) -> Value {
    json!({
        "id": execucao.id.to_string(),
        "fluxo_id": execucao.fluxo_id.to_string(),
        "revisao_id": execucao.revisao_id.to_string(),
        "status": execucao.status,
        "no_atual": execucao.no_atual,
        "resultado": execucao.result,
        "motivo_fim": execucao.motivo_fim,
        "iniciada_em": execucao.iniciada_em,
        "finalizada_em": execucao.finalizada_em,
        "version": execucao.version,
        "etapas": etapas.iter().map(etapa_json).collect::<Vec<Value>>(),
    })
}

pub fn etapa_json(etapa: &ExecutionStep) -> Value {
    json!({
        "no_id": etapa.no_id,
        "ordem": etapa.ordem,
        "status": etapa.status,
        "dados": dados_em_json(&etapa.dados_json),
        "atores_que_agiram": etapa.acting_actors,
        "entrou_em": etapa.entrou_em,
        "concluiu_em": etapa.concluiu_em,
    })
}

/// Fluxo liberado ao agente (a permissão que vale para abrir execução na Ficha dele).
pub fn fluxo_liberado_json(fluxo: &Fluxo) -> Value {
    json!({
        "id": fluxo.id.to_string(),
        "slug": fluxo.slug,
        "name": fluxo.name,
        "status": fluxo.status.as_text(),
    })
}

/// Evento da linha do tempo. A chave de idempotência fica de fora: é detalhe interno de gravação.
pub fn evento_json(evento: &RecordEvent) -> Value {
    json!({
        "id": evento.id.to_string(),
        "seq": evento.seq,
        "tipo": evento.tipo,
        "ator_tipo": evento.ator_tipo,
        "ator_ref": evento.ator_ref,
        "execucao_id": evento.execucao_id.as_ref().map(ToString::to_string),
        "payload": dados_em_json(&evento.payload_json),
        "criado_em": evento.created_at,
    })
}

/// JSON que o próprio servidor gravou. Se um dia não parsear, sai `null` em vez de derrubar a leitura.
pub fn dados_em_json(texto: &str) -> Value {
    serde_json::from_str(texto).unwrap_or(Value::Null)
}
