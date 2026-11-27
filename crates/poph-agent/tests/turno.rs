use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poph_agent::actors::{ActorOptions, Actors};
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::entry_output::{FlowOptions, Fluxo, OutputTransport, ReceivedMessage, SendFuture};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsFlow, MetricsMemory, MetricsTurn};
use poph_agent::turno::{
    FuturePause, NoopObserver, Pause, ToolRegistry, TurnEngine, TurnEvent, TurnInput, TurnObserver,
    TurnOptions, TurnProcessor,
};
use poph_core::llm::{
    Cancellation, FakeProvider, FinishReason, LlmError, LlmEvent, LlmProvider, Script, ToolCall,
    Usage,
};
use poph_store::{
    Direction, NewAgent, NewConversation, NewMessage, NewOrg, OutboxItem, OutboxState, Page, Role,
    Store, TurnState,
};
use poph_types::{AgentId, ConversationId, IntegrationId, MessageId, OrgId, ToolConfig};

type OnWait = Arc<dyn Fn() + Send + Sync>;

#[derive(Default, Clone)]
struct FakePause {
    requests: Arc<Mutex<Vec<Duration>>>,
    on_wait: Option<OnWait>,
}

impl FakePause {
    fn with_action(acao: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            requests: Arc::default(),
            on_wait: Some(Arc::new(acao)),
        }
    }

    fn requests(&self) -> Vec<Duration> {
        self.requests
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}

impl Pause for FakePause {
    fn wait(&self, duration: Duration) -> FuturePause {
        self.requests
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(duration);
        if let Some(acao) = &self.on_wait
            && !duration.is_zero()
        {
            acao();
        }
        Box::pin(async {})
    }
}

#[derive(Default)]
struct Collector {
    events: Vec<TurnEvent>,
}

impl Collector {
    fn texto(&self) -> String {
        let mut texto = String::new();
        for evento in &self.events {
            if let TurnEvent::Texto(piece) = evento {
                texto.push_str(piece);
            }
        }
        texto
    }
}

impl TurnObserver for Collector {
    fn evento(&mut self, evento: TurnEvent) {
        self.events.push(evento);
    }
}

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    conversa: ConversationId,
    agente: AgentId,
    clock: Arc<AtomicI64>,
    integracao: IntegrationId,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-hr13-{label}-{}-{}",
            std::process::id(),
            AgentId::generate()
        ));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("cria a pasta do teste");
        let clock = Arc::new(AtomicI64::new(1_795_130_160_000));
        let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
            .expect("abre o banco")
            .with_clock({
                let clock = Arc::clone(&clock);
                Arc::new(move || clock.load(Ordering::SeqCst))
            });
        let store = Arc::new(store);
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
                    modelo: "gemini-3.8-flash-tiered".to_owned(),
                    grouping_window_ms: 2_000,
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente");
        store
            .instructions()
            .save(
                org,
                agente.id,
                0,
                "{}",
                "# Bia\n\nVocê é a atendente da FixProLar.",
                true,
            )
            .expect("grava as instruções");
        let conversa = Self::open_conversation(&store, org, agente.id, "cli");
        Self {
            folder,
            store,
            org,
            conversa,
            agente: agente.id,
            clock,
            integracao: IntegrationId::generate(),
        }
    }

    fn open_conversation(
        store: &Store,
        org: OrgId,
        agente: AgentId,
        canal: &str,
    ) -> ConversationId {
        store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente,
                    integration_id: None,
                    canal: canal.to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("abre a conversa")
            .id
    }

    fn advance(&self, ms: i64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }

    fn say(&self, texto: &str) -> MessageId {
        self.store
            .messages()
            .write(
                self.org,
                NewMessage {
                    conversation_id: self.conversa,
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
            .expect("grava a mensagem do cliente")
            .id
    }

    fn pause(&self) -> Arc<FakePause> {
        Arc::new(FakePause::default())
    }

    fn engine(
        &self,
        scripts: Vec<Script>,
        pause: Arc<FakePause>,
    ) -> (TurnEngine, Arc<FakeProvider>) {
        self.engine_with(scripts, pause, TurnOptions::default())
    }

    fn engine_with(
        &self,
        scripts: Vec<Script>,
        pause: Arc<FakePause>,
        options: TurnOptions,
    ) -> (TurnEngine, Arc<FakeProvider>) {
        let fake = Arc::new(FakeProvider::with_scripts(scripts));
        let provedor: Arc<dyn LlmProvider> = fake.clone();
        let config = ConfigHot::new(
            Arc::clone(&self.store),
            HotConfigOptions::default(),
            MetricsMemory::own(),
        );
        let limiter = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
        let engine = TurnEngine::new(
            Arc::clone(&self.store),
            config,
            limiter,
            provedor,
            options,
            MetricsTurn::own(),
        )
        .with_pause(pause);
        (engine, fake)
    }

    fn fluxo(&self, engine: Arc<TurnEngine>) -> Fluxo {
        self.flow_with(engine, FlowOptions::default())
    }

    fn flow_with(&self, engine: Arc<TurnEngine>, options: FlowOptions) -> Fluxo {
        let processor = Arc::new(TurnProcessor::new(engine));
        let actors = Actors::new(
            processor.clone(),
            ActorOptions::default(),
            MetricsMemory::own(),
        );
        let fluxo =
            Fluxo::with_metrics(Arc::clone(&self.store), actors, options, MetricsFlow::own());
        processor.attach_io(fluxo.clone());
        fluxo
    }

    fn messages(&self) -> Vec<poph_store::Mensagem> {
        self.store
            .messages()
            .list(self.org, self.conversa, Page::with_limit(200))
            .expect("lista as mensagens")
    }

    fn turns(&self) -> Vec<poph_store::Turno> {
        self.store
            .turns()
            .list(self.org, self.conversa, Page::with_limit(200))
            .expect("lista os turnos")
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

fn response(texto: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(texto.to_owned()),
        LlmEvent::Usage(Usage {
            input: 120,
            saida: 30,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

#[tokio::test]
async fn five_messages_record_stream_messages_turn() {
    let scenario = Scenario::new("cinco");
    let scripts: Vec<Script> = (1..=5)
        .map(|n| response(&format!("resposta {n}")))
        .collect();
    let pause = scenario.pause();
    let (engine, fake) = scenario.engine(scripts, Arc::clone(&pause));

    let mut texts = Vec::new();
    for n in 1..=5 {
        let gatilho = scenario.say(&format!("pergunta {n}"));
        let mut collector = Collector::default();
        let summary = engine
            .run(
                TurnInput::new(scenario.org, scenario.conversa, gatilho),
                &mut collector,
            )
            .await
            .expect("turno roda");
        assert_eq!(summary.estado, TurnState::Completed, "{:?}", summary.error);
        assert_eq!(summary.texto, format!("resposta {n}"));
        assert_eq!(
            collector.texto(),
            summary.texto,
            "o stream é a resposta final"
        );
        assert!(
            collector
                .events
                .iter()
                .any(|evento| matches!(evento, TurnEvent::Started { .. })),
            "o turno avisa o início"
        );
        texts.push(summary);
    }

    assert_eq!(fake.received().len(), 5, "uma chamada ao modelo por turno");

    let messages = scenario.messages();
    assert_eq!(messages.len(), 10);
    for (index, mensagem) in messages.iter().enumerate() {
        if index % 2 == 0 {
            assert_eq!(mensagem.role, Role::User);
            assert_eq!(mensagem.direction, Direction::Inbound);
        } else {
            assert_eq!(mensagem.role, Role::Assistant);
            assert_eq!(mensagem.direction, Direction::Outbound);
        }
    }

    let turns = scenario.turns();
    assert_eq!(turns.len(), 5);
    for turno in &turns {
        assert_eq!(turno.state, TurnState::Completed);
        assert_eq!(turno.input_tokens, 120);
        assert_eq!(turno.output_tokens, 30);
        let times: serde_json::Value =
            serde_json::from_str(&turno.times_json).expect("tempos em JSON");
        for chave in [
            "fila_ms",
            "montagem_ms",
            "primeiro_texto_ms",
            "total_ms",
            "rodadas_ferramenta",
        ] {
            assert!(times.get(chave).is_some(), "falta `{chave}`: {times}");
        }
        assert_eq!(times["modelo"], "gemini-3.8-flash-tiered");
        assert_eq!(times["motivo_fim"], "concluido");
    }

    let saida = scenario
        .store
        .outbox()
        .list_pending(scenario.org, 10)
        .expect("lista a saída");
    assert_eq!(saida.len(), 5);
    assert_eq!(
        saida[0].content_json,
        serde_json::json!({"texto": "resposta 1"}).to_string()
    );
}

#[tokio::test]
async fn prefix_identical_three_turns() {
    let scenario = Scenario::new("prefixo");
    let pause = scenario.pause();
    let (engine, fake) = scenario.engine(
        vec![response("um"), response("dois"), response("três")],
        pause,
    );

    for n in 1..=3 {
        let gatilho = scenario.say(&format!("mensagem {n}"));
        scenario.advance(60_000);
        engine
            .run(
                TurnInput::new(scenario.org, scenario.conversa, gatilho),
                &mut NoopObserver,
            )
            .await
            .expect("turno roda");
    }

    let requests = fake.received();
    assert_eq!(requests.len(), 3);
    let prefix = |request: &poph_core::llm::Request| match &request.messages[0] {
        poph_core::llm::Message::System(texto) => texto.clone(),
        other => panic!("a primeira mensagem devia ser o prefixo: {other:?}"),
    };
    let first = prefix(&requests[0]);
    assert!(first.contains("FixProLar"), "{first}");
    for request in &requests[1..] {
        assert_eq!(prefix(request), first, "prefixo mudou entre turnos");
    }

    let volatile = |request: &poph_core::llm::Request| match &request.messages[1] {
        poph_core::llm::Message::System(texto) => texto.clone(),
        other => panic!("a segunda mensagem devia ser o bloco volátil: {other:?}"),
    };
    assert!(volatile(&requests[0]).contains("hora:"));
    assert_ne!(volatile(&requests[0]), volatile(&requests[1]));
}

#[tokio::test]
async fn two_orgs_not_mix() {
    let alpha = Scenario::new("org-alfa");
    let beta = Scenario::new("org-beta");
    let (engine_alpha, fake_alpha) = alpha.engine(vec![response("sou a alfa")], alpha.pause());
    let (engine_beta, fake_beta) = beta.engine(vec![response("sou a beta")], beta.pause());

    let trigger_alpha = alpha.say("quem é você?");
    let trigger_beta = beta.say("e você?");
    let (mut obs_alpha, mut obs_beta) = (NoopObserver, NoopObserver);
    let (summary_alpha, summary_beta) = tokio::join!(
        engine_alpha.run(
            TurnInput::new(alpha.org, alpha.conversa, trigger_alpha),
            &mut obs_alpha
        ),
        engine_beta.run(
            TurnInput::new(beta.org, beta.conversa, trigger_beta),
            &mut obs_beta
        ),
    );
    let summary_alpha = summary_alpha.expect("turno da alfa");
    let summary_beta = summary_beta.expect("turno da beta");

    assert_eq!(summary_alpha.texto, "sou a alfa");
    assert_eq!(summary_beta.texto, "sou a beta");

    for request in fake_alpha.received() {
        assert_eq!(request.org, alpha.org);
    }
    for request in fake_beta.received() {
        assert_eq!(request.org, beta.org);
    }

    assert_eq!(alpha.turns().len(), 1);
    assert_eq!(beta.turns().len(), 1);
    let messages_beta = beta.messages();
    assert_eq!(messages_beta.len(), 2);
    assert_eq!(messages_beta[1].texto, "sou a beta");
    assert_eq!(messages_beta[1].conversation_id, beta.conversa);
}

#[tokio::test]
async fn window_grouping_joins_two_separates_after_cap() {
    let scenario = Scenario::new("janela");
    let second_one: Arc<Mutex<Option<String>>> = Arc::default();
    let second_text = "queria um orçamento";
    let escrever = {
        let second_one = Arc::clone(&second_one);
        let scenario_folder = scenario.folder.clone();
        let store = Arc::clone(&scenario.store);
        let org = scenario.org;
        let conversa = scenario.conversa;
        move || {
            let mut guard = second_one.lock().unwrap_or_else(|v| v.into_inner());
            if guard.is_none() {
                let mensagem = store
                    .messages()
                    .write(
                        org,
                        NewMessage {
                            conversation_id: conversa,
                            direction: Direction::Inbound,
                            role: Role::User,
                            texto: second_text.to_owned(),
                            attachments_json: "[]".to_owned(),
                            external_id: None,
                            turn_id: None,
                            origin: poph_store::Origin::Agente,
                            author_name: None,
                        },
                    )
                    .expect("grava a mensagem que chega na janela");
                *guard = Some(mensagem.id.to_string());
                let _ = scenario_folder;
            }
        }
    };
    let pause = Arc::new(FakePause::with_action(escrever));
    let (engine, fake) = scenario.engine(
        vec![response("juntas"), response("sozinha")],
        Arc::clone(&pause),
    );

    let first = scenario.say("oi");
    let summary = engine
        .run(
            TurnInput::new(scenario.org, scenario.conversa, first),
            &mut NoopObserver,
        )
        .await
        .expect("turno roda");
    assert_eq!(summary.estado, TurnState::Completed);
    let second_id: MessageId = second_one
        .lock()
        .unwrap_or_else(|v| v.into_inner())
        .as_ref()
        .expect("a segunda mensagem chegou")
        .parse()
        .expect("id da segunda mensagem");
    assert_eq!(
        summary.messages,
        vec![first, second_id],
        "as duas mensagens da janela entraram no mesmo turno"
    );
    assert_eq!(fake.received().len(), 1, "um turno, uma chamada ao modelo");

    let requests = pause.requests();
    assert_eq!(requests, vec![Duration::from_millis(2_000)]);

    scenario.advance(9_000);
    let third = scenario.say("e o prazo?");
    let summary = engine
        .run(
            TurnInput::new(scenario.org, scenario.conversa, third),
            &mut NoopObserver,
        )
        .await
        .expect("segundo turno roda");
    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.messages, vec![third], "turno novo, uma mensagem");
    assert_eq!(fake.received().len(), 2);

    let messages = scenario.messages();
    assert_eq!(messages.len(), 5);
    assert_eq!(scenario.turns().len(), 2);
}

struct ToolTest {
    nome: &'static str,
}

impl poph_agent::turno::InternalTool for ToolTest {
    fn nome(&self) -> &'static str {
        self.nome
    }

    fn descricao(&self) -> &'static str {
        "ferramenta de teste"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }

    fn run(
        &self,
        _context: poph_agent::turno::ToolContext,
        _arguments: serde_json::Value,
    ) -> poph_agent::turno::ToolFuture {
        Box::pin(async { Ok("resultado da ferramenta".to_owned()) })
    }
}

struct TwoRecords;

impl ToolRegistry for TwoRecords {
    fn tool(&self, nome: &str) -> Option<Arc<dyn poph_agent::turno::InternalTool>> {
        match nome {
            "consultar_base" => Some(Arc::new(ToolTest {
                nome: "consultar_base",
            })),
            "buscar_produto" => Some(Arc::new(ToolTest {
                nome: "buscar_produto",
            })),
            _ => None,
        }
    }
}

#[tokio::test]
async fn tool_not_enabled_not_offered_loop_for_limit() {
    let scenario = Scenario::new("ferramentas");
    scenario
        .store
        .agents()
        .update(
            scenario.org,
            scenario.agente,
            1,
            poph_store::AgentChange {
                max_tool_rounds: Some(1),
                max_tools_per_message: Some(1),
                ..poph_store::AgentChange::default()
            },
        )
        .expect("ajusta o agente");
    let instructions = scenario
        .store
        .instructions()
        .get(scenario.org, scenario.agente)
        .expect("lê as instruções");
    let structured = poph_types::StructuredInstructions {
        tools: vec![
            ToolConfig {
                name: "consultar_base".to_owned(),
                enabled: true,
                when_to_use: String::new(),
            },
            ToolConfig {
                name: "buscar_produto".to_owned(),
                enabled: false,
                when_to_use: String::new(),
            },
        ],
        ..poph_types::StructuredInstructions::default()
    };
    scenario
        .store
        .instructions()
        .save(
            scenario.org,
            scenario.agente,
            instructions.version,
            &structured.to_json(),
            &instructions.markdown,
            false,
        )
        .expect("grava as ferramentas");

    let two_calls = Script::Events(vec![
        LlmEvent::Tool(ToolCall {
            id: "c1".to_owned(),
            nome: "buscar_produto".to_owned(),
            arguments: serde_json::json!({}),
        }),
        LlmEvent::Tool(ToolCall {
            id: "c2".to_owned(),
            nome: "consultar_base".to_owned(),
            arguments: serde_json::json!({}),
        }),
        LlmEvent::Fim(FinishReason::ToolCalls),
    ]);
    let (engine, fake) = scenario.engine(vec![two_calls, response("pronto")], scenario.pause());
    let engine = engine.with_tools(Arc::new(TwoRecords));

    let gatilho = scenario.say("quanto custa a dedetização?");
    let summary = engine
        .run(
            TurnInput::new(scenario.org, scenario.conversa, gatilho),
            &mut NoopObserver,
        )
        .await
        .expect("turno roda");
    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, "pronto");

    let requests = fake.received();
    assert_eq!(
        requests.len(),
        2,
        "uma rodada de ferramenta (o limite) e o fecho em texto"
    );

    let names: Vec<String> = requests[0]
        .tools
        .iter()
        .map(|tool| tool.nome.clone())
        .collect();
    assert_eq!(names, vec!["consultar_base".to_owned()]);

    let results: Vec<String> = requests[1]
        .messages
        .iter()
        .filter_map(|mensagem| match mensagem {
            poph_core::llm::Message::Resultado { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect();
    assert!(results[0].contains("não está habilitada"), "{:?}", results);

    assert!(results[1].contains("limite de 1 chamadas"), "{:?}", results);

    assert!(requests[1].tools.is_empty());
}

#[tokio::test]
async fn cancellation_ends_turn_concludes_entry() {
    let scenario = Scenario::new("cancelamento");
    let (engine, _fake) = scenario.engine(
        vec![Script::Pending {
            events: vec![LlmEvent::Texto("parcial ".to_owned())],
            after: Duration::from_secs(30),
            error: LlmError::TimedOut { seconds: 30 },
        }],
        scenario.pause(),
    );
    let shared = Cancellation::new();
    let fluxo = scenario.fluxo(Arc::new(engine.with_cancellation(shared.clone())));

    fluxo
        .receive_message(
            scenario.org,
            ReceivedMessage {
                integration_id: scenario.integracao,
                id_externo: "ext-cancel-1".to_owned(),
                conversa: scenario.conversa,
                texto: "me conta tudo, com detalhes".to_owned(),
                attachments_json: "[]".to_owned(),
            },
        )
        .expect("grava a entrada");
    fluxo.dispatch(scenario.org).expect("despacha");

    tokio::time::sleep(Duration::from_millis(150)).await;
    shared.cancel();
    wait_queue_empty(&fluxo, &scenario).await;

    let turns = scenario.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].state, TurnState::Failed);
    assert!(
        turns[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("cancelada"),
        "{:?}",
        turns[0].error
    );
    assert!(turns[0].finished_at.is_some(), "o turno foi fechado");

    let pending = scenario
        .store
        .inbox()
        .count_pending(scenario.org)
        .expect("conta a entrada");
    assert_eq!(pending, 0, "a entrada foi concluída mesmo no cancelamento");

    let saida = scenario
        .store
        .outbox()
        .list_pending(scenario.org, 10)
        .expect("lista a saída");
    assert!(saida.is_empty(), "nada de resposta parcial na outbox");
    let messages = scenario.messages();
    assert_eq!(messages.len(), 1, "só a mensagem do cliente");
    assert_eq!(messages[0].role, Role::User);
}

#[tokio::test]
async fn error_provider_records_turn_not_leaves_item_stuck() {
    let scenario = Scenario::new("erro-provedor");
    let (engine, _fake) = scenario.engine(
        vec![Script::Failure(LlmError::Unavailable {
            tentativas: 3,
            last_error: "503".to_owned(),
        })],
        scenario.pause(),
    );
    let fluxo = scenario.fluxo(Arc::new(engine));

    let incoming = fluxo
        .receive_message(
            scenario.org,
            ReceivedMessage {
                integration_id: IntegrationId::generate(),
                id_externo: "ext-erro-1".to_owned(),
                conversa: scenario.conversa,
                texto: "oi".to_owned(),
                attachments_json: "[]".to_owned(),
            },
        )
        .expect("grava a entrada");
    assert_eq!(incoming.situation, poph_store::Received::New);
    fluxo.dispatch(scenario.org).expect("despacha");
    wait_attempts(&scenario, incoming.item.id, 1).await;

    let transport = TransportTest::default();
    fluxo
        .send_pending(scenario.org, &transport)
        .await
        .expect("envia");

    let turns = scenario.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].state, TurnState::Failed);
    assert!(
        turns[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("503"),
        "{:?}",
        turns[0].error
    );
    let pending = scenario
        .store
        .inbox()
        .count_pending(scenario.org)
        .expect("conta a entrada");
    assert_eq!(pending, 1, "a entrada fica para nova tentativa");
    let tentativas = scenario
        .store
        .inbox()
        .attempts(scenario.org, &[incoming.item.id])
        .expect("tentativas");
    assert_eq!(tentativas, vec![1]);
    let sending = scenario
        .store
        .outbox()
        .count_by_state(scenario.org, OutboxState::Sending)
        .expect("conta a saída");
    assert_eq!(sending, 0);
    assert!(transport.sent().is_empty(), "não há resposta para enviar");
    let messages = scenario.messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, Role::User);
}

#[tokio::test]
async fn without_slot_leaves_entry_pending_for_new_attempt() {
    let scenario = Scenario::new("sem-vaga");
    let (engine, fake) =
        scenario.engine(vec![response("não deveria ser chamada")], scenario.pause());
    let _ = &engine;
    let provedor: Arc<dyn LlmProvider> = fake.clone();
    let limiter = Arc::new(Limiter::new(
        LimitOptions {
            turns_global: 1,
            turns_by_org: 1,
            turns_by_agent: 0,
            calls_by_provider: 8,
            cap_by_provider: Default::default(),
            taxa_por_org_por_minuto: 0,
        },
        MetricsMemory::own(),
    ));
    let slot = limiter
        .acquire_turn(scenario.org, scenario.agente, Duration::from_millis(10))
        .await
        .expect("pega a única vaga");
    let config = ConfigHot::new(
        Arc::clone(&scenario.store),
        HotConfigOptions::default(),
        MetricsMemory::own(),
    );
    let engine = TurnEngine::new(
        Arc::clone(&scenario.store),
        config,
        Arc::clone(&limiter),
        provedor,
        TurnOptions {
            slot_wait: Duration::from_millis(20),
            ..TurnOptions::default()
        },
        MetricsTurn::own(),
    )
    .with_pause(scenario.pause());
    let fluxo = scenario.fluxo(Arc::new(engine));

    fluxo
        .receive_message(
            scenario.org,
            ReceivedMessage {
                integration_id: scenario.integracao,
                id_externo: "ext-vaga-1".to_owned(),
                conversa: scenario.conversa,
                texto: "oi".to_owned(),
                attachments_json: "[]".to_owned(),
            },
        )
        .expect("grava a entrada");
    fluxo.dispatch(scenario.org).expect("despacha");
    wait_turn_closed(&scenario).await;

    let turns = scenario.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].state, TurnState::Failed);
    assert!(
        turns[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("sem vaga"),
        "{:?}",
        turns[0].error
    );
    let pending = scenario
        .store
        .inbox()
        .count_pending(scenario.org)
        .expect("conta a entrada");
    assert_eq!(
        pending, 1,
        "a entrada fica adiada e pendente quando a falta de vaga é transitória"
    );
    drop(slot);
}

fn limiter_slot() -> Arc<Limiter> {
    Arc::new(Limiter::new(
        LimitOptions {
            turns_global: 1,
            turns_by_org: 1,
            turns_by_agent: 0,
            calls_by_provider: 1,
            cap_by_provider: Default::default(),
            taxa_por_org_por_minuto: 0,
        },
        MetricsMemory::own(),
    ))
}

fn engine_with_limiter(
    scenario: &Scenario,
    scripts: Vec<Script>,
    limiter: Arc<Limiter>,
) -> (TurnEngine, Arc<FakeProvider>) {
    let fake = Arc::new(FakeProvider::with_scripts(scripts));
    let provedor: Arc<dyn LlmProvider> = fake.clone();
    let config = ConfigHot::new(
        Arc::clone(&scenario.store),
        HotConfigOptions::default(),
        MetricsMemory::own(),
    );
    let engine = TurnEngine::new(
        Arc::clone(&scenario.store),
        config,
        limiter,
        provedor,
        TurnOptions {
            slot_wait: Duration::from_millis(20),
            ..TurnOptions::default()
        },
        MetricsTurn::own(),
    )
    .with_pause(scenario.pause());
    (engine, fake)
}

const PROVIDER_AGENT: &str = "antigravity2api";

#[tokio::test]
async fn missing_slot_turn_enters_ladder_exhausts_three_attempts() {
    let scenario = Scenario::new("vaga-turno-esgota");
    let limiter = limiter_slot();
    let _busy = limiter
        .acquire_turn(scenario.org, scenario.agente, Duration::from_millis(10))
        .await
        .expect("ocupa a vaga da instância");
    let (engine, fake) = engine_with_limiter(
        &scenario,
        vec![response("não deveria ser chamada")],
        Arc::clone(&limiter),
    );
    let engine = Arc::new(engine);
    let fluxo = scenario.flow_with(
        Arc::clone(&engine),
        FlowOptions::default().with_failure_model(3, Duration::from_millis(1_000)),
    );
    let item = deliver(&fluxo, &scenario, "ext-vaga-turno-1", "oi");

    fluxo.dispatch(scenario.org).expect("despacha");
    wait_attempts(&scenario, item, 1).await;
    assert!(
        scenario
            .store
            .outbox()
            .list_pending(scenario.org, 10)
            .expect("saída")
            .is_empty()
    );
    scenario.advance(1_000);
    fluxo.dispatch(scenario.org).expect("despacha 2");
    wait_attempts(&scenario, item, 2).await;
    scenario.advance(4_000);
    fluxo.dispatch(scenario.org).expect("despacha 3");
    wait_queue_empty(&fluxo, &scenario).await;

    assert_eq!(scenario.turns().len(), 3);
    assert!(
        scenario
            .turns()
            .iter()
            .all(|t| t.state == TurnState::Failed)
    );
    assert_eq!(fake.received().len(), 0);
    let finished = scenario
        .store
        .inbox()
        .get(scenario.org, item)
        .expect("item");
    assert!(
        finished.processed_at.is_some(),
        "concluída depois do limite"
    );
    assert!(
        finished
            .ultimo_erro
            .as_deref()
            .unwrap_or_default()
            .contains("sem vaga"),
        "{:?}",
        finished.ultimo_erro
    );
    assert_eq!(scenario.messages().len(), 1, "só a mensagem do cliente");

    scenario.advance(3_600_000);
    fluxo.dispatch(scenario.org).expect("despacha de novo");
    leave_pass(60).await;
    assert_eq!(
        scenario.turns().len(),
        3,
        "nenhuma repetição depois do limite"
    );
    assert_eq!(fake.received().len(), 0);

    let metrics = engine.metrics();
    assert_eq!(metrics.failures_model_deferred.value(), 2);
    assert_eq!(metrics.failures_model_definitive.value(), 1);
    assert_eq!(metrics.failures_model_refused.value(), 0);
}

#[tokio::test]
async fn missing_slot_turn_first_attempt_reply_second() {
    let scenario = Scenario::new("vaga-turno-repete");
    let limiter = limiter_slot();
    let busy = limiter
        .acquire_turn(scenario.org, scenario.agente, Duration::from_millis(10))
        .await
        .expect("ocupa a vaga da instância");
    let (engine, fake) = engine_with_limiter(
        &scenario,
        vec![response("bom dia, em que posso ajudar")],
        Arc::clone(&limiter),
    );
    let engine = Arc::new(engine);
    let fluxo = scenario.flow_with(
        Arc::clone(&engine),
        FlowOptions::default().with_failure_model(3, Duration::from_millis(1_000)),
    );
    let item = deliver(&fluxo, &scenario, "ext-vaga-turno-2", "oi");

    fluxo.dispatch(scenario.org).expect("despacha");
    wait_attempts(&scenario, item, 1).await;
    drop(busy);
    scenario.advance(1_000);
    fluxo.dispatch(scenario.org).expect("despacha 2");
    wait_queue_empty(&fluxo, &scenario).await;

    assert_eq!(fake.received().len(), 1);
    let replies = scenario
        .messages()
        .into_iter()
        .filter(|m| m.role == Role::Assistant)
        .count();
    assert_eq!(replies, 1);
    assert_eq!(
        scenario
            .store
            .outbox()
            .list_pending(scenario.org, 10)
            .expect("saída")
            .len(),
        1
    );
    let metrics = engine.metrics();
    assert_eq!(metrics.failures_model_deferred.value(), 1);
    assert_eq!(metrics.failures_model_definitive.value(), 0);
}

#[tokio::test]
async fn missing_slot_provider_enters_ladder_not_calls_model() {
    let scenario = Scenario::new("vaga-provedor");
    let limiter = limiter_slot();
    let mut busy = Some(
        limiter
            .acquire_provider(PROVIDER_AGENT, Duration::from_millis(10))
            .await
            .expect("ocupa a vaga do provedor"),
    );
    let (engine, fake) = engine_with_limiter(
        &scenario,
        vec![response("bom dia, em que posso ajudar")],
        Arc::clone(&limiter),
    );
    let engine = Arc::new(engine);
    let fluxo = scenario.flow_with(
        Arc::clone(&engine),
        FlowOptions::default().with_failure_model(3, Duration::from_millis(1_000)),
    );
    let item = deliver(&fluxo, &scenario, "ext-vaga-prov-1", "oi");

    fluxo.dispatch(scenario.org).expect("despacha");
    wait_attempts(&scenario, item, 1).await;
    assert_eq!(fake.received().len(), 0);
    assert!(
        scenario
            .store
            .outbox()
            .list_pending(scenario.org, 10)
            .expect("saída")
            .is_empty()
    );
    drop(busy.take());
    scenario.advance(1_000);
    fluxo.dispatch(scenario.org).expect("despacha 2");
    wait_queue_empty(&fluxo, &scenario).await;

    assert_eq!(fake.received().len(), 1, "uma única chamada ao provedor");
    assert_eq!(scenario.turns().len(), 2);
    let metrics = engine.metrics();
    assert_eq!(metrics.failures_model_deferred.value(), 1);
    assert_eq!(metrics.failures_model_definitive.value(), 0);
}

#[tokio::test]
async fn provider_refusal_key_not_spends_ladder() {
    let scenario = Scenario::new("recusa-401");
    let (engine, fake) = scenario.engine(
        vec![Script::Failure(LlmError::Rejected {
            status: 401,
            mensagem: "chave recusada".to_owned(),
        })],
        scenario.pause(),
    );
    let engine = Arc::new(engine);
    let fluxo = scenario.flow_with(
        Arc::clone(&engine),
        FlowOptions::default().with_failure_model(3, Duration::from_millis(1_000)),
    );
    let item = deliver(&fluxo, &scenario, "ext-401-1", "oi");

    fluxo.dispatch(scenario.org).expect("despacha");
    wait_queue_empty(&fluxo, &scenario).await;

    assert_eq!(fake.received().len(), 1);
    let turns = scenario.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].state, TurnState::Failed);
    let finished = scenario
        .store
        .inbox()
        .get(scenario.org, item)
        .expect("item");
    assert!(
        finished.processed_at.is_some(),
        "concluída na primeira tentativa"
    );
    assert!(
        finished
            .ultimo_erro
            .as_deref()
            .unwrap_or_default()
            .contains("401"),
        "{:?}",
        finished.ultimo_erro
    );
    assert!(
        scenario
            .store
            .outbox()
            .list_pending(scenario.org, 10)
            .expect("saída")
            .is_empty()
    );

    scenario.advance(3_600_000);
    fluxo.dispatch(scenario.org).expect("despacha de novo");
    leave_pass(60).await;
    assert_eq!(fake.received().len(), 1);

    let metrics = engine.metrics();
    assert_eq!(metrics.failures_model_refused.value(), 1);
    assert_eq!(metrics.failures_model_definitive.value(), 1);
    assert_eq!(metrics.failures_model_deferred.value(), 0);
}

#[derive(Default)]
struct TransportTest {
    sent: Mutex<Vec<OutboxItem>>,
}

impl TransportTest {
    fn sent(&self) -> Vec<OutboxItem> {
        self.sent
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}

impl OutputTransport for TransportTest {
    fn send(&self, item: OutboxItem) -> SendFuture {
        self.sent
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(item);
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn flow_complete_inbox_until_outbox_delivers_reply() {
    let scenario = Scenario::new("fluxo-completo");
    let (engine, fake) = scenario.engine(vec![response("claro, posso ajudar")], scenario.pause());
    let fluxo = scenario.fluxo(Arc::new(engine));

    let delivers = || ReceivedMessage {
        integration_id: scenario.integracao,
        id_externo: "ext-1".to_owned(),
        conversa: scenario.conversa,
        texto: "bom dia".to_owned(),
        attachments_json: "[]".to_owned(),
    };
    assert_eq!(
        fluxo
            .receive_message(scenario.org, delivers())
            .expect("grava")
            .situation,
        poph_store::Received::New
    );
    let second_one = fluxo
        .receive_message(scenario.org, delivers())
        .expect("reentrega da mesma mensagem");
    assert_eq!(
        second_one.situation,
        poph_store::Received::Duplicate,
        "a segunda entrega é duplicata, não erro"
    );

    fluxo.dispatch(scenario.org).expect("despacha");
    wait_queue_empty(&fluxo, &scenario).await;

    let transport = TransportTest::default();
    let report = fluxo
        .send_pending(scenario.org, &transport)
        .await
        .expect("envia");
    assert_eq!(report.sent, 1);
    let sent = transport.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].content_json,
        serde_json::json!({"texto": "claro, posso ajudar"}).to_string()
    );
    assert_eq!(
        sent[0].conversation_id, scenario.conversa,
        "o item de saída é da conversa do turno"
    );

    assert_eq!(fake.received().len(), 1, "a duplicata não abre turno");
    let turns = scenario.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].state, TurnState::Completed);
    let messages = scenario.messages();
    assert_eq!(messages.len(), 2, "uma fala do cliente e uma do agente");
}

async fn wait_turn_closed(scenario: &Scenario) {
    for _ in 0..300 {
        if scenario
            .turns()
            .iter()
            .any(|turno| turno.state != TurnState::Running)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("o turno não fechou a tempo");
}

async fn wait_queue_empty(fluxo: &Fluxo, scenario: &Scenario) {
    for _ in 0..300 {
        let pending = fluxo
            .statistics(scenario.org)
            .expect("lê as estatísticas")
            .entry_pending;
        let turn_running = scenario
            .turns()
            .iter()
            .any(|turno| turno.state == TurnState::Running);
        if pending == 0 && !turn_running {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("a fila não esvaziou a tempo");
}

fn failure_provider() -> Script {
    Script::Failure(LlmError::Unavailable {
        tentativas: 3,
        last_error: "503 indisponível".to_owned(),
    })
}

fn deliver(
    fluxo: &Fluxo,
    scenario: &Scenario,
    id_externo: &str,
    texto: &str,
) -> poph_types::InboxId {
    fluxo
        .receive_message(
            scenario.org,
            ReceivedMessage {
                integration_id: scenario.integracao,
                id_externo: id_externo.to_owned(),
                conversa: scenario.conversa,
                texto: texto.to_owned(),
                attachments_json: "[]".to_owned(),
            },
        )
        .expect("grava a entrada")
        .item
        .id
}

async fn wait_attempts(scenario: &Scenario, id: poph_types::InboxId, minimum: u32) {
    for _ in 0..500 {
        let current = scenario
            .store
            .inbox()
            .attempts(scenario.org, &[id])
            .expect("tentativas")[0];
        if current >= minimum {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("a tentativa {minimum} do item não foi registrada a tempo");
}

async fn leave_pass(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tokio::test]
async fn failure_model_repeats_with_wait_growing_answers_third() {
    let scenario = Scenario::new("falha-repete");
    let (engine, fake) = scenario.engine(
        vec![
            failure_provider(),
            failure_provider(),
            response("bom dia, em que posso ajudar"),
        ],
        scenario.pause(),
    );
    let engine = Arc::new(engine);
    let fluxo = scenario.flow_with(
        Arc::clone(&engine),
        FlowOptions::default().with_failure_model(3, Duration::from_millis(1_000)),
    );
    let item = deliver(&fluxo, &scenario, "ext-repete-1", "oi");

    fluxo.dispatch(scenario.org).expect("despacha");
    wait_attempts(&scenario, item, 1).await;
    assert_eq!(fake.received().len(), 1);
    assert_eq!(
        scenario
            .store
            .inbox()
            .count_pending(scenario.org)
            .expect("pendentes"),
        1,
        "adiado continua pendente"
    );
    assert!(
        scenario
            .store
            .outbox()
            .list_pending(scenario.org, 10)
            .expect("saída")
            .is_empty(),
        "nada foi para a saída por causa da falha"
    );

    fluxo
        .dispatch(scenario.org)
        .expect("despacha antes da hora");
    leave_pass(60).await;
    assert_eq!(scenario.turns().len(), 1, "nada roda antes do instante");
    assert_eq!(fake.received().len(), 1);

    scenario.advance(1_000);
    fluxo.dispatch(scenario.org).expect("despacha vencido");
    wait_attempts(&scenario, item, 2).await;
    assert_eq!(fake.received().len(), 2);

    scenario.advance(3_000);
    fluxo
        .dispatch(scenario.org)
        .expect("despacha antes da segunda espera");
    leave_pass(60).await;
    assert_eq!(
        fake.received().len(),
        2,
        "espera crescente: ainda não venceu"
    );
    scenario.advance(1_000);
    fluxo.dispatch(scenario.org).expect("despacha vencido 2");
    wait_queue_empty(&fluxo, &scenario).await;

    assert_eq!(fake.received().len(), 3);
    let states: Vec<TurnState> = scenario.turns().iter().map(|t| t.state).collect();
    assert_eq!(
        states.iter().filter(|e| **e == TurnState::Failed).count(),
        2
    );
    assert_eq!(
        states
            .iter()
            .filter(|e| **e == TurnState::Completed)
            .count(),
        1
    );

    let messages = scenario.messages();
    let replies: Vec<_> = messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .collect();
    assert_eq!(replies.len(), 1, "uma resposta do agente");
    let saida = scenario
        .store
        .outbox()
        .list_pending(scenario.org, 10)
        .expect("saída");
    assert_eq!(saida.len(), 1, "uma mensagem na outbox");
    assert_eq!(
        scenario
            .store
            .inbox()
            .count_pending(scenario.org)
            .expect("pendentes"),
        0
    );

    let metrics = engine.metrics();
    assert_eq!(metrics.failures_model_deferred.value(), 2);
    assert_eq!(metrics.failures_model_definitive.value(), 0);
}

#[tokio::test]
async fn failure_model_always_exhausts_three_attempts_concludes_without_reply() {
    let scenario = Scenario::new("falha-esgota");
    let (engine, fake) = scenario.engine(
        vec![failure_provider(), failure_provider(), failure_provider()],
        scenario.pause(),
    );
    let engine = Arc::new(engine);
    let fluxo = scenario.flow_with(
        Arc::clone(&engine),
        FlowOptions::default().with_failure_model(3, Duration::from_millis(1_000)),
    );
    let item = deliver(&fluxo, &scenario, "ext-esgota-1", "oi");

    fluxo.dispatch(scenario.org).expect("despacha");
    wait_attempts(&scenario, item, 1).await;
    scenario.advance(1_000);
    fluxo.dispatch(scenario.org).expect("despacha 2");
    wait_attempts(&scenario, item, 2).await;
    scenario.advance(4_000);
    fluxo.dispatch(scenario.org).expect("despacha 3");
    wait_queue_empty(&fluxo, &scenario).await;

    assert_eq!(fake.received().len(), 3);
    let turns = scenario.turns();
    assert_eq!(turns.len(), 3);
    assert!(turns.iter().all(|t| t.state == TurnState::Failed));

    let finished = scenario
        .store
        .inbox()
        .get(scenario.org, item)
        .expect("item");
    assert!(finished.processed_at.is_some(), "a entrada foi concluída");
    assert!(
        finished
            .ultimo_erro
            .as_deref()
            .unwrap_or_default()
            .contains("503"),
        "{:?}",
        finished.ultimo_erro
    );
    assert!(
        scenario
            .store
            .outbox()
            .list_pending(scenario.org, 10)
            .expect("saída")
            .is_empty()
    );
    assert_eq!(scenario.messages().len(), 1, "só a mensagem do cliente");

    scenario.advance(3_600_000);
    fluxo.dispatch(scenario.org).expect("despacha de novo");
    leave_pass(60).await;
    assert_eq!(scenario.turns().len(), 3);
    assert_eq!(fake.received().len(), 3);

    let metrics = engine.metrics();
    assert_eq!(metrics.failures_model_deferred.value(), 2);
    assert_eq!(metrics.failures_model_definitive.value(), 1);
}

#[tokio::test]
async fn with_limit_attempt_first_failure_definitive() {
    let scenario = Scenario::new("falha-uma");
    let (engine, fake) = scenario.engine(vec![failure_provider()], scenario.pause());
    let engine = Arc::new(engine);
    let fluxo = scenario.flow_with(
        Arc::clone(&engine),
        FlowOptions::default().with_failure_model(1, Duration::from_millis(1_000)),
    );
    deliver(&fluxo, &scenario, "ext-uma-1", "oi");
    fluxo.dispatch(scenario.org).expect("despacha");
    wait_queue_empty(&fluxo, &scenario).await;
    assert_eq!(fake.received().len(), 1);
    assert_eq!(engine.metrics().failures_model_definitive.value(), 1);
    assert_eq!(engine.metrics().failures_model_deferred.value(), 0);
}

#[tokio::test]
async fn time_maximum_turn_counts_as_failure_model_enters_ladder() {
    let scenario = Scenario::new("tempo-maximo");
    let pendurado = Script::Pending {
        events: Vec::new(),
        after: Duration::from_secs(10),
        error: LlmError::TimedOut { seconds: 10 },
    };
    let options_turn = TurnOptions {
        turn_timeout: Duration::from_millis(50),
        ..TurnOptions::default()
    };
    let (engine, fake) = scenario.engine_with(
        vec![pendurado, response("consegui responder")],
        scenario.pause(),
        options_turn,
    );
    let engine = Arc::new(engine);
    let fluxo = scenario.flow_with(
        Arc::clone(&engine),
        FlowOptions::default().with_failure_model(3, Duration::from_millis(1_000)),
    );
    let item = deliver(&fluxo, &scenario, "ext-tempo-1", "oi");

    fluxo.dispatch(scenario.org).expect("despacha");
    wait_attempts(&scenario, item, 1).await;
    let turno = scenario.turns().into_iter().next().expect("um turno");
    assert_eq!(turno.state, TurnState::Failed);
    assert!(
        turno
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("tempo máximo"),
        "{:?}",
        turno.error
    );
    assert_eq!(engine.metrics().failures_model_deferred.value(), 1);

    scenario.advance(1_000);
    fluxo.dispatch(scenario.org).expect("despacha 2");
    wait_queue_empty(&fluxo, &scenario).await;
    assert_eq!(fake.received().len(), 2);
    assert_eq!(
        scenario
            .messages()
            .iter()
            .filter(|m| m.role == Role::Assistant)
            .count(),
        1
    );
}

#[tokio::test]
async fn repeat_after_reply_recorded_not_calls_model() {
    let scenario = Scenario::new("ja-respondida");
    let (engine, fake) = scenario.engine(
        vec![failure_provider(), response("não deveria ser usada")],
        scenario.pause(),
    );
    let engine = Arc::new(engine);
    let fluxo = scenario.flow_with(
        Arc::clone(&engine),
        FlowOptions::default().with_failure_model(3, Duration::from_millis(1_000)),
    );
    let item = deliver(&fluxo, &scenario, "ext-ja-1", "oi");
    fluxo.dispatch(scenario.org).expect("despacha");
    wait_attempts(&scenario, item, 1).await;
    assert_eq!(fake.received().len(), 1);

    scenario
        .store
        .messages()
        .write(
            scenario.org,
            NewMessage {
                conversation_id: scenario.conversa,
                direction: Direction::Outbound,
                role: Role::Assistant,
                texto: "resposta que já saiu".to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
        )
        .expect("grava a resposta anterior");

    scenario.advance(1_000);
    fluxo.dispatch(scenario.org).expect("despacha de novo");
    wait_queue_empty(&fluxo, &scenario).await;

    assert_eq!(fake.received().len(), 1, "o modelo não foi chamado de novo");
    assert_eq!(
        scenario
            .store
            .inbox()
            .count_pending(scenario.org)
            .expect("pendentes"),
        0
    );
    assert!(
        scenario
            .store
            .outbox()
            .list_pending(scenario.org, 10)
            .expect("saída")
            .is_empty(),
        "nenhuma resposta nova"
    );
}

#[tokio::test]
async fn item_deferred_return_together_with_message_new_not_before_instant() {
    let scenario = Scenario::new("adiado-com-nova");
    let (engine, fake) = scenario.engine(
        vec![failure_provider(), response("respondi as duas")],
        scenario.pause(),
    );
    let fluxo = scenario.flow_with(
        Arc::new(engine),
        FlowOptions::default().with_failure_model(3, Duration::from_millis(5_000)),
    );
    let first = deliver(&fluxo, &scenario, "ext-antecipa-1", "primeira");
    fluxo.dispatch(scenario.org).expect("despacha");
    wait_attempts(&scenario, first, 1).await;
    assert_eq!(scenario.turns().len(), 1);

    fluxo.dispatch(scenario.org).expect("despacha sem vencer");
    leave_pass(60).await;
    assert_eq!(scenario.turns().len(), 1, "o adiado não roda antes da hora");
    assert_eq!(fake.received().len(), 1);

    deliver(&fluxo, &scenario, "ext-antecipa-2", "segunda");
    fluxo.dispatch(scenario.org).expect("despacha a nova");
    wait_queue_empty(&fluxo, &scenario).await;

    assert_eq!(fake.received().len(), 2);
    let replies = scenario
        .messages()
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .count();
    assert_eq!(replies, 1, "uma resposta para as duas mensagens");
    assert_eq!(
        scenario
            .store
            .inbox()
            .count_pending(scenario.org)
            .expect("pendentes"),
        0
    );
}
