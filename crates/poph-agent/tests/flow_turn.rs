use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsG9, MetricsMemory, MetricsTurn};
use poph_agent::turno::{
    FuturePause, NoopObserver, Pause, TurnEngine, TurnInput, TurnOptions, TurnSummary,
};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, Script, Usage};
use poph_flows::{Limites, compile};
use poph_jev::{FakeJev, G9_NONE, IdGate, JevEngine, JevError};
use poph_store::{
    AgentChange, Direction, NewAgent, NewConversation, NewFlow, NewMessage, NewOrg,
    RevisionPublication, Role, Store,
};
use poph_types::{AgentId, ConversationId, FlowsConfig, JevGates, MessageId, OrgId};

struct NoWait;

impl Pause for NoWait {
    fn wait(&self, _duration: Duration) -> FuturePause {
        Box::pin(async {})
    }
}

const SCRIPT: &str = r#"fluxo(
    id = "agendar_visita",
    nome = "Agendar visita",
    inicio = "coleta",
    gatilho = gatilho(
        quando = "O cliente quer marcar uma visita técnica.",
        nao_quando = "O cliente só está perguntando o preço.",
        exemplos = ["quero marcar uma visita", "tem horário para o técnico?"],
    ),
)

etapa(
    id = "coleta",
    nome = "Coleta",
    instrucao = "Peça o nome e o endereço.",
    campos = [campo("nome", "Nome", obrigatorio = True)],
    acoes = [acao("seguir", "Seguir", exige = ["nome"], vai_para = fim("pronto", resultado = "Agendado"))],
)
"#;

struct FlowTest {
    id: poph_types::FlowId,
    slug: String,
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
            "poph-fx23-{label}-{}-{}",
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
                    grouping_window_ms: 0,
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
        Self {
            folder,
            store,
            org,
            agente,
        }
    }

    fn fluxo(&self, agente: AgentId) -> FlowTest {
        let fluxo = self
            .store
            .flows()
            .create(
                self.org,
                NewFlow {
                    slug: "agendar_visita".to_owned(),
                    name: "Agendar visita".to_owned(),
                    description: "Visita técnica".to_owned(),
                    rascunho_fonte: SCRIPT.to_owned(),
                },
            )
            .expect("cria o fluxo");
        self.publicar(fluxo.id);
        self.store
            .flows()
            .release(self.org, agente, fluxo.id)
            .expect("libera o fluxo");
        FlowTest {
            id: fluxo.id,
            slug: "agendar_visita".to_owned(),
        }
    }

    fn publicar(&self, fluxo: poph_types::FlowId) {
        let compiled = compile(SCRIPT, &Limites::default());
        let ir = compiled.ir().expect("o script do teste compila");
        let current = self.store.flows().get(self.org, fluxo).expect("lê o fluxo");
        self.store
            .revisions()
            .publish_revision(
                self.org,
                fluxo,
                RevisionPublication {
                    expected: current.version,
                    fonte: SCRIPT.to_owned(),
                    definicao_json: serde_json::to_string(ir).expect("IR em JSON"),
                    schema_versao: Some(ir.schema_versao),
                    resolucoes_json: "{}".to_owned(),
                    ator_tipo: "system".to_owned(),
                    actor_id: None,
                },
            )
            .expect("publica a revisão");
    }

    fn conversa(&self, agente: AgentId) -> ConversationId {
        self.store
            .conversations()
            .create(
                self.org,
                NewConversation {
                    agent_id: agente,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: Some("cliente-ficticio".to_owned()),
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

    fn adjust_judge_agent(&self, agente: AgentId, jev_json: &str) {
        let current = self
            .store
            .agents()
            .get(self.org, agente)
            .expect("lê o agente");
        self.store
            .agents()
            .update(
                self.org,
                agente,
                current.version,
                AgentChange {
                    jev_json: Some(jev_json.to_owned()),
                    ..AgentChange::default()
                },
            )
            .expect("ajusta o jev_json do agente");
    }

    fn engine(
        &self,
        agente: AgentId,
        jev: Option<Arc<JevEngine>>,
        fluxos: Option<FlowsConfig>,
        g9: MetricsG9,
    ) -> (TurnEngine, Arc<FakeProvider>) {
        let _ = agente;
        let config = ConfigHot::new(
            Arc::clone(&self.store),
            HotConfigOptions::default(),
            MetricsMemory::own(),
        );
        let limiter = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
        let provedor = Arc::new(FakeProvider::with_scripts(vec![
            script("Certo! Vou anotar."),
            script("Certo! Vou anotar."),
            script("Certo! Vou anotar."),
        ]));
        let mut engine = TurnEngine::new(
            Arc::clone(&self.store),
            config,
            limiter,
            provedor.clone(),
            TurnOptions::default(),
            MetricsTurn::own(),
        )
        .with_pause(Arc::new(NoWait))
        .with_summary(poph_agent::summary::SummaryOptions {
            ativo: false,
            ..poph_agent::summary::SummaryOptions::default()
        })
        .with_g9_metrics(g9);
        if let Some(adjustments) = fluxos {
            engine = engine.with_flows(&adjustments);
        }
        if let Some(jev) = jev {
            engine = engine.with_jev(jev);
        }
        (engine, provedor)
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

    fn ficha(&self, conversa: ConversationId) -> poph_store::Ficha {
        self.store
            .cases()
            .by_conversation(self.org, conversa)
            .expect("lê a Ficha")
            .expect("a conversa tem Ficha")
    }

    fn active(&self, conversa: ConversationId) -> Option<poph_store::Execution> {
        self.store
            .executions()
            .active_of_case(self.org, self.ficha(conversa).id)
            .expect("lê a execução ativa")
    }

    fn events(&self, conversa: ConversationId) -> Vec<poph_store::RecordEvent> {
        self.store
            .cases()
            .events(self.org, self.ficha(conversa).id, None, 50)
            .expect("lê os eventos")
    }

    fn last_turn(&self, conversa: ConversationId) -> poph_store::Turno {
        self.store
            .turns()
            .list(self.org, conversa, poph_store::Page::with_limit(5))
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
        LlmEvent::Texto(texto.to_owned()),
        LlmEvent::Usage(Usage {
            input: 10,
            saida: 5,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

fn so_g9() -> JevGates {
    JevGates {
        g9: true,
        ..JevGates::default()
    }
}

fn engine_judge(jev: Arc<FakeJev>) -> Arc<JevEngine> {
    Arc::new(JevEngine::new(jev, so_g9(), "jev-latest"))
}

fn systems(request: &poph_core::llm::Request) -> Vec<String> {
    request
        .messages
        .iter()
        .filter_map(|mensagem| match mensagem {
            poph_core::llm::Message::System(texto) => Some(texto.clone()),
            _ => None,
        })
        .collect()
}

fn request(request: &poph_core::llm::Request, passage: &str) -> bool {
    request.messages.iter().any(|mensagem| match mensagem {
        poph_core::llm::Message::System(texto) => texto.contains(passage),
        _ => false,
    })
}

#[tokio::test]
async fn beginning_happens_before_call_model() {
    let scenario = Scenario::new("antes");
    let fluxo = scenario.fluxo(scenario.agente);
    let conversa = scenario.conversa(scenario.agente);
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g9_iniciar_fluxo",
        &fluxo.slug,
    )])));
    let (engine, provedor) = scenario.engine(
        scenario.agente,
        Some(engine_judge(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        MetricsG9::own(),
    );

    let gatilho = scenario.say(conversa, "queria marcar uma visita técnica");
    let summary = scenario.turno(&engine, conversa, gatilho).await;
    assert_eq!(summary.estado, poph_store::TurnState::Completed);

    let active = scenario.active(conversa).expect("a execução foi aberta");
    assert_eq!(active.fluxo_id, fluxo.id);
    assert_eq!(active.no_atual, "coleta");

    let requests = provedor.received();
    assert_eq!(requests.len(), 1, "uma chamada ao modelo");
    assert!(
        request(&requests[0], "Fluxo em andamento: Agendar visita."),
        "o prompt do modelo já nasce com o bloco da etapa de entrada"
    );
    assert!(request(&requests[0], "Peça o nome e o endereço."));

    let events = scenario.events(conversa);
    let started = events
        .iter()
        .find(|evento| evento.tipo == poph_types::RecordEventKind::FlowStarted)
        .expect("há o evento fluxo_iniciado");
    let detail: serde_json::Value =
        serde_json::from_str(&started.payload_json).expect("payload é JSON");
    assert_eq!(detail["fluxo"], "agendar_visita");
    assert_eq!(detail["ator"], "jev");
    assert_eq!(detail["jev"]["escolha"], "agendar_visita");
}

#[tokio::test]
async fn prefix_stable_identical_with_without_flow() {
    let scenario = Scenario::new("prefixo");
    let fluxo = scenario.fluxo(scenario.agente);

    let with_flow = scenario.conversa(scenario.agente);
    let judge_with = Arc::new(FakeJev::with_choices(choices(&[(
        "g9_iniciar_fluxo",
        &fluxo.slug,
    )])));
    let (engine_with, provider_with) = scenario.engine(
        scenario.agente,
        Some(engine_judge(Arc::clone(&judge_with))),
        Some(FlowsConfig::default()),
        MetricsG9::own(),
    );
    let gatilho = scenario.say(with_flow, "quero marcar visita");
    scenario.turno(&engine_with, with_flow, gatilho).await;

    let without_flow = scenario.conversa(scenario.agente);
    let judge_without = Arc::new(FakeJev::with_choices(choices(&[(
        "g9_iniciar_fluxo",
        G9_NONE,
    )])));
    let (engine_without, provider_without) = scenario.engine(
        scenario.agente,
        Some(engine_judge(Arc::clone(&judge_without))),
        Some(FlowsConfig::default()),
        MetricsG9::own(),
    );
    let gatilho = scenario.say(without_flow, "só queria saber o horário");
    scenario.turno(&engine_without, without_flow, gatilho).await;

    let prefix_with = systems(&provider_with.received()[0])[0].clone();
    let prefix_without = systems(&provider_without.received()[0])[0].clone();
    assert_eq!(
        prefix_with, prefix_without,
        "o prefixo estável não pode mudar por causa do fluxo"
    );
    assert!(!prefix_with.contains("Fluxo em andamento"));
    assert!(
        request(
            &provider_with.received()[0],
            "Fluxo em andamento: Agendar visita."
        ),
        "o bloco do fluxo entra no bloco volátil, que é o segundo sistema"
    );
    assert_eq!(systems(&provider_with.received()[0]).len(), 2);
}

#[tokio::test]
async fn none_threshold_low_not_start_not_offer_catalog() {
    let scenario = Scenario::new("nenhum");
    let fluxo = scenario.fluxo(scenario.agente);

    let conversa = scenario.conversa(scenario.agente);
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g9_iniciar_fluxo",
        G9_NONE,
    )])));
    let g9 = MetricsG9::own();
    let (engine, provedor) = scenario.engine(
        scenario.agente,
        Some(engine_judge(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        g9.clone(),
    );
    let gatilho = scenario.say(conversa, "bom dia, tudo bem?");
    scenario.turno(&engine, conversa, gatilho).await;
    assert!(
        scenario.active(conversa).is_none(),
        "nada inicia com `nenhum`"
    );
    assert!(
        !request(&provedor.received()[0], "<fluxos_de_atendimento>"),
        "o catálogo não vai ao prompt com `nenhum`"
    );
    assert_eq!(g9.none.value(), 1);
    let turno = scenario.last_turn(conversa);
    let stored: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    assert_eq!(stored["fluxo"]["resultado"], "nenhum");

    let conversa = scenario.conversa(scenario.agente);
    let low = Arc::new(FakeJev::new(|_request| {
        let mut replies = std::collections::BTreeMap::new();
        replies.insert(
            "g9_iniciar_fluxo".to_owned(),
            poph_jev::JevResponse {
                choice: Some("agendar_visita".to_owned()),
                confidence: 0.3,
                ..poph_jev::JevResponse::default()
            },
        );
        Ok(poph_jev::BatchResponse {
            answers: replies,
            tokens: 1,
            ..poph_jev::BatchResponse::default()
        })
    }));
    let g9 = MetricsG9::own();
    let (engine, provedor) = scenario.engine(
        scenario.agente,
        Some(engine_judge(Arc::clone(&low))),
        Some(FlowsConfig::default()),
        g9.clone(),
    );
    let gatilho = scenario.say(conversa, "acho que queria algo de visita");
    scenario.turno(&engine, conversa, gatilho).await;
    assert!(
        scenario.active(conversa).is_none(),
        "confiança baixa não inicia nada, mesmo escolhendo o fluxo certo"
    );
    assert!(!request(&provedor.received()[0], "<fluxos_de_atendimento>"));
    assert_eq!(g9.below_threshold.value(), 1);
    let _ = fluxo;
}

#[tokio::test]
async fn judge_out_delivers_reservation() {
    let scenario = Scenario::new("reserva");
    let fluxo = scenario.fluxo(scenario.agente);
    let conversa = scenario.conversa(scenario.agente);
    let jev = Arc::new(FakeJev::failing(JevError::Network(
        "sem rede no teste".to_owned(),
    )));
    let g9 = MetricsG9::own();
    let (engine, provedor) = scenario.engine(
        scenario.agente,
        Some(engine_judge(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        g9.clone(),
    );

    let gatilho = scenario.say(conversa, "quero marcar uma visita");
    let summary = scenario.turno(&engine, conversa, gatilho).await;
    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert!(
        scenario.active(conversa).is_none(),
        "Jev fora do ar não inicia nada: quem inicia é o código"
    );
    let requests = provedor.received();
    assert!(
        request(&requests[0], "<fluxos_de_atendimento>"),
        "a reserva põe o catálogo no prompt"
    );
    assert!(
        request(&requests[0], &fluxo.slug),
        "o catálogo traz o id do fluxo"
    );
    assert!(request(
        &requests[0],
        "O cliente quer marcar uma visita técnica."
    ));
    assert!(!request(&requests[0], "chame fluxo_iniciar"));

    let turno = scenario.last_turn(conversa);
    let stored: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    assert_eq!(stored["fluxo"]["resultado"], "fallback");
    assert_eq!(g9.fallback.value(), 1);
}

#[tokio::test]
async fn flow_inactive_not_enters_options_nor_starts() {
    let scenario = Scenario::new("inativo");
    let fluxo = scenario.fluxo(scenario.agente);
    let current = scenario
        .store
        .flows()
        .get(scenario.org, fluxo.id)
        .expect("lê o fluxo");
    scenario
        .store
        .flows()
        .update(
            scenario.org,
            fluxo.id,
            current.version,
            poph_store::FlowChange {
                status: Some(poph_store::Status::Inactive),
                ..poph_store::FlowChange::default()
            },
        )
        .expect("desativa o fluxo");

    let conversa = scenario.conversa(scenario.agente);
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g9_iniciar_fluxo",
        &fluxo.slug,
    )])));
    let (engine, provedor) = scenario.engine(
        scenario.agente,
        Some(engine_judge(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        MetricsG9::own(),
    );
    let gatilho = scenario.say(conversa, "quero marcar visita");
    scenario.turno(&engine, conversa, gatilho).await;
    assert!(
        scenario.active(conversa).is_none(),
        "fluxo inativo não inicia execução nova (PRD §6.2)"
    );
    let requests = provedor.received();
    assert!(!request(&requests[0], "agendar_visita"));
    assert!(!request(&requests[0], "<fluxos_de_atendimento>"));
}

#[tokio::test]
async fn flow_not_released_not_enters_options_nor_starts() {
    let scenario = Scenario::new("nao-liberado");
    let other = scenario
        .store
        .agents()
        .create(
            scenario.org,
            NewAgent {
                name: "Outro".to_owned(),
                slug: "outro".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o outro agente")
        .id;
    let fluxo = scenario.fluxo(other);

    let conversa = scenario.conversa(scenario.agente);
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g9_iniciar_fluxo",
        &fluxo.slug,
    )])));
    let (engine, provedor) = scenario.engine(
        scenario.agente,
        Some(engine_judge(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        MetricsG9::own(),
    );
    let gatilho = scenario.say(conversa, "quero marcar visita");
    scenario.turno(&engine, conversa, gatilho).await;
    assert!(
        scenario.active(conversa).is_none(),
        "sem a permissão, o fluxo não é oferecido nem iniciado"
    );
    let requests = provedor.received();
    assert!(!request(&requests[0], "agendar_visita"));
}

#[tokio::test]
async fn flow_other_org_never_starts() {
    let scenario = Scenario::new("outra-org");
    let other_org = scenario
        .store
        .orgs()
        .create(NewOrg {
            name: "Org vizinha".to_owned(),
            slug: format!("vizinha-{}", AgentId::generate()),
            config_json: "{}".to_owned(),
        })
        .expect("cria a org vizinha")
        .id;
    let agent_neighbor = scenario
        .store
        .agents()
        .create(
            other_org,
            NewAgent {
                name: "Vizinho".to_owned(),
                slug: "vizinho".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente vizinho")
        .id;
    let flow_neighbor = scenario
        .store
        .flows()
        .create(
            other_org,
            NewFlow {
                slug: "agendar_visita".to_owned(),
                name: "Agendar visita (vizinha)".to_owned(),
                description: String::new(),
                rascunho_fonte: SCRIPT.to_owned(),
            },
        )
        .expect("cria o fluxo da vizinha");
    let current = scenario
        .store
        .flows()
        .get(other_org, flow_neighbor.id)
        .expect("lê o fluxo da vizinha");
    let compiled = compile(SCRIPT, &Limites::default());
    let ir = compiled.ir().expect("compila");
    scenario
        .store
        .revisions()
        .publish_revision(
            other_org,
            flow_neighbor.id,
            RevisionPublication {
                expected: current.version,
                fonte: SCRIPT.to_owned(),
                definicao_json: serde_json::to_string(ir).expect("IR em JSON"),
                schema_versao: Some(ir.schema_versao),
                resolucoes_json: "{}".to_owned(),
                ator_tipo: "system".to_owned(),
                actor_id: None,
            },
        )
        .expect("publica na vizinha");
    scenario
        .store
        .flows()
        .release(other_org, agent_neighbor, flow_neighbor.id)
        .expect("libera para o vizinho");

    let conversa = scenario.conversa(scenario.agente);
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g9_iniciar_fluxo",
        "agendar_visita",
    )])));
    let (engine, provedor) = scenario.engine(
        scenario.agente,
        Some(engine_judge(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        MetricsG9::own(),
    );
    let gatilho = scenario.say(conversa, "quero marcar visita");
    scenario.turno(&engine, conversa, gatilho).await;
    assert!(
        scenario.active(conversa).is_none(),
        "fluxo de outra org não existe para esta organização"
    );
    let requests = provedor.received();
    assert!(!request(&requests[0], "agendar_visita"));
    assert!(
        scenario
            .store
            .executions()
            .from_case(scenario.org, scenario.ficha(conversa).id, 10)
            .expect("lê as execuções")
            .is_empty()
    );
}

#[tokio::test]
async fn execution_active_not_question_judge() {
    let scenario = Scenario::new("ativa");
    let fluxo = scenario.fluxo(scenario.agente);
    let conversa = scenario.conversa(scenario.agente);

    let ficha = scenario.ficha(conversa);
    let revision = scenario
        .store
        .revisions()
        .current(scenario.org, fluxo.id)
        .expect("lê a revisão")
        .expect("há revisão publicada");
    let compiled = compile(SCRIPT, &Limites::default());
    let ir = compiled.ir().expect("compila");
    let estado = poph_flows::engine::start(ir).expect("estado inicial");
    let evaluator = poph_flows::RuleEvaluator::without_functions();
    let context = poph_flows::engine::Context::new(&evaluator);
    let effects =
        poph_flows::engine::entry_effects(ir, estado.no_atual.id(), &estado.dados, &context)
            .expect("efeitos de entrada")
            .iter()
            .map(poph_agent::flows_transition::effect_of_transition)
            .collect();
    scenario
        .store
        .executions()
        .start_execution(
            scenario.org,
            poph_store::ExecutionStart {
                case: ficha.id,
                flow: fluxo.id,
                revision: revision.id,
                actor: poph_types::EventActor::Attendant,
                ator_ref: Some("token-de-teste"),
                jev_decision: None,
                initial_state: poph_agent::flows_transition::state_for_database(&estado),
                inbound_effects: effects,
                chave_idempotencia: Some("abertura-do-atendente"),
            },
        )
        .expect("o atendente abre a execução");

    let jev = Arc::new(FakeJev::with_choices(std::collections::BTreeMap::new()));
    let (engine, provedor) = scenario.engine(
        scenario.agente,
        Some(engine_judge(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        MetricsG9::own(),
    );
    let gatilho = scenario.say(conversa, "meu nome é Ana Teste");
    scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(jev.calls(), 0, "com execução ativa não há pergunta ao Jev");
    let requests = provedor.received();
    assert!(request(&requests[0], "Fluxo em andamento: Agendar visita."));
    let turno = scenario.last_turn(conversa);
    let stored: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    assert_eq!(stored["fluxo"]["resultado"], "execucao_ativa");
}

#[tokio::test]
async fn flows_disabled_ignore_all_case_follows() {
    let scenario = Scenario::new("desligado");
    let fluxo = scenario.fluxo(scenario.agente);
    let conversa = scenario.conversa(scenario.agente);
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g9_iniciar_fluxo",
        &fluxo.slug,
    )])));
    let (engine, provedor) = scenario.engine(
        scenario.agente,
        Some(engine_judge(Arc::clone(&jev))),
        Some(FlowsConfig {
            enabled: false,
            ..FlowsConfig::default()
        }),
        MetricsG9::own(),
    );
    let gatilho = scenario.say(conversa, "quero marcar visita");
    scenario.turno(&engine, conversa, gatilho).await;

    assert!(
        scenario.active(conversa).is_none(),
        "motor desligado não inicia"
    );
    assert_eq!(jev.calls(), 0, "motor desligado não pergunta ao Jev");
    assert!(!request(&provedor.received()[0], "<fluxos_de_atendimento>"));
    assert!(
        scenario
            .store
            .cases()
            .by_conversation(scenario.org, conversa)
            .expect("lê")
            .is_some(),
        "a Ficha continua sendo criada com o motor desligado"
    );
}

#[tokio::test]
async fn panel_by_shows_decision() {
    let scenario = Scenario::new("painel");
    let fluxo = scenario.fluxo(scenario.agente);
    let conversa = scenario.conversa(scenario.agente);
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g9_iniciar_fluxo",
        &fluxo.slug,
    )])));
    let g9 = MetricsG9::own();
    let (engine, _provider) = scenario.engine(
        scenario.agente,
        Some(engine_judge(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        g9.clone(),
    );
    let gatilho = scenario.say(conversa, "quero marcar uma visita");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    let registered = summary.jev.expect("o turno devolve o Jev");
    assert_eq!(registered.choice(IdGate::G9), Some(fluxo.slug.as_str()));
    let turno = scenario.last_turn(conversa);
    let stored: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    assert_eq!(stored["decisoes"][0]["gate"], "g9");
    assert_eq!(stored["decisoes"][0]["escolha"], "agendar_visita");
    assert_eq!(stored["fluxo"]["resultado"], "iniciou");
    assert_eq!(stored["fluxo"]["fluxo"], "agendar_visita");
    assert_eq!(stored["fluxo"]["etapa"], "coleta");
    assert!(stored["fluxo"]["execucao"].as_str().is_some());
    assert_eq!(g9.start.value(), 1);
}

#[tokio::test]
async fn gate_disabled_agent_delivers_reservation_without_ask() {
    let scenario = Scenario::new("gate-off");
    let fluxo = scenario.fluxo(scenario.agente);
    scenario.adjust_judge_agent(scenario.agente, r#"{"g9": false}"#);
    let conversa = scenario.conversa(scenario.agente);
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g9_iniciar_fluxo",
        &fluxo.slug,
    )])));
    let (engine, provedor) = scenario.engine(
        scenario.agente,
        Some(engine_judge(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        MetricsG9::own(),
    );
    let gatilho = scenario.say(conversa, "quero marcar uma visita");
    scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(jev.calls(), 0, "gate desligado não é perguntado");
    assert!(scenario.active(conversa).is_none(), "e nada inicia sozinho");
    let requests = provedor.received();
    assert!(
        request(&requests[0], "<fluxos_de_atendimento>"),
        "a reserva entrega o catálogo ao modelo"
    );
    let turno = scenario.last_turn(conversa);
    let stored: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    assert_eq!(stored["fluxo"]["resultado"], "reserva");
}

#[tokio::test]
async fn state_g9_takes_flows_released() {
    let scenario = Scenario::new("estado");
    let fluxo = scenario.fluxo(scenario.agente);
    let conversa = scenario.conversa(scenario.agente);
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g9_iniciar_fluxo",
        &fluxo.slug,
    )])));
    let (engine, _provider) = scenario.engine(
        scenario.agente,
        Some(engine_judge(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        MetricsG9::own(),
    );
    let gatilho = scenario.say(conversa, "quero marcar uma visita");
    scenario.turno(&engine, conversa, gatilho).await;

    let request = &jev.requests()[0];
    assert!(request.questions.contains_key("g9_iniciar_fluxo"));
    let released = &request.estado["fluxos"]["liberados"];
    assert_eq!(released[0]["id"], "agendar_visita");
    assert_eq!(released[0]["nome"], "Agendar visita");
    assert_eq!(
        released[0]["quando"],
        "O cliente quer marcar uma visita técnica."
    );
    assert_eq!(released[0]["exemplos"][0], "quero marcar uma visita");
}

fn choices(items: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
    items
        .iter()
        .map(|(id, choice)| ((*id).to_owned(), (*choice).to_owned()))
        .collect()
}
