//! G1: a instrução é **byte a byte** a de produção (HR-29a) e as criteria são as de produção mais os
//! **três acréscimos declarados** da HR-29b.
//!
//! A medição da HR-29m mostrou recall de 67–70% no G1, com as perdas concentradas em insatisfação,
//! urgência e risco à saúde. O diagnóstico foi que o Harness tinha três desvios em relação ao texto
//! que roda no POP Atende (`apps/backend/core/jev/human_handoff.py`) — e é o texto que o modelo lê
//! para decidir. A HR-29a alinhou tudo; a HR-29b acrescentou, de propósito e com medição, três
//! linhas que **não** existem em produção. Este teste trava as duas coisas: se alguém "melhorar" a
//! redação sem medir, ou se um dos acréscimos sumir, ele falha dizendo qual.
//!
//! O último teste confere os trechos de produção contra o próprio arquivo, quando a árvore do POP
//! Atende está ao lado (é o caso do monorepo). Fora do monorepo ele avisa e passa: os literais abaixo
//! continuam valendo como trava.

use std::path::Path;

use poph_jev::{G1, G1_CONTINUAR, G1_TRANSFERIR, Gate, JevMessage, JevState, Question};

/// `_INSTRUCTIONS` de `core/jev/human_handoff.py`.
const INSTRUCOES_PRODUCAO: &str = "Julgue se a ÚLTIMA mensagem do cliente, lida no contexto das mensagens anteriores e das \
    situacoes_de_handoff configuradas pela empresa, exige um atendente humano AGORA. Atenção: o \
    cliente mencionar 'atendente', 'pessoa' ou 'humano' pede transferência. O assistente ter \
    respondido bem NÃO é motivo para transferir. Insatisfação, reclamação, pedido repetido não \
    atendido, tema fora do escopo configurado e urgência pedem transferência quando as situações \
    da empresa cobrirem o caso. Sem situações configuradas, aplique a política padrão descrita \
    nas criteria.";

/// `_CRITERIA[TRANSFER_OPTION]` de `core/jev/human_handoff.py`.
const CRITERIO_TRANSFERIR_PRODUCAO: &str = "A conversa precisa de um atendente humano agora: o cliente pediu explicitamente uma pessoa; \
    ou a situação se encaixa em uma das `situacoes_de_handoff` da empresa; ou há \
    insatisfação/reclamação/urgência evidente, ou o pedido repetido do cliente continua sem \
    solução pelo assistente. Na dúvida entre 'o assistente ainda pode resolver' e 'precisa de \
    humano', prefira continuar.";

/// `_CRITERIA[CONTINUE_OPTION]` de `core/jev/human_handoff.py`.
const CRITERIO_CONTINUAR_PRODUCAO: &str = "O assistente pode e deve continuar atendendo: dúvida coberta pelo escopo, coleta de dados em \
    andamento, prosseguimento de fluxo, despedida, mensagens sociais. Menção casual à palavra \
    'humano' sem pedido de transferência também continua.";

/// Acréscimo (a) da HR-29b — risco à saúde, inclusive animal peçonhento **presente no local, mesmo sem
/// picada**, quando a situação estiver entre as `situacoes_de_handoff` da empresa (não é
/// transferência incondicional: o mantenedor decidiu em 20/11/2026 que o risco continua dependendo da
/// política configurada).
const ADICAO_RISCO: &str = "Também pede transferência, quando a situação estiver entre as `situacoes_de_handoff` da \
    empresa, o risco à saúde — inclusive animal peçonhento presente no local, mesmo sem picada \
    (escorpião, cobra, aranha, marimbondos) — e ingestão ou contato com produto. ";

/// Acréscimo (b) da HR-29b — erro de cobrança pede transferência.
const ADICAO_COBRANCA: &str = "Erro de cobrança, como cobrança em dobro ou cobrança indevida, também pede \
    transferência. ";

/// Acréscimo (c) da HR-29b — reclamação de preço **sem pedido de solução** continua com o assistente.
const ADICAO_PRECO: &str = " Reclamação de preço sem pedido de solução (achou caro, não gostou do preço) também \
    continua com o assistente.";

/// Onde a frase final da dúvida começa na criteria de produção de transferir.
const MARCA_DUVIDA: &str = "Na dúvida entre";

/// A criteria de transferir esperada: produção com os acréscimos (a) e (b) antes da frase da dúvida.
fn criterio_transferir_esperado() -> String {
    let corte = CRITERIO_TRANSFERIR_PRODUCAO
        .find(MARCA_DUVIDA)
        .expect("a criteria de produção de transferir mudou de redação");
    let (antes, duvida) = CRITERIO_TRANSFERIR_PRODUCAO.split_at(corte);
    format!("{antes}{ADICAO_RISCO}{ADICAO_COBRANCA}{duvida}")
}

/// A criteria de continuar esperada: produção mais o acréscimo (c).
fn criterio_continuar_esperado() -> String {
    format!("{CRITERIO_CONTINUAR_PRODUCAO}{ADICAO_PRECO}")
}

fn pergunta_do_g1() -> Question {
    let estado = JevState::new("whatsapp")
        .with_messages([JevMessage::from_customer("quero falar com um atendente")])
        .with_handoff(vec!["segunda via de nota fiscal".to_owned()], true);
    G1.question(&estado)
}

#[test]
fn instrucao_do_g1_e_byte_a_byte_a_de_producao() {
    let pergunta = pergunta_do_g1();
    assert_eq!(
        pergunta.instructions, INSTRUCOES_PRODUCAO,
        "a instrução do G1 divergiu da de produção"
    );
    // O gate é escolha binária: nenhuma opção a mais entra por engano.
    assert_eq!(pergunta.rubric.len(), 2);
}

/// A trava da HR-29b: as criteria são **produção + os três acréscimos declarados**, nada mais. Se um
/// deles sumir (ou for reescrito), o teste diz qual.
#[test]
fn criteria_do_g1_sao_as_de_producao_com_os_tres_acrescimos() {
    let pergunta = pergunta_do_g1();
    let transferir = pergunta
        .rubric
        .get(G1_TRANSFERIR)
        .cloned()
        .unwrap_or_default();
    let continuar = pergunta
        .rubric
        .get(G1_CONTINUAR)
        .cloned()
        .unwrap_or_default();

    // Cada acréscimo, conferido sozinho — a mensagem de falha diz qual saiu.
    assert!(
        transferir.contains(ADICAO_RISCO.trim()),
        "acréscimo (a) da HR-29b saiu da criteria de transferir: risco à saúde, inclusive animal \
         peçonhento presente no local mesmo sem picada"
    );
    assert!(
        transferir.contains(ADICAO_COBRANCA.trim()),
        "acréscimo (b) da HR-29b saiu da criteria de transferir: erro de cobrança pede transferência"
    );
    assert!(
        continuar.contains(ADICAO_PRECO.trim()),
        "acréscimo (c) da HR-29b saiu da criteria de continuar: reclamação de preço sem pedido de \
         solução continua com o assistente"
    );

    // E o conjunto exato: produção + os três acréscimos, sem sobra nem falta.
    assert_eq!(
        transferir,
        criterio_transferir_esperado(),
        "a criteria de {} não é a de produção mais os acréscimos (a) e (b)",
        G1_TRANSFERIR
    );
    assert_eq!(
        continuar,
        criterio_continuar_esperado(),
        "a criteria de {} não é a de produção mais o acréscimo (c)",
        G1_CONTINUAR
    );
}

/// Os três desvios que a HR-29a corrigiu — se algum voltar, este teste diz qual.
#[test]
fn os_tres_pontos_da_hr_29a_continuam_corrigidos() {
    let pergunta = pergunta_do_g1();
    let instrucao = pergunta.instructions;
    // A criteria como o modelo a recebe (produção + acréscimos da HR-29b): a frase da dúvida tem de
    // continuar no fim dela.
    let transferir = pergunta
        .rubric
        .get(G1_TRANSFERIR)
        .cloned()
        .unwrap_or_default();

    // (a) a política padrão volta a ser mandada para as criteria.
    assert!(
        instrucao.contains(
            "Sem situações configuradas, aplique a política padrão descrita nas criteria"
        ),
        "falta a frase da política padrão na instrução"
    );
    // (b) "na dúvida, prefira continuar" fica no fim da criteria de transferir, não na instrução.
    assert!(
        !instrucao.contains("prefira continuar"),
        "a instrução voltou a mandar preferir continuar no topo (pesa em todo caso)"
    );
    assert!(
        transferir.ends_with("prefira continuar."),
        "a criteria de transferir perdeu a frase da dúvida"
    );
    // (c) "pedido repetido do cliente continua sem solução pelo assistente", como em produção.
    assert!(
        transferir.contains("o pedido repetido do cliente continua sem solução pelo assistente")
    );
    assert!(!instrucao.contains("pedido repetido sem solução"));
}

/// Confere os trechos contra o arquivo de produção, quando ele existe (monorepo).
#[test]
fn trechos_batem_com_o_arquivo_de_producao() {
    let caminho =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../backend/core/jev/human_handoff.py");
    let Ok(fonte) = std::fs::read_to_string(&caminho) else {
        eprintln!(
            "aviso: {} não está nesta árvore — conferência contra produção pulada",
            caminho.display()
        );
        return;
    };
    // O arquivo quebra as frases em várias linhas: confiro por trecho que caiba numa linha só.
    let marcadores = [
        "Sem situações configuradas, aplique a política padrão",
        "Atenção: o cliente mencionar 'atendente', 'pessoa' ou",
        "o pedido repetido do cliente continua sem solução pelo",
        "Na dúvida entre 'o assistente ainda pode resolver' e",
        "Menção casual à palavra 'humano' sem pedido de transferência",
    ];
    for marcador in marcadores {
        assert!(
            fonte.contains(marcador),
            "produção não tem mais o trecho {marcador:?} — a instrução do Harness ficou órfã"
        );
    }
}
