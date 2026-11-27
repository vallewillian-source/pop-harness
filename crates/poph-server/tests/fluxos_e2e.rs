use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsG9, MetricsMemory, MetricsTurn};
use poph_agent::turno::{
    FuturePause, NoopObserver, Pause, TurnEngine, TurnInput, TurnOptions, TurnSummary,
};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, Script, ToolCall, Usage};
use poph_flows::{Limites, compile};
use poph_jev::{FakeJev, JevEngine};
use poph_store::{
    ConversationStatus, Direction, NewAgent, NewConversation, NewFlow, NewMessage, NewOrg,
    RevisionPublication, Role, Store, TurnState,
};
use poph_types::{AgentId, ConversationId, FlowsConfig, JevGates, MessageId, OrgId, RecordStatus};
use serde_json::{Value, json};

struct NoWait;

impl Pause for NoWait {
    fn wait(&self, _duration: Duration) -> FuturePause {
        Box::pin(async {})
    }
}

const ALL: [&str; 3] = [
    poph_agent::turno::NAME_RUN_ACTION,
    poph_agent::turno::NAME_ABANDON,
    poph_agent::turno::NAME_TOOL_START,
];

struct Scenario {
    dir: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
}

impl Scenario {
    fn fresh(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "poph-fx24-{label}-{}-{}",
            std::process::id(),
            AgentId::generate()
        ));
        std::fs::create_dir_all(&dir).expect("pasta do teste");
        let store = Arc::new(
            Store::open_at(&dir.join("harness.db"), &dir.join("rag.db")).expect("abre o banco"),
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
            dir,
            store,
            org,
            agente,
        };
        scenario.habilitar(&ALL);
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
        let expected_value = match current {
            Ok(Some(versao)) => versao,
            _ => 0,
        };
        self.store
            .instructions()
            .save(
                self.org,
                self.agente,
                expected_value,
                &structured,
                "# Bia\n\nVocê é a atendente da FixProLar.",
                true,
            )
            .expect("grava as instruções");
    }

    fn create_flow(
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
        let compiled = compile(script, &Limites::default());
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

    fn speak(&self, conversa: ConversationId, texto: &str) -> MessageId {
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
        scripts: Vec<Script>,
        jev: Option<Arc<JevEngine>>,
        fluxos: Option<FlowsConfig>,
    ) -> (TurnEngine, Arc<FakeProvider>) {
        let config = ConfigHot::new(
            Arc::clone(&self.store),
            HotConfigOptions::default(),
            MetricsMemory::own(),
        );
        let limiter = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
        let provedor = Arc::new(FakeProvider::with_scripts(scripts));
        let mut engine = TurnEngine::new(
            Arc::clone(&self.store),
            config,
            limiter,
            provedor.clone(),
            TurnOptions::default(),
            MetricsTurn::own(),
        )
        .with_pause(Arc::new(NoWait))
        .with_summary(poph_agent::summary::SummaryOptions {
            ativo: false,
            ..poph_agent::summary::SummaryOptions::default()
        })
        .with_g9_metrics(MetricsG9::own());
        let registro = poph_agent::turno::register_flow_tools(
            poph_agent::turno::DefaultRegistry::new(),
            Arc::clone(&self.store),
            &fluxos.clone().unwrap_or_default(),
        );
        let registro = poph_agent::turno::register_closing_tool(registro, Arc::clone(&self.store));
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

    fn execution(&self, conversa: ConversationId) -> poph_store::Execution {
        self.store
            .executions()
            .from_case(self.org, self.ficha(conversa).id, 20)
            .expect("lê as execuções")
            .into_iter()
            .next()
            .expect("a Ficha tem execução")
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
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

fn gates_with_g9() -> JevGates {
    JevGates {
        g9: true,
        ..JevGates::default()
    }
}

fn jev_engine(jev: Arc<FakeJev>) -> Arc<JevEngine> {
    Arc::new(JevEngine::new(jev, gates_with_g9(), "jev-latest"))
}

fn choices(items: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
    items
        .iter()
        .map(|(id, choice)| ((*id).to_owned(), (*choice).to_owned()))
        .collect()
}

const VISIT: &str = include_str!("../../../tests/fluxos/exemplos/agendar_visita.star");
const TRIAGEM: &str = include_str!("../../../tests/fluxos/exemplos/triagem_suporte.star");
const DELIVERY: &str = include_str!("../../../tests/fluxos/exemplos/confirmar_entrega.star");
const TOKEN: &str = "pophg_fx35_ficticio";

async fn api(c: &Scenario) -> String {
    use argon2::password_hash::{PasswordHasher, SaltString};
    let sal = SaltString::generate(&mut rand_core::OsRng);
    let hash = argon2::Argon2::default()
        .hash_password(TOKEN.as_bytes(), &sal)
        .unwrap()
        .to_string();
    let mut config = poph_types::HarnessConfig::default();
    config.admin.tokens_globais = vec![poph_types::config::GlobalTokenConfig {
        label: "teste".into(),
        hash,
    }];
    let estado = poph_server::ServerState::new(Arc::clone(&c.store), config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endereco = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = poph_server::serve_at(listener, estado, std::future::pending::<()>()).await;
    });
    format!("http://{endereco}")
}

async fn get_api(base: &str, c: &Scenario, path: &str) -> Value {
    let url = format!("{base}/admin/v1/orgs/{}/fichas/{}", c.org, path);
    let reply = reqwest::Client::new()
        .get(url)
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert!(reply.status().is_success(), "{}", reply.status());
    reply.json().await.unwrap()
}

async fn run(
    c: &Scenario,
    conversa: ConversationId,
    slug: &str,
    scripts: Vec<Script>,
) -> Arc<FakeProvider> {
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g9_iniciar_fluxo",
        slug,
    )])));
    let (engine, provedor) = c.engine(scripts, Some(jev_engine(jev)), Some(FlowsConfig::default()));
    let mensagem = c.speak(conversa, "Pedido fictício para teste");
    assert_eq!(
        c.turno(&engine, conversa, mensagem).await.estado,
        TurnState::Completed
    );
    provedor
}

#[tokio::test]
async fn example_visit_with_jev_fields_and_http_history() {
    let c = Scenario::fresh("fx35-visita");
    c.create_flow("agendar_visita", "Agendar visita", VISIT, true);
    let conversa = c.conversa();
    let provedor = run(
        &c,
        conversa,
        "agendar_visita",
        vec![
            call(
                "fluxo_executar_acao",
                json!({"acao":"confirmar","campos":{"endereco":"Rua Exemplo 1","periodo":"tarde"}}),
            ),
            call("fluxo_executar_acao", json!({"acao":"ok","campos":{}})),
            texto("Visita combinada."),
        ],
    )
    .await;
    let requests = provedor.received();
    assert!(format!("{:?}", requests[0].messages).contains("Coletar dados"));
    let base = api(&c).await;
    let ficha = c.ficha(conversa);
    let body = get_api(&base, &c, &ficha.id.to_string()).await;
    assert_eq!(body["status"], "open");
    let run = &body["execucoes"][0];
    assert_eq!(run["status"], "concluida", "{body}");
    assert_eq!(run["resultado"], "Visita agendada");
    assert_eq!(run["etapas"][0]["no_id"], "coleta");
    assert_eq!(run["etapas"][0]["dados"]["endereco"], "Rua Exemplo 1");
    assert_eq!(run["etapas"][0]["dados"]["periodo"], "tarde");
    assert_eq!(run["etapas"][0]["atores_que_agiram"], json!(["agente"]));
    assert_eq!(run["etapas"][1]["no_id"], "confirmacao");
    let events = get_api(&base, &c, &format!("{}/events", ficha.id)).await;
    assert!(events["items"].as_array().unwrap().len() >= 3, "{events}");
}

#[tokio::test]
async fn example_triage_exercises_both_branches() {
    for (label, summary, afeta, expected) in [
        ("grave", "sistema nao abre", "sim", "urgente"),
        ("leve", "duvida simples", "sim", "normal"),
        ("sem-impacto", "sistema nao abre", "nao", "normal"),
    ] {
        let c = Scenario::fresh(label);
        c.create_flow("triagem_suporte", "Triagem de suporte", TRIAGEM, true);
        let conversa = c.conversa();
        run(
            &c,
            conversa,
            "triagem_suporte",
            vec![
                call(
                    "fluxo_executar_acao",
                    json!({"acao":"registrado","campos":{"resumo":summary,"afeta_agora":afeta}}),
                ),
                texto("Entendido."),
            ],
        )
        .await;
        assert_eq!(c.execution(conversa).no_atual, expected);
        let base = api(&c).await;
        let ficha = c.ficha(conversa);
        let events = get_api(&base, &c, &format!("{}/events", ficha.id)).await;
        assert!(events.to_string().contains("decisao_resolvida"), "{events}");
    }
}

async fn fake_evolution() -> (String, Arc<std::sync::Mutex<Vec<Value>>>) {
    use axum::{Json, Router, extract::State, routing::post};
    let sends = Arc::new(std::sync::Mutex::new(Vec::new()));
    async fn send(
        State(sends): State<Arc<std::sync::Mutex<Vec<Value>>>>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        sends.lock().unwrap().push(body);
        Json(json!({"key":{"id":"ficticio"}}))
    }
    let router = Router::new()
        .route("/message/sendText/{instancia}", post(send))
        .with_state(Arc::clone(&sends));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endereco = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{endereco}"), sends)
}

#[tokio::test]
async fn example_single_notification_and_transfer_with_open_case() {
    use poph_evolution::{ATTACHMENT_CAP, ConfigEvolution, EvolutionClient};
    use poph_server::{
        evolution::EvolutionChannel,
        flow_effects::{FlowEffectOptions, FlowEffects},
        whatsapp_output::{WhatsAppOutput, WhatsAppOutputOptions},
    };
    use poph_types::{IntegrationId, Secret};
    let c = Scenario::fresh("efeitos-fx35");
    c.create_flow("confirmar_entrega", "Confirmar entrega", DELIVERY, true);
    c.store
        .departments()
        .create(
            c.org,
            poph_store::NewDepartment {
                name: "Logística".into(),
                description: String::new(),
            },
        )
        .unwrap();
    let integracao: IntegrationId = c
        .store
        .create_integration(c.org, c.agente, "whatsapp".into(), "active".into())
        .unwrap()
        .id
        .parse()
        .unwrap();
    let (url, sends) = fake_evolution().await;
    let canal = EvolutionChannel {
        org: c.org,
        instance: "teste".into(),
        integration: integracao,
        secret: None,
        client: Some(
            EvolutionClient::new(
                ConfigEvolution::new(url, "teste", Secret::new("ficticia".into())).unwrap(),
            )
            .unwrap(),
        ),
        attachment_limit: ATTACHMENT_CAP,
    };
    let channels = vec![canal];
    let effects = FlowEffects::new(
        Arc::clone(&c.store),
        &channels,
        c.dir.join("blobs"),
        None,
        FlowEffectOptions::default(),
    );
    let saida = WhatsAppOutput::new(
        Arc::clone(&c.store),
        &channels,
        c.dir.join("blobs"),
        WhatsAppOutputOptions::default(),
    );
    let conversa = |peer: &str| {
        c.store
            .conversations()
            .create(
                c.org,
                NewConversation {
                    agent_id: c.agente,
                    integration_id: Some(integracao),
                    canal: "whatsapp".into(),
                    peer_externo: Some(peer.into()),
                    target_department: None,
                },
            )
            .unwrap()
            .id
    };
    let first = conversa("5511999990001");
    run(
        &c,
        first,
        "confirmar_entrega",
        vec![
            call(
                "fluxo_executar_acao",
                json!({"acao":"localizado","campos":{"pedido":"1234"}}),
            ),
            texto("Vou verificar."),
        ],
    )
    .await;
    effects.cycle_for_org(c.org).await.unwrap();
    saida.cycle().await.unwrap();
    run(
        &c,
        first,
        "confirmar_entrega",
        vec![
            call(
                "fluxo_executar_acao",
                json!({"acao":"confirmada","campos":{"recebido":"sim"}}),
            ),
            texto("Concluído."),
        ],
    )
    .await;
    effects.cycle_for_org(c.org).await.unwrap();
    saida.cycle().await.unwrap();
    effects.cycle_for_org(c.org).await.unwrap();
    saida.cycle().await.unwrap();
    let sent_items = sends.lock().unwrap().clone();
    assert_eq!(
        sent_items
            .iter()
            .filter(|v| v["number"] == "5521999990000")
            .count(),
        1,
        "{sent_items:?}"
    );
    assert!(
        sent_items
            .iter()
            .any(|v| v["text"].as_str().unwrap_or("").contains("pedido 1234")),
        "{sent_items:?}"
    );
    let second = conversa("5511999990002");
    run(
        &c,
        second,
        "confirmar_entrega",
        vec![
            call(
                "fluxo_executar_acao",
                json!({"acao":"localizado","campos":{"pedido":"5678"}}),
            ),
            call(
                "fluxo_executar_acao",
                json!({"acao":"nao_recebida","campos":{"recebido":"nao"}}),
            ),
            call(
                "fluxo_executar_acao",
                json!({"acao":"registrado","campos":{}}),
            ),
            texto("Encaminhado."),
        ],
    )
    .await;
    effects.cycle_for_org(c.org).await.unwrap();
    assert_eq!(
        c.store.conversations().get(c.org, second).unwrap().status,
        ConversationStatus::Transferred
    );
    assert_eq!(c.ficha(second).status, RecordStatus::Open);
    let base = api(&c).await;
    let events = get_api(&base, &c, &format!("{}/events", c.ficha(second).id)).await;
    assert!(
        events.to_string().contains("atendimento_transferido"),
        "{events}"
    );
}

#[tokio::test]
async fn old_revision_human_action_and_two_flows_in_same_case() {
    let c = Scenario::fresh("fx35-sequencia");
    let human_visit = VISIT.replace(
        "nome = \"Confirmar visita\",",
        "nome = \"Confirmar visita\",\n    atores = [\"agente\", \"atendente\"],",
    );
    let fluxo = c.create_flow("agendar_visita", "Agendar visita", &human_visit, true);
    c.create_flow("triagem_suporte", "Triagem de suporte", TRIAGEM, true);
    let conversa = c.conversa();
    run(
        &c,
        conversa,
        "agendar_visita",
        vec![
            call(
                "fluxo_executar_acao",
                json!({"acao":"confirmar","campos":{"endereco":"Rua Teste","periodo":"manhã"}}),
            ),
            texto("Confirme."),
        ],
    )
    .await;
    let previous = c.execution(conversa);
    let fresh_source = human_visit.replace("Visita agendada", "Resultado da versão nova");
    let compiled = compile(&fresh_source, &Limites::default());
    let ir = compiled.ir().unwrap();
    let versao = c.store.flows().get(c.org, fluxo).unwrap().version;
    let new_item = c
        .store
        .revisions()
        .publish_revision(
            c.org,
            fluxo,
            RevisionPublication {
                expected: versao,
                fonte: fresh_source,
                definicao_json: ir.to_json(),
                schema_versao: Some(ir.schema_versao),
                resolucoes_json: "{}".into(),
                ator_tipo: "system".into(),
                actor_id: None,
            },
        )
        .unwrap();
    assert_ne!(new_item.id, previous.revisao_id);
    let base = api(&c).await;
    let ficha = c.ficha(conversa);
    let url = format!(
        "{base}/admin/v1/orgs/{}/fichas/{}/flow-runs/{}/actions",
        c.org, ficha.id, previous.id
    );
    let reply = reqwest::Client::new()
        .post(url)
        .bearer_auth(TOKEN)
        .header("If-Match", previous.version.to_string())
        .json(&json!({"acao":"ok", "campos":{}, "chave_idempotencia":"humano-fx35"}))
        .send()
        .await
        .unwrap();
    let status = reply.status();
    let body: Value = reply.json().await.unwrap();
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    assert_eq!(body["resultado"], "Visita agendada");
    run(
        &c,
        conversa,
        "triagem_suporte",
        vec![
            call(
                "fluxo_executar_acao",
                json!({"acao":"registrado","campos":{"resumo":"erro simples","afeta_agora":"nao"}}),
            ),
            call(
                "fluxo_executar_acao",
                json!({"acao":"encaminhado","campos":{}}),
            ),
            texto("Feito."),
        ],
    )
    .await;
    let history = get_api(&base, &c, &ficha.id.to_string()).await;
    let runs = history["execucoes"].as_array().unwrap();
    assert_eq!(runs.len(), 2, "{history}");
    let visit = runs
        .iter()
        .find(|r| r["id"] == previous.id.to_string())
        .unwrap();
    assert_eq!(visit["revisao_id"], previous.revisao_id.to_string());
    assert_eq!(visit["resultado"], "Visita agendada");
    assert_eq!(
        visit["etapas"][1]["atores_que_agiram"],
        json!(["atendente"])
    );
    assert!(runs.iter().any(|r| r["resultado"] == "Normal"));
    let events = get_api(&base, &c, &format!("{}/events", ficha.id)).await;
    assert!(events.to_string().contains("atendente"), "{events}");
}

#[tokio::test]
async fn jev_failure_reserve_starts_allowed_and_refuses_not_released() {
    let c = Scenario::fresh("fx35-reserva");
    let allowed = c.create_flow("agendar_visita", "Agendar visita", VISIT, true);
    c.create_flow("triagem_suporte", "Triagem de suporte", TRIAGEM, false);
    let conversa = c.conversa();
    let jev = Arc::new(FakeJev::failing(poph_jev::JevError::Network(
        "indisponível no teste".into(),
    )));
    let (engine, provedor) = c.engine(
        vec![
            call("fluxo_iniciar", json!({"fluxo_id":"triagem_suporte"})),
            call("fluxo_iniciar", json!({"fluxo_id":"agendar_visita"})),
            texto("Certo."),
        ],
        Some(jev_engine(jev)),
        Some(FlowsConfig::default()),
    );
    let mensagem = c.speak(conversa, "Quero agendar");
    c.turno(&engine, conversa, mensagem).await;
    let requests = provedor.received();
    assert!(requests[0].tools.iter().any(|f| f.nome == "fluxo_iniciar"));
    assert!(!format!("{:?}", requests[0].messages).contains("triagem_suporte"));
    assert!(format!("{:?}", requests[1].messages).contains("não está entre os liberados"));
    assert_eq!(c.execution(conversa).no_atual, "coleta");
    let runs = c
        .store
        .executions()
        .from_case(c.org, c.ficha(conversa).id, 20)
        .unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].fluxo_id, allowed);
}

#[tokio::test]
async fn agent_closing_and_next_message_open_new_case() {
    let c = Scenario::fresh("fx35-encerrar");
    c.habilitar(&["atendimento_encerrar"]);
    let integracao = c
        .store
        .create_integration(c.org, c.agente, "whatsapp".into(), "active".into())
        .unwrap()
        .id
        .parse()
        .unwrap();
    let input = |id: &str| poph_store::NewChannelInboxItem {
        integration_id: integracao,
        agent_id: c.agente,
        canal: "whatsapp".into(),
        external_id: id.into(),
        peer: "5511999990001".into(),
        event_json: "{}".into(),
        texto: "Olá".into(),
        attachments: vec![],
        customer_name: None,
    };
    let mensagem = c
        .store
        .inbox()
        .receive_from_channel(c.org, input("fx35-1"))
        .unwrap()
        .mensagem;
    let conversa = mensagem.conversation_id;
    let ficha = c.ficha(conversa);
    let (engine, _) = c.engine(
        vec![
            call("atendimento_encerrar", json!({"motivo":"resolvido"})),
            texto("Até logo."),
        ],
        None,
        Some(FlowsConfig::default()),
    );
    c.turno(&engine, conversa, mensagem.id).await;
    assert_eq!(c.ficha(conversa).status, RecordStatus::Closed);
    let next = c
        .store
        .inbox()
        .receive_from_channel(c.org, input("fx35-2"))
        .unwrap()
        .mensagem;
    assert_ne!(next.conversation_id, conversa);
    assert_ne!(c.ficha(next.conversation_id).id, ficha.id);
    let base = api(&c).await;
    assert_eq!(
        get_api(&base, &c, &ficha.id.to_string()).await["status"],
        "closed"
    );
}

#[tokio::test]
async fn turn_counters_count_one_open_and_one_completion() {
    let c = Scenario::fresh("fx40-turno");
    let registro = poph_types::Registry::new();
    let metrics = poph_server::flow_metrics::FlowMetrics::register(&registro)
        .expect("o catálogo de fluxos registra");
    c.store
        .publish_execution_metrics(metrics.execution_metrics());

    c.create_flow("agendar_visita", "Agendar visita", VISIT, true);
    let conversa = c.conversa();
    run(
        &c,
        conversa,
        "agendar_visita",
        vec![
            call(
                "fluxo_executar_acao",
                json!({"acao":"confirmar","campos":{"endereco":"Rua Exemplo 1","periodo":"tarde"}}),
            ),
            call("fluxo_executar_acao", json!({"acao":"ok","campos":{}})),
            texto("Visita combinada."),
        ],
    )
    .await;

    assert_eq!(
        metrics.started_executions.value(),
        1,
        "o turno abriu uma execução (G9 antes do modelo)"
    );
    assert_eq!(
        metrics.completed_executions.value(),
        1,
        "as ações do agente concluíram a mesma execução"
    );
    assert_eq!(metrics.abandoned_executions.value(), 0);
    assert_eq!(metrics.failed_executions.value(), 0);
    assert_eq!(metrics.rule_errors.value(), 0);
}
