use std::sync::Arc;

use poph_fluxos::Limites;
use poph_fluxos::cache::{CacheRevisoes, ChaveRevisao};
use poph_fluxos::ir::Ir;
use poph_fluxos::motor::{self, Contexto as ContextEngine, Estado as StateEngine};
use poph_fluxos::prompt::renderizar_bloco;
use poph_fluxos::regras::{Avaliador, Dados, LimitesRegra};
use poph_jev::{G9_NONE, IdGate, JevFlow, JevResult};
use poph_store::{
    Execution, ExecutionStart, Ficha, FlowRevision, Fluxo, JevDecision, Status, Store, StoreError,
    TransitionEffect,
};
use poph_types::{
    ActionActor, AgentId, ConversationId, DomainError, EventActor, FlowsConfig, OrgId,
};
use serde_json::{Value, json};

use crate::flows_transition::{effect_of_transition, state_for_database};

pub const NAME_TOOL_START: &str = "fluxo_iniciar";

pub const ABRE_FLOWS: &str = "<fluxos_de_atendimento>";
pub const CLOSES_FLOWS: &str = "</fluxos_de_atendimento>";

pub(crate) const NOTE_OF_EFFECT: &str =
    "O fluxo agendou uma mensagem própria, que sai depois da sua resposta: não a repita.";

pub(crate) fn apply_note_of_effect(texto: String, effects: usize) -> String {
    if effects == 0 {
        return texto;
    }
    format!("{texto}\n{NOTE_OF_EFFECT}")
}

#[derive(Clone)]
pub struct AdjustmentsFlow {
    ativo: bool,
    limites: Limites,
    jumps: usize,
    cache: Arc<CacheRevisoes>,
}

impl AdjustmentsFlow {
    pub fn of_config(config: &FlowsConfig) -> Self {
        Self {
            ativo: config.enabled,
            limites: Limites::da_config(config),
            jumps: usize::try_from(config.max_decision_hops)
                .unwrap_or(1)
                .max(1),
            cache: Arc::new(CacheRevisoes::novo(
                usize::try_from(config.revision_cache_mb).unwrap_or(usize::MAX),
            )),
        }
    }

    pub fn ativo(&self) -> bool {
        self.ativo
    }

    pub(crate) fn jumps(&self) -> usize {
        self.jumps
    }

    pub fn with_cache(mut self, cache: Arc<CacheRevisoes>) -> Self {
        self.cache = cache;
        self
    }
}

pub(crate) struct Preparation {
    pub ficha: Ficha,
    pub active: Option<Execution>,
    pub released: Vec<JevFlow>,
}

#[derive(Default)]
pub(crate) struct FlowOutcome {
    pub texto: Option<String>,
    pub outcome_record: Option<Value>,
    pub in_progress: bool,
    pub reservation: bool,
}

impl FlowOutcome {
    fn without_flow() -> Self {
        Self::default()
    }

    fn in_progress(mut self) -> Self {
        self.in_progress = true;
        self
    }

    fn reservation(mut self) -> Self {
        self.reservation = true;
        self
    }
}

pub(crate) fn prepare(
    adjustments: &AdjustmentsFlow,
    store: &Store,
    org: OrgId,
    conversa: ConversationId,
    agente: AgentId,
) -> Option<Preparation> {
    if !adjustments.ativo {
        return None;
    }
    let ficha = match store.cases().by_conversation(org, conversa) {
        Ok(Some(ficha)) => ficha,
        Ok(None) => {
            tracing::debug!(
                conversa = %conversa,
                "a conversa não tem Ficha: o turno segue sem fluxo neste turno"
            );
            return None;
        }
        Err(error) => {
            tracing::warn!(error = %error, "não consegui ler a Ficha da conversa");
            return None;
        }
    };
    let active = match store.executions().active_of_case(org, ficha.id) {
        Ok(active) => active,
        Err(error) => {
            tracing::warn!(error = %error, ficha = %ficha.id, "não consegui ler a execução ativa");
            return None;
        }
    };
    if active.is_some() {
        return Some(Preparation {
            ficha,
            active,
            released: Vec::new(),
        });
    }
    let fluxos = match store.flows().list_from_agent(org, agente) {
        Ok(fluxos) => fluxos,
        Err(error) => {
            tracing::warn!(error = %error, agente = %agente, "não consegui listar os fluxos do agente");
            return None;
        }
    };
    let mut released = Vec::new();
    for fluxo in fluxos {
        if fluxo.status != Status::Active || fluxo.revisao_atual.is_none() {
            continue;
        }
        let Some(ir) = ir_vigente(adjustments, store, org, &fluxo) else {
            continue;
        };
        released.push(flow_jev(&fluxo, &ir));
    }
    Some(Preparation {
        ficha,
        active: None,
        released,
    })
}

fn flow_jev(fluxo: &Fluxo, ir: &Ir) -> JevFlow {
    JevFlow {
        id: fluxo.slug.clone(),
        nome: fluxo.name.clone(),
        when: ir.gatilho.quando.clone(),
        not_when: ir.gatilho.nao_quando.clone().unwrap_or_default(),
        examples: ir.gatilho.exemplos.clone(),
    }
}

fn ir_vigente(
    adjustments: &AdjustmentsFlow,
    store: &Store,
    org: OrgId,
    fluxo: &Fluxo,
) -> Option<Arc<Ir>> {
    let revision = match store.revisions().current(org, fluxo.id) {
        Ok(Some(revision)) => revision,
        Ok(None) => return None,
        Err(error) => {
            tracing::warn!(error = %error, fluxo = %fluxo.id, "não consegui ler a revisão atual");
            return None;
        }
    };
    load_ir(adjustments, fluxo, &revision)
}

fn load_ir(
    adjustments: &AdjustmentsFlow,
    fluxo: &Fluxo,
    revision: &FlowRevision,
) -> Option<Arc<Ir>> {
    let stored = match Ir::de_json(&revision.definicao_json) {
        Ok(ir) => ir,
        Err(error) => {
            tracing::error!(
                error = %error,
                revision = %revision.id,
                "a IR gravada da revisão não é JSON legível"
            );
            return None;
        }
    };
    let chave = ChaveRevisao::com_hash(
        fluxo.slug.clone(),
        u32::try_from(revision.numero).unwrap_or(u32::MAX),
        revision.fonte_sha256.clone(),
    );
    match adjustments
        .cache
        .carregar(&chave, &stored, &revision.fonte, &adjustments.limites)
    {
        Ok(ir) => Some(ir),
        Err(failure) => {
            tracing::error!(
                failure = failure.codigo(),
                revision = %revision.id,
                "a revisão publicada não confere com o texto guardado"
            );
            None
        }
    }
}

fn state_of_execution(
    ir: &Ir,
    execution: &Execution,
    actors_that_acted: Vec<ActionActor>,
) -> Option<StateEngine> {
    let dados: Dados = match serde_json::from_str(&execution.dados_json) {
        Ok(dados) => dados,
        Err(error) => {
            tracing::warn!(error = %error, execution = %execution.id, "dados da execução não leem");
            return None;
        }
    };
    let no_atual = if ir.etapas.iter().any(|etapa| etapa.id == execution.no_atual) {
        poph_fluxos::ir::Destino::Etapa {
            id: execution.no_atual.clone(),
        }
    } else if ir
        .decisoes
        .iter()
        .any(|decisao| decisao.id == execution.no_atual)
    {
        poph_fluxos::ir::Destino::Decisao {
            id: execution.no_atual.clone(),
        }
    } else if let Some(fim) = ir.fins.iter().find(|fim| fim.id == execution.no_atual) {
        poph_fluxos::ir::Destino::Fim {
            id: fim.id.clone(),
            resultado: fim.resultado.clone(),
            encerra_atendimento: fim.encerra_atendimento,
        }
    } else {
        tracing::warn!(
            no = %execution.no_atual,
            execution = %execution.id,
            "o nó da execução não existe na IR da revisão"
        );
        return None;
    };
    Some(StateEngine {
        fluxo_id: ir.fluxo.id.clone(),
        status: execution.status,
        no_atual,
        dados,
        atores_que_agiram: actors_that_acted,
        resultado: execution.result.clone(),
        motivo: execution.motivo_fim.clone(),
        chaves: Default::default(),
    })
}

pub(crate) fn block_of_execution(
    adjustments: &AdjustmentsFlow,
    store: &Store,
    org: OrgId,
    execution: &Execution,
) -> Option<(String, Arc<Ir>)> {
    let revision = match store.revisions().get_by_id(org, execution.revisao_id) {
        Ok(revision) => revision,
        Err(error) => {
            tracing::warn!(error = %error, revision = %execution.revisao_id, "revisão da execução não lê");
            return None;
        }
    };
    let fluxo = match store.flows().get(org, execution.fluxo_id) {
        Ok(fluxo) => fluxo,
        Err(error) => {
            tracing::warn!(error = %error, fluxo = %execution.fluxo_id, "fluxo da execução não lê");
            return None;
        }
    };
    let ir = load_ir(adjustments, &fluxo, &revision)?;
    let estado = state_of_execution(&ir, execution, Vec::new())?;
    Some((renderizar_bloco(&ir, &estado), ir))
}

pub(crate) struct LoadOfAction {
    pub ir: Arc<Ir>,
    pub evaluator: Avaliador,
    pub estado: StateEngine,
}

pub(crate) fn load_for_act(
    adjustments: &AdjustmentsFlow,
    store: &Store,
    org: OrgId,
    execution: &Execution,
) -> Option<LoadOfAction> {
    let revision = match store.revisions().get_by_id(org, execution.revisao_id) {
        Ok(revision) => revision,
        Err(error) => {
            tracing::warn!(error = %error, revision = %execution.revisao_id, "revisão da execução não lê");
            return None;
        }
    };
    let fluxo = match store.flows().get(org, execution.fluxo_id) {
        Ok(fluxo) => fluxo,
        Err(error) => {
            tracing::warn!(error = %error, fluxo = %execution.fluxo_id, "fluxo da execução não lê");
            return None;
        }
    };
    let ir = load_ir(adjustments, &fluxo, &revision)?;
    let evaluator = evaluator_of_revision(&revision, &ir, &adjustments.limites)?;
    let actors = match store.executions().stages(org, execution.id) {
        Ok(steps) => steps
            .iter()
            .rev()
            .find(|etapa| etapa.no_id == execution.no_atual)
            .map(|etapa| etapa.acting_actors.clone())
            .unwrap_or_default(),
        Err(error) => {
            tracing::warn!(error = %error, execution = %execution.id, "etapas da execução não leem");
            return None;
        }
    };
    let estado = state_of_execution(&ir, execution, actors)?;
    Some(LoadOfAction {
        ir,
        evaluator,
        estado,
    })
}

fn evaluator_of_revision(revision: &FlowRevision, ir: &Ir, limites: &Limites) -> Option<Avaliador> {
    if ir.defs.is_empty() {
        return Some(Avaliador::sem_funcoes());
    }
    match Avaliador::novo_com(&revision.fonte, limites, LimitesRegra::default()) {
        Ok(evaluator) => Some(evaluator),
        Err(error) => {
            tracing::error!(
                error = %error,
                revision = %revision.id,
                "as regras da revisão não carregam"
            );
            None
        }
    }
}

pub(crate) enum Beginning {
    Open {
        execution: Box<Execution>,
        effects: usize,
    },
    AlreadyActive,
    Rejected(String),
}

pub(crate) enum Initiator<'a> {
    Jev(JevDecision<'a>),
    Agente,
}

impl Initiator<'_> {
    fn actor(&self) -> (EventActor, Option<&'static str>) {
        match self {
            Self::Jev(_) => (EventActor::Jev, Some("g9")),
            Self::Agente => (EventActor::Agent, None),
        }
    }

    fn decisao(&self) -> Option<JevDecision<'_>> {
        match self {
            Self::Jev(decisao) => Some(decisao.clone()),
            Self::Agente => None,
        }
    }
}

pub(crate) fn start(
    adjustments: &AdjustmentsFlow,
    store: &Store,
    org: OrgId,
    ficha: &Ficha,
    slug: &str,
    who: Initiator<'_>,
    chave_idempotencia: Option<&str>,
) -> Result<Beginning, StoreError> {
    let fluxo = match store.flows().by_slug(org, slug) {
        Ok(Some(fluxo)) => fluxo,
        Ok(None) => {
            return Ok(Beginning::Rejected(format!(
                "o fluxo `{slug}` não existe na organização"
            )));
        }
        Err(error) => return Err(error),
    };
    if fluxo.status != Status::Active {
        return Ok(Beginning::Rejected(format!(
            "o fluxo `{slug}` está inativo e não inicia execução nova"
        )));
    }
    let released = store.flows().list_from_agent(org, ficha.agent_id)?;
    if !released.iter().any(|released| released.id == fluxo.id) {
        return Ok(Beginning::Rejected(format!(
            "o fluxo `{slug}` não está liberado para o agente deste atendimento"
        )));
    }
    let revision = match store.revisions().current(org, fluxo.id)? {
        Some(revision) => revision,
        None => {
            return Ok(Beginning::Rejected(format!(
                "o fluxo `{slug}` não tem revisão publicada"
            )));
        }
    };
    let Some(ir) = load_ir(adjustments, &fluxo, &revision) else {
        return Ok(Beginning::Rejected(format!(
            "a revisão publicada do fluxo `{slug}` não carrega"
        )));
    };
    let estado = match motor::iniciar(&ir) {
        Ok(estado) => estado,
        Err(error) => {
            return Ok(Beginning::Rejected(format!(
                "o fluxo `{slug}` não tem etapa inicial válida: {error}"
            )));
        }
    };
    let evaluator = Avaliador::sem_funcoes();
    let text_of_case = ficha.id.to_string();
    let mut context = ContextEngine::novo(&evaluator)
        .com_ficha(&text_of_case)
        .com_limite_de_saltos(adjustments.jumps);
    if let Some(peer) = ficha.peer_externo.as_deref() {
        context = context.com_cliente(peer);
    }
    let effects: Vec<TransitionEffect> =
        match motor::efeitos_de_entrada(&ir, estado.no_atual.id(), &estado.dados, &context) {
            Ok(effects) => effects.iter().map(effect_of_transition).collect(),
            Err(error) => {
                return Ok(Beginning::Rejected(format!(
                    "os efeitos da etapa de entrada do fluxo `{slug}` não resolvem: {error}"
                )));
            }
        };
    let (actor, ator_ref) = who.actor();
    let beginning_effects = effects.len();
    let beginning = ExecutionStart {
        case: ficha.id,
        flow: fluxo.id,
        revision: revision.id,
        actor,
        ator_ref,
        jev_decision: who.decisao(),
        initial_state: state_for_database(&estado),
        inbound_effects: effects,
        chave_idempotencia,
    };
    let effects_towards_entry = beginning_effects;
    match store.executions().start_execution(org, beginning) {
        Ok(stored) => Ok(Beginning::Open {
            execution: Box::new(stored.execution),
            effects: effects_towards_entry,
        }),
        Err(StoreError::Domain(DomainError::ExecutionAlreadyActive { .. })) => {
            Ok(Beginning::AlreadyActive)
        }
        Err(StoreError::Domain(other)) => Ok(Beginning::Rejected(other.to_string())),
        Err(error) => Err(error),
    }
}

pub(crate) fn block_catalog(fluxos: &[JevFlow], tool: bool) -> Option<String> {
    if fluxos.is_empty() {
        return None;
    }
    let mut block = String::with_capacity(120 + fluxos.len() * 200);
    block.push_str(ABRE_FLOWS);
    block.push('\n');
    if tool {
        block.push_str(&format!(
            "Fluxos de atendimento liberados para você. Se a mensagem do cliente pedir um deles, \
             chame {NAME_TOOL_START} com o id do fluxo (um só) e só então responda.\n"
        ));
    } else {
        block.push_str(
            "Fluxos de atendimento disponíveis para esta empresa (informação; quem inicia um fluxo \
             é o sistema):\n",
        );
    }
    for fluxo in fluxos {
        let when = fluxo.when.trim();
        if when.is_empty() {
            block.push_str(&format!("- {}: {}\n", fluxo.id, fluxo.nome));
        } else {
            block.push_str(&format!(
                "- {}: {} — quando: {}\n",
                fluxo.id,
                fluxo.nome,
                a_line(when)
            ));
        }
    }
    block.push_str(CLOSES_FLOWS);
    Some(block)
}

fn a_line(texto: &str) -> String {
    texto.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) enum NoStartReason {
    None,
    BelowThreshold,
    Failed,
    NotAsked,
}

pub(crate) fn reason_without_beginning(jev: Option<&JevResult>, g9_linked: bool) -> NoStartReason {
    let Some(jev) = jev else {
        return NoStartReason::NotAsked;
    };
    match jev.response(IdGate::G9) {
        Some(raw) if raw.choice.as_deref() == Some(G9_NONE) => NoStartReason::None,
        Some(_) => NoStartReason::BelowThreshold,
        None if g9_linked && jev.failed() => NoStartReason::Failed,
        None => NoStartReason::NotAsked,
    }
}

fn outcome_record(resultado: &str) -> Value {
    json!({ "resultado": resultado })
}

fn record_started(execution: &Execution, ir: &Ir) -> Value {
    json!({
        "resultado": "iniciou",
        "fluxo": ir.fluxo.id,
        "execucao": execution.id.to_string(),
        "etapa": execution.no_atual,
    })
}

pub(crate) fn decide(
    adjustments: &AdjustmentsFlow,
    store: &Store,
    org: OrgId,
    preparation: Option<&Preparation>,
    jev: Option<&JevResult>,
    g9_linked: bool,
    tool_of_beginning: bool,
) -> FlowOutcome {
    let Some(preparation) = preparation else {
        return FlowOutcome::without_flow();
    };
    if let Some(active) = preparation.active.as_ref() {
        return match block_of_execution(adjustments, store, org, active) {
            Some((block, ir)) => FlowOutcome {
                texto: Some(block),
                outcome_record: Some(json!({
                    "resultado": "execucao_ativa",
                    "fluxo": ir.fluxo.id,
                    "execucao": active.id.to_string(),
                    "etapa": active.no_atual,
                })),
                ..FlowOutcome::default()
            }
            .in_progress(),
            None => FlowOutcome {
                outcome_record: Some(json!({
                    "resultado": "execucao_ativa_sem_bloco",
                    "execucao": active.id.to_string(),
                })),
                ..FlowOutcome::default()
            }
            .in_progress(),
        };
    }
    if preparation.released.is_empty() {
        return FlowOutcome::without_flow();
    }

    if let Some(slug) = jev.and_then(|resultado| resultado.reliable_choice(IdGate::G9)) {
        let confidence = jev
            .and_then(|resultado| resultado.decision(IdGate::G9))
            .map(|decisao| decisao.confidence);
        let decisao = JevDecision {
            choice: slug,
            confidence,
        };
        return match start(
            adjustments,
            store,
            org,
            &preparation.ficha,
            slug,
            Initiator::Jev(decisao),
            None,
        ) {
            Ok(Beginning::Open { execution, effects }) => {
                let block = block_of_execution(adjustments, store, org, &execution);
                match block {
                    Some((texto, ir)) => FlowOutcome {
                        texto: Some(apply_note_of_effect(texto, effects)),
                        outcome_record: Some(record_started(&execution, &ir)),
                        ..FlowOutcome::default()
                    }
                    .in_progress(),
                    None => FlowOutcome {
                        outcome_record: Some(json!({
                            "resultado": "iniciou_sem_bloco",
                            "fluxo": slug,
                            "execucao": execution.id.to_string(),
                        })),
                        ..FlowOutcome::default()
                    }
                    .in_progress(),
                }
            }
            Ok(Beginning::AlreadyActive) => {
                tracing::info!(
                    ficha = %preparation.ficha.id,
                    fluxo = slug,
                    "o G9 escolheu um fluxo, mas a Ficha já tinha execução ativa"
                );
                match store.executions().active_of_case(org, preparation.ficha.id) {
                    Ok(Some(active)) => {
                        let block = block_of_execution(adjustments, store, org, &active);
                        FlowOutcome {
                            texto: block.map(|(texto, _)| texto),
                            outcome_record: Some(json!({
                                "resultado": "execucao_ativa",
                                "fluxo": slug,
                                "execucao": active.id.to_string(),
                                "motivo": "a Ficha já tinha execução ativa quando o Jev escolheu",
                            })),
                            ..FlowOutcome::default()
                        }
                        .in_progress()
                    }
                    _ => FlowOutcome {
                        outcome_record: Some(json!({
                            "resultado": "nao_iniciou",
                            "fluxo": slug,
                            "motivo": "a Ficha já tinha execução ativa quando o Jev escolheu",
                        })),
                        ..FlowOutcome::default()
                    }
                    .in_progress(),
                }
            }
            Ok(Beginning::Rejected(motivo)) => {
                tracing::warn!(
                    ficha = %preparation.ficha.id,
                    fluxo = slug,
                    motivo,
                    "o G9 escolheu um fluxo que a validação recusou; o catálogo vai ao prompt"
                );
                FlowOutcome {
                    texto: block_catalog(&preparation.released, tool_of_beginning),
                    outcome_record: Some(json!({
                        "resultado": "nao_iniciou",
                        "fluxo": slug,
                        "motivo": motivo,
                    })),
                    ..FlowOutcome::default()
                }
                .reservation()
            }
            Err(error) => {
                tracing::warn!(error = %error, fluxo = slug, "erro ao iniciar o fluxo escolhido pelo G9");
                FlowOutcome {
                    texto: block_catalog(&preparation.released, tool_of_beginning),
                    outcome_record: Some(json!({
                        "resultado": "falhou",
                        "fluxo": slug,
                        "motivo": "erro de armazenamento ao iniciar a execução",
                    })),
                    ..FlowOutcome::default()
                }
                .reservation()
            }
        };
    }

    match reason_without_beginning(jev, g9_linked) {
        NoStartReason::None => FlowOutcome {
            outcome_record: Some(outcome_record("nenhum")),
            ..FlowOutcome::default()
        },
        NoStartReason::BelowThreshold => FlowOutcome {
            outcome_record: Some(outcome_record("abaixo_do_limiar")),
            ..FlowOutcome::default()
        },
        NoStartReason::Failed => FlowOutcome {
            texto: block_catalog(&preparation.released, tool_of_beginning),
            outcome_record: Some(outcome_record("fallback")),
            ..FlowOutcome::default()
        }
        .reservation(),
        NoStartReason::NotAsked => FlowOutcome {
            texto: block_catalog(&preparation.released, tool_of_beginning),
            outcome_record: Some(outcome_record("reserva")),
            ..FlowOutcome::default()
        }
        .reservation(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use poph_fluxos::{Limites, compilar};
    use poph_store::{NewAgent, NewConversation, NewFlow, NewOrg, RevisionPublication, Store};
    use poph_types::{AgentId, EventActor, FlowsConfig, OrgId};

    use super::{AdjustmentsFlow, Beginning, Initiator, start};

    const SCRIPT: &str = r#"fluxo(
    id = "agendar_visita",
    nome = "Agendar visita",
    inicio = "coleta",
    gatilho = gatilho(quando = "O cliente quer marcar visita."),
)

etapa(
    id = "coleta",
    nome = "Coleta",
    acoes = [acao("seguir", "Seguir", vai_para = fim("pronto"))],
)
"#;

    struct Scenario {
        folder: std::path::PathBuf,
        store: Arc<Store>,
        org: OrgId,
        agente: AgentId,
        ficha: poph_store::Ficha,
        fluxo: poph_types::FlowId,
    }

    impl Scenario {
        fn new(label: &str) -> Self {
            let folder = std::env::temp_dir().join(format!(
                "poph-fx23-un-{label}-{}-{}",
                std::process::id(),
                AgentId::generate()
            ));
            let _ = std::fs::remove_dir_all(&folder);
            std::fs::create_dir_all(&folder).expect("pasta do teste");
            let store = Arc::new(
                Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
                    .expect("abre o banco"),
            );
            let org = store
                .orgs()
                .create(NewOrg {
                    name: format!("Org {label}"),
                    slug: format!("org-{label}-{}", AgentId::generate()),
                    config_json: "{}".to_owned(),
                })
                .expect("cria a org")
                .id;
            let agente = store
                .agents()
                .create(
                    org,
                    NewAgent {
                        name: "Bia".to_owned(),
                        slug: format!("bia-{label}"),
                        ..NewAgent::default()
                    },
                )
                .expect("cria o agente")
                .id;
            store
                .instructions()
                .save(org, agente, 0, "{}", "# Bia", true)
                .expect("grava as instruções");
            let conversa = store
                .conversations()
                .create(
                    org,
                    NewConversation {
                        agent_id: agente,
                        integration_id: None,
                        canal: "cli".to_owned(),
                        peer_externo: Some("cliente-ficticio".to_owned()),
                        target_department: None,
                    },
                )
                .expect("abre a conversa")
                .id;
            let ficha = store
                .cases()
                .by_conversation(org, conversa)
                .expect("lê a Ficha")
                .expect("a conversa nasce com Ficha");
            let fluxo = store
                .flows()
                .create(
                    org,
                    NewFlow {
                        slug: "agendar_visita".to_owned(),
                        name: "Agendar visita".to_owned(),
                        description: String::new(),
                        rascunho_fonte: SCRIPT.to_owned(),
                    },
                )
                .expect("cria o fluxo");
            let compiled = compilar(SCRIPT, &Limites::default());
            let ir = compiled.ir().expect("o script do teste compila");
            store
                .revisions()
                .publish_revision(
                    org,
                    fluxo.id,
                    RevisionPublication {
                        expected: fluxo.version,
                        fonte: SCRIPT.to_owned(),
                        definicao_json: serde_json::to_string(ir).expect("IR em JSON"),
                        schema_versao: Some(ir.schema_versao),
                        resolucoes_json: "{}".to_owned(),
                        ator_tipo: "system".to_owned(),
                        actor_id: None,
                    },
                )
                .expect("publica a revisão");
            Self {
                folder,
                store,
                org,
                agente,
                ficha,
                fluxo: fluxo.id,
            }
        }

        fn liberar(&self) {
            self.store
                .flows()
                .release(self.org, self.agente, self.fluxo)
                .expect("libera o fluxo");
        }

        fn adjustments(&self) -> AdjustmentsFlow {
            AdjustmentsFlow::of_config(&FlowsConfig::default())
        }
    }

    impl Drop for Scenario {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.folder);
        }
    }

    fn decisao<'a>(choice: &'a str) -> poph_store::JevDecision<'a> {
        poph_store::JevDecision {
            choice,
            confidence: Some(0.95),
        }
    }

    #[test]
    fn abre_a_execution_with_a_decision_jev() {
        let scenario = Scenario::new("abre");
        scenario.liberar();
        let adjustments = scenario.adjustments();
        let beginning = start(
            &adjustments,
            &scenario.store,
            scenario.org,
            &scenario.ficha,
            "agendar_visita",
            Initiator::Jev(decisao("agendar_visita")),
            None,
        )
        .expect("sem erro de armazenamento");
        let Beginning::Open { execution, .. } = beginning else {
            panic!("a execução tinha de abrir");
        };
        assert_eq!(execution.no_atual, "coleta");
        let events = scenario
            .store
            .cases()
            .events(scenario.org, scenario.ficha.id, None, 20)
            .expect("lê os eventos");
        let started = events
            .iter()
            .find(|evento| evento.tipo == poph_types::RecordEventKind::FlowStarted)
            .expect("evento fluxo_iniciado");
        assert_eq!(started.ator_tipo, EventActor::Jev);
        assert!(
            started
                .payload_json
                .contains("\"escolha\":\"agendar_visita\"")
        );
    }

    #[test]
    fn flow_inactive_refused_without_write() {
        let scenario = Scenario::new("inativo");
        scenario.liberar();
        let current = scenario
            .store
            .flows()
            .get(scenario.org, scenario.fluxo)
            .expect("lê o fluxo");
        scenario
            .store
            .flows()
            .update(
                scenario.org,
                scenario.fluxo,
                current.version,
                poph_store::FlowChange {
                    status: Some(poph_store::Status::Inactive),
                    ..poph_store::FlowChange::default()
                },
            )
            .expect("desativa");
        let adjustments = scenario.adjustments();
        let beginning = start(
            &adjustments,
            &scenario.store,
            scenario.org,
            &scenario.ficha,
            "agendar_visita",
            Initiator::Jev(decisao("agendar_visita")),
            None,
        )
        .expect("sem erro de armazenamento");
        let Beginning::Rejected(motivo) = beginning else {
            panic!("fluxo inativo tinha de ser recusado");
        };
        assert!(motivo.contains("e não inicia execução nova"), "{motivo}");
        assert!(
            scenario
                .store
                .executions()
                .from_case(scenario.org, scenario.ficha.id, 10)
                .expect("lê as execuções")
                .is_empty()
        );
    }

    #[test]
    fn flow_not_released_refused() {
        let scenario = Scenario::new("sem-permissao");
        let adjustments = scenario.adjustments();
        let beginning = start(
            &adjustments,
            &scenario.store,
            scenario.org,
            &scenario.ficha,
            "agendar_visita",
            Initiator::Jev(decisao("agendar_visita")),
            None,
        )
        .expect("sem erro de armazenamento");
        let Beginning::Rejected(motivo) = beginning else {
            panic!("fluxo não liberado tinha de ser recusado");
        };
        assert!(
            motivo.contains("para o agente deste atendimento"),
            "{motivo}"
        );
    }

    #[test]
    fn slug_missing_refused() {
        let scenario = Scenario::new("inexistente");
        scenario.liberar();
        let adjustments = scenario.adjustments();
        let beginning = start(
            &adjustments,
            &scenario.store,
            scenario.org,
            &scenario.ficha,
            "cancelar_tudo",
            Initiator::Jev(decisao("cancelar_tudo")),
            None,
        )
        .expect("sem erro de armazenamento");
        let Beginning::Rejected(motivo) = beginning else {
            panic!("slug inexistente tinha de ser recusado");
        };
        assert!(motivo.contains("não existe na organização"), "{motivo}");
    }

    #[test]
    fn flow_without_revision_published_refused() {
        let scenario = Scenario::new("sem-revisao");
        let without_revision = scenario
            .store
            .flows()
            .create(
                scenario.org,
                NewFlow {
                    slug: "sem_revisao".to_owned(),
                    name: "Sem revisão".to_owned(),
                    description: String::new(),
                    rascunho_fonte: String::new(),
                },
            )
            .expect("cria o fluxo");
        scenario
            .store
            .flows()
            .release(scenario.org, scenario.agente, without_revision.id)
            .expect("libera");
        let adjustments = scenario.adjustments();
        let beginning = start(
            &adjustments,
            &scenario.store,
            scenario.org,
            &scenario.ficha,
            "sem_revisao",
            Initiator::Jev(decisao("sem_revisao")),
            None,
        )
        .expect("sem erro de armazenamento");
        let Beginning::Rejected(motivo) = beginning else {
            panic!("fluxo sem revisão tinha de ser recusado");
        };
        assert!(motivo.contains("não tem revisão publicada"), "{motivo}");
    }

    #[test]
    fn case_with_execution_active_returns_already_active() {
        let scenario = Scenario::new("ja-ativa");
        scenario.liberar();
        let adjustments = scenario.adjustments();
        let first = start(
            &adjustments,
            &scenario.store,
            scenario.org,
            &scenario.ficha,
            "agendar_visita",
            Initiator::Jev(decisao("agendar_visita")),
            None,
        )
        .expect("sem erro de armazenamento");
        assert!(matches!(first, Beginning::Open { .. }));
        let second = start(
            &adjustments,
            &scenario.store,
            scenario.org,
            &scenario.ficha,
            "agendar_visita",
            Initiator::Jev(decisao("agendar_visita")),
            None,
        )
        .expect("sem erro de armazenamento");
        assert!(matches!(second, Beginning::AlreadyActive));
        assert_eq!(
            scenario
                .store
                .executions()
                .from_case(scenario.org, scenario.ficha.id, 10)
                .expect("lê as execuções")
                .len(),
            1,
            "uma execução ativa por Ficha"
        );
    }
}
