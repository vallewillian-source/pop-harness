use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use poph_agent::base_knowledge::{
    KnowledgeSearcher, QueryKnowledge, RecoveredPassage, SearchFuture, SearchParameters,
};
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::products::{
    FoundProduct, FutureProducts, ProductParameters, ProductSearcher, SearchProduct,
};
use poph_agent::turno::{
    FuturePause, NoopObserver, Pause, TurnEngine, TurnInput, TurnOptions, TurnSummary,
};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, Script, ToolCall, Usage};
use poph_jev::{
    BatchResponse, FakeJev, G3_BASE, G3_NOTHING, IdGate, JevEngine, JevError,
    JevResponse as ReplyJudge,
};
use poph_store::{Direction, NewAgent, NewConversation, NewMessage, NewOrg, Role, Store};
use poph_types::{
    AgentId, ConversationId, JevGates, MessageId, OrgId, StructuredInstructions, ToolConfig,
};

const TEXT_CHUNK: &str = "Prazo de entrega: até 8 dias úteis após a confirmação.";

struct NoWait;

impl Pause for NoWait {
    fn wait(&self, _duration: Duration) -> FuturePause {
        Box::pin(async {})
    }
}

struct SearchCounted {
    searches: Arc<AtomicU64>,
    lag: Duration,
    passages: Vec<RecoveredPassage>,
}

impl SearchCounted {
    fn new() -> Arc<Self> {
        Self::with_delay(Duration::ZERO)
    }

    fn with_delay(lag: Duration) -> Arc<Self> {
        Arc::new(Self {
            searches: Arc::new(AtomicU64::new(0)),
            lag,
            passages: vec![RecoveredPassage {
                item: poph_types::KnowledgeItemId::generate(),
                title: "Prazo de entrega".to_owned(),
                group: "Políticas da loja".to_owned(),
                texto: TEXT_CHUNK.to_owned(),
                similarity: 0.82,
                relevance: 0.5,
            }],
        })
    }

    fn counter(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.searches)
    }
}

impl KnowledgeSearcher for SearchCounted {
    fn search(
        &self,
        _org: OrgId,
        _agent: AgentId,
        _query: String,
        _parameters: SearchParameters,
    ) -> SearchFuture<'_> {
        self.searches.fetch_add(1, Ordering::SeqCst);
        let passages = self.passages.clone();
        let lag = self.lag;
        Box::pin(async move {
            if !lag.is_zero() {
                tokio::time::sleep(lag).await;
            }
            Ok(passages)
        })
    }
}

struct CatalogEmpty;

impl ProductSearcher for CatalogEmpty {
    fn search(
        &self,
        _org: OrgId,
        _agent: AgentId,
        _term: String,
        _category: Option<String>,
        _parameters: ProductParameters,
    ) -> FutureProducts<'_> {
        let vazio: Vec<FoundProduct> = Vec::new();
        Box::pin(async move { Ok(vazio) })
    }
}

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    conversa: ConversationId,
    lookup: Arc<dyn KnowledgeSearcher>,
    searches: Arc<AtomicU64>,
    metrics: MetricsTurn,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let lookup = SearchCounted::new();
        let counter = lookup.counter();
        let base: Arc<dyn KnowledgeSearcher> = lookup;
        Self::with_search(label, base, counter)
    }

    fn with_search(
        label: &str,
        lookup: Arc<dyn KnowledgeSearcher>,
        searches: Arc<AtomicU64>,
    ) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-hr31-{label}-{}-{}",
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
        let structured = StructuredInstructions {
            tools: vec![
                ToolConfig {
                    name: "consultar_base".to_owned(),
                    enabled: true,
                    when_to_use: String::new(),
                },
                ToolConfig {
                    name: "buscar_produto".to_owned(),
                    enabled: true,
                    when_to_use: String::new(),
                },
            ],
            ..StructuredInstructions::default()
        };
        store
            .instructions()
            .save(
                org,
                agente,
                0,
                &structured.to_json(),
                "# Bia\n\nVocê é a atendente da FixProLar.",
                true,
            )
            .expect("grava as instruções");
        let conversa = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("abre a conversa")
            .id;
        Self {
            folder,
            store,
            org,
            agente,
            conversa,
            lookup,
            searches,
            metrics: MetricsTurn::own(),
        }
    }

    fn searches(&self) -> u64 {
        self.searches.load(Ordering::SeqCst)
    }

    fn new_conversation(&self) -> ConversationId {
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

    fn say(&self, texto: &str) -> MessageId {
        self.say_in(self.conversa, texto)
    }

    fn say_in(&self, conversa: ConversationId, texto: &str) -> MessageId {
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

    fn engine(
        &self,
        scripts: Vec<Script>,
        jev: Option<Arc<JevEngine>>,
    ) -> (TurnEngine, Arc<FakeProvider>) {
        let fake = Arc::new(FakeProvider::with_scripts(scripts));
        let config = ConfigHot::new(
            Arc::clone(&self.store),
            HotConfigOptions::default(),
            MetricsMemory::own(),
        );
        let limiter = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
        let catalog: Arc<dyn ProductSearcher> = Arc::new(CatalogEmpty);
        let mut engine = TurnEngine::new(
            Arc::clone(&self.store),
            config,
            limiter,
            fake.clone(),
            TurnOptions::default(),
            self.metrics.clone(),
        )
        .with_pause(Arc::new(NoWait))
        .with_search(Arc::clone(&self.lookup));
        let registro =
            QueryKnowledge::registro(Arc::clone(engine.hot_config()), Arc::clone(&self.lookup))
                .com(SearchProduct::new(Arc::clone(engine.hot_config()), catalog));
        engine = engine.with_tools(Arc::new(registro));
        if let Some(jev) = jev {
            engine = engine.with_jev(jev);
        }
        (engine, fake)
    }

    async fn turno(&self, engine: &TurnEngine, gatilho: MessageId) -> TurnSummary {
        self.turn(engine, self.conversa, gatilho).await
    }

    async fn turn(
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
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

fn script_text(texto: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(texto.to_owned()),
        LlmEvent::Usage(Usage {
            input: 100,
            saida: 10,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

fn so_g3() -> JevGates {
    JevGates {
        g3: true,
        ..JevGates::default()
    }
}

fn engine_judge(jev: Arc<FakeJev>) -> Arc<JevEngine> {
    Arc::new(JevEngine::new(jev, so_g3(), "jev-latest"))
}

fn choices(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(id, choice)| ((*id).to_owned(), (*choice).to_owned()))
        .collect()
}

fn names_tools(request: &poph_core::llm::Request) -> Vec<String> {
    request.tools.iter().map(|tool| tool.nome.clone()).collect()
}

fn text_prompt(request: &poph_core::llm::Request) -> String {
    request
        .messages
        .iter()
        .map(|mensagem| match mensagem {
            poph_core::llm::Message::System(texto) => texto.clone(),
            poph_core::llm::Message::User(parts) => parts
                .iter()
                .map(|part| match part {
                    poph_core::llm::Part::Texto(texto) => texto.clone(),
                    _ => String::new(),
                })
                .collect::<Vec<_>>()
                .join("\n"),
            poph_core::llm::Message::Assistant { texto, .. } => texto.clone(),
            poph_core::llm::Message::Resultado { content, .. } => content.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn g3_nothing_discards_search_not_offers_base_nor_catalog() {
    let scenario = Scenario::new("nada");
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g3_precisa",
        G3_NOTHING,
    )])));
    let (engine, provedor) = scenario.engine(
        vec![script_text("Oi! Tudo bem? Como posso ajudar?")],
        Some(engine_judge(Arc::clone(&jev))),
    );

    let gatilho = scenario.say("oi, bom dia");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert_eq!(summary.texto, "Oi! Tudo bem? Como posso ajudar?");
    assert_eq!(
        summary.jev.as_ref().and_then(|jev| jev.choice(IdGate::G3)),
        Some(G3_NOTHING)
    );

    let requests = provedor.received();
    assert_eq!(requests.len(), 1, "uma chamada ao modelo");
    let prompt = text_prompt(&requests[0]);
    assert!(
        !prompt.contains(TEXT_CHUNK),
        "o trecho da base não pode entrar no prompt quando o G3 diz nada"
    );
    let tools = names_tools(&requests[0]);
    assert!(
        !tools.contains(&"consultar_base".to_owned()),
        "consultar_base não deve ser oferecida: {tools:?}"
    );
    assert!(
        !tools.contains(&"buscar_produto".to_owned()),
        "buscar_produto não deve ser oferecida: {tools:?}"
    );
    let turns = scenario
        .store
        .turns()
        .list(
            scenario.org,
            scenario.conversa,
            poph_store::Page::with_limit(5),
        )
        .expect("lista os turnos");
    assert_eq!(
        turns[0].passages_json, "[]",
        "o registro não guarda trecho quando o G3 diz nada"
    );
    assert_eq!(
        scenario.metrics.searches_puladas_g3.value(),
        1,
        "a métrica de buscas dispensadas pelo G3 conta o turno"
    );
}

#[tokio::test]
async fn g3_nothing_not_executes_tool_invented_model() {
    let scenario = Scenario::new("inventada");
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g3_precisa",
        G3_NOTHING,
    )])));
    let (engine, _) = scenario.engine(
        vec![
            Script::Events(vec![
                LlmEvent::Tool(ToolCall {
                    id: "c1".to_owned(),
                    nome: "consultar_base".to_owned(),
                    arguments: serde_json::json!({"consulta": "prazo"}),
                }),
                LlmEvent::Fim(FinishReason::ToolCalls),
            ]),
            script_text("Tudo certo por aqui!"),
        ],
        Some(engine_judge(Arc::clone(&jev))),
    );

    let gatilho = scenario.say("ok, obrigado");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert_eq!(
        scenario.searches(),
        1,
        "só a busca prévia do turno rodou; a ferramenta inventada não pode consultar a base"
    );
}

#[tokio::test]
async fn g3_base_keeps_search_tools() {
    let scenario = Scenario::new("base");
    let jev = Arc::new(FakeJev::with_choices(choices(&[("g3_precisa", G3_BASE)])));
    let (engine, provedor) = scenario.engine(
        vec![script_text("O prazo é de até 8 dias úteis.")],
        Some(engine_judge(Arc::clone(&jev))),
    );

    let gatilho = scenario.say("qual é o prazo de entrega para o interior?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let requests = provedor.received();
    assert_eq!(requests.len(), 1);
    assert!(
        text_prompt(&requests[0]).contains(TEXT_CHUNK),
        "com o G3 em base, o trecho recuperado entra no prompt"
    );
    let tools = names_tools(&requests[0]);
    assert!(tools.contains(&"consultar_base".to_owned()), "{tools:?}");
    assert!(tools.contains(&"buscar_produto".to_owned()), "{tools:?}");
}

#[tokio::test]
async fn g3_disabled_not_changes_behaviour() {
    let scenario = Scenario::new("desligado");
    let (engine, provedor) = scenario.engine(vec![script_text("Olá! Como posso ajudar?")], None);

    let gatilho = scenario.say("oi");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert!(
        summary.jev.is_none(),
        "sem gate ligado o turno não consulta o Jev"
    );
    let requests = provedor.received();
    assert!(text_prompt(&requests[0]).contains(TEXT_CHUNK));
    let tools = names_tools(&requests[0]);
    assert!(tools.contains(&"consultar_base".to_owned()), "{tools:?}");
    assert!(tools.contains(&"buscar_produto".to_owned()), "{tools:?}");
}

#[tokio::test]
async fn g3_without_reply_falls_behaviour_today() {
    let scenario = Scenario::new("abstencao");
    let jev = Arc::new(FakeJev::with_choices(BTreeMap::new()));
    let (engine, provedor) = scenario.engine(
        vec![script_text("Olá! Como posso ajudar?")],
        Some(engine_judge(Arc::clone(&jev))),
    );

    let gatilho = scenario.say("oi");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let requests = provedor.received();
    assert!(text_prompt(&requests[0]).contains(TEXT_CHUNK));
    let tools = names_tools(&requests[0]);
    assert!(tools.contains(&"consultar_base".to_owned()), "{tools:?}");
}

#[tokio::test]
async fn g3_confidence_low_falls_behaviour_today() {
    let scenario = Scenario::new("confianca");
    let jev = Arc::new(FakeJev::new(|request: &poph_jev::JevRequest| {
        let mut replies = BTreeMap::new();
        for id in request.questions.keys() {
            let mut response = ReplyJudge {
                choice: Some(G3_NOTHING.to_owned()),
                confidence: 0.4,
                ..ReplyJudge::default()
            };
            response.probabilities.insert(G3_NOTHING.to_owned(), 0.4);
            response.probabilities.insert(G3_BASE.to_owned(), 0.4);
            replies.insert(id.clone(), response);
        }
        Ok(BatchResponse {
            answers: replies,
            tokens: 7,
            ..BatchResponse::default()
        })
    }));
    let (engine, provedor) = scenario.engine(
        vec![script_text("Olá! Como posso ajudar?")],
        Some(engine_judge(Arc::clone(&jev))),
    );

    let gatilho = scenario.say("oi");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let requests = provedor.received();
    assert!(
        text_prompt(&requests[0]).contains(TEXT_CHUNK),
        "confiança baixa não pode descartar a base"
    );
}

#[tokio::test]
async fn g3_judge_out_ar_falls_behaviour_today() {
    let scenario = Scenario::new("fora");
    let jev = Arc::new(FakeJev::failing(JevError::Status {
        code: 503,
        motivo: "Service Unavailable".to_owned(),
    }));
    let (engine, provedor) = scenario.engine(
        vec![script_text("Olá! Como posso ajudar?")],
        Some(engine_judge(Arc::clone(&jev))),
    );

    let gatilho = scenario.say("oi");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let requests = provedor.received();
    assert!(text_prompt(&requests[0]).contains(TEXT_CHUNK));
    let tools = names_tools(&requests[0]);
    assert!(tools.contains(&"consultar_base".to_owned()), "{tools:?}");
}

#[tokio::test]
async fn g3_nothing_abandons_search_progress() {
    let scenario = Scenario::with_search(
        "abandona",
        SearchCounted::with_delay(Duration::from_millis(300)) as Arc<dyn KnowledgeSearcher>,
        Arc::new(AtomicU64::new(0)),
    );

    let judge_nothing = Arc::new(FakeJev::with_choices(choices(&[(
        "g3_precisa",
        G3_NOTHING,
    )])));
    let (engine, _) = scenario.engine(
        vec![script_text("Oi! Como posso ajudar?")],
        Some(engine_judge(Arc::clone(&judge_nothing))),
    );
    let gatilho = scenario.say("oi, bom dia");
    let beginning = Instant::now();
    let summary = scenario.turno(&engine, gatilho).await;
    let with_nothing = beginning.elapsed();
    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert!(
        with_nothing < Duration::from_millis(250),
        "com o G3 em nada o turno não pode esperar a busca: levou {with_nothing:?}"
    );

    let scenario_base = Scenario::with_search(
        "espera",
        SearchCounted::with_delay(Duration::from_millis(300)) as Arc<dyn KnowledgeSearcher>,
        Arc::new(AtomicU64::new(0)),
    );
    let jev_base = Arc::new(FakeJev::with_choices(choices(&[("g3_precisa", G3_BASE)])));
    let (engine, _) = scenario_base.engine(
        vec![script_text("O prazo é de até 8 dias úteis.")],
        Some(engine_judge(Arc::clone(&jev_base))),
    );
    let gatilho = scenario_base.say("qual é o prazo de entrega?");
    let beginning = Instant::now();
    let summary = scenario_base.turno(&engine, gatilho).await;
    let with_base = beginning.elapsed();
    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert!(
        with_base >= Duration::from_millis(300),
        "com o G3 em base a busca é aguardada: levou {with_base:?}"
    );
}

#[tokio::test]
async fn g3_nothing_with_transfer_not_breaks_turn() {
    let scenario = Scenario::with_search(
        "transferencia",
        SearchCounted::with_delay(Duration::from_millis(300)) as Arc<dyn KnowledgeSearcher>,
        Arc::new(AtomicU64::new(0)),
    );
    let current = scenario
        .store
        .agents()
        .get(scenario.org, scenario.agente)
        .expect("lê o agente");
    scenario
        .store
        .agents()
        .update(
            scenario.org,
            scenario.agente,
            current.version,
            poph_store::AgentChange {
                handoff_active: Some(true),
                ..poph_store::AgentChange::default()
            },
        )
        .expect("liga o handoff");

    let jev = Arc::new(FakeJev::with_choices(choices(&[
        ("g1_transferir", poph_jev::G1_TRANSFER),
        ("g3_precisa", G3_NOTHING),
    ])));
    let gates = JevGates {
        g1: true,
        g3: true,
        ..JevGates::default()
    };
    let engine_judge = Arc::new(JevEngine::new(jev, gates, "jev-latest"));
    let (engine, _) = scenario.engine(
        vec![script_text("Vou te passar para um atendente.")],
        Some(engine_judge),
    );

    let gatilho = scenario.say("quero falar com uma pessoa de verdade");
    let beginning = Instant::now();
    let summary = scenario.turno(&engine, gatilho).await;
    let elapsed = beginning.elapsed();

    assert!(
        elapsed < Duration::from_millis(250),
        "a busca não devia ser aguardada: levou {elapsed:?}"
    );
    assert_eq!(
        summary.jev.as_ref().and_then(|jev| jev.choice(IdGate::G1)),
        Some(poph_jev::G1_TRANSFER)
    );
    assert_eq!(
        scenario.metrics.searches_puladas_g3.value(),
        1,
        "o G3 contou a busca dispensada"
    );
}

struct SearchRealistic {
    passages: Vec<RecoveredPassage>,
}

impl KnowledgeSearcher for SearchRealistic {
    fn search(
        &self,
        _org: OrgId,
        _agent: AgentId,
        _query: String,
        _parameters: SearchParameters,
    ) -> SearchFuture<'_> {
        let passages = self.passages.clone();
        Box::pin(async move { Ok(passages) })
    }
}

fn chunks_realistic() -> Vec<RecoveredPassage> {
    let texto =
        "Política da loja: o prazo de entrega para o interior é de até 8 dias úteis após a \
confirmação do pagamento, e o frete é grátis acima de R$ 199,00. "
            .repeat(11);
    (0..3)
        .map(|index| RecoveredPassage {
            item: poph_types::KnowledgeItemId::generate(),
            title: format!("Item {} de política", index + 1),
            group: "Políticas da loja".to_owned(),
            texto: texto.clone(),
            similarity: 0.8 - (index as f32) * 0.05,
            relevance: 0.5,
        })
        .collect()
}

#[tokio::test]
#[ignore = "medição da HR-31: rodar com --ignored --nocapture"]
async fn measurement_tokens_latency_g3() {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/jev/g3.jsonl");
    let texto = std::fs::read_to_string(&file).expect("lê o conjunto g3.jsonl");
    let mut messages: Vec<String> = Vec::new();
    for line in texto.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let case: serde_json::Value = serde_json::from_str(line).expect("caso é JSON");
        if case["esperado"] == "nada" {
            let last = case["estado"]["mensagens"]
                .as_array()
                .and_then(|items| items.last())
                .and_then(|item| item["texto"].as_str())
                .unwrap_or_default()
                .to_owned();
            messages.push(last);
        }
    }

    let passages = chunks_realistic();
    let size_base: usize = passages
        .iter()
        .map(|passage| passage.texto.chars().count())
        .sum();

    let mut total_without = 0usize;
    let mut total_with = 0usize;
    let mut duration_without = 0u128;
    let mut duration_with = 0u128;
    let mut tools_without = 0usize;
    let mut tools_with = 0usize;

    for mensagem in &messages {
        let scenario = Scenario::with_search(
            "medicao-sem",
            Arc::new(SearchRealistic {
                passages: passages.clone(),
            }),
            Arc::new(AtomicU64::new(0)),
        );
        let (engine, provedor) = scenario.engine(vec![script_text("Oi! Como posso ajudar?")], None);
        let conversa = scenario.new_conversation();
        let gatilho = scenario.say_in(conversa, mensagem);
        let beginning = Instant::now();
        let summary = scenario.turn(&engine, conversa, gatilho).await;
        duration_without += beginning.elapsed().as_micros();
        assert_eq!(summary.estado, poph_store::TurnState::Completed);
        let request = provedor.received().remove(0);
        total_without += text_prompt(&request).chars().count();
        tools_without += request.tools.len();

        let scenario = Scenario::with_search(
            "medicao-com",
            Arc::new(SearchRealistic {
                passages: passages.clone(),
            }),
            Arc::new(AtomicU64::new(0)),
        );
        let jev = Arc::new(FakeJev::with_choices(choices(&[(
            "g3_precisa",
            G3_NOTHING,
        )])));
        let (engine, provedor) = scenario.engine(
            vec![script_text("Oi! Como posso ajudar?")],
            Some(engine_judge(Arc::clone(&jev))),
        );
        let conversa = scenario.new_conversation();
        let gatilho = scenario.say_in(conversa, mensagem);
        let beginning = Instant::now();
        let summary = scenario.turn(&engine, conversa, gatilho).await;
        duration_with += beginning.elapsed().as_micros();
        assert_eq!(summary.estado, poph_store::TurnState::Completed);
        let request = provedor.received().remove(0);
        total_with += text_prompt(&request).chars().count();
        tools_with += request.tools.len();
    }

    let cases = messages.len();
    let average_without = total_without / cases;
    let average_with = total_with / cases;
    let economia = average_without.saturating_sub(average_with);
    println!("\n=== Medição G3 (HR-31) — {cases} mensagens rotuladas `nada` ===");
    println!(
        "base de exemplo: {} trechos, {size_base} caracteres no total",
        passages.len()
    );
    println!("prompt médio SEM o G3: {average_without} caracteres");
    println!("prompt médio COM o G3 em `nada`: {average_with} caracteres");
    println!(
        "economia média: {economia} caracteres ({:.1}% do prompt) ≈ {} tokens por mensagem \
         (estimativa caracteres/4; o tokenizador do provedor não está no repositório)",
        (economia as f64) * 100.0 / (average_without as f64),
        economia / 4
    );
    println!(
        "ferramentas oferecidas por turno: {:.1} sem o G3, {:.1} com o G3",
        tools_without as f64 / cases as f64,
        tools_with as f64 / cases as f64
    );
    println!(
        "duração média do turno (parte local, provedor falso): {:.2} ms sem o G3, {:.2} ms com o G3",
        duration_without as f64 / cases as f64 / 1000.0,
        duration_with as f64 / cases as f64 / 1000.0
    );
    assert!(economia > 0, "com o G3 em nada o prompt tem de encolher");
}

#[tokio::test]
async fn default_installation_enables_g3_without_configure_agent() {
    let scenario = Scenario::new("padrao-g3");
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g3_precisa",
        G3_NOTHING,
    )])));
    let gates_default = poph_types::JevConfig::default().gates;
    let evaluator = Arc::new(JevEngine::new(jev, gates_default, "jev-latest"));
    let (engine, provedor) =
        scenario.engine(vec![script_text("Oi! Como posso ajudar?")], Some(evaluator));

    let gatilho = scenario.say("oi, bom dia");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert_eq!(
        summary.jev.as_ref().and_then(|jev| jev.choice(IdGate::G3)),
        Some(G3_NOTHING),
        "sem configuração no agente, o G3 do padrão de instalação precisa perguntar"
    );
    let requests = provedor.received();
    assert!(
        !text_prompt(&requests[0]).contains(TEXT_CHUNK),
        "com `nada`, a base não entra no prompt"
    );
    assert!(
        !names_tools(&requests[0]).contains(&"consultar_base".to_owned()),
        "com `nada`, consultar_base não é oferecida"
    );
}

#[tokio::test]
async fn agent_disables_g3_return_behaviour_today() {
    let scenario = Scenario::new("agente-g3-off");
    let current = scenario
        .store
        .agents()
        .get(scenario.org, scenario.agente)
        .expect("lê o agente");
    scenario
        .store
        .agents()
        .update(
            scenario.org,
            scenario.agente,
            current.version,
            poph_store::AgentChange {
                jev_json: Some(r#"{"g3": false}"#.to_owned()),
                ..poph_store::AgentChange::default()
            },
        )
        .expect("desliga o G3 no agente");

    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g3_precisa",
        G3_NOTHING,
    )])));
    let gates_default = poph_types::JevConfig::default().gates;
    let evaluator = Arc::new(JevEngine::new(jev, gates_default, "jev-latest"));
    let (engine, provedor) = scenario.engine(
        vec![script_text("Olá! Como posso ajudar?")],
        Some(evaluator),
    );

    let gatilho = scenario.say("oi");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert!(
        summary
            .jev
            .as_ref()
            .and_then(|jev| jev.choice(IdGate::G3))
            .is_none(),
        "com o G3 desligado no agente, o Jev não decide o G3"
    );
    let requests = provedor.received();
    assert!(text_prompt(&requests[0]).contains(TEXT_CHUNK));
    assert!(names_tools(&requests[0]).contains(&"consultar_base".to_owned()));
}

#[tokio::test]
async fn agent_with_active_false_not_query_judge_nor_g3() {
    let scenario = Scenario::new("agente-inativo-g3");
    let current = scenario
        .store
        .agents()
        .get(scenario.org, scenario.agente)
        .expect("lê o agente");
    scenario
        .store
        .agents()
        .update(
            scenario.org,
            scenario.agente,
            current.version,
            poph_store::AgentChange {
                jev_json: Some(r#"{"ativo": false}"#.to_owned()),
                ..poph_store::AgentChange::default()
            },
        )
        .expect("desliga o Jev do agente");

    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g3_precisa",
        G3_NOTHING,
    )])));
    let gates_default = poph_types::JevConfig::default().gates;
    let evaluator = Arc::new(JevEngine::new(jev, gates_default, "jev-latest"));
    let (engine, provedor) = scenario.engine(
        vec![script_text("Olá! Como posso ajudar?")],
        Some(evaluator),
    );

    let gatilho = scenario.say("oi");
    let summary = scenario.turno(&engine, gatilho).await;

    assert!(
        summary
            .jev
            .as_ref()
            .is_none_or(|jev| jev.decisions.is_empty()),
        "agente com ativo=false não recebe decisão nenhuma do Jev"
    );
    let requests = provedor.received();
    assert!(text_prompt(&requests[0]).contains(TEXT_CHUNK));
    assert!(names_tools(&requests[0]).contains(&"consultar_base".to_owned()));
}
