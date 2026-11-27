use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::cart::format_brl;
use poph_agent::cart::{self, NAME_ADD, NAME_CLEAN, NAME_REMOVE, NAME_SUMMARY};
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::turno::{
    DefaultRegistry, FuturePause, NoopObserver, Pause, ToolContext, ToolRegistry, TurnEngine,
    TurnOptions, UseTools,
};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, LlmProvider, Script, ToolCall};
use poph_store::{NewAgent, NewCategory, NewConversation, NewOrg, NewProduct, Page, Store};
use poph_types::{
    AgentId, ConversationId, OrgId, ProductId, StructuredInstructions, ToolConfig, TurnId,
};
use serde_json::{Value, json};

#[derive(Default, Clone)]
struct FakePause;

impl Pause for FakePause {
    fn wait(&self, _duration: Duration) -> FuturePause {
        Box::pin(async {})
    }
}

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    conversa: ConversationId,
    products: Vec<ProductId>,
    jardim: ProductId,
}

impl Scenario {
    fn new(label: &str, tools: &[&str]) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-carrinho-{label}-{}-{}",
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
            tools: tools
                .iter()
                .map(|nome| ToolConfig {
                    name: (*nome).to_owned(),
                    enabled: true,
                    when_to_use: String::new(),
                })
                .collect(),
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
        let pragas = store
            .categories()
            .create(
                org,
                NewCategory {
                    name: "Pragas".to_owned(),
                    description: String::new(),
                },
            )
            .expect("cria a categoria")
            .id;
        let garden_category = store
            .categories()
            .create(
                org,
                NewCategory {
                    name: "Jardim".to_owned(),
                    description: String::new(),
                },
            )
            .expect("cria a categoria")
            .id;
        let mut products = Vec::new();
        for (nome, cents) in [
            ("Dedetização de baratas", 32_000),
            ("Desentupimento", 9_000),
            ("Visita técnica gratuita", 0),
            ("Projeto completo", 123_456),
        ] {
            let product = store
                .products()
                .create(
                    org,
                    NewProduct {
                        category_id: Some(pragas),
                        name: nome.to_owned(),
                        price_text: String::new(),
                        price_cents: cents,
                        unit: "por serviço".to_owned(),
                        ..NewProduct::default()
                    },
                )
                .expect("cria o produto");
            products.push(product.id);
        }
        let jardim = store
            .products()
            .create(
                org,
                NewProduct {
                    category_id: Some(garden_category),
                    name: "Poda de árvores".to_owned(),
                    price_cents: 15_000,
                    ..NewProduct::default()
                },
            )
            .expect("cria o produto de jardim")
            .id;
        let conversa = Self::open_conversation(&store, org, agente);
        Self {
            folder,
            store,
            org,
            agente,
            conversa,
            products,
            jardim,
        }
    }

    fn open_conversation(store: &Store, org: OrgId, agente: AgentId) -> ConversationId {
        store
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
            .id
    }

    fn context(&self, conversa: ConversationId) -> ToolContext {
        ToolContext {
            org: self.org,
            conversa,
            agente: self.agente,
            turno: TurnId::generate(),
            usage: UseTools::new(),
        }
    }

    async fn invoke(
        &self,
        conversa: ConversationId,
        nome: &str,
        arguments: Value,
    ) -> Result<String, String> {
        let registro = cart::register(DefaultRegistry::new(), Arc::clone(&self.store));
        let tool = registro.tool(nome).expect("ferramenta registrada");
        tool.run(self.context(conversa), arguments).await
    }

    fn total(&self, conversa: ConversationId) -> (i64, usize) {
        let summary = self
            .store
            .carts()
            .summary(self.org, conversa)
            .expect("lê o carrinho");
        (summary.total_cents, summary.items.len())
    }

    fn engine(&self, scripts: Vec<Script>) -> (TurnEngine, Arc<FakeProvider>) {
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
            TurnOptions::default(),
            MetricsTurn::own(),
        )
        .with_pause(Arc::new(FakePause));
        let registro = cart::register(DefaultRegistry::new(), Arc::clone(&self.store));
        let engine = engine.with_tools(Arc::new(registro) as Arc<dyn ToolRegistry>);
        (engine, fake)
    }

    fn say(&self, conversa: ConversationId, texto: &str) -> poph_types::MessageId {
        self.store
            .messages()
            .write(
                self.org,
                poph_store::NewMessage {
                    conversation_id: conversa,
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

fn call(nome: &str, id: &str, arguments: Value) -> Script {
    Script::Events(vec![
        LlmEvent::Tool(ToolCall {
            id: id.to_owned(),
            nome: nome.to_owned(),
            arguments,
        }),
        LlmEvent::Fim(FinishReason::ToolCalls),
    ])
}

enum Op {
    Add(usize, Option<u32>),
    Rem(usize, Option<u32>),
}

#[tokio::test]
async fn ten_cases_total_match_with_server() {
    let cases: Vec<(&str, Vec<Op>, i64, usize)> = vec![
        (
            "um item, quantidade padrão",
            vec![Op::Add(0, None)],
            32_000,
            1,
        ),
        (
            "quantidade maior que 1",
            vec![Op::Add(0, Some(3))],
            96_000,
            1,
        ),
        ("preço zero", vec![Op::Add(2, Some(2))], 0, 1),
        (
            "produto repetido soma na mesma linha",
            vec![Op::Add(0, Some(1)), Op::Add(0, Some(2))],
            96_000,
            1,
        ),
        (
            "dois produtos diferentes",
            vec![Op::Add(0, Some(2)), Op::Add(1, Some(1))],
            73_000,
            2,
        ),
        (
            "remove parte da quantidade",
            vec![Op::Add(0, Some(3)), Op::Rem(0, Some(1))],
            64_000,
            1,
        ),
        (
            "remove a linha inteira",
            vec![Op::Add(0, Some(3)), Op::Rem(0, None)],
            0,
            0,
        ),
        (
            "item removido e outro fica",
            vec![Op::Add(0, None), Op::Add(1, None), Op::Rem(0, None)],
            9_000,
            1,
        ),
        (
            "valor alto com milhar",
            vec![Op::Add(3, Some(2))],
            246_912,
            1,
        ),
        ("carrinho vazio", vec![], 0, 0),
    ];
    assert_eq!(cases.len(), 10);

    let scenario = Scenario::new("totais", &[NAME_ADD, NAME_REMOVE, NAME_SUMMARY]);
    for (nome, operations, total_expected, lines_expected) in cases {
        let conversa = Scenario::open_conversation(&scenario.store, scenario.org, scenario.agente);
        for operation in operations {
            let (name_tool, arguments) = match operation {
                Op::Add(index, quantity) => (
                    NAME_ADD,
                    product_quantity(scenario.products[index], quantity),
                ),
                Op::Rem(index, quantity) => (
                    NAME_REMOVE,
                    product_quantity(scenario.products[index], quantity),
                ),
            };
            scenario
                .invoke(conversa, name_tool, arguments)
                .await
                .unwrap_or_else(|error| panic!("{nome}: a ferramenta recusou: {error}"));
        }
        let (total, lines) = scenario.total(conversa);
        assert_eq!(total, total_expected, "caso: {nome} (total)");
        assert_eq!(lines, lines_expected, "caso: {nome} (linhas)");

        let texto = scenario
            .invoke(conversa, NAME_SUMMARY, json!({}))
            .await
            .expect("resumo");
        assert!(
            texto.contains(&format!("Total: {}", format_brl(total_expected))),
            "caso: {nome}: texto sem o total certo: {texto}"
        );
    }
}

fn product_quantity(product: ProductId, quantity: Option<u32>) -> Value {
    match quantity {
        Some(quantity) => json!({"produto_id": product.to_string(), "quantidade": quantity}),
        None => json!({"produto_id": product.to_string()}),
    }
}

#[tokio::test]
async fn tool_refusal_price_not_records_nothing() {
    let scenario = Scenario::new("recusa", &[NAME_ADD]);
    let conversa = scenario.conversa;
    let product = scenario.products[0].to_string();

    for arguments in [
        json!({"produto_id": product, "preco_centavos": 1}),
        json!({"produto_id": product, "quantidade": 1, "price": 1}),
        json!({"produto_id": product, "total": 1}),
    ] {
        let error = scenario
            .invoke(conversa, NAME_ADD, arguments)
            .await
            .expect_err("preço não pode entrar");
        assert!(error.contains("não é aceito"), "{error}");
        assert!(error.contains("catálogo"), "{error}");
    }
    assert_eq!(scenario.total(conversa), (0, 0), "nada foi gravado");
}

#[test]
fn schema_tool_has_id_quantity() {
    let registro = cart::register(DefaultRegistry::new(), Arc::new(store_empty()));
    let tool = registro.tool(NAME_ADD).expect("registrada");
    let properties: Vec<String> = tool.parameters()["properties"]
        .as_object()
        .expect("objeto")
        .keys()
        .cloned()
        .collect();
    assert_eq!(properties.len(), 2);
    assert!(properties.contains(&"produto_id".to_owned()));
    assert!(properties.contains(&"quantidade".to_owned()));
    assert_eq!(tool.parameters()["additionalProperties"], json!(false));
}

fn store_empty() -> Store {
    let folder = std::env::temp_dir().join(format!("poph-carrinho-esquema-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("pasta");
    Store::open_at(&folder.join("harness.db"), &folder.join("rag.db")).expect("banco")
}

#[tokio::test]
async fn llm_sends_price_turn_total_continues_catalog() {
    let scenario = Scenario::new("turno-preco", &[NAME_ADD, NAME_SUMMARY]);
    let product = scenario.products[0].to_string();
    let (engine, fake) = scenario.engine(vec![
        call(
            NAME_ADD,
            "c1",
            json!({"produto_id": product, "quantidade": 1, "preco_centavos": 1}),
        ),
        call(
            NAME_ADD,
            "c2",
            json!({"produto_id": product, "quantidade": 1}),
        ),
        response("Anotado, a dedetização está no orçamento."),
    ]);
    let mensagem = scenario.say(scenario.conversa, "quero a dedetização");
    let summary = engine
        .run(
            poph_agent::turno::TurnInput::new(scenario.org, scenario.conversa, mensagem),
            &mut NoopObserver,
        )
        .await
        .expect("o turno roda");
    assert_eq!(summary.estado.as_text(), "done");

    let requests = fake.received();
    let resultado = requests[1]
        .messages
        .iter()
        .find_map(|mensagem| match mensagem {
            poph_core::llm::Message::Resultado { id, content } if id == "c1" => {
                Some(content.clone())
            }
            _ => None,
        })
        .expect("resultado da primeira chamada");
    assert!(resultado.contains("não é aceito"), "{resultado}");

    assert_eq!(scenario.total(scenario.conversa), (32_000, 1));
}

#[tokio::test]
async fn agent_without_tool_not_receives() {
    let scenario = Scenario::new("habilitacao", &[NAME_SUMMARY]);
    let (engine, fake) = scenario.engine(vec![response("Seu carrinho está vazio.")]);
    let mensagem = scenario.say(scenario.conversa, "quanto deu até agora?");
    engine
        .run(
            poph_agent::turno::TurnInput::new(scenario.org, scenario.conversa, mensagem),
            &mut NoopObserver,
        )
        .await
        .expect("o turno roda");
    let names: Vec<String> = fake.received()[0]
        .tools
        .iter()
        .map(|tool| tool.nome.clone())
        .collect();
    assert_eq!(names, vec![NAME_SUMMARY.to_owned()]);
}

#[tokio::test]
async fn agent_with_four_receives_four() {
    let scenario = Scenario::new("quatro", &[NAME_ADD, NAME_REMOVE, NAME_SUMMARY, NAME_CLEAN]);
    let (engine, fake) = scenario.engine(vec![response("ok")]);
    let mensagem = scenario.say(scenario.conversa, "oi");
    engine
        .run(
            poph_agent::turno::TurnInput::new(scenario.org, scenario.conversa, mensagem),
            &mut NoopObserver,
        )
        .await
        .expect("o turno roda");
    let mut names: Vec<String> = fake.received()[0]
        .tools
        .iter()
        .map(|tool| tool.nome.clone())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![NAME_ADD, NAME_CLEAN, NAME_REMOVE, NAME_SUMMARY]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn cart_by_conversation_conversation_new_starts_empty() {
    let scenario = Scenario::new("conversas", &[NAME_ADD, NAME_SUMMARY]);
    let first = scenario.conversa;
    scenario
        .invoke(
            first,
            NAME_ADD,
            product_quantity(scenario.products[1], Some(2)),
        )
        .await
        .expect("adiciona");
    assert_eq!(scenario.total(first), (18_000, 1));

    let new = Scenario::open_conversation(&scenario.store, scenario.org, scenario.agente);
    assert_eq!(scenario.total(new), (0, 0), "conversa nova começa vazia");
    let texto = scenario
        .invoke(new, NAME_SUMMARY, json!({}))
        .await
        .expect("resumo");
    assert!(texto.contains("vazio"), "{texto}");
    assert_eq!(
        scenario.total(first),
        (18_000, 1),
        "a antiga continua com o item"
    );
}

#[tokio::test]
async fn org_other_token_not_touches_cart() {
    let scenario = Scenario::new("isolamento", &[NAME_ADD]);
    let conversa = scenario.conversa;
    scenario
        .invoke(
            conversa,
            NAME_ADD,
            product_quantity(scenario.products[0], None),
        )
        .await
        .expect("adiciona");

    let other = scenario
        .store
        .orgs()
        .create(NewOrg {
            name: "Outra".to_owned(),
            slug: format!("outra-{}", AgentId::generate()),
            config_json: "{}".to_owned(),
        })
        .expect("cria a outra org")
        .id;
    let summary = scenario.store.carts().summary(other, conversa).expect("lê");
    assert!(summary.items.is_empty());
    assert!(summary.cart.is_none());
    let error = scenario
        .store
        .carts()
        .add(other, conversa, scenario.agente, scenario.products[1], 1)
        .expect_err("org alheia não adiciona");
    assert!(error.is_not_found(), "{error}");
    assert_eq!(
        scenario.total(conversa),
        (32_000, 1),
        "o carrinho da dona não mudou"
    );
}

#[tokio::test]
async fn product_out_categories_agent_not_enters() {
    let scenario = Scenario::new("visibilidade", &[NAME_ADD]);
    let categories = scenario
        .store
        .categories()
        .list(scenario.org, Page::default())
        .expect("lista")
        .into_iter()
        .filter(|category| category.name == "Pragas")
        .map(|category| category.id)
        .collect::<Vec<_>>();
    let agente = scenario
        .store
        .agents()
        .get(scenario.org, scenario.agente)
        .expect("agente");
    scenario
        .store
        .agents()
        .define_categories(scenario.org, scenario.agente, agente.version, &categories)
        .expect("vincula o agente a Pragas");

    let error = scenario
        .invoke(
            scenario.conversa,
            NAME_ADD,
            product_quantity(scenario.jardim, Some(1)),
        )
        .await
        .expect_err("produto de Jardim não é visível");
    assert!(error.contains("não está disponível"), "{error}");
    assert_eq!(scenario.total(scenario.conversa), (0, 0));

    scenario
        .invoke(
            scenario.conversa,
            NAME_ADD,
            product_quantity(scenario.products[0], Some(1)),
        )
        .await
        .expect("produto de Pragas entra");
    assert_eq!(scenario.total(scenario.conversa), (32_000, 1));
}

#[tokio::test]
async fn remove_not_this_cart_failure_without_change_nothing() {
    let scenario = Scenario::new("remover", &[NAME_REMOVE]);
    scenario
        .invoke(
            scenario.conversa,
            NAME_ADD,
            product_quantity(scenario.products[0], Some(2)),
        )
        .await
        .expect("adiciona");
    let error = scenario
        .invoke(
            scenario.conversa,
            NAME_REMOVE,
            product_quantity(scenario.products[1], None),
        )
        .await
        .expect_err("não estava no carrinho");
    assert!(error.contains("não está no carrinho"), "{error}");
    assert_eq!(scenario.total(scenario.conversa), (64_000, 1));
}

#[tokio::test]
async fn quantity_accumulated_above_999_refused() {
    let scenario = Scenario::new("limite", &[NAME_ADD]);
    scenario
        .invoke(
            scenario.conversa,
            NAME_ADD,
            product_quantity(scenario.products[2], Some(999)),
        )
        .await
        .expect("999 cabem");
    let error = scenario
        .invoke(
            scenario.conversa,
            NAME_ADD,
            product_quantity(scenario.products[2], Some(1)),
        )
        .await
        .expect_err("passa de 999");
    assert!(error.contains("999"), "{error}");
    assert!(
        scenario
            .invoke(
                scenario.conversa,
                NAME_ADD,
                json!({"produto_id": scenario.products[0].to_string(), "quantidade": 0}),
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn clean_empties_counts_lines() {
    let scenario = Scenario::new("limpar", &[NAME_CLEAN]);
    for index in [0, 1, 3] {
        scenario
            .store
            .carts()
            .add(
                scenario.org,
                scenario.conversa,
                scenario.agente,
                scenario.products[index],
                1,
            )
            .expect("adiciona");
    }
    let texto = scenario
        .invoke(scenario.conversa, NAME_CLEAN, json!({}))
        .await
        .expect("limpa");
    assert!(texto.contains("3 item(ns) removido(s)"), "{texto}");
    assert_eq!(scenario.total(scenario.conversa), (0, 0));
    scenario
        .store
        .carts()
        .add(
            scenario.org,
            scenario.conversa,
            scenario.agente,
            scenario.products[0],
            1,
        )
        .expect("volta a adicionar");
    let summary = scenario
        .store
        .carts()
        .summary(scenario.org, scenario.conversa)
        .expect("lê");
    assert!(summary.cart.is_some(), "o carrinho aberto continua o mesmo");
}
