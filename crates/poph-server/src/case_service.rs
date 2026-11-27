use poph_store::{Execution, ExecutionStep, Ficha, Fluxo, RecordEvent, Store, StoreError};
use poph_types::{OrgId, RecordId};
use serde_json::{Value, json};

pub const CASE_EXECUTION_LIMIT: u32 = 500;

pub fn detail(store: &Store, org: OrgId, case_id: RecordId) -> Result<Value, StoreError> {
    let case = store.cases().get(org, case_id)?;
    let executions = store
        .executions()
        .from_case(org, case_id, CASE_EXECUTION_LIMIT)?;
    let mut execution_list = Vec::with_capacity(executions.len());
    for execution in &executions {
        let stages = store.executions().stages(org, execution.id)?;
        execution_list.push(execution_json(execution, &stages));
    }
    let allowed_flows: Vec<Value> = store
        .flows()
        .list_from_agent(org, case.agent_id)?
        .iter()
        .map(allowed_flow_json)
        .collect();
    let mut body = case_json(&case);
    body["execucoes"] = Value::Array(execution_list);
    body["permissoes"] = json!({ "fluxos_liberados_ao_agente": allowed_flows });
    Ok(body)
}

pub fn case_json(case: &Ficha) -> Value {
    json!({
        "id": case.id.to_string(),
        "conversation_id": case.conversation_id.as_ref().map(ToString::to_string),
        "agent_id": case.agent_id.to_string(),
        "canal": case.canal,
        "peer_externo": case.peer_externo,
        "status": case.status,
        "aberta_em": case.aberta_em,
        "fechada_em": case.fechada_em,
        "fechada_por": case.fechada_por,
        "fechada_motivo": case.fechada_motivo,
        "version": case.version,
    })
}

pub fn execution_json(execution: &Execution, stages: &[ExecutionStep]) -> Value {
    json!({
        "id": execution.id.to_string(),
        "fluxo_id": execution.fluxo_id.to_string(),
        "revisao_id": execution.revisao_id.to_string(),
        "status": execution.status,
        "no_atual": execution.no_atual,
        "resultado": execution.result,
        "motivo_fim": execution.motivo_fim,
        "iniciada_em": execution.iniciada_em,
        "finalizada_em": execution.finalizada_em,
        "version": execution.version,
        "etapas": stages.iter().map(stage_json).collect::<Vec<Value>>(),
    })
}

pub fn stage_json(stage: &ExecutionStep) -> Value {
    json!({
        "no_id": stage.no_id,
        "ordem": stage.ordem,
        "status": stage.status,
        "dados": parse_stored_json(&stage.dados_json),
        "atores_que_agiram": stage.acting_actors,
        "entrou_em": stage.entrou_em,
        "concluiu_em": stage.concluiu_em,
    })
}

pub fn allowed_flow_json(flow: &Fluxo) -> Value {
    json!({
        "id": flow.id.to_string(),
        "slug": flow.slug,
        "name": flow.name,
        "status": flow.status.as_text(),
    })
}

pub fn event_json(event: &RecordEvent) -> Value {
    json!({
        "id": event.id.to_string(),
        "seq": event.seq,
        "tipo": event.tipo,
        "ator_tipo": event.ator_tipo,
        "ator_ref": event.ator_ref,
        "execucao_id": event.execucao_id.as_ref().map(ToString::to_string),
        "payload": parse_stored_json(&event.payload_json),
        "criado_em": event.created_at,
    })
}

pub fn parse_stored_json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}
