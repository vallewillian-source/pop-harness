use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use poph_store::{
    ExecutionStart, ExecutionState, FlowTransition, NewAgent, NewConversation, NewOrg, Org, Store,
    StoredExecution, TransitionEffect, TransitionEvent, TransitionExtras,
};
use poph_types::{
    ActionActor, EventActor, ExecutionStatus, FlowId, RecordEventKind, RecordId, RevisionId,
};
use rusqlite::Connection;

struct Scenario {
    folder: PathBuf,
    store: Store,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let folder =
            std::env::temp_dir().join(format!("poph-transicao-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("cria a pasta do teste");
        let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
            .expect("abre o banco");
        Self { folder, store }
    }

    fn connection(&self) -> Connection {
        let conn = Connection::open(self.folder.join("harness.db")).expect("abre para conferir");
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .expect("pragmas");
        conn
    }

    fn org(&self, slug: &str) -> Org {
        self.store
            .orgs()
            .create(NewOrg {
                name: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
    }

    fn agent(&self, org: &Org) -> poph_store::Agente {
        self.store
            .agents()
            .create(
                org.id,
                NewAgent {
                    name: "Agente de medição".to_owned(),
                    slug: "medicao".to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
    }

    fn case(
        &self,
        org: &Org,
        agent: &poph_store::Agente,
        peer: &str,
    ) -> (RecordId, poph_types::ConversationId) {
        let conversation = self
            .store
            .conversations()
            .create(
                org.id,
                NewConversation {
                    agent_id: agent.id,
                    integration_id: None,
                    canal: "console".to_owned(),
                    peer_externo: Some(peer.to_owned()),
                    target_department: None,
                },
            )
            .expect("abre a conversa");
        let case = self
            .store
            .cases()
            .by_conversation(org.id, conversation.id)
            .expect("consulta")
            .expect("a conversa tem Ficha")
            .id;
        (case, conversation.id)
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
        agent: &poph_store::Agente,
        case: RecordId,
        (flow, revision): (FlowId, RevisionId),
    ) -> StoredExecution {
        self.store
            .flows()
            .release(org.id, agent.id, flow)
            .expect("libera o fluxo");
        self.store
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

fn transition_closes(de: &str) -> FlowTransition {
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

fn percentile(sorted: &[u128], p: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[index]
}

fn relatar(label: &str, mut micros: Vec<u128>) {
    let samples = micros.len();
    let total: u128 = micros.iter().sum();
    micros.sort_unstable();
    let average = total as f64 / samples as f64;
    let by_second = if total == 0 {
        0.0
    } else {
        samples as f64 * 1_000_000.0 / total as f64
    };
    println!(
        "{label}: {samples} amostras, p50 {} µs, p95 {} µs, p99 {} µs, média {average:.0} µs ({by_second:.0}/s)",
        percentile(&micros, 0.50),
        percentile(&micros, 0.95),
        percentile(&micros, 0.99),
    );
}

#[test]
#[ignore = "medição de latência: rodar à mão com --ignored --nocapture"]
fn transition_and_conversation_latency_with_case() {
    let scenario = Scenario::new("medicao");
    let org = scenario.org("medicao");
    let agent = scenario.agent(&org);
    let flow = scenario.flow(&org, "medicao");

    {
        let (case, _) = scenario.case(&org, &agent, "cliente-parcial");
        let opened = scenario.open(&org, &agent, case, flow);
        let mut version = opened.execution.version;
        let mut samples = Vec::with_capacity(500);
        for index in 0..500u32 {
            let beginning = Instant::now();
            let stored = scenario
                .store
                .executions()
                .apply_transition(
                    org.id,
                    opened.execution.id,
                    version,
                    &transition_partial("coleta"),
                    &format!("medicao-parcial-{index}"),
                    TransitionExtras::default(),
                )
                .expect("aplica a parcial");
            samples.push(beginning.elapsed().as_micros());
            version = stored.execution.version;
        }
        relatar(
            "transição na etapa (estado + 2 eventos + 1 efeito)",
            samples,
        );
    }

    {
        let (case, _) = scenario.case(&org, &agent, "cliente-avanca");
        let opened = scenario.open(&org, &agent, case, flow);
        let mut version = opened.execution.version;
        let mut samples = Vec::with_capacity(300);
        for index in 0..300u32 {
            let (de, para) = if index % 2 == 0 {
                ("coleta", "confirmacao")
            } else {
                ("confirmacao", "coleta")
            };
            let beginning = Instant::now();
            let stored = scenario
                .store
                .executions()
                .apply_transition(
                    org.id,
                    opened.execution.id,
                    version,
                    &transition_advance(de, para),
                    &format!("medicao-avanca-{index}"),
                    TransitionExtras::default(),
                )
                .expect("aplica o avanço");
            samples.push(beginning.elapsed().as_micros());
            version = stored.execution.version;
        }
        relatar(
            "transição avançando de etapa (estado + 3 eventos + etapa nova)",
            samples,
        );
    }

    {
        let mut samples = Vec::with_capacity(100);
        for index in 0..100u32 {
            let (case, _) = scenario.case(&org, &agent, &format!("cliente-fim-{index}"));
            let opened = scenario.open(&org, &agent, case, flow);
            let beginning = Instant::now();
            let stored = scenario
                .store
                .executions()
                .apply_transition(
                    org.id,
                    opened.execution.id,
                    opened.execution.version,
                    &transition_closes("coleta"),
                    &format!("medicao-fim-{index}"),
                    TransitionExtras::default(),
                )
                .expect("aplica o fim");
            samples.push(beginning.elapsed().as_micros());
            assert!(
                stored.closed_service,
                "o fim de teste encerra o atendimento"
            );
        }
        relatar(
            "transição do fim que encerra o atendimento (fecha conversa e Ficha)",
            samples,
        );
    }

    {
        let mut samples = Vec::with_capacity(200);
        for index in 0..200u32 {
            let beginning = Instant::now();
            scenario
                .store
                .conversations()
                .create(
                    org.id,
                    NewConversation {
                        agent_id: agent.id,
                        integration_id: None,
                        canal: "console".to_owned(),
                        peer_externo: Some(format!("cliente-criacao-{index}")),
                        target_department: None,
                    },
                )
                .expect("cria a conversa");
            samples.push(beginning.elapsed().as_micros());
        }
        relatar("Conversas::criar com a Ficha na mesma escrita", samples);
    }
}
