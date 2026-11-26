use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use poph_jev::{
    DecisionOrigin, FakeJev, G1_CONTINUE, G1_TRANSFER, G2_PROCESS, G3_BASE, G3_NOTHING,
    GateEvaluator, GateOverrides, IdGate, JevEngine, JevError, JevMessage, JevOptions, JevState,
};
use poph_types::JevGates;

fn estado() -> JevState {
    JevState::new("cli")
        .with_messages([
            JevMessage::from_customer("oi, boa tarde"),
            JevMessage::from_agent("Boa tarde! Como posso ajudar?"),
            JevMessage::from_customer("queria saber o prazo de entrega"),
        ])
        .with_handoff(vec!["cliente pede segunda via".to_owned()], true)
        .with_capabilities(true, true)
        .with_departments(vec!["Comercial".to_owned(), "Suporte".to_owned()])
}

fn gates(g1: bool, g2: bool, g3: bool, g4: bool) -> JevGates {
    JevGates {
        g1,
        g2,
        g3,
        g4,
        ..JevGates::default()
    }
}

fn choices(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(id, choice)| ((*id).to_owned(), (*choice).to_owned()))
        .collect()
}

fn build_engine(jev: Arc<FakeJev>, global_gates: JevGates) -> Arc<JevEngine> {
    Arc::new(JevEngine::new(jev, global_gates, "jev-latest"))
}

#[tokio::test]
async fn three_gates_on_make_one_request_and_distribute_answers() {
    let jev = Arc::new(FakeJev::with_choices(choices(&[
        ("g1_transferir", G1_CONTINUE),
        ("g2_esperar", G2_PROCESS),
        ("g3_precisa", G3_BASE),
    ])));
    let evaluator = build_engine(Arc::clone(&jev), gates(true, true, true, false));

    let resultado = evaluator.evaluate(estado(), None).await;

    assert_eq!(jev.calls(), 1, "o lote é um pedido só");
    let request = &jev.requests()[0];
    assert_eq!(request.questions.len(), 3, "três perguntas no mesmo pedido");
    assert_eq!(request.modelo, "jev-latest");
    assert_eq!(resultado.call.questions, 3);
    assert_eq!(resultado.call.tokens, 12);
    assert!(!resultado.failed());

    assert_eq!(resultado.choice(IdGate::G1), Some(G1_CONTINUE));
    assert_eq!(resultado.choice(IdGate::G2), Some(G2_PROCESS));
    assert_eq!(resultado.choice(IdGate::G3), Some(G3_BASE));
    assert!(
        resultado.decision(IdGate::G4).is_none(),
        "G4 desligado não pergunta"
    );
    assert!(
        resultado
            .decisions
            .iter()
            .all(|decisao| decisao.origin == DecisionOrigin::Jev)
    );
}

#[tokio::test]
async fn request_carries_conversation_state() {
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g1_transferir",
        G1_CONTINUE,
    )])));
    let evaluator = build_engine(Arc::clone(&jev), gates(true, false, false, false));
    evaluator.evaluate(estado(), None).await;

    let sent = &jev.requests()[0].estado;
    assert_eq!(sent["canal"], "cli");
    assert_eq!(
        sent["conversa"]["mensagens"][2]["texto"],
        "queria saber o prazo de entrega"
    );
    assert_eq!(sent["politica"]["transferencia_habilitada"], true);
    assert_eq!(sent["capacidades"]["base_de_conhecimento"], true);
}

#[tokio::test]
async fn no_gate_on_sends_no_question() {
    let jev = Arc::new(FakeJev::with_choices(BTreeMap::new()));
    let evaluator = build_engine(Arc::clone(&jev), JevGates::default());

    let resultado = evaluator.evaluate(estado(), None).await;

    assert_eq!(jev.calls(), 0);
    assert!(resultado.decisions.is_empty());
    assert!(!resultado.failed());
}

#[tokio::test]
async fn cache_avoids_repeated_question_in_same_state() {
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g1_transferir",
        G1_TRANSFER,
    )])));
    let evaluator = build_engine(Arc::clone(&jev), gates(true, false, false, false));

    let first = evaluator.evaluate(estado(), None).await;
    let second = evaluator.evaluate(estado(), None).await;

    assert_eq!(jev.calls(), 1, "a segunda vez veio do cache");
    assert_eq!(first.choice(IdGate::G1), Some(G1_TRANSFER));
    assert_eq!(second.choice(IdGate::G1), Some(G1_TRANSFER));
    assert_eq!(
        second.decision(IdGate::G1).unwrap().origin,
        DecisionOrigin::Cache
    );
    assert_eq!(second.call.decided_by_cache, 1);
}

#[tokio::test]
async fn different_state_does_not_reuse_decision() {
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g1_transferir",
        G1_CONTINUE,
    )])));
    let evaluator = build_engine(Arc::clone(&jev), gates(true, false, false, false));

    evaluator.evaluate(estado(), None).await;
    let other = estado().with_messages([JevMessage::from_customer("quero falar com um atendente")]);
    evaluator.evaluate(other, None).await;

    assert_eq!(jev.calls(), 2);
}

#[tokio::test]
async fn jev_failure_sends_all_gates_to_fallback() {
    let jev = Arc::new(FakeJev::failing(JevError::Status {
        code: 503,
        motivo: "Service Unavailable".to_owned(),
    }));
    let evaluator = build_engine(Arc::clone(&jev), gates(true, true, false, false));

    let resultado = evaluator.evaluate(estado(), None).await;

    assert!(resultado.failed());
    assert_eq!(
        resultado.call.failure.as_deref(),
        Some("o Jev respondeu 503 (Service Unavailable)")
    );
    assert_eq!(resultado.decisions.len(), 2);
    assert!(
        resultado
            .decisions
            .iter()
            .all(|decisao| decisao.origin == DecisionOrigin::Fallback)
    );
    assert_eq!(resultado.choice(IdGate::G1), Some(G1_CONTINUE));
    assert!(resultado.reliable_choice(IdGate::G1).is_none());
}

#[tokio::test]
async fn expired_batch_deadline_falls_back() {
    let jev = Arc::new(
        FakeJev::with_choices(choices(&[("g1_transferir", G1_TRANSFER)]))
            .with_delay(Duration::from_secs(5)),
    );
    let evaluator = Arc::new(
        JevEngine::new(
            Arc::clone(&jev) as Arc<dyn poph_jev::JevTransport>,
            gates(true, false, false, false),
            "jev-latest",
        )
        .with_options(JevOptions {
            batch_timeout: Duration::from_millis(200),
            ..JevOptions::default()
        }),
    );

    let resultado = evaluator.evaluate(estado(), None).await;

    assert!(resultado.failed());
    assert!(
        resultado
            .call
            .failure
            .as_deref()
            .unwrap_or_default()
            .contains("não respondeu")
    );
    assert_eq!(
        resultado.decision(IdGate::G1).unwrap().origin,
        DecisionOrigin::Fallback
    );
}

#[tokio::test]
async fn ambiguous_answer_falls_back_with_reason() {
    let jev = Arc::new(FakeJev::new(|request| {
        let mut replies = BTreeMap::new();
        for id in request.questions.keys() {
            replies.insert(
                id.clone(),
                poph_jev::JevResponse {
                    choice: Some("opcao_que_nao_existe".to_owned()),
                    confidence: 0.9,
                    ..poph_jev::JevResponse::default()
                },
            );
        }
        Ok(poph_jev::BatchResponse {
            answers: replies,
            tokens: 3,
            ..poph_jev::BatchResponse::default()
        })
    }));
    let evaluator = build_engine(Arc::clone(&jev), gates(true, false, false, false));

    let resultado = evaluator.evaluate(estado(), None).await;
    let decisao = resultado.decision(IdGate::G1).expect("decisão do G1");

    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    assert!(
        decisao
            .motivo
            .as_deref()
            .unwrap_or_default()
            .contains("fora das opções"),
        "motivo: {:?}",
        decisao.motivo
    );
    assert_eq!(
        decisao.confidence, 0.9,
        "a confiança do Jev fica registrada mesmo no fallback"
    );
}

#[tokio::test]
async fn confidence_below_threshold_is_abstention() {
    let jev = Arc::new(FakeJev::new(|request| {
        let mut replies = BTreeMap::new();
        for id in request.questions.keys() {
            replies.insert(
                id.clone(),
                poph_jev::JevResponse {
                    choice: Some(G1_TRANSFER.to_owned()),
                    confidence: 0.55,
                    ..poph_jev::JevResponse::default()
                },
            );
        }
        Ok(poph_jev::BatchResponse {
            answers: replies,
            tokens: 1,
            ..poph_jev::BatchResponse::default()
        })
    }));
    let evaluator = build_engine(Arc::clone(&jev), gates(true, false, false, false));

    let resultado = evaluator.evaluate(estado(), None).await;
    let decisao = resultado.decision(IdGate::G1).unwrap();

    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    assert!(
        decisao
            .motivo
            .as_deref()
            .unwrap_or_default()
            .contains("confiança 0.55")
    );
    assert_eq!(resultado.choice(IdGate::G1), Some(G1_CONTINUE));
}

#[tokio::test]
async fn gate_that_does_not_apply_is_not_asked() {
    let jev = Arc::new(FakeJev::with_choices(choices(&[(
        "g1_transferir",
        G1_TRANSFER,
    )])));
    let evaluator = build_engine(Arc::clone(&jev), gates(true, false, false, false));
    let without_handoff = estado().with_handoff(Vec::new(), false);

    let resultado = evaluator.evaluate(without_handoff, None).await;

    assert_eq!(jev.calls(), 0);
    assert!(resultado.decisions.is_empty());
}

#[tokio::test]
async fn agent_override_changes_only_what_it_declares() {
    let jev = Arc::new(FakeJev::with_choices(BTreeMap::new()));
    let evaluator = build_engine(Arc::clone(&jev), gates(true, true, false, false));

    let overrides =
        GateOverrides::from_json(r#"{"gates": {"g1": false}}"#).expect("ajuste do agente");
    let effective = evaluator.effective_gates(Some(overrides));

    assert!(!effective.g1, "o agente desligou o G1");
    assert!(effective.g2, "o que ele não declarou herda a global");
    assert!(!effective.g3);

    let disabled = GateOverrides::from_json(r#"{"ativo": false}"#).unwrap();
    assert_eq!(
        evaluator.effective_gates(Some(disabled)),
        JevGates::default()
    );
}

#[tokio::test]
async fn two_gates_in_same_request_is_one_call() {
    let jev = Arc::new(FakeJev::with_choices(choices(&[
        ("g1_transferir", G1_CONTINUE),
        ("g3_precisa", G3_NOTHING),
    ])));
    let evaluator = build_engine(Arc::clone(&jev), gates(true, false, true, false));

    let resultado = evaluator.evaluate(estado(), None).await;

    assert_eq!(jev.calls(), 1);
    assert_eq!(jev.requests()[0].questions.len(), 2);
    assert_eq!(resultado.choice(IdGate::G3), Some(G3_NOTHING));
}
