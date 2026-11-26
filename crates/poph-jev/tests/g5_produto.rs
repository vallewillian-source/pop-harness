use std::collections::BTreeMap;
use std::sync::Arc;

use poph_jev::{
    DecisionOrigin, FakeJev, G5, G5_NONE, Gate, GateEvaluator, IdGate, JevEngine, JevMessage,
    JevResponse, JevState, ProductCandidate, eval::gate_override,
};
use poph_types::JevGates;

fn candidate(id: &str, nome: &str, detail: &str) -> ProductCandidate {
    ProductCandidate {
        id: id.to_owned(),
        nome: nome.to_owned(),
        detail: detail.to_owned(),
    }
}

fn candidates() -> Vec<ProductCandidate> {
    vec![
        candidate(
            "p-baratas",
            "Dedetização de baratas",
            "categoria: Pragas; preço: R$ 180,00",
        ),
        candidate(
            "p-cupins",
            "Descupinização de cupins",
            "categoria: Pragas; preço: R$ 350,00",
        ),
        candidate(
            "p-ratos",
            "Controle de ratos",
            "categoria: Pragas; preço: R$ 220,00",
        ),
    ]
}

fn estado(texto: &str, candidate_list: Vec<ProductCandidate>) -> JevState {
    JevState::new("whatsapp")
        .with_messages(vec![JevMessage::from_customer(texto)])
        .with_capabilities(true, true)
        .with_product_candidates(candidate_list)
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
fn question_has_one_option_per_candidate_and_none() {
    let question = G5.question(&estado("quanto custa para matar barata?", candidates()));
    assert_eq!(question.tipo, poph_jev::QuestionType::Choice);
    assert!(!question.instructions.is_empty());
    let options: Vec<&String> = question.rubric.keys().collect();
    assert_eq!(options, vec!["nenhum", "p-baratas", "p-cupins", "p-ratos"]);
    let criterion = &question.rubric["p-baratas"];
    assert!(criterion.contains("Dedetização de baratas"), "{criterion}");
    assert!(criterion.contains("R$ 180,00"), "{criterion}");
}

#[test]
fn gate_does_not_apply_without_candidate() {
    assert!(!G5.applies(&estado("quanto custa?", Vec::new())));
    assert!(G5.applies(&estado("quanto custa?", candidates())));
}

#[test]
fn candidate_id_becomes_reliable_decision() {
    let estado = estado("quanto custa para matar barata?", candidates());
    let decisao = G5.interpret(&estado, Some(&response("p-baratas", 0.92)), G5.thresholds());
    assert_eq!(decisao.choice.as_deref(), Some("p-baratas"));
    assert_eq!(decisao.origin, DecisionOrigin::Jev);
    assert_eq!(decisao.motivo, None);
}

#[test]
fn id_outside_candidates_is_refused() {
    let estado = estado("quanto custa para matar barata?", candidates());
    let decisao = G5.interpret(
        &estado,
        Some(&response("p-produto-que-nao-existe", 1.0)),
        G5.thresholds(),
    );
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    assert_eq!(decisao.choice, None);
    let motivo = decisao.motivo.unwrap_or_default();
    assert!(motivo.contains("fora das opções"), "{motivo}");
}

#[test]
fn none_is_abstention_with_reason() {
    let estado = estado("qual o prazo de entrega?", candidates());
    let decisao = G5.interpret(&estado, Some(&response(G5_NONE, 0.95)), G5.thresholds());
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    assert_eq!(decisao.choice, None);
    let motivo = decisao.motivo.unwrap_or_default();
    assert!(motivo.contains("não reconheceu o produto"), "{motivo}");
}

#[test]
fn low_confidence_falls_back() {
    let estado = estado("quanto custa para matar barata?", candidates());
    let decisao = G5.interpret(&estado, Some(&response("p-baratas", 0.55)), G5.thresholds());
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    assert_eq!(decisao.choice, None);
    let motivo = decisao.motivo.unwrap_or_default();
    assert!(motivo.contains("abaixo do limiar"), "{motivo}");
}

#[test]
fn no_response_falls_back() {
    let estado = estado("quanto custa?", candidates());
    let decisao = G5.interpret(&estado, None, G5.thresholds());
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    assert_eq!(decisao.motivo.as_deref(), Some("sem resposta do Jev"));
}

#[tokio::test]
async fn g5_is_asked_outside_batch_only_with_candidates() {
    let mut choices: BTreeMap<String, String> = BTreeMap::new();
    choices.insert("g5_produto".to_owned(), "p-baratas".to_owned());
    let jev = Arc::new(FakeJev::with_choices(choices));
    let globals = JevGates {
        g5: true,
        ..JevGates::default()
    };
    let transport: Arc<dyn poph_jev::JevTransport> = jev.clone();
    let evaluator = JevEngine::new(transport, globals, "jev-latest");

    let no_candidates = JevState::new("whatsapp")
        .with_messages(vec![JevMessage::from_customer("oi")])
        .with_capabilities(true, true);
    let batch = evaluator.evaluate(no_candidates, None).await;
    assert!(batch.decisions.is_empty(), "{:?}", batch.decisions);
    assert!(jev.requests().is_empty(), "nenhum pedido ao Jev");

    let candidate_state = estado("quero dedetizar contra baratas", candidates());
    let resultado = evaluator
        .evaluate(candidate_state, Some(gate_override(IdGate::G5)))
        .await;
    let requests = jev.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]
            .questions
            .keys()
            .cloned()
            .collect::<Vec<String>>(),
        vec!["g5_produto".to_owned()]
    );
    assert_eq!(resultado.reliable_choice(IdGate::G5), Some("p-baratas"));
}

#[test]
fn invented_id_from_transport_falls_back() {
    let estado = estado("quero dedetizar", candidates());
    let decisao = G5.interpret(
        &estado,
        Some(&response("p-de-outra-org", 0.99)),
        G5.thresholds(),
    );
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    assert!(decisao.choice.is_none());
}
