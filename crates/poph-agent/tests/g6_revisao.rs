//! Efeito do G6 no turno (HR-34): revisão da resposta antes do envio.
//!
//! O que se prova aqui, com provedor de modelo falso e Jev falso (nenhum teste toca a rede):
//! 1. `aprovada` mantém a resposta do modelo e o turno faz **uma** chamada ao modelo;
//! 2. uma reprovação custa **exatamente uma** regeneração: duas chamadas ao modelo e o texto novo
//!    é o que sai;
//! 3. duas reprovações (ou a segunda revisão sem resposta) usam a **resposta segura** do agente,
//!    quando configurada; sem ela, sai o texto regenerado — o cliente nunca fica sem resposta;
//! 4. abstenção, confiança baixa, opção fora do contrato e Jev fora do ar = resposta do modelo,
//!    como antes da HR-34;
//! 5. gate desligado não pergunta nada ao Jev;
//! 6. a regeneração não duplica a saída: uma mensagem do agente e um item na `outbox`;
//! 7. falha do modelo na regeneração mantém a resposta original e conclui o turno.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasMemoria, MetricasTurno};
use poph_agent::turno::{
    Entrada, FuturoPausa, Motor, ObservadorNulo, OpcoesTurno, Pausa, ResumoTurno,
};
use poph_core::llm::{FakeProvider, FinishReason, LlmError, LlmEvent, Script, Usage};
use poph_jev::{ErroJev, JevFalso, LoteResposta, MotorJev, Resposta as RespostaJev};
use poph_store::{
    AgentChange, Direction, NewAgent, NewConversation, NewMessage, Page, Role, Store, TurnState,
};
use poph_types::{ConversationId, JevGates, MessageId, OrgId, StructuredInstructions};

struct SemEspera;

impl Pausa for SemEspera {
    fn esperar(&self, _duracao: Duration) -> FuturoPausa {
        Box::pin(async {})
    }
}

struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    conversa: ConversationId,
    metricas: MetricasTurno,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        Self::com_jev_json(rotulo, "")
    }

    fn com_jev_json(rotulo: &str, jev_json: &str) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-hr34-{rotulo}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|tempo| tempo.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&pasta);
        std::fs::create_dir_all(&pasta).expect("pasta do teste");
        let store = Arc::new(
            Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre o banco"),
        );
        let org = store
            .orgs()
            .create(poph_store::NewOrg {
                name: format!("Org {rotulo}"),
                slug: format!("org-{rotulo}-{}-{}", std::process::id(), fastrand()),
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
                    slug: format!("bia-{rotulo}-{}", fastrand()),
                    grouping_window_ms: 0,
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente");
        let estruturado = StructuredInstructions::default();
        store
            .instructions()
            .save(
                org,
                agente.id,
                0,
                &estruturado.to_json(),
                "# Bia\n\nVocê é a atendente da loja.",
                true,
            )
            .expect("grava as instruções");
        if !jev_json.is_empty() {
            store
                .agents()
                .update(
                    org,
                    agente.id,
                    agente.version,
                    AgentChange {
                        jev_json: Some(jev_json.to_owned()),
                        ..AgentChange::default()
                    },
                )
                .expect("grava o jev_json do agente");
        }
        let conversa = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente.id,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("abre a conversa")
            .id;
        Self {
            pasta,
            store,
            org,
            conversa,
            metricas: MetricasTurno::proprio(),
        }
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
            .expect("grava a mensagem")
            .id
    }

    fn motor(
        &self,
        roteiros: Vec<Script>,
        jev: Option<Arc<MotorJev>>,
    ) -> (Motor, Arc<FakeProvider>) {
        let fake = Arc::new(FakeProvider::with_scripts(roteiros));
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
            fake.clone(),
            OpcoesTurno::default(),
            self.metricas.clone(),
        )
        .com_pausa(Arc::new(SemEspera))
        // O resumo rolante chama o modelo em segundo plano e bagunçaria a contagem de chamadas.
        .com_resumo(poph_agent::resumo::OpcoesResumo {
            ativo: false,
            ..poph_agent::resumo::OpcoesResumo::default()
        });
        if let Some(jev) = jev {
            motor = motor.com_jev(jev);
        }
        (motor, fake)
    }

    async fn turno(&self, motor: &Motor, gatilho: MessageId) -> ResumoTurno {
        motor
            .executar(
                Entrada::nova(self.org, self.conversa, gatilho),
                &mut ObservadorNulo,
            )
            .await
            .expect("o turno roda")
    }

    fn saidas(&self) -> Vec<String> {
        self.store
            .messages()
            .list(self.org, self.conversa, Page::with_limit(200))
            .expect("lista as mensagens")
            .into_iter()
            .filter(|mensagem| mensagem.direction == Direction::Outbound)
            .map(|mensagem| mensagem.texto)
            .collect()
    }

    fn fila(&self) -> Vec<String> {
        self.store
            .outbox()
            .list_pending(self.org, 50)
            .expect("lista a fila de saída")
            .into_iter()
            .map(|item| item.content_json)
            .collect()
    }

    fn turno_gravado(&self, turno: &ResumoTurno) -> String {
        self.store
            .turns()
            .get(self.org, turno.turno)
            .expect("lê o turno")
            .jev_json
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

fn fastrand() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|tempo| tempo.as_nanos() as u64)
        .unwrap_or(0)
}

fn roteiro_texto(texto: &str) -> Script {
    Script::Eventos(vec![
        LlmEvent::Texto(texto.to_owned()),
        LlmEvent::Usage(Usage {
            entrada: 100,
            saida: 10,
        }),
        LlmEvent::Fim(FinishReason::Concluido),
    ])
}

fn so_g6() -> JevGates {
    JevGates {
        g6: true,
        ..JevGates::default()
    }
}

/// Uma resposta do Jev para uma pergunta: escolha e confiança.
type RespostaRoteirizada = (&'static str, f64);

/// Jev falso que responde pela **resposta em revisão** (`estado.resposta_candidata`): para cada
/// texto, a escolha do G6. Texto fora da tabela não recebe resposta (abstenção).
fn jev_por_texto(tabela: &[(&str, RespostaRoteirizada)]) -> Arc<JevFalso> {
    let tabela: Vec<(String, String, f64)> = tabela
        .iter()
        .map(|(texto, (escolha, confianca))| {
            ((*texto).to_owned(), (*escolha).to_owned(), *confianca)
        })
        .collect();
    Arc::new(JevFalso::novo(move |pedido| {
        let candidata = pedido.estado["resposta_candidata"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let mut respostas = BTreeMap::new();
        if let Some((_, escolha, confianca)) =
            tabela.iter().find(|(texto, _, _)| *texto == candidata)
        {
            respostas.insert(
                "g6_revisao".to_owned(),
                RespostaJev {
                    escolha: Some(escolha.clone()),
                    confianca: *confianca,
                    ..RespostaJev::default()
                },
            );
        }
        Ok(LoteResposta {
            respostas,
            tokens: 20,
            ..LoteResposta::default()
        })
    }))
}

/// Jev falso que falha em todas as chamadas.
fn jev_fora() -> Arc<JevFalso> {
    Arc::new(JevFalso::novo(|_pedido: &poph_jev::Pedido| {
        Err(ErroJev::Status {
            codigo: 503,
            motivo: "Service Unavailable".to_owned(),
        })
    }))
}

/// Jev falso que acerta na primeira revisão e falha na segunda (por contagem de chamadas).
fn jev_que_falha_na_segunda(primeira: RespostaRoteirizada) -> Arc<JevFalso> {
    let chamadas = Arc::new(AtomicU64::new(0));
    Arc::new(JevFalso::novo(move |_pedido: &poph_jev::Pedido| {
        let chamada = chamadas.fetch_add(1, Ordering::SeqCst);
        if chamada > 0 {
            return Err(ErroJev::Status {
                codigo: 503,
                motivo: "Service Unavailable".to_owned(),
            });
        }
        let mut respostas = BTreeMap::new();
        respostas.insert(
            "g6_revisao".to_owned(),
            RespostaJev {
                escolha: Some(primeira.0.to_owned()),
                confianca: primeira.1,
                ..RespostaJev::default()
            },
        );
        Ok(LoteResposta {
            respostas,
            tokens: 20,
            ..LoteResposta::default()
        })
    }))
}

fn motor_jev(jev: Arc<JevFalso>, gates: JevGates) -> Arc<MotorJev> {
    Arc::new(MotorJev::novo(jev, gates, "jev-latest"))
}

const PERGUNTA: &str = "quanto custa para dedetizar baratas no apê?";
const RASCUNHO: &str = "Sai R$ 150,00 e amanhã mandamos o atendente aí.";
const REESCRITA: &str = "A dedetização de baratas custa R$ 180,00 por visita.";
const SEGURA: &str = "Vou confirmar o valor com a equipe e já te respondo.";

#[tokio::test]
async fn g6_aprovada_mantem_a_resposta_e_faz_uma_chamada_ao_modelo() {
    let cenario = Cenario::novo("aprovada");
    let jev = jev_por_texto(&[(RASCUNHO, ("aprovada", 0.95))]);
    let (motor, provedor) =
        cenario.motor(vec![roteiro_texto(RASCUNHO)], Some(motor_jev(jev, so_g6())));

    let gatilho = cenario.falar(PERGUNTA);
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert_eq!(resumo.texto, RASCUNHO);
    assert_eq!(provedor.received().len(), 1, "uma chamada ao modelo");
    assert_eq!(cenario.saidas(), vec![RASCUNHO]);
    assert_eq!(cenario.fila().len(), 1, "um item na fila de saída");
    assert_eq!(cenario.metricas.revisoes_g6.value(), 1);
    assert_eq!(cenario.metricas.reprovadas_g6.value(), 0);
    assert_eq!(cenario.metricas.respostas_seguras_g6.value(), 0);
    let registro = cenario.turno_gravado(&resumo);
    assert!(registro.contains("\"revisao\""), "{registro}");
    assert!(registro.contains("\"aprovada\""), "{registro}");
}

#[tokio::test]
async fn g6_reprovada_regenera_uma_vez_e_envia_o_texto_novo() {
    let cenario = Cenario::novo("regenera");
    let jev = jev_por_texto(&[
        (RASCUNHO, ("revisar_preco", 0.91)),
        (REESCRITA, ("aprovada", 0.93)),
    ]);
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto(RASCUNHO), roteiro_texto(REESCRITA)],
        Some(motor_jev(jev, so_g6())),
    );

    let gatilho = cenario.falar(PERGUNTA);
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert_eq!(resumo.texto, REESCRITA, "sai o texto regenerado");
    let pedidos = provedor.received();
    assert_eq!(pedidos.len(), 2, "exatamente uma regeneração");
    // O pedido de regeneração leva o rascunho como resposta do modelo e a orientação como usuário.
    let ultimo = pedidos.last().expect("o pedido da regeneração");
    assert!(
        matches!(
            ultimo.mensagens.last(),
            Some(poph_core::llm::Message::Usuario(_))
        ),
        "o pedido da regeneração termina com a orientação do usuário"
    );
    assert!(
        ultimo.ferramentas.is_empty(),
        "regeneração não reabre ferramenta"
    );
    assert_eq!(cenario.saidas(), vec![REESCRITA], "uma única saída");
    assert_eq!(cenario.fila().len(), 1, "um único item na fila");
    assert_eq!(cenario.metricas.revisoes_g6.value(), 2);
    assert_eq!(cenario.metricas.reprovadas_g6.value(), 1);
    assert_eq!(cenario.metricas.respostas_seguras_g6.value(), 0);
    // O custo da regeneração entra nos tokens do turno (100 + 100 de entrada).
    assert_eq!(resumo.tokens_entrada, 200);
    let registro = cenario.turno_gravado(&resumo);
    assert!(registro.contains("\"regenerou\":true"), "{registro}");
    assert!(
        registro.contains("\"segunda_escolha\":\"aprovada\""),
        "{registro}"
    );
}

#[tokio::test]
async fn g6_duas_reprovacoes_usam_a_resposta_segura() {
    let cenario = Cenario::com_jev_json("segura", &format!(r#"{{"resposta_segura": "{SEGURA}"}}"#));
    let jev = jev_por_texto(&[
        (RASCUNHO, ("revisar_preco", 0.91)),
        (REESCRITA, ("revisar_transferencia", 0.88)),
    ]);
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto(RASCUNHO), roteiro_texto(REESCRITA)],
        Some(motor_jev(jev, so_g6())),
    );

    let gatilho = cenario.falar(PERGUNTA);
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert_eq!(resumo.texto, SEGURA, "a resposta segura é a que sai");
    assert_eq!(provedor.received().len(), 2, "sem terceira tentativa");
    assert_eq!(cenario.saidas(), vec![SEGURA]);
    assert_eq!(cenario.fila().len(), 1);
    assert_eq!(cenario.metricas.respostas_seguras_g6.value(), 1);
    let registro = cenario.turno_gravado(&resumo);
    assert!(registro.contains("\"resposta_segura\":true"), "{registro}");
}

#[tokio::test]
async fn g6_sem_resposta_segura_na_segunda_reprovacao_envia_o_texto_regenerado() {
    let cenario = Cenario::novo("sem-segura");
    let jev = jev_por_texto(&[
        (RASCUNHO, ("revisar_preco", 0.91)),
        (REESCRITA, ("revisar_longa", 0.85)),
    ]);
    let (motor, _provedor) = cenario.motor(
        vec![roteiro_texto(RASCUNHO), roteiro_texto(REESCRITA)],
        Some(motor_jev(jev, so_g6())),
    );

    let gatilho = cenario.falar(PERGUNTA);
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(
        resumo.texto, REESCRITA,
        "sem resposta segura, sai a regenerada"
    );
    assert_eq!(cenario.saidas(), vec![REESCRITA]);
    assert_eq!(cenario.metricas.respostas_seguras_g6.value(), 0);
}

#[tokio::test]
async fn g6_segunda_revisao_sem_resposta_usa_a_segura() {
    let cenario = Cenario::com_jev_json(
        "segunda-fora",
        &format!(r#"{{"resposta_segura": "{SEGURA}"}}"#),
    );
    let jev = jev_que_falha_na_segunda(("revisar_preco", 0.92));
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto(RASCUNHO), roteiro_texto(REESCRITA)],
        Some(motor_jev(jev, so_g6())),
    );

    let gatilho = cenario.falar(PERGUNTA);
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.texto, SEGURA);
    assert_eq!(provedor.received().len(), 2);
    assert_eq!(cenario.metricas.respostas_seguras_g6.value(), 1);
}

#[tokio::test]
async fn g6_jev_fora_nao_revisa_e_mantem_a_resposta() {
    let cenario = Cenario::novo("fora");
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto(RASCUNHO)],
        Some(motor_jev(jev_fora(), so_g6())),
    );

    let gatilho = cenario.falar(PERGUNTA);
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert_eq!(resumo.texto, RASCUNHO, "Jev fora = comportamento de hoje");
    assert_eq!(provedor.received().len(), 1, "sem regeneração");
    assert_eq!(cenario.metricas.reprovadas_g6.value(), 0);
}

#[tokio::test]
async fn g6_abstencao_e_confianca_baixa_mantem_a_resposta() {
    for (rotulo, tabela) in [
        ("sem-resposta", Vec::new()),
        ("baixa-confianca", vec![(RASCUNHO, ("revisar_preco", 0.45))]),
        (
            "fora-do-contrato",
            vec![(RASCUNHO, ("reescrever_tudo", 0.99))],
        ),
    ] {
        let cenario = Cenario::novo(rotulo);
        let jev = jev_por_texto(&tabela);
        let (motor, provedor) =
            cenario.motor(vec![roteiro_texto(RASCUNHO)], Some(motor_jev(jev, so_g6())));
        let gatilho = cenario.falar(PERGUNTA);
        let resumo = cenario.turno(&motor, gatilho).await;
        assert_eq!(resumo.texto, RASCUNHO, "{rotulo}");
        assert_eq!(provedor.received().len(), 1, "{rotulo}: sem regeneração");
        assert_eq!(cenario.metricas.reprovadas_g6.value(), 0, "{rotulo}");
    }
}

#[tokio::test]
async fn g6_desligado_nao_pergunta_e_mantem_a_resposta() {
    let cenario = Cenario::novo("desligado");
    let jev = jev_por_texto(&[(RASCUNHO, ("revisar_preco", 0.99))]);
    // Gates sem G6 (padrão de instalação: G3, G5 e G8 ligados; G6 desligado).
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto(RASCUNHO)],
        Some(motor_jev(Arc::clone(&jev), JevGates::default_value())),
    );

    let gatilho = cenario.falar(PERGUNTA);
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.texto, RASCUNHO);
    assert_eq!(provedor.received().len(), 1);
    assert!(jev.pedidos().is_empty(), "o G6 desligado não pergunta nada");
    assert_eq!(cenario.metricas.revisoes_g6.value(), 0);
    assert!(
        !JevGates::default_value().g6,
        "o G6 não entra no padrão de instalação"
    );
}

#[tokio::test]
async fn g6_falha_do_modelo_na_regeneracao_mantem_a_resposta_original() {
    let cenario = Cenario::novo("falha-modelo");
    let jev = jev_por_texto(&[(RASCUNHO, ("revisar_preco", 0.93))]);
    let (motor, provedor) = cenario.motor(
        vec![
            roteiro_texto(RASCUNHO),
            Script::Falha(LlmError::ConnectionInterrupted("conexão caiu".to_owned())),
        ],
        Some(motor_jev(jev, so_g6())),
    );

    let gatilho = cenario.falar(PERGUNTA);
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert_eq!(
        resumo.texto, RASCUNHO,
        "a falha da reescrita não derruba o turno"
    );
    assert_eq!(provedor.received().len(), 2);
    assert_eq!(cenario.saidas(), vec![RASCUNHO]);
    assert_eq!(cenario.metricas.respostas_seguras_g6.value(), 0);
}

// ───────────────────────────── medição (HR-34) ─────────────────────────────

/// Mede o custo do G6 no turno, nas duas condições, com a mesma sequência de mensagens.
///
/// O que sai no relatório: chamadas ao modelo de texto, chamadas de revisão ao Jev, quantas
/// regenerações custaram token de modelo e quantas respostas caíram na resposta segura — com o gate
/// desligado e ligado. Rodar com:
/// `cargo test -p poph-agent --test g6_revisao medicao -- --ignored --nocapture`
#[tokio::test]
#[ignore = "medição: imprime o custo do G6 nas duas condições"]
async fn medicao_do_g6() {
    // 60% aprovadas, 25% reprovadas uma vez (e aprovadas na segunda), 15% reprovadas duas vezes.
    let aprovadas = 15usize;
    let regeneradas = 6usize;
    let seguras = 3usize;
    let total = aprovadas + regeneradas + seguras;

    let mut mensagens: Vec<String> = Vec::new();
    let mut roteiros: Vec<Script> = Vec::new();
    let mut tabela: Vec<(String, String, f64)> = Vec::new();
    for indice in 0..total {
        let pergunta = format!("mensagem {indice}: quero um orçamento de dedetização");
        mensagens.push(pergunta);
        let rascunho =
            format!("rascunho {indice}: fica R$ 150,00 e já te passo para um atendente.");
        let reescrita =
            format!("rascunho {indice}: a dedetização de baratas custa R$ 180,00 por visita.");
        roteiros.push(roteiro_texto(&rascunho));
        if indice < aprovadas {
            tabela.push((rascunho, "aprovada".to_owned(), 0.93));
            continue;
        }
        tabela.push((rascunho, "revisar_preco".to_owned(), 0.93));
        roteiros.push(roteiro_texto(&reescrita));
        if indice < aprovadas + regeneradas {
            tabela.push((reescrita, "aprovada".to_owned(), 0.91));
        } else {
            tabela.push((reescrita, "revisar_transferencia".to_owned(), 0.90));
        }
    }

    let mut resultados = Vec::new();
    for (rotulo, ligado) in [("sem G6", false), ("com G6", true)] {
        let cenario = if ligado {
            Cenario::com_jev_json(
                "medicao-com",
                &format!(r#"{{"resposta_segura": "{SEGURA}"}}"#),
            )
        } else {
            Cenario::novo("medicao-sem")
        };
        let tabela_json: Vec<(String, String, f64)> = tabela.clone();
        let jev = Arc::new(JevFalso::novo(move |pedido| {
            let candidata = pedido.estado["resposta_candidata"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let mut respostas = BTreeMap::new();
            if let Some((_, escolha, confianca)) =
                tabela_json.iter().find(|(texto, _, _)| *texto == candidata)
            {
                respostas.insert(
                    "g6_revisao".to_owned(),
                    RespostaJev {
                        escolha: Some(escolha.clone()),
                        confianca: *confianca,
                        ..RespostaJev::default()
                    },
                );
            }
            Ok(LoteResposta {
                respostas,
                tokens: 20,
                ..LoteResposta::default()
            })
        }));
        let gates = if ligado { so_g6() } else { JevGates::default() };
        let motor_jev = Arc::new(MotorJev::novo(
            Arc::clone(&jev) as Arc<dyn poph_jev::TransporteJev>,
            gates,
            "jev-latest",
        ));
        let (motor, provedor) = cenario.motor(roteiros.clone(), Some(motor_jev));
        for mensagem in &mensagens {
            let gatilho = cenario.falar(mensagem);
            let resumo = cenario.turno(&motor, gatilho).await;
            assert_eq!(resumo.estado, TurnState::Completed);
        }
        resultados.push((
            rotulo,
            provedor.received().len(),
            jev.pedidos().len(),
            cenario.metricas.reprovadas_g6.value(),
            cenario.metricas.respostas_seguras_g6.value(),
        ));
    }

    println!("\nG6 · custo por turno ({total} turnos, mesmo roteiro nas duas condições)");
    println!(
        "condição | chamadas ao modelo | chamadas à revisão (Jev) | regenerações | respostas seguras"
    );
    for (rotulo, modelo, revisao, regeneracoes, seguras) in &resultados {
        println!("{rotulo} | {modelo} | {revisao} | {regeneracoes} | {seguras}");
    }
    let sem = &resultados[0];
    let com = &resultados[1];
    println!(
        "acréscimo com o gate ligado: {} chamadas ao modelo (+{:.0}%) · {:.2} revisões por turno",
        com.1 - sem.1,
        (com.1 as f64 / sem.1 as f64 - 1.0) * 100.0,
        com.2 as f64 / total as f64
    );
}
