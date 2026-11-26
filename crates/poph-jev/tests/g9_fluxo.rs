use std::collections::BTreeMap;
use std::sync::Arc;

use poph_jev::{
    DecisionOrigin, FakeJev, G1_CONTINUE, G3_BASE, G9, G9_NONE, Gate, GateEvaluator, GateOverrides,
    IdGate, JevEngine, JevFlow, JevMessage, JevResponse, JevState, MAX_RELEASED_FLOWS,
    MAX_TRIGGER_CHARS, Thresholds, flow_policy, read_dataset,
};
use poph_types::JevGates;

fn fluxo(id: &str, nome: &str) -> JevFlow {
    JevFlow {
        id: id.to_owned(),
        nome: nome.to_owned(),
        when: format!("O cliente quer {nome}."),
        not_when: "Só está perguntando sobre outra coisa.".to_owned(),
        examples: vec![format!("queria {nome}"), format!("dá para {nome}?")],
    }
}

fn fluxos() -> Vec<JevFlow> {
    vec![
        fluxo("agendar_visita", "marcar uma visita técnica"),
        fluxo("segunda_via_fatura", "receber a segunda via da fatura"),
        fluxo("abrir_chamado", "abrir um chamado de suporte"),
    ]
}

fn estado() -> JevState {
    JevState::new("whatsapp")
        .with_messages([
            JevMessage::from_agent("Boa tarde! Como posso ajudar?"),
            JevMessage::from_customer("queria marcar uma visita técnica na minha casa"),
        ])
        .with_released_flows(fluxos())
}

fn response(choice: &str, confidence_value: f64) -> JevResponse {
    let mut scores = BTreeMap::new();
    scores.insert(choice.to_owned(), 0.9);
    JevResponse {
        choice: Some(choice.to_owned()),
        probabilities: scores,
        confidence: confidence_value,
        ..JevResponse::default()
    }
}

fn gates(g1: bool, g3: bool, g9: bool) -> JevGates {
    JevGates {
        g1,
        g3,
        g9,
        ..JevGates::default()
    }
}

#[test]
fn question_has_flow_ids_and_none_with_trigger_rubric() {
    let gate = G9;
    assert_eq!(gate.id(), IdGate::G9);
    assert_eq!(gate.id().question_id(), "g9_iniciar_fluxo");

    let question = gate.question(&estado());
    let options: Vec<&str> = question.rubric.keys().map(String::as_str).collect();
    assert_eq!(
        options,
        vec![
            "abrir_chamado",
            "agendar_visita",
            "nenhum",
            "segunda_via_fatura"
        ],
        "os ids dos fluxos + nenhum, em ordem estável"
    );

    let criterion = &question.rubric["agendar_visita"];
    assert!(criterion.contains("Quando usar:"), "{criterion}");
    assert!(criterion.contains("Não usar quando:"), "{criterion}");
    assert!(
        criterion.contains("Exemplos de fala do cliente:"),
        "{criterion}"
    );
    assert!(
        criterion.contains("\"queria marcar uma visita técnica\""),
        "{criterion}"
    );
    assert!(
        question.instructions.contains("ÚLTIMA mensagem do cliente"),
        "a instrução diz o que julgar"
    );
}

#[test]
fn each_trigger_piece_is_cut_at_400_chars() {
    let long_text = "a".repeat(1000);
    let long_flow = JevFlow {
        id: "agendar_visita".to_owned(),
        nome: "Agendar visita".to_owned(),
        when: long_text.clone(),
        not_when: long_text.clone(),
        examples: vec![long_text.clone()],
    };
    let estado = JevState::new("cli")
        .with_messages([JevMessage::from_customer("queria agendar")])
        .with_released_flows([long_flow]);

    let question = G9.question(&estado);
    let criterion = &question.rubric["agendar_visita"];
    for piece in criterion.split('\n') {
        let raw = piece.split_once(": ").map(|(_, part)| part).unwrap_or("");
        let value = raw.trim_matches('"');
        assert!(
            value.chars().count() <= MAX_TRIGGER_CHARS + 1,
            "pedaço sem corte: {} caracteres",
            value.chars().count()
        );
    }
    assert_eq!(criterion.matches('…').count(), 3, "{criterion}");

    let stored = &estado.released_flows[0];
    assert_eq!(stored.when.chars().count(), MAX_TRIGGER_CHARS + 1);
    assert_eq!(stored.examples[0].chars().count(), MAX_TRIGGER_CHARS + 1);
}

#[test]
fn criterion_falls_back_to_name_when_trigger_is_empty() {
    let without_trigger = JevFlow {
        id: "abrir_chamado".to_owned(),
        nome: "Abrir chamado".to_owned(),
        when: String::new(),
        not_when: "  ".to_owned(),
        examples: Vec::new(),
    };
    let criterion = flow_policy(&without_trigger);
    assert_eq!(
        criterion,
        "O cliente quer o que o fluxo \"Abrir chamado\" faz."
    );
}

#[test]
fn applies_only_with_released_flow_and_within_cap() {
    let gate = G9;

    assert!(gate.applies(&estado()), "três fluxos liberados: pergunta");
    assert_eq!(gate.warning(&estado()), None, "nada a avisar");

    let no_flow = JevState::new("whatsapp")
        .with_messages([JevMessage::from_customer("queria marcar uma visita")]);
    assert!(
        !gate.applies(&no_flow),
        "sem fluxo liberado a pergunta não existe"
    );
    assert_eq!(gate.warning(&no_flow), None, "ausência de fluxo é normal");

    let no_customer_message = JevState::new("whatsapp")
        .with_messages([JevMessage::from_agent("Olá!")])
        .with_released_flows(fluxos());
    assert!(
        !gate.applies(&no_customer_message),
        "sem fala do cliente não há o que escolher"
    );

    let at_cap: Vec<JevFlow> = (0..MAX_RELEASED_FLOWS)
        .map(|index| fluxo(&format!("fluxo_{index:02}"), "atender o cliente"))
        .collect();
    let state_at_cap = JevState::new("cli")
        .with_messages([JevMessage::from_customer("quero atendimento")])
        .with_released_flows(at_cap);
    assert!(gate.applies(&state_at_cap));
    assert_eq!(
        gate.question(&state_at_cap).rubric.len(),
        MAX_RELEASED_FLOWS + 1,
        "vinte fluxos + nenhum"
    );
}

#[tokio::test]
async fn more_than_twenty_flows_makes_gate_inapplicable_with_warning() {
    let gate = G9;
    let extra: Vec<JevFlow> = (0..MAX_RELEASED_FLOWS + 1)
        .map(|index| fluxo(&format!("fluxo_{index:02}"), "atender o cliente"))
        .collect();
    let estado = JevState::new("cli")
        .with_messages([JevMessage::from_customer("quero atendimento")])
        .with_released_flows(extra);

    assert!(
        !gate.applies(&estado),
        "lista cortada faria o modelo escolher entre as que sobraram"
    );
    let warning = gate.warning(&estado).expect("avisa o operador");
    assert!(warning.contains("21"), "{warning}");
    assert!(
        warning.contains(&MAX_RELEASED_FLOWS.to_string()),
        "{warning}"
    );

    let jev = Arc::new(FakeJev::with_choices(BTreeMap::new()));
    let engine = JevEngine::new(jev.clone(), gates(false, false, true), "jev-latest");
    let resultado = engine.evaluate(estado, None).await;
    assert_eq!(jev.calls(), 0, "acima do teto: nenhuma chamada");
    assert!(resultado.decision(IdGate::G9).is_none());
}

#[test]
fn reliable_choice_becomes_decision() {
    let gate = G9;
    let thresholds = gate.thresholds();
    assert_eq!(
        thresholds,
        Thresholds::new(0.7, 0.1),
        "confiança 0,7 e margem 0,1 no padrão do PRD"
    );

    let decisao = gate.interpret(
        &estado(),
        Some(&response("agendar_visita", 0.92)),
        thresholds,
    );
    assert_eq!(decisao.origin, DecisionOrigin::Jev);
    assert_eq!(decisao.choice.as_deref(), Some("agendar_visita"));
    assert!(decisao.is_reliable());
    assert_eq!(decisao.motivo, None);
}

#[test]
fn none_low_confidence_short_margin_and_invented_option_fall_back_with_reason() {
    let gate = G9;
    let thresholds = gate.thresholds();

    let none_decision = gate.interpret(&estado(), Some(&response(G9_NONE, 0.95)), thresholds);
    assert_eq!(none_decision.origin, DecisionOrigin::Fallback);
    assert_eq!(none_decision.choice, None, "nada inicia");
    assert!(
        none_decision
            .motivo
            .as_deref()
            .unwrap_or("")
            .contains("nenhum"),
        "{:?}",
        none_decision.motivo
    );

    let low = gate.interpret(
        &estado(),
        Some(&response("agendar_visita", 0.4)),
        thresholds,
    );
    assert_eq!(low.origin, DecisionOrigin::Fallback);
    assert_eq!(low.choice, None);
    assert!(
        low.motivo
            .as_deref()
            .unwrap_or("")
            .contains("confiança 0.40"),
        "{:?}",
        low.motivo
    );

    let mut tied = JevResponse {
        choice: Some("agendar_visita".to_owned()),
        confidence: 0.7,
        ..JevResponse::default()
    };
    tied.probabilities.insert("agendar_visita".to_owned(), 0.7);
    tied.probabilities.insert("abrir_chamado".to_owned(), 0.68);
    let margin = gate.interpret(&estado(), Some(&tied), thresholds);
    assert_eq!(margin.origin, DecisionOrigin::Fallback);
    assert!(
        margin.motivo.as_deref().unwrap_or("").contains("margem"),
        "{:?}",
        margin.motivo
    );

    let invented = gate.interpret(
        &estado(),
        Some(&response("cancelar_tudo", 0.99)),
        thresholds,
    );
    assert_eq!(invented.origin, DecisionOrigin::Fallback);
    assert_eq!(invented.choice, None);
    assert!(
        invented
            .motivo
            .as_deref()
            .unwrap_or("")
            .contains("fora das opções"),
        "{:?}",
        invented.motivo
    );

    let missing = gate.interpret(&estado(), None, thresholds);
    assert_eq!(missing.origin, DecisionOrigin::Fallback);
    assert_eq!(
        missing.motivo.as_deref(),
        Some("sem resposta do Jev"),
        "motivo do fallback preenchido"
    );
}

#[tokio::test]
async fn three_gates_with_flow_make_one_request_only() {
    let jev = Arc::new(FakeJev::with_choices(BTreeMap::from([
        ("g1_transferir".to_owned(), G1_CONTINUE.to_owned()),
        ("g3_precisa".to_owned(), G3_BASE.to_owned()),
        ("g9_iniciar_fluxo".to_owned(), "agendar_visita".to_owned()),
    ])));
    let engine = JevEngine::new(jev.clone(), gates(true, true, true), "jev-latest");

    let estado = JevState::new("whatsapp")
        .with_messages([
            JevMessage::from_customer("oi, boa tarde"),
            JevMessage::from_agent("Boa tarde! Como posso ajudar?"),
            JevMessage::from_customer("queria marcar uma visita técnica"),
        ])
        .with_handoff(vec!["cliente pede segunda via".to_owned()], true)
        .with_capabilities(true, true)
        .with_released_flows(fluxos());

    let resultado = engine.evaluate(estado, None).await;

    assert_eq!(jev.calls(), 1, "o lote é um pedido só");
    let request = &jev.requests()[0];
    assert_eq!(request.questions.len(), 3);
    assert!(request.questions.contains_key("g9_iniciar_fluxo"));
    assert_eq!(
        request.estado["fluxos"]["liberados"][0]["id"],
        "agendar_visita"
    );
    assert!(
        request.questions["g9_iniciar_fluxo"].rubric["segunda_via_fatura"].contains("Quando usar:")
    );
    assert_eq!(
        resultado.reliable_choice(IdGate::G9),
        Some("agendar_visita")
    );
}

#[tokio::test]
async fn gate_without_released_flow_is_not_asked() {
    let jev = Arc::new(FakeJev::with_choices(BTreeMap::from([(
        "g9_iniciar_fluxo".to_owned(),
        "agendar_visita".to_owned(),
    )])));
    let engine = JevEngine::new(jev.clone(), gates(false, false, true), "jev-latest");

    let estado = JevState::new("whatsapp")
        .with_messages([JevMessage::from_customer("queria marcar uma visita")]);
    let resultado = engine.evaluate(estado, None).await;

    assert_eq!(jev.calls(), 0, "sem fluxo liberado não custa chamada");
    assert!(resultado.decision(IdGate::G9).is_none());
}

#[tokio::test]
async fn agent_flag_and_global_govern_the_question() {
    let jev = Arc::new(FakeJev::with_choices(BTreeMap::from([(
        "g9_iniciar_fluxo".to_owned(),
        "agendar_visita".to_owned(),
    )])));

    let engine = JevEngine::new(jev.clone(), gates(false, false, true), "jev-latest");
    let disabled = GateOverrides::from_json(r#"{"gates": {"g9": false}}"#).expect("ajuste válido");
    assert!(!engine.effective_gates(Some(disabled)).g9);
    let resultado = engine.evaluate(estado(), Some(disabled)).await;
    assert_eq!(jev.calls(), 0, "flag do agente desliga o G9");
    assert!(resultado.decision(IdGate::G9).is_none());

    let engine = JevEngine::new(jev.clone(), gates(false, false, false), "jev-latest");
    let enabled = GateOverrides::from_json(r#"{"g9": true}"#).expect("ajuste válido");
    assert!(engine.effective_gates(Some(enabled)).g9);
    let resultado = engine.evaluate(estado(), Some(enabled)).await;
    assert_eq!(jev.calls(), 1);
    assert_eq!(
        resultado.reliable_choice(IdGate::G9),
        Some("agendar_visita")
    );

    let only = GateOverrides::only(IdGate::G9);
    assert_eq!(only.g9, Some(true));
    assert_eq!(only.g8, Some(false));
}

#[test]
fn state_carries_flows_only_when_present_and_changes_hash() {
    let no_flow = JevState::new("cli").with_messages([JevMessage::from_customer("oi")]);
    assert!(
        no_flow.to_json().get("fluxos").is_none(),
        "sem fluxo o estado dos outros gates não muda de forma"
    );

    let with_flow = no_flow.clone().with_released_flows(fluxos());
    let json = with_flow.to_json();
    assert_eq!(json["fluxos"]["liberados"].as_array().unwrap().len(), 3);
    assert_eq!(json["fluxos"]["liberados"][1]["id"], "segunda_via_fatura");
    assert_ne!(no_flow.hash(), with_flow.hash(), "o cache separa os dois");
}

#[test]
fn flow_examples_are_capped_in_state() {
    let many = JevFlow {
        id: "agendar_visita".to_owned(),
        nome: "Agendar visita".to_owned(),
        when: "O cliente quer marcar.".to_owned(),
        not_when: String::new(),
        examples: (0..20).map(|index| format!("exemplo {index}")).collect(),
    };
    let estado = JevState::new("cli")
        .with_messages([JevMessage::from_customer("queria agendar")])
        .with_released_flows([many]);
    assert_eq!(estado.released_flows[0].examples.len(), 5);
}

#[test]
fn g9_labeled_set_serves_measurement() {
    let dataset_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/jev/g9.jsonl");
    let cases = read_dataset(&dataset_path).expect("lê tests/jev/g9.jsonl");

    assert!(
        cases.len() >= 40,
        "o conjunto precisa de pelo menos 40 casos, tem {}",
        cases.len()
    );

    for (index, case) in cases.iter().enumerate() {
        let case_number = index + 1;
        assert!(
            G9.applies(&case.estado),
            "caso {case_number}: o G9 precisa se aplicar para a medição valer"
        );
        let ids: Vec<&str> = case
            .estado
            .released_flows
            .iter()
            .map(|fluxo| fluxo.id.as_str())
            .collect();
        assert!(
            case.expected == G9_NONE || ids.contains(&case.expected.as_str()),
            "caso {case_number}: esperado `{}` não é um fluxo do caso nem `nenhum`",
            case.expected
        );
        assert!(
            case.category.is_some() && case.rationale.is_some(),
            "caso {case_number}: falta categoria ou justificativa"
        );
    }

    let positives = cases.iter().filter(|case| case.expected != G9_NONE).count();
    let negatives = cases.len() - positives;
    assert!(
        positives >= 15 && negatives >= 15,
        "o conjunto precisa de positivos e negativos: {positives} × {negatives}"
    );
}
