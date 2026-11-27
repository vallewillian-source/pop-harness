use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::actors::{ActorOptions, Actors};
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::entry_output::{FlowOptions, Fluxo, ReceivedMessage};
use poph_agent::library::{self, NAME_TOOL};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsFlow, MetricsMemory, MetricsTurn};
use poph_agent::turno::{
    DefaultRegistry, FuturePause, InternalTool, NoopObserver, Pause, ToolContext, ToolRegistry,
    TurnEngine, TurnOptions, TurnProcessor, UseTools,
};
use poph_core::llm::{
    FakeProvider, FinishReason, LlmError, LlmEvent, LlmProvider, Script, ToolCall,
};
use poph_store::{
    LibraryFile, NewAgent, NewConversation, NewLibraryFile, NewOrg, OutboxItem, Store,
};
use poph_types::{
    AgentId, ConversationId, FileId, OrgId, StructuredInstructions, ToolConfig, TurnId,
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
    other_agent: AgentId,
    conversa: ConversationId,
    catalog: FileId,
    proposta: FileId,
}

impl Scenario {
    fn new(label: &str, tools: &[&str]) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-biblioteca-{label}-{}-{}",
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
        let agente = Self::create_agent(&store, org, label, tools);
        let other_agent = Self::create_agent(&store, org, &format!("{label}-2"), &[]);
        let catalog = Self::file(&store, org, agente, "catalogo.pdf", "catálogo de serviços");
        let proposta = Self::file(&store, org, agente, "proposta.pdf", "modelo de proposta");
        let conversa = Self::open_conversation(&store, org, agente);
        Self {
            folder,
            store,
            org,
            agente,
            other_agent,
            conversa,
            catalog,
            proposta,
        }
    }

    fn create_agent(store: &Store, org: OrgId, label: &str, tools: &[&str]) -> AgentId {
        let suffix = AgentId::generate().to_string();
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: format!("Bia {label}"),
                    slug: format!("bia-{label}-{}", &suffix[..8]),
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
        agente
    }

    fn file(store: &Store, org: OrgId, agente: AgentId, nome: &str, descricao: &str) -> FileId {
        store
            .library()
            .create(
                org,
                NewLibraryFile {
                    agent_id: agente,
                    name: nome.to_owned(),
                    description: descricao.to_owned(),
                    mime: "application/pdf".to_owned(),
                    size_bytes: 4_096,
                    sha256: "b".repeat(64),
                    blob_path: format!("{org}/{nome}"),
                },
            )
            .expect("põe o arquivo na biblioteca")
            .id
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

    fn context(&self, turno: TurnId) -> ToolContext {
        ToolContext {
            org: self.org,
            conversa: self.conversa,
            agente: self.agente,
            turno,
            usage: UseTools::new(),
        }
    }

    fn registro(&self) -> DefaultRegistry {
        library::register(DefaultRegistry::new(), Arc::clone(&self.store))
    }

    fn tool(&self) -> Arc<dyn InternalTool> {
        self.registro()
            .tool(NAME_TOOL)
            .expect("ferramenta registrada")
    }

    async fn invoke(&self, turno: TurnId, arguments: Value) -> Result<String, String> {
        self.tool().run(self.context(turno), arguments).await
    }

    fn outputs(&self, turno: TurnId) -> Vec<OutboxItem> {
        self.store
            .outbox()
            .list_pending(self.org, 50)
            .expect("lê a outbox")
            .into_iter()
            .filter(|item| item.turn_id == turno)
            .collect()
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
        let engine = engine.with_tools(Arc::new(self.registro()) as Arc<dyn ToolRegistry>);
        (engine, fake)
    }

    fn say(&self, texto: &str) -> poph_types::MessageId {
        self.store
            .messages()
            .write(
                self.org,
                poph_store::NewMessage {
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

    async fn run_turn(&self, engine: &TurnEngine) -> poph_agent::turno::TurnSummary {
        let mensagem = self.say("manda o catálogo por favor");
        engine
            .run(
                poph_agent::turno::TurnInput::new(self.org, self.conversa, mensagem),
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

fn response(texto: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(texto.to_owned()),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

fn call(id: &str, arguments: Value) -> Script {
    Script::Events(vec![
        LlmEvent::Tool(ToolCall {
            id: id.to_owned(),
            nome: NAME_TOOL.to_owned(),
            arguments,
        }),
        LlmEvent::Fim(FinishReason::ToolCalls),
    ])
}

fn file_item(item: &OutboxItem) -> Option<Value> {
    serde_json::from_str(&item.content_json).ok()
}

#[tokio::test]
async fn id_other_agent_same_org_refused() {
    let scenario = Scenario::new("outro-agente", &[NAME_TOOL]);
    let foreign = Scenario::file(
        &scenario.store,
        scenario.org,
        scenario.other_agent,
        "interno.pdf",
        "documento interno",
    );
    let turno = TurnId::generate();
    let error = scenario
        .invoke(turno, json!({"id": foreign.to_string()}))
        .await
        .expect_err("id de outro agente é recusado");
    assert!(error.contains("não existe arquivo"), "{error}");
    assert!(scenario.outputs(turno).is_empty(), "nada foi enfileirado");
}

#[tokio::test]
async fn id_other_org_refused_same_with_org_forged() {
    let scenario = Scenario::new("outra-org", &[NAME_TOOL]);
    let other_org = scenario
        .store
        .orgs()
        .create(NewOrg {
            name: "Outra".to_owned(),
            slug: format!("outra-{}", AgentId::generate()),
            config_json: "{}".to_owned(),
        })
        .expect("cria a outra org");
    let agent_foreign = Scenario::create_agent(&scenario.store, other_org.id, "fora", &[]);
    let file_foreign = Scenario::file(
        &scenario.store,
        other_org.id,
        agent_foreign,
        "segredo.pdf",
        "não é desta org",
    );

    let error = scenario
        .invoke(TurnId::generate(), json!({"id": file_foreign.to_string()}))
        .await
        .expect_err("arquivo de outra org é recusado");
    assert!(error.contains("não existe arquivo"), "{error}");

    let turno = TurnId::generate();
    let tool = scenario.tool();
    let mut context = scenario.context(turno);
    context.org = other_org.id;
    let error = tool
        .run(context, json!({"id": scenario.catalog.to_string()}))
        .await
        .expect_err("id do próprio agente não vale com a org trocada");
    assert!(error.contains("não existe arquivo"), "{error}");
    assert!(scenario.outputs(turno).is_empty());
}

#[tokio::test]
async fn id_missing_returns_error_readable_turn_follows() {
    let scenario = Scenario::new("inexistente", &[NAME_TOOL]);
    let error = scenario
        .invoke(
            TurnId::generate(),
            json!({"id": FileId::generate().to_string()}),
        )
        .await
        .expect_err("id inexistente é recusado");
    assert!(error.starts_with("erro:"), "{error}");
    assert!(error.contains("lista de arquivos"), "{error}");

    let (engine, _fake) = scenario.engine(vec![
        call("c1", json!({"id": FileId::generate().to_string()})),
        response("Não achei esse arquivo; posso te mandar o catálogo."),
    ]);
    let summary = scenario.run_turn(&engine).await;
    assert!(summary.texto.contains("catálogo"), "{}", summary.texto);
    let outputs = scenario.outputs(summary.turno);
    assert!(
        outputs.iter().all(|item| item.tipo == "text"),
        "nenhum arquivo enfileirado: {outputs:?}"
    );
}

#[tokio::test]
async fn send_enqueues_file_outbox_repeat_not_duplicates() {
    let scenario = Scenario::new("envio", &[NAME_TOOL]);
    let turno = TurnId::generate();
    let first = scenario
        .invoke(turno, json!({"id": scenario.catalog.to_string()}))
        .await
        .expect("enfileira");
    assert!(first.contains("enfileirado"), "{first}");

    let outputs = scenario.outputs(turno);
    assert_eq!(outputs.len(), 1);
    let item = &outputs[0];
    assert_eq!(item.tipo, "file");
    let payload = file_item(item).expect("conteúdo em JSON");
    assert_eq!(
        payload["blob_ref"],
        json!(format!("{}/catalogo.pdf", scenario.org))
    );
    assert_eq!(payload["arquivo_id"], json!(scenario.catalog.to_string()));
    assert_eq!(payload["mime"], json!("application/pdf"));

    let second = scenario
        .invoke(turno, json!({"id": scenario.catalog.to_string()}))
        .await
        .expect("repetição é aceita");
    assert!(second.contains("já estava enfileirado"), "{second}");
    assert_eq!(scenario.outputs(turno).len(), 1);

    scenario
        .invoke(turno, json!({"id": scenario.proposta.to_string()}))
        .await
        .expect("enfileira o segundo");
    assert_eq!(scenario.outputs(turno).len(), 2);
}

#[tokio::test]
async fn turn_complete_sends_file_answers() {
    let scenario = Scenario::new("turno", &[NAME_TOOL]);
    let (engine, _fake) = scenario.engine(vec![
        call("c1", json!({"id": scenario.catalog.to_string()})),
        response("Estou enviando o catálogo."),
    ]);
    let summary = scenario.run_turn(&engine).await;
    let outputs = scenario.outputs(summary.turno);
    let kinds: Vec<&str> = outputs.iter().map(|item| item.tipo.as_str()).collect();
    assert!(kinds.contains(&"file"), "{kinds:?}");
    assert!(kinds.contains(&"text"), "{kinds:?}");
}

#[tokio::test]
async fn agent_without_tool_not_receives() {
    let scenario = Scenario::new("sem-ferramenta", &[]);
    let (engine, fake) = scenario.engine(vec![response("ok")]);
    scenario.run_turn(&engine).await;
    let names: Vec<String> = fake.received()[0]
        .tools
        .iter()
        .map(|tool| tool.nome.clone())
        .collect();
    assert!(names.is_empty(), "{names:?}");
}

#[tokio::test]
async fn agent_with_tool_sees_description_files() {
    let scenario = Scenario::new("com-ferramenta", &[NAME_TOOL]);
    let (engine, fake) = scenario.engine(vec![response("ok")]);
    scenario.run_turn(&engine).await;
    let request = &fake.received()[0];
    let tool = request
        .tools
        .iter()
        .find(|tool| tool.nome == NAME_TOOL)
        .expect("a ferramenta vai no pedido");
    assert!(
        tool.descricao.contains(&scenario.catalog.to_string()),
        "a descrição traz o id do arquivo: {}",
        tool.descricao
    );
    assert!(
        tool.descricao.contains("catálogo de serviços"),
        "{}",
        tool.descricao
    );
    assert!(
        tool.descricao.contains("modelo de proposta"),
        "{}",
        tool.descricao
    );
    assert_eq!(
        tool.parameters["properties"]
            .as_object()
            .map(|map| map.keys().cloned().collect::<Vec<_>>()),
        Some(vec!["id".to_owned()])
    );
}

#[tokio::test]
async fn library_empty_appears_as_such_not_breaks_turn() {
    let scenario = Scenario::new("vazia", &[NAME_TOOL]);
    for file in [scenario.catalog, scenario.proposta] {
        scenario
            .store
            .library()
            .delete(scenario.org, scenario.agente, file)
            .expect("apaga o arquivo");
    }
    let (engine, fake) = scenario.engine(vec![response("ok")]);
    scenario.run_turn(&engine).await;
    let received = fake.received();
    let tool = received[0]
        .tools
        .iter()
        .find(|tool| tool.nome == NAME_TOOL)
        .expect("a ferramenta continua no pedido");
    assert!(
        tool.descricao.contains("biblioteca de arquivos está vazia"),
        "{}",
        tool.descricao
    );
}

#[tokio::test]
async fn argument_beyond_id_refused() {
    let scenario = Scenario::new("argumentos", &[NAME_TOOL]);
    let turno = TurnId::generate();
    let error = scenario
        .invoke(
            turno,
            json!({"id": scenario.catalog.to_string(), "caminho": "/etc/passwd"}),
        )
        .await
        .expect_err("caminho não é aceito");
    assert!(error.contains("não existe nesta ferramenta"), "{error}");
    assert!(scenario.outputs(turno).is_empty());
    assert!(scenario.invoke(turno, json!({"id": "  "})).await.is_err());
}

#[test]
fn file_library_brings_path_relative_blob() {
    let file = LibraryFile {
        id: FileId::generate(),
        org_id: OrgId::generate(),
        agent_id: AgentId::generate(),
        name: "x.pdf".to_owned(),
        description: String::new(),
        mime: "application/pdf".to_owned(),
        size_bytes: 1,
        sha256: "c".repeat(64),
        blob_path: "org/hash".to_owned(),
        created_at: 0,
    };
    assert!(!file.blob_path.starts_with('/'));
}

#[tokio::test]
async fn failure_model_after_file_enqueued_not_repeats_send() {
    let scenario = Scenario::new("falha-arquivo", &[NAME_TOOL]);
    let failure = Script::Failure(LlmError::Unavailable {
        tentativas: 3,
        last_error: "503".to_owned(),
    });
    let (engine, fake) = scenario.engine(vec![
        call("c1", json!({"id": scenario.catalog.to_string()})),
        failure,
        response("não deveria sair"),
    ]);
    let engine = Arc::new(engine);
    let processor = Arc::new(TurnProcessor::new(Arc::clone(&engine)));
    let actors = Actors::new(
        processor.clone(),
        ActorOptions::default(),
        MetricsMemory::own(),
    );
    let fluxo = Fluxo::with_metrics(
        Arc::clone(&scenario.store),
        actors,
        FlowOptions::default().with_failure_model(3, Duration::from_millis(50)),
        MetricsFlow::own(),
    );
    processor.attach_io(fluxo.clone());

    fluxo
        .receive_message(
            scenario.org,
            ReceivedMessage {
                integration_id: poph_types::IntegrationId::generate(),
                id_externo: "ext-arquivo-falha-1".to_owned(),
                conversa: scenario.conversa,
                texto: "manda o catálogo".to_owned(),
                attachments_json: "[]".to_owned(),
            },
        )
        .expect("grava a entrada");
    fluxo.dispatch(scenario.org).expect("despacha");

    for _ in 0..500 {
        let pending = scenario
            .store
            .inbox()
            .count_pending(scenario.org)
            .expect("pendentes");
        let running = scenario
            .store
            .turns()
            .list(
                scenario.org,
                scenario.conversa,
                poph_store::Page::with_limit(10),
            )
            .expect("turnos")
            .iter()
            .any(|turno| turno.state == poph_store::TurnState::Running);
        if pending == 0 && !running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
    fluxo.dispatch(scenario.org).expect("despacha de novo");
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(fake.received().len(), 2, "sem repetição do turno");
    let turns = scenario
        .store
        .turns()
        .list(
            scenario.org,
            scenario.conversa,
            poph_store::Page::with_limit(10),
        )
        .expect("turnos");
    assert_eq!(turns.len(), 1, "um turno só");
    let outputs = scenario
        .store
        .outbox()
        .list_pending(scenario.org, 50)
        .expect("saída");
    let files = outputs.iter().filter(|item| item.tipo == "file").count();
    assert_eq!(files, 1, "o arquivo sai uma vez");
    assert_eq!(
        scenario
            .store
            .inbox()
            .count_pending(scenario.org)
            .expect("pendentes"),
        0,
        "a entrada saiu da fila (concluída com o motivo)"
    );
}
