use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::turno::{
    FuturePause, NoopObserver, Pause, TurnEngine, TurnInput, TurnOptions, TurnSummary,
};
use poph_core::llm::{FakeProvider, FinishReason, LlmError, LlmEvent, Script, Usage};
use poph_jev::{BatchResponse, FakeJev, JevEngine, JevError, JevResponse as RespostaJev};
use poph_store::{
    AgentChange, Direction, NewAgent, NewConversation, NewMessage, Page, Role, Store, TurnState,
};
use poph_types::{ConversationId, JevGates, MessageId, OrgId, StructuredInstructions};

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
    conversa: ConversationId,
    metrics: MetricsTurn,
}

impl Scenario {
    fn new(label: &str) -> Self {
        Self::com_jev_json(label, "")
    }

    fn com_jev_json(label: &str, jev_json: &str) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-hr34-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|tempo| tempo.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("pasta do teste");
        let store = Arc::new(
            Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
                .expect("abre o banco"),
        );
        let org = store
            .orgs()
            .create(poph_store::NewOrg {
                name: format!("Org {label}"),
                slug: format!("org-{label}-{}-{}", std::process::id(), fastrand()),
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
                    slug: format!("bia-{label}-{}", fastrand()),
                    grouping_window_ms: 0,
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente");
        let structured = StructuredInstructions::default();
        store
            .instructions()
            .save(
                org,
                agente.id,
                0,
                &structured.to_json(),
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
            folder,
            store,
            org,
            conversa,
            metrics: MetricsTurn::own(),
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

    fn engine(
        &self,
        roteiros: Vec<Script>,
        jev: Option<Arc<JevEngine>>,
    ) -> (TurnEngine, Arc<FakeProvider>) {
        let fake = Arc::new(FakeProvider::with_scripts(roteiros));
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
            fake.clone(),
            TurnOptions::default(),
            self.metrics.clone(),
        )
        .with_pause(Arc::new(SemEspera))
        .with_summary(poph_agent::summary::SummaryOptions {
            ativo: false,
            ..poph_agent::summary::SummaryOptions::default()
        });
        if let Some(jev) = jev {
            engine = engine.with_jev(jev);
        }
        (engine, fake)
    }

    async fn turno(&self, engine: &TurnEngine, gatilho: MessageId) -> TurnSummary {
        engine
            .run(
                TurnInput::new(self.org, self.conversa, gatilho),
                &mut NoopObserver,
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

    fn turno_gravado(&self, turno: &TurnSummary) -> String {
        self.store
            .turns()
            .get(self.org, turno.turno)
            .expect("lê o turno")
            .jev_json
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

fn fastrand() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|tempo| tempo.as_nanos() as u64)
        .unwrap_or(0)
}

fn roteiro_texto(texto: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(texto.to_owned()),
        LlmEvent::Usage(Usage {
            input: 100,
            saida: 10,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

fn so_g6() -> JevGates {
    JevGates {
        g6: true,
        ..JevGates::default()
    }
}

type RespostaRoteirizada = (&'static str, f64);

fn jev_por_texto(tabela: &[(&str, RespostaRoteirizada)]) -> Arc<FakeJev> {
    let tabela: Vec<(String, String, f64)> = tabela
        .iter()
        .map(|(texto, (choice, confidence))| {
            ((*texto).to_owned(), (*choice).to_owned(), *confidence)
        })
        .collect();
    Arc::new(FakeJev::new(move |request| {
        let candidata = request.estado["resposta_candidata"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let mut respostas = BTreeMap::new();
        if let Some((_, choice, confidence)) =
            tabela.iter().find(|(texto, _, _)| *texto == candidata)
        {
            respostas.insert(
                "g6_revisao".to_owned(),
                RespostaJev {
                    choice: Some(choice.clone()),
                    confidence: *confidence,
                    ..RespostaJev::default()
                },
            );
        }
        Ok(BatchResponse {
            answers: respostas,
            tokens: 20,
            ..BatchResponse::default()
        })
    }))
}

fn jev_fora() -> Arc<FakeJev> {
    Arc::new(FakeJev::new(|_pedido: &poph_jev::JevRequest| {
        Err(JevError::Status {
            code: 503,
            motivo: "Service Unavailable".to_owned(),
        })
    }))
}

fn jev_que_falha_na_segunda(first: RespostaRoteirizada) -> Arc<FakeJev> {
    let calls = Arc::new(AtomicU64::new(0));
    Arc::new(FakeJev::new(move |_pedido: &poph_jev::JevRequest| {
        let call = calls.fetch_add(1, Ordering::SeqCst);
        if call > 0 {
            return Err(JevError::Status {
                code: 503,
                motivo: "Service Unavailable".to_owned(),
            });
        }
        let mut respostas = BTreeMap::new();
        respostas.insert(
            "g6_revisao".to_owned(),
            RespostaJev {
                choice: Some(first.0.to_owned()),
                confidence: first.1,
                ..RespostaJev::default()
            },
        );
        Ok(BatchResponse {
            answers: respostas,
            tokens: 20,
            ..BatchResponse::default()
        })
    }))
}

fn motor_jev(jev: Arc<FakeJev>, gates: JevGates) -> Arc<JevEngine> {
    Arc::new(JevEngine::new(jev, gates, "jev-latest"))
}

const PERGUNTA: &str = "quanto custa para dedetizar baratas no apê?";
const RASCUNHO: &str = "Sai R$ 150,00 e amanhã mandamos o atendente aí.";
const REESCRITA: &str = "A dedetização de baratas custa R$ 180,00 por visita.";
const SEGURA: &str = "Vou confirmar o valor com a equipe e já te respondo.";

#[tokio::test]
async fn g6_aprovada_mantem_a_resposta_e_faz_uma_chamada_ao_modelo() {
    let scenario = Scenario::new("aprovada");
    let jev = jev_por_texto(&[(RASCUNHO, ("aprovada", 0.95))]);
    let (engine, provedor) =
        scenario.engine(vec![roteiro_texto(RASCUNHO)], Some(motor_jev(jev, so_g6())));

    let gatilho = scenario.falar(PERGUNTA);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, RASCUNHO);
    assert_eq!(provedor.received().len(), 1, "uma chamada ao modelo");
    assert_eq!(scenario.saidas(), vec![RASCUNHO]);
    assert_eq!(scenario.fila().len(), 1, "um item na fila de saída");
    assert_eq!(scenario.metrics.revisions_g6.value(), 1);
    assert_eq!(scenario.metrics.reprovadas_g6.value(), 0);
    assert_eq!(scenario.metrics.responses_safe_g6.value(), 0);
    let registro = scenario.turno_gravado(&summary);
    assert!(registro.contains("\"revisao\""), "{registro}");
    assert!(registro.contains("\"aprovada\""), "{registro}");
}

#[tokio::test]
async fn g6_reprovada_regenera_uma_vez_e_envia_o_texto_novo() {
    let scenario = Scenario::new("regenera");
    let jev = jev_por_texto(&[
        (RASCUNHO, ("revisar_preco", 0.91)),
        (REESCRITA, ("aprovada", 0.93)),
    ]);
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto(RASCUNHO), roteiro_texto(REESCRITA)],
        Some(motor_jev(jev, so_g6())),
    );

    let gatilho = scenario.falar(PERGUNTA);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, REESCRITA, "sai o texto regenerado");
    let pedidos = provedor.received();
    assert_eq!(pedidos.len(), 2, "exatamente uma regeneração");
    let last = pedidos.last().expect("o pedido da regeneração");
    assert!(
        matches!(last.messages.last(), Some(poph_core::llm::Message::User(_))),
        "o pedido da regeneração termina com a orientação do usuário"
    );
    assert!(last.tools.is_empty(), "regeneração não reabre ferramenta");
    assert_eq!(scenario.saidas(), vec![REESCRITA], "uma única saída");
    assert_eq!(scenario.fila().len(), 1, "um único item na fila");
    assert_eq!(scenario.metrics.revisions_g6.value(), 2);
    assert_eq!(scenario.metrics.reprovadas_g6.value(), 1);
    assert_eq!(scenario.metrics.responses_safe_g6.value(), 0);
    assert_eq!(summary.input_tokens, 200);
    let registro = scenario.turno_gravado(&summary);
    assert!(registro.contains("\"regenerou\":true"), "{registro}");
    assert!(
        registro.contains("\"segunda_escolha\":\"aprovada\""),
        "{registro}"
    );
}

#[tokio::test]
async fn g6_duas_reprovacoes_usam_a_resposta_segura() {
    let scenario =
        Scenario::com_jev_json("segura", &format!(r#"{{"resposta_segura": "{SEGURA}"}}"#));
    let jev = jev_por_texto(&[
        (RASCUNHO, ("revisar_preco", 0.91)),
        (REESCRITA, ("revisar_transferencia", 0.88)),
    ]);
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto(RASCUNHO), roteiro_texto(REESCRITA)],
        Some(motor_jev(jev, so_g6())),
    );

    let gatilho = scenario.falar(PERGUNTA);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, SEGURA, "a resposta segura é a que sai");
    assert_eq!(provedor.received().len(), 2, "sem terceira tentativa");
    assert_eq!(scenario.saidas(), vec![SEGURA]);
    assert_eq!(scenario.fila().len(), 1);
    assert_eq!(scenario.metrics.responses_safe_g6.value(), 1);
    let registro = scenario.turno_gravado(&summary);
    assert!(registro.contains("\"resposta_segura\":true"), "{registro}");
}

#[tokio::test]
async fn g6_sem_resposta_segura_na_segunda_reprovacao_envia_o_texto_regenerado() {
    let scenario = Scenario::new("sem-segura");
    let jev = jev_por_texto(&[
        (RASCUNHO, ("revisar_preco", 0.91)),
        (REESCRITA, ("revisar_longa", 0.85)),
    ]);
    let (engine, _provedor) = scenario.engine(
        vec![roteiro_texto(RASCUNHO), roteiro_texto(REESCRITA)],
        Some(motor_jev(jev, so_g6())),
    );

    let gatilho = scenario.falar(PERGUNTA);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(
        summary.texto, REESCRITA,
        "sem resposta segura, sai a regenerada"
    );
    assert_eq!(scenario.saidas(), vec![REESCRITA]);
    assert_eq!(scenario.metrics.responses_safe_g6.value(), 0);
}

#[tokio::test]
async fn g6_segunda_revisao_sem_resposta_usa_a_segura() {
    let scenario = Scenario::com_jev_json(
        "segunda-fora",
        &format!(r#"{{"resposta_segura": "{SEGURA}"}}"#),
    );
    let jev = jev_que_falha_na_segunda(("revisar_preco", 0.92));
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto(RASCUNHO), roteiro_texto(REESCRITA)],
        Some(motor_jev(jev, so_g6())),
    );

    let gatilho = scenario.falar(PERGUNTA);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.texto, SEGURA);
    assert_eq!(provedor.received().len(), 2);
    assert_eq!(scenario.metrics.responses_safe_g6.value(), 1);
}

#[tokio::test]
async fn g6_jev_fora_nao_revisa_e_mantem_a_resposta() {
    let scenario = Scenario::new("fora");
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto(RASCUNHO)],
        Some(motor_jev(jev_fora(), so_g6())),
    );

    let gatilho = scenario.falar(PERGUNTA);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, RASCUNHO, "Jev fora = comportamento de hoje");
    assert_eq!(provedor.received().len(), 1, "sem regeneração");
    assert_eq!(scenario.metrics.reprovadas_g6.value(), 0);
}

#[tokio::test]
async fn g6_abstencao_e_confianca_baixa_mantem_a_resposta() {
    for (label, tabela) in [
        ("sem-resposta", Vec::new()),
        ("baixa-confianca", vec![(RASCUNHO, ("revisar_preco", 0.45))]),
        (
            "fora-do-contrato",
            vec![(RASCUNHO, ("reescrever_tudo", 0.99))],
        ),
    ] {
        let scenario = Scenario::new(label);
        let jev = jev_por_texto(&tabela);
        let (engine, provedor) =
            scenario.engine(vec![roteiro_texto(RASCUNHO)], Some(motor_jev(jev, so_g6())));
        let gatilho = scenario.falar(PERGUNTA);
        let summary = scenario.turno(&engine, gatilho).await;
        assert_eq!(summary.texto, RASCUNHO, "{label}");
        assert_eq!(provedor.received().len(), 1, "{label}: sem regeneração");
        assert_eq!(scenario.metrics.reprovadas_g6.value(), 0, "{label}");
    }
}

#[tokio::test]
async fn g6_desligado_nao_pergunta_e_mantem_a_resposta() {
    let scenario = Scenario::new("desligado");
    let jev = jev_por_texto(&[(RASCUNHO, ("revisar_preco", 0.99))]);
    let (engine, provedor) = scenario.engine(
        vec![roteiro_texto(RASCUNHO)],
        Some(motor_jev(Arc::clone(&jev), JevGates::default_value())),
    );

    let gatilho = scenario.falar(PERGUNTA);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.texto, RASCUNHO);
    assert_eq!(provedor.received().len(), 1);
    assert!(
        jev.requests().is_empty(),
        "o G6 desligado não pergunta nada"
    );
    assert_eq!(scenario.metrics.revisions_g6.value(), 0);
    assert!(
        !JevGates::default_value().g6,
        "o G6 não entra no padrão de instalação"
    );
}

#[tokio::test]
async fn g6_falha_do_modelo_na_regeneracao_mantem_a_resposta_original() {
    let scenario = Scenario::new("falha-modelo");
    let jev = jev_por_texto(&[(RASCUNHO, ("revisar_preco", 0.93))]);
    let (engine, provedor) = scenario.engine(
        vec![
            roteiro_texto(RASCUNHO),
            Script::Failure(LlmError::ConnectionInterrupted("conexão caiu".to_owned())),
        ],
        Some(motor_jev(jev, so_g6())),
    );

    let gatilho = scenario.falar(PERGUNTA);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(
        summary.texto, RASCUNHO,
        "a falha da reescrita não derruba o turno"
    );
    assert_eq!(provedor.received().len(), 2);
    assert_eq!(scenario.saidas(), vec![RASCUNHO]);
    assert_eq!(scenario.metrics.responses_safe_g6.value(), 0);
}

#[tokio::test]
#[ignore = "medição: imprime o custo do G6 nas duas condições"]
async fn medicao_do_g6() {
    let aprovadas = 15usize;
    let regeneradas = 6usize;
    let seguras = 3usize;
    let total = aprovadas + regeneradas + seguras;

    let mut messages: Vec<String> = Vec::new();
    let mut roteiros: Vec<Script> = Vec::new();
    let mut tabela: Vec<(String, String, f64)> = Vec::new();
    for index in 0..total {
        let pergunta = format!("mensagem {index}: quero um orçamento de dedetização");
        messages.push(pergunta);
        let rascunho = format!("rascunho {index}: fica R$ 150,00 e já te passo para um atendente.");
        let reescrita =
            format!("rascunho {index}: a dedetização de baratas custa R$ 180,00 por visita.");
        roteiros.push(roteiro_texto(&rascunho));
        if index < aprovadas {
            tabela.push((rascunho, "aprovada".to_owned(), 0.93));
            continue;
        }
        tabela.push((rascunho, "revisar_preco".to_owned(), 0.93));
        roteiros.push(roteiro_texto(&reescrita));
        if index < aprovadas + regeneradas {
            tabela.push((reescrita, "aprovada".to_owned(), 0.91));
        } else {
            tabela.push((reescrita, "revisar_transferencia".to_owned(), 0.90));
        }
    }

    let mut resultados = Vec::new();
    for (label, ligado) in [("sem G6", false), ("com G6", true)] {
        let scenario = if ligado {
            Scenario::com_jev_json(
                "medicao-com",
                &format!(r#"{{"resposta_segura": "{SEGURA}"}}"#),
            )
        } else {
            Scenario::new("medicao-sem")
        };
        let tabela_json: Vec<(String, String, f64)> = tabela.clone();
        let jev = Arc::new(FakeJev::new(move |request| {
            let candidata = request.estado["resposta_candidata"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let mut respostas = BTreeMap::new();
            if let Some((_, choice, confidence)) =
                tabela_json.iter().find(|(texto, _, _)| *texto == candidata)
            {
                respostas.insert(
                    "g6_revisao".to_owned(),
                    RespostaJev {
                        choice: Some(choice.clone()),
                        confidence: *confidence,
                        ..RespostaJev::default()
                    },
                );
            }
            Ok(BatchResponse {
                answers: respostas,
                tokens: 20,
                ..BatchResponse::default()
            })
        }));
        let gates = if ligado { so_g6() } else { JevGates::default() };
        let motor_jev = Arc::new(JevEngine::new(
            Arc::clone(&jev) as Arc<dyn poph_jev::JevTransport>,
            gates,
            "jev-latest",
        ));
        let (engine, provedor) = scenario.engine(roteiros.clone(), Some(motor_jev));
        for mensagem in &messages {
            let gatilho = scenario.falar(mensagem);
            let summary = scenario.turno(&engine, gatilho).await;
            assert_eq!(summary.estado, TurnState::Completed);
        }
        resultados.push((
            label,
            provedor.received().len(),
            jev.requests().len(),
            scenario.metrics.reprovadas_g6.value(),
            scenario.metrics.responses_safe_g6.value(),
        ));
    }

    println!("\nG6 · custo por turno ({total} turnos, mesmo roteiro nas duas condições)");
    println!(
        "condição | chamadas ao modelo | chamadas à revisão (Jev) | regenerações | respostas seguras"
    );
    for (label, modelo, revision, regeneracoes, seguras) in &resultados {
        println!("{label} | {modelo} | {revision} | {regeneracoes} | {seguras}");
    }
    let without = &resultados[0];
    let com = &resultados[1];
    println!(
        "acréscimo com o gate ligado: {} chamadas ao modelo (+{:.0}%) · {:.2} revisões por turno",
        com.1 - without.1,
        (com.1 as f64 / without.1 as f64 - 1.0) * 100.0,
        com.2 as f64 / total as f64
    );
}
