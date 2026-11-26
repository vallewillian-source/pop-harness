use poph_jev::GateOverrides;
use poph_types::JevGates;

fn effective_gates(text: &str) -> JevGates {
    GateOverrides::from_json(text)
        .expect("o ajuste é lido")
        .with_base(JevGates::default_value())
}

#[test]
fn agent_without_override_inherits_default() {
    assert!(GateOverrides::from_json("{}").is_none());
    assert!(JevGates::default_value().g3);
    assert!(JevGates::default_value().g5);
    assert!(JevGates::default_value().g8);
}

#[test]
fn agent_can_disable_g3_or_g5_without_touching_the_other() {
    let g3_off = effective_gates(r#"{"g3": false}"#);
    assert!(!g3_off.g3, "g3 false no agente desliga o G3");
    assert!(g3_off.g5, "o G5 continua no padrão");

    let g5_off = effective_gates(r#"{"gates": {"g5": false}}"#);
    assert!(!g5_off.g5, "g5 false no formato aninhado desliga o G5");
    assert!(g5_off.g3, "o G3 continua no padrão");
}

#[test]
fn agent_can_disable_g8_without_touching_g3_or_g5() {
    let g8_off = effective_gates(r#"{"g8": false}"#);
    assert!(!g8_off.g8, "g8 false no agente desliga o G8");
    assert!(g8_off.g3 && g8_off.g5, "G3 e G5 seguem no padrão");
    let g8_off_nested = effective_gates(r#"{"gates": {"g8": false}}"#);
    assert!(
        !g8_off_nested.g8,
        "g8 false no formato aninhado desliga o G8"
    );
}

#[test]
fn active_false_disables_all_agent_gates() {
    assert_eq!(effective_gates(r#"{"ativo": false}"#), JevGates::default());
}

#[test]
fn other_gate_override_keeps_defaults() {
    let only_g1 = effective_gates(r#"{"g1": true}"#);
    assert!(only_g1.g1);
    assert!(
        only_g1.g3 && only_g1.g5 && only_g1.g8,
        "G3, G5 e G8 seguem no padrão"
    );
    assert!(!only_g1.g2 && !only_g1.g4 && !only_g1.g6 && !only_g1.g7);
}

#[test]
fn agent_can_disable_g9_without_touching_the_others() {
    let g9_off = effective_gates(r#"{"g9": false}"#);
    assert!(!g9_off.g9, "g9 false no agente desliga o G9");
    assert!(
        g9_off.g3 && g9_off.g5 && g9_off.g8,
        "G3, G5 e G8 seguem no padrão"
    );
    let g9_off_nested = effective_gates(r#"{"gates": {"g9": false}}"#);
    assert!(
        !g9_off_nested.g9,
        "g9 false no formato aninhado desliga o G9"
    );
    assert!(
        effective_gates(r#"{"g1": true}"#).g9,
        "um ajuste de outro gate não desliga o G9 do padrão"
    );
}
