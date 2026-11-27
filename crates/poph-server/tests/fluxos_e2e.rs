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

struct SemEspera;

impl Pause for SemEspera {
    fn wait(&self, _duracao: Duration) -> FuturePause {
        Box::pin(async {})
    }
}

/// As três ferramentas do fluxo, como o agente as habilita (D-04).
const TODAS: [&str; 3] = [
    poph_agent::turno::NAME_RUN_ACTION,
    poph_agent::turno::NAME_ABANDON,
    poph_agent::turno::NAME_TOOL_START,
];

/// Banco, org, agente e conversa de verdade numa pasta temporária.
struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-fx24-{rotulo}-{}-{}",
            std::process::id(),
            AgentId::generate()
        ));
        std::fs::create_dir_all(&pasta).expect("pasta do teste");
        let store = Arc::new(
            Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre o banco"),
        );
        let org = store
            .orgs()
            .create(NewOrg {
                name: format!("Org {rotulo}"),
                slug: format!("org-{rotulo}-{}", AgentId::generate()),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: format!("Bia {rotulo}"),
                    slug: format!("bia-{rotulo}"),
                    grouping_window_ms: 0,
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        let cenario = Self {
            pasta,
            store,
            org,
            agente,
        };
        cenario.habilitar(&TODAS);
        cenario
    }

    /// Grava as instruções com as ferramentas habilitadas informadas (D-04).
    fn habilitar(&self, ferramentas: &[&str]) {
        let estruturado = json!({
            "tools": ferramentas
                .iter()
                .map(|nome| json!({ "name": nome, "enabled": true }))
                .collect::<Vec<_>>()
        })
        .to_string();
        let atual = self.store.instructions().version(self.org, self.agente);
        let esperada = match atual {
            Ok(Some(versao)) => versao,
            _ => 0,
        };
        self.store
            .instructions()
            .save(
                self.org,
                self.agente,
                esperada,
                &estruturado,
                "# Bia\n\nVocê é a atendente da FixProLar.",
                true,
            )
            .expect("grava as instruções");
    }

    /// Cria um fluxo com o script dado, publica a revisão e (opcionalmente) libera ao agente.
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
        let compilado = compile(script, &Limites::default());
        let ir = compilado.ir().expect("o script do teste compila");
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

    fn motor(
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
        let limitador = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
        let provedor = Arc::new(FakeProvider::with_scripts(roteiros));
        let mut motor = TurnEngine::new(
            Arc::clone(&self.store),
            config,
            limitador,
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
        // FX-24: as ferramentas do fluxo entram no registro do turno, como na montagem real.
        let registro = poph_agent::turno::register_flow_tools(
            poph_agent::turno::DefaultRegistry::new(),
            Arc::clone(&self.store),
            &fluxos.clone().unwrap_or_default(),
        );
        let registro = poph_agent::turno::register_closing_tool(registro, Arc::clone(&self.store));
        motor = motor.with_tools(Arc::new(registro));
        if let Some(ajustes) = fluxos {
            motor = motor.with_flows(&ajustes);
        }
        if let Some(jev) = jev {
            motor = motor.with_jev(jev);
        }
        (motor, provedor)
    }

    async fn turno(
        &self,
        motor: &TurnEngine,
        conversa: ConversationId,
        gatilho: MessageId,
    ) -> TurnSummary {
        motor
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

    fn execucao(&self, conversa: ConversationId) -> poph_store::Execution {
        self.store
            .executions()
            .from_case(self.org, self.ficha(conversa).id, 20)
            .expect("lê as execuções")
            .into_iter()
            .next()
            .expect("a Ficha tem execução")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

// ───────────────────────────────────────────────────────────────────────── utilidades

fn texto(conteudo: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(conteudo.to_owned()),
        LlmEvent::Usage(Usage {
            input: 10,
            saida: 5,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

/// Roteiro do modelo que pede uma única ferramenta.
fn chamada(nome: &str, argumentos: Value) -> Script {
    Script::Events(vec![
        LlmEvent::Tool(ToolCall {
            id: "chamada-1".to_owned(),
            nome: nome.to_owned(),
            arguments: argumentos,
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

fn escolhas(itens: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
    itens
        .iter()
        .map(|(id, escolha)| ((*id).to_owned(), (*escolha).to_owned()))
        .collect()
}

const VISITA: &str = include_str!("../../../tests/fluxos/exemplos/agendar_visita.star");
const TRIAGEM: &str = include_str!("../../../tests/fluxos/exemplos/triagem_suporte.star");
const ENTREGA: &str = include_str!("../../../tests/fluxos/exemplos/confirmar_entrega.star");
const TOKEN: &str = "pophg_fx35_ficticio";

async fn api(c: &Cenario) -> String {
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
    let estado = poph_server::Estado::novo(Arc::clone(&c.store), config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endereco = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = poph_server::serve_at(listener, estado, std::future::pending::<()>()).await;
    });
    format!("http://{endereco}")
}

async fn get_api(base: &str, c: &Cenario, caminho: &str) -> Value {
    let url = format!("{base}/admin/v1/orgs/{}/fichas/{}", c.org, caminho);
    let resposta = reqwest::Client::new()
        .get(url)
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert!(resposta.status().is_success(), "{}", resposta.status());
    resposta.json().await.unwrap()
}

async fn executar(
    c: &Cenario,
    conversa: ConversationId,
    slug: &str,
    roteiros: Vec<Script>,
) -> Arc<FakeProvider> {
    let jev = Arc::new(FakeJev::with_choices(escolhas(&[(
        "g9_iniciar_fluxo",
        slug,
    )])));
    let (motor, provedor) = c.motor(roteiros, Some(motor_jev(jev)), Some(FlowsConfig::default()));
    let mensagem = c.falar(conversa, "Pedido fictício para teste");
    assert_eq!(
        c.turno(&motor, conversa, mensagem).await.estado,
        TurnState::Completed
    );
    provedor
}

#[tokio::test]
async fn exemplo_visita_com_jev_campos_e_historico_http() {
    let c = Cenario::novo("fx35-visita");
    c.criar_fluxo("agendar_visita", "Agendar visita", VISITA, true);
    let conversa = c.conversa();
    let provedor = executar(
        &c,
        conversa,
        "agendar_visita",
        vec![
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"confirmar","campos":{"endereco":"Rua Exemplo 1","periodo":"tarde"}}),
            ),
            chamada("fluxo_executar_acao", json!({"acao":"ok","campos":{}})),
            texto("Visita combinada."),
        ],
    )
    .await;
    let pedidos = provedor.received();
    assert!(format!("{:?}", pedidos[0].messages).contains("Coletar dados"));
    let base = api(&c).await;
    let ficha = c.ficha(conversa);
    let corpo = get_api(&base, &c, &ficha.id.to_string()).await;
    assert_eq!(corpo["status"], "open");
    let run = &corpo["execucoes"][0];
    assert_eq!(run["status"], "concluida", "{corpo}");
    assert_eq!(run["resultado"], "Visita agendada");
    assert_eq!(run["etapas"][0]["no_id"], "coleta");
    assert_eq!(run["etapas"][0]["dados"]["endereco"], "Rua Exemplo 1");
    assert_eq!(run["etapas"][0]["dados"]["periodo"], "tarde");
    assert_eq!(run["etapas"][0]["atores_que_agiram"], json!(["agente"]));
    assert_eq!(run["etapas"][1]["no_id"], "confirmacao");
    let eventos = get_api(&base, &c, &format!("{}/events", ficha.id)).await;
    assert!(eventos["items"].as_array().unwrap().len() >= 3, "{eventos}");
}

#[tokio::test]
async fn exemplo_triagem_exercita_ambos_os_ramos() {
    for (rotulo, resumo, afeta, esperado) in [
        ("grave", "sistema nao abre", "sim", "urgente"),
        ("leve", "duvida simples", "sim", "normal"),
        ("sem-impacto", "sistema nao abre", "nao", "normal"),
    ] {
        let c = Cenario::novo(rotulo);
        c.criar_fluxo("triagem_suporte", "Triagem de suporte", TRIAGEM, true);
        let conversa = c.conversa();
        executar(
            &c,
            conversa,
            "triagem_suporte",
            vec![
                chamada(
                    "fluxo_executar_acao",
                    json!({"acao":"registrado","campos":{"resumo":resumo,"afeta_agora":afeta}}),
                ),
                texto("Entendido."),
            ],
        )
        .await;
        assert_eq!(c.execucao(conversa).no_atual, esperado);
        let base = api(&c).await;
        let ficha = c.ficha(conversa);
        let eventos = get_api(&base, &c, &format!("{}/events", ficha.id)).await;
        assert!(
            eventos.to_string().contains("decisao_resolvida"),
            "{eventos}"
        );
    }
}

async fn evolution_falsa() -> (String, Arc<std::sync::Mutex<Vec<Value>>>) {
    use axum::{Json, Router, extract::State, routing::post};
    let envios = Arc::new(std::sync::Mutex::new(Vec::new()));
    async fn enviar(
        State(envios): State<Arc<std::sync::Mutex<Vec<Value>>>>,
        Json(corpo): Json<Value>,
    ) -> Json<Value> {
        envios.lock().unwrap().push(corpo);
        Json(json!({"key":{"id":"ficticio"}}))
    }
    let router = Router::new()
        .route("/message/sendText/{instancia}", post(enviar))
        .with_state(Arc::clone(&envios));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endereco = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{endereco}"), envios)
}

#[tokio::test]
async fn exemplo_entrega_notificacao_unica_e_transferencia_com_ficha_aberta() {
    use poph_evolution::{ATTACHMENT_CAP, ConfigEvolution, EvolutionClient};
    use poph_server::{
        evolution::CanalEvolution,
        fluxos_efeitos::{EfeitosFluxo, OpcoesEfeitosFluxo},
        saida_whatsapp::{OpcoesSaidaWhatsApp, SaidaWhatsApp},
    };
    use poph_types::{IntegrationId, Secret};
    let c = Cenario::novo("efeitos-fx35");
    c.criar_fluxo("confirmar_entrega", "Confirmar entrega", ENTREGA, true);
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
    let (url, envios) = evolution_falsa().await;
    let canal = CanalEvolution {
        org: c.org,
        instancia: "teste".into(),
        integracao,
        segredo: None,
        cliente: Some(
            EvolutionClient::new(
                ConfigEvolution::new(url, "teste", Secret::new("ficticia".into())).unwrap(),
            )
            .unwrap(),
        ),
        teto_midia: ATTACHMENT_CAP,
    };
    let canais = vec![canal];
    let efeitos = EfeitosFluxo::novo(
        Arc::clone(&c.store),
        &canais,
        c.pasta.join("blobs"),
        None,
        OpcoesEfeitosFluxo::default(),
    );
    let saida = SaidaWhatsApp::novo(
        Arc::clone(&c.store),
        &canais,
        c.pasta.join("blobs"),
        OpcoesSaidaWhatsApp::default(),
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
    let primeira = conversa("5511999990001");
    executar(
        &c,
        primeira,
        "confirmar_entrega",
        vec![
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"localizado","campos":{"pedido":"1234"}}),
            ),
            texto("Vou verificar."),
        ],
    )
    .await;
    efeitos.ciclo_da_org(c.org).await.unwrap();
    saida.ciclo().await.unwrap();
    executar(
        &c,
        primeira,
        "confirmar_entrega",
        vec![
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"confirmada","campos":{"recebido":"sim"}}),
            ),
            texto("Concluído."),
        ],
    )
    .await;
    efeitos.ciclo_da_org(c.org).await.unwrap();
    saida.ciclo().await.unwrap();
    // Um ciclo a mais não reenvia o que já saiu (a notificação é uma só).
    efeitos.ciclo_da_org(c.org).await.unwrap();
    saida.ciclo().await.unwrap();
    let enviados = envios.lock().unwrap().clone();
    assert_eq!(
        enviados
            .iter()
            .filter(|v| v["number"] == "5521999990000")
            .count(),
        1,
        "{enviados:?}"
    );
    assert!(
        enviados
            .iter()
            .any(|v| v["text"].as_str().unwrap_or("").contains("pedido 1234")),
        "{enviados:?}"
    );
    let segunda = conversa("5511999990002");
    executar(
        &c,
        segunda,
        "confirmar_entrega",
        vec![
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"localizado","campos":{"pedido":"5678"}}),
            ),
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"nao_recebida","campos":{"recebido":"nao"}}),
            ),
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"registrado","campos":{}}),
            ),
            texto("Encaminhado."),
        ],
    )
    .await;
    efeitos.ciclo_da_org(c.org).await.unwrap();
    assert_eq!(
        c.store.conversations().get(c.org, segunda).unwrap().status,
        ConversationStatus::Transferred
    );
    assert_eq!(c.ficha(segunda).status, RecordStatus::Open);
    let base = api(&c).await;
    let eventos = get_api(&base, &c, &format!("{}/events", c.ficha(segunda).id)).await;
    assert!(
        eventos.to_string().contains("atendimento_transferido"),
        "{eventos}"
    );
}

#[tokio::test]
async fn revisao_antiga_acao_humana_e_dois_fluxos_na_mesma_ficha() {
    let c = Cenario::novo("fx35-sequencia");
    let visita_humana = VISITA.replace(
        "nome = \"Confirmar visita\",",
        "nome = \"Confirmar visita\",\n    atores = [\"agente\", \"atendente\"],",
    );
    let fluxo = c.criar_fluxo("agendar_visita", "Agendar visita", &visita_humana, true);
    c.criar_fluxo("triagem_suporte", "Triagem de suporte", TRIAGEM, true);
    let conversa = c.conversa();
    executar(
        &c,
        conversa,
        "agendar_visita",
        vec![
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"confirmar","campos":{"endereco":"Rua Teste","periodo":"manhã"}}),
            ),
            texto("Confirme."),
        ],
    )
    .await;
    let anterior = c.execucao(conversa);
    let nova_fonte = visita_humana.replace("Visita agendada", "Resultado da versão nova");
    let compilado = compile(&nova_fonte, &Limites::default());
    let ir = compilado.ir().unwrap();
    let versao = c.store.flows().get(c.org, fluxo).unwrap().version;
    let nova = c
        .store
        .revisions()
        .publish_revision(
            c.org,
            fluxo,
            RevisionPublication {
                expected: versao,
                fonte: nova_fonte,
                definicao_json: ir.to_json(),
                schema_versao: Some(ir.schema_versao),
                resolucoes_json: "{}".into(),
                ator_tipo: "system".into(),
                actor_id: None,
            },
        )
        .unwrap();
    assert_ne!(nova.id, anterior.revisao_id);
    let base = api(&c).await;
    let ficha = c.ficha(conversa);
    let url = format!(
        "{base}/admin/v1/orgs/{}/fichas/{}/flow-runs/{}/actions",
        c.org, ficha.id, anterior.id
    );
    let resposta = reqwest::Client::new()
        .post(url)
        .bearer_auth(TOKEN)
        .header("If-Match", anterior.version.to_string())
        .json(&json!({"acao":"ok", "campos":{}, "chave_idempotencia":"humano-fx35"}))
        .send()
        .await
        .unwrap();
    let status = resposta.status();
    let corpo: Value = resposta.json().await.unwrap();
    assert_eq!(status, reqwest::StatusCode::OK, "{corpo}");
    assert_eq!(corpo["resultado"], "Visita agendada");
    executar(
        &c,
        conversa,
        "triagem_suporte",
        vec![
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"registrado","campos":{"resumo":"erro simples","afeta_agora":"nao"}}),
            ),
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"encaminhado","campos":{}}),
            ),
            texto("Feito."),
        ],
    )
    .await;
    let historico = get_api(&base, &c, &ficha.id.to_string()).await;
    let runs = historico["execucoes"].as_array().unwrap();
    assert_eq!(runs.len(), 2, "{historico}");
    let visita = runs
        .iter()
        .find(|r| r["id"] == anterior.id.to_string())
        .unwrap();
    assert_eq!(visita["revisao_id"], anterior.revisao_id.to_string());
    assert_eq!(visita["resultado"], "Visita agendada");
    assert_eq!(
        visita["etapas"][1]["atores_que_agiram"],
        json!(["atendente"])
    );
    assert!(runs.iter().any(|r| r["resultado"] == "Normal"));
    let eventos = get_api(&base, &c, &format!("{}/events", ficha.id)).await;
    assert!(eventos.to_string().contains("atendente"), "{eventos}");
}

#[tokio::test]
async fn jev_falha_reserva_inicia_permitido_e_recusa_nao_liberado() {
    let c = Cenario::novo("fx35-reserva");
    let permitido = c.criar_fluxo("agendar_visita", "Agendar visita", VISITA, true);
    c.criar_fluxo("triagem_suporte", "Triagem de suporte", TRIAGEM, false);
    let conversa = c.conversa();
    let jev = Arc::new(FakeJev::failing(poph_jev::JevError::Network(
        "indisponível no teste".into(),
    )));
    let (motor, provedor) = c.motor(
        vec![
            chamada("fluxo_iniciar", json!({"fluxo_id":"triagem_suporte"})),
            chamada("fluxo_iniciar", json!({"fluxo_id":"agendar_visita"})),
            texto("Certo."),
        ],
        Some(motor_jev(jev)),
        Some(FlowsConfig::default()),
    );
    let mensagem = c.falar(conversa, "Quero agendar");
    c.turno(&motor, conversa, mensagem).await;
    let pedidos = provedor.received();
    assert!(pedidos[0].tools.iter().any(|f| f.nome == "fluxo_iniciar"));
    assert!(!format!("{:?}", pedidos[0].messages).contains("triagem_suporte"));
    assert!(format!("{:?}", pedidos[1].messages).contains("não está entre os liberados"));
    assert_eq!(c.execucao(conversa).no_atual, "coleta");
    let runs = c
        .store
        .executions()
        .from_case(c.org, c.ficha(conversa).id, 20)
        .unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].fluxo_id, permitido);
}

#[tokio::test]
async fn encerramento_do_agente_e_proxima_mensagem_abrem_nova_ficha() {
    let c = Cenario::novo("fx35-encerrar");
    c.habilitar(&["atendimento_encerrar"]);
    let integracao = c
        .store
        .create_integration(c.org, c.agente, "whatsapp".into(), "active".into())
        .unwrap()
        .id
        .parse()
        .unwrap();
    let entrada = |id: &str| poph_store::NewChannelInboxItem {
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
        .receive_from_channel(c.org, entrada("fx35-1"))
        .unwrap()
        .mensagem;
    let conversa = mensagem.conversation_id;
    let ficha = c.ficha(conversa);
    let (motor, _) = c.motor(
        vec![
            chamada("atendimento_encerrar", json!({"motivo":"resolvido"})),
            texto("Até logo."),
        ],
        None,
        Some(FlowsConfig::default()),
    );
    c.turno(&motor, conversa, mensagem.id).await;
    assert_eq!(c.ficha(conversa).status, RecordStatus::Closed);
    let proxima = c
        .store
        .inbox()
        .receive_from_channel(c.org, entrada("fx35-2"))
        .unwrap()
        .mensagem;
    assert_ne!(proxima.conversation_id, conversa);
    assert_ne!(c.ficha(proxima.conversation_id).id, ficha.id);
    let base = api(&c).await;
    assert_eq!(
        get_api(&base, &c, &ficha.id.to_string()).await["status"],
        "closed"
    );
}

// ─────────────────────────────────────────── FX-40: contadores no caminho do turno

/// Os contadores das execuções contam o turno de verdade: o G9 abre a execução antes do modelo e a
/// ferramenta do agente conclui a **mesma** execução (uma iniciada, uma concluída, nada duplicado).
///
/// Os handles são os do catálogo do `/metrics`, publicados no store como `Estado::novo` faz na
/// subida — aqui o servidor não sobe, então a publicação é explícita.
#[tokio::test]
async fn contadores_do_turno_contam_uma_abertura_e_uma_conclusao() {
    let c = Cenario::novo("fx40-turno");
    let registro = poph_types::Registry::new();
    let metricas = poph_server::fluxos_metricas::MetricasFluxos::registrar(&registro)
        .expect("o catálogo de fluxos registra");
    c.store.publish_execution_metrics(metricas.das_execucoes());

    c.criar_fluxo("agendar_visita", "Agendar visita", VISITA, true);
    let conversa = c.conversa();
    executar(
        &c,
        conversa,
        "agendar_visita",
        vec![
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"confirmar","campos":{"endereco":"Rua Exemplo 1","periodo":"tarde"}}),
            ),
            chamada("fluxo_executar_acao", json!({"acao":"ok","campos":{}})),
            texto("Visita combinada."),
        ],
    )
    .await;

    assert_eq!(
        metricas.execucoes_iniciadas.value(),
        1,
        "o turno abriu uma execução (G9 antes do modelo)"
    );
    assert_eq!(
        metricas.execucoes_concluidas.value(),
        1,
        "as ações do agente concluíram a mesma execução"
    );
    assert_eq!(metricas.execucoes_abandonadas.value(), 0);
    assert_eq!(metricas.execucoes_falhas.value(), 0);
    assert_eq!(metricas.erros_regra.value(), 0);
}
