use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

use poph_store::{
    Agente, ExecutionStart, ExecutionState, FlowTransition, NewAgent, NewConversation, NewOrg, Org,
    Store, StoredExecution, TransitionEffect, TransitionEvent, TransitionExtras,
};
use poph_types::{
    ActionActor, ClosedBy, DomainError, EffectState, EventActor, ExecutionStatus, FlowId,
    RecordEventKind, RecordId, RecordStatus, RevisionId, StepStatus,
};
use rusqlite::Connection;

struct Scenario {
    folder: PathBuf,
    store: Option<Arc<Store>>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder =
            std::env::temp_dir().join(format!("poph-execucoes-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("cria a pasta do teste");
        let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
            .expect("abre o banco");
        Self {
            folder,
            store: Some(Arc::new(store)),
        }
    }

    fn store(&self) -> &Store {
        self.store.as_ref().expect("store viva")
    }

    fn shared(&self) -> Arc<Store> {
        Arc::clone(self.store.as_ref().expect("store viva"))
    }

    fn connection(&self) -> Connection {
        let conn = Connection::open(self.folder.join("harness.db")).expect("abre para conferir");
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .expect("pragmas");
        conn
    }

    fn org(&self, slug: &str) -> Org {
        self.store()
            .orgs()
            .create(NewOrg {
                name: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
    }

    fn agent(&self, org: &Org, slug: &str) -> Agente {
        self.store()
            .agents()
            .create(
                org.id,
                NewAgent {
                    name: format!("Agente {slug}"),
                    slug: slug.to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
    }

    fn case(&self, org: &Org, agent: &Agente) -> (poph_store::Conversa, RecordId) {
        let conversation = self
            .store()
            .conversations()
            .create(
                org.id,
                NewConversation {
                    agent_id: agent.id,
                    integration_id: None,
                    canal: "console".to_owned(),
                    peer_externo: Some("cliente-1".to_owned()),
                    target_department: None,
                },
            )
            .expect("abre a conversa");
        let case = self
            .store()
            .cases()
            .by_conversation(org.id, conversation.id)
            .expect("consulta")
            .expect("a conversa tem Ficha")
            .id;
        (conversation, case)
    }

    fn flow(&self, org: &Org, slug: &str) -> (FlowId, RevisionId) {
        let conn = self.connection();
        let flow: FlowId = uuid::Uuid::now_v7().to_string().parse().expect("fluxo");
        let revision: RevisionId = uuid::Uuid::now_v7().to_string().parse().expect("revisão");
        conn.execute(
            "INSERT INTO fluxos (id, org_id, slug, nome, descricao, status, rascunho_fonte,
                                 revisao_atual, version)
             VALUES (?1, ?2, ?3, ?4, '', 'active', '', 1, 1)",
            rusqlite::params![
                flow.to_string(),
                org.id.to_string(),
                slug,
                format!("Fluxo {slug}")
            ],
        )
        .expect("fluxo");
        conn.execute(
            "INSERT INTO fluxo_revisoes (id, org_id, fluxo_id, numero, fonte, fonte_sha256,
                                        definicao_json, schema_versao, publicada_em, publicada_por,
                                        resolucoes_json)
             VALUES (?1, ?2, ?3, 1, 'fluxo()', 'sha', '{}', 1, 1, 'admin_token', '{}')",
            rusqlite::params![revision.to_string(), org.id.to_string(), flow.to_string()],
        )
        .expect("revisão");
        (flow, revision)
    }

    fn open(
        &self,
        org: &Org,
        agent: &Agente,
        case: RecordId,
        flow: FlowId,
        revision: RevisionId,
    ) -> StoredExecution {
        self.store()
            .flows()
            .release(org.id, agent.id, flow)
            .expect("libera o fluxo");
        self.store()
            .executions()
            .start_execution(
                org.id,
                ExecutionStart {
                    chave_idempotencia: None,
                    case,
                    flow,
                    revision,
                    actor: EventActor::Agent,
                    ator_ref: Some("agente-de-teste"),
                    jev_decision: None,
                    initial_state: state_initial("coleta"),
                    inbound_effects: Vec::new(),
                },
            )
            .expect("abre a execução")
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.folder);
    }
}

fn state_initial(etapa: &str) -> ExecutionState {
    ExecutionState {
        status: ExecutionStatus::Active,
        no_atual: etapa.to_owned(),
        dados_json: "{}".to_owned(),
        acting_actors: Vec::new(),
        result: None,
        reason: None,
    }
}

fn transition_partial(etapa: &str) -> FlowTransition {
    FlowTransition {
        new_state: ExecutionState {
            status: ExecutionStatus::Active,
            no_atual: etapa.to_owned(),
            dados_json: format!("{{\"{etapa}\":{{\"endereco\":\"Rua das Flores, 10\"}}}}"),
            acting_actors: vec![ActionActor::Agent],
            result: None,
            reason: None,
        },
        events: vec![
            TransitionEvent {
                tipo: RecordEventKind::FieldsFilled,
                ator_tipo: EventActor::Agent,
                payload_json: format!(
                    "{{\"etapa\":\"{etapa}\",\"campos\":{{\"endereco\":\"x\"}}}}"
                ),
            },
            TransitionEvent {
                tipo: RecordEventKind::ActionExecuted,
                ator_tipo: EventActor::Agent,
                payload_json: format!(
                    "{{\"etapa\":\"{etapa}\",\"acao\":\"registrar\",\"parcial\":true}}"
                ),
            },
        ],
        effects: vec![TransitionEffect {
            key: format!("acao:{etapa}.registrar/0"),
            tipo: "mensagem_ao_cliente".to_owned(),
            payload_json: "{\"texto\":\"Recebi seu endereço\"}".to_owned(),
        }],
        closes_service: false,
        repeated: false,
        rule_error: false,
    }
}

fn transition_advance(de: &str, para: &str) -> FlowTransition {
    FlowTransition {
        new_state: ExecutionState {
            status: ExecutionStatus::Active,
            no_atual: para.to_owned(),
            dados_json: format!(
                "{{\"{de}\":{{\"endereco\":\"Rua das Flores, 10\"}},\"{para}\":{{}}}}"
            ),
            acting_actors: Vec::new(),
            result: None,
            reason: None,
        },
        events: vec![
            TransitionEvent {
                tipo: RecordEventKind::ActionExecuted,
                ator_tipo: EventActor::Attendant,
                payload_json: format!("{{\"etapa\":\"{de}\",\"acao\":\"confirmar\"}}"),
            },
            TransitionEvent {
                tipo: RecordEventKind::StepCompleted,
                ator_tipo: EventActor::System,
                payload_json: format!("{{\"etapa\":\"{de}\",\"acao\":\"confirmar\"}}"),
            },
            TransitionEvent {
                tipo: RecordEventKind::StepEntered,
                ator_tipo: EventActor::System,
                payload_json: format!("{{\"etapa\":\"{para}\"}}"),
            },
        ],
        effects: Vec::new(),
        closes_service: false,
        repeated: false,
        rule_error: false,
    }
}

fn transition_completes(de: &str) -> FlowTransition {
    FlowTransition {
        new_state: ExecutionState {
            status: ExecutionStatus::Completed,
            no_atual: "concluida".to_owned(),
            dados_json: format!("{{\"{de}\":{{\"endereco\":\"Rua das Flores, 10\"}}}}"),
            acting_actors: Vec::new(),
            result: Some("Visita agendada".to_owned()),
            reason: None,
        },
        events: vec![
            TransitionEvent {
                tipo: RecordEventKind::StepCompleted,
                ator_tipo: EventActor::System,
                payload_json: format!("{{\"etapa\":\"{de}\"}}"),
            },
            TransitionEvent {
                tipo: RecordEventKind::ExecutionCompleted,
                ator_tipo: EventActor::System,
                payload_json: "{\"resultado\":\"Visita agendada\"}".to_owned(),
            },
        ],
        effects: Vec::new(),
        closes_service: true,
        repeated: false,
        rule_error: false,
    }
}

fn without_extras() -> TransitionExtras<'static> {
    TransitionExtras::default()
}

fn status_of_case(scenario: &Scenario, org: &Org, case: RecordId) -> RecordStatus {
    let conn = scenario.connection();
    let texto: String = conn
        .query_row(
            "SELECT status FROM fichas WHERE org_id = ?1 AND id = ?2",
            rusqlite::params![org.id.to_string(), case.to_string()],
            |row| row.get(0),
        )
        .expect("status da ficha");
    RecordStatus::from_text(&texto).expect("situação conhecida")
}

fn total(scenario: &Scenario, table: &str, org: &Org) -> i64 {
    let conn = scenario.connection();
    conn.query_row(
        &format!("SELECT COUNT(*) FROM {table} WHERE org_id = ?1"),
        rusqlite::params![org.id.to_string()],
        |row| row.get(0),
    )
    .expect("conta")
}

#[test]
fn start_writes_execution_first_step_event_and_entry_effects() {
    let scenario = Scenario::new("iniciar");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let (_conversation, case) = scenario.case(&org, &agent);
    let (flow, revision) = scenario.flow(&org, "agendar_visita");
    scenario
        .store()
        .flows()
        .release(org.id, agent.id, flow)
        .expect("libera");

    let stored = scenario
        .store()
        .executions()
        .start_execution(
            org.id,
            ExecutionStart {
                chave_idempotencia: None,
                case,
                flow,
                revision,
                actor: EventActor::Jev,
                ator_ref: Some("jev"),
                jev_decision: Some(poph_store::JevDecision {
                    choice: "agendar_visita",
                    confidence: Some(0.93),
                }),
                initial_state: state_initial("coleta"),
                inbound_effects: vec![TransitionEffect {
                    key: "etapa:coleta/0".to_owned(),
                    tipo: "mensagem_ao_cliente".to_owned(),
                    payload_json: "{\"texto\":\"Vamos agendar sua visita\"}".to_owned(),
                }],
            },
        )
        .expect("inicia");

    assert_eq!(stored.execution.status, ExecutionStatus::Active);
    assert_eq!(stored.execution.no_atual, "coleta");
    assert_eq!(stored.execution.ficha_id, case);
    assert_eq!(stored.execution.revisao_id, revision);
    assert_eq!(stored.execution.version, 1);
    assert_eq!(stored.execution.finalizada_em, None);
    assert!(!stored.closed_service);
    assert!(!stored.repeated);

    assert_eq!(stored.events.len(), 1);
    assert_eq!(stored.events[0].tipo, RecordEventKind::FlowStarted);
    assert_eq!(stored.events[0].ator_tipo, EventActor::Jev);
    assert_eq!(stored.events[0].ator_ref.as_deref(), Some("jev"));
    assert!(
        stored.events[0].payload_json.contains("agendar_visita"),
        "o evento cita o fluxo: {}",
        stored.events[0].payload_json
    );
    assert!(
        stored.events[0].payload_json.contains("0.93"),
        "o evento cita a confiança do Jev: {}",
        stored.events[0].payload_json
    );

    let stages = scenario
        .store()
        .executions()
        .stages(org.id, stored.execution.id)
        .expect("etapas");
    assert_eq!(stages.len(), 1);
    assert_eq!(stages[0].no_id, "coleta");
    assert_eq!(stages[0].ordem, 1);
    assert_eq!(stages[0].status, StepStatus::Pending);
    assert_eq!(stages[0].concluiu_em, None);

    assert_eq!(stored.effects.len(), 1);
    let effects = scenario
        .store()
        .executions()
        .effects(org.id, stored.execution.id)
        .expect("efeitos");
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].key, "etapa:coleta/0");
    assert_eq!(effects[0].state, EffectState::Pending);
    assert_eq!(effects[0].attempts, 0);
}

#[test]
fn transition_writes_state_step_event_and_effect_in_one_write() {
    let scenario = Scenario::new("transicao");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let (_conversation, case) = scenario.case(&org, &agent);
    let (flow, revision) = scenario.flow(&org, "agendar_visita");
    let opened = scenario.open(&org, &agent, case, flow, revision);

    let partial = scenario
        .store()
        .executions()
        .apply_transition(
            org.id,
            opened.execution.id,
            opened.execution.version,
            &transition_partial("coleta"),
            "chave-parcial",
            without_extras(),
        )
        .expect("aplica a parcial");
    assert_eq!(partial.execution.no_atual, "coleta");
    assert_eq!(partial.execution.version, 2);
    assert_eq!(partial.execution.status, ExecutionStatus::Active);
    assert_eq!(partial.events.len(), 2);
    assert!(!partial.repeated);

    let stages = scenario
        .store()
        .executions()
        .stages(org.id, opened.execution.id)
        .expect("etapas");
    assert_eq!(stages.len(), 1, "não entrou em etapa nova");
    assert_eq!(stages[0].status, StepStatus::Partial);
    assert_eq!(stages[0].acting_actors, vec![ActionActor::Agent]);
    assert!(
        stages[0].dados_json.contains("Rua das Flores"),
        "campos da etapa"
    );

    let effects = scenario
        .store()
        .executions()
        .effects(org.id, opened.execution.id)
        .expect("efeitos");
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].key, "acao:coleta.registrar/0");
    assert_eq!(effects[0].tipo, "mensagem_ao_cliente");
    assert_eq!(effects[0].state, EffectState::Pending);

    let advanced = scenario
        .store()
        .executions()
        .apply_transition(
            org.id,
            opened.execution.id,
            partial.execution.version,
            &transition_advance("coleta", "confirmacao"),
            "chave-avanca",
            without_extras(),
        )
        .expect("aplica o avanço");
    assert_eq!(advanced.execution.no_atual, "confirmacao");
    assert_eq!(advanced.execution.version, 3);

    let stages = scenario
        .store()
        .executions()
        .stages(org.id, opened.execution.id)
        .expect("etapas");
    assert_eq!(stages.len(), 2, "a etapa nova abre uma entrada");
    assert_eq!(stages[0].no_id, "coleta");
    assert_eq!(stages[0].status, StepStatus::Completed);
    assert!(
        stages[0].concluiu_em.is_some(),
        "etapa concluída tem carimbo"
    );
    assert_eq!(stages[1].no_id, "confirmacao");
    assert_eq!(stages[1].ordem, 2);
    assert_eq!(stages[1].status, StepStatus::Pending);

    let events = scenario
        .store()
        .cases()
        .events(org.id, case, None, 100)
        .expect("eventos");
    let seqs: Vec<i64> = events.iter().map(|evento| evento.seq).collect();
    let expected_value: Vec<i64> = (1..=seqs.len() as i64).collect();
    assert_eq!(seqs, expected_value, "seq contínuo de 1 até o fim");
    assert_eq!(
        events
            .iter()
            .filter(|evento| evento.tipo == RecordEventKind::StepCompleted)
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|evento| evento.tipo == RecordEventKind::StepEntered)
            .count(),
        1
    );
}

#[test]
fn failure_midway_leaves_no_event_without_state() {
    let scenario = Scenario::new("atômica");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let (_conversation, case) = scenario.case(&org, &agent);
    let (flow, revision) = scenario.flow(&org, "agendar_visita");
    let opened = scenario.open(&org, &agent, case, flow, revision);

    let events_before = total(&scenario, "ficha_eventos", &org);
    let steps_before = total(&scenario, "fluxo_execucao_etapas", &org);
    let effects_before = total(&scenario, "fluxo_efeitos", &org);

    let mut quebrada = transition_advance("coleta", "confirmacao");
    quebrada.effects.push(TransitionEffect {
        key: "acao:coleta.confirmar/0".to_owned(),
        tipo: "tipo_que_a_coluna_recusa".to_owned(),
        payload_json: "{}".to_owned(),
    });
    let error = scenario
        .store()
        .executions()
        .apply_transition(
            org.id,
            opened.execution.id,
            opened.execution.version,
            &quebrada,
            "chave-quebrada",
            without_extras(),
        )
        .expect_err("a transição falha no efeito");
    assert!(
        matches!(
            error,
            poph_store::StoreError::Domain(DomainError::Validation { field, .. })
                if field == "fluxo_efeitos"
        ),
        "erro do CHECK do tipo do efeito: {error:?}"
    );

    let later = scenario
        .store()
        .executions()
        .get(org.id, opened.execution.id)
        .expect("lê a execução");
    assert_eq!(later.version, opened.execution.version);
    assert_eq!(later.no_atual, "coleta");
    assert_eq!(later.status, ExecutionStatus::Active);
    assert_eq!(total(&scenario, "ficha_eventos", &org), events_before);
    assert_eq!(
        total(&scenario, "fluxo_execucao_etapas", &org),
        steps_before
    );
    assert_eq!(total(&scenario, "fluxo_efeitos", &org), effects_before);

    let certa = scenario
        .store()
        .executions()
        .apply_transition(
            org.id,
            opened.execution.id,
            opened.execution.version,
            &transition_advance("coleta", "confirmacao"),
            "chave-quebrada",
            without_extras(),
        )
        .expect("a chave não foi marcada pela transição que falhou");
    assert_eq!(certa.execution.no_atual, "confirmacao");
    assert!(!certa.repeated);
}

#[test]
fn old_version_gets_412_and_changes_nothing() {
    let scenario = Scenario::new("versao");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let (_conversation, case) = scenario.case(&org, &agent);
    let (flow, revision) = scenario.flow(&org, "agendar_visita");
    let opened = scenario.open(&org, &agent, case, flow, revision);

    let first = scenario
        .store()
        .executions()
        .apply_transition(
            org.id,
            opened.execution.id,
            1,
            &transition_partial("coleta"),
            "chave-1",
            without_extras(),
        )
        .expect("a primeira vence");
    assert_eq!(first.execution.version, 2);

    let error = scenario
        .store()
        .executions()
        .apply_transition(
            org.id,
            opened.execution.id,
            1,
            &transition_partial("coleta"),
            "chave-2",
            without_extras(),
        )
        .expect_err("versão velha é recusada");
    match error {
        poph_store::StoreError::Domain(DomainError::VersionConflict {
            expected, current, ..
        }) => {
            assert_eq!(expected, 1);
            assert_eq!(current, 2);
        }
        another => panic!("esperado conflito de versão, veio {another:?}"),
    }
    assert_eq!(
        DomainError::VersionConflict {
            entity: "execução de fluxo",
            id: opened.execution.id.to_string(),
            expected: 1,
            current: 2,
        }
        .http_status(),
        412
    );

    let later = scenario
        .store()
        .executions()
        .get(org.id, opened.execution.id)
        .expect("lê");
    assert_eq!(later.version, 2);
    let events = scenario
        .store()
        .cases()
        .events(org.id, case, None, 100)
        .expect("eventos");
    assert_eq!(
        events
            .iter()
            .filter(|evento| evento.tipo == RecordEventKind::ActionExecuted)
            .count(),
        1,
        "a recusa não grava evento"
    );
}

#[test]
fn repeating_key_returns_outcome_without_duplicating_event_nor_effect() {
    let scenario = Scenario::new("repetida");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let (_conversation, case) = scenario.case(&org, &agent);
    let (flow, revision) = scenario.flow(&org, "agendar_visita");
    let opened = scenario.open(&org, &agent, case, flow, revision);

    let first = scenario
        .store()
        .executions()
        .apply_transition(
            org.id,
            opened.execution.id,
            opened.execution.version,
            &transition_partial("coleta"),
            "chave-repetida",
            without_extras(),
        )
        .expect("a primeira aplica");
    assert!(!first.repeated);
    let events_before = total(&scenario, "ficha_eventos", &org);
    let effects_before = total(&scenario, "fluxo_efeitos", &org);

    let repeated_item = scenario
        .store()
        .executions()
        .apply_transition(
            org.id,
            opened.execution.id,
            opened.execution.version,
            &transition_partial("coleta"),
            "chave-repetida",
            without_extras(),
        )
        .expect("repetir não é erro");
    assert!(repeated_item.repeated, "marcada como repetida");
    assert_eq!(repeated_item.execution.version, first.execution.version);
    assert!(repeated_item.events.is_empty());
    assert!(repeated_item.effects.is_empty());
    assert_eq!(total(&scenario, "ficha_eventos", &org), events_before);
    assert_eq!(total(&scenario, "fluxo_efeitos", &org), effects_before);

    let from_engine = scenario
        .store()
        .executions()
        .apply_transition(
            org.id,
            opened.execution.id,
            first.execution.version,
            &FlowTransition {
                new_state: state_initial("coleta"),
                events: Vec::new(),
                effects: Vec::new(),
                closes_service: false,
                repeated: true,
                rule_error: false,
            },
            "outra-chave",
            without_extras(),
        )
        .expect("repetida do motor");
    assert!(from_engine.repeated);
    assert_eq!(total(&scenario, "ficha_eventos", &org), events_before);
}

#[test]
fn start_refused_with_closed_case() {
    let scenario = Scenario::new("ficha-fechada");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let (conversation, case) = scenario.case(&org, &agent);
    let (flow, revision) = scenario.flow(&org, "agendar_visita");
    scenario
        .store()
        .flows()
        .release(org.id, agent.id, flow)
        .expect("libera");

    scenario
        .store()
        .conversations()
        .close_audited(
            org.id,
            conversation.id,
            conversation.version,
            "token".to_owned(),
        )
        .expect("fecha");
    assert_eq!(status_of_case(&scenario, &org, case), RecordStatus::Closed);

    let error = scenario
        .store()
        .executions()
        .start_execution(
            org.id,
            ExecutionStart {
                chave_idempotencia: None,
                case,
                flow,
                revision,
                actor: EventActor::Agent,
                ator_ref: None,
                jev_decision: None,
                initial_state: state_initial("coleta"),
                inbound_effects: Vec::new(),
            },
        )
        .expect_err("Ficha fechada recusa execução nova");
    assert!(
        matches!(
            error,
            poph_store::StoreError::Domain(DomainError::RecordClosed { .. })
        ),
        "veio {error:?}"
    );
}

#[test]
fn start_refused_with_unreleased_flow() {
    let scenario = Scenario::new("nao-liberado");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let (_conversation, case) = scenario.case(&org, &agent);
    let (flow, revision) = scenario.flow(&org, "agendar_visita");

    let error = scenario
        .store()
        .executions()
        .start_execution(
            org.id,
            ExecutionStart {
                chave_idempotencia: None,
                case,
                flow,
                revision,
                actor: EventActor::Agent,
                ator_ref: None,
                jev_decision: None,
                initial_state: state_initial("coleta"),
                inbound_effects: Vec::new(),
            },
        )
        .expect_err("sem permissão não abre");
    match error {
        poph_store::StoreError::Domain(DomainError::FlowNotAllowed { fluxo_id, agent_id }) => {
            assert_eq!(fluxo_id, flow.to_string());
            assert_eq!(agent_id, agent.id.to_string());
        }
        another => panic!("esperado fluxo não liberado, veio {another:?}"),
    }
}

#[test]
fn start_refused_with_inactive_flow() {
    let scenario = Scenario::new("inativo");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let (_conversation, case) = scenario.case(&org, &agent);
    let (flow, revision) = scenario.flow(&org, "agendar_visita");
    scenario
        .store()
        .flows()
        .release(org.id, agent.id, flow)
        .expect("libera");
    let conn = scenario.connection();
    conn.execute(
        "UPDATE fluxos SET status = 'inactive' WHERE org_id = ?1 AND id = ?2",
        rusqlite::params![org.id.to_string(), flow.to_string()],
    )
    .expect("inativa o fluxo");

    let error = scenario
        .store()
        .executions()
        .start_execution(
            org.id,
            ExecutionStart {
                chave_idempotencia: None,
                case,
                flow,
                revision,
                actor: EventActor::Agent,
                ator_ref: None,
                jev_decision: None,
                initial_state: state_initial("coleta"),
                inbound_effects: Vec::new(),
            },
        )
        .expect_err("fluxo inativo não abre");
    match error {
        poph_store::StoreError::Domain(DomainError::Conflict { entity, reason, .. }) => {
            assert_eq!(entity, "fluxo");
            assert!(reason.contains("inativo"), "motivo: {reason}");
        }
        another => panic!("esperado conflito de fluxo inativo, veio {another:?}"),
    }
}

#[test]
fn start_refused_with_second_active_execution() {
    let scenario = Scenario::new("segunda");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let (_conversation, case) = scenario.case(&org, &agent);
    let (flow, revision) = scenario.flow(&org, "agendar_visita");
    let opened = scenario.open(&org, &agent, case, flow, revision);

    let error = scenario
        .store()
        .executions()
        .start_execution(
            org.id,
            ExecutionStart {
                chave_idempotencia: None,
                case,
                flow,
                revision,
                actor: EventActor::Agent,
                ator_ref: None,
                jev_decision: None,
                initial_state: state_initial("coleta"),
                inbound_effects: Vec::new(),
            },
        )
        .expect_err("uma execução ativa por Ficha (PRD L6)");
    match error {
        poph_store::StoreError::Domain(DomainError::ExecutionAlreadyActive {
            ficha_id,
            execucao_id,
        }) => {
            assert_eq!(ficha_id, case.to_string());
            assert_eq!(execucao_id, opened.execution.id.to_string());
        }
        another => panic!("esperado execução já ativa, veio {another:?}"),
    }
    assert_eq!(
        scenario
            .store()
            .executions()
            .from_case(org.id, case, 10)
            .expect("histórico")
            .len(),
        1
    );
}

#[test]
fn end_with_close_service_closes_conversation_and_case_in_same_write() {
    let scenario = Scenario::new("encerra");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let (conversation, case) = scenario.case(&org, &agent);
    let (flow, revision) = scenario.flow(&org, "agendar_visita");
    let opened = scenario.open(&org, &agent, case, flow, revision);

    let stored = scenario
        .store()
        .executions()
        .apply_transition(
            org.id,
            opened.execution.id,
            opened.execution.version,
            &transition_completes("coleta"),
            "chave-fim",
            TransitionExtras {
                ator_ref: Some("atendente-1"),
                ..TransitionExtras::default()
            },
        )
        .expect("aplica o fim");

    assert_eq!(stored.execution.status, ExecutionStatus::Completed);
    assert_eq!(stored.execution.no_atual, "concluida");
    assert_eq!(stored.execution.result.as_deref(), Some("Visita agendada"));
    assert!(stored.execution.finalizada_em.is_some());
    assert!(stored.closed_service);

    let case_stored = scenario
        .store()
        .cases()
        .get(org.id, case)
        .expect("lê a Ficha");
    assert_eq!(case_stored.status, RecordStatus::Closed);
    assert_eq!(case_stored.fechada_por, Some(ClosedBy::Flow));
    assert_eq!(
        case_stored.fechada_motivo.as_deref(),
        Some("Visita agendada")
    );

    let closed = scenario
        .store()
        .conversations()
        .get(org.id, conversation.id)
        .expect("lê a conversa");
    assert_eq!(
        closed.status,
        poph_store::ConversationStatus::Closed,
        "a conversa fecha junto"
    );

    let events = scenario
        .store()
        .cases()
        .events(org.id, case, None, 100)
        .expect("eventos");
    let kinds: Vec<RecordEventKind> = events.iter().map(|evento| evento.tipo).collect();
    assert!(kinds.contains(&RecordEventKind::ExecutionCompleted));
    assert_eq!(
        kinds.last().copied(),
        Some(RecordEventKind::RecordClosed),
        "o fechamento é o último evento"
    );
    let closing = events.last().expect("último evento");
    assert_eq!(closing.tipo, RecordEventKind::RecordClosed);
    assert!(closing.payload_json.contains("Visita agendada"));
}

#[test]
fn empty_idempotency_key_is_refused() {
    let scenario = Scenario::new("chave-vazia");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let (_conversation, case) = scenario.case(&org, &agent);
    let (flow, revision) = scenario.flow(&org, "agendar_visita");
    let opened = scenario.open(&org, &agent, case, flow, revision);

    for empty in ["", "   "] {
        let error = scenario
            .store()
            .executions()
            .apply_transition(
                org.id,
                opened.execution.id,
                opened.execution.version,
                &transition_partial("coleta"),
                empty,
                without_extras(),
            )
            .expect_err("chave vazia é recusada");
        assert!(
            matches!(
                error,
                poph_store::StoreError::Domain(DomainError::Validation {
                    field: "chave_idempotencia",
                    ..
                })
            ),
            "veio {error:?}"
        );
    }
    assert_eq!(
        scenario
            .store()
            .executions()
            .get(org.id, opened.execution.id)
            .expect("lê")
            .version,
        1
    );
}

#[test]
fn incoherent_transition_that_closes_without_completing_is_refused() {
    let scenario = Scenario::new("incoerente");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let (_conversation, case) = scenario.case(&org, &agent);
    let (flow, revision) = scenario.flow(&org, "agendar_visita");
    let opened = scenario.open(&org, &agent, case, flow, revision);

    let incoerente = FlowTransition {
        closes_service: true,
        ..transition_partial("coleta")
    };
    let error = scenario
        .store()
        .executions()
        .apply_transition(
            org.id,
            opened.execution.id,
            opened.execution.version,
            &incoerente,
            "chave-incoerente",
            without_extras(),
        )
        .expect_err("encerrar sem concluir é recusado");
    assert!(
        matches!(
            error,
            poph_store::StoreError::Domain(DomainError::Validation {
                field: "encerra_atendimento",
                ..
            })
        ),
        "veio {error:?}"
    );
    assert_eq!(status_of_case(&scenario, &org, case), RecordStatus::Open);
    assert_eq!(
        scenario
            .store()
            .executions()
            .get(org.id, opened.execution.id)
            .expect("lê")
            .version,
        1
    );
}

#[test]
fn closed_execution_accepts_no_transition() {
    let scenario = Scenario::new("encerrada");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let (_conversation, case) = scenario.case(&org, &agent);
    let (flow, revision) = scenario.flow(&org, "agendar_visita");
    let opened = scenario.open(&org, &agent, case, flow, revision);

    let conclui = scenario
        .store()
        .executions()
        .apply_transition(
            org.id,
            opened.execution.id,
            opened.execution.version,
            &FlowTransition {
                closes_service: false,
                ..transition_completes("coleta")
            },
            "chave-conclui",
            without_extras(),
        )
        .expect("conclui sem encerrar");
    assert_eq!(conclui.execution.status, ExecutionStatus::Completed);
    assert!(!conclui.closed_service);
    assert_eq!(status_of_case(&scenario, &org, case), RecordStatus::Open);

    let error = scenario
        .store()
        .executions()
        .apply_transition(
            org.id,
            opened.execution.id,
            conclui.execution.version,
            &transition_partial("coleta"),
            "chave-depois",
            without_extras(),
        )
        .expect_err("execução concluída não aceita transição");
    match error {
        poph_store::StoreError::Domain(DomainError::Conflict { entity, reason, .. }) => {
            assert_eq!(entity, "execução de fluxo");
            assert!(reason.contains("concluida"), "motivo: {reason}");
        }
        another => panic!("esperado conflito, veio {another:?}"),
    }

    let (other_flow, other_revision) = scenario.flow(&org, "confirmar_entrega");
    scenario
        .store()
        .flows()
        .release(org.id, agent.id, other_flow)
        .expect("libera o segundo");
    let second = scenario
        .store()
        .executions()
        .start_execution(
            org.id,
            ExecutionStart {
                chave_idempotencia: None,
                case,
                flow: other_flow,
                revision: other_revision,
                actor: EventActor::Agent,
                ator_ref: None,
                jev_decision: None,
                initial_state: state_initial("entrega"),
                inbound_effects: Vec::new(),
            },
        )
        .expect("fluxo novo na mesma Ficha");
    let history = scenario
        .store()
        .executions()
        .from_case(org.id, case, 10)
        .expect("histórico");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].id, opened.execution.id);
    assert_eq!(history[1].id, second.execution.id);
}

#[test]
fn two_threads_agent_and_attendant_only_one_wins() {
    let scenario = Scenario::new("threads");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let (_conversation, case) = scenario.case(&org, &agent);
    let (flow, revision) = scenario.flow(&org, "agendar_visita");
    let opened = scenario.open(&org, &agent, case, flow, revision);

    let store: Arc<Store> = scenario.shared();
    let barreira = Arc::new(Barrier::new(2));
    let expected = opened.execution.version;
    let id = opened.execution.id;

    let mut results = Vec::new();
    for (label, actor) in [
        ("agente", EventActor::Agent),
        ("atendente", EventActor::Attendant),
    ] {
        let store = Arc::clone(&store);
        let barreira = Arc::clone(&barreira);
        let mut transition = transition_partial("coleta");
        transition.events[0].ator_tipo = actor;
        transition.new_state.acting_actors = vec![match actor {
            EventActor::Agent => ActionActor::Agent,
            _ => ActionActor::Attendant,
        }];
        let chave = format!("chave-{label}");
        results.push(std::thread::spawn(move || {
            barreira.wait();
            store.executions().apply_transition(
                org.id,
                id,
                expected,
                &transition,
                &chave,
                TransitionExtras::default(),
            )
        }));
    }
    let outputs: Vec<_> = results
        .into_iter()
        .map(|thread| thread.join().expect("a thread não entra em pânico"))
        .collect();

    let vencedoras = outputs.iter().filter(|saida| saida.is_ok()).count();
    let perdedoras = outputs
        .iter()
        .filter(|saida| {
            matches!(
                saida,
                Err(poph_store::StoreError::Domain(
                    DomainError::VersionConflict { .. }
                ))
            )
        })
        .count();
    assert_eq!(vencedoras, 1, "só uma transição vence: {outputs:?}");
    assert_eq!(perdedoras, 1, "a outra recebe 412: {outputs:?}");

    let final_ = store.executions().get(org.id, id).expect("lê");
    assert_eq!(final_.version, opened.execution.version + 1);
    let events = store
        .cases()
        .events(org.id, case, None, 100)
        .expect("eventos");
    assert_eq!(
        events
            .iter()
            .filter(|evento| evento.tipo == RecordEventKind::ActionExecuted)
            .count(),
        1
    );
    assert_eq!(
        total(&scenario, "fluxo_efeitos", &org),
        1,
        "o efeito da transição vencedora entrou uma vez só"
    );
}

#[test]
fn execution_from_other_org_is_neither_seen_nor_written() {
    let scenario = Scenario::new("isolamento");
    let org_a = scenario.org("a");
    let org_b = scenario.org("b");
    let agent_a = scenario.agent(&org_a, "a");
    let (_conversation, case) = scenario.case(&org_a, &agent_a);
    let (flow, revision) = scenario.flow(&org_a, "agendar_visita");
    let opened = scenario.open(&org_a, &agent_a, case, flow, revision);

    let error = scenario
        .store()
        .executions()
        .get(org_b.id, opened.execution.id)
        .expect_err("outra org não vê");
    assert!(
        matches!(
            error,
            poph_store::StoreError::Domain(DomainError::NotFound { .. })
        ),
        "veio {error:?}"
    );
    assert!(
        scenario
            .store()
            .executions()
            .from_case(org_b.id, case, 10)
            .expect("lista vazia")
            .is_empty()
    );
    assert!(
        scenario
            .store()
            .executions()
            .active_of_case(org_b.id, case)
            .expect("nada")
            .is_none()
    );
    let error = scenario
        .store()
        .executions()
        .apply_transition(
            org_b.id,
            opened.execution.id,
            opened.execution.version,
            &transition_partial("coleta"),
            "chave-b",
            without_extras(),
        )
        .expect_err("outra org não escreve");
    assert!(
        matches!(
            error,
            poph_store::StoreError::Domain(DomainError::NotFound { .. })
        ),
        "veio {error:?}"
    );

    let later = scenario
        .store()
        .executions()
        .get(org_a.id, opened.execution.id)
        .expect("lê");
    assert_eq!(later.version, 1);
    assert_eq!(later.no_atual, "coleta");
    assert_eq!(total(&scenario, "fluxo_efeitos", &org_a), 0);
    assert_eq!(total(&scenario, "fluxo_efeitos", &org_b), 0);
}
