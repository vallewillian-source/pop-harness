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
    GateOverrides, IdGate, JevEngine, JevError, JevOptions, JevResponse as RespostaJev, JevResult,
    JevState,
};
use poph_store::{Direction, NewAgent, NewConversation, NewMessage, NewOrg, Role, Store};
use poph_types::{
    AgentId, ConversationId, JevGates, MessageId, OrgId, ProductId, StructuredInstructions,
    ToolConfig,
};

const NOME_BARATAS: &str = "Dedetização de baratas";
const PRECO_BARATAS: &str = "R$ 180,00";
const NOME_CUPINS: &str = "Descupinização de cupins";
const PRECO_CUPINS: &str = "R$ 350,00";

struct SemEspera;

impl Pause for SemEspera {
    fn wait(&self, _duracao: Duration) -> FuturePause {
        Box::pin(async {})
    }
}

struct BuscaSimples {
    passages: Vec<RecoveredPassage>,
}

impl KnowledgeSearcher for BuscaSimples {
    fn search(
        &self,
        _org: OrgId,
        _agent: AgentId,
        _consulta: String,
        _parametros: SearchParameters,
    ) -> SearchFuture<'_> {
        let passages = self.passages.clone();
        Box::pin(async move { Ok(passages) })
    }
}

struct CatalogoContado {
    buscas: Arc<AtomicU64>,
    products: Vec<FoundProduct>,
}

impl CatalogoContado {
    fn new(baratas: ProductId, cupins: ProductId, buscas: Arc<AtomicU64>) -> Arc<Self> {
        Arc::new(Self {
            buscas,
            products: vec![
                product(baratas, NOME_BARATAS, PRECO_BARATAS, 18_000),
                product(cupins, NOME_CUPINS, PRECO_CUPINS, 35_000),
            ],
        })
    }

    fn vazio(buscas: Arc<AtomicU64>) -> Arc<Self> {
        Arc::new(Self {
            buscas,
            products: Vec::new(),
        })
    }
}

fn product(id: ProductId, nome: &str, preco: &str, cents: i64) -> FoundProduct {
    FoundProduct {
        product: id,
        nome: nome.to_owned(),
        category: "Pragas".to_owned(),
        price_text: preco.to_owned(),
        price_cents: cents,
        unit: "por visita".to_owned(),
        includes: String::new(),
        exclui: String::new(),
        conditions: "Garantia de 90 dias.".to_owned(),
        similarity: 0.81,
        relevance: 0.5,
    }
}

impl ProductSearcher for CatalogoContado {
    fn search(
        &self,
        _org: OrgId,
        _agent: AgentId,
        _termo: String,
        _categoria: Option<String>,
        _parametros: ProductParameters,
    ) -> FutureProducts<'_> {
        self.buscas.fetch_add(1, Ordering::SeqCst);
        let products = self.products.clone();
        Box::pin(async move { Ok(products) })
    }
}

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    conversa: ConversationId,
    busca: Arc<dyn KnowledgeSearcher>,
    catalog: Arc<dyn ProductSearcher>,
    buscas_produto: Arc<AtomicU64>,
    baratas: ProductId,
    metrics: MetricsTurn,
}

impl Scenario {
    fn new(label: &str, catalogo_vazio: bool) -> Self {
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
        let baratas = ProductId::generate();
        let cupins = ProductId::generate();
        let buscas_produto = Arc::new(AtomicU64::new(0));
        let catalog: Arc<dyn ProductSearcher> = if catalogo_vazio {
            CatalogoContado::vazio(Arc::clone(&buscas_produto))
        } else {
            CatalogoContado::new(baratas, cupins, Arc::clone(&buscas_produto))
        };
        let busca: Arc<dyn KnowledgeSearcher> = Arc::new(BuscaSimples {
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
            busca,
            catalog,
            buscas_produto,
            baratas,
            metrics: MetricsTurn::own(),
        }
    }

    fn catalogo_buscas(&self) -> u64 {
        self.buscas_produto.load(Ordering::SeqCst)
    }

    fn falar(&self, texto: &str) -> MessageId {
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
        roteiros: Vec<Script>,
        jev: Option<Arc<dyn GateEvaluator>>,
    ) -> (TurnEngine, Arc<FakeProvider>) {
        let fake = Arc::new(FakeProvider::with_scripts(roteiros));
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
        .with_pause(Arc::new(SemEspera))
        .with_search(Arc::clone(&self.busca))
        .with_products(Arc::clone(&self.catalog));
        let registro =
            QueryKnowledge::registro(Arc::clone(engine.hot_config()), Arc::clone(&self.busca)).com(
                SearchProduct::new(Arc::clone(engine.hot_config()), Arc::clone(&self.catalog)),
            );
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

fn roteiro_texto(texto: &str) -> Script {
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

fn motor_jev(jev: Arc<FakeJev>, gates: JevGates) -> Arc<dyn GateEvaluator> {
    Arc::new(JevEngine::new(jev, gates, "jev-latest"))
}

fn escolhas(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(id, choice)| ((*id).to_owned(), (*choice).to_owned()))
        .collect()
}

fn nomes_das_ferramentas(request: &poph_core::llm::Request) -> Vec<String> {
    request.tools.iter().map(|tool| tool.nome.clone()).collect()
}

fn texto_do_prompt(request: &poph_core::llm::Request) -> String {
    request
        .messages
        .iter()
        .map(|mensagem| match mensagem {
            poph_core::llm::Message::System(texto) => texto.clone(),
            poph_core::llm::Message::User(parts) => parts
                .iter()
                .map(|parte| match parte {
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

fn turnos_json(scenario: &Scenario) -> (String, String) {
    let turnos = scenario
        .store
        .turns()
        .list(
            scenario.org,
            scenario.conversa,
            poph_store::Page::with_limit(5),
        )
        .expect("lista os turnos");
    (turnos[0].passages_json.clone(), turnos[0].jev_json.clone())
}

#[tokio::test]
async fn g5_escolhe_o_produto_e_dispensa_a_ferramenta() {
    let scenario = Scenario::new("escolhe", false);
    let baratas = scenario.baratas.to_string();
    let jev = Arc::new(FakeJev::with_choices(escolhas(&[(
        "g5_produto",
        baratas.as_str(),
    )])));
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto(
            "A dedetização de baratas custa R$ 180,00 por visita.",
        )],
        Some(motor_jev(Arc::clone(&jev), so_g5())),
    );

    let gatilho = scenario.falar("quanto custa para matar as baratas daqui de casa?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert_eq!(summary.timings.tool_rounds, 0);
    assert_eq!(
        provedor.received().len(),
        1,
        "uma chamada ao modelo, sem ferramenta"
    );
    let request = &provedor.received()[0];
    let prompt = texto_do_prompt(request);
    assert!(
        prompt.contains(NOME_BARATAS),
        "o produto escolhido entra no prompt"
    );
    assert!(prompt.contains(PRECO_BARATAS), "o preço vem do registro");
    assert!(
        !prompt.contains(NOME_CUPINS),
        "só o produto escolhido entra no bloco, não a lista inteira"
    );
    let tools = nomes_das_ferramentas(request);
    assert!(
        !tools.contains(&"buscar_produto".to_owned()),
        "o catálogo já foi consultado: {tools:?}"
    );
    assert!(tools.contains(&"consultar_base".to_owned()), "{tools:?}");

    let (trechos_json, jev_json) = turnos_json(&scenario);
    assert!(trechos_json.contains(&baratas), "{trechos_json}");
    assert!(
        trechos_json.contains("\"tipo\":\"produto\""),
        "{trechos_json}"
    );
    assert!(jev_json.contains("\"gate\":\"g5\""), "{jev_json}");
    assert!(jev_json.contains(&baratas), "{jev_json}");
    assert_eq!(scenario.metrics.products_chosen_g5.value(), 1);
    assert_eq!(
        scenario.catalogo_buscas(),
        1,
        "a busca de produtos roda uma vez, antes do modelo"
    );
}

struct JevInventor {
    product: String,
}

impl GateEvaluator for JevInventor {
    fn effective_gates(&self, _ajuste: Option<GateOverrides>) -> JevGates {
        JevGates {
            g5: true,
            ..JevGates::default()
        }
    }

    fn evaluate<'a>(
        &'a self,
        _estado: JevState,
        _ajuste: Option<GateOverrides>,
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
async fn g5_id_fora_dos_candidatos_nunca_e_injetado_pelo_codigo() {
    let scenario = Scenario::new("fora-candidatos", false);
    let jev: Arc<dyn GateEvaluator> = Arc::new(JevInventor {
        product: "p-de-outra-org".to_owned(),
    });
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto("Vou confirmar o valor com a equipe.")],
        Some(jev),
    );

    let gatilho = scenario.falar("quanto custa a dedetização?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let request = &provedor.received()[0];
    let prompt = texto_do_prompt(request);
    assert!(
        !prompt.contains(PRECO_BARATAS),
        "nada de produto fora dos candidatos"
    );
    assert!(!prompt.contains(NOME_CUPINS));
    assert!(nomes_das_ferramentas(request).contains(&"buscar_produto".to_owned()));
    assert_eq!(scenario.metrics.products_chosen_g5.value(), 0);
}

#[tokio::test]
async fn g5_id_inventado_mantem_o_comportamento_de_hoje() {
    let scenario = Scenario::new("inventado", false);
    let jev = Arc::new(FakeJev::with_choices(escolhas(&[(
        "g5_produto",
        "p-produto-que-nao-existe",
    )])));
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto("Vou confirmar o valor com a equipe.")],
        Some(motor_jev(Arc::clone(&jev), so_g5())),
    );

    let gatilho = scenario.falar("quanto custa a dedetização?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let request = &provedor.received()[0];
    let prompt = texto_do_prompt(request);
    assert!(
        !prompt.contains(NOME_BARATAS),
        "produto inventado não entra no prompt"
    );
    assert!(!prompt.contains(PRECO_BARATAS));
    let tools = nomes_das_ferramentas(request);
    assert!(
        tools.contains(&"buscar_produto".to_owned()),
        "sem escolha válida, o LLM continua com a ferramenta: {tools:?}"
    );
    assert_eq!(scenario.metrics.products_chosen_g5.value(), 0);
}

#[tokio::test]
async fn g5_nenhum_mantem_o_comportamento_de_hoje() {
    let scenario = Scenario::new("nenhum", false);
    let jev = Arc::new(FakeJev::with_choices(escolhas(&[("g5_produto", G5_NONE)])));
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto("Posso ajudar com o prazo e o orçamento.")],
        Some(motor_jev(Arc::clone(&jev), so_g5())),
    );

    let gatilho = scenario.falar("qual é o prazo de entrega?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let request = &provedor.received()[0];
    assert!(!texto_do_prompt(request).contains(PRECO_CUPINS));
    let tools = nomes_das_ferramentas(request);
    assert!(tools.contains(&"buscar_produto".to_owned()), "{tools:?}");
}

#[tokio::test]
async fn g5_confianca_baixa_mantem_o_comportamento_de_hoje() {
    let scenario = Scenario::new("confianca", false);
    let baratas = scenario.baratas.to_string();
    let choice = baratas.clone();
    let jev = Arc::new(FakeJev::new(move |request: &poph_jev::JevRequest| {
        let mut respostas = BTreeMap::new();
        for id in request.questions.keys() {
            let mut response = RespostaJev {
                choice: Some(choice.clone()),
                confidence: 0.4,
                ..RespostaJev::default()
            };
            response.probabilities.insert(choice.clone(), 0.4);
            response.probabilities.insert(G5_NONE.to_owned(), 0.4);
            respostas.insert(id.clone(), response);
        }
        Ok(BatchResponse {
            answers: respostas,
            tokens: 7,
            ..BatchResponse::default()
        })
    }));
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto("Vou verificar o valor.")],
        Some(motor_jev(Arc::clone(&jev), so_g5())),
    );

    let gatilho = scenario.falar("quanto custa a dedetização?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let request = &provedor.received()[0];
    assert!(!texto_do_prompt(request).contains(PRECO_BARATAS));
    assert!(nomes_das_ferramentas(request).contains(&"buscar_produto".to_owned()));
}

#[tokio::test]
async fn g5_jev_fora_do_ar_mantem_o_comportamento_de_hoje() {
    let scenario = Scenario::new("fora", false);
    let jev = Arc::new(FakeJev::failing(JevError::Status {
        code: 503,
        motivo: "Service Unavailable".to_owned(),
    }));
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto("Olá! Como posso ajudar?")],
        Some(motor_jev(Arc::clone(&jev), so_g5())),
    );

    let gatilho = scenario.falar("bom dia, quanto custa a dedetização?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let request = &provedor.received()[0];
    assert!(!texto_do_prompt(request).contains(PRECO_BARATAS));
    assert!(nomes_das_ferramentas(request).contains(&"buscar_produto".to_owned()));
}

#[tokio::test]
async fn g5_jev_que_nao_responde_em_prazo_mantem_o_comportamento_de_hoje() {
    let scenario = Scenario::new("prazo", false);
    let jev = Arc::new(FakeJev::with_choices(escolhas(&[])).with_delay(Duration::from_millis(200)));
    let evaluator: Arc<dyn GateEvaluator> = Arc::new(
        JevEngine::new(jev, so_g5(), "jev-latest").with_options(JevOptions {
            batch_timeout: Duration::from_millis(30),
            ..JevOptions::default()
        }),
    );
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto("Vou confirmar o valor com a equipe.")],
        Some(evaluator),
    );

    let gatilho = scenario.falar("quanto custa a dedetização?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let request = &provedor.received()[0];
    assert!(!texto_do_prompt(request).contains(PRECO_BARATAS));
    assert!(nomes_das_ferramentas(request).contains(&"buscar_produto".to_owned()));
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
async fn g5_sem_candidatos_nao_pergunta() {
    let scenario = Scenario::new("vazio", true);
    let jev = Arc::new(FakeJev::with_choices(escolhas(&[])));
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto("Não encontrei esse produto no catálogo.")],
        Some(motor_jev(Arc::clone(&jev), so_g5())),
    );

    let gatilho = scenario.falar("vocês fazem dedetização?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert!(
        jev.requests().is_empty(),
        "sem candidatos o G5 nem é perguntado: {:?}",
        jev.requests()
    );
    let request = &provedor.received()[0];
    assert!(nomes_das_ferramentas(request).contains(&"buscar_produto".to_owned()));
}

#[tokio::test]
async fn g5_desligado_nao_faz_busca_de_produto_nem_pergunta() {
    let scenario = Scenario::new("desligado", false);
    let jev = Arc::new(FakeJev::with_choices(escolhas(&[])));
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto("Olá! Como posso ajudar?")],
        Some(motor_jev(Arc::clone(&jev), JevGates::default())),
    );

    let gatilho = scenario.falar("quanto custa a dedetização?");
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
        scenario.catalogo_buscas(),
        0,
        "o gate desligado não faz pré-consulta"
    );
    let request = &provedor.received()[0];
    assert!(nomes_das_ferramentas(request).contains(&"buscar_produto".to_owned()));
}

#[tokio::test]
async fn g5_ferramenta_inventada_pelo_modelo_nao_e_executada() {
    let scenario = Scenario::new("inventada", false);
    let baratas = scenario.baratas.to_string();
    let jev = Arc::new(FakeJev::with_choices(escolhas(&[(
        "g5_produto",
        baratas.as_str(),
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
            roteiro_texto("A dedetização de baratas custa R$ 180,00."),
        ],
        Some(motor_jev(Arc::clone(&jev), so_g5())),
    );

    let gatilho = scenario.falar("quanto custa para matar baratas?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert_eq!(
        scenario.catalogo_buscas(),
        1,
        "só a pré-consulta do G5; a chamada inventada não busca de novo"
    );
    let pedidos = provedor.received();
    assert_eq!(pedidos.len(), 2);
    let resultado = texto_do_prompt(&pedidos[1]);
    assert!(
        resultado.contains("já foi consultado"),
        "o modelo recebe o erro explicando que o catálogo já foi consultado: {resultado}"
    );
}

#[tokio::test]
async fn g3_nada_dispensa_o_g5() {
    let scenario = Scenario::new("g3nada", false);
    let baratas = scenario.baratas.to_string();
    let jev = Arc::new(FakeJev::with_choices(escolhas(&[
        ("g3_precisa", G3_NOTHING),
        ("g5_produto", baratas.as_str()),
    ])));
    let gates = JevGates {
        g3: true,
        g5: true,
        ..JevGates::default()
    };
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto("Oi! Tudo bem? Como posso ajudar?")],
        Some(motor_jev(Arc::clone(&jev), gates)),
    );

    let gatilho = scenario.falar("oi, bom dia");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert_eq!(scenario.catalogo_buscas(), 0);
    let tools = nomes_das_ferramentas(&provedor.received()[0]);
    assert!(
        !tools.contains(&"buscar_produto".to_owned()),
        "o G3 em nada já tira o catálogo do turno: {tools:?}"
    );
}

#[test]
fn ajuste_apenas_liga_so_o_gate_pedido() {
    let ajuste = GateOverrides::only(IdGate::G5);
    let efetivos = ajuste.with_base(JevGates {
        g1: true,
        g3: true,
        ..JevGates::default()
    });
    assert!(efetivos.g5);
    assert!(!efetivos.g1 && !efetivos.g2 && !efetivos.g3 && !efetivos.g4);
}

#[tokio::test]
async fn medicao_rodadas_de_ferramenta_por_turno() {
    let without = Scenario::new("medicao-sem", false);
    let jev = Arc::new(FakeJev::with_choices(escolhas(&[])));
    let (motor_sem, _) = without.engine(
        vec![
            Script::Events(vec![
                LlmEvent::Tool(ToolCall {
                    id: "c1".to_owned(),
                    nome: "buscar_produto".to_owned(),
                    arguments: serde_json::json!({"termo": "baratas"}),
                }),
                LlmEvent::Fim(FinishReason::ToolCalls),
            ]),
            roteiro_texto("A dedetização de baratas custa R$ 180,00."),
        ],
        Some(motor_jev(Arc::clone(&jev), JevGates::default())),
    );
    let gatilho = without.falar("quanto custa para matar baratas?");
    let sem_g5 = without.turno(&motor_sem, gatilho).await;
    assert_eq!(sem_g5.estado, poph_store::TurnState::Completed);
    assert_eq!(sem_g5.timings.tool_rounds, 1, "sem o G5 há uma rodada");

    let com = Scenario::new("medicao-com", false);
    let baratas = com.baratas.to_string();
    let jev = Arc::new(FakeJev::with_choices(escolhas(&[(
        "g5_produto",
        baratas.as_str(),
    )])));
    let (motor_com, _) = com.engine(
        vec![roteiro_texto("A dedetização de baratas custa R$ 180,00.")],
        Some(motor_jev(Arc::clone(&jev), so_g5())),
    );
    let gatilho = com.falar("quanto custa para matar baratas?");
    let com_g5 = com.turno(&motor_com, gatilho).await;
    assert_eq!(com_g5.estado, poph_store::TurnState::Completed);
    assert_eq!(com_g5.timings.tool_rounds, 0, "com o G5 não há rodada");

    println!(
        "rodadas de ferramenta por turno: sem o G5 = {} · com o G5 = {}",
        sem_g5.timings.tool_rounds, com_g5.timings.tool_rounds
    );
}

#[tokio::test]
async fn padrao_de_instalacao_liga_o_g5_sem_configurar_o_agente() {
    let scenario = Scenario::new("padrao-g5", false);
    let baratas = scenario.baratas.to_string();
    let jev = Arc::new(FakeJev::with_choices(escolhas(&[
        ("g3_precisa", poph_jev::G3_PRODUCT),
        ("g5_produto", baratas.as_str()),
    ])));
    let gates_padrao = poph_types::JevConfig::default().gates;
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto(
            "A dedetização de baratas custa R$ 180,00 por visita.",
        )],
        Some(motor_jev(Arc::clone(&jev), gates_padrao)),
    );

    let gatilho = scenario.falar("quanto custa para matar as baratas daqui de casa?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert_eq!(summary.timings.tool_rounds, 0, "sem rodada de ferramenta");
    let request = &provedor.received()[0];
    assert!(
        texto_do_prompt(request).contains(NOME_BARATAS),
        "o produto escolhido entra no prompt"
    );
    assert!(
        !nomes_das_ferramentas(request).contains(&"buscar_produto".to_owned()),
        "com o G5 escolhido, buscar_produto sai do turno"
    );
}
