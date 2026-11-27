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
const APPROVED_OTHER: &str = "⚠️ Se alguém corre risco de saúde agora, ligue para o SAMU (192) ou \
vá a um pronto-socorro sem esperar a nossa resposta.";
const APPROVED_AUTOMATIC: &str = "Sua conversa foi transferida para um atendente humano da nossa \
equipe, que já irá respondê-la por aqui. 🙋";

const REPLY_MODEL: &str = "Olá! Como posso ajudar?";

struct NoWait;

impl Pause for NoWait {
    fn wait(&self, _duration: Duration) -> FuturePause {
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
    fn new(label: &str, handoff_active: bool) -> Self {
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
        if handoff_active {
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
            (0..8).map(|_| script(REPLY_MODEL)).collect(),
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

    fn say(&self, conversa: ConversationId, texto: &str) -> MessageId {
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

    fn situations_handoff(&self, situations: &[&str]) {
        let current = self
            .store
            .agents()
            .get(self.org, self.agente)
            .expect("lê o agente");
        let json = serde_json::json!({ "situacoes_de_handoff": situations }).to_string();
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

    fn engine(&self, jev: Option<Arc<JevEngine>>, guidance: HandoffGuidance) -> TurnEngine {
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
        .with_pause(Arc::new(NoWait))
        .with_handoff(guidance);
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
        self.turn_observed(engine, conversa, gatilho, &mut NoopObserver)
            .await
    }

    async fn turn_observed(
        &self,
        engine: &TurnEngine,
        conversa: ConversationId,
        gatilho: MessageId,
        observer: &mut dyn TurnObserver,
    ) -> TurnSummary {
        engine
            .run(TurnInput::new(self.org, conversa, gatilho), observer)
            .await
            .expect("o turno roda")
    }

    fn conversation_recorded(&self, conversa: ConversationId) -> Vec<(Role, String)> {
        self.store
            .messages()
            .last_set(self.org, conversa, 50)
            .expect("lê as mensagens")
            .into_iter()
            .map(|mensagem| (mensagem.role, mensagem.texto))
            .collect()
    }

    fn replies_agent(&self, conversa: ConversationId) -> Vec<String> {
        self.conversation_recorded(conversa)
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

    fn last_turn(&self, conversa: ConversationId) -> poph_store::Turno {
        self.store
            .turns()
            .list(self.org, conversa, Page::with_limit(20))
            .expect("lista os turnos")
            .into_iter()
            .next_back()
            .expect("há um turno gravado")
    }

    fn audit(&self) -> Vec<poph_store::AuditEntry> {
        self.store
            .audit()
            .list(self.org, Page::with_limit(50))
            .expect("lê a auditoria")
    }

    fn reopen(&self, conversa: ConversationId) {
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

fn script(texto: &str) -> Script {
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

fn jev(decisao: &'static str, confidence: f64, risk: Option<(&'static str, f64)>) -> Arc<FakeJev> {
    Arc::new(FakeJev::new(move |request: &JevRequest| {
        let mut replies = BTreeMap::new();
        if request.questions.contains_key("g1_transferir") {
            replies.insert("g1_transferir".to_owned(), response(decisao, confidence));
        }
        if request.questions.contains_key(RISK_QUESTION_ID)
            && let Some((tipo, confidence)) = risk
        {
            replies.insert(RISK_QUESTION_ID.to_owned(), response(tipo, confidence));
        }
        Ok(BatchResponse {
            answers: replies,
            tokens: 9,
            ..BatchResponse::default()
        })
    }))
}

fn engine_judge(jev: Arc<FakeJev>) -> Arc<JevEngine> {
    Arc::new(JevEngine::new(jev, so_g1(), "jev-latest"))
}

#[tokio::test]
async fn transfers_with_guidance_not_calls_model() {
    let scenario = Scenario::new("orientacao", true);
    scenario.situations_handoff(&["risco com produto"]);
    let conversa = scenario.conversa();
    let jev = jev(G1_TRANSFER, 0.95, Some((INGESTION_RISK, 0.9)));
    let engine = scenario.engine(
        Some(engine_judge(Arc::clone(&jev))),
        HandoffGuidance::default(),
    );

    let gatilho = scenario.say(conversa, "meu filho engoliu o veneno da isca, e agora?");
    let mut observer = Gravador::default();
    let summary = scenario
        .turn_observed(&engine, conversa, gatilho, &mut observer)
        .await;

    assert!(
        scenario.provedor.received().is_empty(),
        "conversa transferida não pode chamar o LLM"
    );
    assert_eq!(summary.estado, TurnState::Completed);
    let expected = format!("{APROVADO_INGESTAO}\n\n{APPROVED_AUTOMATIC}");
    assert_eq!(summary.texto, expected);
    assert_eq!(
        summary.motivo_fim.as_deref(),
        Some("transferida para humano")
    );

    assert_eq!(scenario.replies_agent(conversa), vec![expected.clone()]);
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

    let turno = scenario.last_turn(conversa);
    let stored: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("jev_json é JSON");
    assert_eq!(stored["decisoes"][0]["gate"], "g1");
    assert_eq!(stored["decisoes"][0]["escolha"], G1_TRANSFER);
    assert_eq!(stored["risco"]["tipo"], INGESTION_RISK);
    assert_eq!(turno.state, TurnState::Completed);

    let records = scenario.audit();
    let transfer = records
        .iter()
        .find(|registro| registro.acao == "transfer")
        .expect("a transferência é auditada");
    assert_eq!(transfer.ator_tipo, "system");
    assert_eq!(transfer.entity, "conversation");
    assert_eq!(
        transfer.entity_id.as_deref(),
        Some(conversa.to_string().as_str())
    );
    let detail: serde_json::Value =
        serde_json::from_str(&transfer.detail_json).expect("detalhe é JSON");
    assert_eq!(detail["gate"], "g1");
    assert_eq!(detail["risco"], INGESTION_RISK);

    assert!(
        observer.events.iter().any(|evento| matches!(
            evento,
            TurnEvent::Handoff {
                guidance: true,
                risk_kind: Some(tipo),
                ..
            } if tipo == INGESTION_RISK
        )),
        "faltou o evento de transferência: {:?}",
        observer.events
    );
    let requests = jev.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].questions.keys().collect::<Vec<_>>(),
        vec![RISK_QUESTION_ID]
    );
    assert_eq!(requests[1].estado["canal"], "cli");
}

#[tokio::test]
async fn others_two_kinds_risk_use_texts_approved() {
    for (tipo, aprovado) in [(ANIMAL_RISK, APROVADO_ANIMAL), (OTHER_RISK, APPROVED_OTHER)] {
        let scenario = Scenario::new(&format!("tipo-{tipo}"), true);
        let conversa = scenario.conversa();
        let jev = jev(G1_TRANSFER, 0.95, Some((tipo, 0.8)));
        let engine = scenario.engine(Some(engine_judge(jev)), HandoffGuidance::default());
        let gatilho = scenario.say(conversa, "tem um escorpião no berço do meu filho");
        let summary = scenario.turno(&engine, conversa, gatilho).await;
        assert_eq!(
            summary.texto,
            format!("{aprovado}\n\n{APPROVED_AUTOMATIC}"),
            "texto errado para {tipo}"
        );
    }
}

#[tokio::test]
async fn transferred_not_answers_until_reopen() {
    let scenario = Scenario::new("reabrir", true);
    let conversa = scenario.conversa();
    let jev = Arc::new(FakeJev::new(|request: &JevRequest| {
        let last = request.estado["conversa"]["mensagens"]
            .as_array()
            .and_then(|messages| messages.last())
            .and_then(|mensagem| mensagem["texto"].as_str())
            .unwrap_or("")
            .to_owned();
        let mut replies = BTreeMap::new();
        if request.questions.contains_key("g1_transferir") {
            let choice = if last.contains("cobra") {
                G1_TRANSFER
            } else {
                G1_CONTINUE
            };
            replies.insert("g1_transferir".to_owned(), response(choice, 0.95));
        }
        if request.questions.contains_key(RISK_QUESTION_ID) {
            replies.insert(RISK_QUESTION_ID.to_owned(), response(ANIMAL_RISK, 0.9));
        }
        Ok(BatchResponse {
            answers: replies,
            tokens: 9,
            ..BatchResponse::default()
        })
    }));
    let engine = scenario.engine(Some(engine_judge(jev)), HandoffGuidance::default());

    let gatilho = scenario.say(conversa, "quero falar com uma pessoa, tem uma cobra aqui");
    let first = scenario.turno(&engine, conversa, gatilho).await;
    assert_eq!(first.estado, TurnState::Completed);
    assert_eq!(scenario.provedor.received().len(), 0);

    let second_trigger = scenario.say(conversa, "alguém aí?");
    let second = scenario.turno(&engine, conversa, second_trigger).await;
    assert_eq!(second.estado, TurnState::Skipped);
    assert!(
        second.error.unwrap_or_default().contains("transferred"),
        "o motivo do turno ignorado diz que a conversa está transferida"
    );
    assert_eq!(scenario.provedor.received().len(), 0);
    let texts: Vec<String> = scenario
        .conversation_recorded(conversa)
        .into_iter()
        .map(|(_, texto)| texto)
        .collect();
    assert!(texts.contains(&"alguém aí?".to_owned()));

    scenario.reopen(conversa);
    assert_eq!(
        scenario.status(conversa),
        poph_store::ConversationStatus::Open
    );
    let third_trigger = scenario.say(conversa, "e o prazo de entrega?");
    let third_one = scenario.turno(&engine, conversa, third_trigger).await;
    assert_eq!(third_one.estado, TurnState::Completed);
    assert_eq!(third_one.texto, REPLY_MODEL);
    assert_eq!(scenario.provedor.received().len(), 1);
}

#[tokio::test]
async fn error_question_risk_transfers_with_message_automatic() {
    let scenario = Scenario::new("risco-erro", true);
    let conversa = scenario.conversa();
    let jev = Arc::new(FakeJev::new(|request: &JevRequest| {
        if request.questions.contains_key(RISK_QUESTION_ID) {
            return Err(poph_jev::JevError::Status {
                code: 503,
                motivo: "Service Unavailable".to_owned(),
            });
        }
        let mut replies = BTreeMap::new();
        replies.insert("g1_transferir".to_owned(), response(G1_TRANSFER, 0.99));
        Ok(BatchResponse {
            answers: replies,
            tokens: 9,
            ..BatchResponse::default()
        })
    }));
    let engine = scenario.engine(Some(engine_judge(jev)), HandoffGuidance::default());

    let gatilho = scenario.say(conversa, "quero um atendente");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, APPROVED_AUTOMATIC);
    assert_eq!(
        scenario.status(conversa),
        poph_store::ConversationStatus::Transferred
    );
    assert_eq!(
        scenario.replies_agent(conversa),
        vec![APPROVED_AUTOMATIC.to_owned()]
    );
    assert!(scenario.provedor.received().is_empty());
    let turno = scenario.last_turn(conversa);
    let stored: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    assert!(stored["risco"]["falha"].as_str().is_some());
}

#[tokio::test]
async fn deadline_question_risk_suppresses_guidance() {
    let scenario = Scenario::new("risco-prazo", true);
    let conversa = scenario.conversa();
    let jev = Arc::new(
        FakeJev::with_choices(BTreeMap::from([
            ("g1_transferir".to_owned(), G1_TRANSFER.to_owned()),
            (RISK_QUESTION_ID.to_owned(), INGESTION_RISK.to_owned()),
        ]))
        .with_delay(Duration::from_millis(200)),
    );
    let guidance = HandoffGuidance::default().with_timeout(Duration::from_millis(20));
    let engine = scenario.engine(Some(engine_judge(jev)), guidance);

    let gatilho = scenario.say(conversa, "meu filho tomou o veneno");
    let beginning = Instant::now();
    let summary = scenario.turno(&engine, conversa, gatilho).await;
    assert!(beginning.elapsed() < Duration::from_secs(5));

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, APPROVED_AUTOMATIC);
    assert_eq!(
        scenario.status(conversa),
        poph_store::ConversationStatus::Transferred
    );
}

#[tokio::test]
async fn without_risk_reliable_sends_message_automatic() {
    for (label, risk) in [
        ("nenhum", (NO_RISK, 0.99)),
        ("confianca-baixa", (INGESTION_RISK, 0.2)),
    ] {
        let scenario = Scenario::new(&format!("risco-{label}"), true);
        let conversa = scenario.conversa();
        let jev = jev(G1_TRANSFER, 0.95, Some(risk));
        let engine = scenario.engine(Some(engine_judge(jev)), HandoffGuidance::default());
        let gatilho = scenario.say(conversa, "quero falar com o atendente, por favor");
        let summary = scenario.turno(&engine, conversa, gatilho).await;
        assert_eq!(summary.texto, APPROVED_AUTOMATIC, "caso {label}");
        assert_eq!(
            scenario.status(conversa),
            poph_store::ConversationStatus::Transferred
        );
    }
}

#[tokio::test]
async fn guidance_disabled_not_does_second_question() {
    let scenario = Scenario::new("orientacao-desligada", true);
    let conversa = scenario.conversa();
    let jev = jev(G1_TRANSFER, 0.95, Some((INGESTION_RISK, 0.9)));
    let guidance = HandoffGuidance {
        ativo: false,
        ..HandoffGuidance::default()
    };
    let engine = scenario.engine(Some(engine_judge(Arc::clone(&jev))), guidance);

    let gatilho = scenario.say(conversa, "quero falar com alguém");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.texto, APPROVED_AUTOMATIC);
    assert_eq!(jev.requests().len(), 1, "só o lote do turno foi chamado");
    assert_eq!(jev.calls(), 1);
}

#[tokio::test]
async fn text_empty_configuration_not_guides() {
    let scenario = Scenario::new("texto-vazio", true);
    let conversa = scenario.conversa();
    let jev = jev(G1_TRANSFER, 0.95, Some((INGESTION_RISK, 0.9)));
    let guidance = HandoffGuidance::default().with_text(INGESTION_RISK, "");
    let engine = scenario.engine(Some(engine_judge(jev)), guidance);

    let gatilho = scenario.say(conversa, "meu filho engoliu o veneno");
    let summary = scenario.turno(&engine, conversa, gatilho).await;
    assert_eq!(summary.texto, APPROVED_AUTOMATIC);
}

#[tokio::test]
async fn g1_disabled_not_transfers() {
    let scenario = Scenario::new("g1-desligado", true);
    let conversa = scenario.conversa();
    let jev = jev(G1_TRANSFER, 0.99, Some((INGESTION_RISK, 0.99)));
    let evaluator = Arc::new(JevEngine::new(jev, JevGates::default(), "jev-latest"));
    let engine = scenario.engine(Some(evaluator), HandoffGuidance::default());

    let gatilho = scenario.say(conversa, "quero falar com um atendente humano");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, REPLY_MODEL);
    assert_eq!(
        scenario.status(conversa),
        poph_store::ConversationStatus::Open
    );
    assert_eq!(scenario.provedor.received().len(), 1);
    assert!(summary.jev.is_none() || summary.jev.as_ref().unwrap().decisions.is_empty());
}

#[tokio::test]
async fn agent_without_handoff_not_transfers() {
    let scenario = Scenario::new("sem-handoff", false);
    let conversa = scenario.conversa();
    let jev = jev(G1_TRANSFER, 0.99, Some((INGESTION_RISK, 0.99)));
    let engine = scenario.engine(Some(engine_judge(jev)), HandoffGuidance::default());

    let gatilho = scenario.say(conversa, "quero falar com um atendente humano");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.texto, REPLY_MODEL);
    assert_eq!(
        scenario.status(conversa),
        poph_store::ConversationStatus::Open
    );
}

#[tokio::test]
async fn g1_continue_answers_normally() {
    let scenario = Scenario::new("continuar", true);
    let conversa = scenario.conversa();
    let jev = jev(G1_CONTINUE, 0.99, None);
    let engine = scenario.engine(Some(engine_judge(jev)), HandoffGuidance::default());

    let gatilho = scenario.say(conversa, "qual é o prazo de entrega?");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, REPLY_MODEL);
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
            .audit()
            .iter()
            .any(|registro| registro.acao == "transfer")
    );
}

#[tokio::test]
async fn transfer_keeps_department_agent() {
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
    let engine = scenario.engine(Some(engine_judge(jev)), HandoffGuidance::default());
    let gatilho = scenario.say(conversa, "tem uma aranha no quarto");
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
async fn second_question_not_uses_cache_batch() {
    let scenario = Scenario::new("cache", true);
    let conversa = scenario.conversa();
    let jev = jev(G1_TRANSFER, 0.95, Some((INGESTION_RISK, 0.9)));
    let evaluator = engine_judge(Arc::clone(&jev));
    let engine = scenario.engine(Some(evaluator), HandoffGuidance::default());

    let gatilho = scenario.say(conversa, "engoliu o produto");
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
