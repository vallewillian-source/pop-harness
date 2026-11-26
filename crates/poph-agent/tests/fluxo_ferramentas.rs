use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsG9, MetricsMemory, MetricsTurn};
use poph_agent::turno::{
    FuturePause, NoopObserver, Pause, TurnEngine, TurnInput, TurnOptions, TurnSummary,
};
use poph_core::llm::{
    FakeProvider, FinishReason, LlmEvent, Message, Request, Script, ToolCall, Usage,
};
use poph_fluxos::{Limites, compilar};
use poph_jev::{FakeJev, G9_NONE, JevEngine};
use poph_store::{
    AgentChange, ConversationStatus, Direction, NewAgent, NewConversation, NewFlow, NewMessage,
    NewOrg, RevisionPublication, Role, Store, TurnState,
};
use poph_types::{
    AgentId, ClosedBy, ConversationId, ExecutionStatus, FlowsConfig, JevGates, MessageId, OrgId,
    RecordEventKind, RecordStatus,
};
use serde_json::{Value, json};

struct SemEspera;

impl Pause for SemEspera {
    fn wait(&self, _duracao: Duration) -> FuturePause {
        Box::pin(async {})
    }
}

const SCRIPT: &str = r#"fluxo(
    id = "agendar_visita",
    nome = "Agendar visita",
    inicio = "coleta",
    gatilho = gatilho(quando = "O cliente quer marcar uma visita técnica."),
)

etapa(
    id = "coleta",
    nome = "Coleta",
    instrucao = "Peça o nome do cliente.",
    campos = [campo("nome", "Nome", obrigatorio = True)],
    acoes = [
        acao(
            "seguir",
            "Seguir",
            exige = ["nome"],
            vai_para = "confirmacao",
            efeitos = [mensagem_ao_cliente("Já vou confirmar sua visita.")],
        ),
    ],
)

etapa(
    id = "confirmacao",
    nome = "Confirmação",
    instrucao = "Confirme a visita com o cliente.",
    acoes = [
        acao(
            "confirmar",
            "Confirmar",
            vai_para = fim("pronto", resultado = "Visita agendada", encerra_atendimento = True),
        ),
    ],
)
"#;

const SCRIPT_B: &str = r#"fluxo(
    id = "confirmar_dados",
    nome = "Confirmar dados",
    inicio = "dados",
)

etapa(
    id = "dados",
    nome = "Dados",
    acoes = [acao("seguir", "Seguir", vai_para = fim("pronto"))],
)
"#;

const TODAS: [&str; 3] = [
    poph_agent::turno::NAME_RUN_ACTION,
    poph_agent::turno::NAME_ABANDON,
    poph_agent::turno::NAME_TOOL_START,
];

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-fx24-{label}-{}-{}",
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
        let scenario = Self {
            folder,
            store,
            org,
            agente,
        };
        scenario.habilitar(&TODAS);
        scenario
    }

    fn habilitar(&self, tools: &[&str]) {
        let structured = json!({
            "tools": tools
                .iter()
                .map(|nome| json!({ "name": nome, "enabled": true }))
                .collect::<Vec<_>>()
        })
        .to_string();
        let current = self.store.instructions().version(self.org, self.agente);
        let esperada = match current {
            Ok(Some(versao)) => versao,
            _ => 0,
        };
        self.store
            .instructions()
            .save(
                self.org,
                self.agente,
                esperada,
                &structured,
                "# Bia\n\nVocê é a atendente da FixProLar.",
                true,
            )
            .expect("grava as instruções");
    }

    fn fluxo(&self) -> poph_types::FlowId {
        self.criar_fluxo("agendar_visita", "Agendar visita", SCRIPT, true)
    }

    fn criar_fluxo(
        &self,
        slug: &str,
        nome: &str,
        script: &str,
        liberar: bool,
    ) -> poph_types::FlowId {
        let fluxo = self
            .store
            .flows()
            .create(
                self.org,
                NewFlow {
                    slug: slug.to_owned(),
                    name: nome.to_owned(),
                    description: String::new(),
                    rascunho_fonte: script.to_owned(),
                },
            )
            .expect("cria o fluxo");
        let compiled = compilar(script, &Limites::default());
        let ir = compiled.ir().expect("o script do teste compila");
        self.store
            .revisions()
            .publish_revision(
                self.org,
                fluxo.id,
                RevisionPublication {
                    expected: fluxo.version,
                    fonte: script.to_owned(),
                    definicao_json: serde_json::to_string(ir).expect("IR em JSON"),
                    schema_versao: Some(ir.schema_versao),
                    resolucoes_json: "{}".to_owned(),
                    ator_tipo: "system".to_owned(),
                    actor_id: None,
                },
            )
            .expect("publica a revisão");
        if liberar {
            self.store
                .flows()
                .release(self.org, self.agente, fluxo.id)
                .expect("libera o fluxo");
        }
        fluxo.id
    }

    fn desativar(&self, fluxo: poph_types::FlowId) {
        let current = self.store.flows().get(self.org, fluxo).expect("lê o fluxo");
        self.store
            .flows()
            .update(
                self.org,
                fluxo,
                current.version,
                poph_store::FlowChange {
                    status: Some(poph_store::Status::Inactive),
                    ..poph_store::FlowChange::default()
                },
            )
            .expect("desativa o fluxo");
    }

    fn conversa(&self) -> ConversationId {
        self.store
            .conversations()
            .create(
                self.org,
                NewConversation {
                    agent_id: self.agente,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: Some("cliente-ficticio".to_owned()),
                    target_department: None,
                },
            )
            .expect("abre a conversa")
            .id
    }

    fn falar(&self, conversa: ConversationId, texto: &str) -> MessageId {
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
        fluxos: Option<FlowsConfig>,
    ) -> (TurnEngine, Arc<FakeProvider>) {
        let config = ConfigHot::new(
            Arc::clone(&self.store),
            HotConfigOptions::default(),
            MetricsMemory::own(),
        );
        let limiter = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
        let provedor = Arc::new(FakeProvider::with_scripts(roteiros));
        let mut engine = TurnEngine::new(
            Arc::clone(&self.store),
            config,
            limiter,
            provedor.clone(),
            TurnOptions::default(),
            MetricsTurn::own(),
        )
        .with_pause(Arc::new(SemEspera))
        .with_summary(poph_agent::summary::SummaryOptions {
            ativo: false,
            ..poph_agent::summary::SummaryOptions::default()
        })
        .with_g9_metrics(MetricsG9::own());
        let registro = poph_agent::turno::registrar_ferramentas_do_fluxo(
            poph_agent::turno::DefaultRegistry::new(),
            Arc::clone(&self.store),
            &fluxos.clone().unwrap_or_default(),
        );
        let registro =
            poph_agent::turno::registrar_ferramenta_encerramento(registro, Arc::clone(&self.store));
        engine = engine.with_tools(Arc::new(registro));
        if let Some(adjustments) = fluxos {
            engine = engine.with_flows(&adjustments);
        }
        if let Some(jev) = jev {
            engine = engine.with_jev(jev);
        }
        (engine, provedor)
    }

    async fn turno(
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

    fn ficha(&self, conversa: ConversationId) -> poph_store::Ficha {
        self.store
            .cases()
            .by_conversation(self.org, conversa)
            .expect("lê a Ficha")
            .expect("a conversa tem Ficha")
    }

    fn active(&self, conversa: ConversationId) -> Option<poph_store::Execution> {
        self.store
            .executions()
            .active_of_case(self.org, self.ficha(conversa).id)
            .expect("lê a execução ativa")
    }

    fn execution(&self, conversa: ConversationId) -> poph_store::Execution {
        self.store
            .executions()
            .from_case(self.org, self.ficha(conversa).id, 20)
            .expect("lê as execuções")
            .into_iter()
            .next()
            .expect("a Ficha tem execução")
    }

    fn events(&self, conversa: ConversationId) -> Vec<poph_store::RecordEvent> {
        self.store
            .cases()
            .events(self.org, self.ficha(conversa).id, None, 50)
            .expect("lê os eventos")
    }

    fn pending(&self) -> Vec<poph_store::FlowEffect> {
        self.store
            .effects()
            .list_pending(self.org, 20)
            .expect("lê a fila de efeitos")
    }

    fn messages(&self, conversa: ConversationId) -> Vec<poph_store::Mensagem> {
        self.store
            .messages()
            .list(self.org, conversa, poph_store::Page::with_limit(50))
            .expect("lista as mensagens")
    }

    fn turno_gravado(&self, conversa: ConversationId) -> poph_store::Turno {
        self.store
            .turns()
            .list(self.org, conversa, poph_store::Page::with_limit(5))
            .expect("lista os turnos")
            .into_iter()
            .next()
            .expect("há um turno gravado")
    }

    fn calls(&self, conversa: ConversationId) -> Vec<Value> {
        let turno = self.turno_gravado(conversa);
        let registro: Value = serde_json::from_str(&turno.tools_json).expect("tools_json é JSON");
        registro["chamadas"].as_array().cloned().unwrap_or_default()
    }

    fn ajustar(&self, mudanca: AgentChange) {
        let current = self
            .store
            .agents()
            .get(self.org, self.agente)
            .expect("lê o agente");
        self.store
            .agents()
            .update(self.org, self.agente, current.version, mudanca)
            .expect("ajusta o agente");
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

fn texto(content: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(content.to_owned()),
        LlmEvent::Usage(Usage {
            input: 10,
            saida: 5,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

fn call(nome: &str, arguments: Value) -> Script {
    Script::Events(vec![
        LlmEvent::Tool(ToolCall {
            id: "chamada-1".to_owned(),
            nome: nome.to_owned(),
            arguments,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

fn duas_chamadas(first: (&str, Value), segunda: (&str, Value)) -> Script {
    Script::Events(vec![
        LlmEvent::Tool(ToolCall {
            id: "chamada-1".to_owned(),
            nome: first.0.to_owned(),
            arguments: first.1,
        }),
        LlmEvent::Tool(ToolCall {
            id: "chamada-2".to_owned(),
            nome: segunda.0.to_owned(),
            arguments: segunda.1,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

fn gates_com_g9() -> JevGates {
    JevGates {
        g9: true,
        ..JevGates::default()
    }
}

fn motor_jev(jev: Arc<FakeJev>) -> Arc<JevEngine> {
    Arc::new(JevEngine::new(jev, gates_com_g9(), "jev-latest"))
}

fn escolhas(items: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
    items
        .iter()
        .map(|(id, choice)| ((*id).to_owned(), (*choice).to_owned()))
        .collect()
}

fn resultados(request: &Request) -> Vec<String> {
    request
        .messages
        .iter()
        .filter_map(|mensagem| match mensagem {
            Message::Resultado { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect()
}

fn offered(request: &Request) -> Vec<String> {
    request.tools.iter().map(|tool| tool.nome.clone()).collect()
}

async fn turno_com_fluxo(
    scenario: &Scenario,
    roteiros: Vec<Script>,
    conversa: ConversationId,
) -> (Arc<FakeProvider>, TurnSummary) {
    let jev = Arc::new(FakeJev::with_choices(escolhas(&[(
        "g9_iniciar_fluxo",
        "agendar_visita",
    )])));
    let (engine, provedor) =
        scenario.engine(roteiros, Some(motor_jev(jev)), Some(FlowsConfig::default()));
    let gatilho = scenario.falar(conversa, "queria marcar uma visita técnica");
    let summary = scenario.turno(&engine, conversa, gatilho).await;
    (provedor, summary)
}

#[tokio::test]
async fn acao_valida_avanca_e_a_resposta_diz_a_etapa_nova() {
    let scenario = Scenario::new("avanca");
    scenario.fluxo();
    let conversa = scenario.conversa();
    let (provedor, summary) = turno_com_fluxo(
        &scenario,
        vec![
            call(
                "fluxo_executar_acao",
                json!({"acao": "seguir", "campos": {"nome": "Ana Teste"}}),
            ),
            texto("Pronto, Ana! Visita confirmada."),
        ],
        conversa,
    )
    .await;
    assert_eq!(summary.estado, TurnState::Completed);

    let execution = scenario.execution(conversa);
    assert_eq!(execution.no_atual, "confirmacao");
    assert_eq!(execution.status, ExecutionStatus::Active);

    let pedidos = provedor.received();
    assert_eq!(
        pedidos.len(),
        2,
        "uma rodada de ferramenta e a resposta final"
    );
    assert!(
        offered(&pedidos[0]).contains(&"fluxo_executar_acao".to_owned()),
        "com execução ativa a ferramenta é oferecida: {:?}",
        offered(&pedidos[0])
    );
    let resultado = resultados(&pedidos[1])
        .into_iter()
        .next()
        .expect("o modelo recebeu o resultado da ferramenta");
    assert!(resultado.contains("avançou"), "{resultado}");
    assert!(resultado.contains("Confirmação"), "{resultado}");
    assert!(resultado.contains("`confirmar`"), "{resultado}");
    assert!(
        resultado.contains("agendou uma mensagem própria"),
        "a resposta avisa do efeito enfileirado: {resultado}"
    );

    let events = scenario.events(conversa);
    let acao = events
        .iter()
        .find(|evento| evento.tipo == RecordEventKind::ActionExecuted)
        .expect("há o evento acao_executada");
    assert_eq!(acao.ator_tipo, poph_types::EventActor::Agent);

    let pending = scenario.pending();
    assert_eq!(pending.len(), 1, "um efeito na fila");
    assert_eq!(pending[0].tipo, "mensagem_ao_cliente");
    let saida: Vec<_> = scenario
        .messages(conversa)
        .into_iter()
        .filter(|mensagem| mensagem.direction == Direction::Outbound)
        .collect();
    assert_eq!(
        saida.len(),
        1,
        "só a resposta do agente foi para a conversa"
    );
    assert!(saida[0].texto.contains("Pronto, Ana!"));

    let calls = scenario.calls(conversa);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["nome"], "fluxo_executar_acao");
    assert_eq!(calls[0]["estado"], "executada");
}

#[tokio::test]
async fn campo_faltando_devolve_a_lista_e_nada_muda() {
    let scenario = Scenario::new("campo");
    scenario.fluxo();
    let conversa = scenario.conversa();
    let (provedor, _) = turno_com_fluxo(
        &scenario,
        vec![
            call("fluxo_executar_acao", json!({"acao": "seguir"})),
            texto("Pode me dizer seu nome?"),
        ],
        conversa,
    )
    .await;

    let resultado = resultados(&provedor.received()[1])
        .into_iter()
        .next()
        .expect("o modelo recebeu a recusa");
    assert!(
        resultado.contains("exige o campo `nome`"),
        "a recusa diz qual campo falta: {resultado}"
    );
    assert!(
        resultado.contains("Campos obrigatórios que faltam: `nome`"),
        "e a descrição da etapa repete a lista: {resultado}"
    );

    let execution = scenario.execution(conversa);
    assert_eq!(execution.no_atual, "coleta");
    assert_eq!(execution.version, 1, "a versão não anda na recusa");
    assert!(
        scenario
            .events(conversa)
            .iter()
            .all(|evento| evento.tipo != RecordEventKind::ActionExecuted),
        "nenhum evento de ação gravado"
    );
    assert!(scenario.pending().is_empty(), "nenhum efeito enfileirado");
}

#[tokio::test]
async fn acao_inexistente_devolve_as_validas_e_nada_muda() {
    let scenario = Scenario::new("inexistente");
    scenario.fluxo();
    let conversa = scenario.conversa();
    let (provedor, _) = turno_com_fluxo(
        &scenario,
        vec![
            call("fluxo_executar_acao", json!({"acao": "cancelar_tudo"})),
            texto("Certo, não vou marcar nada."),
        ],
        conversa,
    )
    .await;

    let resultado = resultados(&provedor.received()[1])
        .into_iter()
        .next()
        .expect("o modelo recebeu a recusa");
    assert!(resultado.contains("`cancelar_tudo`"), "{resultado}");
    assert!(
        resultado.contains("válidas: `seguir`") || resultado.contains("Ações válidas: `seguir`"),
        "a recusa traz as ações válidas: {resultado}"
    );
    let execution = scenario.execution(conversa);
    assert_eq!(execution.version, 1);
    assert_eq!(execution.no_atual, "coleta");
}

#[tokio::test]
async fn ferramenta_desabilitada_no_agente_e_recusada_sem_executar() {
    let scenario = Scenario::new("desabilitada");
    scenario.habilitar(&["fluxo_abandonar"]);
    scenario.fluxo();
    let conversa = scenario.conversa();
    let (provedor, _) = turno_com_fluxo(
        &scenario,
        vec![
            call(
                "fluxo_executar_acao",
                json!({"acao": "seguir", "campos": {"nome": "Ana Teste"}}),
            ),
            texto("Sem problemas."),
        ],
        conversa,
    )
    .await;

    let names = offered(&provedor.received()[0]);
    assert!(
        !names.contains(&"fluxo_executar_acao".to_owned()),
        "só a habilitada é oferecida: {names:?}"
    );
    assert!(names.contains(&"fluxo_abandonar".to_owned()), "{names:?}");

    let resultado = resultados(&provedor.received()[1])
        .into_iter()
        .next()
        .expect("o modelo recebeu a recusa");
    assert!(resultado.contains("não está habilitada"), "{resultado}");

    let calls = scenario.calls(conversa);
    assert_eq!(calls[0]["estado"], "recusada");
    assert_eq!(calls[0]["motivo"], "nao_habilitada");

    let execution = scenario.execution(conversa);
    assert_eq!(execution.no_atual, "coleta", "nada mudou");
    assert_eq!(execution.version, 1);
}

#[tokio::test]
async fn fluxo_iniciar_so_aparece_na_reserva_e_valida_como_o_g9() {
    let scenario = Scenario::new("iniciar-nenhum");
    scenario.fluxo();
    let conversa = scenario.conversa();
    let jev = Arc::new(FakeJev::with_choices(escolhas(&[(
        "g9_iniciar_fluxo",
        G9_NONE,
    )])));
    let (engine, provedor) = scenario.engine(
        vec![
            call("fluxo_iniciar", json!({"fluxo_id": "agendar_visita"})),
            texto("Certo."),
        ],
        Some(motor_jev(jev)),
        Some(FlowsConfig::default()),
    );
    let gatilho = scenario.falar(conversa, "bom dia, tudo bem?");
    scenario.turno(&engine, conversa, gatilho).await;
    let names = offered(&provedor.received()[0]);
    assert!(
        !names.contains(&"fluxo_iniciar".to_owned()),
        "com o Jev respondendo, iniciar fluxo não é do modelo: {names:?}"
    );
    assert!(
        resultados(&provedor.received()[1])[0].contains("não é sua decisão"),
        "a chamada inventada é recusada"
    );
    assert!(scenario.active(conversa).is_none(), "nada iniciou");

    let scenario = Scenario::new("iniciar-reserva");
    scenario.fluxo();
    let conversa = scenario.conversa();
    let (engine, provedor) = scenario.engine(
        vec![
            call("fluxo_iniciar", json!({"fluxo_id": "agendar_visita"})),
            texto("Perfeito, vou agendar."),
        ],
        None,
        Some(FlowsConfig::default()),
    );
    let gatilho = scenario.falar(conversa, "queria marcar uma visita");
    scenario.turno(&engine, conversa, gatilho).await;
    let names = offered(&provedor.received()[0]);
    assert!(
        names.contains(&"fluxo_iniciar".to_owned()),
        "na reserva a ferramenta é oferecida: {names:?}"
    );
    let active = scenario
        .active(conversa)
        .expect("o modelo abriu a execução");
    assert_eq!(active.no_atual, "coleta");
    let resultado = resultados(&provedor.received()[1])
        .into_iter()
        .next()
        .expect("o modelo recebeu o estado da etapa de entrada");
    assert!(resultado.contains("iniciado"), "{resultado}");
    assert!(resultado.contains("`seguir`"), "{resultado}");
    let started = scenario
        .events(conversa)
        .into_iter()
        .find(|evento| evento.tipo == RecordEventKind::FlowStarted)
        .expect("evento fluxo_iniciado");
    assert_eq!(started.ator_tipo, poph_types::EventActor::Agent);

    let scenario = Scenario::new("iniciar-nao-liberado");
    scenario.criar_fluxo("confirmar_dados", "Confirmar dados", SCRIPT_B, true);
    scenario.criar_fluxo("agendar_visita", "Agendar visita", SCRIPT, false);
    let conversa = scenario.conversa();
    let (engine, provedor) = scenario.engine(
        vec![
            call("fluxo_iniciar", json!({"fluxo_id": "agendar_visita"})),
            texto("Não encontrei esse roteiro."),
        ],
        None,
        Some(FlowsConfig::default()),
    );
    let gatilho = scenario.falar(conversa, "queria marcar uma visita");
    scenario.turno(&engine, conversa, gatilho).await;
    let names = offered(&provedor.received()[0]);
    assert!(
        names.contains(&"fluxo_iniciar".to_owned()),
        "a reserva acontece pelo fluxo liberado: {names:?}"
    );
    let resultado = resultados(&provedor.received()[1])
        .into_iter()
        .next()
        .expect("o modelo recebeu a recusa");
    assert!(
        resultado.contains("não está entre os liberados"),
        "{resultado}"
    );
    assert!(scenario.active(conversa).is_none(), "nada iniciou");

    let scenario = Scenario::new("iniciar-inativo");
    let target = scenario.fluxo();
    scenario.desativar(target);
    scenario.criar_fluxo("confirmar_dados", "Confirmar dados", SCRIPT_B, true);
    let conversa = scenario.conversa();
    let (engine, provedor) = scenario.engine(
        vec![
            call("fluxo_iniciar", json!({"fluxo_id": "agendar_visita"})),
            texto("Esse roteiro está fora do ar."),
        ],
        None,
        Some(FlowsConfig::default()),
    );
    let gatilho = scenario.falar(conversa, "queria marcar uma visita");
    scenario.turno(&engine, conversa, gatilho).await;
    let resultado = resultados(&provedor.received()[1])
        .into_iter()
        .next()
        .expect("o modelo recebeu a recusa");
    assert!(
        resultado.contains("está inativo e não inicia execução nova"),
        "{resultado}"
    );
    assert!(scenario.active(conversa).is_none(), "nada iniciou");
}

#[tokio::test]
async fn chave_repetida_nao_duplica_evento_nem_efeito() {
    let scenario = Scenario::new("chave");
    scenario.fluxo();
    let conversa = scenario.conversa();
    let (_provedor, _) = turno_com_fluxo(
        &scenario,
        vec![
            call(
                "fluxo_executar_acao",
                json!({"acao": "seguir", "campos": {"nome": "Ana Teste"}, "chave_idempotencia": "acao-do-turno-1"}),
            ),
            texto("Certo."),
        ],
        conversa,
    )
    .await;
    let execution = scenario.execution(conversa);
    assert_eq!(execution.no_atual, "confirmacao");
    let eventos_antes = scenario.events(conversa).len();
    let efeitos_antes = scenario.pending().len();
    assert_eq!(efeitos_antes, 1, "o efeito da ação entrou uma vez");

    let (engine, provedor) = scenario.engine(
        vec![
            call(
                "fluxo_executar_acao",
                json!({"acao": "seguir", "campos": {"nome": "Ana Teste"}, "chave_idempotencia": "acao-do-turno-1"}),
            ),
            texto("De novo."),
        ],
        None,
        Some(FlowsConfig::default()),
    );
    let gatilho = scenario.falar(conversa, "como ficou?");
    scenario.turno(&engine, conversa, gatilho).await;
    let resultado = resultados(&provedor.received()[1])
        .into_iter()
        .next()
        .expect("o modelo recebeu a resposta da repetição");
    assert!(
        resultado.contains("já tinha sido registrada"),
        "{resultado}"
    );

    let execution = scenario.execution(conversa);
    assert_eq!(execution.no_atual, "confirmacao", "não andou de novo");
    assert_eq!(execution.version, 2, "a versão ficou como estava");
    assert_eq!(scenario.pending().len(), efeitos_antes, "sem efeito novo");
    assert_eq!(
        scenario.events(conversa).len(),
        eventos_antes,
        "sem evento novo"
    );
}

#[tokio::test]
async fn limite_de_chamadas_por_mensagem_continua_valendo() {
    let scenario = Scenario::new("limite");
    scenario.ajustar(AgentChange {
        max_tools_per_message: Some(1),
        ..AgentChange::default()
    });
    scenario.fluxo();
    let conversa = scenario.conversa();
    let (provedor, _) = turno_com_fluxo(
        &scenario,
        vec![
            duas_chamadas(
                (
                    "fluxo_executar_acao",
                    json!({"acao": "seguir", "campos": {"nome": "Ana Teste"}}),
                ),
                ("fluxo_abandonar", json!({"motivo": "cliente desistiu"})),
            ),
            texto("Certo."),
        ],
        conversa,
    )
    .await;

    let resultados = resultados(&provedor.received()[1]);
    assert!(
        resultados
            .iter()
            .any(|texto| texto.contains("limite de 1 chamadas")),
        "a segunda chamada é recusada pelo teto: {resultados:?}"
    );
    let calls = scenario.calls(conversa);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1]["estado"], "recusada");
    assert_eq!(calls[1]["motivo"], "limite_de_chamadas");

    let execution = scenario.execution(conversa);
    assert_eq!(execution.no_atual, "confirmacao");
}

#[tokio::test]
async fn abandonar_grava_motivo_e_nao_gera_efeito() {
    let scenario = Scenario::new("abandonar");
    scenario.fluxo();
    let conversa = scenario.conversa();
    let (provedor, _) = turno_com_fluxo(
        &scenario,
        vec![
            call(
                "fluxo_abandonar",
                json!({"motivo": "cliente desistiu da visita"}),
            ),
            texto("Sem problemas, fico à disposição."),
        ],
        conversa,
    )
    .await;

    let resultado = resultados(&provedor.received()[1])
        .into_iter()
        .next()
        .expect("o modelo recebeu a confirmação");
    assert!(resultado.contains("abandonado"), "{resultado}");

    let execution = scenario.execution(conversa);
    assert_eq!(execution.status, ExecutionStatus::Abandoned);
    assert_eq!(
        execution.motivo_fim.as_deref(),
        Some("cliente desistiu da visita")
    );
    assert!(
        scenario.active(conversa).is_none(),
        "não há mais execução ativa"
    );
    assert!(scenario.pending().is_empty(), "abandono não gera efeito");
    let evento = scenario
        .events(conversa)
        .into_iter()
        .find(|evento| evento.tipo == RecordEventKind::ExecutionAbandoned)
        .expect("evento de abandono");
    assert_eq!(evento.ator_tipo, poph_types::EventActor::Agent);
}

#[tokio::test]
async fn acao_que_encerra_o_atendimento_fecha_a_ficha() {
    let scenario = Scenario::new("encerra");
    scenario.fluxo();
    let conversa = scenario.conversa();
    let (provedor, _) = turno_com_fluxo(
        &scenario,
        vec![
            call(
                "fluxo_executar_acao",
                json!({"acao": "seguir", "campos": {"nome": "Ana Teste"}}),
            ),
            call("fluxo_executar_acao", json!({"acao": "confirmar"})),
            texto("Visita confirmada, até lá!"),
        ],
        conversa,
    )
    .await;

    let resultado = resultados(&provedor.received()[2])
        .into_iter()
        .last()
        .expect("o modelo recebeu o desfecho");
    assert!(resultado.contains("fim"), "{resultado}");

    let ficha = scenario.ficha(conversa);
    assert_eq!(ficha.status, RecordStatus::Closed);
    assert_eq!(ficha.fechada_por, Some(ClosedBy::Flow));
    let conversa_gravada = scenario
        .store
        .conversations()
        .get(scenario.org, conversa)
        .expect("lê a conversa");
    assert_eq!(conversa_gravada.status, ConversationStatus::Closed);
    let execution = scenario.execution(conversa);
    assert_eq!(execution.status, ExecutionStatus::Completed);
    assert_eq!(execution.result.as_deref(), Some("Visita agendada"));
}

#[tokio::test]
async fn motor_desligado_nao_registra_nem_oferece_as_ferramentas() {
    let scenario = Scenario::new("desligado");
    scenario.fluxo();
    let conversa = scenario.conversa();
    let (engine, provedor) = scenario.engine(
        vec![texto("Oi! Como posso ajudar?")],
        None,
        Some(FlowsConfig {
            enabled: false,
            ..FlowsConfig::default()
        }),
    );
    let gatilho = scenario.falar(conversa, "oi");
    scenario.turno(&engine, conversa, gatilho).await;
    let names = offered(&provedor.received()[0]);
    for tool in TODAS {
        assert!(
            !names.contains(&tool.to_owned()),
            "`{tool}` não é oferecida com o motor desligado: {names:?}"
        );
    }
    let registro = poph_agent::turno::registrar_ferramentas_do_fluxo(
        poph_agent::turno::DefaultRegistry::new(),
        Arc::clone(&scenario.store),
        &FlowsConfig {
            enabled: false,
            ..FlowsConfig::default()
        },
    );
    assert!(
        registro.names().is_empty(),
        "motor desligado não registra ferramenta de fluxo: {:?}",
        registro.names()
    );
    let ligado = poph_agent::turno::registrar_ferramentas_do_fluxo(
        poph_agent::turno::DefaultRegistry::new(),
        Arc::clone(&scenario.store),
        &FlowsConfig::default(),
    );
    assert_eq!(ligado.names().len(), 3, "{:?}", ligado.names());

    assert_eq!(scenario.ficha(conversa).status, RecordStatus::Open);
}

#[tokio::test]
async fn encerrar_pelo_agente_fecha_e_repeticao_e_idempotente() {
    let c = Scenario::new("fx25-fechar");
    c.habilitar(&["atendimento_encerrar"]);
    c.fluxo();
    let conversa = c.conversa();
    let (provedor, summary) = turno_com_fluxo(
        &c,
        vec![
            duas_chamadas(
                ("atendimento_encerrar", json!({"motivo":"resolvido"})),
                ("atendimento_encerrar", json!({"motivo":"resolvido"})),
            ),
            texto("Até logo!"),
        ],
        conversa,
    )
    .await;
    assert_eq!(summary.estado, TurnState::Completed);
    assert!(offered(&provedor.received()[0]).contains(&"atendimento_encerrar".to_owned()));
    let respostas = resultados(&provedor.received()[1]);
    assert!(
        respostas.iter().any(|s| s.contains("já encerrado")),
        "{respostas:?}"
    );
    assert_eq!(
        c.store.conversations().get(c.org, conversa).unwrap().status,
        ConversationStatus::Closed
    );
    let ficha = c.ficha(conversa);
    assert_eq!(ficha.status, RecordStatus::Closed);
    assert_eq!(ficha.fechada_por, Some(ClosedBy::Agent));
    assert_eq!(ficha.fechada_motivo.as_deref(), Some("resolvido"));
    assert_eq!(c.execution(conversa).status, ExecutionStatus::Abandoned);
    assert_eq!(
        c.events(conversa)
            .iter()
            .filter(|e| e.tipo == RecordEventKind::RecordClosed)
            .count(),
        1
    );
    assert_eq!(
        c.messages(conversa)
            .iter()
            .filter(|m| m.direction == Direction::Outbound)
            .count(),
        1
    );
    let conn = rusqlite::Connection::open(c.folder.join("harness.db")).unwrap();
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM audit_log WHERE action='close' AND entity_id=?1",
            [conversa.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn encerramento_desabilitado_e_recusado() {
    let c = Scenario::new("fx25-desligada");
    c.habilitar(&[]);
    let conversa = c.conversa();
    let (provedor, _) = turno_com_fluxo(
        &c,
        vec![
            call("atendimento_encerrar", json!({"motivo":"resolvido"})),
            texto("Até logo!"),
        ],
        conversa,
    )
    .await;
    assert!(!offered(&provedor.received()[0]).contains(&"atendimento_encerrar".to_owned()));
    assert_eq!(c.calls(conversa)[0]["motivo"], "nao_habilitada");
    assert_eq!(c.ficha(conversa).status, RecordStatus::Open);
}
