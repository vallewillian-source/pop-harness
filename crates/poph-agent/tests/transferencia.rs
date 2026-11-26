//! G1 com efeito: transferir para humano e orientar o cliente (HR-29).
//!
//! O que se prova aqui, no turno de verdade (banco, agente, conversa e `outbox` reais):
//! - decidida a transferência, a conversa vira `transferred`, o cliente recebe **uma** mensagem com
//!   a orientação aprovada + a mensagem automática e o **modelo de texto não é chamado**;
//! - daí em diante nada responde (mensagem do cliente continua gravada) até `reopen`;
//! - todo fail-safe (risco com erro, prazo, `nenhum`, confiança baixa, texto vazio) transfere
//!   mandando só a mensagem automática, sem derrubar o turno;
//! - com o G1 desligado, ou agente sem handoff, o comportamento é o de antes da HR-29.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasMemoria, MetricasTurno};
use poph_agent::turno::{
    Entrada, EventoTurno, FuturoPausa, Motor, Observador, ObservadorNulo, OpcoesTurno, Pausa,
    ResumoTurno,
};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, Script, Usage};
use poph_jev::{
    BatchResponse, G1_CONTINUE, G1_TRANSFER, IdGate, JevFalso, JevResponse, MotorJev, Orientacao,
    Pedido, RISCO_ANIMAL, RISCO_INGESTAO, RISCO_NENHUM, RISCO_OUTRO, RISCO_PERGUNTA_ID,
};
use poph_store::{
    AgentChange, Direction, NewAgent, NewConversation, NewMessage, NewOrg, Page, Role, Store,
    TurnState,
};
use poph_types::{AgentId, ConversationId, JevGates, MessageId, OrgId};

/// Textos aprovados em 19/11/2026 (`HUMAN_HANDOFF_GUIDANCE_PRD.md` §5.3), literais do documento.
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

impl Pausa for SemEspera {
    fn esperar(&self, _duracao: Duration) -> FuturoPausa {
        Box::pin(async {})
    }
}

/// Observador que guarda os eventos (para conferir o aviso de transferência).
#[derive(Default)]
struct Gravador {
    eventos: Vec<EventoTurno>,
}

impl Observador for Gravador {
    fn evento(&mut self, evento: EventoTurno) {
        self.eventos.push(evento);
    }
}

/// Banco, org, agente e conversa de verdade numa pasta temporária.
struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    provedor: Arc<FakeProvider>,
}

impl Cenario {
    fn novo(rotulo: &str, handoff_ativo: bool) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-hr29-{rotulo}-{}-{}",
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
        // Roteiros de sobra: quem prova que o modelo não foi chamado é o contador do provedor.
        let provedor = Arc::new(FakeProvider::with_scripts(
            (0..8).map(|_| roteiro(RESPOSTA_DO_MODELO)).collect(),
        ));
        Self {
            pasta,
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
        let atual = self
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
                atual.version,
                AgentChange {
                    jev_json: Some(json),
                    ..AgentChange::default()
                },
            )
            .expect("ajusta o jev_json do agente");
    }

    fn motor(&self, jev: Option<Arc<MotorJev>>, orientacao: Orientacao) -> Motor {
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
            Arc::clone(&self.provedor) as Arc<dyn poph_core::llm::LlmProvider>,
            OpcoesTurno::default(),
            MetricasTurno::proprio(),
        )
        .com_pausa(Arc::new(SemEspera))
        .com_handoff(orientacao);
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
        self.turno_observado(motor, conversa, gatilho, &mut ObservadorNulo)
            .await
    }

    async fn turno_observado(
        &self,
        motor: &Motor,
        conversa: ConversationId,
        gatilho: MessageId,
        observador: &mut dyn Observador,
    ) -> ResumoTurno {
        motor
            .executar(Entrada::nova(self.org, conversa, gatilho), observador)
            .await
            .expect("o turno roda")
    }

    /// Textos das mensagens da conversa, na ordem (quem falou, texto).
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
            .filter(|(papel, _)| *papel == Role::Assistant)
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
        let atual = self
            .store
            .conversations()
            .get(self.org, conversa)
            .expect("lê a conversa");
        self.store
            .conversations()
            .reopen_audited(self.org, conversa, atual.version, "teste".to_owned())
            .expect("reabre a conversa");
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
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

fn resposta(escolha: &str, confianca: f64) -> JevResponse {
    let mut item = JevResponse {
        choice: Some(escolha.to_owned()),
        confidence: confianca,
        ..JevResponse::default()
    };
    item.probabilities.insert(escolha.to_owned(), confianca);
    item
}

/// Jev falso que decide o G1 e responde a pergunta de risco como pedido.
fn jev(decisao: &'static str, confianca: f64, risco: Option<(&'static str, f64)>) -> Arc<JevFalso> {
    Arc::new(JevFalso::novo(move |pedido: &Pedido| {
        let mut respostas = BTreeMap::new();
        if pedido.perguntas.contains_key("g1_transferir") {
            respostas.insert("g1_transferir".to_owned(), resposta(decisao, confianca));
        }
        if pedido.perguntas.contains_key(RISCO_PERGUNTA_ID)
            && let Some((tipo, confianca)) = risco
        {
            respostas.insert(RISCO_PERGUNTA_ID.to_owned(), resposta(tipo, confianca));
        }
        Ok(BatchResponse {
            answers: respostas,
            tokens: 9,
            ..BatchResponse::default()
        })
    }))
}

fn motor_jev(jev: Arc<JevFalso>) -> Arc<MotorJev> {
    Arc::new(MotorJev::novo(jev, so_g1(), "jev-latest"))
}

// ───────────────────────────── aceite: transfere de verdade ─────────────────────────────

/// Aceite 1 e 2: com risco de ingestão, a conversa fica `transferred`, o cliente recebe a
/// orientação aprovada + mensagem automática, a auditoria registra o ator e o **modelo não é
/// chamado**.
#[tokio::test]
async fn transfere_com_orientacao_e_nao_chama_o_modelo() {
    let cenario = Cenario::novo("orientacao", true);
    cenario.situacoes_de_handoff(&["risco com produto"]);
    let conversa = cenario.conversa();
    let jev = jev(G1_TRANSFER, 0.95, Some((RISCO_INGESTAO, 0.9)));
    let motor = cenario.motor(Some(motor_jev(Arc::clone(&jev))), Orientacao::default());

    let gatilho = cenario.falar(conversa, "meu filho engoliu o veneno da isca, e agora?");
    let mut observador = Gravador::default();
    let resumo = cenario
        .turno_observado(&motor, conversa, gatilho, &mut observador)
        .await;

    // O modelo de texto não foi chamado nenhuma vez.
    assert!(
        cenario.provedor.received().is_empty(),
        "conversa transferida não pode chamar o LLM"
    );
    // O turno terminou bem, com o texto que o cliente recebeu.
    assert_eq!(resumo.estado, TurnState::Completed);
    let esperado = format!("{APROVADO_INGESTAO}\n\n{APROVADA_AUTOMATICA}");
    assert_eq!(resumo.texto, esperado);
    assert_eq!(
        resumo.motivo_fim.as_deref(),
        Some("transferida para humano")
    );

    // Uma única mensagem do agente, exatamente com os textos aprovados.
    assert_eq!(
        cenario.respostas_do_agente(conversa),
        vec![esperado.clone()]
    );
    // A conversa ficou transferida.
    assert_eq!(
        cenario.status(conversa),
        poph_store::ConversationStatus::Transferred
    );
    // O mesmo texto foi para a `outbox` (é o que o canal entrega).
    let pendentes = cenario
        .store
        .outbox()
        .list_pending(cenario.org, 10)
        .expect("lista a outbox");
    assert_eq!(pendentes.len(), 1);
    assert_eq!(pendentes[0].tipo, "text");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&pendentes[0].content_json)
            .expect("conteúdo é JSON")["texto"],
        esperado
    );

    // Registro do turno: decisão do G1 e classificação de risco.
    let turno = cenario.ultimo_turno(conversa);
    let gravado: serde_json::Value =
        serde_json::from_str(&turno.jev_json).expect("jev_json é JSON");
    assert_eq!(gravado["decisoes"][0]["gate"], "g1");
    assert_eq!(gravado["decisoes"][0]["escolha"], G1_TRANSFER);
    assert_eq!(gravado["risco"]["tipo"], RISCO_INGESTAO);
    assert_eq!(turno.state, TurnState::Completed);

    // Auditoria: ator do sistema, ação `transfer`, na conversa certa.
    let registros = cenario.auditoria();
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
    assert_eq!(detalhe["risco"], RISCO_INGESTAO);

    // O aviso de transferência chegou a quem observa o turno.
    assert!(
        observador.eventos.iter().any(|evento| matches!(
            evento,
            EventoTurno::Transferencia {
                orientacao: true,
                tipo_risco: Some(tipo),
                ..
            } if tipo == RISCO_INGESTAO
        )),
        "faltou o evento de transferência: {:?}",
        observador.eventos
    );
    // A segunda pergunta saiu numa chamada própria, com o estado do turno.
    let pedidos = jev.pedidos();
    assert_eq!(pedidos.len(), 2);
    assert_eq!(
        pedidos[1].perguntas.keys().collect::<Vec<_>>(),
        vec![RISCO_PERGUNTA_ID]
    );
    assert_eq!(pedidos[1].estado["canal"], "cli");
}

/// Os outros dois textos de risco são os aprovados, byte a byte.
#[tokio::test]
async fn outros_dois_tipos_de_risco_usam_os_textos_aprovados() {
    for (tipo, aprovado) in [
        (RISCO_ANIMAL, APROVADO_ANIMAL),
        (RISCO_OUTRO, APROVADO_OUTRO),
    ] {
        let cenario = Cenario::novo(&format!("tipo-{tipo}"), true);
        let conversa = cenario.conversa();
        let jev = jev(G1_TRANSFER, 0.95, Some((tipo, 0.8)));
        let motor = cenario.motor(Some(motor_jev(jev)), Orientacao::default());
        let gatilho = cenario.falar(conversa, "tem um escorpião no berço do meu filho");
        let resumo = cenario.turno(&motor, conversa, gatilho).await;
        assert_eq!(
            resumo.texto,
            format!("{aprovado}\n\n{APROVADA_AUTOMATICA}"),
            "texto errado para {tipo}"
        );
    }
}

/// Aceite 1: conversa transferida não gera turno de LLM e `reopen` a reativa; as mensagens do
/// cliente continuam gravadas enquanto está transferida.
#[tokio::test]
async fn transferida_nao_responde_ate_reabrir() {
    let cenario = Cenario::novo("reabrir", true);
    let conversa = cenario.conversa();
    // O Jev só transfere quando a última mensagem fala da cobra: depois de reabrir, a conversa
    // segue normalmente, como aconteceria com o Jev de verdade.
    let jev = Arc::new(JevFalso::novo(|pedido: &Pedido| {
        let ultima = pedido.estado["conversa"]["mensagens"]
            .as_array()
            .and_then(|mensagens| mensagens.last())
            .and_then(|mensagem| mensagem["texto"].as_str())
            .unwrap_or("")
            .to_owned();
        let mut respostas = BTreeMap::new();
        if pedido.perguntas.contains_key("g1_transferir") {
            let escolha = if ultima.contains("cobra") {
                G1_TRANSFER
            } else {
                G1_CONTINUE
            };
            respostas.insert("g1_transferir".to_owned(), resposta(escolha, 0.95));
        }
        if pedido.perguntas.contains_key(RISCO_PERGUNTA_ID) {
            respostas.insert(RISCO_PERGUNTA_ID.to_owned(), resposta(RISCO_ANIMAL, 0.9));
        }
        Ok(BatchResponse {
            answers: respostas,
            tokens: 9,
            ..BatchResponse::default()
        })
    }));
    let motor = cenario.motor(Some(motor_jev(jev)), Orientacao::default());

    let gatilho = cenario.falar(conversa, "quero falar com uma pessoa, tem uma cobra aqui");
    let primeira = cenario.turno(&motor, conversa, gatilho).await;
    assert_eq!(primeira.estado, TurnState::Completed);
    assert_eq!(cenario.provedor.received().len(), 0);

    // Cliente insiste: a mensagem é gravada, o turno é ignorado e o modelo continua sem ser chamado.
    let segundo_gatilho = cenario.falar(conversa, "alguém aí?");
    let segundo = cenario.turno(&motor, conversa, segundo_gatilho).await;
    assert_eq!(segundo.estado, TurnState::Skipped);
    assert!(
        segundo.erro.unwrap_or_default().contains("transferred"),
        "o motivo do turno ignorado diz que a conversa está transferida"
    );
    assert_eq!(cenario.provedor.received().len(), 0);
    let textos: Vec<String> = cenario
        .conversa_gravada(conversa)
        .into_iter()
        .map(|(_, texto)| texto)
        .collect();
    assert!(textos.contains(&"alguém aí?".to_owned()));

    // Reaberta, volta a responder normalmente.
    cenario.reabrir(conversa);
    assert_eq!(
        cenario.status(conversa),
        poph_store::ConversationStatus::Open
    );
    let terceiro_gatilho = cenario.falar(conversa, "e o prazo de entrega?");
    let terceiro = cenario.turno(&motor, conversa, terceiro_gatilho).await;
    assert_eq!(terceiro.estado, TurnState::Completed);
    assert_eq!(terceiro.texto, RESPOSTA_DO_MODELO);
    assert_eq!(cenario.provedor.received().len(), 1);
}

// ───────────────────────────── fail-safe ─────────────────────────────

/// Aceite 3: risco com erro transfere mandando só a mensagem automática, sem derrubar o turno.
#[tokio::test]
async fn erro_na_pergunta_de_risco_transfere_com_a_mensagem_automatica() {
    let cenario = Cenario::novo("risco-erro", true);
    let conversa = cenario.conversa();
    // O G1 responde; a pergunta de risco recebe erro do provedor.
    let jev = Arc::new(JevFalso::novo(|pedido: &Pedido| {
        if pedido.perguntas.contains_key(RISCO_PERGUNTA_ID) {
            return Err(poph_jev::JevError::Status {
                code: 503,
                motivo: "Service Unavailable".to_owned(),
            });
        }
        let mut respostas = BTreeMap::new();
        respostas.insert("g1_transferir".to_owned(), resposta(G1_TRANSFER, 0.99));
        Ok(BatchResponse {
            answers: respostas,
            tokens: 9,
            ..BatchResponse::default()
        })
    }));
    let motor = cenario.motor(Some(motor_jev(jev)), Orientacao::default());

    let gatilho = cenario.falar(conversa, "quero um atendente");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert_eq!(resumo.texto, APROVADA_AUTOMATICA);
    assert_eq!(
        cenario.status(conversa),
        poph_store::ConversationStatus::Transferred
    );
    assert_eq!(
        cenario.respostas_do_agente(conversa),
        vec![APROVADA_AUTOMATICA.to_owned()]
    );
    assert!(cenario.provedor.received().is_empty());
    // A falha fica registrada no turno, para diagnóstico.
    let turno = cenario.ultimo_turno(conversa);
    let gravado: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    assert!(gravado["risco"]["falha"].as_str().is_some());
}

/// Aceite 3: prazo estourado na pergunta de risco também só suprime a orientação.
#[tokio::test]
async fn prazo_na_pergunta_de_risco_suprime_so_a_orientacao() {
    let cenario = Cenario::novo("risco-prazo", true);
    let conversa = cenario.conversa();
    // O Jev falso responde com 200 ms de atraso nas duas chamadas; o prazo da pergunta de risco
    // (20 ms) é o que se mede aqui — o lote do G1 tem prazo próprio, bem maior.
    let jev = Arc::new(
        JevFalso::com_escolhas(BTreeMap::from([
            ("g1_transferir".to_owned(), G1_TRANSFER.to_owned()),
            (RISCO_PERGUNTA_ID.to_owned(), RISCO_INGESTAO.to_owned()),
        ]))
        .com_atraso(Duration::from_millis(200)),
    );
    let orientacao = Orientacao::default().com_prazo(Duration::from_millis(20));
    let motor = cenario.motor(Some(motor_jev(jev)), orientacao);

    let gatilho = cenario.falar(conversa, "meu filho tomou o veneno");
    let inicio = Instant::now();
    let resumo = cenario.turno(&motor, conversa, gatilho).await;
    assert!(inicio.elapsed() < Duration::from_secs(5));

    assert_eq!(resumo.estado, TurnState::Completed);
    assert_eq!(resumo.texto, APROVADA_AUTOMATICA);
    assert_eq!(
        cenario.status(conversa),
        poph_store::ConversationStatus::Transferred
    );
}

/// Aceite 3: `nenhum` e confiança abaixo do limiar não orientam.
#[tokio::test]
async fn sem_risco_confiavel_manda_so_a_mensagem_automatica() {
    for (rotulo, risco) in [
        ("nenhum", (RISCO_NENHUM, 0.99)),
        ("confianca-baixa", (RISCO_INGESTAO, 0.2)),
    ] {
        let cenario = Cenario::novo(&format!("risco-{rotulo}"), true);
        let conversa = cenario.conversa();
        let jev = jev(G1_TRANSFER, 0.95, Some(risco));
        let motor = cenario.motor(Some(motor_jev(jev)), Orientacao::default());
        let gatilho = cenario.falar(conversa, "quero falar com o atendente, por favor");
        let resumo = cenario.turno(&motor, conversa, gatilho).await;
        assert_eq!(resumo.texto, APROVADA_AUTOMATICA, "caso {rotulo}");
        assert_eq!(
            cenario.status(conversa),
            poph_store::ConversationStatus::Transferred
        );
    }
}

/// Orientação desligada na configuração: nenhuma segunda pergunta é feita.
#[tokio::test]
async fn orientacao_desligada_nao_faz_a_segunda_pergunta() {
    let cenario = Cenario::novo("orientacao-desligada", true);
    let conversa = cenario.conversa();
    let jev = jev(G1_TRANSFER, 0.95, Some((RISCO_INGESTAO, 0.9)));
    let orientacao = Orientacao {
        ativo: false,
        ..Orientacao::default()
    };
    let motor = cenario.motor(Some(motor_jev(Arc::clone(&jev))), orientacao);

    let gatilho = cenario.falar(conversa, "quero falar com alguém");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.texto, APROVADA_AUTOMATICA);
    assert_eq!(jev.pedidos().len(), 1, "só o lote do turno foi chamado");
    assert_eq!(jev.chamadas(), 1);
}

/// Texto de orientação vazio na configuração: o tipo não orienta (§5.3).
#[tokio::test]
async fn texto_vazio_na_configuracao_nao_orienta() {
    let cenario = Cenario::novo("texto-vazio", true);
    let conversa = cenario.conversa();
    let jev = jev(G1_TRANSFER, 0.95, Some((RISCO_INGESTAO, 0.9)));
    let orientacao = Orientacao::default().com_texto(RISCO_INGESTAO, "");
    let motor = cenario.motor(Some(motor_jev(jev)), orientacao);

    let gatilho = cenario.falar(conversa, "meu filho engoliu o veneno");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;
    assert_eq!(resumo.texto, APROVADA_AUTOMATICA);
}

// ───────────────────────────── sem efeito quando não deve ─────────────────────────────

/// Aceite 3: G1 desligado não pergunta nada e o comportamento é o de antes da HR-29.
#[tokio::test]
async fn g1_desligado_nao_transfere() {
    let cenario = Cenario::novo("g1-desligado", true);
    let conversa = cenario.conversa();
    let jev = jev(G1_TRANSFER, 0.99, Some((RISCO_INGESTAO, 0.99)));
    let avaliador = Arc::new(MotorJev::novo(jev, JevGates::default(), "jev-latest"));
    let motor = cenario.motor(Some(avaliador), Orientacao::default());

    let gatilho = cenario.falar(conversa, "quero falar com um atendente humano");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert_eq!(resumo.texto, RESPOSTA_DO_MODELO);
    assert_eq!(
        cenario.status(conversa),
        poph_store::ConversationStatus::Open
    );
    assert_eq!(cenario.provedor.received().len(), 1);
    assert!(resumo.jev.is_none() || resumo.jev.as_ref().unwrap().decisoes.is_empty());
}

/// Agente com handoff desligado: o G1 nem se aplica (regra do POP Atende).
#[tokio::test]
async fn agente_sem_handoff_nao_transfere() {
    let cenario = Cenario::novo("sem-handoff", false);
    let conversa = cenario.conversa();
    let jev = jev(G1_TRANSFER, 0.99, Some((RISCO_INGESTAO, 0.99)));
    let motor = cenario.motor(Some(motor_jev(jev)), Orientacao::default());

    let gatilho = cenario.falar(conversa, "quero falar com um atendente humano");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.texto, RESPOSTA_DO_MODELO);
    assert_eq!(
        cenario.status(conversa),
        poph_store::ConversationStatus::Open
    );
}

/// Jev decidindo continuar: nada muda, o modelo responde normalmente.
#[tokio::test]
async fn g1_continuar_responde_normalmente() {
    let cenario = Cenario::novo("continuar", true);
    let conversa = cenario.conversa();
    let jev = jev(G1_CONTINUE, 0.99, None);
    let motor = cenario.motor(Some(motor_jev(jev)), Orientacao::default());

    let gatilho = cenario.falar(conversa, "qual é o prazo de entrega?");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, TurnState::Completed);
    assert_eq!(resumo.texto, RESPOSTA_DO_MODELO);
    assert_eq!(
        cenario.status(conversa),
        poph_store::ConversationStatus::Open
    );
    assert_eq!(
        resumo
            .jev
            .as_ref()
            .and_then(|item| item.escolha(IdGate::G1)),
        Some(G1_CONTINUE)
    );
    assert_eq!(cenario.provedor.received().len(), 1);
    assert!(
        !cenario
            .auditoria()
            .iter()
            .any(|registro| registro.acao == "transfer")
    );
}

/// A conversa transferida guarda o departamento humano configurado no agente.
#[tokio::test]
async fn transferencia_guarda_o_departamento_do_agente() {
    let cenario = Cenario::novo("departamento", true);
    let padrao = cenario
        .store
        .departments()
        .create(
            cenario.org,
            poph_store::NewDepartment {
                name: "Atendimento humano".to_owned(),
                description: String::new(),
            },
        )
        .expect("cria o departamento")
        .id;
    let atual = cenario
        .store
        .agents()
        .get(cenario.org, cenario.agente)
        .expect("lê o agente");
    cenario
        .store
        .agents()
        .update(
            cenario.org,
            cenario.agente,
            atual.version,
            AgentChange {
                handoff_department: Some(padrao),
                ..AgentChange::default()
            },
        )
        .expect("define o departamento humano");

    let conversa = cenario.conversa();
    let jev = jev(G1_TRANSFER, 0.95, Some((RISCO_ANIMAL, 0.9)));
    let motor = cenario.motor(Some(motor_jev(jev)), Orientacao::default());
    let gatilho = cenario.falar(conversa, "tem uma aranha no quarto");
    cenario.turno(&motor, conversa, gatilho).await;

    let gravada = cenario
        .store
        .conversations()
        .get(cenario.org, conversa)
        .expect("lê a conversa");
    assert_eq!(gravada.status, poph_store::ConversationStatus::Transferred);
    assert_eq!(gravada.target_department, Some(padrao));
}

/// A classificação de risco não é guardada no cache de decisões: a segunda pergunta é uma vez só.
#[tokio::test]
async fn segunda_pergunta_nao_usa_o_cache_do_lote() {
    let cenario = Cenario::novo("cache", true);
    let conversa = cenario.conversa();
    let jev = jev(G1_TRANSFER, 0.95, Some((RISCO_INGESTAO, 0.9)));
    let avaliador = motor_jev(Arc::clone(&jev));
    let motor = cenario.motor(Some(avaliador), Orientacao::default());

    let gatilho = cenario.falar(conversa, "engoliu o produto");
    cenario.turno(&motor, conversa, gatilho).await;
    // Uma chamada para o lote e uma para o risco — o risco não vem do cache do G1.
    assert_eq!(jev.pedidos().len(), 2);
    assert_eq!(
        jev.pedidos()[1].perguntas.keys().collect::<Vec<_>>(),
        vec![RISCO_PERGUNTA_ID]
    );
    // O estado da segunda pergunta é o mesmo da primeira (mesma conversa).
    assert_eq!(
        jev.pedidos()[0].estado["conversa"],
        jev.pedidos()[1].estado["conversa"]
    );
}
