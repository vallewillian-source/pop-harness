use std::collections::BTreeMap;
use std::sync::Arc;

use poph_jev::{
    DecisionOrigin, FakeJev, G6, G6_APPROVED, G6_REVIEW, G6_REVIEW_LONG, G6_REVIEW_PRICE,
    G6_REVIEW_SCOPE, G6_REVIEW_TONE, G6_REVIEW_TRANSFER, Gate, GateEvaluator, IdGate, JevEngine,
    JevMessage, JevResponse, JevState, QuestionType, eval::gate_override, g6_guidance,
    g6_needs_review,
};
use poph_types::JevGates;

fn estado(candidate: &str) -> JevState {
    JevState::new("whatsapp")
        .with_messages(vec![
            JevMessage::from_agent("Qual é a sua dúvida?"),
            JevMessage::from_customer("quanto custa para dedetizar baratas?"),
        ])
        .with_capabilities(true, true)
        .with_candidate_response(candidate)
}

fn response(choice: &str, confidence_value: f64) -> JevResponse {
    let mut item = JevResponse {
        choice: Some(choice.to_owned()),
        confidence: confidence_value,
        ..JevResponse::default()
    };
    item.probabilities
        .insert(choice.to_owned(), confidence_value);
    item
}

#[test]
fn question_has_approved_and_five_reasons() {
    let question = G6.question(&estado("Sai R$ 180,00."));
    assert_eq!(question.tipo, QuestionType::Choice);
    assert!(!question.instructions.is_empty());
    let options: Vec<&String> = question.rubric.keys().collect();
    assert_eq!(
        options,
        vec![
            "aprovada",
            "revisar_escopo",
            "revisar_longa",
            "revisar_preco",
            "revisar_tom",
            "revisar_transferencia",
        ]
    );
}

#[test]
fn gate_does_not_apply_without_candidate_under_review() {
    let no_candidate = JevState::new("whatsapp")
        .with_messages(vec![JevMessage::from_customer("oi")])
        .with_capabilities(true, true);
    assert!(!G6.applies(&no_candidate));
    assert!(G6.applies(&estado("Sai R$ 180,00.")));
}

#[test]
fn candidate_under_review_is_truncated() {
    let long_text = "a".repeat(poph_jev::MAX_CANDIDATE_RESPONSE_CHARS + 50);
    let estado = JevState::new("whatsapp").with_candidate_response(long_text);
    let texto = estado.candidate_response.unwrap_or_default();
    assert_eq!(
        texto.chars().count(),
        poph_jev::MAX_CANDIDATE_RESPONSE_CHARS + 1
    );
    assert!(texto.ends_with('…'));
}

#[test]
fn approved_is_reliable_decision() {
    let decisao = G6.interpret(
        &estado("Sai R$ 180,00 por visita."),
        Some(&response(G6_APPROVED, 0.93)),
        G6.thresholds(),
    );
    assert_eq!(decisao.origin, DecisionOrigin::Jev);
    assert_eq!(decisao.choice.as_deref(), Some(G6_APPROVED));
    assert_eq!(decisao.motivo, None);
}

#[test]
fn each_review_reason_is_reliable_decision_with_guidance() {
    for motivo in G6_REVIEW {
        let decisao = G6.interpret(
            &estado("Amanhã mandamos alguém."),
            Some(&response(motivo, 0.88)),
            G6.thresholds(),
        );
        assert_eq!(decisao.origin, DecisionOrigin::Jev, "{motivo}");
        assert_eq!(decisao.choice.as_deref(), Some(motivo));
        assert!(g6_needs_review(motivo));
        let guidance = g6_guidance(motivo);
        assert!(
            guidance.starts_with("Revise a resposta anterior"),
            "{motivo}"
        );
    }
    assert!(!g6_needs_review(G6_APPROVED));
}

#[test]
fn low_confidence_approves() {
    let decisao = G6.interpret(
        &estado("Sai R$ 180,00."),
        Some(&response(G6_REVIEW_PRICE, 0.45)),
        G6.thresholds(),
    );
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    assert_eq!(decisao.choice.as_deref(), Some(G6_APPROVED));
    let motivo = decisao.motivo.unwrap_or_default();
    assert!(motivo.contains("abaixo do limiar"), "{motivo}");
}

#[test]
fn option_outside_contract_approves() {
    let decisao = G6.interpret(
        &estado("Sai R$ 180,00."),
        Some(&response("reescrever_tudo", 1.0)),
        G6.thresholds(),
    );
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    assert_eq!(decisao.choice.as_deref(), Some(G6_APPROVED));
    let motivo = decisao.motivo.unwrap_or_default();
    assert!(motivo.contains("fora das opções"), "{motivo}");
}

#[test]
fn missing_jev_response_approves() {
    let decisao = G6.interpret(&estado("Sai R$ 180,00."), None, G6.thresholds());
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    assert_eq!(decisao.choice.as_deref(), Some(G6_APPROVED));
    assert_eq!(decisao.motivo.as_deref(), Some("sem resposta do Jev"));
}

#[tokio::test]
async fn g6_is_asked_outside_batch_only_with_candidate_under_review() {
    let mut choices: BTreeMap<String, String> = BTreeMap::new();
    choices.insert("g6_revisao".to_owned(), G6_REVIEW_PRICE.to_owned());
    let jev = Arc::new(FakeJev::with_choices(choices));
    let globals = JevGates {
        g6: true,
        ..JevGates::default()
    };
    let transport: Arc<dyn poph_jev::JevTransport> = jev.clone();
    let evaluator = JevEngine::new(transport, globals, "jev-latest");

    let no_candidate = JevState::new("whatsapp")
        .with_messages(vec![JevMessage::from_customer("quanto custa?")])
        .with_capabilities(true, true);
    let batch = evaluator.evaluate(no_candidate, None).await;
    assert!(batch.decisions.is_empty(), "{:?}", batch.decisions);
    assert!(jev.requests().is_empty(), "nenhum pedido ao Jev");

    let resultado = evaluator
        .evaluate(estado("Sai R$ 180,00."), Some(gate_override(IdGate::G6)))
        .await;
    let requests = jev.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]
            .questions
            .keys()
            .cloned()
            .collect::<Vec<String>>(),
        vec!["g6_revisao".to_owned()]
    );
    assert_eq!(resultado.reliable_choice(IdGate::G6), Some(G6_REVIEW_PRICE));
    assert_eq!(
        requests[0].estado["resposta_candidata"].as_str(),
        Some("Sai R$ 180,00.")
    );
}

#[test]
fn state_without_candidate_has_no_field() {
    let estado = JevState::new("whatsapp").with_messages(vec![JevMessage::from_customer("oi")]);
    let json = estado.to_json();
    assert!(json.get("resposta_candidata").is_none());
}

#[test]
fn guidance_is_distinct_per_reason() {
    let texts: Vec<&str> = [
        G6_REVIEW_PRICE,
        G6_REVIEW_TRANSFER,
        G6_REVIEW_SCOPE,
        G6_REVIEW_LONG,
        G6_REVIEW_TONE,
    ]
    .into_iter()
    .map(g6_guidance)
    .collect();
    let mut unique = texts.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), texts.len(), "{texts:?}");
}
