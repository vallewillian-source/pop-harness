use std::path::Path;

use poph_jev::{G1, G1_CONTINUE, G1_TRANSFER, Gate, JevMessage, JevState, Question};

const PRODUCTION_INSTRUCTIONS: &str = "Julgue se a ÚLTIMA mensagem do cliente, lida no contexto das mensagens anteriores e das \
    situacoes_de_handoff configuradas pela empresa, exige um atendente humano AGORA. Atenção: o \
    cliente mencionar 'atendente', 'pessoa' ou 'humano' pede transferência. O assistente ter \
    respondido bem NÃO é motivo para transferir. Insatisfação, reclamação, pedido repetido não \
    atendido, tema fora do escopo configurado e urgência pedem transferência quando as situações \
    da empresa cobrirem o caso. Sem situações configuradas, aplique a política padrão descrita \
    nas criteria.";

const PRODUCTION_TRANSFER_RUBRIC: &str = "A conversa precisa de um atendente humano agora: o cliente pediu explicitamente uma pessoa; \
    ou a situação se encaixa em uma das `situacoes_de_handoff` da empresa; ou há \
    insatisfação/reclamação/urgência evidente, ou o pedido repetido do cliente continua sem \
    solução pelo assistente. Na dúvida entre 'o assistente ainda pode resolver' e 'precisa de \
    humano', prefira continuar.";

const PRODUCTION_CONTINUE_RUBRIC: &str = "O assistente pode e deve continuar atendendo: dúvida coberta pelo escopo, coleta de dados em \
    andamento, prosseguimento de fluxo, despedida, mensagens sociais. Menção casual à palavra \
    'humano' sem pedido de transferência também continua.";

const HEALTH_RISK_ADDITION: &str = "Também pede transferência, quando a situação estiver entre as `situacoes_de_handoff` da \
    empresa, o risco à saúde — inclusive animal peçonhento presente no local, mesmo sem picada \
    (escorpião, cobra, aranha, marimbondos) — e ingestão ou contato com produto. ";

const BILLING_ADDITION: &str = "Erro de cobrança, como cobrança em dobro ou cobrança indevida, também pede \
    transferência. ";

const PRICE_ADDITION: &str = " Reclamação de preço sem pedido de solução (achou caro, não gostou do preço) também \
    continua com o assistente.";

const DOUBT_MARKER: &str = "Na dúvida entre";

fn expected_transfer_rubric() -> String {
    let cut = PRODUCTION_TRANSFER_RUBRIC
        .find(DOUBT_MARKER)
        .expect("a criteria de produção de transferir mudou de redação");
    let (before, doubt) = PRODUCTION_TRANSFER_RUBRIC.split_at(cut);
    format!("{before}{HEALTH_RISK_ADDITION}{BILLING_ADDITION}{doubt}")
}

fn expected_continue_rubric() -> String {
    format!("{PRODUCTION_CONTINUE_RUBRIC}{PRICE_ADDITION}")
}

fn g1_question() -> Question {
    let state = JevState::new("whatsapp")
        .with_messages([JevMessage::from_customer("quero falar com um atendente")])
        .with_handoff(vec!["segunda via de nota fiscal".to_owned()], true);
    G1.question(&state)
}

#[test]
fn g1_instruction_is_byte_for_byte_production() {
    let prompt = g1_question();
    assert_eq!(
        prompt.instructions, PRODUCTION_INSTRUCTIONS,
        "a instrução do G1 divergiu da de produção"
    );
    assert_eq!(prompt.rubric.len(), 2);
}

#[test]
fn g1_rubrics_are_production_plus_three_additions() {
    let prompt = g1_question();
    let transfer_rubric = prompt.rubric.get(G1_TRANSFER).cloned().unwrap_or_default();
    let continue_rubric = prompt.rubric.get(G1_CONTINUE).cloned().unwrap_or_default();

    assert!(
        transfer_rubric.contains(HEALTH_RISK_ADDITION.trim()),
        "acréscimo (a) da HR-29b saiu da criteria de transferir: risco à saúde, inclusive animal \
         peçonhento presente no local mesmo sem picada"
    );
    assert!(
        transfer_rubric.contains(BILLING_ADDITION.trim()),
        "acréscimo (b) da HR-29b saiu da criteria de transferir: erro de cobrança pede transferência"
    );
    assert!(
        continue_rubric.contains(PRICE_ADDITION.trim()),
        "acréscimo (c) da HR-29b saiu da criteria de continuar: reclamação de preço sem pedido de \
         solução continua com o assistente"
    );

    assert_eq!(
        transfer_rubric,
        expected_transfer_rubric(),
        "a criteria de {} não é a de produção mais os acréscimos (a) e (b)",
        G1_TRANSFER
    );
    assert_eq!(
        continue_rubric,
        expected_continue_rubric(),
        "a criteria de {} não é a de produção mais o acréscimo (c)",
        G1_CONTINUE
    );
}

#[test]
fn hr_29a_three_points_stay_fixed() {
    let prompt = g1_question();
    let instruction = prompt.instructions;
    let transfer_rubric = prompt.rubric.get(G1_TRANSFER).cloned().unwrap_or_default();

    assert!(
        instruction.contains(
            "Sem situações configuradas, aplique a política padrão descrita nas criteria"
        ),
        "falta a frase da política padrão na instrução"
    );
    assert!(
        !instruction.contains("prefira continuar"),
        "a instrução voltou a mandar preferir continuar no topo (pesa em todo caso)"
    );
    assert!(
        transfer_rubric.ends_with("prefira continuar."),
        "a criteria de transferir perdeu a frase da dúvida"
    );
    assert!(
        transfer_rubric
            .contains("o pedido repetido do cliente continua sem solução pelo assistente")
    );
    assert!(!instruction.contains("pedido repetido sem solução"));
}

#[test]
fn excerpts_match_production_file() {
    let production_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../backend/core/jev/human_handoff.py");
    let Ok(source) = std::fs::read_to_string(&production_path) else {
        eprintln!(
            "aviso: {} não está nesta árvore — conferência contra produção pulada",
            production_path.display()
        );
        return;
    };
    let markers = [
        "Sem situações configuradas, aplique a política padrão",
        "Atenção: o cliente mencionar 'atendente', 'pessoa' ou",
        "o pedido repetido do cliente continua sem solução pelo",
        "Na dúvida entre 'o assistente ainda pode resolver' e",
        "Menção casual à palavra 'humano' sem pedido de transferência",
    ];
    for marker in markers {
        assert!(
            source.contains(marker),
            "produção não tem mais o trecho {marker:?} — a instrução do Harness ficou órfã"
        );
    }
}
