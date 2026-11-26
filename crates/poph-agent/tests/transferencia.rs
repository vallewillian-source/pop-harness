use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::turno::{
    FuturePause, NoopObserver, Pause, TurnEngine, TurnEvent, TurnInput, TurnObserver, TurnOptions,
    TurnSummary,
};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, Script, Usage};
use poph_jev::{
    ANIMAL_RISK, BatchResponse, FakeJev, G1_CONTINUE, G1_TRANSFER, HandoffGuidance, INGESTION_RISK,
    IdGate, JevEngine, JevRequest, JevResponse, NO_RISK, OTHER_RISK, RISK_QUESTION_ID,
};
use poph_store::{
    AgentChange, Direction, NewAgent, NewConversation, NewMessage, NewOrg, Page, Role, Store,
    TurnState,
};
use poph_types::{AgentId, ConversationId, JevGates, MessageId, OrgId};

const APROVADO_INGESTAO: &str = "⚠️ Se alguém engoliu ou teve contato com o produto, procure \
atendimento médico agora ou ligue para o SAMU (192) ou para o Disque-Intoxicação \
(0800 722 6001). Leve a embalagem do produto. Não provoque vômito e não dê nada para beber ou \
comer sem orientação médica.";
const APROVADO_ANIMAL: &str = "⚠️ Não toque nem tente pegar o animal. Mantenha crianças e outras \
pessoas longe dele e, se puder, feche a porta do cômodo. Se alguém foi picado, vá agora a um \
pronto-socorro ou ligue para o SAMU (192).";
const APROVADO_OUTRO: &str = "⚠️ Se alguém corre risco de saúde agora, ligue para o SAMU (192) ou \
vá a um pronto-socorro sem esperar a nossa resposta.";
const APROVADA_AUTOMATICA: &str = "Sua conversa foi transferida para um atendente humano da nossa \
equipe, que já irá respondê-la por aqui. 🙋";

const RESPOSTA_DO_MODELO: &str = "Olá! Como posso ajudar?";

struct SemEspera;

impl Pause for SemEspera {
    fn wait(&self, _duracao: Duration) -> FuturePause {
        Box::pin(async {})
    }
}

#[derive(Default)]
struct Gravador {
    events: Vec<TurnEvent>,
}

impl TurnObserver for Gravador {
    fn evento(&mut self, evento: TurnEvent) {
        self.events.push(evento);
    }
}

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    provedor: Arc<FakeProvider>,
}

impl Scenario {
    fn new(label: &str, handoff_ativo: bool) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-hr29-{label}-{}-{}",
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
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        store
            .instructions()
            .save(
                org,
                agente,
                0,
                "{}",
                "# Bia\n\nVocê é a atendente da FixProLar.",
                true,
            )
            .expect("grava as instruções");
        if handoff_ativo {
            store
                .agents()
                .update(
                    org,
                    agente,
                    1,
                    AgentChange {
                        handoff_active: Some(true),
                        ..AgentChange::default()
                    },
                )
                .expect("liga o handoff");
        }
        let provedor = Arc::new(FakeProvider::with_scripts(
            (0..8).map(|_| roteiro(RESPOSTA_DO_MODELO)).collect(),
        ));
        Self {
            folder,
            store,
            org,
            agente,
            provedor,
        }
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
                    peer_externo: None,
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

    fn situacoes_de_handoff(&self, situacoes: &[&str]) {
        let current = self
            .store
            .agents()
            .get(self.org, self.agente)
            .expect("lê o agente");
        let json = serde_json::json!({ "situacoes_de_handoff": situacoes }).to_string();
        self.store
            .agents()
            .update(
                self.org,
                self.agente,
                current.version,
                AgentChange {
                    jev_json: Some(json),
                    ..AgentChange::default()
                },
            )
            .expect("ajusta o jev_json do agente");
    }

    fn engine(&self, jev: Option<Arc<JevEngine>>, orientacao: HandoffGuidance) -> TurnEngine {
        let config = ConfigHot::new(
            Arc::clone(&self.store),
            HotConfigOptions::default(),
            MetricsMemory::own(),
        );
        let limiter = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
        let mut engine = TurnEngine::new(
            Arc::clone(&self.store),
            config,
            limiter,
            Arc::clone(&self.provedor) as Arc<dyn poph_core::llm::LlmProvider>,
            TurnOptions::default(),
            MetricsTurn::own(),
        )
        .with_pause(Arc::new(SemEspera))
        .with_handoff(orientacao);
        if let Some(jev) = jev {
            engine = engine.with_jev(jev);
        }
        engine
    }

    async fn turno(
        &self,
        engine: &TurnEngine,
        conversa: ConversationId,
        gatilho: MessageId,
    ) -> TurnSummary {
        self.turno_observado(engine, conversa, gatilho, &mut NoopObserver)
            .await
    }

    async fn turno_observado(
        &self,
        engine: &TurnEngine,
        conversa: ConversationId,
        gatilho: MessageId,
        observador: &mut dyn TurnObserver,
    ) -> TurnSummary {
        engine
            .run(TurnInput::new(self.org, conversa, gatilho), observador)
            .await
            .expect("o turno roda")
    }

    fn conversa_gravada(&self, conversa: ConversationId) -> Vec<(Role, String)> {
        self.store
            .messages()
            .last_set(self.org, conversa, 50)
            .expect("lê as mensagens")
            .into_iter()
            .map(|mensagem| (mensagem.role, mensagem.texto))
            .collect()
    }

    fn respostas_do_agente(&self, conversa: ConversationId) -> Vec<String> {
        self.conversa_gravada(conversa)
            .into_iter()
            .filter(|(role, _)| *role == Role::Assistant)
            .map(|(_, texto)| texto)
            .collect()
    }

    fn status(&self, conversa: ConversationId) -> poph_store::ConversationStatus {
        self.store
            .conversations()
            .get(self.org, conversa)
            .expect("lê a conversa")
            .status
    }

    fn ultimo_turno(&self, conversa: ConversationId) -> poph_store::Turno {
        self.store
            .turns()
            .list(self.org, conversa, Page::with_limit(20))
            .expect("lista os turnos")
            .into_iter()
            .next_back()
            .expect("há um turno gravado")
    }

    fn auditoria(&self) -> Vec<poph_store::AuditEntry> {
        self.store
            .audit()
            .list(self.org, Page::with_limit(50))
            .expect("lê a auditoria")
    }

    fn reabrir(&self, conversa: ConversationId) {
        let current = self
            .store
            .conversations()
            .get(self.org, conversa)
            .expect("lê a conversa");
        self.store
            .conversations()
            .reopen_audited(self.org, conversa, current.version, "teste".to_owned())
            .expect("reabre a conversa");
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

fn roteiro(texto: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(texto.to_owned()),
        LlmEvent::Usage(Usage {
            input: 10,
            saida: 5,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

fn so_g1() -> JevGates {
    JevGates {
        g1: true,
        ..JevGates::default()
    }
}

fn response(choice: &str, confidence: f64) -> JevResponse {
    let mut item = JevResponse {
        choice: Some(choice.to_owned()),
        confidence,
        ..JevResponse::default()
    };
    item.probabilities.insert(choice.to_owned(), confidence);
    item
}

fn jev(decisao: &'static str, confidence: f64, risco: Option<(&'static str, f64)>) -> Arc<FakeJev> {
    Arc::new(FakeJev::new(move |request: &JevRequest| {
        let mut respostas = BTreeMap::new();
        if request.questions.contains_key("g1_transferir") {
            respostas.insert("g1_transferir".to_owned(), response(decisao, confidence));
        }
        if request.questions.contains_key(RISK_QUESTION_ID)
            && let Some((tipo, confidence)) = risco
        {
            respostas.insert(RISK_QUESTION_ID.to_owned(), response(tipo, confidence));
        }
        Ok(BatchResponse {
            answers: respostas,
            tokens: 9,
            ..BatchResponse::default()
        })
    }))
}

fn motor_jev(jev: Arc<FakeJev>) -> Arc<JevEngine> {
    Arc::new(JevEngine::new(jev, so_g1(), "jev-latest"))
}

#[tokio::test]
async fn transfere_com_orientacao_e_nao_chama_o_modelo() {
    let scenario = Scenario::new("orientacao", true);
    scenario.situacoes_de_handoff(&["risco com produto"]);
    let conversa = scenario.conversa();
    let jev = jev(G1_TRANSFER, 0.95, Some((INGESTION_RISK, 0.9)));
    let engine = scenario.engine(
        Some(motor_jev(Arc::clone(&jev))),
        HandoffGuidance::default(),
    );

    let gatilho = scenario.falar(conversa, "meu filho engoliu o veneno da isca, e agora?");
    let mut observador = Gravador::default();
    let summary = scenario
        .turno_observado(&engine, conversa, gatilho, &mut observador)
        .await;

    assert!(
        scenario.provedor.received().is_empty(),
        "conversa transferida não pode chamar o LLM"
    );
    assert_eq!(summary.estado, TurnState::Completed);
    let expected = format!("{APROVADO_INGESTAO}\n\n{APROVADA_AUTOMATICA}");
    assert_eq!(summary.texto, expected);
    assert_eq!(
        summary.motivo_fim.as_deref(),
        Some("transferida para humano")
    );

    assert_eq!(
        scenario.respostas_do_agente(conversa),
        vec![expected.clone()]
    );
    assert_eq!(
        scenario.status(conversa),
        poph_store::ConversationStatus::Transferred
    );
    let pending = scenario
        .store
        .outbox()
        .list_pending(scenario.org, 10)
        .expect("lista a outbox");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].tipo, "text");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&pending[0].content_json)
            .expect("conteúdo é JSON")["texto"],
        expected
    );

    let turno = scenario.ultimo_turno(conversa);
    let stored: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("jev_json é JSON");
    assert_eq!(stored["decisoes"][0]["gate"], "g1");
    assert_eq!(stored["decisoes"][0]["escolha"], G1_TRANSFER);
    assert_eq!(stored["risco"]["tipo"], INGESTION_RISK);
    assert_eq!(turno.state, TurnState::Completed);

    let registros = scenario.auditoria();
    let transferencia = registros
        .iter()
        .find(|registro| registro.acao == "transfer")
        .expect("a transferência é auditada");
    assert_eq!(transferencia.ator_tipo, "system");
    assert_eq!(transferencia.entity, "conversation");
    assert_eq!(
        transferencia.entity_id.as_deref(),
        Some(conversa.to_string().as_str())
    );
    let detalhe: serde_json::Value =
        serde_json::from_str(&transferencia.detail_json).expect("detalhe é JSON");
    assert_eq!(detalhe["gate"], "g1");
    assert_eq!(detalhe["risco"], INGESTION_RISK);

    assert!(
        observador.events.iter().any(|evento| matches!(
            evento,
            TurnEvent::Handoff {
                guidance: true,
                risk_kind: Some(tipo),
                ..
            } if tipo == INGESTION_RISK
        )),
        "faltou o evento de transferência: {:?}",
        observador.events
    );
    let pedidos = jev.requests();
    assert_eq!(pedidos.len(), 2);
    assert_eq!(
        pedidos[1].questions.keys().collect::<Vec<_>>(),
        vec![RISK_QUESTION_ID]
    );
    assert_eq!(pedidos[1].estado["canal"], "cli");
}

#[tokio::test]
async fn outros_dois_tipos_de_risco_usam_os_textos_aprovados() {
    for (tipo, aprovado) in [(ANIMAL_RISK, APROVADO_ANIMAL), (OTHER_RISK, APROVADO_OUTRO)] {
        let scenario = Scenario::new(&format!("tipo-{tipo}"), true);
        let conversa = scenario.conversa();
        let jev = jev(G1_TRANSFER, 0.95, Some((tipo, 0.8)));
        let engine = scenario.engine(Some(motor_jev(jev)), HandoffGuidance::default());
        let gatilho = scenario.falar(conversa, "tem um escorpião no berço do meu filho");
        let summary = scenario.turno(&engine, conversa, gatilho).await;
        assert_eq!(
            summary.texto,
            format!("{aprovado}\n\n{APROVADA_AUTOMATICA}"),
            "texto errado para {tipo}"
        );
    }
}

#[tokio::test]
async fn transferida_nao_responde_ate_reabrir() {
    let scenario = Scenario::new("reabrir", true);
    let conversa = scenario.conversa();
    let jev = Arc::new(FakeJev::new(|request: &JevRequest| {
        let last = request.estado["conversa"]["mensagens"]
            .as_array()
            .and_then(|messages| messages.last())
            .and_then(|mensagem| mensagem["texto"].as_str())
            .unwrap_or("")
            .to_owned();
        let mut respostas = BTreeMap::new();
        if request.questions.contains_key("g1_transferir") {
            let choice = if last.contains("cobra") {
                G1_TRANSFER
            } else {
                G1_CONTINUE
            };
            respostas.insert("g1_transferir".to_owned(), response(choice, 0.95));
        }
        if request.questions.contains_key(RISK_QUESTION_ID) {
            respostas.insert(RISK_QUESTION_ID.to_owned(), response(ANIMAL_RISK, 0.9));
        }
        Ok(BatchResponse {
            answers: respostas,
            tokens: 9,
            ..BatchResponse::default()
        })
    }));
    let engine = scenario.engine(Some(motor_jev(jev)), HandoffGuidance::default());

    let gatilho = scenario.falar(conversa, "quero falar com uma pessoa, tem uma cobra aqui");
    let first = scenario.turno(&engine, conversa, gatilho).await;
    assert_eq!(first.estado, TurnState::Completed);
    assert_eq!(scenario.provedor.received().len(), 0);

    let segundo_gatilho = scenario.falar(conversa, "alguém aí?");
    let second = scenario.turno(&engine, conversa, segundo_gatilho).await;
    assert_eq!(second.estado, TurnState::Skipped);
    assert!(
        second.error.unwrap_or_default().contains("transferred"),
        "o motivo do turno ignorado diz que a conversa está transferida"
    );
    assert_eq!(scenario.provedor.received().len(), 0);
    let textos: Vec<String> = scenario
        .conversa_gravada(conversa)
        .into_iter()
        .map(|(_, texto)| texto)
        .collect();
    assert!(textos.contains(&"alguém aí?".to_owned()));

    scenario.reabrir(conversa);
    assert_eq!(
        scenario.status(conversa),
        poph_store::ConversationStatus::Open
    );
    let terceiro_gatilho = scenario.falar(conversa, "e o prazo de entrega?");
    let terceiro = scenario.turno(&engine, conversa, terceiro_gatilho).await;
    assert_eq!(terceiro.estado, TurnState::Completed);
    assert_eq!(terceiro.texto, RESPOSTA_DO_MODELO);
    assert_eq!(scenario.provedor.received().len(), 1);
}

#[tokio::test]
async fn erro_na_pergunta_de_risco_transfere_com_a_mensagem_automatica() {
    let scenario = Scenario::new("risco-erro", true);
    let conversa = scenario.conversa();
    let jev = Arc::new(FakeJev::new(|request: &JevRequest| {
        if request.questions.contains_key(RISK_QUESTION_ID) {
            return Err(poph_jev::JevError::Status {
                code: 503,
                motivo: "Service Unavailable".to_owned(),
            });
        }
        let mut respostas = BTreeMap::new();
        respostas.insert("g1_transferir".to_owned(), response(G1_TRANSFER, 0.99));
        Ok(BatchResponse {
            answers: respostas,
            tokens: 9,
            ..BatchResponse::default()
        })
    }));
    let engine = scenario.engine(Some(motor_jev(jev)), HandoffGuidance::default());

    let gatilho = scenario.falar(conversa, "quero um atendente");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, APROVADA_AUTOMATICA);
    assert_eq!(
        scenario.status(conversa),
        poph_store::ConversationStatus::Transferred
    );
    assert_eq!(
        scenario.respostas_do_agente(conversa),
        vec![APROVADA_AUTOMATICA.to_owned()]
    );
    assert!(scenario.provedor.received().is_empty());
    let turno = scenario.ultimo_turno(conversa);
    let stored: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    assert!(stored["risco"]["falha"].as_str().is_some());
}

#[tokio::test]
async fn prazo_na_pergunta_de_risco_suprime_so_a_orientacao() {
    let scenario = Scenario::new("risco-prazo", true);
    let conversa = scenario.conversa();
    let jev = Arc::new(
        FakeJev::with_choices(BTreeMap::from([
            ("g1_transferir".to_owned(), G1_TRANSFER.to_owned()),
            (RISK_QUESTION_ID.to_owned(), INGESTION_RISK.to_owned()),
        ]))
        .with_delay(Duration::from_millis(200)),
    );
    let orientacao = HandoffGuidance::default().with_timeout(Duration::from_millis(20));
    let engine = scenario.engine(Some(motor_jev(jev)), orientacao);

    let gatilho = scenario.falar(conversa, "meu filho tomou o veneno");
    let beginning = Instant::now();
    let summary = scenario.turno(&engine, conversa, gatilho).await;
    assert!(beginning.elapsed() < Duration::from_secs(5));

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, APROVADA_AUTOMATICA);
    assert_eq!(
        scenario.status(conversa),
        poph_store::ConversationStatus::Transferred
    );
}

#[tokio::test]
async fn sem_risco_confiavel_manda_so_a_mensagem_automatica() {
    for (label, risco) in [
        ("nenhum", (NO_RISK, 0.99)),
        ("confianca-baixa", (INGESTION_RISK, 0.2)),
    ] {
        let scenario = Scenario::new(&format!("risco-{label}"), true);
        let conversa = scenario.conversa();
        let jev = jev(G1_TRANSFER, 0.95, Some(risco));
        let engine = scenario.engine(Some(motor_jev(jev)), HandoffGuidance::default());
        let gatilho = scenario.falar(conversa, "quero falar com o atendente, por favor");
        let summary = scenario.turno(&engine, conversa, gatilho).await;
        assert_eq!(summary.texto, APROVADA_AUTOMATICA, "caso {label}");
        assert_eq!(
            scenario.status(conversa),
            poph_store::ConversationStatus::Transferred
        );
    }
}

#[tokio::test]
async fn orientacao_desligada_nao_faz_a_segunda_pergunta() {
    let scenario = Scenario::new("orientacao-desligada", true);
    let conversa = scenario.conversa();
    let jev = jev(G1_TRANSFER, 0.95, Some((INGESTION_RISK, 0.9)));
    let orientacao = HandoffGuidance {
        ativo: false,
        ..HandoffGuidance::default()
    };
    let engine = scenario.engine(Some(motor_jev(Arc::clone(&jev))), orientacao);

    let gatilho = scenario.falar(conversa, "quero falar com alguém");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.texto, APROVADA_AUTOMATICA);
    assert_eq!(jev.requests().len(), 1, "só o lote do turno foi chamado");
    assert_eq!(jev.calls(), 1);
}

#[tokio::test]
async fn texto_vazio_na_configuracao_nao_orienta() {
    let scenario = Scenario::new("texto-vazio", true);
    let conversa = scenario.conversa();
    let jev = jev(G1_TRANSFER, 0.95, Some((INGESTION_RISK, 0.9)));
    let orientacao = HandoffGuidance::default().with_text(INGESTION_RISK, "");
    let engine = scenario.engine(Some(motor_jev(jev)), orientacao);

    let gatilho = scenario.falar(conversa, "meu filho engoliu o veneno");
    let summary = scenario.turno(&engine, conversa, gatilho).await;
    assert_eq!(summary.texto, APROVADA_AUTOMATICA);
}

#[tokio::test]
async fn g1_desligado_nao_transfere() {
    let scenario = Scenario::new("g1-desligado", true);
    let conversa = scenario.conversa();
    let jev = jev(G1_TRANSFER, 0.99, Some((INGESTION_RISK, 0.99)));
    let evaluator = Arc::new(JevEngine::new(jev, JevGates::default(), "jev-latest"));
    let engine = scenario.engine(Some(evaluator), HandoffGuidance::default());

    let gatilho = scenario.falar(conversa, "quero falar com um atendente humano");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, RESPOSTA_DO_MODELO);
    assert_eq!(
        scenario.status(conversa),
        poph_store::ConversationStatus::Open
    );
    assert_eq!(scenario.provedor.received().len(), 1);
    assert!(summary.jev.is_none() || summary.jev.as_ref().unwrap().decisions.is_empty());
}

#[tokio::test]
async fn agente_sem_handoff_nao_transfere() {
    let scenario = Scenario::new("sem-handoff", false);
    let conversa = scenario.conversa();
    let jev = jev(G1_TRANSFER, 0.99, Some((INGESTION_RISK, 0.99)));
    let engine = scenario.engine(Some(motor_jev(jev)), HandoffGuidance::default());

    let gatilho = scenario.falar(conversa, "quero falar com um atendente humano");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.texto, RESPOSTA_DO_MODELO);
    assert_eq!(
        scenario.status(conversa),
        poph_store::ConversationStatus::Open
    );
}

#[tokio::test]
async fn g1_continuar_responde_normalmente() {
    let scenario = Scenario::new("continuar", true);
    let conversa = scenario.conversa();
    let jev = jev(G1_CONTINUE, 0.99, None);
    let engine = scenario.engine(Some(motor_jev(jev)), HandoffGuidance::default());

    let gatilho = scenario.falar(conversa, "qual é o prazo de entrega?");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, RESPOSTA_DO_MODELO);
    assert_eq!(
        scenario.status(conversa),
        poph_store::ConversationStatus::Open
    );
    assert_eq!(
        summary
            .jev
            .as_ref()
            .and_then(|item| item.choice(IdGate::G1)),
        Some(G1_CONTINUE)
    );
    assert_eq!(scenario.provedor.received().len(), 1);
    assert!(
        !scenario
            .auditoria()
            .iter()
            .any(|registro| registro.acao == "transfer")
    );
}

#[tokio::test]
async fn transferencia_guarda_o_departamento_do_agente() {
    let scenario = Scenario::new("departamento", true);
    let default = scenario
        .store
        .departments()
        .create(
            scenario.org,
            poph_store::NewDepartment {
                name: "Atendimento humano".to_owned(),
                description: String::new(),
            },
        )
        .expect("cria o departamento")
        .id;
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
            AgentChange {
                handoff_department: Some(default),
                ..AgentChange::default()
            },
        )
        .expect("define o departamento humano");

    let conversa = scenario.conversa();
    let jev = jev(G1_TRANSFER, 0.95, Some((ANIMAL_RISK, 0.9)));
    let engine = scenario.engine(Some(motor_jev(jev)), HandoffGuidance::default());
    let gatilho = scenario.falar(conversa, "tem uma aranha no quarto");
    scenario.turno(&engine, conversa, gatilho).await;

    let stored = scenario
        .store
        .conversations()
        .get(scenario.org, conversa)
        .expect("lê a conversa");
    assert_eq!(stored.status, poph_store::ConversationStatus::Transferred);
    assert_eq!(stored.target_department, Some(default));
}

#[tokio::test]
async fn segunda_pergunta_nao_usa_o_cache_do_lote() {
    let scenario = Scenario::new("cache", true);
    let conversa = scenario.conversa();
    let jev = jev(G1_TRANSFER, 0.95, Some((INGESTION_RISK, 0.9)));
    let evaluator = motor_jev(Arc::clone(&jev));
    let engine = scenario.engine(Some(evaluator), HandoffGuidance::default());

    let gatilho = scenario.falar(conversa, "engoliu o produto");
    scenario.turno(&engine, conversa, gatilho).await;
    assert_eq!(jev.requests().len(), 2);
    assert_eq!(
        jev.requests()[1].questions.keys().collect::<Vec<_>>(),
        vec![RISK_QUESTION_ID]
    );
    assert_eq!(
        jev.requests()[0].estado["conversa"],
        jev.requests()[1].estado["conversa"]
    );
}
