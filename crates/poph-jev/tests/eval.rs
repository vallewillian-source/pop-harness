//! Testes do conjunto rotulado e da medição (`poph jev eval` usa este mesmo caminho).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use poph_jev::{
    CasoRotulado, ErroConjunto, FakeJev, G1_CONTINUE, G1_TRANSFER, IdGate, JevEngine, JevMessage,
    JevState, avaliar_conjunto, ler_conjunto,
};
use poph_types::JevGates;

fn globais(g1: bool) -> JevGates {
    JevGates {
        g1,
        ..JevGates::default()
    }
}

fn caso(texto: &str, esperado: &str) -> CasoRotulado {
    CasoRotulado {
        estado: JevState::new("whatsapp")
            .with_messages([JevMessage::from_customer(texto)])
            .with_handoff(Vec::new(), true),
        esperado: esperado.to_owned(),
        categoria: None,
        justificativa: None,
    }
}

/// Jev falso que acerta "pedido de humano", erra o resto e se abstém quando a resposta é fraca.
fn jev_de_teste() -> Arc<FakeJev> {
    Arc::new(FakeJev::new(|pedido| {
        let texto = pedido.estado["conversa"]["mensagens"][0]["texto"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let (escolha, confianca) = if texto.contains("atendente") {
            (G1_TRANSFER, 0.95)
        } else if texto.contains("talvez") {
            (G1_CONTINUE, 0.2)
        } else {
            (G1_CONTINUE, 0.9)
        };
        let mut respostas = BTreeMap::new();
        for id in pedido.questions.keys() {
            respostas.insert(
                id.clone(),
                poph_jev::JevResponse {
                    choice: Some(escolha.to_owned()),
                    confidence: confianca,
                    probabilities: BTreeMap::from([(escolha.to_owned(), confianca)]),
                    ..poph_jev::JevResponse::default()
                },
            );
        }
        Ok(poph_jev::BatchResponse {
            answers: respostas,
            tokens: 7,
            ..poph_jev::BatchResponse::default()
        })
    }))
}

#[tokio::test]
async fn mede_acerto_abstencao_e_latencia() {
    let jev = jev_de_teste();
    let motor = Arc::new(JevEngine::new(jev, globais(true), "jev-latest"));
    let casos = vec![
        caso("quero falar com um atendente", G1_TRANSFER),
        caso("qual o prazo de entrega?", G1_CONTINUE),
        caso("isso sim, talvez eu precise", G1_CONTINUE),
    ];

    let relatorio = avaliar_conjunto(motor.as_ref(), IdGate::G1, &casos).await;

    assert_eq!(relatorio.gate, "g1");
    assert_eq!(relatorio.casos, 3);
    assert_eq!(relatorio.acertos, 2, "o terceiro é abstenção, não acerto");
    assert_eq!(relatorio.erros, 0);
    assert_eq!(relatorio.abstencoes, 1);
    assert!((relatorio.taxa_acerto() - 2.0 / 3.0).abs() < 1e-9);
    assert!((relatorio.taxa_abstencao() - 1.0 / 3.0).abs() < 1e-9);
    assert_eq!(relatorio.latencias_ms.len(), 3);
    assert_eq!(relatorio.detalhes[2].resultado, "abstencao");
    assert!(!relatorio.texto("conjunto.jsonl").is_empty());
}

/// Gate que não se aplica ao estado (agente sem transferência): nada é perguntado e o caso conta
/// como não aplicável, não como erro.
#[tokio::test]
async fn gate_que_nao_se_aplica_conta_como_nao_aplicavel() {
    let jev = jev_de_teste();
    let motor = Arc::new(JevEngine::new(
        Arc::clone(&jev) as Arc<dyn poph_jev::JevTransport>,
        globais(false),
        "jev-latest",
    ));
    let mut caso = caso("quero falar com um atendente", G1_TRANSFER);
    caso.estado = caso.estado.with_handoff(Vec::new(), false);
    let casos = vec![caso];

    let relatorio = avaliar_conjunto(motor.as_ref(), IdGate::G1, &casos).await;

    assert_eq!(relatorio.abstencoes, 1);
    assert_eq!(relatorio.acertos, 0);
    assert_eq!(relatorio.detalhes[0].resultado, "nao_aplicavel");
}

/// Falha do Jev conta como abstenção e aparece no relatório.
#[tokio::test]
async fn falha_da_chamada_conta_como_abstencao() {
    let jev = Arc::new(FakeJev::failing(poph_jev::JevError::Status {
        code: 500,
        motivo: "Internal Server Error".to_owned(),
    }));
    let motor = Arc::new(JevEngine::new(jev, globais(true), "jev-latest"));
    let casos = vec![caso("quero falar com um atendente", G1_TRANSFER)];

    let relatorio = avaliar_conjunto(motor.as_ref(), IdGate::G1, &casos).await;

    assert_eq!(relatorio.falhas_de_chamada, 1);
    assert_eq!(relatorio.abstencoes, 1);
    assert_eq!(relatorio.acertos, 0);
}

#[tokio::test]
async fn cache_do_motor_aparece_como_zero_de_latencia() {
    let jev = jev_de_teste();
    let motor = Arc::new(
        JevEngine::new(
            Arc::clone(&jev) as Arc<dyn poph_jev::JevTransport>,
            globais(true),
            "jev-latest",
        )
        .with_options(poph_jev::JevOptions {
            batch_timeout: Duration::from_secs(2),
            ..poph_jev::JevOptions::default()
        }),
    );
    let casos = vec![
        caso("qual o prazo?", G1_CONTINUE),
        caso("qual o prazo?", G1_CONTINUE),
    ];

    let relatorio = avaliar_conjunto(motor.as_ref(), IdGate::G1, &casos).await;

    assert_eq!(relatorio.acertos, 2);
    assert_eq!(jev.calls(), 1, "o segundo caso veio do cache");
}

/// Abstenção mostra a escolha que o modelo devolveu e o motivo do fallback (aceite da calibração).
#[tokio::test]
async fn abstencao_mostra_a_escolha_do_jev_e_o_motivo() {
    // O Jev responde "transferir_agora" com confiança 0,55: abaixo do limiar 0,70 do G1.
    let jev = Arc::new(FakeJev::new(|pedido: &poph_jev::JevRequest| {
        let mut respostas = BTreeMap::new();
        for id in pedido.questions.keys() {
            respostas.insert(
                id.clone(),
                poph_jev::JevResponse {
                    choice: Some(G1_TRANSFER.to_owned()),
                    confidence: 0.55,
                    probabilities: BTreeMap::from([
                        (G1_TRANSFER.to_owned(), 0.55),
                        (G1_CONTINUE.to_owned(), 0.45),
                    ]),
                    ..poph_jev::JevResponse::default()
                },
            );
        }
        Ok(poph_jev::BatchResponse {
            answers: respostas,
            tokens: 4,
            ..poph_jev::BatchResponse::default()
        })
    }));
    let motor = Arc::new(JevEngine::new(jev, globais(true), "jev-latest"));
    let casos = vec![caso("quero falar com um atendente", G1_TRANSFER)];

    let relatorio = avaliar_conjunto(motor.as_ref(), IdGate::G1, &casos).await;

    assert_eq!(relatorio.abstencoes, 1);
    assert_eq!(relatorio.erros, 0, "abstenção não é erro");
    let detalhe = &relatorio.detalhes[0];
    assert_eq!(
        detalhe.obtido, None,
        "no fallback o turno não recebe a escolha"
    );
    assert_eq!(
        detalhe.escolha.as_deref(),
        Some(G1_TRANSFER),
        "a escolha do modelo aparece mesmo no fallback"
    );
    assert_eq!(detalhe.confianca, "0.55");
    assert!(
        detalhe
            .motivo
            .as_deref()
            .unwrap_or_default()
            .contains("abaixo do limiar"),
        "motivo: {:?}",
        detalhe.motivo
    );

    let texto = relatorio.texto("conjunto.jsonl");
    assert!(texto.contains("detalhe por caso"), "saída: {texto}");
    assert!(texto.contains("jev transferir_agora"), "saída: {texto}");
    assert!(texto.contains("motivo:"), "saída: {texto}");

    // E o JSON do relatório leva os mesmos campos.
    let valor = serde_json::to_value(&relatorio).expect("serializa");
    assert_eq!(valor["detalhes"][0]["escolha"], G1_TRANSFER);
    assert_eq!(valor["detalhes"][0]["obtido"], serde_json::Value::Null);
    assert!(
        valor["detalhes"][0]["motivo"]
            .as_str()
            .unwrap_or_default()
            .contains("abaixo do limiar")
    );
}

/// `usage` do lote aparece no relatório, somado por execução, quando o Jev informa.
#[tokio::test]
async fn usage_da_chamada_aparece_no_relatorio() {
    let jev = Arc::new(FakeJev::new(|pedido: &poph_jev::JevRequest| {
        let mut respostas = BTreeMap::new();
        for id in pedido.questions.keys() {
            respostas.insert(
                id.clone(),
                poph_jev::JevResponse {
                    choice: Some(G1_CONTINUE.to_owned()),
                    confidence: 0.95,
                    ..poph_jev::JevResponse::default()
                },
            );
        }
        Ok(poph_jev::BatchResponse {
            answers: respostas,
            tokens: 34,
            usage: poph_jev::UsageTokens {
                input: Some(30),
                output: Some(4),
                total: Some(34),
            },
        })
    }));
    let motor = Arc::new(JevEngine::new(jev, globais(true), "jev-latest"));
    let casos = vec![
        caso("qual o prazo?", G1_CONTINUE),
        caso("e o frete?", G1_CONTINUE),
    ];

    let relatorio = avaliar_conjunto(motor.as_ref(), IdGate::G1, &casos).await;

    assert_eq!(relatorio.usage.input, Some(60), "duas chamadas de 30");
    assert_eq!(relatorio.usage.output, Some(8));
    assert_eq!(relatorio.usage.total, Some(68));
    let texto = relatorio.texto("conjunto.jsonl");
    assert!(
        texto.contains("tokens: entrada 60 · saída 8 · total 68"),
        "saída: {texto}"
    );
    let valor = serde_json::to_value(&relatorio).expect("serializa");
    assert_eq!(valor["usage"]["entrada"], 60);
    assert_eq!(valor["usage"]["saida"], 8);
    assert_eq!(valor["usage"]["total"], 68);
    assert_eq!(valor["usage"]["informado"], true);
}

/// Sem `usage` no corpo, o relatório diz "não informado" em vez de inventar número.
#[tokio::test]
async fn sem_usage_o_relatorio_diz_nao_informado() {
    let jev = jev_de_teste();
    let motor = Arc::new(JevEngine::new(jev, globais(true), "jev-latest"));
    let casos = vec![caso("qual o prazo?", G1_CONTINUE)];

    let relatorio = avaliar_conjunto(motor.as_ref(), IdGate::G1, &casos).await;

    assert!(!relatorio.usage.reported());
    let texto = relatorio.texto("conjunto.jsonl");
    assert!(texto.contains("tokens: não informado"), "saída: {texto}");
    let valor = serde_json::to_value(&relatorio).expect("serializa");
    assert_eq!(valor["usage"]["informado"], false);
    assert_eq!(valor["usage"]["entrada"], serde_json::Value::Null);
}

// ───────────────────────────── Leitura do conjunto ─────────────────────────────

#[test]
fn le_conjunto_com_comentarios_e_linhas_em_branco() {
    let pasta = std::env::temp_dir().join(format!("poph-jev-eval-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pasta);
    std::fs::create_dir_all(&pasta).expect("pasta");
    let caminho = pasta.join("g1.jsonl");
    let conteudo = "\n# comentário\n{\"estado\": {\"canal\": \"cli\", \"mensagens\": [{\"papel\": \"cliente\", \"texto\": \"oi\"}]}, \"esperado\": \"continuar_agente\"}\n\n";
    std::fs::write(&caminho, conteudo).expect("grava");

    let casos = ler_conjunto(&caminho).expect("lê o conjunto");
    assert_eq!(casos.len(), 1);
    assert_eq!(casos[0].esperado, "continuar_agente");
    assert_eq!(casos[0].estado.messages.len(), 1);

    let _ = std::fs::remove_dir_all(&pasta);
}

#[test]
fn conjunto_invalido_diz_a_linha() {
    let pasta = std::env::temp_dir().join(format!("poph-jev-eval-erro-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pasta);
    std::fs::create_dir_all(&pasta).expect("pasta");
    let caminho = pasta.join("ruim.jsonl");
    std::fs::write(
        &caminho,
        "{\"estado\": {}, \"esperado\": \"x\"}\n{ isto não é json }\n",
    )
    .expect("grava");

    let erro = ler_conjunto(&caminho).expect_err("recusa a segunda linha");
    match erro {
        ErroConjunto::Caso { linha, .. } => assert_eq!(linha, 2),
        outro => panic!("erro inesperado: {outro:?}"),
    }

    let vazio = pasta.join("vazio.jsonl");
    std::fs::write(&vazio, "# só comentário\n").expect("grava");
    assert!(matches!(
        ler_conjunto(&vazio).expect_err("conjunto vazio"),
        ErroConjunto::Vazio { .. }
    ));

    let _ = std::fs::remove_dir_all(&pasta);
}

/// Categoria e justificativa do conjunto: carregam do arquivo, saem no detalhe e no resumo.
#[tokio::test]
async fn categoria_e_justificativa_do_conjunto_aparecem_no_relatorio() {
    let caminho =
        std::env::temp_dir().join(format!("poph-jev-categoria-{}.jsonl", std::process::id()));
    std::fs::write(
        &caminho,
        concat!(
            r#"{"estado": {"canal": "whatsapp", "mensagens": [{"papel": "cliente", "texto": "quero falar com um atendente"}], "transferencia_habilitada": true}, "esperado": "transferir_agora", "categoria": "pedido_explicito", "justificativa": "o cliente pede explicitamente uma pessoa"}"#,
            "\n",
            r#"{"estado": {"canal": "whatsapp", "mensagens": [{"papel": "cliente", "texto": "qual o prazo?"}], "transferencia_habilitada": true}, "esperado": "continuar_agente"}"#,
            "\n",
        ),
    )
    .unwrap();

    let casos = ler_conjunto(&caminho).unwrap();
    std::fs::remove_file(&caminho).ok();
    assert_eq!(casos.len(), 2);
    assert_eq!(casos[0].categoria.as_deref(), Some("pedido_explicito"));
    assert_eq!(
        casos[0].justificativa.as_deref(),
        Some("o cliente pede explicitamente uma pessoa")
    );
    // Conjunto antigo, sem categoria, continua valendo.
    assert_eq!(casos[1].categoria, None);

    let motor = Arc::new(JevEngine::new(jev_de_teste(), globais(true), "jev-latest"));
    let relatorio = avaliar_conjunto(motor.as_ref(), IdGate::G1, &casos).await;
    assert_eq!(relatorio.acertos, 2);
    assert_eq!(
        relatorio.detalhes[0].categoria.as_deref(),
        Some("pedido_explicito")
    );
    let resumo = relatorio.resumo_por_categoria().expect("há categoria");
    assert!(
        resumo.contains("pedido_explicito: 1 casos · 1 acerto"),
        "{resumo}"
    );
    // O detalhe por caso mostra a categoria junto do resultado.
    assert!(
        relatorio
            .texto("conjunto.jsonl")
            .contains("· pedido_explicito")
    );
}
