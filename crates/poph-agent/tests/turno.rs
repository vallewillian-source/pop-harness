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

type AoEsperar = Arc<dyn Fn() + Send + Sync>;

#[derive(Default, Clone)]
struct PausaFalsa {
    pedidos: Arc<Mutex<Vec<Duration>>>,
    ao_esperar: Option<AoEsperar>,
}

impl PausaFalsa {
    fn com_acao(acao: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            pedidos: Arc::default(),
            ao_esperar: Some(Arc::new(acao)),
        }
    }

    fn pedidos(&self) -> Vec<Duration> {
        self.pedidos
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}

impl Pause for PausaFalsa {
    fn wait(&self, duration: Duration) -> FuturePause {
        self.pedidos
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(duration);
        if let Some(acao) = &self.ao_esperar
            && !duration.is_zero()
        {
            acao();
        }
        Box::pin(async {})
    }
}

#[derive(Default)]
struct Coletor {
    events: Vec<TurnEvent>,
}

impl Coletor {
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

impl TurnObserver for Coletor {
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
        let conversa = Self::abrir_conversa(&store, org, agente.id, "cli");
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

    fn abrir_conversa(store: &Store, org: OrgId, agente: AgentId, canal: &str) -> ConversationId {
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

    fn avancar(&self, ms: i64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
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
            .expect("grava a mensagem do cliente")
            .id
    }

    fn pausa(&self) -> Arc<PausaFalsa> {
        Arc::new(PausaFalsa::default())
    }

    fn engine(
        &self,
        roteiros: Vec<Script>,
        pausa: Arc<PausaFalsa>,
    ) -> (TurnEngine, Arc<FakeProvider>) {
        self.motor_com(roteiros, pausa, TurnOptions::default())
    }

    fn motor_com(
        &self,
        roteiros: Vec<Script>,
        pausa: Arc<PausaFalsa>,
        options: TurnOptions,
    ) -> (TurnEngine, Arc<FakeProvider>) {
        let fake = Arc::new(FakeProvider::with_scripts(roteiros));
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
        .with_pause(pausa);
        (engine, fake)
    }

    fn fluxo(&self, engine: Arc<TurnEngine>) -> Fluxo {
        self.fluxo_com(engine, FlowOptions::default())
    }

    fn fluxo_com(&self, engine: Arc<TurnEngine>, options: FlowOptions) -> Fluxo {
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

    fn turnos(&self) -> Vec<poph_store::Turno> {
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
async fn cinco_mensagens_gravam_stream_mensagens_e_turno() {
    let scenario = Scenario::new("cinco");
    let roteiros: Vec<Script> = (1..=5)
        .map(|n| response(&format!("resposta {n}")))
        .collect();
    let pausa = scenario.pausa();
    let (engine, fake) = scenario.engine(roteiros, Arc::clone(&pausa));

    let mut textos = Vec::new();
    for n in 1..=5 {
        let gatilho = scenario.falar(&format!("pergunta {n}"));
        let mut coletor = Coletor::default();
        let summary = engine
            .run(
                TurnInput::new(scenario.org, scenario.conversa, gatilho),
                &mut coletor,
            )
            .await
            .expect("turno roda");
        assert_eq!(summary.estado, TurnState::Completed, "{:?}", summary.error);
        assert_eq!(summary.texto, format!("resposta {n}"));
        assert_eq!(
            coletor.texto(),
            summary.texto,
            "o stream é a resposta final"
        );
        assert!(
            coletor
                .events
                .iter()
                .any(|evento| matches!(evento, TurnEvent::Started { .. })),
            "o turno avisa o início"
        );
        textos.push(summary);
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

    let turnos = scenario.turnos();
    assert_eq!(turnos.len(), 5);
    for turno in &turnos {
        assert_eq!(turno.state, TurnState::Completed);
        assert_eq!(turno.input_tokens, 120);
        assert_eq!(turno.output_tokens, 30);
        let tempos: serde_json::Value =
            serde_json::from_str(&turno.times_json).expect("tempos em JSON");
        for chave in [
            "fila_ms",
            "montagem_ms",
            "primeiro_texto_ms",
            "total_ms",
            "rodadas_ferramenta",
        ] {
            assert!(tempos.get(chave).is_some(), "falta `{chave}`: {tempos}");
        }
        assert_eq!(tempos["modelo"], "gemini-3.8-flash-tiered");
        assert_eq!(tempos["motivo_fim"], "concluido");
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
async fn prefixo_identico_em_tres_turnos() {
    let scenario = Scenario::new("prefixo");
    let pausa = scenario.pausa();
    let (engine, fake) = scenario.engine(
        vec![response("um"), response("dois"), response("três")],
        pausa,
    );

    for n in 1..=3 {
        let gatilho = scenario.falar(&format!("mensagem {n}"));
        scenario.avancar(60_000);
        engine
            .run(
                TurnInput::new(scenario.org, scenario.conversa, gatilho),
                &mut NoopObserver,
            )
            .await
            .expect("turno roda");
    }

    let pedidos = fake.received();
    assert_eq!(pedidos.len(), 3);
    let prefix = |request: &poph_core::llm::Request| match &request.messages[0] {
        poph_core::llm::Message::System(texto) => texto.clone(),
        other => panic!("a primeira mensagem devia ser o prefixo: {other:?}"),
    };
    let first = prefix(&pedidos[0]);
    assert!(first.contains("FixProLar"), "{first}");
    for request in &pedidos[1..] {
        assert_eq!(prefix(request), first, "prefixo mudou entre turnos");
    }

    let volatile = |request: &poph_core::llm::Request| match &request.messages[1] {
        poph_core::llm::Message::System(texto) => texto.clone(),
        other => panic!("a segunda mensagem devia ser o bloco volátil: {other:?}"),
    };
    assert!(volatile(&pedidos[0]).contains("hora:"));
    assert_ne!(volatile(&pedidos[0]), volatile(&pedidos[1]));
}

#[tokio::test]
async fn duas_orgs_nao_se_misturam() {
    let alfa = Scenario::new("org-alfa");
    let beta = Scenario::new("org-beta");
    let (motor_alfa, fake_alfa) = alfa.engine(vec![response("sou a alfa")], alfa.pausa());
    let (motor_beta, fake_beta) = beta.engine(vec![response("sou a beta")], beta.pausa());

    let gatilho_alfa = alfa.falar("quem é você?");
    let gatilho_beta = beta.falar("e você?");
    let (mut obs_alfa, mut obs_beta) = (NoopObserver, NoopObserver);
    let (resumo_alfa, resumo_beta) = tokio::join!(
        motor_alfa.run(
            TurnInput::new(alfa.org, alfa.conversa, gatilho_alfa),
            &mut obs_alfa
        ),
        motor_beta.run(
            TurnInput::new(beta.org, beta.conversa, gatilho_beta),
            &mut obs_beta
        ),
    );
    let resumo_alfa = resumo_alfa.expect("turno da alfa");
    let resumo_beta = resumo_beta.expect("turno da beta");

    assert_eq!(resumo_alfa.texto, "sou a alfa");
    assert_eq!(resumo_beta.texto, "sou a beta");

    for request in fake_alfa.received() {
        assert_eq!(request.org, alfa.org);
    }
    for request in fake_beta.received() {
        assert_eq!(request.org, beta.org);
    }

    assert_eq!(alfa.turnos().len(), 1);
    assert_eq!(beta.turnos().len(), 1);
    let mensagens_beta = beta.messages();
    assert_eq!(mensagens_beta.len(), 2);
    assert_eq!(mensagens_beta[1].texto, "sou a beta");
    assert_eq!(mensagens_beta[1].conversation_id, beta.conversa);
}

#[tokio::test]
async fn janela_de_agrupamento_junta_duas_e_separa_depois_do_teto() {
    let scenario = Scenario::new("janela");
    let segunda: Arc<Mutex<Option<String>>> = Arc::default();
    let segunda_texto = "queria um orçamento";
    let escrever = {
        let segunda = Arc::clone(&segunda);
        let cenario_pasta = scenario.folder.clone();
        let store = Arc::clone(&scenario.store);
        let org = scenario.org;
        let conversa = scenario.conversa;
        move || {
            let mut guard = segunda.lock().unwrap_or_else(|v| v.into_inner());
            if guard.is_none() {
                let mensagem = store
                    .messages()
                    .write(
                        org,
                        NewMessage {
                            conversation_id: conversa,
                            direction: Direction::Inbound,
                            role: Role::User,
                            texto: segunda_texto.to_owned(),
                            attachments_json: "[]".to_owned(),
                            external_id: None,
                            turn_id: None,
                            origin: poph_store::Origin::Agente,
                            author_name: None,
                        },
                    )
                    .expect("grava a mensagem que chega na janela");
                *guard = Some(mensagem.id.to_string());
                let _ = cenario_pasta;
            }
        }
    };
    let pausa = Arc::new(PausaFalsa::com_acao(escrever));
    let (engine, fake) = scenario.engine(
        vec![response("juntas"), response("sozinha")],
        Arc::clone(&pausa),
    );

    let first = scenario.falar("oi");
    let summary = engine
        .run(
            TurnInput::new(scenario.org, scenario.conversa, first),
            &mut NoopObserver,
        )
        .await
        .expect("turno roda");
    assert_eq!(summary.estado, TurnState::Completed);
    let segunda_id: MessageId = segunda
        .lock()
        .unwrap_or_else(|v| v.into_inner())
        .as_ref()
        .expect("a segunda mensagem chegou")
        .parse()
        .expect("id da segunda mensagem");
    assert_eq!(
        summary.messages,
        vec![first, segunda_id],
        "as duas mensagens da janela entraram no mesmo turno"
    );
    assert_eq!(fake.received().len(), 1, "um turno, uma chamada ao modelo");

    let pedidos = pausa.pedidos();
    assert_eq!(pedidos, vec![Duration::from_millis(2_000)]);

    scenario.avancar(9_000);
    let terceira = scenario.falar("e o prazo?");
    let summary = engine
        .run(
            TurnInput::new(scenario.org, scenario.conversa, terceira),
            &mut NoopObserver,
        )
        .await
        .expect("segundo turno roda");
    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.messages, vec![terceira], "turno novo, uma mensagem");
    assert_eq!(fake.received().len(), 2);

    let messages = scenario.messages();
    assert_eq!(messages.len(), 5);
    assert_eq!(scenario.turnos().len(), 2);
}

struct FerramentaDeTeste {
    nome: &'static str,
}

impl poph_agent::turno::InternalTool for FerramentaDeTeste {
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

struct DoisRegistros;

impl ToolRegistry for DoisRegistros {
    fn tool(&self, nome: &str) -> Option<Arc<dyn poph_agent::turno::InternalTool>> {
        match nome {
            "consultar_base" => Some(Arc::new(FerramentaDeTeste {
                nome: "consultar_base",
            })),
            "buscar_produto" => Some(Arc::new(FerramentaDeTeste {
                nome: "buscar_produto",
            })),
            _ => None,
        }
    }
}

#[tokio::test]
async fn ferramenta_nao_habilitada_nao_e_oferecida_e_laco_para_no_limite() {
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

    let duas_chamadas = Script::Events(vec![
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
    let (engine, fake) = scenario.engine(vec![duas_chamadas, response("pronto")], scenario.pausa());
    let engine = engine.with_tools(Arc::new(DoisRegistros));

    let gatilho = scenario.falar("quanto custa a dedetização?");
    let summary = engine
        .run(
            TurnInput::new(scenario.org, scenario.conversa, gatilho),
            &mut NoopObserver,
        )
        .await
        .expect("turno roda");
    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, "pronto");

    let pedidos = fake.received();
    assert_eq!(
        pedidos.len(),
        2,
        "uma rodada de ferramenta (o limite) e o fecho em texto"
    );

    let names: Vec<String> = pedidos[0]
        .tools
        .iter()
        .map(|tool| tool.nome.clone())
        .collect();
    assert_eq!(names, vec!["consultar_base".to_owned()]);

    let resultados: Vec<String> = pedidos[1]
        .messages
        .iter()
        .filter_map(|mensagem| match mensagem {
            poph_core::llm::Message::Resultado { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect();
    assert!(
        resultados[0].contains("não está habilitada"),
        "{:?}",
        resultados
    );

    assert!(
        resultados[1].contains("limite de 1 chamadas"),
        "{:?}",
        resultados
    );

    assert!(pedidos[1].tools.is_empty());
}

#[tokio::test]
async fn cancelamento_encerra_o_turno_e_conclui_a_entrada() {
    let scenario = Scenario::new("cancelamento");
    let (engine, _fake) = scenario.engine(
        vec![Script::Pending {
            events: vec![LlmEvent::Texto("parcial ".to_owned())],
            after: Duration::from_secs(30),
            error: LlmError::TimedOut { seconds: 30 },
        }],
        scenario.pausa(),
    );
    let compartilhado = Cancellation::new();
    let fluxo = scenario.fluxo(Arc::new(engine.with_cancellation(compartilhado.clone())));

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
    compartilhado.cancel();
    esperar_fila_vazia(&fluxo, &scenario).await;

    let turnos = scenario.turnos();
    assert_eq!(turnos.len(), 1);
    assert_eq!(turnos[0].state, TurnState::Failed);
    assert!(
        turnos[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("cancelada"),
        "{:?}",
        turnos[0].error
    );
    assert!(turnos[0].finished_at.is_some(), "o turno foi fechado");

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
async fn erro_do_provedor_grava_o_turno_e_nao_deixa_item_preso() {
    let scenario = Scenario::new("erro-provedor");
    let (engine, _fake) = scenario.engine(
        vec![Script::Failure(LlmError::Unavailable {
            tentativas: 3,
            last_error: "503".to_owned(),
        })],
        scenario.pausa(),
    );
    let fluxo = scenario.fluxo(Arc::new(engine));

    let recebido = fluxo
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
    assert_eq!(recebido.situation, poph_store::Received::New);
    fluxo.dispatch(scenario.org).expect("despacha");
    esperar_tentativas(&scenario, recebido.item.id, 1).await;

    let transport = TransporteDeTeste::default();
    fluxo
        .send_pending(scenario.org, &transport)
        .await
        .expect("envia");

    let turnos = scenario.turnos();
    assert_eq!(turnos.len(), 1);
    assert_eq!(turnos[0].state, TurnState::Failed);
    assert!(
        turnos[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("503"),
        "{:?}",
        turnos[0].error
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
        .attempts(scenario.org, &[recebido.item.id])
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
async fn sem_vaga_deixa_a_entrada_pendente_para_nova_tentativa() {
    let scenario = Scenario::new("sem-vaga");
    let (engine, fake) =
        scenario.engine(vec![response("não deveria ser chamada")], scenario.pausa());
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
    .with_pause(scenario.pausa());
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
    esperar_turno_fechado(&scenario).await;

    let turnos = scenario.turnos();
    assert_eq!(turnos.len(), 1);
    assert_eq!(turnos[0].state, TurnState::Failed);
    assert!(
        turnos[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("sem vaga"),
        "{:?}",
        turnos[0].error
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

fn limitador_de_uma_vaga() -> Arc<Limiter> {
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

fn motor_com_limitador(
    scenario: &Scenario,
    roteiros: Vec<Script>,
    limiter: Arc<Limiter>,
) -> (TurnEngine, Arc<FakeProvider>) {
    let fake = Arc::new(FakeProvider::with_scripts(roteiros));
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
    .with_pause(scenario.pausa());
    (engine, fake)
}

const PROVEDOR_DO_AGENTE: &str = "antigravity2api";

#[tokio::test]
async fn falta_de_vaga_do_turno_entra_na_escada_e_esgota_em_tres_tentativas() {
    let scenario = Scenario::new("vaga-turno-esgota");
    let limiter = limitador_de_uma_vaga();
    let _ocupada = limiter
        .acquire_turn(scenario.org, scenario.agente, Duration::from_millis(10))
        .await
        .expect("ocupa a vaga da instância");
    let (engine, fake) = motor_com_limitador(
        &scenario,
        vec![response("não deveria ser chamada")],
        Arc::clone(&limiter),
    );
    let engine = Arc::new(engine);
    let fluxo = scenario.fluxo_com(
        Arc::clone(&engine),
        FlowOptions::default().with_failure_model(3, Duration::from_millis(1_000)),
    );
    let item = entregar(&fluxo, &scenario, "ext-vaga-turno-1", "oi");

    fluxo.dispatch(scenario.org).expect("despacha");
    esperar_tentativas(&scenario, item, 1).await;
    assert!(
        scenario
            .store
            .outbox()
            .list_pending(scenario.org, 10)
            .expect("saída")
            .is_empty()
    );
    scenario.avancar(1_000);
    fluxo.dispatch(scenario.org).expect("despacha 2");
    esperar_tentativas(&scenario, item, 2).await;
    scenario.avancar(4_000);
    fluxo.dispatch(scenario.org).expect("despacha 3");
    esperar_fila_vazia(&fluxo, &scenario).await;

    assert_eq!(scenario.turnos().len(), 3);
    assert!(
        scenario
            .turnos()
            .iter()
            .all(|t| t.state == TurnState::Failed)
    );
    assert_eq!(fake.received().len(), 0);
    let concluido = scenario
        .store
        .inbox()
        .get(scenario.org, item)
        .expect("item");
    assert!(
        concluido.processed_at.is_some(),
        "concluída depois do limite"
    );
    assert!(
        concluido
            .ultimo_erro
            .as_deref()
            .unwrap_or_default()
            .contains("sem vaga"),
        "{:?}",
        concluido.ultimo_erro
    );
    assert_eq!(scenario.messages().len(), 1, "só a mensagem do cliente");

    scenario.avancar(3_600_000);
    fluxo.dispatch(scenario.org).expect("despacha de novo");
    deixar_passar(60).await;
    assert_eq!(
        scenario.turnos().len(),
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
async fn falta_de_vaga_do_turno_na_primeira_tentativa_e_resposta_na_segunda() {
    let scenario = Scenario::new("vaga-turno-repete");
    let limiter = limitador_de_uma_vaga();
    let ocupada = limiter
        .acquire_turn(scenario.org, scenario.agente, Duration::from_millis(10))
        .await
        .expect("ocupa a vaga da instância");
    let (engine, fake) = motor_com_limitador(
        &scenario,
        vec![response("bom dia, em que posso ajudar")],
        Arc::clone(&limiter),
    );
    let engine = Arc::new(engine);
    let fluxo = scenario.fluxo_com(
        Arc::clone(&engine),
        FlowOptions::default().with_failure_model(3, Duration::from_millis(1_000)),
    );
    let item = entregar(&fluxo, &scenario, "ext-vaga-turno-2", "oi");

    fluxo.dispatch(scenario.org).expect("despacha");
    esperar_tentativas(&scenario, item, 1).await;
    drop(ocupada);
    scenario.avancar(1_000);
    fluxo.dispatch(scenario.org).expect("despacha 2");
    esperar_fila_vazia(&fluxo, &scenario).await;

    assert_eq!(fake.received().len(), 1);
    let respostas = scenario
        .messages()
        .into_iter()
        .filter(|m| m.role == Role::Assistant)
        .count();
    assert_eq!(respostas, 1);
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
async fn falta_de_vaga_do_provedor_entra_na_escada_e_nao_chama_o_modelo() {
    let scenario = Scenario::new("vaga-provedor");
    let limiter = limitador_de_uma_vaga();
    let mut ocupada = Some(
        limiter
            .acquire_provider(PROVEDOR_DO_AGENTE, Duration::from_millis(10))
            .await
            .expect("ocupa a vaga do provedor"),
    );
    let (engine, fake) = motor_com_limitador(
        &scenario,
        vec![response("bom dia, em que posso ajudar")],
        Arc::clone(&limiter),
    );
    let engine = Arc::new(engine);
    let fluxo = scenario.fluxo_com(
        Arc::clone(&engine),
        FlowOptions::default().with_failure_model(3, Duration::from_millis(1_000)),
    );
    let item = entregar(&fluxo, &scenario, "ext-vaga-prov-1", "oi");

    fluxo.dispatch(scenario.org).expect("despacha");
    esperar_tentativas(&scenario, item, 1).await;
    assert_eq!(fake.received().len(), 0);
    assert!(
        scenario
            .store
            .outbox()
            .list_pending(scenario.org, 10)
            .expect("saída")
            .is_empty()
    );
    drop(ocupada.take());
    scenario.avancar(1_000);
    fluxo.dispatch(scenario.org).expect("despacha 2");
    esperar_fila_vazia(&fluxo, &scenario).await;

    assert_eq!(fake.received().len(), 1, "uma única chamada ao provedor");
    assert_eq!(scenario.turnos().len(), 2);
    let metrics = engine.metrics();
    assert_eq!(metrics.failures_model_deferred.value(), 1);
    assert_eq!(metrics.failures_model_definitive.value(), 0);
}

#[tokio::test]
async fn provedor_que_recusa_a_chave_nao_gasta_a_escada() {
    let scenario = Scenario::new("recusa-401");
    let (engine, fake) = scenario.engine(
        vec![Script::Failure(LlmError::Rejected {
            status: 401,
            mensagem: "chave recusada".to_owned(),
        })],
        scenario.pausa(),
    );
    let engine = Arc::new(engine);
    let fluxo = scenario.fluxo_com(
        Arc::clone(&engine),
        FlowOptions::default().with_failure_model(3, Duration::from_millis(1_000)),
    );
    let item = entregar(&fluxo, &scenario, "ext-401-1", "oi");

    fluxo.dispatch(scenario.org).expect("despacha");
    esperar_fila_vazia(&fluxo, &scenario).await;

    assert_eq!(fake.received().len(), 1);
    let turnos = scenario.turnos();
    assert_eq!(turnos.len(), 1);
    assert_eq!(turnos[0].state, TurnState::Failed);
    let concluido = scenario
        .store
        .inbox()
        .get(scenario.org, item)
        .expect("item");
    assert!(
        concluido.processed_at.is_some(),
        "concluída na primeira tentativa"
    );
    assert!(
        concluido
            .ultimo_erro
            .as_deref()
            .unwrap_or_default()
            .contains("401"),
        "{:?}",
        concluido.ultimo_erro
    );
    assert!(
        scenario
            .store
            .outbox()
            .list_pending(scenario.org, 10)
            .expect("saída")
            .is_empty()
    );

    scenario.avancar(3_600_000);
    fluxo.dispatch(scenario.org).expect("despacha de novo");
    deixar_passar(60).await;
    assert_eq!(fake.received().len(), 1);

    let metrics = engine.metrics();
    assert_eq!(metrics.failures_model_refused.value(), 1);
    assert_eq!(metrics.failures_model_definitive.value(), 1);
    assert_eq!(metrics.failures_model_deferred.value(), 0);
}

#[derive(Default)]
struct TransporteDeTeste {
    sent: Mutex<Vec<OutboxItem>>,
}

impl TransporteDeTeste {
    fn sent(&self) -> Vec<OutboxItem> {
        self.sent
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}

impl OutputTransport for TransporteDeTeste {
    fn send(&self, item: OutboxItem) -> SendFuture {
        self.sent
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(item);
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn fluxo_completo_da_inbox_ate_a_outbox_entrega_a_resposta() {
    let scenario = Scenario::new("fluxo-completo");
    let (engine, fake) = scenario.engine(vec![response("claro, posso ajudar")], scenario.pausa());
    let fluxo = scenario.fluxo(Arc::new(engine));

    let entrega = || ReceivedMessage {
        integration_id: scenario.integracao,
        id_externo: "ext-1".to_owned(),
        conversa: scenario.conversa,
        texto: "bom dia".to_owned(),
        attachments_json: "[]".to_owned(),
    };
    assert_eq!(
        fluxo
            .receive_message(scenario.org, entrega())
            .expect("grava")
            .situation,
        poph_store::Received::New
    );
    let segunda = fluxo
        .receive_message(scenario.org, entrega())
        .expect("reentrega da mesma mensagem");
    assert_eq!(
        segunda.situation,
        poph_store::Received::Duplicate,
        "a segunda entrega é duplicata, não erro"
    );

    fluxo.dispatch(scenario.org).expect("despacha");
    esperar_fila_vazia(&fluxo, &scenario).await;

    let transport = TransporteDeTeste::default();
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
    let turnos = scenario.turnos();
    assert_eq!(turnos.len(), 1);
    assert_eq!(turnos[0].state, TurnState::Completed);
    let messages = scenario.messages();
    assert_eq!(messages.len(), 2, "uma fala do cliente e uma do agente");
}

async fn esperar_turno_fechado(scenario: &Scenario) {
    for _ in 0..300 {
        if scenario
            .turnos()
            .iter()
            .any(|turno| turno.state != TurnState::Running)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("o turno não fechou a tempo");
}

async fn esperar_fila_vazia(fluxo: &Fluxo, scenario: &Scenario) {
    for _ in 0..300 {
        let pending = fluxo
            .statistics(scenario.org)
            .expect("lê as estatísticas")
            .entry_pending;
        let turno_rodando = scenario
            .turnos()
            .iter()
            .any(|turno| turno.state == TurnState::Running);
        if pending == 0 && !turno_rodando {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("a fila não esvaziou a tempo");
}

fn falha_do_provedor() -> Script {
    Script::Failure(LlmError::Unavailable {
        tentativas: 3,
        last_error: "503 indisponível".to_owned(),
    })
}

fn entregar(
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

async fn esperar_tentativas(scenario: &Scenario, id: poph_types::InboxId, minimo: u32) {
    for _ in 0..500 {
        let current = scenario
            .store
            .inbox()
            .attempts(scenario.org, &[id])
            .expect("tentativas")[0];
        if current >= minimo {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("a tentativa {minimo} do item não foi registrada a tempo");
}

async fn deixar_passar(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tokio::test]
async fn falha_do_modelo_repete_com_espera_crescente_e_responde_na_terceira() {
    let scenario = Scenario::new("falha-repete");
    let (engine, fake) = scenario.engine(
        vec![
            falha_do_provedor(),
            falha_do_provedor(),
            response("bom dia, em que posso ajudar"),
        ],
        scenario.pausa(),
    );
    let engine = Arc::new(engine);
    let fluxo = scenario.fluxo_com(
        Arc::clone(&engine),
        FlowOptions::default().with_failure_model(3, Duration::from_millis(1_000)),
    );
    let item = entregar(&fluxo, &scenario, "ext-repete-1", "oi");

    fluxo.dispatch(scenario.org).expect("despacha");
    esperar_tentativas(&scenario, item, 1).await;
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
    deixar_passar(60).await;
    assert_eq!(scenario.turnos().len(), 1, "nada roda antes do instante");
    assert_eq!(fake.received().len(), 1);

    scenario.avancar(1_000);
    fluxo.dispatch(scenario.org).expect("despacha vencido");
    esperar_tentativas(&scenario, item, 2).await;
    assert_eq!(fake.received().len(), 2);

    scenario.avancar(3_000);
    fluxo
        .dispatch(scenario.org)
        .expect("despacha antes da segunda espera");
    deixar_passar(60).await;
    assert_eq!(
        fake.received().len(),
        2,
        "espera crescente: ainda não venceu"
    );
    scenario.avancar(1_000);
    fluxo.dispatch(scenario.org).expect("despacha vencido 2");
    esperar_fila_vazia(&fluxo, &scenario).await;

    assert_eq!(fake.received().len(), 3);
    let states: Vec<TurnState> = scenario.turnos().iter().map(|t| t.state).collect();
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
    let respostas: Vec<_> = messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .collect();
    assert_eq!(respostas.len(), 1, "uma resposta do agente");
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
async fn falha_do_modelo_sempre_esgota_em_tres_tentativas_e_conclui_sem_resposta() {
    let scenario = Scenario::new("falha-esgota");
    let (engine, fake) = scenario.engine(
        vec![
            falha_do_provedor(),
            falha_do_provedor(),
            falha_do_provedor(),
        ],
        scenario.pausa(),
    );
    let engine = Arc::new(engine);
    let fluxo = scenario.fluxo_com(
        Arc::clone(&engine),
        FlowOptions::default().with_failure_model(3, Duration::from_millis(1_000)),
    );
    let item = entregar(&fluxo, &scenario, "ext-esgota-1", "oi");

    fluxo.dispatch(scenario.org).expect("despacha");
    esperar_tentativas(&scenario, item, 1).await;
    scenario.avancar(1_000);
    fluxo.dispatch(scenario.org).expect("despacha 2");
    esperar_tentativas(&scenario, item, 2).await;
    scenario.avancar(4_000);
    fluxo.dispatch(scenario.org).expect("despacha 3");
    esperar_fila_vazia(&fluxo, &scenario).await;

    assert_eq!(fake.received().len(), 3);
    let turnos = scenario.turnos();
    assert_eq!(turnos.len(), 3);
    assert!(turnos.iter().all(|t| t.state == TurnState::Failed));

    let concluido = scenario
        .store
        .inbox()
        .get(scenario.org, item)
        .expect("item");
    assert!(concluido.processed_at.is_some(), "a entrada foi concluída");
    assert!(
        concluido
            .ultimo_erro
            .as_deref()
            .unwrap_or_default()
            .contains("503"),
        "{:?}",
        concluido.ultimo_erro
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

    scenario.avancar(3_600_000);
    fluxo.dispatch(scenario.org).expect("despacha de novo");
    deixar_passar(60).await;
    assert_eq!(scenario.turnos().len(), 3);
    assert_eq!(fake.received().len(), 3);

    let metrics = engine.metrics();
    assert_eq!(metrics.failures_model_deferred.value(), 2);
    assert_eq!(metrics.failures_model_definitive.value(), 1);
}

#[tokio::test]
async fn com_limite_de_uma_tentativa_a_primeira_falha_ja_e_definitiva() {
    let scenario = Scenario::new("falha-uma");
    let (engine, fake) = scenario.engine(vec![falha_do_provedor()], scenario.pausa());
    let engine = Arc::new(engine);
    let fluxo = scenario.fluxo_com(
        Arc::clone(&engine),
        FlowOptions::default().with_failure_model(1, Duration::from_millis(1_000)),
    );
    entregar(&fluxo, &scenario, "ext-uma-1", "oi");
    fluxo.dispatch(scenario.org).expect("despacha");
    esperar_fila_vazia(&fluxo, &scenario).await;
    assert_eq!(fake.received().len(), 1);
    assert_eq!(engine.metrics().failures_model_definitive.value(), 1);
    assert_eq!(engine.metrics().failures_model_deferred.value(), 0);
}

#[tokio::test]
async fn tempo_maximo_do_turno_conta_como_falha_do_modelo_e_entra_na_escada() {
    let scenario = Scenario::new("tempo-maximo");
    let pendurado = Script::Pending {
        events: Vec::new(),
        after: Duration::from_secs(10),
        error: LlmError::TimedOut { seconds: 10 },
    };
    let opcoes_turno = TurnOptions {
        turn_timeout: Duration::from_millis(50),
        ..TurnOptions::default()
    };
    let (engine, fake) = scenario.motor_com(
        vec![pendurado, response("consegui responder")],
        scenario.pausa(),
        opcoes_turno,
    );
    let engine = Arc::new(engine);
    let fluxo = scenario.fluxo_com(
        Arc::clone(&engine),
        FlowOptions::default().with_failure_model(3, Duration::from_millis(1_000)),
    );
    let item = entregar(&fluxo, &scenario, "ext-tempo-1", "oi");

    fluxo.dispatch(scenario.org).expect("despacha");
    esperar_tentativas(&scenario, item, 1).await;
    let turno = scenario.turnos().into_iter().next().expect("um turno");
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

    scenario.avancar(1_000);
    fluxo.dispatch(scenario.org).expect("despacha 2");
    esperar_fila_vazia(&fluxo, &scenario).await;
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
async fn repetir_depois_de_resposta_ja_gravada_nao_chama_o_modelo() {
    let scenario = Scenario::new("ja-respondida");
    let (engine, fake) = scenario.engine(
        vec![falha_do_provedor(), response("não deveria ser usada")],
        scenario.pausa(),
    );
    let engine = Arc::new(engine);
    let fluxo = scenario.fluxo_com(
        Arc::clone(&engine),
        FlowOptions::default().with_failure_model(3, Duration::from_millis(1_000)),
    );
    let item = entregar(&fluxo, &scenario, "ext-ja-1", "oi");
    fluxo.dispatch(scenario.org).expect("despacha");
    esperar_tentativas(&scenario, item, 1).await;
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

    scenario.avancar(1_000);
    fluxo.dispatch(scenario.org).expect("despacha de novo");
    esperar_fila_vazia(&fluxo, &scenario).await;

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
async fn item_adiado_so_volta_junto_com_a_mensagem_nova_e_nao_antes_do_instante() {
    let scenario = Scenario::new("adiado-com-nova");
    let (engine, fake) = scenario.engine(
        vec![falha_do_provedor(), response("respondi as duas")],
        scenario.pausa(),
    );
    let fluxo = scenario.fluxo_com(
        Arc::new(engine),
        FlowOptions::default().with_failure_model(3, Duration::from_millis(5_000)),
    );
    let first = entregar(&fluxo, &scenario, "ext-antecipa-1", "primeira");
    fluxo.dispatch(scenario.org).expect("despacha");
    esperar_tentativas(&scenario, first, 1).await;
    assert_eq!(scenario.turnos().len(), 1);

    fluxo.dispatch(scenario.org).expect("despacha sem vencer");
    deixar_passar(60).await;
    assert_eq!(
        scenario.turnos().len(),
        1,
        "o adiado não roda antes da hora"
    );
    assert_eq!(fake.received().len(), 1);

    entregar(&fluxo, &scenario, "ext-antecipa-2", "segunda");
    fluxo.dispatch(scenario.org).expect("despacha a nova");
    esperar_fila_vazia(&fluxo, &scenario).await;

    assert_eq!(fake.received().len(), 2);
    let respostas = scenario
        .messages()
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .count();
    assert_eq!(respostas, 1, "uma resposta para as duas mensagens");
    assert_eq!(
        scenario
            .store
            .inbox()
            .count_pending(scenario.org)
            .expect("pendentes"),
        0
    );
}
