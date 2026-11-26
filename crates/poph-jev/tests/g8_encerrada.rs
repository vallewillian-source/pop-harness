use std::collections::BTreeMap;
use std::sync::Arc;

use poph_jev::{
    DecisionOrigin, FakeJev, G8, G8_CONTINUES, G8_DEFAULT_FAREWELL, G8_ENDED, Gate, GateEvaluator,
    IdGate, JevEngine, JevMessage, JevResponse, JevState, Thresholds, farewell_from_agent,
};
use poph_types::JevGates;

fn estado(messages: Vec<JevMessage>) -> JevState {
    JevState::new("whatsapp").with_messages(messages)
}

fn with_agent() -> JevState {
    estado(vec![
        JevMessage::from_agent("Mais alguma dúvida?"),
        JevMessage::from_customer("era só isso, obrigado"),
    ])
}

fn response(choice: &str, confidence_value: f64) -> JevResponse {
    JevResponse {
        choice: Some(choice.to_owned()),
        confidence: confidence_value,
        ..JevResponse::default()
    }
}

#[test]
fn question_has_two_options_and_applies_only_with_agent_speech() {
    let gate = G8;
    assert_eq!(gate.id(), IdGate::G8);

    let question = gate.question(&with_agent());
    let options: Vec<&str> = question.rubric.keys().map(String::as_str).collect();
    assert_eq!(
        options,
        vec![G8_CONTINUES, G8_ENDED],
        "as duas opções, em ordem"
    );

    assert!(
        gate.applies(&with_agent()),
        "há fala do agente antes do cliente"
    );
    assert!(
        !gate.applies(&estado(vec![JevMessage::from_customer("obrigado")])),
        "primeira mensagem da conversa não fecha: não há o que encerrar"
    );
    assert!(
        !gate.applies(&estado(vec![JevMessage::from_agent("Olá!")])),
        "sem mensagem do cliente para julgar, não se aplica"
    );
}

#[test]
fn only_ended_with_confidence_at_threshold_is_decision() {
    let gate = G8;
    let thresholds = gate.thresholds();
    assert_eq!(
        thresholds,
        Thresholds::new(0.8, 0.0),
        "limiar mais alto: fechar por engano é o caro"
    );

    let above = gate.interpret(&with_agent(), Some(&response(G8_ENDED, 0.9)), thresholds);
    assert_eq!(above.origin, DecisionOrigin::Jev);
    assert_eq!(above.choice.as_deref(), Some(G8_ENDED));

    let at_threshold = gate.interpret(&with_agent(), Some(&response(G8_ENDED, 0.8)), thresholds);
    assert_eq!(
        at_threshold.choice.as_deref(),
        Some(G8_ENDED),
        "0,8 exato passa"
    );

    let below = gate.interpret(&with_agent(), Some(&response(G8_ENDED, 0.79)), thresholds);
    assert_eq!(below.origin, DecisionOrigin::Fallback);
    assert_eq!(
        below.choice.as_deref(),
        Some(G8_CONTINUES),
        "abaixo do limiar: conversa segue"
    );
    assert!(
        below
            .motivo
            .unwrap_or_default()
            .contains("abaixo do limiar")
    );
}

#[test]
fn continues_confident_decision_has_no_ending_effect() {
    let decisao = G8.interpret(
        &with_agent(),
        Some(&response(G8_CONTINUES, 0.95)),
        G8.thresholds(),
    );
    assert_eq!(decisao.origin, DecisionOrigin::Jev);
    assert_eq!(decisao.choice.as_deref(), Some(G8_CONTINUES));
}

#[test]
fn outside_contract_and_missing_become_continues() {
    let thresholds = G8.thresholds();
    let outside = G8.interpret(&with_agent(), Some(&response("talvez", 1.0)), thresholds);
    assert_eq!(outside.origin, DecisionOrigin::Fallback);
    assert_eq!(outside.choice.as_deref(), Some(G8_CONTINUES));

    let missing = G8.interpret(&with_agent(), None, thresholds);
    assert_eq!(missing.origin, DecisionOrigin::Fallback);
    assert_eq!(missing.choice.as_deref(), Some(G8_CONTINUES));
    assert_eq!(missing.motivo.as_deref(), Some("sem resposta do Jev"));
}

#[test]
fn engine_asks_g8_only_with_flag_on() {
    let jev = Arc::new(FakeJev::with_choices(BTreeMap::from([(
        "g8_encerrada".to_owned(),
        G8_ENDED.to_owned(),
    )])));

    let disabled = JevEngine::new(jev.clone(), JevGates::default(), "jev-latest");
    let _ = futures_block(disabled.evaluate(with_agent(), None));
    assert_eq!(jev.calls(), 0, "com o G8 desligado, nenhuma pergunta");

    let enabled = JevEngine::new(
        jev.clone(),
        JevGates {
            g8: true,
            ..JevGates::default()
        },
        "jev-latest",
    );
    let resultado = futures_block(enabled.evaluate(with_agent(), None));
    assert_eq!(jev.calls(), 1);
    let request = jev.requests().pop().expect("um pedido");
    assert!(request.questions.contains_key("g8_encerrada"));
    assert_eq!(resultado.choice(IdGate::G8), Some(G8_ENDED));
}

#[test]
fn agent_farewell_respects_missing_empty_invalid_and_long() {
    assert_eq!(farewell_from_agent("{}"), None, "sem campo: padrão do gate");
    assert_eq!(farewell_from_agent(""), None);
    assert_eq!(
        farewell_from_agent("não é json"),
        None,
        "JSON inválido não derruba o turno"
    );
    assert_eq!(
        farewell_from_agent(r#"{"despedida": 42}"#),
        None,
        "campo com outro tipo é ignorado"
    );
    assert_eq!(
        farewell_from_agent(r#"{"despedida": ""}"#),
        Some(String::new()),
        "vazio = encerrar sem mensagem"
    );
    assert_eq!(
        farewell_from_agent(r#"{"despedida": "  Até mais!  "}"#),
        Some("Até mais!".to_owned())
    );
    let long_text = "x".repeat(900);
    let parsed =
        farewell_from_agent(&format!(r#"{{"despedida": "{long_text}"}}"#)).unwrap_or_default();
    assert_eq!(parsed.chars().count(), poph_jev::flags::MAX_FAREWELL_CHARS);
    assert!(!G8_DEFAULT_FAREWELL.is_empty());
}

fn futures_block<T>(future: futures::future::BoxFuture<'_, T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime de teste")
        .block_on(future)
}
