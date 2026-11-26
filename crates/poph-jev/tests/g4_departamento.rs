use std::collections::BTreeMap;

use poph_jev::{
    BatchResponse, DecisionOrigin, FakeJev, G4, Gate, GateEvaluator, IdGate, JevEngine, JevMessage,
    JevRequest, JevResponse, JevState, eval::gate_override, match_candidate,
};

fn estado(departments: &[&str], texto: &str) -> JevState {
    JevState::new("whatsapp")
        .with_messages(vec![JevMessage::from_customer(texto)])
        .with_departments(departments.iter().map(|nome| (*nome).to_owned()).collect())
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

fn candidates() -> Vec<String> {
    ["Comercial", "Suporte", "Financeiro"]
        .iter()
        .map(|nome| (*nome).to_owned())
        .collect()
}

#[test]
fn question_has_one_option_per_department() {
    let question = G4.question(&estado(
        &["Comercial", "Suporte", "Financeiro"],
        "chegou quebrado",
    ));
    assert_eq!(question.tipo, poph_jev::QuestionType::Choice);
    assert!(!question.instructions.is_empty());
    let options: Vec<&String> = question.rubric.keys().collect();
    assert_eq!(options, vec!["Comercial", "Financeiro", "Suporte"]);
}

#[test]
fn gate_does_not_apply_without_possible_choice() {
    assert!(!G4.applies(&estado(&["Suporte"], "chegou quebrado")));
    assert!(!G4.applies(&estado(&[], "chegou quebrado")));
    assert!(G4.applies(&estado(&["Suporte", "Comercial"], "chegou quebrado")));
}

#[test]
fn valid_choice_becomes_reliable_decision() {
    let estado = estado(
        &["Comercial", "Suporte", "Financeiro"],
        "o produto chegou quebrado",
    );
    let decisao = G4.interpret(&estado, Some(&response("Suporte", 0.94)), G4.thresholds());
    assert_eq!(decisao.choice.as_deref(), Some("Suporte"));
    assert_eq!(decisao.origin, DecisionOrigin::Jev);
    assert_eq!(decisao.motivo, None);
    assert!(decisao.is_reliable());
}

#[test]
fn choice_with_other_case_becomes_canonical_name() {
    let estado = estado(
        &["Comercial", "Suporte", "Financeiro"],
        "quero a segunda via do boleto",
    );
    let decisao = G4.interpret(&estado, Some(&response("financeiro", 0.9)), G4.thresholds());
    assert_eq!(decisao.choice.as_deref(), Some("Financeiro"));
    assert!(decisao.is_reliable());
}

#[test]
fn choice_outside_options_falls_back_with_reason() {
    let estado = estado(
        &["Comercial", "Suporte", "Financeiro"],
        "quero falar do contrato",
    );
    let decisao = G4.interpret(&estado, Some(&response("Jurídico", 0.99)), G4.thresholds());
    assert_eq!(decisao.choice, None);
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    let motivo = decisao.motivo.unwrap_or_default();
    assert!(
        motivo.contains("não é um departamento ativo"),
        "motivo devia dizer que não é departamento da empresa: {motivo}"
    );
}

#[test]
fn confidence_below_threshold_falls_back() {
    let estado = estado(
        &["Comercial", "Suporte", "Financeiro"],
        "quero negociar um desconto",
    );
    let decisao = G4.interpret(&estado, Some(&response("Comercial", 0.61)), G4.thresholds());
    assert_eq!(decisao.choice, None);
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    let motivo = decisao.motivo.unwrap_or_default();
    assert!(motivo.contains("confiança 0.61"), "motivo: {motivo}");
}

#[test]
fn no_response_falls_back() {
    let estado = estado(&["Comercial", "Suporte"], "oi");
    let decisao = G4.interpret(&estado, None, G4.thresholds());
    assert_eq!(decisao.choice, None);
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    assert!(decisao.motivo.unwrap_or_default().contains("sem resposta"));

    let no_choice = JevResponse {
        choice: None,
        confidence: 0.9,
        ..JevResponse::default()
    };
    let decisao = G4.interpret(&estado, Some(&no_choice), G4.thresholds());
    assert_eq!(decisao.choice, None);
    assert!(!decisao.is_reliable());
}

#[test]
fn name_matching_is_strict_but_not_dumb() {
    let candidates_list = candidates();
    assert_eq!(
        match_candidate("Suporte", &candidates_list).as_deref(),
        Ok("Suporte")
    );
    assert_eq!(
        match_candidate("  Suporte  ", &candidates_list).as_deref(),
        Ok("Suporte")
    );
    assert_eq!(
        match_candidate("SUPORTE", &candidates_list).as_deref(),
        Ok("Suporte")
    );
    assert!(match_candidate("Jurídico", &candidates_list).is_err());
    assert!(match_candidate("", &candidates_list).is_err());
    let ambiguous = vec!["Financeiro".to_owned(), "FINANCEIRO".to_owned()];
    let error = match_candidate("financeiro", &ambiguous).unwrap_err();
    assert!(error.contains("ambíguo"), "{error}");
}

#[tokio::test]
async fn batch_g4_and_fallback_keep_raw_choice() {
    let jev = std::sync::Arc::new(FakeJev::new(|request: &JevRequest| {
        assert_eq!(
            request.questions.keys().collect::<Vec<_>>(),
            vec![IdGate::G4.question_id()]
        );
        let mut replies = BTreeMap::new();
        replies.insert(
            IdGate::G4.question_id().to_owned(),
            response("Marketing", 0.95),
        );
        Ok(BatchResponse {
            answers: replies,
            tokens: 7,
            ..BatchResponse::default()
        })
    }));
    let engine = JevEngine::new(jev, poph_types::JevGates::default(), "jev-latest");
    let resultado = engine
        .evaluate(
            estado(
                &["Comercial", "Suporte", "Financeiro"],
                "quero o preço do produto",
            ),
            Some(gate_override(IdGate::G4)),
        )
        .await;

    let decisao = resultado.decision(IdGate::G4).expect("o G4 rodou");
    assert_eq!(decisao.choice, None);
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    assert_eq!(
        resultado
            .response(IdGate::G4)
            .and_then(|raw| raw.choice.as_deref()),
        Some("Marketing")
    );
}

#[tokio::test]
async fn disabled_gate_decides_nothing() {
    let jev = std::sync::Arc::new(FakeJev::with_choices(BTreeMap::from([(
        IdGate::G4.question_id().to_owned(),
        "Suporte".to_owned(),
    )])));
    let engine = JevEngine::new(jev, poph_types::JevGates::default(), "jev-latest");
    let resultado = engine
        .evaluate(
            estado(&["Comercial", "Suporte"], "chegou quebrado"),
            Some(poph_jev::GateOverrides::default()),
        )
        .await;
    assert!(resultado.decision(IdGate::G4).is_none());
}
