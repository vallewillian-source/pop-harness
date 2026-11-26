use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::turno::{FuturePause, NoopObserver, Pause, TurnEngine, TurnOptions};
use poph_agent::{ProductCatalog, ProductSearcher, SearchProduct};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, LlmProvider, Script, ToolCall};
use poph_rag::{Embedder, FakeEmbedder, Index, IndexConfig};
use poph_store::{
    NewAgent, NewCategory, NewConversation, NewMessage, NewOrg, NewProduct, Page, Store,
};
use poph_types::{AgentId, ConversationId, OrgId, StructuredInstructions, ToolConfig};

#[derive(Default, Clone)]
struct PausaFalsa;

impl Pause for PausaFalsa {
    fn wait(&self, _duracao: Duration) -> FuturePause {
        Box::pin(async {})
    }
}

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    conversa: ConversationId,
    catalog: Arc<ProductCatalog>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-turno-produto-{label}-{}-{}",
            std::process::id(),
            AgentId::generate()
        ));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("cria a pasta");
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
            tools: vec![ToolConfig {
                name: "buscar_produto".to_owned(),
                enabled: true,
                when_to_use: String::new(),
            }],
            ..StructuredInstructions::default()
        };
        store
            .instructions()
            .save(
                org,
                agente,
                0,
                &structured.to_json(),
                "# Bia\n\nVocê é a atendente da FixProLar. Nunca invente preço.",
                true,
            )
            .expect("grava as instruções");
        let category = store
            .categories()
            .create(
                org,
                NewCategory {
                    name: "Pragas urbanas".to_owned(),
                    description: String::new(),
                },
            )
            .expect("cria a categoria")
            .id;
        store
            .products()
            .create(
                org,
                NewProduct {
                    category_id: Some(category),
                    name: "Dedetização de baratas".to_owned(),
                    aliases: vec!["baratas".to_owned(), "barata".to_owned()],
                    description: "Tratamento com gel e pulverização para eliminar baratas."
                        .to_owned(),
                    price_text: "R$ 320,00".to_owned(),
                    price_cents: 32_000,
                    unit: "por visita".to_owned(),
                    conditions: "Pagamento em até 3x".to_owned(),
                    ..NewProduct::default()
                },
            )
            .expect("cria o produto");
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new());
        let (index, _) =
            Index::open(store.rag_path(), embedder, IndexConfig::default()).expect("abre o índice");
        let catalog = ProductCatalog::new(Arc::clone(&store), Arc::new(index));
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
            catalog,
        }
    }

    fn engine(&self, roteiros: Vec<Script>) -> (TurnEngine, Arc<FakeProvider>) {
        let fake = Arc::new(FakeProvider::with_scripts(roteiros));
        let provedor: Arc<dyn LlmProvider> = fake.clone();
        let config = ConfigHot::new(
            Arc::clone(&self.store),
            HotConfigOptions::default(),
            MetricsMemory::own(),
        );
        let limiter = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
        let catalog: Arc<dyn ProductSearcher> =
            Arc::clone(&self.catalog) as Arc<dyn ProductSearcher>;
        let engine = TurnEngine::new(
            Arc::clone(&self.store),
            config,
            limiter,
            provedor,
            TurnOptions::default(),
            MetricsTurn::own(),
        )
        .with_pause(Arc::new(PausaFalsa));
        let registro = SearchProduct::registro(Arc::clone(engine.hot_config()), catalog);
        let engine = engine.with_tools(Arc::new(registro));
        (engine, fake)
    }

    fn falar(&self, texto: &str) -> poph_types::MessageId {
        self.store
            .messages()
            .write(
                self.org,
                NewMessage {
                    conversation_id: self.conversa,
                    direction: poph_store::Direction::Inbound,
                    role: poph_store::Role::User,
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

    fn turnos(&self) -> Vec<poph_store::Turno> {
        self.store
            .turns()
            .list(self.org, self.conversa, Page::default())
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
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

#[tokio::test]
async fn turno_grava_o_produto_usado_com_o_preco_do_registro() {
    let scenario = Scenario::new("registro");
    scenario
        .catalog
        .synchronise_agent(scenario.org, scenario.agente)
        .await
        .expect("sincroniza o índice");

    let call = Script::Events(vec![
        LlmEvent::Tool(ToolCall {
            id: "c1".to_owned(),
            nome: "buscar_produto".to_owned(),
            arguments: serde_json::json!({"termo": "baratas"}),
        }),
        LlmEvent::Fim(FinishReason::ToolCalls),
    ]);
    let (engine, fake) = scenario.engine(vec![call, response("A dedetização custa R$ 320,00.")]);

    let mensagem = scenario.falar("quanto custa a dedetização de baratas?");
    let summary = engine
        .run(
            poph_agent::turno::TurnInput::new(scenario.org, scenario.conversa, mensagem),
            &mut NoopObserver,
        )
        .await
        .expect("o turno roda");
    assert_eq!(summary.estado.as_text(), "done");
    assert_eq!(summary.texto, "A dedetização custa R$ 320,00.");

    let pedidos = fake.received();
    assert_eq!(
        pedidos.len(),
        2,
        "uma rodada de ferramenta e o fecho em texto"
    );
    let names: Vec<String> = pedidos[0]
        .tools
        .iter()
        .map(|tool| tool.nome.clone())
        .collect();
    assert_eq!(names, vec!["buscar_produto".to_owned()]);

    let turnos = scenario.turnos();
    assert_eq!(turnos.len(), 1);
    let registrado: serde_json::Value =
        serde_json::from_str(&turnos[0].passages_json).expect("JSON do registro");
    let items = registrado.as_array().expect("lista");
    let product = items
        .iter()
        .find(|item| item["tipo"] == "produto")
        .expect("o produto usado está no registro");
    assert_eq!(product["nome"], "Dedetização de baratas");
    assert_eq!(product["preco_centavos"], 32_000);
    assert_eq!(product["preco_texto"], "R$ 320,00");
    assert_eq!(product["unidade"], "por visita");
}

#[tokio::test]
async fn turno_sem_produto_mantem_o_registro_vazio() {
    let scenario = Scenario::new("vazio");
    let (engine, _) = scenario.engine(vec![response("Olá! Como posso ajudar?")]);
    let mensagem = scenario.falar("oi");
    engine
        .run(
            poph_agent::turno::TurnInput::new(scenario.org, scenario.conversa, mensagem),
            &mut NoopObserver,
        )
        .await
        .expect("o turno roda");
    let turnos = scenario.turnos();
    assert_eq!(turnos[0].passages_json, "[]");
}
