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
use poph_jev::{BatchResponse, FakeJev, JevEngine, JevError, JevResponse as ReplyJudge};
use poph_store::{
    AgentChange, Direction, NewAgent, NewConversation, NewMessage, Page, Role, Store, TurnState,
};
use poph_types::{ConversationId, JevGates, MessageId, OrgId, StructuredInstructions};

struct NoWait;

impl Pause for NoWait {
    fn wait(&self, _duration: Duration) -> FuturePause {
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
        Self::with_judge_json(label, "")
    }

    fn with_judge_json(label: &str, jev_json: &str) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-hr34-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|time| time.as_nanos())
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

    fn say(&self, texto: &str) -> MessageId {
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
        scripts: Vec<Script>,
        jev: Option<Arc<JevEngine>>,
    ) -> (TurnEngine, Arc<FakeProvider>) {
        let fake = Arc::new(FakeProvider::with_scripts(scripts));
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
        .with_pause(Arc::new(NoWait))
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

    fn outputs(&self) -> Vec<String> {
        self.store
            .messages()
            .list(self.org, self.conversa, Page::with_limit(200))
            .expect("lista as mensagens")
            .into_iter()
            .filter(|mensagem| mensagem.direction == Direction::Outbound)
            .map(|mensagem| mensagem.texto)
            .collect()
    }

    fn queue(&self) -> Vec<String> {
        self.store
            .outbox()
            .list_pending(self.org, 50)
            .expect("lista a fila de saída")
            .into_iter()
            .map(|item| item.content_json)
            .collect()
    }

    fn turn_stored(&self, turno: &TurnSummary) -> String {
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
        .map(|time| time.as_nanos() as u64)
        .unwrap_or(0)
}

fn script_text(texto: &str) -> Script {
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

type ReplyScripted = (&'static str, f64);

fn judge_by_text(table: &[(&str, ReplyScripted)]) -> Arc<FakeJev> {
    let table: Vec<(String, String, f64)> = table
        .iter()
        .map(|(texto, (choice, confidence))| {
            ((*texto).to_owned(), (*choice).to_owned(), *confidence)
        })
        .collect();
    Arc::new(FakeJev::new(move |request| {
        let candidate = request.estado["resposta_candidata"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let mut replies = BTreeMap::new();
        if let Some((_, choice, confidence)) =
            table.iter().find(|(texto, _, _)| *texto == candidate)
        {
            replies.insert(
                "g6_revisao".to_owned(),
                ReplyJudge {
                    choice: Some(choice.clone()),
                    confidence: *confidence,
                    ..ReplyJudge::default()
                },
            );
        }
        Ok(BatchResponse {
            answers: replies,
            tokens: 20,
            ..BatchResponse::default()
        })
    }))
}

fn judge_out() -> Arc<FakeJev> {
    Arc::new(FakeJev::new(|_request: &poph_jev::JevRequest| {
        Err(JevError::Status {
            code: 503,
            motivo: "Service Unavailable".to_owned(),
        })
    }))
}

fn judge_failure_second(first: ReplyScripted) -> Arc<FakeJev> {
    let calls = Arc::new(AtomicU64::new(0));
    Arc::new(FakeJev::new(move |_request: &poph_jev::JevRequest| {
        let call = calls.fetch_add(1, Ordering::SeqCst);
        if call > 0 {
            return Err(JevError::Status {
                code: 503,
                motivo: "Service Unavailable".to_owned(),
            });
        }
        let mut replies = BTreeMap::new();
        replies.insert(
            "g6_revisao".to_owned(),
            ReplyJudge {
                choice: Some(first.0.to_owned()),
                confidence: first.1,
                ..ReplyJudge::default()
            },
        );
        Ok(BatchResponse {
            answers: replies,
            tokens: 20,
            ..BatchResponse::default()
        })
    }))
}

fn engine_judge(jev: Arc<FakeJev>, gates: JevGates) -> Arc<JevEngine> {
    Arc::new(JevEngine::new(jev, gates, "jev-latest"))
}

const QUESTION: &str = "quanto custa para dedetizar baratas no apê?";
const DRAFT: &str = "Sai R$ 150,00 e amanhã mandamos o atendente aí.";
const REESCRITA: &str = "A dedetização de baratas custa R$ 180,00 por visita.";
const SAFE: &str = "Vou confirmar o valor com a equipe e já te respondo.";

#[tokio::test]
async fn g6_approved_keeps_reply_does_call_model() {
    let scenario = Scenario::new("aprovada");
    let jev = judge_by_text(&[(DRAFT, ("aprovada", 0.95))]);
    let (engine, provedor) =
        scenario.engine(vec![script_text(DRAFT)], Some(engine_judge(jev, so_g6())));

    let gatilho = scenario.say(QUESTION);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, DRAFT);
    assert_eq!(provedor.received().len(), 1, "uma chamada ao modelo");
    assert_eq!(scenario.outputs(), vec![DRAFT]);
    assert_eq!(scenario.queue().len(), 1, "um item na fila de saída");
    assert_eq!(scenario.metrics.revisions_g6.value(), 1);
    assert_eq!(scenario.metrics.reprovadas_g6.value(), 0);
    assert_eq!(scenario.metrics.responses_safe_g6.value(), 0);
    let registro = scenario.turn_stored(&summary);
    assert!(registro.contains("\"revisao\""), "{registro}");
    assert!(registro.contains("\"aprovada\""), "{registro}");
}

#[tokio::test]
async fn g6_rejected_regenerates_once_sends_text_new() {
    let scenario = Scenario::new("regenera");
    let jev = judge_by_text(&[
        (DRAFT, ("revisar_preco", 0.91)),
        (REESCRITA, ("aprovada", 0.93)),
    ]);
    let (engine, provedor) = scenario.engine(
        vec![script_text(DRAFT), script_text(REESCRITA)],
        Some(engine_judge(jev, so_g6())),
    );

    let gatilho = scenario.say(QUESTION);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, REESCRITA, "sai o texto regenerado");
    let requests = provedor.received();
    assert_eq!(requests.len(), 2, "exatamente uma regeneração");
    let last = requests.last().expect("o pedido da regeneração");
    assert!(
        matches!(last.messages.last(), Some(poph_core::llm::Message::User(_))),
        "o pedido da regeneração termina com a orientação do usuário"
    );
    assert!(last.tools.is_empty(), "regeneração não reabre ferramenta");
    assert_eq!(scenario.outputs(), vec![REESCRITA], "uma única saída");
    assert_eq!(scenario.queue().len(), 1, "um único item na fila");
    assert_eq!(scenario.metrics.revisions_g6.value(), 2);
    assert_eq!(scenario.metrics.reprovadas_g6.value(), 1);
    assert_eq!(scenario.metrics.responses_safe_g6.value(), 0);
    assert_eq!(summary.input_tokens, 200);
    let registro = scenario.turn_stored(&summary);
    assert!(registro.contains("\"regenerou\":true"), "{registro}");
    assert!(
        registro.contains("\"segunda_escolha\":\"aprovada\""),
        "{registro}"
    );
}

#[tokio::test]
async fn g6_two_rejections_use_reply_safe() {
    let scenario =
        Scenario::with_judge_json("segura", &format!(r#"{{"resposta_segura": "{SAFE}"}}"#));
    let jev = judge_by_text(&[
        (DRAFT, ("revisar_preco", 0.91)),
        (REESCRITA, ("revisar_transferencia", 0.88)),
    ]);
    let (engine, provedor) = scenario.engine(
        vec![script_text(DRAFT), script_text(REESCRITA)],
        Some(engine_judge(jev, so_g6())),
    );

    let gatilho = scenario.say(QUESTION);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, SAFE, "a resposta segura é a que sai");
    assert_eq!(provedor.received().len(), 2, "sem terceira tentativa");
    assert_eq!(scenario.outputs(), vec![SAFE]);
    assert_eq!(scenario.queue().len(), 1);
    assert_eq!(scenario.metrics.responses_safe_g6.value(), 1);
    let registro = scenario.turn_stored(&summary);
    assert!(registro.contains("\"resposta_segura\":true"), "{registro}");
}

#[tokio::test]
async fn g6_without_reply_safe_second_rejection_sends_text_regenerated() {
    let scenario = Scenario::new("sem-segura");
    let jev = judge_by_text(&[
        (DRAFT, ("revisar_preco", 0.91)),
        (REESCRITA, ("revisar_longa", 0.85)),
    ]);
    let (engine, _provider) = scenario.engine(
        vec![script_text(DRAFT), script_text(REESCRITA)],
        Some(engine_judge(jev, so_g6())),
    );

    let gatilho = scenario.say(QUESTION);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(
        summary.texto, REESCRITA,
        "sem resposta segura, sai a regenerada"
    );
    assert_eq!(scenario.outputs(), vec![REESCRITA]);
    assert_eq!(scenario.metrics.responses_safe_g6.value(), 0);
}

#[tokio::test]
async fn g6_second_revision_without_reply_uses_safe() {
    let scenario = Scenario::with_judge_json(
        "segunda-fora",
        &format!(r#"{{"resposta_segura": "{SAFE}"}}"#),
    );
    let jev = judge_failure_second(("revisar_preco", 0.92));
    let (engine, provedor) = scenario.engine(
        vec![script_text(DRAFT), script_text(REESCRITA)],
        Some(engine_judge(jev, so_g6())),
    );

    let gatilho = scenario.say(QUESTION);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.texto, SAFE);
    assert_eq!(provedor.received().len(), 2);
    assert_eq!(scenario.metrics.responses_safe_g6.value(), 1);
}

#[tokio::test]
async fn g6_judge_out_not_reviews_keeps_reply() {
    let scenario = Scenario::new("fora");
    let (engine, provedor) = scenario.engine(
        vec![script_text(DRAFT)],
        Some(engine_judge(judge_out(), so_g6())),
    );

    let gatilho = scenario.say(QUESTION);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(summary.texto, DRAFT, "Jev fora = comportamento de hoje");
    assert_eq!(provedor.received().len(), 1, "sem regeneração");
    assert_eq!(scenario.metrics.reprovadas_g6.value(), 0);
}

#[tokio::test]
async fn g6_abstention_confidence_low_keeps_reply() {
    for (label, table) in [
        ("sem-resposta", Vec::new()),
        ("baixa-confianca", vec![(DRAFT, ("revisar_preco", 0.45))]),
        ("fora-do-contrato", vec![(DRAFT, ("reescrever_tudo", 0.99))]),
    ] {
        let scenario = Scenario::new(label);
        let jev = judge_by_text(&table);
        let (engine, provedor) =
            scenario.engine(vec![script_text(DRAFT)], Some(engine_judge(jev, so_g6())));
        let gatilho = scenario.say(QUESTION);
        let summary = scenario.turno(&engine, gatilho).await;
        assert_eq!(summary.texto, DRAFT, "{label}");
        assert_eq!(provedor.received().len(), 1, "{label}: sem regeneração");
        assert_eq!(scenario.metrics.reprovadas_g6.value(), 0, "{label}");
    }
}

#[tokio::test]
async fn g6_disabled_not_question_keeps_reply() {
    let scenario = Scenario::new("desligado");
    let jev = judge_by_text(&[(DRAFT, ("revisar_preco", 0.99))]);
    let (engine, provedor) = scenario.engine(
        vec![script_text(DRAFT)],
        Some(engine_judge(Arc::clone(&jev), JevGates::default_value())),
    );

    let gatilho = scenario.say(QUESTION);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.texto, DRAFT);
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
async fn g6_failure_model_regeneration_keeps_reply_original() {
    let scenario = Scenario::new("falha-modelo");
    let jev = judge_by_text(&[(DRAFT, ("revisar_preco", 0.93))]);
    let (engine, provedor) = scenario.engine(
        vec![
            script_text(DRAFT),
            Script::Failure(LlmError::ConnectionInterrupted("conexão caiu".to_owned())),
        ],
        Some(engine_judge(jev, so_g6())),
    );

    let gatilho = scenario.say(QUESTION);
    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed);
    assert_eq!(
        summary.texto, DRAFT,
        "a falha da reescrita não derruba o turno"
    );
    assert_eq!(provedor.received().len(), 2);
    assert_eq!(scenario.outputs(), vec![DRAFT]);
    assert_eq!(scenario.metrics.responses_safe_g6.value(), 0);
}

#[tokio::test]
#[ignore = "medição: imprime o custo do G6 nas duas condições"]
async fn measurement_g6() {
    let aprovadas = 15usize;
    let regenerated = 6usize;
    let safe_ones = 3usize;
    let total = aprovadas + regenerated + safe_ones;

    let mut messages: Vec<String> = Vec::new();
    let mut scripts: Vec<Script> = Vec::new();
    let mut table: Vec<(String, String, f64)> = Vec::new();
    for index in 0..total {
        let question = format!("mensagem {index}: quero um orçamento de dedetização");
        messages.push(question);
        let draft = format!("rascunho {index}: fica R$ 150,00 e já te passo para um atendente.");
        let reescrita =
            format!("rascunho {index}: a dedetização de baratas custa R$ 180,00 por visita.");
        scripts.push(script_text(&draft));
        if index < aprovadas {
            table.push((draft, "aprovada".to_owned(), 0.93));
            continue;
        }
        table.push((draft, "revisar_preco".to_owned(), 0.93));
        scripts.push(script_text(&reescrita));
        if index < aprovadas + regenerated {
            table.push((reescrita, "aprovada".to_owned(), 0.91));
        } else {
            table.push((reescrita, "revisar_transferencia".to_owned(), 0.90));
        }
    }

    let mut results = Vec::new();
    for (label, linked) in [("sem G6", false), ("com G6", true)] {
        let scenario = if linked {
            Scenario::with_judge_json(
                "medicao-com",
                &format!(r#"{{"resposta_segura": "{SAFE}"}}"#),
            )
        } else {
            Scenario::new("medicao-sem")
        };
        let table_json: Vec<(String, String, f64)> = table.clone();
        let jev = Arc::new(FakeJev::new(move |request| {
            let candidate = request.estado["resposta_candidata"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let mut replies = BTreeMap::new();
            if let Some((_, choice, confidence)) =
                table_json.iter().find(|(texto, _, _)| *texto == candidate)
            {
                replies.insert(
                    "g6_revisao".to_owned(),
                    ReplyJudge {
                        choice: Some(choice.clone()),
                        confidence: *confidence,
                        ..ReplyJudge::default()
                    },
                );
            }
            Ok(BatchResponse {
                answers: replies,
                tokens: 20,
                ..BatchResponse::default()
            })
        }));
        let gates = if linked { so_g6() } else { JevGates::default() };
        let engine_judge = Arc::new(JevEngine::new(
            Arc::clone(&jev) as Arc<dyn poph_jev::JevTransport>,
            gates,
            "jev-latest",
        ));
        let (engine, provedor) = scenario.engine(scripts.clone(), Some(engine_judge));
        for mensagem in &messages {
            let gatilho = scenario.say(mensagem);
            let summary = scenario.turno(&engine, gatilho).await;
            assert_eq!(summary.estado, TurnState::Completed);
        }
        results.push((
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
    for (label, modelo, revision, regeneracoes, safe_ones) in &results {
        println!("{label} | {modelo} | {revision} | {regeneracoes} | {safe_ones}");
    }
    let without = &results[0];
    let com = &results[1];
    println!(
        "acréscimo com o gate ligado: {} chamadas ao modelo (+{:.0}%) · {:.2} revisões por turno",
        com.1 - without.1,
        (com.1 as f64 / without.1 as f64 - 1.0) * 100.0,
        com.2 as f64 / total as f64
    );
}
