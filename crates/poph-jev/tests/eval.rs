use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use poph_jev::{
    CasoRotulado, ErroConjunto, FakeJev, G1_CONTINUE, G1_TRANSFER, IdGate, JevEngine, JevMessage,
    JevState, avaliar_conjunto, ler_conjunto,
};
use poph_types::JevGates;

fn globals(g1: bool) -> JevGates {
    JevGates {
        g1,
        ..JevGates::default()
    }
}

fn labeled_case(texto: &str, expected: &str) -> CasoRotulado {
    CasoRotulado {
        estado: JevState::new("whatsapp")
            .with_messages([JevMessage::from_customer(texto)])
            .with_handoff(Vec::new(), true),
        esperado: expected.to_owned(),
        categoria: None,
        justificativa: None,
    }
}

fn test_jev() -> Arc<FakeJev> {
    Arc::new(FakeJev::new(|request| {
        let texto = request.estado["conversa"]["mensagens"][0]["texto"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let (choice, certainty) = if texto.contains("atendente") {
            (G1_TRANSFER, 0.95)
        } else if texto.contains("talvez") {
            (G1_CONTINUE, 0.2)
        } else {
            (G1_CONTINUE, 0.9)
        };
        let mut answer_map = BTreeMap::new();
        for id in request.questions.keys() {
            answer_map.insert(
                id.clone(),
                poph_jev::JevResponse {
                    choice: Some(choice.to_owned()),
                    confidence: certainty,
                    probabilities: BTreeMap::from([(choice.to_owned(), certainty)]),
                    ..poph_jev::JevResponse::default()
                },
            );
        }
        Ok(poph_jev::BatchResponse {
            answers: answer_map,
            tokens: 7,
            ..poph_jev::BatchResponse::default()
        })
    }))
}

#[tokio::test]
async fn measures_hits_abstentions_and_latency() {
    let jev = test_jev();
    let engine = Arc::new(JevEngine::new(jev, globals(true), "jev-latest"));
    let cases = vec![
        labeled_case("quero falar com um atendente", G1_TRANSFER),
        labeled_case("qual o prazo de entrega?", G1_CONTINUE),
        labeled_case("isso sim, talvez eu precise", G1_CONTINUE),
    ];

    let report = avaliar_conjunto(engine.as_ref(), IdGate::G1, &cases).await;

    assert_eq!(report.gate, "g1");
    assert_eq!(report.casos, 3);
    assert_eq!(report.acertos, 2, "o terceiro é abstenção, não acerto");
    assert_eq!(report.erros, 0);
    assert_eq!(report.abstencoes, 1);
    assert!((report.taxa_acerto() - 2.0 / 3.0).abs() < 1e-9);
    assert!((report.taxa_abstencao() - 1.0 / 3.0).abs() < 1e-9);
    assert_eq!(report.latencias_ms.len(), 3);
    assert_eq!(report.detalhes[2].resultado, "abstencao");
    assert!(!report.texto("conjunto.jsonl").is_empty());
}

#[tokio::test]
async fn gate_that_does_not_apply_counts_as_not_applicable() {
    let jev = test_jev();
    let engine = Arc::new(JevEngine::new(
        Arc::clone(&jev) as Arc<dyn poph_jev::JevTransport>,
        globals(false),
        "jev-latest",
    ));
    let mut case = labeled_case("quero falar com um atendente", G1_TRANSFER);
    case.estado = case.estado.with_handoff(Vec::new(), false);
    let cases = vec![case];

    let report = avaliar_conjunto(engine.as_ref(), IdGate::G1, &cases).await;

    assert_eq!(report.abstencoes, 1);
    assert_eq!(report.acertos, 0);
    assert_eq!(report.detalhes[0].resultado, "nao_aplicavel");
}

#[tokio::test]
async fn call_failure_counts_as_abstention() {
    let jev = Arc::new(FakeJev::failing(poph_jev::JevError::Status {
        code: 500,
        motivo: "Internal Server Error".to_owned(),
    }));
    let engine = Arc::new(JevEngine::new(jev, globals(true), "jev-latest"));
    let cases = vec![labeled_case("quero falar com um atendente", G1_TRANSFER)];

    let report = avaliar_conjunto(engine.as_ref(), IdGate::G1, &cases).await;

    assert_eq!(report.falhas_de_chamada, 1);
    assert_eq!(report.abstencoes, 1);
    assert_eq!(report.acertos, 0);
}

#[tokio::test]
async fn engine_cache_shows_as_zero_latency() {
    let jev = test_jev();
    let engine = Arc::new(
        JevEngine::new(
            Arc::clone(&jev) as Arc<dyn poph_jev::JevTransport>,
            globals(true),
            "jev-latest",
        )
        .with_options(poph_jev::JevOptions {
            batch_timeout: Duration::from_secs(2),
            ..poph_jev::JevOptions::default()
        }),
    );
    let cases = vec![
        labeled_case("qual o prazo?", G1_CONTINUE),
        labeled_case("qual o prazo?", G1_CONTINUE),
    ];

    let report = avaliar_conjunto(engine.as_ref(), IdGate::G1, &cases).await;

    assert_eq!(report.acertos, 2);
    assert_eq!(jev.calls(), 1, "o segundo caso veio do cache");
}

#[tokio::test]
async fn abstention_shows_the_jev_choice_and_reason() {
    let jev = Arc::new(FakeJev::new(|request: &poph_jev::JevRequest| {
        let mut answer_map = BTreeMap::new();
        for id in request.questions.keys() {
            answer_map.insert(
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
            answers: answer_map,
            tokens: 4,
            ..poph_jev::BatchResponse::default()
        })
    }));
    let engine = Arc::new(JevEngine::new(jev, globals(true), "jev-latest"));
    let cases = vec![labeled_case("quero falar com um atendente", G1_TRANSFER)];

    let report = avaliar_conjunto(engine.as_ref(), IdGate::G1, &cases).await;

    assert_eq!(report.abstencoes, 1);
    assert_eq!(report.erros, 0, "abstenção não é erro");
    let detail = &report.detalhes[0];
    assert_eq!(
        detail.obtido, None,
        "no fallback o turno não recebe a escolha"
    );
    assert_eq!(
        detail.escolha.as_deref(),
        Some(G1_TRANSFER),
        "a escolha do modelo aparece mesmo no fallback"
    );
    assert_eq!(detail.confianca, "0.55");
    assert!(
        detail
            .motivo
            .as_deref()
            .unwrap_or_default()
            .contains("abaixo do limiar"),
        "motivo: {:?}",
        detail.motivo
    );

    let texto = report.texto("conjunto.jsonl");
    assert!(texto.contains("detalhe por caso"), "saída: {texto}");
    assert!(texto.contains("jev transferir_agora"), "saída: {texto}");
    assert!(texto.contains("motivo:"), "saída: {texto}");

    let value = serde_json::to_value(&report).expect("serializa");
    assert_eq!(value["detalhes"][0]["escolha"], G1_TRANSFER);
    assert_eq!(value["detalhes"][0]["obtido"], serde_json::Value::Null);
    assert!(
        value["detalhes"][0]["motivo"]
            .as_str()
            .unwrap_or_default()
            .contains("abaixo do limiar")
    );
}

#[tokio::test]
async fn call_usage_appears_in_the_report() {
    let jev = Arc::new(FakeJev::new(|request: &poph_jev::JevRequest| {
        let mut answer_map = BTreeMap::new();
        for id in request.questions.keys() {
            answer_map.insert(
                id.clone(),
                poph_jev::JevResponse {
                    choice: Some(G1_CONTINUE.to_owned()),
                    confidence: 0.95,
                    ..poph_jev::JevResponse::default()
                },
            );
        }
        Ok(poph_jev::BatchResponse {
            answers: answer_map,
            tokens: 34,
            usage: poph_jev::UsageTokens {
                input: Some(30),
                output: Some(4),
                total: Some(34),
            },
        })
    }));
    let engine = Arc::new(JevEngine::new(jev, globals(true), "jev-latest"));
    let cases = vec![
        labeled_case("qual o prazo?", G1_CONTINUE),
        labeled_case("e o frete?", G1_CONTINUE),
    ];

    let report = avaliar_conjunto(engine.as_ref(), IdGate::G1, &cases).await;

    assert_eq!(report.usage.input, Some(60), "duas chamadas de 30");
    assert_eq!(report.usage.output, Some(8));
    assert_eq!(report.usage.total, Some(68));
    let texto = report.texto("conjunto.jsonl");
    assert!(
        texto.contains("tokens: entrada 60 · saída 8 · total 68"),
        "saída: {texto}"
    );
    let value = serde_json::to_value(&report).expect("serializa");
    assert_eq!(value["usage"]["entrada"], 60);
    assert_eq!(value["usage"]["saida"], 8);
    assert_eq!(value["usage"]["total"], 68);
    assert_eq!(value["usage"]["informado"], true);
}

#[tokio::test]
async fn without_usage_the_report_says_not_reported() {
    let jev = test_jev();
    let engine = Arc::new(JevEngine::new(jev, globals(true), "jev-latest"));
    let cases = vec![labeled_case("qual o prazo?", G1_CONTINUE)];

    let report = avaliar_conjunto(engine.as_ref(), IdGate::G1, &cases).await;

    assert!(!report.usage.reported());
    let texto = report.texto("conjunto.jsonl");
    assert!(texto.contains("tokens: não informado"), "saída: {texto}");
    let value = serde_json::to_value(&report).expect("serializa");
    assert_eq!(value["usage"]["informado"], false);
    assert_eq!(value["usage"]["entrada"], serde_json::Value::Null);
}

#[test]
fn reads_set_with_comments_and_blank_lines() {
    let folder = std::env::temp_dir().join(format!("poph-jev-eval-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("pasta");
    let path = folder.join("g1.jsonl");
    let content = "\n# comentário\n{\"estado\": {\"canal\": \"cli\", \"mensagens\": [{\"papel\": \"cliente\", \"texto\": \"oi\"}]}, \"esperado\": \"continuar_agente\"}\n\n";
    std::fs::write(&path, content).expect("grava");

    let cases = ler_conjunto(&path).expect("lê o conjunto");
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].esperado, "continuar_agente");
    assert_eq!(cases[0].estado.messages.len(), 1);

    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn invalid_set_reports_the_line() {
    let folder = std::env::temp_dir().join(format!("poph-jev-eval-erro-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("pasta");
    let path = folder.join("ruim.jsonl");
    std::fs::write(
        &path,
        "{\"estado\": {}, \"esperado\": \"x\"}\n{ isto não é json }\n",
    )
    .expect("grava");

    let error = ler_conjunto(&path).expect_err("recusa a segunda linha");
    match error {
        ErroConjunto::Caso { linha: line, .. } => assert_eq!(line, 2),
        other => panic!("erro inesperado: {other:?}"),
    }

    let vazio = folder.join("vazio.jsonl");
    std::fs::write(&vazio, "# só comentário\n").expect("grava");
    assert!(matches!(
        ler_conjunto(&vazio).expect_err("conjunto vazio"),
        ErroConjunto::Vazio { .. }
    ));

    let _ = std::fs::remove_dir_all(&folder);
}

#[tokio::test]
async fn category_and_justification_appear_in_the_report() {
    let path =
        std::env::temp_dir().join(format!("poph-jev-categoria-{}.jsonl", std::process::id()));
    std::fs::write(
        &path,
        concat!(
            r#"{"estado": {"canal": "whatsapp", "mensagens": [{"papel": "cliente", "texto": "quero falar com um atendente"}], "transferencia_habilitada": true}, "esperado": "transferir_agora", "categoria": "pedido_explicito", "justificativa": "o cliente pede explicitamente uma pessoa"}"#,
            "\n",
            r#"{"estado": {"canal": "whatsapp", "mensagens": [{"papel": "cliente", "texto": "qual o prazo?"}], "transferencia_habilitada": true}, "esperado": "continuar_agente"}"#,
            "\n",
        ),
    )
    .unwrap();

    let cases = ler_conjunto(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(cases.len(), 2);
    assert_eq!(cases[0].categoria.as_deref(), Some("pedido_explicito"));
    assert_eq!(
        cases[0].justificativa.as_deref(),
        Some("o cliente pede explicitamente uma pessoa")
    );
    assert_eq!(cases[1].categoria, None);

    let engine = Arc::new(JevEngine::new(test_jev(), globals(true), "jev-latest"));
    let report = avaliar_conjunto(engine.as_ref(), IdGate::G1, &cases).await;
    assert_eq!(report.acertos, 2);
    assert_eq!(
        report.detalhes[0].categoria.as_deref(),
        Some("pedido_explicito")
    );
    let summary = report.resumo_por_categoria().expect("há categoria");
    assert!(
        summary.contains("pedido_explicito: 1 casos · 1 acerto"),
        "{summary}"
    );
    assert!(
        report
            .texto("conjunto.jsonl")
            .contains("· pedido_explicito")
    );
}
