use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::turno::{
    CONFIRMATION_WINDOW, FuturePause, NoopObserver, Pause, TurnEngine, TurnInput, TurnOptions,
    TurnSummary, window_with_g2,
};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, Message, Script, Usage};
use poph_jev::{
    FakeJev, G1_CONTINUE, G2_PROCESS, G2_WAIT, IdGate, JevEngine, JevOptions, JevState,
};
use poph_store::{
    AgentChange, Direction, NewAgent, NewConversation, NewMessage, NewOrg, Role, Store, TurnState,
};
use poph_types::{AgentId, ConversationId, JevGates, MessageId, OrgId};

#[derive(Debug, Clone)]
struct Continuacao {
    instante_ms: u64,
    texto: String,
}

struct PausaVirtual {
    store: Arc<Store>,
    org: OrgId,
    conversa: ConversationId,
    now_ms: Mutex<u64>,
    esperas: Mutex<Vec<Duration>>,
    continuacoes: Mutex<Vec<Continuacao>>,
}

impl PausaVirtual {
    fn new(store: Arc<Store>, org: OrgId, conversa: ConversationId) -> Self {
        Self {
            store,
            org,
            conversa,
            now_ms: Mutex::new(0),
            esperas: Mutex::new(Vec::new()),
            continuacoes: Mutex::new(Vec::new()),
        }
    }

    fn schedule(&self, em_ms: u64, texto: &str) {
        self.continuacoes.lock().expect("agenda").push(Continuacao {
            instante_ms: em_ms,
            texto: texto.to_owned(),
        });
    }

    fn esperas(&self) -> Vec<Duration> {
        self.esperas.lock().expect("esperas").clone()
    }

    fn espera_total(&self) -> Duration {
        self.esperas().iter().copied().sum()
    }

    fn avancar(&self, duration: Duration) {
        let mut now = self.now_ms.lock().expect("relógio");
        *now = now.saturating_add(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX));
        let now = *now;
        let mut continuacoes = self.continuacoes.lock().expect("continuações");
        let mut pending = Vec::new();
        for continuacao in continuacoes.drain(..) {
            if continuacao.instante_ms <= now {
                self.store
                    .messages()
                    .write(
                        self.org,
                        NewMessage {
                            conversation_id: self.conversa,
                            direction: Direction::Inbound,
                            role: Role::User,
                            texto: continuacao.texto,
                            attachments_json: "[]".to_owned(),
                            external_id: None,
                            turn_id: None,
                            origin: poph_store::Origin::Agente,
                            author_name: None,
                        },
                    )
                    .expect("grava a continuação do cliente");
            } else {
                pending.push(continuacao);
            }
        }
        *continuacoes = pending;
    }
}

impl Pause for PausaVirtual {
    fn wait(&self, duration: Duration) -> FuturePause {
        self.esperas.lock().expect("esperas").push(duration);
        self.avancar(duration);
        Box::pin(async {})
    }
}

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-hr30-{label}-{}-{}",
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
        Self {
            folder,
            store,
            org,
            agente,
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
                    canal: "whatsapp".to_owned(),
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

    fn engine(
        &self,
        jev: Option<Arc<JevEngine>>,
        pausa: Arc<PausaVirtual>,
        provedor: Arc<FakeProvider>,
    ) -> TurnEngine {
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
            provedor,
            TurnOptions::default(),
            MetricsTurn::own(),
        )
        .with_pause(pausa);
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
        engine
            .run(
                TurnInput::new(self.org, conversa, gatilho),
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

fn provedor() -> Arc<FakeProvider> {
    Arc::new(FakeProvider::with_scripts(vec![Script::Events(vec![
        LlmEvent::Texto("Claro, como posso ajudar?".to_owned()),
        LlmEvent::Usage(Usage {
            input: 10,
            saida: 5,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])]))
}

fn so_g2() -> JevGates {
    JevGates {
        g2: true,
        ..JevGates::default()
    }
}

fn motor_jev(jev: Arc<FakeJev>, gates: JevGates) -> Arc<JevEngine> {
    Arc::new(JevEngine::new(jev, gates, "jev-latest"))
}

fn motor_jev_com(jev: Arc<FakeJev>, gates: JevGates, options: JevOptions) -> Arc<JevEngine> {
    Arc::new(JevEngine::new(jev, gates, "jev-latest").with_options(options))
}

fn choice(pergunta: &str, choice: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(pergunta.to_owned(), choice.to_owned())])
}

const JANELA: Duration = Duration::from_secs(2);
const TOLERANCIA: Duration = Duration::from_millis(150);

fn perto(medido: Duration, expected: Duration) -> bool {
    medido <= expected && medido + TOLERANCIA >= expected
}

#[tokio::test]
async fn esperar_mais_estica_a_janela_e_agrupa_a_mensagem_seguinte() {
    let scenario = Scenario::new("esperar");
    let conversa = scenario.conversa();
    let pausa = Arc::new(PausaVirtual::new(
        Arc::clone(&scenario.store),
        scenario.org,
        conversa,
    ));
    pausa.schedule(3000, "Rua das Flores, 120 — apto 3");

    let jev = Arc::new(FakeJev::with_choices(choice("g2_esperar", G2_WAIT)));
    let provedor = provedor();
    let engine = scenario.engine(
        Some(motor_jev(Arc::clone(&jev), so_g2())),
        Arc::clone(&pausa),
        Arc::clone(&provedor),
    );

    let gatilho = scenario.falar(conversa, "vou te passar meu endereço");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert!(
        perto(pausa.espera_total(), Duration::from_secs(4)),
        "espera devia ser uma janela esticada (4 s), foi {:?}",
        pausa.espera_total()
    );
    assert_eq!(
        summary.messages.len(),
        2,
        "a mensagem seguinte entra na mesma janela"
    );
    assert_eq!(provedor.received().len(), 1, "uma chamada ao modelo");
    let request = &provedor.received()[0];
    let do_cliente = request
        .messages
        .iter()
        .filter(|mensagem| matches!(mensagem, Message::User(_)))
        .count();
    assert_eq!(
        do_cliente, 2,
        "o prompt leva as duas mensagens do cliente no mesmo turno"
    );
    assert_eq!(
        summary.jev.as_ref().and_then(|jev| jev.choice(IdGate::G2)),
        Some(G2_WAIT)
    );
}

#[tokio::test]
async fn mensagem_nova_durante_a_espera_refaz_o_lote() {
    let scenario = Scenario::new("refaz");
    let conversa = scenario.conversa();
    let pausa = Arc::new(PausaVirtual::new(
        Arc::clone(&scenario.store),
        scenario.org,
        conversa,
    ));
    pausa.schedule(3000, "o produto que eu quero é o azul");

    let jev = Arc::new(FakeJev::with_choices(choice("g2_esperar", G2_WAIT)));
    let engine = scenario.engine(
        Some(motor_jev(Arc::clone(&jev), so_g2())),
        Arc::clone(&pausa),
        provedor(),
    );

    let gatilho = scenario.falar(conversa, "queria saber do");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.messages.len(), 2);
    assert_eq!(
        jev.calls(),
        2,
        "o lote é refeito quando o cliente fala durante a espera"
    );
    let last = jev.requests().pop().expect("há pedido");
    assert_eq!(
        last.estado["conversa"]["mensagens"][1]["texto"],
        "o produto que eu quero é o azul"
    );
}

#[tokio::test]
async fn processar_agora_encurta_a_espera() {
    let scenario = Scenario::new("processar");
    let conversa = scenario.conversa();
    let pausa = Arc::new(PausaVirtual::new(
        Arc::clone(&scenario.store),
        scenario.org,
        conversa,
    ));
    let jev = Arc::new(FakeJev::with_choices(choice("g2_esperar", G2_PROCESS)));
    let engine = scenario.engine(
        Some(motor_jev(Arc::clone(&jev), so_g2())),
        Arc::clone(&pausa),
        provedor(),
    );

    let gatilho = scenario.falar(conversa, "vocês atendem no domingo?");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert!(
        pausa.espera_total() < JANELA,
        "processar agora tem de esperar menos que a janela fixa, esperou {:?}",
        pausa.espera_total()
    );
    assert!(
        pausa.espera_total() <= CONFIRMATION_WINDOW
            && pausa.espera_total() + TOLERANCIA >= CONFIRMATION_WINDOW,
        "a espera devia ser a confirmação curta, foi {:?}",
        pausa.espera_total()
    );
    assert_eq!(jev.calls(), 1, "sem mensagem nova, um lote só");
    assert_eq!(summary.messages.len(), 1);
}

#[tokio::test]
async fn gate_desligado_mantem_a_janela_fixa() {
    let scenario = Scenario::new("desligado");
    let conversa = scenario.conversa();
    let pausa = Arc::new(PausaVirtual::new(
        Arc::clone(&scenario.store),
        scenario.org,
        conversa,
    ));
    let gates = JevGates {
        g1: true,
        ..JevGates::default()
    };
    let jev = Arc::new(FakeJev::with_choices(choice("g1_transferir", G1_CONTINUE)));
    let engine = scenario.engine(
        Some(motor_jev(Arc::clone(&jev), gates)),
        Arc::clone(&pausa),
        provedor(),
    );

    let gatilho = scenario.falar(conversa, "bom dia");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(pausa.esperas().len(), 1);
    assert!(
        perto(pausa.espera_total(), JANELA),
        "{:?}",
        pausa.espera_total()
    );
    assert_eq!(jev.calls(), 1, "o G1 continua sendo consultado");
    assert!(summary.jev.is_some());
}

#[tokio::test]
async fn sem_jev_a_janela_e_a_de_hoje() {
    let scenario = Scenario::new("sem-jev");
    let conversa = scenario.conversa();
    let pausa = Arc::new(PausaVirtual::new(
        Arc::clone(&scenario.store),
        scenario.org,
        conversa,
    ));
    let engine = scenario.engine(None, Arc::clone(&pausa), provedor());

    let gatilho = scenario.falar(conversa, "bom dia");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert!(
        perto(pausa.espera_total(), JANELA),
        "{:?}",
        pausa.espera_total()
    );
    assert!(summary.jev.is_none());
}

#[tokio::test]
async fn falha_do_jev_mantem_a_janela_fixa() {
    let scenario = Scenario::new("falha");
    let conversa = scenario.conversa();
    let pausa = Arc::new(PausaVirtual::new(
        Arc::clone(&scenario.store),
        scenario.org,
        conversa,
    ));
    let jev = Arc::new(FakeJev::failing(poph_jev::JevError::Status {
        code: 503,
        motivo: "Service Unavailable".to_owned(),
    }));
    let engine = scenario.engine(
        Some(motor_jev(Arc::clone(&jev), so_g2())),
        Arc::clone(&pausa),
        provedor(),
    );

    let gatilho = scenario.falar(conversa, "me deixa ver aqui");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert!(
        perto(pausa.espera_total(), JANELA),
        "{:?}",
        pausa.espera_total()
    );
    let registro = summary.jev.expect("o turno registra a tentativa");
    assert!(registro.failed());
    assert_eq!(
        registro.choice(IdGate::G2),
        Some(G2_PROCESS),
        "fallback do G2"
    );
}

#[tokio::test]
async fn escolha_fora_das_opcoes_mantem_a_janela_fixa() {
    let scenario = Scenario::new("ambigua");
    let conversa = scenario.conversa();
    let pausa = Arc::new(PausaVirtual::new(
        Arc::clone(&scenario.store),
        scenario.org,
        conversa,
    ));
    let jev = Arc::new(FakeJev::with_choices(choice("g2_esperar", "talvez")));
    let engine = scenario.engine(
        Some(motor_jev(Arc::clone(&jev), so_g2())),
        Arc::clone(&pausa),
        provedor(),
    );

    let gatilho = scenario.falar(conversa, "oi");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert!(
        perto(pausa.espera_total(), JANELA),
        "{:?}",
        pausa.espera_total()
    );
}

#[tokio::test]
async fn confianca_baixa_mantem_a_janela_fixa() {
    let scenario = Scenario::new("confianca");
    let conversa = scenario.conversa();
    let pausa = Arc::new(PausaVirtual::new(
        Arc::clone(&scenario.store),
        scenario.org,
        conversa,
    ));
    let jev = Arc::new(FakeJev::new(|request: &poph_jev::JevRequest| {
        let mut respostas = BTreeMap::new();
        for id in request.questions.keys() {
            let mut response = poph_jev::JevResponse {
                choice: Some(G2_WAIT.to_owned()),
                confidence: 0.4,
                ..poph_jev::JevResponse::default()
            };
            response.probabilities.insert(G2_WAIT.to_owned(), 0.4);
            respostas.insert(id.clone(), response);
        }
        Ok(poph_jev::BatchResponse {
            answers: respostas,
            ..poph_jev::BatchResponse::default()
        })
    }));
    let engine = scenario.engine(
        Some(motor_jev(Arc::clone(&jev), so_g2())),
        Arc::clone(&pausa),
        provedor(),
    );

    let gatilho = scenario.falar(conversa, "me deixa ver");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert!(
        perto(pausa.espera_total(), JANELA),
        "confiança 0,4 é fallback: janela fixa, foi {:?}",
        pausa.espera_total()
    );
}

#[tokio::test]
async fn prazo_do_jev_mantem_a_janela_fixa() {
    let scenario = Scenario::new("prazo");
    let conversa = scenario.conversa();
    let pausa = Arc::new(PausaVirtual::new(
        Arc::clone(&scenario.store),
        scenario.org,
        conversa,
    ));
    let jev = Arc::new(
        FakeJev::with_choices(choice("g2_esperar", G2_WAIT)).with_delay(Duration::from_millis(250)),
    );
    let options = JevOptions {
        batch_timeout: Duration::from_millis(50),
        ..JevOptions::default()
    };
    let engine = scenario.engine(
        Some(motor_jev_com(Arc::clone(&jev), so_g2(), options)),
        Arc::clone(&pausa),
        provedor(),
    );

    let gatilho = scenario.falar(conversa, "peraí");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert!(
        perto(pausa.espera_total(), JANELA),
        "{:?}",
        pausa.espera_total()
    );
    let registro = summary.jev.expect("registra a tentativa");
    assert!(registro.failed());
}

#[tokio::test]
async fn conversa_transferida_nao_responde_nem_consulta_o_jev() {
    let scenario = Scenario::new("transferida");
    let conversa = scenario.conversa();
    scenario
        .store
        .conversations()
        .transfer(
            scenario.org,
            conversa,
            None,
            Some("teste".to_owned()),
            "{}".to_owned(),
        )
        .expect("transfere a conversa");

    let pausa = Arc::new(PausaVirtual::new(
        Arc::clone(&scenario.store),
        scenario.org,
        conversa,
    ));
    let jev = Arc::new(FakeJev::with_choices(choice("g2_esperar", G2_WAIT)));
    let provedor = provedor();
    let engine = scenario.engine(
        Some(motor_jev(Arc::clone(&jev), so_g2())),
        Arc::clone(&pausa),
        Arc::clone(&provedor),
    );

    let gatilho = scenario.falar(conversa, "oi, ainda estou aqui");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Skipped);
    assert!(summary.texto.is_empty());
    assert_eq!(jev.calls(), 0, "nem consulta o Jev");
    assert!(provedor.received().is_empty(), "nem chama o modelo");
    assert!(pausa.esperas().is_empty(), "nem espera a janela");
    let messages = scenario
        .store
        .messages()
        .last_set(scenario.org, conversa, 10)
        .expect("lê as mensagens");
    assert!(messages.iter().any(|mensagem| mensagem.id == gatilho));
}

#[test]
fn janela_com_g2_respeita_o_teto_e_o_fallback() {
    let base = Duration::from_secs(2);
    let cap = Duration::from_secs(8);
    let confirmacao = CONFIRMATION_WINDOW;

    assert_eq!(
        window_with_g2(base, cap, confirmacao, Some(G2_WAIT)),
        Duration::from_secs(4)
    );
    assert_eq!(
        window_with_g2(Duration::from_secs(6), cap, confirmacao, Some(G2_WAIT)),
        cap,
        "esticar nunca passa do teto do PRD §6.3"
    );
    assert_eq!(
        window_with_g2(base, cap, confirmacao, Some(G2_PROCESS)),
        confirmacao
    );
    assert_eq!(window_with_g2(base, cap, confirmacao, None), base);
    assert_eq!(
        window_with_g2(base, cap, confirmacao, Some("opcao_desconhecida")),
        base
    );
}

#[derive(Debug, serde::Deserialize)]
struct CasoDoConjunto {
    estado: JevState,
    #[serde(rename = "esperado")]
    expected: String,
    #[serde(default)]
    continuacao: Option<ContinuacaoDoConjunto>,
}

#[derive(Debug, serde::Deserialize)]
struct ContinuacaoDoConjunto {
    texto: String,
    atraso_ms: u64,
}

fn ler_conjunto() -> Vec<CasoDoConjunto> {
    let caminho = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/jev/g2.jsonl");
    let texto = std::fs::read_to_string(&caminho).expect("lê tests/jev/g2.jsonl");
    texto
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| serde_json::from_str(line).expect("caso do conjunto em JSON"))
        .collect()
}

fn texto_do_caso(caso: &CasoDoConjunto) -> String {
    caso.estado
        .messages
        .iter()
        .rev()
        .find(|mensagem| mensagem.role == poph_jev::JevRole::Customer)
        .map(|mensagem| mensagem.texto.clone())
        .unwrap_or_default()
}

async fn rodar_caso(
    scenario: &Scenario,
    conversa: ConversationId,
    caso: &CasoDoConjunto,
    decisao: Option<&str>,
) -> (Arc<PausaVirtual>, TurnSummary) {
    let pausa = Arc::new(PausaVirtual::new(
        Arc::clone(&scenario.store),
        scenario.org,
        conversa,
    ));
    if let Some(continuacao) = &caso.continuacao {
        pausa.schedule(continuacao.atraso_ms, &continuacao.texto);
    }
    let escolhas = match decisao {
        Some(decidido) => choice("g2_esperar", decidido),
        None => BTreeMap::new(),
    };
    let jev = Arc::new(FakeJev::with_choices(escolhas));
    let engine = scenario.engine(
        Some(motor_jev(Arc::clone(&jev), so_g2())),
        Arc::clone(&pausa),
        provedor(),
    );
    let gatilho = scenario.falar(conversa, &texto_do_caso(caso));
    let summary = scenario.turno(&engine, conversa, gatilho).await;
    (pausa, summary)
}

#[tokio::test]
async fn latencia_menor_nas_completas_sem_partir_mais_frases() {
    let casos = ler_conjunto();
    assert!(
        casos.len() >= 60,
        "o conjunto do G2 precisa de pelo menos 60 casos (tem {})",
        casos.len()
    );
    let scenario = Scenario::new("latencia");

    let completas: Vec<&CasoDoConjunto> = casos
        .iter()
        .filter(|caso| caso.expected == G2_PROCESS)
        .collect();
    assert!(
        completas.len() >= 30,
        "casos completos: {}",
        completas.len()
    );
    let mut soma_com = Duration::ZERO;
    let mut soma_sem = Duration::ZERO;
    for caso in &completas {
        let conversa = scenario.conversa();
        let (pausa, summary) = rodar_caso(&scenario, conversa, caso, Some(G2_PROCESS)).await;
        assert_eq!(summary.estado, TurnState::Completed);
        soma_com += pausa.espera_total();

        let conversa = scenario.conversa();
        let (pausa, summary) = rodar_caso(&scenario, conversa, caso, None).await;
        assert_eq!(summary.estado, TurnState::Completed);
        soma_sem += pausa.espera_total();
    }
    let media_com = soma_com / u32::try_from(completas.len()).expect("casos");
    let media_sem = soma_sem / u32::try_from(completas.len()).expect("casos");
    println!(
        "mensagens completas ({}): espera média com G2 {:.0} ms × sem G2 {:.0} ms",
        completas.len(),
        media_com.as_secs_f64() * 1000.0,
        media_sem.as_secs_f64() * 1000.0
    );
    assert!(
        media_com < JANELA,
        "com o G2 a espera devia ser menor que a janela fixa ({:?}), foi {:?}",
        JANELA,
        media_com
    );
    assert!(media_com < media_sem);

    let com_continuacao: Vec<&CasoDoConjunto> = casos
        .iter()
        .filter(|caso| caso.continuacao.is_some())
        .collect();
    assert!(
        com_continuacao.len() >= 10,
        "casos com continuação: {}",
        com_continuacao.len()
    );
    let mut partidas_com = 0;
    let mut partidas_sem = 0;
    for caso in &com_continuacao {
        let conversa = scenario.conversa();
        let (_, summary) = rodar_caso(&scenario, conversa, caso, Some(&caso.expected)).await;
        if summary.messages.len() < 2 {
            partidas_com += 1;
        }

        let conversa = scenario.conversa();
        let (_, summary) = rodar_caso(&scenario, conversa, caso, None).await;
        if summary.messages.len() < 2 {
            partidas_sem += 1;
        }
    }
    let total = com_continuacao.len() as f64;
    println!(
        "frases partidas: com G2 {}/{} ({:.0}%) × sem G2 {}/{} ({:.0}%)",
        partidas_com,
        com_continuacao.len(),
        100.0 * f64::from(partidas_com) / total,
        partidas_sem,
        com_continuacao.len(),
        100.0 * f64::from(partidas_sem) / total,
    );
    assert!(
        partidas_com <= partidas_sem,
        "o G2 não pode partir mais frases: {partidas_com} × {partidas_sem}"
    );
}
