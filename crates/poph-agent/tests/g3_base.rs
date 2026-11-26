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
    JevResponse as RespostaJev,
};
use poph_store::{Direction, NewAgent, NewConversation, NewMessage, NewOrg, Role, Store};
use poph_types::{
    AgentId, ConversationId, JevGates, MessageId, OrgId, StructuredInstructions, ToolConfig,
};

const TEXTO_DO_TRECHO: &str = "Prazo de entrega: até 8 dias úteis após a confirmação.";

struct SemEspera;

impl Pause for SemEspera {
    fn wait(&self, _duracao: Duration) -> FuturePause {
        Box::pin(async {})
    }
}

struct BuscaContada {
    buscas: Arc<AtomicU64>,
    demora: Duration,
    passages: Vec<RecoveredPassage>,
}

impl BuscaContada {
    fn new() -> Arc<Self> {
        Self::com_demora(Duration::ZERO)
    }

    fn com_demora(demora: Duration) -> Arc<Self> {
        Arc::new(Self {
            buscas: Arc::new(AtomicU64::new(0)),
            demora,
            passages: vec![RecoveredPassage {
                item: poph_types::KnowledgeItemId::generate(),
                title: "Prazo de entrega".to_owned(),
                group: "Políticas da loja".to_owned(),
                texto: TEXTO_DO_TRECHO.to_owned(),
                similarity: 0.82,
                relevance: 0.5,
            }],
        })
    }

    fn contador(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.buscas)
    }
}

impl KnowledgeSearcher for BuscaContada {
    fn search(
        &self,
        _org: OrgId,
        _agent: AgentId,
        _consulta: String,
        _parametros: SearchParameters,
    ) -> SearchFuture<'_> {
        self.buscas.fetch_add(1, Ordering::SeqCst);
        let passages = self.passages.clone();
        let demora = self.demora;
        Box::pin(async move {
            if !demora.is_zero() {
                tokio::time::sleep(demora).await;
            }
            Ok(passages)
        })
    }
}

struct CatalogoVazio;

impl ProductSearcher for CatalogoVazio {
    fn search(
        &self,
        _org: OrgId,
        _agent: AgentId,
        _termo: String,
        _categoria: Option<String>,
        _parametros: ProductParameters,
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
    busca: Arc<dyn KnowledgeSearcher>,
    buscas: Arc<AtomicU64>,
    metrics: MetricsTurn,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let busca = BuscaContada::new();
        let contador = busca.contador();
        let base: Arc<dyn KnowledgeSearcher> = busca;
        Self::com_busca(label, base, contador)
    }

    fn com_busca(label: &str, busca: Arc<dyn KnowledgeSearcher>, buscas: Arc<AtomicU64>) -> Self {
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
            busca,
            buscas,
            metrics: MetricsTurn::own(),
        }
    }

    fn buscas(&self) -> u64 {
        self.buscas.load(Ordering::SeqCst)
    }

    fn nova_conversa(&self) -> ConversationId {
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

    fn falar(&self, texto: &str) -> MessageId {
        self.falar_em(self.conversa, texto)
    }

    fn falar_em(&self, conversa: ConversationId, texto: &str) -> MessageId {
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
        roteiros: Vec<Script>,
        jev: Option<Arc<JevEngine>>,
    ) -> (TurnEngine, Arc<FakeProvider>) {
        let fake = Arc::new(FakeProvider::with_scripts(roteiros));
        let config = ConfigHot::new(
            Arc::clone(&self.store),
            HotConfigOptions::default(),
            MetricsMemory::own(),
        );
        let limiter = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
        let catalog: Arc<dyn ProductSearcher> = Arc::new(CatalogoVazio);
        let mut engine = TurnEngine::new(
            Arc::clone(&self.store),
            config,
            limiter,
            fake.clone(),
            TurnOptions::default(),
            self.metrics.clone(),
        )
        .with_pause(Arc::new(SemEspera))
        .with_search(Arc::clone(&self.busca));
        let registro =
            QueryKnowledge::registro(Arc::clone(engine.hot_config()), Arc::clone(&self.busca))
                .com(SearchProduct::new(Arc::clone(engine.hot_config()), catalog));
        engine = engine.with_tools(Arc::new(registro));
        if let Some(jev) = jev {
            engine = engine.with_jev(jev);
        }
        (engine, fake)
    }

    async fn turno(&self, engine: &TurnEngine, gatilho: MessageId) -> TurnSummary {
        self.turno_em(engine, self.conversa, gatilho).await
    }

    async fn turno_em(
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

fn so_g3() -> JevGates {
    JevGates {
        g3: true,
        ..JevGates::default()
    }
}

fn motor_jev(jev: Arc<FakeJev>) -> Arc<JevEngine> {
    Arc::new(JevEngine::new(jev, so_g3(), "jev-latest"))
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

#[tokio::test]
async fn g3_nada_descarta_a_busca_e_nao_oferece_base_nem_catalogo() {
    let scenario = Scenario::new("nada");
    let jev = Arc::new(FakeJev::with_choices(escolhas(&[(
        "g3_precisa",
        G3_NOTHING,
    )])));
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto("Oi! Tudo bem? Como posso ajudar?")],
        Some(motor_jev(Arc::clone(&jev))),
    );

    let gatilho = scenario.falar("oi, bom dia");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert_eq!(summary.texto, "Oi! Tudo bem? Como posso ajudar?");
    assert_eq!(
        summary.jev.as_ref().and_then(|jev| jev.choice(IdGate::G3)),
        Some(G3_NOTHING)
    );

    let pedidos = provedor.received();
    assert_eq!(pedidos.len(), 1, "uma chamada ao modelo");
    let prompt = texto_do_prompt(&pedidos[0]);
    assert!(
        !prompt.contains(TEXTO_DO_TRECHO),
        "o trecho da base não pode entrar no prompt quando o G3 diz nada"
    );
    let tools = nomes_das_ferramentas(&pedidos[0]);
    assert!(
        !tools.contains(&"consultar_base".to_owned()),
        "consultar_base não deve ser oferecida: {tools:?}"
    );
    assert!(
        !tools.contains(&"buscar_produto".to_owned()),
        "buscar_produto não deve ser oferecida: {tools:?}"
    );
    let turnos = scenario
        .store
        .turns()
        .list(
            scenario.org,
            scenario.conversa,
            poph_store::Page::with_limit(5),
        )
        .expect("lista os turnos");
    assert_eq!(
        turnos[0].passages_json, "[]",
        "o registro não guarda trecho quando o G3 diz nada"
    );
    assert_eq!(
        scenario.metrics.searches_puladas_g3.value(),
        1,
        "a métrica de buscas dispensadas pelo G3 conta o turno"
    );
}

#[tokio::test]
async fn g3_nada_nao_executa_ferramenta_inventada_pelo_modelo() {
    let scenario = Scenario::new("inventada");
    let jev = Arc::new(FakeJev::with_choices(escolhas(&[(
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
            roteiro_texto("Tudo certo por aqui!"),
        ],
        Some(motor_jev(Arc::clone(&jev))),
    );

    let gatilho = scenario.falar("ok, obrigado");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert_eq!(
        scenario.buscas(),
        1,
        "só a busca prévia do turno rodou; a ferramenta inventada não pode consultar a base"
    );
}

#[tokio::test]
async fn g3_base_mantem_a_busca_e_as_ferramentas() {
    let scenario = Scenario::new("base");
    let jev = Arc::new(FakeJev::with_choices(escolhas(&[("g3_precisa", G3_BASE)])));
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto("O prazo é de até 8 dias úteis.")],
        Some(motor_jev(Arc::clone(&jev))),
    );

    let gatilho = scenario.falar("qual é o prazo de entrega para o interior?");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let pedidos = provedor.received();
    assert_eq!(pedidos.len(), 1);
    assert!(
        texto_do_prompt(&pedidos[0]).contains(TEXTO_DO_TRECHO),
        "com o G3 em base, o trecho recuperado entra no prompt"
    );
    let tools = nomes_das_ferramentas(&pedidos[0]);
    assert!(tools.contains(&"consultar_base".to_owned()), "{tools:?}");
    assert!(tools.contains(&"buscar_produto".to_owned()), "{tools:?}");
}

#[tokio::test]
async fn g3_desligado_nao_muda_o_comportamento() {
    let scenario = Scenario::new("desligado");
    let (engine, provedor) = scenario.engine(vec![roteiro_texto("Olá! Como posso ajudar?")], None);

    let gatilho = scenario.falar("oi");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert!(
        summary.jev.is_none(),
        "sem gate ligado o turno não consulta o Jev"
    );
    let pedidos = provedor.received();
    assert!(texto_do_prompt(&pedidos[0]).contains(TEXTO_DO_TRECHO));
    let tools = nomes_das_ferramentas(&pedidos[0]);
    assert!(tools.contains(&"consultar_base".to_owned()), "{tools:?}");
    assert!(tools.contains(&"buscar_produto".to_owned()), "{tools:?}");
}

#[tokio::test]
async fn g3_sem_resposta_cai_no_comportamento_de_hoje() {
    let scenario = Scenario::new("abstencao");
    let jev = Arc::new(FakeJev::with_choices(BTreeMap::new()));
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto("Olá! Como posso ajudar?")],
        Some(motor_jev(Arc::clone(&jev))),
    );

    let gatilho = scenario.falar("oi");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let pedidos = provedor.received();
    assert!(texto_do_prompt(&pedidos[0]).contains(TEXTO_DO_TRECHO));
    let tools = nomes_das_ferramentas(&pedidos[0]);
    assert!(tools.contains(&"consultar_base".to_owned()), "{tools:?}");
}

#[tokio::test]
async fn g3_confianca_baixa_cai_no_comportamento_de_hoje() {
    let scenario = Scenario::new("confianca");
    let jev = Arc::new(FakeJev::new(|request: &poph_jev::JevRequest| {
        let mut respostas = BTreeMap::new();
        for id in request.questions.keys() {
            let mut response = RespostaJev {
                choice: Some(G3_NOTHING.to_owned()),
                confidence: 0.4,
                ..RespostaJev::default()
            };
            response.probabilities.insert(G3_NOTHING.to_owned(), 0.4);
            response.probabilities.insert(G3_BASE.to_owned(), 0.4);
            respostas.insert(id.clone(), response);
        }
        Ok(BatchResponse {
            answers: respostas,
            tokens: 7,
            ..BatchResponse::default()
        })
    }));
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto("Olá! Como posso ajudar?")],
        Some(motor_jev(Arc::clone(&jev))),
    );

    let gatilho = scenario.falar("oi");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let pedidos = provedor.received();
    assert!(
        texto_do_prompt(&pedidos[0]).contains(TEXTO_DO_TRECHO),
        "confiança baixa não pode descartar a base"
    );
}

#[tokio::test]
async fn g3_jev_fora_do_ar_cai_no_comportamento_de_hoje() {
    let scenario = Scenario::new("fora");
    let jev = Arc::new(FakeJev::failing(JevError::Status {
        code: 503,
        motivo: "Service Unavailable".to_owned(),
    }));
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto("Olá! Como posso ajudar?")],
        Some(motor_jev(Arc::clone(&jev))),
    );

    let gatilho = scenario.falar("oi");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    let pedidos = provedor.received();
    assert!(texto_do_prompt(&pedidos[0]).contains(TEXTO_DO_TRECHO));
    let tools = nomes_das_ferramentas(&pedidos[0]);
    assert!(tools.contains(&"consultar_base".to_owned()), "{tools:?}");
}

#[tokio::test]
async fn g3_nada_abandona_a_busca_em_andamento() {
    let scenario = Scenario::com_busca(
        "abandona",
        BuscaContada::com_demora(Duration::from_millis(300)) as Arc<dyn KnowledgeSearcher>,
        Arc::new(AtomicU64::new(0)),
    );

    let jev_nada = Arc::new(FakeJev::with_choices(escolhas(&[(
        "g3_precisa",
        G3_NOTHING,
    )])));
    let (engine, _) = scenario.engine(
        vec![roteiro_texto("Oi! Como posso ajudar?")],
        Some(motor_jev(Arc::clone(&jev_nada))),
    );
    let gatilho = scenario.falar("oi, bom dia");
    let beginning = Instant::now();
    let summary = scenario.turno(&engine, gatilho).await;
    let com_nada = beginning.elapsed();
    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert!(
        com_nada < Duration::from_millis(250),
        "com o G3 em nada o turno não pode esperar a busca: levou {com_nada:?}"
    );

    let cenario_base = Scenario::com_busca(
        "espera",
        BuscaContada::com_demora(Duration::from_millis(300)) as Arc<dyn KnowledgeSearcher>,
        Arc::new(AtomicU64::new(0)),
    );
    let jev_base = Arc::new(FakeJev::with_choices(escolhas(&[("g3_precisa", G3_BASE)])));
    let (engine, _) = cenario_base.engine(
        vec![roteiro_texto("O prazo é de até 8 dias úteis.")],
        Some(motor_jev(Arc::clone(&jev_base))),
    );
    let gatilho = cenario_base.falar("qual é o prazo de entrega?");
    let beginning = Instant::now();
    let summary = cenario_base.turno(&engine, gatilho).await;
    let com_base = beginning.elapsed();
    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert!(
        com_base >= Duration::from_millis(300),
        "com o G3 em base a busca é aguardada: levou {com_base:?}"
    );
}

#[tokio::test]
async fn g3_nada_com_transferencia_nao_quebra_o_turno() {
    let scenario = Scenario::com_busca(
        "transferencia",
        BuscaContada::com_demora(Duration::from_millis(300)) as Arc<dyn KnowledgeSearcher>,
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

    let jev = Arc::new(FakeJev::with_choices(escolhas(&[
        ("g1_transferir", poph_jev::G1_TRANSFER),
        ("g3_precisa", G3_NOTHING),
    ])));
    let gates = JevGates {
        g1: true,
        g3: true,
        ..JevGates::default()
    };
    let motor_jev = Arc::new(JevEngine::new(jev, gates, "jev-latest"));
    let (engine, _) = scenario.engine(
        vec![roteiro_texto("Vou te passar para um atendente.")],
        Some(motor_jev),
    );

    let gatilho = scenario.falar("quero falar com uma pessoa de verdade");
    let beginning = Instant::now();
    let summary = scenario.turno(&engine, gatilho).await;
    let decorrido = beginning.elapsed();

    assert!(
        decorrido < Duration::from_millis(250),
        "a busca não devia ser aguardada: levou {decorrido:?}"
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

struct BuscaRealista {
    passages: Vec<RecoveredPassage>,
}

impl KnowledgeSearcher for BuscaRealista {
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

fn trechos_realistas() -> Vec<RecoveredPassage> {
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
async fn medicao_de_tokens_e_latencia_do_g3() {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/jev/g3.jsonl");
    let texto = std::fs::read_to_string(&file).expect("lê o conjunto g3.jsonl");
    let mut messages: Vec<String> = Vec::new();
    for line in texto.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let caso: serde_json::Value = serde_json::from_str(line).expect("caso é JSON");
        if caso["esperado"] == "nada" {
            let last = caso["estado"]["mensagens"]
                .as_array()
                .and_then(|items| items.last())
                .and_then(|item| item["texto"].as_str())
                .unwrap_or_default()
                .to_owned();
            messages.push(last);
        }
    }

    let passages = trechos_realistas();
    let tamanho_da_base: usize = passages
        .iter()
        .map(|passage| passage.texto.chars().count())
        .sum();

    let mut total_sem = 0usize;
    let mut total_com = 0usize;
    let mut duracao_sem = 0u128;
    let mut duracao_com = 0u128;
    let mut ferramentas_sem = 0usize;
    let mut ferramentas_com = 0usize;

    for mensagem in &messages {
        let scenario = Scenario::com_busca(
            "medicao-sem",
            Arc::new(BuscaRealista {
                passages: passages.clone(),
            }),
            Arc::new(AtomicU64::new(0)),
        );
        let (engine, provedor) =
            scenario.engine(vec![roteiro_texto("Oi! Como posso ajudar?")], None);
        let conversa = scenario.nova_conversa();
        let gatilho = scenario.falar_em(conversa, mensagem);
        let beginning = Instant::now();
        let summary = scenario.turno_em(&engine, conversa, gatilho).await;
        duracao_sem += beginning.elapsed().as_micros();
        assert_eq!(summary.estado, poph_store::TurnState::Completed);
        let request = provedor.received().remove(0);
        total_sem += texto_do_prompt(&request).chars().count();
        ferramentas_sem += request.tools.len();

        let scenario = Scenario::com_busca(
            "medicao-com",
            Arc::new(BuscaRealista {
                passages: passages.clone(),
            }),
            Arc::new(AtomicU64::new(0)),
        );
        let jev = Arc::new(FakeJev::with_choices(escolhas(&[(
            "g3_precisa",
            G3_NOTHING,
        )])));
        let (engine, provedor) = scenario.engine(
            vec![roteiro_texto("Oi! Como posso ajudar?")],
            Some(motor_jev(Arc::clone(&jev))),
        );
        let conversa = scenario.nova_conversa();
        let gatilho = scenario.falar_em(conversa, mensagem);
        let beginning = Instant::now();
        let summary = scenario.turno_em(&engine, conversa, gatilho).await;
        duracao_com += beginning.elapsed().as_micros();
        assert_eq!(summary.estado, poph_store::TurnState::Completed);
        let request = provedor.received().remove(0);
        total_com += texto_do_prompt(&request).chars().count();
        ferramentas_com += request.tools.len();
    }

    let casos = messages.len();
    let media_sem = total_sem / casos;
    let media_com = total_com / casos;
    let economia = media_sem.saturating_sub(media_com);
    println!("\n=== Medição G3 (HR-31) — {casos} mensagens rotuladas `nada` ===");
    println!(
        "base de exemplo: {} trechos, {tamanho_da_base} caracteres no total",
        passages.len()
    );
    println!("prompt médio SEM o G3: {media_sem} caracteres");
    println!("prompt médio COM o G3 em `nada`: {media_com} caracteres");
    println!(
        "economia média: {economia} caracteres ({:.1}% do prompt) ≈ {} tokens por mensagem \
         (estimativa caracteres/4; o tokenizador do provedor não está no repositório)",
        (economia as f64) * 100.0 / (media_sem as f64),
        economia / 4
    );
    println!(
        "ferramentas oferecidas por turno: {:.1} sem o G3, {:.1} com o G3",
        ferramentas_sem as f64 / casos as f64,
        ferramentas_com as f64 / casos as f64
    );
    println!(
        "duração média do turno (parte local, provedor falso): {:.2} ms sem o G3, {:.2} ms com o G3",
        duracao_sem as f64 / casos as f64 / 1000.0,
        duracao_com as f64 / casos as f64 / 1000.0
    );
    assert!(economia > 0, "com o G3 em nada o prompt tem de encolher");
}

#[tokio::test]
async fn padrao_de_instalacao_liga_o_g3_sem_configurar_o_agente() {
    let scenario = Scenario::new("padrao-g3");
    let jev = Arc::new(FakeJev::with_choices(escolhas(&[(
        "g3_precisa",
        G3_NOTHING,
    )])));
    let gates_padrao = poph_types::JevConfig::default().gates;
    let evaluator = Arc::new(JevEngine::new(jev, gates_padrao, "jev-latest"));
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto("Oi! Como posso ajudar?")],
        Some(evaluator),
    );

    let gatilho = scenario.falar("oi, bom dia");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Completed);
    assert_eq!(
        summary.jev.as_ref().and_then(|jev| jev.choice(IdGate::G3)),
        Some(G3_NOTHING),
        "sem configuração no agente, o G3 do padrão de instalação precisa perguntar"
    );
    let pedidos = provedor.received();
    assert!(
        !texto_do_prompt(&pedidos[0]).contains(TEXTO_DO_TRECHO),
        "com `nada`, a base não entra no prompt"
    );
    assert!(
        !nomes_das_ferramentas(&pedidos[0]).contains(&"consultar_base".to_owned()),
        "com `nada`, consultar_base não é oferecida"
    );
}

#[tokio::test]
async fn agente_que_desliga_o_g3_volta_ao_comportamento_de_hoje() {
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

    let jev = Arc::new(FakeJev::with_choices(escolhas(&[(
        "g3_precisa",
        G3_NOTHING,
    )])));
    let gates_padrao = poph_types::JevConfig::default().gates;
    let evaluator = Arc::new(JevEngine::new(jev, gates_padrao, "jev-latest"));
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto("Olá! Como posso ajudar?")],
        Some(evaluator),
    );

    let gatilho = scenario.falar("oi");
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
    let pedidos = provedor.received();
    assert!(texto_do_prompt(&pedidos[0]).contains(TEXTO_DO_TRECHO));
    assert!(nomes_das_ferramentas(&pedidos[0]).contains(&"consultar_base".to_owned()));
}

#[tokio::test]
async fn agente_com_ativo_false_nao_consulta_o_jev_nem_o_g3() {
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

    let jev = Arc::new(FakeJev::with_choices(escolhas(&[(
        "g3_precisa",
        G3_NOTHING,
    )])));
    let gates_padrao = poph_types::JevConfig::default().gates;
    let evaluator = Arc::new(JevEngine::new(jev, gates_padrao, "jev-latest"));
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto("Olá! Como posso ajudar?")],
        Some(evaluator),
    );

    let gatilho = scenario.falar("oi");
    let summary = scenario.turno(&engine, gatilho).await;

    assert!(
        summary
            .jev
            .as_ref()
            .is_none_or(|jev| jev.decisions.is_empty()),
        "agente com ativo=false não recebe decisão nenhuma do Jev"
    );
    let pedidos = provedor.received();
    assert!(texto_do_prompt(&pedidos[0]).contains(TEXTO_DO_TRECHO));
    assert!(nomes_das_ferramentas(&pedidos[0]).contains(&"consultar_base".to_owned()));
}
