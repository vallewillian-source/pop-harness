use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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
    BatchResponse, Decisao, DecisionOrigin, FakeJev, G3_NOTHING, G5_NONE, GateEvaluator,
    GateOverrides, IdGate, JevEngine, JevError, JevOptions, JevResponse as ReplyJudge, JevResult,
    JevState,
};
use poph_store::{Direction, NewAgent, NewConversation, NewMessage, NewOrg, Role, Store};
use poph_types::{
    AgentId, ConversationId, JevGates, MessageId, OrgId, ProductId, StructuredInstructions,
    ToolConfig,
};

const NAME_CHEAP: &str = "Dedetização de baratas";
const PRICE_CHEAP: &str = "R$ 180,00";
const NAME_TERMITES: &str = "Descupinização de cupins";
const PRICE_TERMITES: &str = "R$ 350,00";

struct NoWait;

impl Pause for NoWait {
    fn wait(&self, _duration: Duration) -> FuturePause {
        Box::pin(async {})
    }
}

struct SearchSimple {
    passages: Vec<RecoveredPassage>,
}

impl KnowledgeSearcher for SearchSimple {
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

struct CatalogCounted {
    searches: Arc<AtomicU64>,
    products: Vec<FoundProduct>,
}

impl CatalogCounted {
    fn new(cheap: ProductId, cupins: ProductId, searches: Arc<AtomicU64>) -> Arc<Self> {
        Arc::new(Self {
            searches,
            products: vec![
                product(cheap, NAME_CHEAP, PRICE_CHEAP, 18_000),
                product(cupins, NAME_TERMITES, PRICE_TERMITES, 35_000),
            ],
        })
    }

    fn vazio(searches: Arc<AtomicU64>) -> Arc<Self> {
        Arc::new(Self {
            searches,
            products: Vec::new(),
        })
    }
}

fn product(id: ProductId, nome: &str, price: &str, cents: i64) -> FoundProduct {
    FoundProduct {
        product: id,
        nome: nome.to_owned(),
        category: "Pragas".to_owned(),
        price_text: price.to_owned(),
        price_cents: cents,
        unit: "por visita".to_owned(),
        includes: String::new(),
        exclui: String::new(),
        conditions: "Garantia de 90 dias.".to_owned(),
        similarity: 0.81,
        relevance: 0.5,
    }
}

impl ProductSearcher for CatalogCounted {
    fn search(
        &self,
        _org: OrgId,
        _agent: AgentId,
        _term: String,
        _category: Option<String>,
        _parameters: ProductParameters,
    ) -> FutureProducts<'_> {
        self.searches.fetch_add(1, Ordering::SeqCst);
        let products = self.products.clone();
        Box::pin(async move { Ok(products) })
    }
}

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    conversa: ConversationId,
    lookup: Arc<dyn KnowledgeSearcher>,
    catalog: Arc<dyn ProductSearcher>,
    searches_product: Arc<AtomicU64>,
    cheap: ProductId,
    metrics: MetricsTurn,
}

impl Scenario {
    fn new(label: &str, catalog_empty: bool) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-hr33-{label}-{}-{}",
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
        let cheap = ProductId::generate();
        let cupins = ProductId::generate();
        let searches_product = Arc::new(AtomicU64::new(0));
        let catalog: Arc<dyn ProductSearcher> = if catalog_empty {
            CatalogCounted::vazio(Arc::clone(&searches_product))
        } else {
            CatalogCounted::new(cheap, cupins, Arc::clone(&searches_product))
        };
        let lookup: Arc<dyn KnowledgeSearcher> = Arc::new(SearchSimple {
            passages: vec![RecoveredPassage {
                item: poph_types::KnowledgeItemId::generate(),
                title: "Prazo de entrega".to_owned(),
                group: "Políticas da loja".to_owned(),
                texto: "Prazo de entrega: até 8 dias úteis.".to_owned(),
                similarity: 0.8,
                relevance: 0.5,
            }],
        });
        Self {
            folder,
            store,
            org,
            conversa,
            lookup,
            catalog,
            searches_product,
            cheap,
            metrics: MetricsTurn::own(),
        }
    }

    fn catalog_searches(&self) -> u64 {
        self.searches_product.load(Ordering::SeqCst)
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
            .expect("grava a mensagem")
            .id
    }

    fn engine(
        &self,
        scripts: Vec<Script>,
        jev: Option<Arc<dyn GateEvaluator>>,
    ) -> (TurnEngine, Arc<FakeProvider>) {
        let fake = Arc::new(FakeProvider::with_scripts(scripts));
        let config = ConfigHot::new(
            Arc::clone(&self.store),
            HotConfigOptions::default(),
            MetricsMemory::own(),
        );
        let limiter = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
        let mut engine = TurnEngine::new(
            Arc::clone(&self.store),
            config,
            limiter,
            fake.clone(),
            TurnOptions::default(),
            self.metrics.clone(),
        )
        .with_pause(Arc::new(NoWait))
        .with_search(Arc::clone(&self.lookup))
        .with_products(Arc::clone(&self.catalog));
        let registro =
            QueryKnowledge::registro(Arc::clone(engine.hot_config()), Arc::clone(&self.lookup))
                .com(SearchProduct::new(
                    Arc::clone(engine.hot_config()),
                    Arc::clone(&self.catalog),
                ));
        engine = engine.with_tools(Arc::new(registro));
        if let Some(jev) = jev {
            engine = engine.with_jev(jev);
        }
        (engine, fake)
    }

    async fn turno(&self, engine: &TurnEngine, gatilho: MessageId) -> TurnSummary {
        engine
            .run(
                TurnInput::new(self.org, self.conversa, gatilho),
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

fn so_g5() -> JevGates {
    JevGates {
        g5: true,
        ..JevGates::default()
    }
}

fn engine_judge(jev: Arc<FakeJev>, gates: JevGates) -> Arc<dyn GateEvaluator> {
    Arc::new(JevEngine::new(jev, gates, "jev-latest"))
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

fn turns_json(scenario: &Scenario) -> (String, String) {
    let turns = scenario
        .store
        .turns()
        .list(
            scenario.org,
            scenario.conversa,
            poph_store::Page::with_limit(5),
        )
        .expect("lista os turnos");
    (turns[0].passages_json.clone(), turns[0].jev_json.clone())
}

#[tokio::test]
async fn g5_chooses_product_skips_tool() {
    let scenario = Scenario::new("escolhe", false);
    let cheap = scenario.cheap.to_string();
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g5_produto",
        cheap.as_str(),
    )])));
    let (engine, provedor) = scenario.engine(
        vec![script_text(
            "A dedetização de baratas custa R$ 180,00 por visita.",
        )],
        Some(engine_judge(Arc::clone(&jev), so_g5())),
    );

    let gatilho = scenario.say("quanto custa para matar as baratas daqui de casa?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert_eq!(summary.timings.tool_rounds, 0);
    assert_eq!(
        provedor.received().len(),
        1,
        "uma chamada ao modelo, sem ferramenta"
    );
    let request = &provedor.received()[0];
    let prompt = text_prompt(request);
    assert!(
        prompt.contains(NAME_CHEAP),
        "o produto escolhido entra no prompt"
    );
    assert!(prompt.contains(PRICE_CHEAP), "o preço vem do registro");
    assert!(
        !prompt.contains(NAME_TERMITES),
        "só o produto escolhido entra no bloco, não a lista inteira"
    );
    let tools = names_tools(request);
    assert!(
        !tools.contains(&"buscar_produto".to_owned()),
        "o catálogo já foi consultado: {tools:?}"
    );
    assert!(tools.contains(&"consultar_base".to_owned()), "{tools:?}");

    let (chunks_json, jev_json) = turns_json(&scenario);
    assert!(chunks_json.contains(&cheap), "{chunks_json}");
    assert!(
        chunks_json.contains("\"tipo\":\"produto\""),
        "{chunks_json}"
    );
    assert!(jev_json.contains("\"gate\":\"g5\""), "{jev_json}");
    assert!(jev_json.contains(&cheap), "{jev_json}");
    assert_eq!(scenario.metrics.products_chosen_g5.value(), 1);
    assert_eq!(
        scenario.catalog_searches(),
        1,
        "a busca de produtos roda uma vez, antes do modelo"
    );
}

struct JevInventor {
    product: String,
}

impl GateEvaluator for JevInventor {
    fn effective_gates(&self, _adjustment: Option<GateOverrides>) -> JevGates {
        JevGates {
            g5: true,
            ..JevGates::default()
        }
    }

    fn evaluate<'a>(
        &'a self,
        _state: JevState,
        _adjustment: Option<GateOverrides>,
    ) -> futures::future::BoxFuture<'a, JevResult> {
        let product = self.product.clone();
        Box::pin(async move {
            JevResult {
                decisions: vec![Decisao {
                    gate: IdGate::G5,
                    choice: Some(product),
                    origin: DecisionOrigin::Jev,
                    confidence: 0.99,
                    margin: 0.9,
                    motivo: None,
                    ms: 1,
                    tokens: 1,
                }],
                ..JevResult::default()
            }
        })
    }
}

#[tokio::test]
async fn g5_id_out_candidates_never_injected_code() {
    let scenario = Scenario::new("fora-candidatos", false);
    let jev: Arc<dyn GateEvaluator> = Arc::new(JevInventor {
        product: "p-de-outra-org".to_owned(),
    });
    let (engine, provedor) = scenario.engine(
        vec![script_text("Vou confirmar o valor com a equipe.")],
        Some(jev),
    );

    let gatilho = scenario.say("quanto custa a dedetização?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let request = &provedor.received()[0];
    let prompt = text_prompt(request);
    assert!(
        !prompt.contains(PRICE_CHEAP),
        "nada de produto fora dos candidatos"
    );
    assert!(!prompt.contains(NAME_TERMITES));
    assert!(names_tools(request).contains(&"buscar_produto".to_owned()));
    assert_eq!(scenario.metrics.products_chosen_g5.value(), 0);
}

#[tokio::test]
async fn g5_id_invented_keeps_behaviour_today() {
    let scenario = Scenario::new("inventado", false);
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g5_produto",
        "p-produto-que-nao-existe",
    )])));
    let (engine, provedor) = scenario.engine(
        vec![script_text("Vou confirmar o valor com a equipe.")],
        Some(engine_judge(Arc::clone(&jev), so_g5())),
    );

    let gatilho = scenario.say("quanto custa a dedetização?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let request = &provedor.received()[0];
    let prompt = text_prompt(request);
    assert!(
        !prompt.contains(NAME_CHEAP),
        "produto inventado não entra no prompt"
    );
    assert!(!prompt.contains(PRICE_CHEAP));
    let tools = names_tools(request);
    assert!(
        tools.contains(&"buscar_produto".to_owned()),
        "sem escolha válida, o LLM continua com a ferramenta: {tools:?}"
    );
    assert_eq!(scenario.metrics.products_chosen_g5.value(), 0);
}

#[tokio::test]
async fn g5_none_keeps_behaviour_today() {
    let scenario = Scenario::new("nenhum", false);
    let jev = Arc::new(FakeJev::with_choices(choices(&[("g5_produto", G5_NONE)])));
    let (engine, provedor) = scenario.engine(
        vec![script_text("Posso ajudar com o prazo e o orçamento.")],
        Some(engine_judge(Arc::clone(&jev), so_g5())),
    );

    let gatilho = scenario.say("qual é o prazo de entrega?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let request = &provedor.received()[0];
    assert!(!text_prompt(request).contains(PRICE_TERMITES));
    let tools = names_tools(request);
    assert!(tools.contains(&"buscar_produto".to_owned()), "{tools:?}");
}

#[tokio::test]
async fn g5_confidence_low_keeps_behaviour_today() {
    let scenario = Scenario::new("confianca", false);
    let cheap = scenario.cheap.to_string();
    let choice = cheap.clone();
    let jev = Arc::new(FakeJev::new(move |request: &poph_jev::JevRequest| {
        let mut replies = BTreeMap::new();
        for id in request.questions.keys() {
            let mut response = ReplyJudge {
                choice: Some(choice.clone()),
                confidence: 0.4,
                ..ReplyJudge::default()
            };
            response.probabilities.insert(choice.clone(), 0.4);
            response.probabilities.insert(G5_NONE.to_owned(), 0.4);
            replies.insert(id.clone(), response);
        }
        Ok(BatchResponse {
            answers: replies,
            tokens: 7,
            ..BatchResponse::default()
        })
    }));
    let (engine, provedor) = scenario.engine(
        vec![script_text("Vou verificar o valor.")],
        Some(engine_judge(Arc::clone(&jev), so_g5())),
    );

    let gatilho = scenario.say("quanto custa a dedetização?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let request = &provedor.received()[0];
    assert!(!text_prompt(request).contains(PRICE_CHEAP));
    assert!(names_tools(request).contains(&"buscar_produto".to_owned()));
}

#[tokio::test]
async fn g5_judge_out_ar_keeps_behaviour_today() {
    let scenario = Scenario::new("fora", false);
    let jev = Arc::new(FakeJev::failing(JevError::Status {
        code: 503,
        motivo: "Service Unavailable".to_owned(),
    }));
    let (engine, provedor) = scenario.engine(
        vec![script_text("Olá! Como posso ajudar?")],
        Some(engine_judge(Arc::clone(&jev), so_g5())),
    );

    let gatilho = scenario.say("bom dia, quanto custa a dedetização?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let request = &provedor.received()[0];
    assert!(!text_prompt(request).contains(PRICE_CHEAP));
    assert!(names_tools(request).contains(&"buscar_produto".to_owned()));
}

#[tokio::test]
async fn g5_judge_not_answers_deadline_keeps_behaviour_today() {
    let scenario = Scenario::new("prazo", false);
    let jev = Arc::new(FakeJev::with_choices(choices(&[])).with_delay(Duration::from_millis(200)));
    let evaluator: Arc<dyn GateEvaluator> = Arc::new(
        JevEngine::new(jev, so_g5(), "jev-latest").with_options(JevOptions {
            batch_timeout: Duration::from_millis(30),
            ..JevOptions::default()
        }),
    );
    let (engine, provedor) = scenario.engine(
        vec![script_text("Vou confirmar o valor com a equipe.")],
        Some(evaluator),
    );

    let gatilho = scenario.say("quanto custa a dedetização?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let request = &provedor.received()[0];
    assert!(!text_prompt(request).contains(PRICE_CHEAP));
    assert!(names_tools(request).contains(&"buscar_produto".to_owned()));
    let jev = summary
        .jev
        .expect("o resultado do G5 fica registrado mesmo no fallback");
    assert!(
        jev.failed(),
        "o prazo estourado aparece no relato da chamada"
    );
    assert_eq!(jev.reliable_choice(IdGate::G5), None);
}

#[tokio::test]
async fn g5_without_candidates_not_question() {
    let scenario = Scenario::new("vazio", true);
    let jev = Arc::new(FakeJev::with_choices(choices(&[])));
    let (engine, provedor) = scenario.engine(
        vec![script_text("Não encontrei esse produto no catálogo.")],
        Some(engine_judge(Arc::clone(&jev), so_g5())),
    );

    let gatilho = scenario.say("vocês fazem dedetização?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert!(
        jev.requests().is_empty(),
        "sem candidatos o G5 nem é perguntado: {:?}",
        jev.requests()
    );
    let request = &provedor.received()[0];
    assert!(names_tools(request).contains(&"buscar_produto".to_owned()));
}

#[tokio::test]
async fn g5_disabled_not_does_search_product_nor_question() {
    let scenario = Scenario::new("desligado", false);
    let jev = Arc::new(FakeJev::with_choices(choices(&[])));
    let (engine, provedor) = scenario.engine(
        vec![script_text("Olá! Como posso ajudar?")],
        Some(engine_judge(Arc::clone(&jev), JevGates::default())),
    );

    let gatilho = scenario.say("quanto custa a dedetização?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert!(
        summary.jev.is_none()
            || summary
                .jev
                .as_ref()
                .is_some_and(|jev| jev.decisions.is_empty())
    );
    assert_eq!(
        scenario.catalog_searches(),
        0,
        "o gate desligado não faz pré-consulta"
    );
    let request = &provedor.received()[0];
    assert!(names_tools(request).contains(&"buscar_produto".to_owned()));
}

#[tokio::test]
async fn g5_tool_invented_model_not_executed() {
    let scenario = Scenario::new("inventada", false);
    let cheap = scenario.cheap.to_string();
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g5_produto",
        cheap.as_str(),
    )])));
    let (engine, provedor) = scenario.engine(
        vec![
            Script::Events(vec![
                LlmEvent::Tool(ToolCall {
                    id: "c1".to_owned(),
                    nome: "buscar_produto".to_owned(),
                    arguments: serde_json::json!({"termo": "baratas"}),
                }),
                LlmEvent::Fim(FinishReason::ToolCalls),
            ]),
            script_text("A dedetização de baratas custa R$ 180,00."),
        ],
        Some(engine_judge(Arc::clone(&jev), so_g5())),
    );

    let gatilho = scenario.say("quanto custa para matar baratas?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert_eq!(
        scenario.catalog_searches(),
        1,
        "só a pré-consulta do G5; a chamada inventada não busca de novo"
    );
    let requests = provedor.received();
    assert_eq!(requests.len(), 2);
    let resultado = text_prompt(&requests[1]);
    assert!(
        resultado.contains("já foi consultado"),
        "o modelo recebe o erro explicando que o catálogo já foi consultado: {resultado}"
    );
}

#[tokio::test]
async fn g3_nothing_skips_g5() {
    let scenario = Scenario::new("g3nada", false);
    let cheap = scenario.cheap.to_string();
    let jev = Arc::new(FakeJev::with_choices(choices(&[
        ("g3_precisa", G3_NOTHING),
        ("g5_produto", cheap.as_str()),
    ])));
    let gates = JevGates {
        g3: true,
        g5: true,
        ..JevGates::default()
    };
    let (engine, provedor) = scenario.engine(
        vec![script_text("Oi! Tudo bem? Como posso ajudar?")],
        Some(engine_judge(Arc::clone(&jev), gates)),
    );

    let gatilho = scenario.say("oi, bom dia");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert_eq!(scenario.catalog_searches(), 0);
    let tools = names_tools(&provedor.received()[0]);
    assert!(
        !tools.contains(&"buscar_produto".to_owned()),
        "o G3 em nada já tira o catálogo do turno: {tools:?}"
    );
}

#[test]
fn adjustment_only_enables_gate_request() {
    let adjustment = GateOverrides::only(IdGate::G5);
    let effective = adjustment.with_base(JevGates {
        g1: true,
        g3: true,
        ..JevGates::default()
    });
    assert!(effective.g5);
    assert!(!effective.g1 && !effective.g2 && !effective.g3 && !effective.g4);
}

#[tokio::test]
async fn measurement_rounds_tool_by_turn() {
    let without = Scenario::new("medicao-sem", false);
    let jev = Arc::new(FakeJev::with_choices(choices(&[])));
    let (engine_without, _) = without.engine(
        vec![
            Script::Events(vec![
                LlmEvent::Tool(ToolCall {
                    id: "c1".to_owned(),
                    nome: "buscar_produto".to_owned(),
                    arguments: serde_json::json!({"termo": "baratas"}),
                }),
                LlmEvent::Fim(FinishReason::ToolCalls),
            ]),
            script_text("A dedetização de baratas custa R$ 180,00."),
        ],
        Some(engine_judge(Arc::clone(&jev), JevGates::default())),
    );
    let gatilho = without.say("quanto custa para matar baratas?");
    let without_g5 = without.turno(&engine_without, gatilho).await;
    assert_eq!(without_g5.estado, poph_store::TurnState::Completed);
    assert_eq!(without_g5.timings.tool_rounds, 1, "sem o G5 há uma rodada");

    let com = Scenario::new("medicao-com", false);
    let cheap = com.cheap.to_string();
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g5_produto",
        cheap.as_str(),
    )])));
    let (engine_with, _) = com.engine(
        vec![script_text("A dedetização de baratas custa R$ 180,00.")],
        Some(engine_judge(Arc::clone(&jev), so_g5())),
    );
    let gatilho = com.say("quanto custa para matar baratas?");
    let with_g5 = com.turno(&engine_with, gatilho).await;
    assert_eq!(with_g5.estado, poph_store::TurnState::Completed);
    assert_eq!(with_g5.timings.tool_rounds, 0, "com o G5 não há rodada");

    println!(
        "rodadas de ferramenta por turno: sem o G5 = {} · com o G5 = {}",
        without_g5.timings.tool_rounds, with_g5.timings.tool_rounds
    );
}

#[tokio::test]
async fn default_installation_enables_g5_without_configure_agent() {
    let scenario = Scenario::new("padrao-g5", false);
    let cheap = scenario.cheap.to_string();
    let jev = Arc::new(FakeJev::with_choices(choices(&[
        ("g3_precisa", poph_jev::G3_PRODUCT),
        ("g5_produto", cheap.as_str()),
    ])));
    let gates_default = poph_types::JevConfig::default().gates;
    let (engine, provedor) = scenario.engine(
        vec![script_text(
            "A dedetização de baratas custa R$ 180,00 por visita.",
        )],
        Some(engine_judge(Arc::clone(&jev), gates_default)),
    );

    let gatilho = scenario.say("quanto custa para matar as baratas daqui de casa?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert_eq!(summary.timings.tool_rounds, 0, "sem rodada de ferramenta");
    let request = &provedor.received()[0];
    assert!(
        text_prompt(request).contains(NAME_CHEAP),
        "o produto escolhido entra no prompt"
    );
    assert!(
        !names_tools(request).contains(&"buscar_produto".to_owned()),
        "com o G5 escolhido, buscar_produto sai do turno"
    );
}
