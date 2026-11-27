use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use poph_agent::base_knowledge::{KnowledgeSearcher, SearchFuture, SearchParameters};
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::turno::{
    FuturePause, NoopObserver, Pause, TurnEngine, TurnInput, TurnOptions, TurnSummary,
};
use poph_core::llm::{FakeProvider, Script};
use poph_jev::{
    FakeJev, G1_CONTINUE, G1_TRANSFER, G4, GateEvaluator, IdGate, JevEngine, JevError, JevMessage,
    JevState,
};
use poph_store::{
    AgentChange, Direction, NewAgent, NewConversation, NewDepartment, NewMessage, NewOrg, Role,
    Store, TurnState,
};
use poph_types::{AgentId, ConversationId, JevGates, MessageId, OrgId};

struct NoWait;

impl Pause for NoWait {
    fn wait(&self, _duration: Duration) -> FuturePause {
        Box::pin(async {})
    }
}

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-hr28-{label}-{}-{}",
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
                    name: format!("Bia {label}"),
                    slug: format!("bia-{label}"),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        store
            .instructions()
            .save(
                org,
                agente,
                0,
                "{}",
                "# Bia\n\nVocê é a atendente da FixProLar.",
                true,
            )
            .expect("grava as instruções");
        store
            .agents()
            .update(
                org,
                agente,
                1,
                AgentChange {
                    handoff_active: Some(true),
                    ..AgentChange::default()
                },
            )
            .expect("liga o handoff");
        Self {
            folder,
            store,
            org,
            agente,
        }
    }

    fn conversa(&self) -> ConversationId {
        self.store
            .conversations()
            .create(
                self.org,
                NewConversation {
                    agent_id: self.agente,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("abre a conversa")
            .id
    }

    fn say(&self, conversa: ConversationId, texto: &str) -> MessageId {
        self.store
            .messages()
            .write(
                self.org,
                NewMessage {
                    conversation_id: conversa,
                    direction: Direction::Inbound,
                    role: Role::User,
                    texto: texto.to_owned(),
                    attachments_json: "[]".to_owned(),
                    external_id: None,
                    turn_id: None,
                    origin: poph_store::Origin::Agente,
                    author_name: None,
                },
            )
            .expect("grava a mensagem")
            .id
    }

    fn departamento(&self, nome: &str) {
        self.store
            .departments()
            .create(
                self.org,
                NewDepartment {
                    name: nome.to_owned(),
                    description: String::new(),
                },
            )
            .expect("cria o departamento");
    }

    fn adjust_judge_agent(&self, jev_json: &str) {
        let current = self
            .store
            .agents()
            .get(self.org, self.agente)
            .expect("lê o agente");
        self.store
            .agents()
            .update(
                self.org,
                self.agente,
                current.version,
                AgentChange {
                    jev_json: Some(jev_json.to_owned()),
                    ..AgentChange::default()
                },
            )
            .expect("ajusta o jev_json do agente");
    }

    fn engine(&self, globals: JevGates, jev: Option<Arc<JevEngine>>) -> TurnEngine {
        let config = ConfigHot::new(
            Arc::clone(&self.store),
            HotConfigOptions::default(),
            MetricsMemory::own(),
        );
        let limiter = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
        let provedor = Arc::new(FakeProvider::with_scripts(vec![script(
            "Olá! Como posso ajudar?",
        )]));
        let _ = globals;
        let mut engine = TurnEngine::new(
            Arc::clone(&self.store),
            config,
            limiter,
            provedor,
            TurnOptions::default(),
            MetricsTurn::own(),
        )
        .with_pause(Arc::new(NoWait));
        if let Some(jev) = jev {
            engine = engine.with_jev(jev);
        }
        engine
    }

    async fn turno(
        &self,
        engine: &TurnEngine,
        conversa: ConversationId,
        gatilho: MessageId,
    ) -> TurnSummary {
        engine
            .run(
                TurnInput::new(self.org, conversa, gatilho),
                &mut NoopObserver,
            )
            .await
            .expect("o turno roda")
    }

    fn last_turn(&self, conversa: ConversationId) -> poph_store::Turno {
        self.store
            .turns()
            .list(self.org, conversa, poph_store::Page::with_limit(10))
            .expect("lista os turnos")
            .into_iter()
            .next()
            .expect("há um turno gravado")
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

fn script(texto: &str) -> Script {
    Script::Events(vec![
        poph_core::llm::LlmEvent::Texto(texto.to_owned()),
        poph_core::llm::LlmEvent::Usage(poph_core::llm::Usage {
            input: 10,
            saida: 5,
        }),
        poph_core::llm::LlmEvent::Fim(poph_core::llm::FinishReason::Completed),
    ])
}

fn so_g1() -> JevGates {
    JevGates {
        g1: true,
        ..JevGates::default()
    }
}

fn choices(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(id, choice)| ((*id).to_owned(), (*choice).to_owned()))
        .collect()
}

fn engine_judge(jev: Arc<FakeJev>, globals: JevGates) -> Arc<JevEngine> {
    Arc::new(JevEngine::new(jev, globals, "jev-latest"))
}

#[tokio::test]
async fn turn_records_decision_judge_record() {
    let scenario = Scenario::new("registro");
    let conversa = scenario.conversa();
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g1_transferir",
        G1_CONTINUE,
    )])));
    let engine = scenario.engine(so_g1(), Some(engine_judge(Arc::clone(&jev), so_g1())));

    let gatilho = scenario.say(conversa, "queria saber o prazo de entrega");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, "Olá! Como posso ajudar?");
    let jev = summary.jev.expect("o turno registra o Jev");
    assert_eq!(jev.choice(IdGate::G1), Some(G1_CONTINUE));
    assert_eq!(jev.call.questions, 1);
    assert_eq!(jev.call.decided_by_cache, 0);

    let turno = scenario.last_turn(conversa);
    let stored: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("jev_json é JSON");
    assert_eq!(stored["decisoes"][0]["gate"], "g1");
    assert_eq!(stored["decisoes"][0]["escolha"], G1_CONTINUE);
    assert_eq!(stored["chamada"]["perguntas"], 1);
}

#[tokio::test]
async fn state_turn_takes_conversation_policy() {
    let scenario = Scenario::new("estado");
    let conversa = scenario.conversa();
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g1_transferir",
        G1_TRANSFER,
    )])));
    let engine = scenario.engine(so_g1(), Some(engine_judge(Arc::clone(&jev), so_g1())));

    let gatilho = scenario.say(conversa, "quero falar com um atendente agora");
    let summary = scenario.turno(&engine, conversa, gatilho).await;
    assert_eq!(
        summary.jev.as_ref().and_then(|jev| jev.choice(IdGate::G1)),
        Some(G1_TRANSFER)
    );

    let request = &jev.requests()[0];
    assert_eq!(request.estado["canal"], "cli");
    assert_eq!(
        request.estado["conversa"]["mensagens"][0]["texto"],
        "quero falar com um atendente agora"
    );
    assert_eq!(request.estado["politica"]["transferencia_habilitada"], true);
    assert!(request.questions.contains_key("g1_transferir"));
}

#[tokio::test]
async fn gates_disabled_not_generate_question() {
    let scenario = Scenario::new("desligado");
    let conversa = scenario.conversa();
    let jev = Arc::new(FakeJev::with_choices(BTreeMap::new()));
    let engine = scenario.engine(
        JevGates::default(),
        Some(engine_judge(Arc::clone(&jev), JevGates::default())),
    );

    let gatilho = scenario.say(conversa, "bom dia");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert!(summary.jev.is_none());
    assert_eq!(jev.calls(), 0);
    assert_eq!(scenario.last_turn(conversa).jev_json, "{}");
}

#[tokio::test]
async fn adjustment_agent_disables_gate_turn() {
    let scenario = Scenario::new("ajuste");
    let conversa = scenario.conversa();
    scenario.adjust_judge_agent(r#"{"gates": {"g1": false}}"#);
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g1_transferir",
        G1_TRANSFER,
    )])));
    let engine = scenario.engine(so_g1(), Some(engine_judge(Arc::clone(&jev), so_g1())));

    let gatilho = scenario.say(conversa, "quero falar com um atendente");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(jev.calls(), 0, "o agente desligou o G1");
    assert!(summary.jev.is_none());
}

#[tokio::test]
async fn failure_judge_not_crashes_turn() {
    let scenario = Scenario::new("falha");
    let conversa = scenario.conversa();
    let jev = Arc::new(FakeJev::failing(JevError::Status {
        code: 503,
        motivo: "Service Unavailable".to_owned(),
    }));
    let engine = scenario.engine(so_g1(), Some(engine_judge(Arc::clone(&jev), so_g1())));

    let gatilho = scenario.say(conversa, "quero falar com um atendente");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, "Olá! Como posso ajudar?");
    let jev = summary.jev.expect("o turno registra a tentativa");
    assert!(jev.failed());
    assert_eq!(jev.choice(IdGate::G1), Some(G1_CONTINUE), "fallback do G1");

    let turno = scenario.last_turn(conversa);
    let stored: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    assert!(
        stored["chamada"]["falha"]
            .as_str()
            .unwrap_or_default()
            .contains("503")
    );
    assert_eq!(stored["decisoes"][0]["origem"], "fallback");
}

#[tokio::test]
async fn g4_takes_departments_active_org() {
    let scenario = Scenario::new("g4");
    scenario.departamento("Comercial");
    scenario.departamento("Suporte");
    let conversa = scenario.conversa();
    let globals = JevGates {
        g4: true,
        ..JevGates::default()
    };
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g4_departamento",
        "Suporte",
    )])));
    let engine = scenario.engine(globals, Some(engine_judge(Arc::clone(&jev), globals)));

    let gatilho = scenario.say(conversa, "meu pedido chegou quebrado");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(
        summary.jev.as_ref().and_then(|jev| jev.choice(IdGate::G4)),
        Some("Suporte")
    );
    let request = &jev.requests()[0];
    assert_eq!(request.questions.len(), 1, "só o G4 está ligado");
    let rules = &request.questions["g4_departamento"].rubric;
    assert!(rules.contains_key("Comercial"));
    assert!(rules.contains_key("Suporte"));
    assert_eq!(
        request.estado["politica"]["departamentos"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
}

#[tokio::test]
async fn state_has_maximum_ten_messages() {
    let estado = JevState::new("cli")
        .with_messages((0..15).map(|i| JevMessage::from_customer(format!("m{i}"))));
    assert_eq!(estado.messages.len(), 10);
    assert_eq!(estado.messages[0].texto, "m5");
    assert_eq!(estado.last_message().map(|m| m.texto.as_str()), Some("m14"));

    let long = JevState::new("cli").with_messages([JevMessage::from_customer("a".repeat(2000))]);
    assert!(long.messages[0].texto.chars().count() <= 801);
}

#[tokio::test]
async fn engine_without_judge_answers_equal() {
    let scenario = Scenario::new("sem-jev");
    let conversa = scenario.conversa();
    let engine = scenario.engine(JevGates::default(), None);

    let gatilho = scenario.say(conversa, "bom dia");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert!(summary.jev.is_none());
    assert_eq!(scenario.last_turn(conversa).jev_json, "{}");
}

#[tokio::test]
async fn g4_without_departments_not_question() {
    let estado = JevState::new("cli").with_messages([JevMessage::from_customer("oi")]);
    let jev = Arc::new(FakeJev::with_choices(BTreeMap::new()));
    let engine = engine_judge(
        Arc::clone(&jev),
        JevGates {
            g4: true,
            ..JevGates::default()
        },
    );

    let resultado = engine
        .evaluate(estado, Some(poph_jev::eval::gate_override(IdGate::G4)))
        .await;

    assert_eq!(jev.calls(), 0);
    assert!(resultado.decisions.is_empty());
    let _ = G4;
}

struct SearchSlow(Duration);

impl KnowledgeSearcher for SearchSlow {
    fn search(
        &self,
        _org: OrgId,
        _agent: AgentId,
        _query: String,
        _parameters: SearchParameters,
    ) -> SearchFuture<'_> {
        let wait = self.0;
        Box::pin(async move {
            tokio::time::sleep(wait).await;
            Ok(Vec::new())
        })
    }
}

#[tokio::test]
async fn judge_search_run_together() {
    let scenario = Scenario::new("paralelo");
    let conversa = scenario.conversa();
    let jev = Arc::new(
        FakeJev::with_choices(choices(&[("g1_transferir", G1_CONTINUE)]))
            .with_delay(Duration::from_millis(300)),
    );
    let engine = scenario
        .engine(so_g1(), Some(engine_judge(Arc::clone(&jev), so_g1())))
        .with_search(Arc::new(SearchSlow(Duration::from_millis(300))));

    let gatilho = scenario.say(conversa, "queria saber o prazo de entrega");
    let beginning = Instant::now();
    let summary = scenario.turno(&engine, conversa, gatilho).await;
    let elapsed = beginning.elapsed();

    assert_eq!(summary.estado, TurnState::Completed);
    assert!(summary.jev.is_some());
    assert!(
        elapsed < Duration::from_millis(520),
        "o turno levou {elapsed:?}: busca e Jev não correram juntos"
    );
}
