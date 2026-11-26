//! G2 com efeito: a janela de agrupamento governada pelo Jev (HR-30).
//!
//! O que se prova aqui:
//! 1. `esperar_mais` estica a janela (dentro do teto do PRD §6.3) e a mensagem seguinte do cliente
//!    entra no **mesmo** turno — uma resposta, uma chamada ao modelo, as duas mensagens no prompt;
//! 2. `processar_agora` encurta a espera para a confirmação curta;
//! 3. gate desligado, Jev fora, erro, prazo, escolha fora das opções e confiança baixa mantêm a
//!    janela fixa do agente — o comportamento de antes da HR-30;
//! 4. conversa transferida continua sem responder e sem consultar o Jev (D12);
//! 5. o teste de latência do conjunto rotulado: nas mensagens completas o tempo até processar é
//!    menor que a janela fixa, e o percentual de frases partidas não piora.
//!
//! A espera é a `Pausa` injetável com **relógio virtual**: ela registra o que foi pedido e, ao
//! avançar o relógio, grava no banco as mensagens que o cliente "continuou digitando".

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasMemoria, MetricasTurno};
use poph_agent::turno::{
    Entrada, FuturoPausa, JANELA_CONFIRMACAO, Motor, ObservadorNulo, OpcoesTurno, Pausa,
    ResumoTurno, janela_com_g2,
};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, Message, Script, Usage};
use poph_jev::{
    EstadoJev, G1_CONTINUAR, G2_ESPERAR, G2_PROCESSAR, IdGate, JevFalso, MotorJev, OpcoesJev,
};
use poph_store::{
    AgentChange, Direction, NewAgent, NewConversation, NewMessage, NewOrg, Role, Store, TurnState,
};
use poph_types::{AgentId, ConversationId, JevGates, MessageId, OrgId};

// ───────────────────────────── Pausa com relógio virtual ─────────────────────────────

/// Mensagem que o cliente manda enquanto o turno espera a janela.
#[derive(Debug, Clone)]
struct Continuacao {
    /// Instante (ms, no relógio virtual do teste) em que a mensagem chega.
    instante_ms: u64,
    /// Texto digitado pelo cliente.
    texto: String,
}

/// Pausa de teste: registra a espera pedida e avança um relógio virtual, gravando as continuações
/// que chegam até aquele instante — é a forma de provar "a mensagem seguinte entrou na mesma
/// janela" sem dormir.
struct PausaVirtual {
    store: Arc<Store>,
    org: OrgId,
    conversa: ConversationId,
    agora_ms: Mutex<u64>,
    esperas: Mutex<Vec<Duration>>,
    continuacoes: Mutex<Vec<Continuacao>>,
}

impl PausaVirtual {
    fn nova(store: Arc<Store>, org: OrgId, conversa: ConversationId) -> Self {
        Self {
            store,
            org,
            conversa,
            agora_ms: Mutex::new(0),
            esperas: Mutex::new(Vec::new()),
            continuacoes: Mutex::new(Vec::new()),
        }
    }

    fn agendar(&self, em_ms: u64, texto: &str) {
        self.continuacoes.lock().expect("agenda").push(Continuacao {
            instante_ms: em_ms,
            texto: texto.to_owned(),
        });
    }

    /// Esperas pedidas, na ordem.
    fn esperas(&self) -> Vec<Duration> {
        self.esperas.lock().expect("esperas").clone()
    }

    /// Espera total pedida.
    fn espera_total(&self) -> Duration {
        self.esperas().iter().copied().sum()
    }

    fn avancar(&self, duracao: Duration) {
        let mut agora = self.agora_ms.lock().expect("relógio");
        *agora = agora.saturating_add(u64::try_from(duracao.as_millis()).unwrap_or(u64::MAX));
        let agora = *agora;
        let mut continuacoes = self.continuacoes.lock().expect("continuações");
        let mut pendentes = Vec::new();
        for continuacao in continuacoes.drain(..) {
            if continuacao.instante_ms <= agora {
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
                pendentes.push(continuacao);
            }
        }
        *continuacoes = pendentes;
    }
}

impl Pausa for PausaVirtual {
    fn esperar(&self, duracao: Duration) -> FuturoPausa {
        self.esperas.lock().expect("esperas").push(duracao);
        self.avancar(duracao);
        Box::pin(async {})
    }
}

// ───────────────────────────── Cenário ─────────────────────────────

struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-hr30-{rotulo}-{}-{}",
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
        // Handoff ligado: sem isso o G1 nem se aplica (regra do POP Atende) e o teste do gate
        // desligado não teria como provar que ele continua sendo consultado.
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

    fn motor(
        &self,
        jev: Option<Arc<MotorJev>>,
        pausa: Arc<PausaVirtual>,
        provedor: Arc<FakeProvider>,
    ) -> Motor {
        let config = ConfigQuente::nova(
            Arc::clone(&self.store),
            OpcoesConfigQuente::default(),
            MetricasMemoria::proprio(),
        );
        let limitador = Arc::new(Limitador::novo(
            OpcoesLimites::default(),
            MetricasMemoria::proprio(),
        ));
        let mut motor = Motor::novo(
            Arc::clone(&self.store),
            config,
            limitador,
            provedor,
            OpcoesTurno::default(),
            MetricasTurno::proprio(),
        )
        .com_pausa(pausa);
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
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

// ───────────────────────────── Apoio ─────────────────────────────

fn provedor() -> Arc<FakeProvider> {
    Arc::new(FakeProvider::with_scripts(vec![Script::Eventos(vec![
        LlmEvent::Texto("Claro, como posso ajudar?".to_owned()),
        LlmEvent::Usage(Usage {
            entrada: 10,
            saida: 5,
        }),
        LlmEvent::Fim(FinishReason::Concluido),
    ])]))
}

fn so_g2() -> JevGates {
    JevGates {
        g2: true,
        ..JevGates::default()
    }
}

fn motor_jev(jev: Arc<JevFalso>, gates: JevGates) -> Arc<MotorJev> {
    Arc::new(MotorJev::novo(jev, gates, "jev-latest"))
}

/// Motor do Jev com opções trocadas (prazo do lote, testes de timeout).
fn motor_jev_com(jev: Arc<JevFalso>, gates: JevGates, opcoes: OpcoesJev) -> Arc<MotorJev> {
    Arc::new(MotorJev::novo(jev, gates, "jev-latest").com_opcoes(opcoes))
}

fn escolha(pergunta: &str, escolha: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(pergunta.to_owned(), escolha.to_owned())])
}

/// Janela padrão do agente (2 s), tolerância para o tempo real decorrido entre a gravação da
/// mensagem e a leitura do relógio pelo motor.
const JANELA: Duration = Duration::from_secs(2);
const TOLERANCIA: Duration = Duration::from_millis(150);

fn perto(medido: Duration, esperado: Duration) -> bool {
    medido <= esperado && medido + TOLERANCIA >= esperado
}

// ───────────────────────────── 1. esperar mais ─────────────────────────────

/// `esperar_mais` estica a janela até o teto e a mensagem seguinte do cliente entra no mesmo turno:
/// uma resposta só, uma chamada ao modelo, as duas mensagens no prompt.
#[tokio::test]
async fn esperar_mais_estica_a_janela_e_agrupa_a_mensagem_seguinte() {
    let cenario = Cenario::novo("esperar");
    let conversa = cenario.conversa();
    let pausa = Arc::new(PausaVirtual::nova(
        Arc::clone(&cenario.store),
        cenario.org,
        conversa,
    ));
    // O cliente manda o endereço 3 s depois — depois da janela fixa de 2 s e dentro dos 4 s do G2.
    pausa.agendar(3000, "Rua das Flores, 120 — apto 3");

    let jev = Arc::new(JevFalso::com_escolhas(escolha("g2_esperar", G2_ESPERAR)));
    let provedor = provedor();
    let motor = cenario.motor(
        Some(motor_jev(Arc::clone(&jev), so_g2())),
        Arc::clone(&pausa),
        Arc::clone(&provedor),
    );

    let gatilho = cenario.falar(conversa, "vou te passar meu endereço");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert!(
        perto(pausa.espera_total(), Duration::from_secs(4)),
        "espera devia ser uma janela esticada (4 s), foi {:?}",
        pausa.espera_total()
    );
    assert_eq!(
        resumo.mensagens.len(),
        2,
        "a mensagem seguinte entra na mesma janela"
    );
    assert_eq!(provedor.received().len(), 1, "uma chamada ao modelo");
    let pedido = &provedor.received()[0];
    let do_cliente = pedido
        .mensagens
        .iter()
        .filter(|mensagem| matches!(mensagem, Message::Usuario(_)))
        .count();
    assert_eq!(
        do_cliente, 2,
        "o prompt leva as duas mensagens do cliente no mesmo turno"
    );
    assert_eq!(
        resumo.jev.as_ref().and_then(|jev| jev.escolha(IdGate::G2)),
        Some(G2_ESPERAR)
    );
}

/// Dois lotes: um na chegada (que decidiu "esperar mais") e outro depois da mensagem nova, porque o
/// estado mudou. O registro do turno guarda a decisão mais recente.
#[tokio::test]
async fn mensagem_nova_durante_a_espera_refaz_o_lote() {
    let cenario = Cenario::novo("refaz");
    let conversa = cenario.conversa();
    let pausa = Arc::new(PausaVirtual::nova(
        Arc::clone(&cenario.store),
        cenario.org,
        conversa,
    ));
    pausa.agendar(3000, "o produto que eu quero é o azul");

    let jev = Arc::new(JevFalso::com_escolhas(escolha("g2_esperar", G2_ESPERAR)));
    let motor = cenario.motor(
        Some(motor_jev(Arc::clone(&jev), so_g2())),
        Arc::clone(&pausa),
        provedor(),
    );

    let gatilho = cenario.falar(conversa, "queria saber do");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert_eq!(resumo.mensagens.len(), 2);
    assert_eq!(
        jev.chamadas(),
        2,
        "o lote é refeito quando o cliente fala durante a espera"
    );
    let ultimo = jev.pedidos().pop().expect("há pedido");
    assert_eq!(
        ultimo.estado["conversa"]["mensagens"][1]["texto"],
        "o produto que eu quero é o azul"
    );
}

// ───────────────────────────── 2. processar agora ─────────────────────────────

/// `processar_agora` encurta a espera para a confirmação curta: menos tempo até a resposta, sem o
/// lote refeito (ninguém falou durante a janela).
#[tokio::test]
async fn processar_agora_encurta_a_espera() {
    let cenario = Cenario::novo("processar");
    let conversa = cenario.conversa();
    let pausa = Arc::new(PausaVirtual::nova(
        Arc::clone(&cenario.store),
        cenario.org,
        conversa,
    ));
    let jev = Arc::new(JevFalso::com_escolhas(escolha("g2_esperar", G2_PROCESSAR)));
    let motor = cenario.motor(
        Some(motor_jev(Arc::clone(&jev), so_g2())),
        Arc::clone(&pausa),
        provedor(),
    );

    let gatilho = cenario.falar(conversa, "vocês atendem no domingo?");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert!(
        pausa.espera_total() < JANELA,
        "processar agora tem de esperar menos que a janela fixa, esperou {:?}",
        pausa.espera_total()
    );
    assert!(
        pausa.espera_total() <= JANELA_CONFIRMACAO
            && pausa.espera_total() + TOLERANCIA >= JANELA_CONFIRMACAO,
        "a espera devia ser a confirmação curta, foi {:?}",
        pausa.espera_total()
    );
    assert_eq!(jev.chamadas(), 1, "sem mensagem nova, um lote só");
    assert_eq!(resumo.mensagens.len(), 1);
}

// ───────────────────────────── 3. janela fixa (fallback) ─────────────────────────────

/// Gate desligado: uma espera de janela fixa, o lote continua acontecendo depois da janela — o
/// comportamento de antes da HR-30.
#[tokio::test]
async fn gate_desligado_mantem_a_janela_fixa() {
    let cenario = Cenario::novo("desligado");
    let conversa = cenario.conversa();
    let pausa = Arc::new(PausaVirtual::nova(
        Arc::clone(&cenario.store),
        cenario.org,
        conversa,
    ));
    let gates = JevGates {
        g1: true,
        ..JevGates::default()
    };
    let jev = Arc::new(JevFalso::com_escolhas(escolha(
        "g1_transferir",
        G1_CONTINUAR,
    )));
    let motor = cenario.motor(
        Some(motor_jev(Arc::clone(&jev), gates)),
        Arc::clone(&pausa),
        provedor(),
    );

    let gatilho = cenario.falar(conversa, "bom dia");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert_eq!(pausa.esperas().len(), 1);
    assert!(
        perto(pausa.espera_total(), JANELA),
        "{:?}",
        pausa.espera_total()
    );
    assert_eq!(jev.chamadas(), 1, "o G1 continua sendo consultado");
    assert!(resumo.jev.is_some());
}

/// Sem Jev nenhum: janela fixa, nenhuma chamada.
#[tokio::test]
async fn sem_jev_a_janela_e_a_de_hoje() {
    let cenario = Cenario::novo("sem-jev");
    let conversa = cenario.conversa();
    let pausa = Arc::new(PausaVirtual::nova(
        Arc::clone(&cenario.store),
        cenario.org,
        conversa,
    ));
    let motor = cenario.motor(None, Arc::clone(&pausa), provedor());

    let gatilho = cenario.falar(conversa, "bom dia");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert!(
        perto(pausa.espera_total(), JANELA),
        "{:?}",
        pausa.espera_total()
    );
    assert!(resumo.jev.is_none());
}

/// Erro do Jev: janela fixa e o turno responde normalmente.
#[tokio::test]
async fn falha_do_jev_mantem_a_janela_fixa() {
    let cenario = Cenario::novo("falha");
    let conversa = cenario.conversa();
    let pausa = Arc::new(PausaVirtual::nova(
        Arc::clone(&cenario.store),
        cenario.org,
        conversa,
    ));
    let jev = Arc::new(JevFalso::falhando(poph_jev::ErroJev::Status {
        codigo: 503,
        motivo: "Service Unavailable".to_owned(),
    }));
    let motor = cenario.motor(
        Some(motor_jev(Arc::clone(&jev), so_g2())),
        Arc::clone(&pausa),
        provedor(),
    );

    let gatilho = cenario.falar(conversa, "me deixa ver aqui");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert!(
        perto(pausa.espera_total(), JANELA),
        "{:?}",
        pausa.espera_total()
    );
    let registro = resumo.jev.expect("o turno registra a tentativa");
    assert!(registro.falhou());
    assert_eq!(
        registro.escolha(IdGate::G2),
        Some(G2_PROCESSAR),
        "fallback do G2"
    );
}

/// Resposta fora das opções (ambígua): janela fixa.
#[tokio::test]
async fn escolha_fora_das_opcoes_mantem_a_janela_fixa() {
    let cenario = Cenario::novo("ambigua");
    let conversa = cenario.conversa();
    let pausa = Arc::new(PausaVirtual::nova(
        Arc::clone(&cenario.store),
        cenario.org,
        conversa,
    ));
    let jev = Arc::new(JevFalso::com_escolhas(escolha("g2_esperar", "talvez")));
    let motor = cenario.motor(
        Some(motor_jev(Arc::clone(&jev), so_g2())),
        Arc::clone(&pausa),
        provedor(),
    );

    let gatilho = cenario.falar(conversa, "oi");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert!(
        perto(pausa.espera_total(), JANELA),
        "{:?}",
        pausa.espera_total()
    );
}

/// Confiança abaixo do limiar: janela fixa (o modelo queria esperar, mas a decisão não é confiável).
#[tokio::test]
async fn confianca_baixa_mantem_a_janela_fixa() {
    let cenario = Cenario::novo("confianca");
    let conversa = cenario.conversa();
    let pausa = Arc::new(PausaVirtual::nova(
        Arc::clone(&cenario.store),
        cenario.org,
        conversa,
    ));
    let jev = Arc::new(JevFalso::novo(|pedido: &poph_jev::Pedido| {
        let mut respostas = BTreeMap::new();
        for id in pedido.perguntas.keys() {
            let mut resposta = poph_jev::Resposta {
                escolha: Some(G2_ESPERAR.to_owned()),
                confianca: 0.4,
                ..poph_jev::Resposta::default()
            };
            resposta.probabilidades.insert(G2_ESPERAR.to_owned(), 0.4);
            respostas.insert(id.clone(), resposta);
        }
        Ok(poph_jev::LoteResposta {
            respostas,
            ..poph_jev::LoteResposta::default()
        })
    }));
    let motor = cenario.motor(
        Some(motor_jev(Arc::clone(&jev), so_g2())),
        Arc::clone(&pausa),
        provedor(),
    );

    let gatilho = cenario.falar(conversa, "me deixa ver");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert!(
        perto(pausa.espera_total(), JANELA),
        "confiança 0,4 é fallback: janela fixa, foi {:?}",
        pausa.espera_total()
    );
}

/// Prazo estourado no lote: janela fixa e o turno termina.
#[tokio::test]
async fn prazo_do_jev_mantem_a_janela_fixa() {
    let cenario = Cenario::novo("prazo");
    let conversa = cenario.conversa();
    let pausa = Arc::new(PausaVirtual::nova(
        Arc::clone(&cenario.store),
        cenario.org,
        conversa,
    ));
    let jev = Arc::new(
        JevFalso::com_escolhas(escolha("g2_esperar", G2_ESPERAR))
            .com_atraso(Duration::from_millis(250)),
    );
    let opcoes = OpcoesJev {
        tempo_maximo: Duration::from_millis(50),
        ..OpcoesJev::default()
    };
    let motor = cenario.motor(
        Some(motor_jev_com(Arc::clone(&jev), so_g2(), opcoes)),
        Arc::clone(&pausa),
        provedor(),
    );

    let gatilho = cenario.falar(conversa, "peraí");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert!(
        perto(pausa.espera_total(), JANELA),
        "{:?}",
        pausa.espera_total()
    );
    let registro = resumo.jev.expect("registra a tentativa");
    assert!(registro.falhou());
}

// ───────────────────────────── 4. conversa transferida ─────────────────────────────

/// Conversa transferida (D12) não abre turno, não consulta o Jev e não chama o modelo — mesmo com o
/// G2 ligado.
#[tokio::test]
async fn conversa_transferida_nao_responde_nem_consulta_o_jev() {
    let cenario = Cenario::novo("transferida");
    let conversa = cenario.conversa();
    cenario
        .store
        .conversations()
        .transfer(
            cenario.org,
            conversa,
            None,
            Some("teste".to_owned()),
            "{}".to_owned(),
        )
        .expect("transfere a conversa");

    let pausa = Arc::new(PausaVirtual::nova(
        Arc::clone(&cenario.store),
        cenario.org,
        conversa,
    ));
    let jev = Arc::new(JevFalso::com_escolhas(escolha("g2_esperar", G2_ESPERAR)));
    let provedor = provedor();
    let motor = cenario.motor(
        Some(motor_jev(Arc::clone(&jev), so_g2())),
        Arc::clone(&pausa),
        Arc::clone(&provedor),
    );

    let gatilho = cenario.falar(conversa, "oi, ainda estou aqui");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Skipped);
    assert!(resumo.texto.is_empty());
    assert_eq!(jev.chamadas(), 0, "nem consulta o Jev");
    assert!(provedor.received().is_empty(), "nem chama o modelo");
    assert!(pausa.esperas().is_empty(), "nem espera a janela");
    // A mensagem do cliente continua gravada (D12).
    let mensagens = cenario
        .store
        .messages()
        .last_set(cenario.org, conversa, 10)
        .expect("lê as mensagens");
    assert!(mensagens.iter().any(|mensagem| mensagem.id == gatilho));
}

// ───────────────────────────── 5. janela efetiva (unidade) ─────────────────────────────

/// A regra da janela, isolada: teto do PRD, esticar, encurtar e fallback.
#[test]
fn janela_com_g2_respeita_o_teto_e_o_fallback() {
    let base = Duration::from_secs(2);
    let teto = Duration::from_secs(8);
    let confirmacao = JANELA_CONFIRMACAO;

    assert_eq!(
        janela_com_g2(base, teto, confirmacao, Some(G2_ESPERAR)),
        Duration::from_secs(4)
    );
    assert_eq!(
        janela_com_g2(Duration::from_secs(6), teto, confirmacao, Some(G2_ESPERAR)),
        teto,
        "esticar nunca passa do teto do PRD §6.3"
    );
    assert_eq!(
        janela_com_g2(base, teto, confirmacao, Some(G2_PROCESSAR)),
        confirmacao
    );
    assert_eq!(janela_com_g2(base, teto, confirmacao, None), base);
    assert_eq!(
        janela_com_g2(base, teto, confirmacao, Some("opcao_desconhecida")),
        base
    );
}

// ───────────────────────────── 6. latência e frases partidas ─────────────────────────────

/// Caso do conjunto `tests/jev/g2.jsonl` com a continuação que o cliente digitaria depois.
#[derive(Debug, serde::Deserialize)]
struct CasoDoConjunto {
    estado: EstadoJev,
    esperado: String,
    #[serde(default)]
    continuacao: Option<ContinuacaoDoConjunto>,
}

/// Texto que o cliente continua digitando e quanto tempo depois.
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
        .filter(|linha| !linha.is_empty() && !linha.starts_with('#'))
        .map(|linha| serde_json::from_str(linha).expect("caso do conjunto em JSON"))
        .collect()
}

/// Texto da última mensagem do cliente no caso.
fn texto_do_caso(caso: &CasoDoConjunto) -> String {
    caso.estado
        .mensagens
        .iter()
        .rev()
        .find(|mensagem| mensagem.papel == poph_jev::PapelJev::Cliente)
        .map(|mensagem| mensagem.texto.clone())
        .unwrap_or_default()
}

/// Roda um turno para um caso do conjunto, com o G2 respondendo o que o caso espera.
async fn rodar_caso(
    cenario: &Cenario,
    conversa: ConversationId,
    caso: &CasoDoConjunto,
    decisao: Option<&str>,
) -> (Arc<PausaVirtual>, ResumoTurno) {
    let pausa = Arc::new(PausaVirtual::nova(
        Arc::clone(&cenario.store),
        cenario.org,
        conversa,
    ));
    if let Some(continuacao) = &caso.continuacao {
        pausa.agendar(continuacao.atraso_ms, &continuacao.texto);
    }
    let escolhas = match decisao {
        Some(decidido) => escolha("g2_esperar", decidido),
        None => BTreeMap::new(),
    };
    let jev = Arc::new(JevFalso::com_escolhas(escolhas));
    let motor = cenario.motor(
        Some(motor_jev(Arc::clone(&jev), so_g2())),
        Arc::clone(&pausa),
        provedor(),
    );
    let gatilho = cenario.falar(conversa, &texto_do_caso(caso));
    let resumo = cenario.turno(&motor, conversa, gatilho).await;
    (pausa, resumo)
}

/// Aceite da HR-30: nas mensagens completas do conjunto o tempo até processar é menor que a janela
/// fixa, e o percentual de frases partidas **não piora** com o G2 ligado.
#[tokio::test]
async fn latencia_menor_nas_completas_sem_partir_mais_frases() {
    let casos = ler_conjunto();
    assert!(
        casos.len() >= 60,
        "o conjunto do G2 precisa de pelo menos 60 casos (tem {})",
        casos.len()
    );
    let cenario = Cenario::novo("latencia");

    // (a) Mensagens completas: com o G2 dizendo "processar agora", a espera tem de ser menor que a
    // janela fixa. O mesmo caso sem o gate é a linha de base.
    let completas: Vec<&CasoDoConjunto> = casos
        .iter()
        .filter(|caso| caso.esperado == G2_PROCESSAR)
        .collect();
    assert!(
        completas.len() >= 30,
        "casos completos: {}",
        completas.len()
    );
    let mut soma_com = Duration::ZERO;
    let mut soma_sem = Duration::ZERO;
    for caso in &completas {
        let conversa = cenario.conversa();
        let (pausa, resumo) = rodar_caso(&cenario, conversa, caso, Some(G2_PROCESSAR)).await;
        assert_eq!(resumo.estado, TurnState::Completed);
        soma_com += pausa.espera_total();

        let conversa = cenario.conversa();
        let (pausa, resumo) = rodar_caso(&cenario, conversa, caso, None).await;
        assert_eq!(resumo.estado, TurnState::Completed);
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

    // (b) Frases partidas: nos casos com continuação, contamos quantas vezes a mensagem seguinte NÃO
    // entrou na mesma resposta. Com o G2 (usando o rótulo do próprio caso) não pode ser pior que sem
    // ele.
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
        let conversa = cenario.conversa();
        let (_, resumo) = rodar_caso(&cenario, conversa, caso, Some(&caso.esperado)).await;
        if resumo.mensagens.len() < 2 {
            partidas_com += 1;
        }

        let conversa = cenario.conversa();
        let (_, resumo) = rodar_caso(&cenario, conversa, caso, None).await;
        if resumo.mensagens.len() < 2 {
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
