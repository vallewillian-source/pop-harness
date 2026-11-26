//! Encaixe do Jev no turno (HR-28).
//!
//! O que se prova aqui, e só isso: o turno consulta os gates ligados **uma vez**, grava a decisão em
//! `turns.jev_json`, devolve-a no `ResumoTurno`, respeita a flag do agente e continua respondendo
//! quando o Jev falha. O efeito de cada gate (transferir, esperar, pular a busca) é das tarefas
//! HR-29 a HR-32 e **não** muda nada neste turno.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use poph_agent::base_conhecimento::{BuscadorBase, FuturoBusca, ParametrosBusca};
use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasMemoria, MetricasTurno};
use poph_agent::turno::{
    Entrada, FuturoPausa, Motor, ObservadorNulo, OpcoesTurno, Pausa, ResumoTurno,
};
use poph_core::llm::{FakeProvider, Script};
use poph_jev::{
    G1_CONTINUE, G1_TRANSFER, G4, GateEvaluator, IdGate, JevEngine, JevError, JevFalso, JevMessage,
    JevState,
};
use poph_store::{
    AgentChange, Direction, NewAgent, NewConversation, NewDepartment, NewMessage, NewOrg, Role,
    Store, TurnState,
};
use poph_types::{AgentId, ConversationId, JevGates, MessageId, OrgId};

struct SemEspera;

impl Pausa for SemEspera {
    fn esperar(&self, _duracao: Duration) -> FuturoPausa {
        Box::pin(async {})
    }
}

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
            "poph-hr28-{rotulo}-{}-{}",
            std::process::id(),
            AgentId::generate()
        ));
        let _ = std::fs::remove_dir_all(&pasta);
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
        // Handoff ligado: sem isso o G1 nem se aplica (regra do POP Atende).
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
            pasta,
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

    fn departamento(&self, nome: &str) {
        self.store
            .departments()
            .create(
                self.org,
                NewDepartment {
                    name: nome.to_owned(),
                    description: String::new(),
                },
            )
            .expect("cria o departamento");
    }

    fn ajustar_jev_do_agente(&self, jev_json: &str) {
        let atual = self
            .store
            .agents()
            .get(self.org, self.agente)
            .expect("lê o agente");
        self.store
            .agents()
            .update(
                self.org,
                self.agente,
                atual.version,
                AgentChange {
                    jev_json: Some(jev_json.to_owned()),
                    ..AgentChange::default()
                },
            )
            .expect("ajusta o jev_json do agente");
    }

    fn motor(&self, globais: JevGates, jev: Option<Arc<JevEngine>>) -> Motor {
        let config = ConfigQuente::nova(
            Arc::clone(&self.store),
            OpcoesConfigQuente::default(),
            MetricasMemoria::proprio(),
        );
        let limitador = Arc::new(Limitador::novo(
            OpcoesLimites::default(),
            MetricasMemoria::proprio(),
        ));
        let provedor = Arc::new(FakeProvider::with_scripts(vec![roteiro(
            "Olá! Como posso ajudar?",
        )]));
        let _ = globais;
        let mut motor = Motor::novo(
            Arc::clone(&self.store),
            config,
            limitador,
            provedor,
            OpcoesTurno::default(),
            MetricasTurno::proprio(),
        )
        .com_pausa(Arc::new(SemEspera));
        if let Some(jev) = jev {
            motor = motor.com_jev(jev);
        }
        motor
    }

    async fn turno(
        &self,
        motor: &Motor,
        conversa: ConversationId,
        gatilho: MessageId,
    ) -> ResumoTurno {
        motor
            .executar(
                Entrada::nova(self.org, conversa, gatilho),
                &mut ObservadorNulo,
            )
            .await
            .expect("o turno roda")
    }

    /// Último turno gravado da conversa.
    fn ultimo_turno(&self, conversa: ConversationId) -> poph_store::Turno {
        self.store
            .turns()
            .list(self.org, conversa, poph_store::Page::with_limit(10))
            .expect("lista os turnos")
            .into_iter()
            .next()
            .expect("há um turno gravado")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

fn roteiro(texto: &str) -> Script {
    Script::Events(vec![
        poph_core::llm::LlmEvent::Texto(texto.to_owned()),
        poph_core::llm::LlmEvent::Usage(poph_core::llm::Usage {
            input: 10,
            saida: 5,
        }),
        poph_core::llm::LlmEvent::Fim(poph_core::llm::FinishReason::Completed),
    ])
}

fn so_g1() -> JevGates {
    JevGates {
        g1: true,
        ..JevGates::default()
    }
}

fn escolhas(itens: &[(&str, &str)]) -> BTreeMap<String, String> {
    itens
        .iter()
        .map(|(id, escolha)| ((*id).to_owned(), (*escolha).to_owned()))
        .collect()
}

fn motor_jev(jev: Arc<JevFalso>, globais: JevGates) -> Arc<JevEngine> {
    Arc::new(JevEngine::new(jev, globais, "jev-latest"))
}

/// Aceite: a decisão do gate é consultada uma vez e gravada em `turns.jev_json`.
#[tokio::test]
async fn turno_grava_a_decisao_do_jev_no_registro() {
    let cenario = Cenario::novo("registro");
    let conversa = cenario.conversa();
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g1_transferir",
        G1_CONTINUE,
    )])));
    let motor = cenario.motor(so_g1(), Some(motor_jev(Arc::clone(&jev), so_g1())));

    let gatilho = cenario.falar(conversa, "queria saber o prazo de entrega");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert_eq!(resumo.texto, "Olá! Como posso ajudar?");
    let jev = resumo.jev.expect("o turno registra o Jev");
    assert_eq!(jev.choice(IdGate::G1), Some(G1_CONTINUE));
    assert_eq!(jev.call.questions, 1);
    assert_eq!(jev.call.decided_by_cache, 0);

    let turno = cenario.ultimo_turno(conversa);
    let gravado: serde_json::Value =
        serde_json::from_str(&turno.jev_json).expect("jev_json é JSON");
    assert_eq!(gravado["decisoes"][0]["gate"], "g1");
    assert_eq!(gravado["decisoes"][0]["escolha"], G1_CONTINUE);
    assert_eq!(gravado["chamada"]["perguntas"], 1);
}

/// O estado enviado ao Jev tem a conversa, o canal e a política do agente.
#[tokio::test]
async fn estado_do_turno_leva_conversa_e_politica() {
    let cenario = Cenario::novo("estado");
    let conversa = cenario.conversa();
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g1_transferir",
        G1_TRANSFER,
    )])));
    let motor = cenario.motor(so_g1(), Some(motor_jev(Arc::clone(&jev), so_g1())));

    let gatilho = cenario.falar(conversa, "quero falar com um atendente agora");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;
    assert_eq!(
        resumo.jev.as_ref().and_then(|jev| jev.choice(IdGate::G1)),
        Some(G1_TRANSFER)
    );

    let pedido = &jev.pedidos()[0];
    assert_eq!(pedido.estado["canal"], "cli");
    assert_eq!(
        pedido.estado["conversa"]["mensagens"][0]["texto"],
        "quero falar com um atendente agora"
    );
    assert_eq!(pedido.estado["politica"]["transferencia_habilitada"], true);
    assert!(pedido.perguntas.contains_key("g1_transferir"));
}

/// Sem gate ligado, o turno não chama o Jev e o registro fica com o padrão.
#[tokio::test]
async fn gates_desligados_nao_geram_pergunta() {
    let cenario = Cenario::novo("desligado");
    let conversa = cenario.conversa();
    let jev = Arc::new(JevFalso::com_escolhas(BTreeMap::new()));
    let motor = cenario.motor(
        JevGates::default(),
        Some(motor_jev(Arc::clone(&jev), JevGates::default())),
    );

    let gatilho = cenario.falar(conversa, "bom dia");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert!(resumo.jev.is_none());
    assert_eq!(jev.chamadas(), 0);
    assert_eq!(cenario.ultimo_turno(conversa).jev_json, "{}");
}

/// A flag do agente vence a global: `{"gates": {"g1": false}}` cala o gate.
#[tokio::test]
async fn ajuste_do_agente_desliga_o_gate_no_turno() {
    let cenario = Cenario::novo("ajuste");
    let conversa = cenario.conversa();
    cenario.ajustar_jev_do_agente(r#"{"gates": {"g1": false}}"#);
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g1_transferir",
        G1_TRANSFER,
    )])));
    let motor = cenario.motor(so_g1(), Some(motor_jev(Arc::clone(&jev), so_g1())));

    let gatilho = cenario.falar(conversa, "quero falar com um atendente");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert_eq!(jev.chamadas(), 0, "o agente desligou o G1");
    assert!(resumo.jev.is_none());
}

/// Aceite: falha do Jev não derruba o turno — a resposta sai e o fallback fica registrado.
#[tokio::test]
async fn falha_do_jev_nao_derruba_o_turno() {
    let cenario = Cenario::novo("falha");
    let conversa = cenario.conversa();
    let jev = Arc::new(JevFalso::falhando(JevError::Status {
        code: 503,
        motivo: "Service Unavailable".to_owned(),
    }));
    let motor = cenario.motor(so_g1(), Some(motor_jev(Arc::clone(&jev), so_g1())));

    let gatilho = cenario.falar(conversa, "quero falar com um atendente");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert_eq!(resumo.texto, "Olá! Como posso ajudar?");
    let jev = resumo.jev.expect("o turno registra a tentativa");
    assert!(jev.failed());
    assert_eq!(jev.choice(IdGate::G1), Some(G1_CONTINUE), "fallback do G1");

    let turno = cenario.ultimo_turno(conversa);
    let gravado: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    assert!(
        gravado["chamada"]["falha"]
            .as_str()
            .unwrap_or_default()
            .contains("503")
    );
    assert_eq!(gravado["decisoes"][0]["origem"], "fallback");
}

/// G4 ligado: o turno leva os departamentos ativos da org como candidatos.
#[tokio::test]
async fn g4_leva_os_departamentos_ativos_da_org() {
    let cenario = Cenario::novo("g4");
    cenario.departamento("Comercial");
    cenario.departamento("Suporte");
    let conversa = cenario.conversa();
    let globais = JevGates {
        g4: true,
        ..JevGates::default()
    };
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g4_departamento",
        "Suporte",
    )])));
    let motor = cenario.motor(globais, Some(motor_jev(Arc::clone(&jev), globais)));

    let gatilho = cenario.falar(conversa, "meu pedido chegou quebrado");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(
        resumo.jev.as_ref().and_then(|jev| jev.choice(IdGate::G4)),
        Some("Suporte")
    );
    let pedido = &jev.pedidos()[0];
    assert_eq!(pedido.perguntas.len(), 1, "só o G4 está ligado");
    let criterios = &pedido.perguntas["g4_departamento"].rubric;
    assert!(criterios.contains_key("Comercial"));
    assert!(criterios.contains_key("Suporte"));
    assert_eq!(
        pedido.estado["politica"]["departamentos"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
}

/// A janela de mensagens do estado é a mesma janela recente do turno, truncada.
#[tokio::test]
async fn estado_tem_no_maximo_dez_mensagens() {
    let estado = JevState::new("cli")
        .with_messages((0..15).map(|i| JevMessage::from_customer(format!("m{i}"))));
    assert_eq!(estado.messages.len(), 10);
    assert_eq!(estado.messages[0].texto, "m5");
    assert_eq!(estado.last_message().map(|m| m.texto.as_str()), Some("m14"));

    let longa = JevState::new("cli").with_messages([JevMessage::from_customer("a".repeat(2000))]);
    assert!(longa.messages[0].texto.chars().count() <= 801);
}

/// O motor sem Jev continua exatamente como antes (nenhum gate é consultado).
#[tokio::test]
async fn motor_sem_jev_responde_igual() {
    let cenario = Cenario::novo("sem-jev");
    let conversa = cenario.conversa();
    let motor = cenario.motor(JevGates::default(), None);

    let gatilho = cenario.falar(conversa, "bom dia");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert!(resumo.jev.is_none());
    assert_eq!(cenario.ultimo_turno(conversa).jev_json, "{}");
}

/// A "letra miúda": `MotorJev` com G4 e estado sem departamentos não pergunta nada.
#[tokio::test]
async fn g4_sem_departamentos_nao_pergunta() {
    let estado = JevState::new("cli").with_messages([JevMessage::from_customer("oi")]);
    let jev = Arc::new(JevFalso::com_escolhas(BTreeMap::new()));
    let motor = motor_jev(
        Arc::clone(&jev),
        JevGates {
            g4: true,
            ..JevGates::default()
        },
    );

    let resultado = motor
        .evaluate(estado, Some(poph_jev::eval::ajuste_de_gate(IdGate::G4)))
        .await;

    assert_eq!(jev.chamadas(), 0);
    assert!(resultado.decisions.is_empty());
    let _ = G4;
}

/// Busca falsa que só demora: serve para medir a concorrência com o Jev.
struct BuscaLenta(Duration);

impl BuscadorBase for BuscaLenta {
    fn buscar(
        &self,
        _org: OrgId,
        _agente: AgentId,
        _consulta: String,
        _parametros: ParametrosBusca,
    ) -> FuturoBusca<'_> {
        let espera = self.0;
        Box::pin(async move {
            tokio::time::sleep(espera).await;
            Ok(Vec::new())
        })
    }
}

/// O lote do Jev roda **junto** com a busca: o turno custa o maior dos dois, não a soma.
#[tokio::test]
async fn jev_e_busca_correm_juntos() {
    let cenario = Cenario::novo("paralelo");
    let conversa = cenario.conversa();
    let jev = Arc::new(
        JevFalso::com_escolhas(escolhas(&[("g1_transferir", G1_CONTINUE)]))
            .com_atraso(Duration::from_millis(300)),
    );
    let motor = cenario
        .motor(so_g1(), Some(motor_jev(Arc::clone(&jev), so_g1())))
        .com_busca(Arc::new(BuscaLenta(Duration::from_millis(300))));

    let gatilho = cenario.falar(conversa, "queria saber o prazo de entrega");
    let inicio = Instant::now();
    let resumo = cenario.turno(&motor, conversa, gatilho).await;
    let decorrido = inicio.elapsed();

    assert_eq!(resumo.estado, TurnState::Completed);
    assert!(resumo.jev.is_some());
    // Sequencial seria ~600 ms; concorrente fica perto de 300 ms. Folga larga para máquina cheia.
    assert!(
        decorrido < Duration::from_millis(520),
        "o turno levou {decorrido:?}: busca e Jev não correram juntos"
    );
}
