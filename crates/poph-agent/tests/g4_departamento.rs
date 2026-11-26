use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::turno::{FuturePause, NoopObserver, Pause, TurnEngine, TurnInput, TurnOptions};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, Script, Usage};
use poph_jev::{
    BatchResponse, FakeJev, G1_TRANSFER, HandoffGuidance, IdGate, JevEngine, JevRequest,
    JevResponse,
};
use poph_store::{
    AgentChange, DepartmentChange, Direction, NewAgent, NewConversation, NewDepartment, NewMessage,
    NewOrg, Page, Role, Status, Store, TurnState,
};
use poph_types::{AgentId, ConversationId, DepartmentId, JevGates, MessageId, OrgId};

const RESPOSTA_DO_MODELO: &str = "Olá! Como posso ajudar?";

struct SemEspera;

impl Pause for SemEspera {
    fn wait(&self, _duracao: Duration) -> FuturePause {
        Box::pin(async {})
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
    fn new(label: &str) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-hr32-{label}-{}-{}",
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
        let current = store.agents().get(org, agente).expect("lê o agente");
        store
            .agents()
            .update(
                org,
                agente,
                current.version,
                AgentChange {
                    handoff_active: Some(true),
                    ..AgentChange::default()
                },
            )
            .expect("liga o handoff");
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

    fn departamento(&self, nome: &str) -> DepartmentId {
        self.store
            .departments()
            .create(
                self.org,
                NewDepartment {
                    name: nome.to_owned(),
                    description: String::new(),
                },
            )
            .expect("cria o departamento")
            .id
    }

    fn departamento_em(&self, org: OrgId, nome: &str) -> DepartmentId {
        self.store
            .departments()
            .create(
                org,
                NewDepartment {
                    name: nome.to_owned(),
                    description: String::new(),
                },
            )
            .expect("cria o departamento")
            .id
    }

    fn outra_org(&self) -> OrgId {
        self.store
            .orgs()
            .create(NewOrg {
                name: "Outra org".to_owned(),
                slug: format!("outra-{}", AgentId::generate()),
                config_json: "{}".to_owned(),
            })
            .expect("cria a outra org")
            .id
    }

    fn desativar(&self, departamento: DepartmentId) {
        let current = self
            .store
            .departments()
            .get(self.org, departamento)
            .expect("lê o departamento");
        self.store
            .departments()
            .update(
                self.org,
                departamento,
                current.version,
                DepartmentChange {
                    status: Some(Status::Inactive),
                    ..DepartmentChange::default()
                },
            )
            .expect("desativa o departamento");
    }

    fn apagar(&self, departamento: DepartmentId) {
        self.store
            .departments()
            .delete(self.org, departamento)
            .expect("apaga o departamento");
    }

    fn definir_padrao(&self, departamento: DepartmentId) {
        let current = self
            .store
            .agents()
            .get(self.org, self.agente)
            .expect("agente");
        self.store
            .agents()
            .update(
                self.org,
                self.agente,
                current.version,
                AgentChange {
                    handoff_department: Some(departamento),
                    ..AgentChange::default()
                },
            )
            .expect("define o departamento de transferência");
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

    fn engine(&self, jev: Option<Arc<JevEngine>>) -> TurnEngine {
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
        .with_handoff(HandoffGuidance::default());
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
    ) -> poph_agent::turno::TurnSummary {
        engine
            .run(
                TurnInput::new(self.org, conversa, gatilho),
                &mut NoopObserver,
            )
            .await
            .expect("o turno roda")
    }

    fn conversa_gravada(&self, conversa: ConversationId) -> poph_store::Conversa {
        self.store
            .conversations()
            .get(self.org, conversa)
            .expect("lê a conversa")
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

    fn auditoria_da_transferencia(&self) -> serde_json::Value {
        let registro = self
            .store
            .audit()
            .list(self.org, Page::with_limit(50))
            .expect("lê a auditoria")
            .into_iter()
            .find(|registro| registro.acao == "transfer")
            .expect("a transferência é auditada");
        serde_json::from_str(&registro.detail_json).expect("detalhe é JSON")
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

fn response(choice: &str, confidence: f64) -> JevResponse {
    let mut item = JevResponse {
        choice: Some(choice.to_owned()),
        confidence,
        ..JevResponse::default()
    };
    item.probabilities.insert(choice.to_owned(), confidence);
    item
}

fn jev(departamento: Option<(&'static str, f64)>, veio_o_g4: bool) -> Arc<FakeJev> {
    Arc::new(FakeJev::new(move |request: &JevRequest| {
        let mut respostas = BTreeMap::new();
        if request.questions.contains_key(IdGate::G1.question_id()) {
            respostas.insert(
                IdGate::G1.question_id().to_owned(),
                response(G1_TRANSFER, 0.96),
            );
        }
        if veio_o_g4
            && request.questions.contains_key(IdGate::G4.question_id())
            && let Some((nome, confidence)) = departamento
        {
            respostas.insert(
                IdGate::G4.question_id().to_owned(),
                response(nome, confidence),
            );
        }
        Ok(BatchResponse {
            answers: respostas,
            tokens: 11,
            ..BatchResponse::default()
        })
    }))
}

fn motor_jev(jev: Arc<FakeJev>, com_g4: bool) -> Arc<JevEngine> {
    let gates = JevGates {
        g1: true,
        g4: com_g4,
        ..JevGates::default()
    };
    Arc::new(JevEngine::new(jev, gates, "jev-latest"))
}

#[tokio::test]
async fn g4_manda_a_conversa_para_o_departamento_escolhido() {
    let scenario = Scenario::new("escolha-valida");
    let default = scenario.departamento("Atendimento humano");
    let escolhido = scenario.departamento("Financeiro");
    scenario.definir_padrao(default);
    let conversa = scenario.conversa();
    let jev = jev(Some(("Financeiro", 0.93)), true);
    let engine = scenario.engine(Some(motor_jev(jev, true)));

    let gatilho = scenario.falar(conversa, "vocês cobraram duas vezes no meu cartão");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert!(
        scenario.provedor.received().is_empty(),
        "transferida não chama o modelo"
    );
    let stored = scenario.conversa_gravada(conversa);
    assert_eq!(stored.status, poph_store::ConversationStatus::Transferred);
    assert_eq!(stored.target_department, Some(escolhido));

    let detalhe = scenario.auditoria_da_transferencia();
    assert_eq!(detalhe["departamento_origem"], "g4");
    assert_eq!(detalhe["departamento_escolhido_pelo_jev"], "Financeiro");
    assert_eq!(detalhe["departamento"], escolhido.to_string());
    assert_eq!(detalhe["departamento_motivo"], serde_json::Value::Null);

    let turno = scenario.ultimo_turno(conversa);
    let stored: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    assert_eq!(stored["transferencia"]["origem"], "g4");
    assert_eq!(
        stored["transferencia"]["departamento"],
        escolhido.to_string()
    );
    let g4 = stored["decisoes"]
        .as_array()
        .expect("decisões")
        .iter()
        .find(|decisao| decisao["gate"] == "g4")
        .expect("o G4 decidiu no turno");
    assert_eq!(g4["escolha"], "Financeiro");
}

#[tokio::test]
async fn escolha_igual_ao_padrao_continua_vindo_do_g4() {
    let scenario = Scenario::new("escolha-igual");
    let default = scenario.departamento("Suporte");
    scenario.departamento("Comercial");
    scenario.definir_padrao(default);
    let conversa = scenario.conversa();
    let engine = scenario.engine(Some(motor_jev(jev(Some(("Suporte", 0.9)), true), true)));
    let gatilho = scenario.falar(conversa, "o produto chegou quebrado");
    scenario.turno(&engine, conversa, gatilho).await;

    let detalhe = scenario.auditoria_da_transferencia();
    assert_eq!(detalhe["departamento_origem"], "g4");
    assert_eq!(detalhe["departamento"], default.to_string());
}

#[tokio::test]
async fn departamento_de_outra_org_cai_no_padrao() {
    let scenario = Scenario::new("outra-org");
    let default = scenario.departamento("Atendimento humano");
    scenario.departamento("Comercial");
    scenario.definir_padrao(default);
    scenario.departamento_em(scenario.outra_org(), "Jurídico");

    let conversa = scenario.conversa();
    let engine = scenario.engine(Some(motor_jev(jev(Some(("Jurídico", 0.97)), true), true)));
    let gatilho = scenario.falar(conversa, "quero falar do contrato");
    scenario.turno(&engine, conversa, gatilho).await;

    let stored = scenario.conversa_gravada(conversa);
    assert_eq!(stored.status, poph_store::ConversationStatus::Transferred);
    assert_eq!(stored.target_department, Some(default));
    let detalhe = scenario.auditoria_da_transferencia();
    assert_eq!(detalhe["departamento_origem"], "padrao");
    assert!(
        detalhe["departamento_motivo"]
            .as_str()
            .unwrap_or_default()
            .contains("não é um departamento ativo"),
        "motivo: {detalhe}"
    );
}

#[tokio::test]
async fn departamento_inativo_cai_no_padrao() {
    let scenario = Scenario::new("inativo");
    let default = scenario.departamento("Atendimento humano");
    let desativado = scenario.departamento("Fiscal");
    scenario.departamento("Comercial");
    scenario.definir_padrao(default);
    scenario.desativar(desativado);

    let conversa = scenario.conversa();
    let engine = scenario.engine(Some(motor_jev(jev(Some(("Fiscal", 0.95)), true), true)));
    let gatilho = scenario.falar(conversa, "preciso da nota fiscal da compra");
    scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(
        scenario.conversa_gravada(conversa).target_department,
        Some(default)
    );
}

#[tokio::test]
async fn departamento_apagado_cai_no_padrao() {
    let scenario = Scenario::new("apagado");
    let default = scenario.departamento("Atendimento humano");
    let apagado = scenario.departamento("Logística");
    scenario.departamento("Comercial");
    scenario.definir_padrao(default);
    scenario.apagar(apagado);

    let conversa = scenario.conversa();
    let engine = scenario.engine(Some(motor_jev(jev(Some(("Logística", 0.95)), true), true)));
    let gatilho = scenario.falar(conversa, "meu pedido não chegou");
    scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(
        scenario.conversa_gravada(conversa).target_department,
        Some(default)
    );
}

#[test]
fn o_banco_nao_deixa_dois_departamentos_com_o_mesmo_nome() {
    let scenario = Scenario::new("nome-unico");
    scenario.departamento("Financeiro");
    let error = scenario
        .store
        .departments()
        .create(
            scenario.org,
            NewDepartment {
                name: "FINANCEIRO".to_owned(),
                description: String::new(),
            },
        )
        .expect_err("nome repetido (sem diferenciar maiúsculas) devia ser recusado");
    assert!(
        error.to_string().contains("já existe"),
        "erro inesperado: {error}"
    );
}

#[tokio::test]
async fn abstencao_do_g4_cai_no_padrao_sem_travar_o_turno() {
    type Caso = (&'static str, Option<(&'static str, f64)>, bool);
    let casos: [Caso; 3] = [
        ("fora-das-opcoes", Some(("Marketing", 0.99)), true),
        ("sem-resposta", None, false),
        ("confianca-baixa", Some(("Comercial", 0.41)), true),
    ];
    for (label, departamento, veio) in casos {
        let scenario = Scenario::new(label);
        let default = scenario.departamento("Atendimento humano");
        scenario.departamento("Comercial");
        scenario.definir_padrao(default);
        let conversa = scenario.conversa();
        let engine = scenario.engine(Some(motor_jev(jev(departamento, veio), true)));
        let gatilho = scenario.falar(conversa, "quero falar com alguém sobre o pedido");
        let summary = scenario.turno(&engine, conversa, gatilho).await;

        assert_eq!(summary.estado, TurnState::Completed, "caso {label}");
        assert_eq!(
            scenario.conversa_gravada(conversa).target_department,
            Some(default),
            "caso {label}"
        );
        let detalhe = scenario.auditoria_da_transferencia();
        assert_eq!(detalhe["departamento_origem"], "padrao", "caso {label}");
        assert!(
            detalhe["departamento_motivo"].as_str().is_some(),
            "caso {label} precisa registrar o motivo da recusa"
        );
    }
}

#[tokio::test]
async fn jev_fora_do_ar_nao_trava_o_turno() {
    let scenario = Scenario::new("jev-fora");
    let default = scenario.departamento("Atendimento humano");
    scenario.departamento("Comercial");
    scenario.definir_padrao(default);
    let conversa = scenario.conversa();

    let jev = Arc::new(
        FakeJev::with_choices(BTreeMap::from([(
            IdGate::G1.question_id().to_owned(),
            G1_TRANSFER.to_owned(),
        )]))
        .with_delay(Duration::from_millis(400)),
    );
    let evaluator = Arc::new(
        JevEngine::new(
            jev,
            JevGates {
                g1: true,
                g4: true,
                ..JevGates::default()
            },
            "jev-latest",
        )
        .with_options(poph_jev::JevOptions {
            batch_timeout: Duration::from_millis(50),
            ..poph_jev::JevOptions::default()
        }),
    );
    let engine = scenario.engine(Some(evaluator));
    let gatilho = scenario.falar(conversa, "quero falar com um atendente");
    let summary = scenario.turno(&engine, conversa, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(
        scenario.conversa_gravada(conversa).status,
        poph_store::ConversationStatus::Open
    );
    assert_eq!(summary.texto, RESPOSTA_DO_MODELO);
}

#[tokio::test]
async fn g4_desligado_e_o_comportamento_de_hoje() {
    let scenario = Scenario::new("desligado");
    let default = scenario.departamento("Atendimento humano");
    scenario.departamento("Comercial");
    scenario.definir_padrao(default);
    let conversa = scenario.conversa();
    let jev = jev(Some(("Comercial", 0.99)), true);
    let evaluator = motor_jev(Arc::clone(&jev), false);
    let engine = scenario.engine(Some(evaluator));

    let gatilho = scenario.falar(conversa, "quero falar com um atendente");
    scenario.turno(&engine, conversa, gatilho).await;

    let stored = scenario.conversa_gravada(conversa);
    assert_eq!(stored.status, poph_store::ConversationStatus::Transferred);
    assert_eq!(stored.target_department, Some(default));

    let pedidos = jev.requests();
    assert!(
        pedidos
            .iter()
            .all(|request| !request.questions.contains_key(IdGate::G4.question_id())),
        "G4 desligado não pode perguntar o departamento"
    );
    let detalhe = scenario.auditoria_da_transferencia();
    assert_eq!(detalhe["departamento"], default.to_string());
    assert_eq!(detalhe["departamento_origem"], "padrao");
    assert_eq!(
        detalhe["departamento_escolhido_pelo_jev"],
        serde_json::Value::Null
    );

    let turno = scenario.ultimo_turno(conversa);
    let stored: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    let gates: Vec<&str> = stored["decisoes"]
        .as_array()
        .expect("decisões")
        .iter()
        .filter_map(|decisao| decisao["gate"].as_str())
        .collect();
    assert_eq!(gates, vec!["g1"], "só o G1 decide com o G4 desligado");
    assert_eq!(stored["transferencia"]["origem"], "padrao");
    assert_eq!(stored["transferencia"]["departamento"], default.to_string());
    assert_eq!(
        stored["transferencia"]["motivo"],
        serde_json::Value::Null,
        "G4 desligado não inventa motivo de fallback"
    );
}

#[tokio::test]
async fn sem_padrao_configurado_transfere_sem_departamento() {
    let scenario = Scenario::new("sem-padrao");
    scenario.departamento("Comercial");
    let conversa = scenario.conversa();
    for (label, com_g4) in [("desligado", false), ("ligado-recusado", true)] {
        let jev = jev(Some(("Marketing", 0.97)), true);
        let engine = scenario.engine(Some(motor_jev(jev, com_g4)));
        let gatilho = scenario.falar(conversa, "quero falar com uma pessoa");
        scenario.turno(&engine, conversa, gatilho).await;
        let stored = scenario.conversa_gravada(conversa);
        assert_eq!(
            stored.status,
            poph_store::ConversationStatus::Transferred,
            "caso {label}"
        );
        assert_eq!(stored.target_department, None, "caso {label}");
        scenario
            .store
            .conversations()
            .reopen_audited(scenario.org, conversa, stored.version, "teste".to_owned())
            .expect("reabre");
    }
}

#[tokio::test]
async fn a_escolha_do_departamento_nao_muda_o_texto_ao_cliente() {
    let scenario = Scenario::new("texto");
    let default = scenario.departamento("Atendimento humano");
    scenario.departamento("Financeiro");
    scenario.definir_padrao(default);
    let conversa = scenario.conversa();
    let engine = scenario.engine(Some(motor_jev(jev(Some(("Financeiro", 0.95)), true), true)));
    let gatilho = scenario.falar(conversa, "quero falar com um atendente sobre a cobrança");
    let summary = scenario.turno(&engine, conversa, gatilho).await;
    assert_eq!(
        summary.texto,
        HandoffGuidance::default().mensagem_automatica
    );
}
