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
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, Script, Usage};
use poph_jev::{
    BatchResponse, FakeJev, G8_DEFAULT_FAREWELL, JevEngine, JevError, JevResponse as ReplyJudge,
};
use poph_store::{
    AgentChange, ConversationStatus, Direction, NewAgent, NewConversation, NewMessage, Page, Role,
    Store, TurnState,
};
use poph_types::{
    AgentId, ConversationId, JevGates, MessageId, OrgId, StructuredInstructions, ToolConfig,
};

struct NoWait;

impl Pause for NoWait {
    fn wait(&self, _duration: Duration) -> FuturePause {
        Box::pin(async {})
    }
}

struct FakeSearch {
    searches: Arc<AtomicU64>,
    lag: Duration,
}

impl KnowledgeSearcher for FakeSearch {
    fn search(
        &self,
        _org: OrgId,
        _agent: AgentId,
        _query: String,
        _parameters: SearchParameters,
    ) -> SearchFuture<'_> {
        self.searches.fetch_add(1, Ordering::SeqCst);
        let lag = self.lag;
        Box::pin(async move {
            if !lag.is_zero() {
                tokio::time::sleep(lag).await;
            }
            let passage = RecoveredPassage {
                item: poph_types::KnowledgeItemId::generate(),
                title: "Prazo de entrega".to_owned(),
                group: "Políticas da loja".to_owned(),
                texto: "Prazo de entrega: até 8 dias úteis.".to_owned(),
                similarity: 0.82,
                relevance: 0.5,
            };
            Ok(vec![passage])
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
    conversa: ConversationId,
    lookup: Arc<dyn KnowledgeSearcher>,
    metrics: MetricsTurn,
}

impl Scenario {
    fn new(label: &str) -> Self {
        Self::with_delay(label, Duration::ZERO, "")
    }

    fn with_delay(label: &str, lag: Duration, jev_json: &str) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-hr36-{label}-{}-{}",
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
            .create(poph_store::NewOrg {
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
            .expect("cria o agente");
        let structured = StructuredInstructions {
            tools: vec![ToolConfig {
                name: "consultar_base".to_owned(),
                enabled: true,
                when_to_use: String::new(),
            }],
            ..StructuredInstructions::default()
        };
        store
            .instructions()
            .save(
                org,
                agente.id,
                0,
                &structured.to_json(),
                "# Bia\n\nVocê é a atendente da loja.",
                true,
            )
            .expect("grava as instruções");
        store
            .agents()
            .update(
                org,
                agente.id,
                agente.version,
                AgentChange {
                    jev_json: Some(jev_json.to_owned()),
                    handoff_active: Some(true),
                    ..AgentChange::default()
                },
            )
            .expect("grava o jev_json do agente");
        let conversa = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente.id,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("abre a conversa")
            .id;
        let lookup: Arc<dyn KnowledgeSearcher> = Arc::new(FakeSearch {
            searches: Arc::new(AtomicU64::new(0)),
            lag,
        });
        Self {
            folder,
            store,
            org,
            conversa,
            lookup,
            metrics: MetricsTurn::own(),
        }
    }

    fn reply_agent(&self, texto: &str) {
        self.store
            .messages()
            .write(
                self.org,
                NewMessage {
                    conversation_id: self.conversa,
                    direction: Direction::Outbound,
                    role: Role::Assistant,
                    texto: texto.to_owned(),
                    attachments_json: "[]".to_owned(),
                    external_id: None,
                    turn_id: None,
                    origin: poph_store::Origin::Agente,
                    author_name: None,
                },
            )
            .expect("grava a resposta anterior");
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
        engine
            .run(
                TurnInput::new(self.org, self.conversa, gatilho),
                &mut NoopObserver,
            )
            .await
            .expect("o turno roda")
    }

    fn outputs(&self) -> Vec<String> {
        self.store
            .messages()
            .list(self.org, self.conversa, Page::with_limit(200))
            .expect("lista as mensagens")
            .into_iter()
            .filter(|mensagem| mensagem.direction == Direction::Outbound)
            .map(|mensagem| mensagem.texto)
            .collect()
    }

    fn queue(&self) -> Vec<String> {
        self.store
            .outbox()
            .list_pending(self.org, 50)
            .expect("lista a fila de saída")
            .into_iter()
            .map(|item| item.content_json)
            .collect()
    }

    fn status(&self) -> ConversationStatus {
        self.store
            .conversations()
            .get(self.org, self.conversa)
            .expect("lê a conversa")
            .status
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

fn so_g8() -> JevGates {
    JevGates {
        g8: true,
        ..JevGates::default()
    }
}

type ReplyScripted = (&'static str, &'static str, f64);
type RepliesText = Vec<(String, String, f64)>;

fn judge_by_text(table: &[(&str, &[ReplyScripted])]) -> Arc<FakeJev> {
    let table: Vec<(String, RepliesText)> = table
        .iter()
        .map(|(texto, items)| {
            (
                (*texto).to_owned(),
                items
                    .iter()
                    .map(|(id, choice, confidence)| {
                        ((*id).to_owned(), (*choice).to_owned(), *confidence)
                    })
                    .collect(),
            )
        })
        .collect();
    Arc::new(FakeJev::new(move |request| {
        let last = last_speaks_client(&request.estado);
        let mut replies = BTreeMap::new();
        if let Some((_, items)) = table.iter().find(|(texto, _)| *texto == last) {
            for (id, choice, confidence) in items {
                replies.insert(
                    id.clone(),
                    ReplyJudge {
                        choice: Some(choice.clone()),
                        confidence: *confidence,
                        ..ReplyJudge::default()
                    },
                );
            }
        }
        Ok(BatchResponse {
            answers: replies,
            tokens: 12,
            ..BatchResponse::default()
        })
    }))
}

fn last_speaks_client(estado: &serde_json::Value) -> String {
    estado["conversa"]["mensagens"]
        .as_array()
        .and_then(|messages| {
            messages
                .iter()
                .rev()
                .find(|mensagem| mensagem["papel"] == "cliente")
        })
        .and_then(|mensagem| mensagem["texto"].as_str())
        .unwrap_or_default()
        .to_owned()
}

fn engine_judge(jev: Arc<FakeJev>, gates: JevGates) -> Arc<JevEngine> {
    Arc::new(JevEngine::new(jev, gates, "jev-latest"))
}

const ENDS: &str = "era só isso, obrigado";
const DUVIDA: &str = "obrigado, mas ainda tenho uma dúvida: e se a praga voltar?";
const RETURNED: &str = "oi, voltei. preciso de um orçamento";

#[tokio::test]
async fn g8_ends_without_call_model_records_farewell_default() {
    let scenario = Scenario::new("padrao");
    scenario.reply_agent("Mais alguma dúvida?");
    let jev = judge_by_text(&[(ENDS, &[("g8_encerrada", "encerrada", 1.0)])]);
    let (engine, provedor) = scenario.engine(
        vec![script_text("isto não deve sair")],
        Some(engine_judge(Arc::clone(&jev), so_g8())),
    );

    let gatilho = scenario.say(ENDS);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, G8_DEFAULT_FAREWELL);
    assert_eq!(
        provedor.received().len(),
        0,
        "o modelo de texto não é chamado"
    );
    assert_eq!(
        scenario.outputs(),
        vec!["Mais alguma dúvida?", G8_DEFAULT_FAREWELL]
    );
    assert_eq!(
        scenario.queue().len(),
        1,
        "a despedida vai para a fila de saída"
    );
    assert_eq!(scenario.metrics.closed_g8.value(), 1);
    assert_eq!(scenario.metrics.turns_finished.value(), 1);
    assert_eq!(
        scenario.status(),
        ConversationStatus::Open,
        "o encerramento não muda a conversa"
    );
}

#[tokio::test]
async fn g8_uses_farewell_configured_agent() {
    let scenario = Scenario::with_delay(
        "despedida",
        Duration::ZERO,
        r#"{"despedida": "Valeu, até a próxima!"}"#,
    );
    scenario.reply_agent("Mais alguma dúvida?");
    let jev = judge_by_text(&[(ENDS, &[("g8_encerrada", "encerrada", 1.0)])]);
    let (engine, provedor) = scenario.engine(
        vec![script_text("não sai")],
        Some(engine_judge(jev, so_g8())),
    );

    let gatilho = scenario.say(ENDS);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.texto, "Valeu, até a próxima!");
    assert_eq!(provedor.received().len(), 0);
    assert_eq!(
        scenario.outputs().last().map(String::as_str),
        Some("Valeu, até a próxima!")
    );
}

#[tokio::test]
async fn g8_farewell_empty_ends_without_send_message() {
    let scenario = Scenario::with_delay("silencio", Duration::ZERO, r#"{"despedida": ""}"#);
    scenario.reply_agent("Mais alguma dúvida?");
    let jev = judge_by_text(&[(ENDS, &[("g8_encerrada", "encerrada", 1.0)])]);
    let (engine, provedor) = scenario.engine(
        vec![script_text("não sai")],
        Some(engine_judge(jev, so_g8())),
    );

    let gatilho = scenario.say(ENDS);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert!(summary.texto.is_empty());
    assert_eq!(provedor.received().len(), 0);
    assert_eq!(
        scenario.outputs(),
        vec!["Mais alguma dúvida?"],
        "só a resposta anterior"
    );
    assert!(scenario.queue().is_empty());
    assert_eq!(scenario.metrics.closed_g8.value(), 1);
}

#[tokio::test]
async fn g8_message_next_reopens_conversation_return_model() {
    let scenario = Scenario::new("reabre");
    scenario.reply_agent("Mais alguma dúvida?");
    let jev = judge_by_text(&[
        (ENDS, &[("g8_encerrada", "encerrada", 1.0)]),
        (RETURNED, &[("g8_encerrada", "continua", 0.95)]),
    ]);
    let (engine, provedor) = scenario.engine(
        vec![script_text("Claro! Qual o tamanho do imóvel?")],
        Some(engine_judge(jev, so_g8())),
    );

    let first = scenario.say(ENDS);
    scenario.turno(&engine, first).await;
    assert_eq!(provedor.received().len(), 0);

    let second = scenario.say(RETURNED);
    let summary = scenario.turno(&engine, second).await;

    assert_eq!(summary.texto, "Claro! Qual o tamanho do imóvel?");
    assert_eq!(
        provedor.received().len(),
        1,
        "a mensagem seguinte entra no turno normal"
    );
    assert_eq!(
        scenario.outputs().last().map(String::as_str),
        Some("Claro! Qual o tamanho do imóvel?")
    );
}

#[tokio::test]
async fn g8_farewell_empty_not_swallows_message_next() {
    let scenario =
        Scenario::with_delay("silencio-seguinte", Duration::ZERO, r#"{"despedida": ""}"#);
    scenario.reply_agent("Mais alguma dúvida?");
    let jev = judge_by_text(&[
        (ENDS, &[("g8_encerrada", "encerrada", 1.0)]),
        (RETURNED, &[("g8_encerrada", "continua", 0.95)]),
    ]);
    let (engine, provedor) = scenario.engine(
        vec![script_text("Claro, em que posso ajudar?")],
        Some(engine_judge(jev, so_g8())),
    );

    let first = scenario.say(ENDS);
    scenario.turno(&engine, first).await;
    let second = scenario.say(RETURNED);
    let summary = scenario.turno(&engine, second).await;

    assert_eq!(summary.texto, "Claro, em que posso ajudar?");
    assert_eq!(provedor.received().len(), 1);
    assert_eq!(
        scenario.outputs().last().map(String::as_str),
        Some("Claro, em que posso ajudar?")
    );
}

#[tokio::test]
async fn g8_thanks_with_doubt_not_closes() {
    let scenario = Scenario::new("duvida");
    scenario.reply_agent("O serviço tem garantia de 6 meses.");
    let jev = judge_by_text(&[(DUVIDA, &[("g8_encerrada", "continua", 0.95)])]);
    let (engine, provedor) = scenario.engine(
        vec![script_text("Se a praga voltar, a garantia cobre a visita.")],
        Some(engine_judge(jev, so_g8())),
    );

    let gatilho = scenario.say(DUVIDA);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(
        summary.texto,
        "Se a praga voltar, a garantia cobre a visita."
    );
    assert_eq!(provedor.received().len(), 1, "a dúvida segue ao modelo");
    assert_eq!(scenario.metrics.closed_g8.value(), 0);
}

#[tokio::test]
async fn g8_messages_grouped_not_close() {
    let scenario = Scenario::new("agrupadas");
    scenario.reply_agent("Mais alguma dúvida?");
    let duvida = "e a garantia, vale por quanto tempo?";
    let jev = judge_by_text(&[(ENDS, &[("g8_encerrada", "encerrada", 1.0)])]);
    let (engine, provedor) = scenario.engine(
        vec![script_text("A garantia vale por 6 meses.")],
        Some(engine_judge(jev, so_g8())),
    );

    scenario.say(duvida);
    let gatilho = scenario.say(ENDS);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(
        summary.texto, "A garantia vale por 6 meses.",
        "a dúvida é respondida"
    );
    assert_eq!(provedor.received().len(), 1);
    assert_eq!(scenario.metrics.closed_g8.value(), 0);
}

#[tokio::test]
async fn g8_first_message_not_question_nor_closes() {
    let scenario = Scenario::new("primeira");
    let jev = judge_by_text(&[("obrigado", &[("g8_encerrada", "encerrada", 1.0)])]);
    let (engine, provedor) = scenario.engine(
        vec![script_text("De nada! Como posso ajudar?")],
        Some(engine_judge(Arc::clone(&jev), so_g8())),
    );

    let gatilho = scenario.say("obrigado");
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(
        jev.calls(),
        0,
        "sem fala anterior do agente, o G8 nem é perguntado"
    );
    assert_eq!(summary.texto, "De nada! Como posso ajudar?");
    assert_eq!(provedor.received().len(), 1);
    assert_eq!(scenario.metrics.closed_g8.value(), 0);
}

#[tokio::test]
async fn g8_disabled_not_question_nor_closes() {
    let scenario = Scenario::new("desligado");
    scenario.reply_agent("Mais alguma dúvida?");
    let jev = judge_by_text(&[(ENDS, &[("g8_encerrada", "encerrada", 1.0)])]);
    let (engine, provedor) = scenario.engine(
        vec![script_text("Até mais, qualquer coisa estou aqui.")],
        Some(engine_judge(Arc::clone(&jev), JevGates::default())),
    );

    let gatilho = scenario.say(ENDS);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(jev.calls(), 0, "com o G8 desligado nenhuma pergunta");
    assert_eq!(summary.texto, "Até mais, qualquer coisa estou aqui.");
    assert_eq!(
        provedor.received().len(),
        1,
        "comportamento de antes da HR-36"
    );
}

#[tokio::test]
async fn g8_failure_or_doubt_judge_follows_as_today() {
    let cases: Vec<(&str, Arc<FakeJev>)> = vec![
        (
            "jev fora do ar",
            Arc::new(FakeJev::failing(JevError::TimedOut { ms: 8000 })),
        ),
        (
            "confiança 0,79",
            judge_by_text(&[(ENDS, &[("g8_encerrada", "encerrada", 0.79)])]),
        ),
        (
            "escolha fora do contrato",
            judge_by_text(&[(ENDS, &[("g8_encerrada", "talvez", 1.0)])]),
        ),
        ("sem resposta", judge_by_text(&[])),
    ];
    for (label, jev) in cases {
        let scenario = Scenario::new(&format!("falha-{}", label.replace(' ', "-")));
        scenario.reply_agent("Mais alguma dúvida?");
        let (engine, provedor) = scenario.engine(
            vec![script_text("Certo, qualquer coisa estou aqui.")],
            Some(engine_judge(jev, so_g8())),
        );

        let gatilho = scenario.say(ENDS);
        let summary = scenario.turno(&engine, gatilho).await;

        assert_eq!(
            summary.texto, "Certo, qualquer coisa estou aqui.",
            "{label}"
        );
        assert_eq!(provedor.received().len(), 1, "{label}: o modelo responde");
        assert_eq!(scenario.metrics.closed_g8.value(), 0, "{label}: não fecha");
        assert_eq!(
            scenario.outputs().last().map(String::as_str),
            Some("Certo, qualquer coisa estou aqui."),
            "{label}"
        );
    }
}

#[tokio::test]
async fn g8_with_transfer_g1_transfer_wins() {
    let scenario = Scenario::new("g1-vence");
    scenario.reply_agent("Mais alguma dúvida?");
    let texto = "era só isso, tchau, mas quero falar com uma pessoa";
    let jev = judge_by_text(&[(
        texto,
        &[
            ("g1_transferir", "transferir_agora", 1.0),
            ("g8_encerrada", "encerrada", 1.0),
        ],
    )]);
    let gates = JevGates {
        g1: true,
        g8: true,
        ..JevGates::default()
    };
    let (engine, provedor) =
        scenario.engine(vec![script_text("não sai")], Some(engine_judge(jev, gates)));

    let gatilho = scenario.say(texto);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(
        scenario.status(),
        ConversationStatus::Transferred,
        "o G1 transfere"
    );
    assert_eq!(provedor.received().len(), 0);
    assert_eq!(
        scenario.metrics.closed_g8.value(),
        0,
        "não conta como encerramento"
    );
}

#[tokio::test]
async fn g8_closing_not_wait_search_progress() {
    let scenario = Scenario::with_delay("abandona", Duration::from_millis(300), "");
    scenario.reply_agent("Mais alguma dúvida?");
    let jev = judge_by_text(&[(ENDS, &[("g8_encerrada", "encerrada", 1.0)])]);
    let (engine, provedor) = scenario.engine(
        vec![script_text("não sai")],
        Some(engine_judge(jev, so_g8())),
    );

    let gatilho = scenario.say(ENDS);
    let beginning = Instant::now();
    let summary = scenario.turno(&engine, gatilho).await;
    let levou = beginning.elapsed();

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(provedor.received().len(), 0);
    assert!(
        levou < Duration::from_millis(250),
        "encerrando, o turno não pode esperar a busca de 300 ms: levou {levou:?}"
    );
}

#[tokio::test]
async fn default_installation_enables_g8_without_configure_agent() {
    let scenario = Scenario::new("padrao-g8");
    scenario.reply_agent("Mais alguma dúvida?");
    let jev = judge_by_text(&[
        (ENDS, &[("g8_encerrada", "encerrada", 1.0)]),
        (RETURNED, &[("g8_encerrada", "continua", 0.95)]),
    ]);
    let gates_default = poph_types::JevConfig::default().gates;
    assert!(gates_default.g8, "o padrão de instalação liga o G8");
    let (engine, provedor) = scenario.engine(
        vec![script_text("Claro! Qual o tamanho do imóvel?")],
        Some(engine_judge(jev, gates_default)),
    );

    let first = scenario.say(ENDS);
    let closed = scenario.turno(&engine, first).await;

    assert_eq!(closed.texto, G8_DEFAULT_FAREWELL);
    assert_eq!(
        provedor.received().len(),
        0,
        "sem configuração no agente, o encerramento não chama o modelo de texto"
    );
    assert_eq!(scenario.metrics.closed_g8.value(), 1);

    let second = scenario.say(RETURNED);
    let summary = scenario.turno(&engine, second).await;

    assert_eq!(summary.texto, "Claro! Qual o tamanho do imóvel?");
    assert_eq!(
        provedor.received().len(),
        1,
        "a mensagem seguinte reabre e volta ao modelo"
    );
}

#[tokio::test]
async fn agent_disables_g8_return_behaviour_today() {
    let scenario = Scenario::with_delay("agente-g8-off", Duration::ZERO, r#"{"g8": false}"#);
    scenario.reply_agent("Mais alguma dúvida?");
    let jev = judge_by_text(&[(ENDS, &[("g8_encerrada", "encerrada", 1.0)])]);
    let gates_default = poph_types::JevConfig::default().gates;
    let (engine, provedor) = scenario.engine(
        vec![script_text("Até mais, qualquer coisa estou aqui.")],
        Some(engine_judge(jev, gates_default)),
    );

    let gatilho = scenario.say(ENDS);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.texto, "Até mais, qualquer coisa estou aqui.");
    assert_eq!(
        provedor.received().len(),
        1,
        "o modelo responde, como antes"
    );
    assert_eq!(scenario.metrics.closed_g8.value(), 0);
    assert!(
        summary
            .jev
            .as_ref()
            .and_then(|jev| jev.choice(poph_jev::IdGate::G8))
            .is_none(),
        "com o G8 desligado no agente, o Jev não é perguntado sobre ele"
    );
}

fn characters_request(request: &poph_core::llm::Request) -> usize {
    request
        .messages
        .iter()
        .map(|mensagem| match mensagem {
            poph_core::llm::Message::System(texto) => texto.chars().count(),
            poph_core::llm::Message::User(parts) => parts
                .iter()
                .map(|part| match part {
                    poph_core::llm::Part::Texto(texto) => texto.chars().count(),
                    _ => 0,
                })
                .sum(),
            poph_core::llm::Message::Assistant { texto, .. } => texto.chars().count(),
            poph_core::llm::Message::Resultado { content, .. } => content.chars().count(),
        })
        .sum()
}

#[tokio::test]
#[ignore]
async fn measurement_closure() {
    let messages = [
        ENDS,
        "tchau, valeu",
        "ok, obrigado, era isso mesmo",
        "certo, muito obrigado, até mais",
    ];
    for linked in [false, true] {
        let mut calls = 0_usize;
        let mut characters = 0_usize;
        for (index, texto) in messages.iter().enumerate() {
            let scenario = Scenario::new(&format!("medicao-{linked}-{index}"));
            scenario.reply_agent("Mais alguma dúvida sobre a dedetização?");
            let jev = judge_by_text(&[(*texto, &[("g8_encerrada", "encerrada", 1.0)])]);
            let gates = if linked { so_g8() } else { JevGates::default() };
            let (engine, provedor) = scenario.engine(
                vec![script_text("Certo, qualquer coisa estou aqui.")],
                Some(engine_judge(jev, gates)),
            );
            let gatilho = scenario.say(texto);
            scenario.turno(&engine, gatilho).await;
            for request in provedor.received() {
                calls += 1;
                characters += characters_request(&request);
            }
        }
        let label = if linked { "G8 ligado" } else { "G8 desligado" };
        println!(
            "{label}: {} mensagens de encerramento → {calls} chamadas ao modelo de texto, {characters} caracteres de prompt",
            messages.len()
        );
    }
}
