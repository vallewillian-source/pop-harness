use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::turno::{
    CAP_SUMMARY_TOOL, DefaultRegistry, FuturePause, InternalTool, MARK_OF_CUTOFF, NoopObserver,
    Pause, REASON_NOT_ENABLED, REASON_UNKNOWN, ToolContext, ToolFuture, TurnEngine, TurnOptions,
};
use poph_core::llm::{
    FakeProvider, FinishReason, LlmEvent, LlmProvider, Message, Script, ToolCall,
};
use poph_store::{NewAgent, NewConversation, NewMessage, NewOrg, Page, Store, Turno};
use poph_types::{AgentId, ConversationId, OrgId, StructuredInstructions, ToolConfig};

#[derive(Default, Clone)]
struct FakePause;

impl Pause for FakePause {
    fn wait(&self, _duration: Duration) -> FuturePause {
        Box::pin(async {})
    }
}

#[derive(Clone)]
enum Behavior {
    Texto(String),
    WithDelay(String, u64),
    Error(String),
}

struct FakeTool {
    nome: &'static str,
    behaviour: Behavior,
}

impl InternalTool for FakeTool {
    fn nome(&self) -> &'static str {
        self.nome
    }

    fn descricao(&self) -> &'static str {
        "ferramenta de teste"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {"consulta": {"type": "string"}},
            "required": ["consulta"]
        })
    }

    fn run(&self, _context: ToolContext, _arguments: serde_json::Value) -> ToolFuture {
        let behaviour = self.behaviour.clone();
        Box::pin(async move {
            match behaviour {
                Behavior::Texto(texto) => Ok(texto),
                Behavior::WithDelay(texto, ms) => {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                    Ok(texto)
                }
                Behavior::Error(error) => Err(error),
            }
        })
    }
}

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    conversa: ConversationId,
}

impl Scenario {
    fn new(label: &str, enabled: &[&str]) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-registro-ferramentas-{label}-{}-{}",
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
                    name: format!("Atendente {label}"),
                    slug: format!("atendente-{label}"),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        let structured = StructuredInstructions {
            tools: enabled
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
            conversa,
        }
    }

    fn engine(
        &self,
        scripts: Vec<Script>,
        registro: DefaultRegistry,
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
            TurnOptions::default(),
            MetricsTurn::own(),
        )
        .with_pause(Arc::new(FakePause))
        .with_tools(Arc::new(registro));
        (engine, fake)
    }

    fn say(&self, texto: &str) -> poph_types::MessageId {
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

    fn turno(&self) -> Turno {
        let turns = self
            .store
            .turns()
            .list(self.org, self.conversa, Page::default())
            .expect("lista os turnos");
        assert_eq!(turns.len(), 1, "um turno por mensagem");
        turns.into_iter().next().expect("o turno")
    }

    async fn execute(&self, engine: &TurnEngine, speaks: &str) -> serde_json::Value {
        let mensagem = self.say(speaks);
        engine
            .run(
                poph_agent::turno::TurnInput::new(self.org, self.conversa, mensagem),
                &mut NoopObserver,
            )
            .await
            .expect("o turno roda");
        let turno = self.turno();
        serde_json::from_str(&turno.tools_json).expect("JSON do registro de ferramentas")
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

fn call(nome: &str, arguments: serde_json::Value) -> Script {
    Script::Events(vec![
        LlmEvent::Tool(ToolCall {
            id: "c1".to_owned(),
            nome: nome.to_owned(),
            arguments,
        }),
        LlmEvent::Fim(FinishReason::ToolCalls),
    ])
}

fn record_query_base(behaviour: Behavior) -> DefaultRegistry {
    DefaultRegistry::new().com(Arc::new(FakeTool {
        nome: "consultar_base",
        behaviour,
    }))
}

fn record_query_base_cart() -> DefaultRegistry {
    DefaultRegistry::new()
        .com(Arc::new(FakeTool {
            nome: "consultar_base",
            behaviour: Behavior::Texto("ok".to_owned()),
        }))
        .com(Arc::new(FakeTool {
            nome: "cart_add_item",
            behaviour: Behavior::Texto("ok".to_owned()),
        }))
}

#[tokio::test]
async fn call_executed_records_arguments_result_duration_round() {
    let scenario = Scenario::new("executada", &["consultar_base"]);
    let (engine, _) = scenario.engine(
        vec![
            call(
                "consultar_base",
                serde_json::json!({"consulta": "prazo de entrega"}),
            ),
            response("O prazo é de cinco dias úteis."),
        ],
        record_query_base(Behavior::WithDelay("Prazo de 5 dias úteis.".to_owned(), 20)),
    );

    let registro = scenario.execute(&engine, "qual o prazo?").await;
    assert_eq!(registro["nomes"], serde_json::json!(["consultar_base"]));
    let calls = registro["chamadas"].as_array().expect("lista de chamadas");
    assert_eq!(calls.len(), 1);
    let item = &calls[0];
    assert_eq!(item["nome"], "consultar_base");
    assert_eq!(item["estado"], "executada");
    assert!(item["motivo"].is_null(), "executada não tem motivo");
    assert_eq!(item["rodada"], 0);
    assert_eq!(item["ordem"], 1);
    assert!(
        item["duracao_ms"].as_u64().expect("duração") >= 5,
        "a duração mede a execução: {item}"
    );
    assert!(
        item["argumentos"]
            .as_str()
            .expect("argumentos")
            .contains("prazo de entrega")
    );
    assert_eq!(item["resultado"], "Prazo de 5 dias úteis.");
}

#[tokio::test]
async fn tool_not_enabled_records_refusal_model_receives_error() {
    let scenario = Scenario::new("nao-habilitada", &["consultar_base"]);
    let (engine, fake) = scenario.engine(
        vec![
            call(
                "cart_add_item",
                serde_json::json!({"produto_id": "qualquer", "quantidade": 1}),
            ),
            response("Não consigo montar o carrinho."),
        ],
        record_query_base_cart(),
    );

    let registro = scenario.execute(&engine, "põe no carrinho").await;
    let calls = registro["chamadas"].as_array().expect("lista de chamadas");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["estado"], "recusada");
    assert_eq!(calls[0]["motivo"], REASON_NOT_ENABLED);
    assert_eq!(calls[0]["duracao_ms"], 0);
    assert_eq!(registro["nomes"], serde_json::json!(["cart_add_item"]));

    let requests = fake.received();
    assert_eq!(requests.len(), 2);
    let error = requests[1]
        .messages
        .iter()
        .find_map(|mensagem| match mensagem {
            Message::Resultado { content, .. } => Some(content.clone()),
            _ => None,
        })
        .expect("o resultado da chamada vai para o modelo");
    assert!(
        error.contains("não está habilitada"),
        "erro ao modelo: {error}"
    );
}

#[tokio::test]
async fn tool_unknown_records_refusal_with_reason_own() {
    let scenario = Scenario::new("desconhecida", &["cart_add_item"]);
    let (engine, _) = scenario.engine(
        vec![
            call("cart_add_item", serde_json::json!({"produto_id": "x"})),
            response("Sigo sem o carrinho."),
        ],
        record_query_base(Behavior::Texto("ok".to_owned())),
    );

    let registro = scenario.execute(&engine, "põe no carrinho").await;
    let calls = registro["chamadas"].as_array().expect("lista de chamadas");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["estado"], "recusada");
    assert_eq!(calls[0]["motivo"], REASON_UNKNOWN);
}

#[tokio::test]
async fn error_argument_tool_becomes_reason_argument_invalid() {
    let scenario = Scenario::new("argumento", &["consultar_base"]);
    let (engine, _) = scenario.engine(
        vec![
            call("consultar_base", serde_json::json!({})),
            response("Não encontrei."),
        ],
        record_query_base(Behavior::Error(
            "erro: o argumento `consulta` é obrigatório".to_owned(),
        )),
    );

    let registro = scenario.execute(&engine, "consulta").await;
    let calls = registro["chamadas"].as_array().expect("lista de chamadas");
    assert_eq!(calls[0]["estado"], "recusada");
    assert_eq!(calls[0]["motivo"], "argumento_invalido");
    assert!(
        calls[0]["resultado"]
            .as_str()
            .expect("resultado")
            .contains("obrigatório")
    );
}

#[tokio::test]
async fn arguments_result_respect_cap_size() {
    let scenario = Scenario::new("teto", &["consultar_base"]);
    let argument_large = "consulta".repeat(500);
    let result_large = "retorno".repeat(500);
    let (engine, _) = scenario.engine(
        vec![
            call(
                "consultar_base",
                serde_json::json!({"consulta": argument_large}),
            ),
            response("Certo."),
        ],
        record_query_base(Behavior::Texto(result_large)),
    );

    let registro = scenario.execute(&engine, "consulta grande").await;
    let calls = registro["chamadas"].as_array().expect("lista de chamadas");
    let arguments = calls[0]["argumentos"].as_str().expect("argumentos");
    let resultado = calls[0]["resultado"].as_str().expect("resultado");
    assert!(arguments.ends_with(MARK_OF_CUTOFF), "argumentos cortados");
    assert!(resultado.ends_with(MARK_OF_CUTOFF), "resultado cortado");
    assert_eq!(
        arguments.chars().count(),
        CAP_SUMMARY_TOOL + MARK_OF_CUTOFF.chars().count()
    );
    assert_eq!(
        resultado.chars().count(),
        CAP_SUMMARY_TOOL + MARK_OF_CUTOFF.chars().count()
    );
    assert!(!registro.to_string().contains(&"consulta".repeat(500)));
}

#[tokio::test]
async fn argument_with_name_secret_not_goes_for_record() {
    let scenario = Scenario::new("segredo", &["consultar_base"]);
    let (engine, _) = scenario.engine(
        vec![
            call(
                "consultar_base",
                serde_json::json!({
                    "consulta": "prazo",
                    "token": "segredo-que-nao-pode-vazar",
                    "api_key": "chave-que-nao-pode-vazar",
                }),
            ),
            response("Prazo de 5 dias."),
        ],
        record_query_base(Behavior::Texto("ok".to_owned())),
    );

    let registro = scenario.execute(&engine, "qual o prazo?").await;
    let texto = registro.to_string();
    assert!(texto.contains("prazo"));
    assert!(texto.contains("[redigido]"));
    for forbidden in ["segredo-que-nao-pode-vazar", "chave-que-nao-pode-vazar"] {
        assert!(!texto.contains(forbidden), "vazou `{forbidden}`: {texto}");
    }
}

#[tokio::test]
async fn turn_without_tool_records_list_calls_empty() {
    let scenario = Scenario::new("sem-ferramenta", &["consultar_base"]);
    let (engine, _) = scenario.engine(
        vec![response("Olá! Como posso ajudar?")],
        record_query_base(Behavior::Texto("ok".to_owned())),
    );

    let registro = scenario.execute(&engine, "oi").await;
    assert_eq!(registro["nomes"], serde_json::json!([]));
    assert_eq!(registro["chamadas"], serde_json::json!([]));
}
