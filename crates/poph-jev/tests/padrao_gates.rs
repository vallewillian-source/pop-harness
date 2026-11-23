//! Padrão de instalação dos gates (decisões do mantenedor, 20/11/2026): G3, G5 e G8 ligados, os outros
//! cinco desligados — e o G9, que a configuração `[fluxos]` ligou em seguida (FX-26), sem pergunta
//! quando o agente não tem fluxo liberado. O agente sobrepõe esse padrão pelo `jev_json`, campo a campo.

use poph_jev::AjusteGates;
use poph_types::GatesJev;

fn efetivos(texto: &str) -> GatesJev {
    AjusteGates::de_json(texto)
        .expect("o ajuste é lido")
        .sobre(GatesJev::padrao())
}

#[test]
fn agente_sem_ajuste_herda_o_padrao() {
    // Sem nada declarado, o agente não tem ajuste: vale a configuração global (padrão).
    assert!(AjusteGates::de_json("{}").is_none());
    assert!(GatesJev::padrao().g3);
    assert!(GatesJev::padrao().g5);
    assert!(GatesJev::padrao().g8);
}

#[test]
fn agente_pode_desligar_g3_ou_g5_sem_mexer_no_outro() {
    let sem_g3 = efetivos(r#"{"g3": false}"#);
    assert!(!sem_g3.g3, "g3 false no agente desliga o G3");
    assert!(sem_g3.g5, "o G5 continua no padrão");

    let sem_g5 = efetivos(r#"{"gates": {"g5": false}}"#);
    assert!(!sem_g5.g5, "g5 false no formato aninhado desliga o G5");
    assert!(sem_g5.g3, "o G3 continua no padrão");
}

#[test]
fn agente_pode_desligar_g8_sem_mexer_em_g3_e_g5() {
    let sem_g8 = efetivos(r#"{"g8": false}"#);
    assert!(!sem_g8.g8, "g8 false no agente desliga o G8");
    assert!(sem_g8.g3 && sem_g8.g5, "G3 e G5 seguem no padrão");
    let sem_g8_aninhado = efetivos(r#"{"gates": {"g8": false}}"#);
    assert!(
        !sem_g8_aninhado.g8,
        "g8 false no formato aninhado desliga o G8"
    );
}

#[test]
fn ativo_false_desliga_todos_os_gates_do_agente() {
    assert_eq!(efetivos(r#"{"ativo": false}"#), GatesJev::default());
}

#[test]
fn ajuste_de_outro_gate_nao_liga_nem_desliga_os_padrao() {
    let so_g1 = efetivos(r#"{"g1": true}"#);
    assert!(so_g1.g1);
    assert!(
        so_g1.g3 && so_g1.g5 && so_g1.g8,
        "G3, G5 e G8 seguem no padrão"
    );
    assert!(!so_g1.g2 && !so_g1.g4 && !so_g1.g6 && !so_g1.g7);
}

/// O G9 (quem inicia um fluxo) é ajustável por agente como os outros gates (PRD de fluxos §9).
#[test]
fn agente_pode_desligar_g9_sem_mexer_nos_outros() {
    let sem_g9 = efetivos(r#"{"g9": false}"#);
    assert!(!sem_g9.g9, "g9 false no agente desliga o G9");
    assert!(
        sem_g9.g3 && sem_g9.g5 && sem_g9.g8,
        "G3, G5 e G8 seguem no padrão"
    );
    let sem_g9_aninhado = efetivos(r#"{"gates": {"g9": false}}"#);
    assert!(
        !sem_g9_aninhado.g9,
        "g9 false no formato aninhado desliga o G9"
    );
    assert!(
        efetivos(r#"{"g1": true}"#).g9,
        "um ajuste de outro gate não desliga o G9 do padrão"
    );
}
