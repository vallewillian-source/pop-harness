#[path = "apoio/mod.rs"]
mod support;

use std::time::Duration;

use futures::StreamExt;
use poph_core::llm::base64::encode;
use poph_core::llm::{
    FinishReason, LlmError, LlmEvent, LlmProvider, Message, Part, Request, Tool, ToolCall, Usage,
};
use poph_types::OrgId;
use serde_json::json;
use support::*;

const HELLO_CHUNK: &str = r#"{"choices":[{"index":0,"delta":{"role":"assistant","content":"Olá"},"finish_reason":null}]}"#;
const CONTINUATION_CHUNK: &str =
    r#"{"choices":[{"index":0,"delta":{"content":", tudo bem?"},"finish_reason":null}]}"#;
const END_WITH_USAGE: &str = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":110,"completion_tokens":7,"total_tokens":117}}"#;

#[tokio::test]
async fn text_arrives_in_stream_with_usage_and_end() {
    let servidor = Servidor::iniciar(vec![Resposta::ok(sse(&[
        HELLO_CHUNK,
        CONTINUATION_CHUNK,
        END_WITH_USAGE,
        "[DONE]",
    ]))])
    .await;
    let m = montar(&servidor.url_base, 2, Duration::from_secs(5));

    let fluxo = m.cliente.stream(pergunta("oi")).await.expect("abre");
    let events = so_eventos(colher(fluxo).await);

    assert_eq!(
        events,
        vec![
            LlmEvent::Texto("Olá".into()),
            LlmEvent::Texto(", tudo bem?".into()),
            LlmEvent::Usage(Usage {
                input: 110,
                saida: 7
            }),
            LlmEvent::Fim(FinishReason::Completed),
        ]
    );
    let body = &servidor.pedidos()[0];
    assert_eq!(body["stream"], json!(true));
    assert_eq!(body["model"], "gemini-3.8-flash-tiered");
    let messages = body["messages"].as_array().expect("lista");
    assert_eq!(messages.last().expect("última")["role"], "user");
    assert_eq!(m.metricas.tokens_total.value(), 117);
    assert_eq!(m.metricas.llm_calls.value(), 1);
}

#[tokio::test]
async fn tool_call_round_trip_sends_full_arguments_and_gets_result() {
    let tool_call_stream = sse(&[
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"buscar_produto","arguments":"{\"termo\":"}}]}}]}"#,
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"dedetização\"}"}}]}}]}"#,
        r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
        "[DONE]",
    ]);
    let answer_stream = sse(&[
        r#"{"choices":[{"index":0,"delta":{"content":"O preço é R$ 350."},"finish_reason":"stop"}]}"#,
        "[DONE]",
    ]);
    let servidor = Servidor::iniciar(vec![
        Resposta::ok(tool_call_stream),
        Resposta::ok(answer_stream),
    ])
    .await;
    let m = montar(&servidor.url_base, 2, Duration::from_secs(5));
    let org = OrgId::generate();

    let mut first = Request::new(
        org,
        vec![Message::User(vec![Part::Texto("quanto custa?".into())])],
    );
    first.tools = vec![Tool {
        nome: "buscar_produto".into(),
        descricao: "Busca um produto e devolve o preço".into(),
        parameters: json!({"type": "object", "properties": {"termo": {"type": "string"}}}),
    }];
    let fluxo = m.cliente.stream(first).await.expect("abre");
    let events = so_eventos(colher(fluxo).await);
    assert_eq!(
        events,
        vec![
            LlmEvent::Tool(ToolCall {
                id: "call_1".into(),
                nome: "buscar_produto".into(),
                arguments: json!({"termo": "dedetização"}),
            }),
            LlmEvent::Fim(FinishReason::ToolCalls),
        ]
    );
    let first_body = &servidor.pedidos()[0];
    assert_eq!(first_body["tools"][0]["function"]["name"], "buscar_produto");

    let second = pedido_com(vec![
        Message::User(vec![Part::Texto("quanto custa?".into())]),
        Message::Assistant {
            texto: String::new(),
            calls: vec![ToolCall {
                id: "call_1".into(),
                nome: "buscar_produto".into(),
                arguments: json!({"termo": "dedetização"}),
            }],
        },
        Message::Resultado {
            id: "call_1".into(),
            content: "R$ 350,00".into(),
        },
    ]);
    let fluxo = m.cliente.stream(second).await.expect("abre");
    let events = so_eventos(colher(fluxo).await);
    assert_eq!(
        events,
        vec![
            LlmEvent::Texto("O preço é R$ 350.".into()),
            LlmEvent::Fim(FinishReason::Completed),
        ]
    );
    let second_body = &servidor.pedidos()[1];
    let messages = second_body["messages"].as_array().expect("lista");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[1]["tool_calls"][0]["id"], "call_1");
    assert_eq!(
        messages[1]["tool_calls"][0]["function"]["arguments"],
        "{\"termo\":\"dedetização\"}"
    );
    assert_eq!(messages[2]["role"], "tool");
    assert_eq!(messages[2]["tool_call_id"], "call_1");
    assert_eq!(messages[2]["content"], "R$ 350,00");
}

#[tokio::test]
async fn image_goes_as_data_url_with_text() {
    let servidor = Servidor::iniciar(vec![Resposta::ok(sse(&[
        r#"{"choices":[{"index":0,"delta":{"content":"ARARA AZUL 42"},"finish_reason":"stop"}]}"#,
        "[DONE]",
    ]))])
    .await;
    let m = montar(&servidor.url_base, 0, Duration::from_secs(5));
    let bytes = b"\x89PNG\r\n\x1a\nconteudo".to_vec();
    let request = pedido_com(vec![Message::User(vec![
        Part::Texto("o que está escrito?".into()),
        Part::Image {
            mime: "image/png".into(),
            bytes: bytes.clone(),
        },
    ])]);

    let fluxo = m.cliente.stream(request).await.expect("abre");
    let events = so_eventos(colher(fluxo).await);
    assert_eq!(events[0], LlmEvent::Texto("ARARA AZUL 42".into()));

    let body = &servidor.pedidos()[0];
    let parts = body["messages"][0]["content"].as_array().expect("partes");
    assert_eq!(parts[0]["type"], "text");
    assert_eq!(parts[1]["type"], "image_url");
    assert_eq!(
        parts[1]["image_url"]["url"],
        format!("data:image/png;base64,{}", encode(&bytes))
    );
}

#[tokio::test]
async fn retries_429_and_5xx_without_changing_request() {
    let servidor = Servidor::iniciar(vec![
        Resposta::erro(429, r#"{"error":{"message":"muitas requisições"}}"#),
        Resposta::erro(503, "indisponível"),
        Resposta::ok(sse(&[
            r#"{"choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ])),
    ])
    .await;
    let m = montar(&servidor.url_base, 2, Duration::from_secs(5));

    let fluxo = m
        .cliente
        .stream(pergunta("oi"))
        .await
        .expect("3ª tentativa dá certo");
    let events = so_eventos(colher(fluxo).await);
    assert_eq!(events[0], LlmEvent::Texto("ok".into()));

    let requests = servidor.pedidos();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0], requests[1]);
    assert_eq!(requests[1], requests[2]);
}

#[tokio::test]
async fn definitive_error_is_not_retried_and_does_not_echo_key() {
    let body = format!(r#"{{"error":{{"message":"chave recusada: {CHAVE}"}}}}"#);
    let servidor = Servidor::iniciar(vec![Resposta::erro(401, &body)]).await;
    let m = montar(&servidor.url_base, 2, Duration::from_secs(5));

    let error = m
        .cliente
        .stream(pergunta("oi"))
        .await
        .err()
        .expect("recusado");
    match &error {
        LlmError::Rejected { status, mensagem } => {
            assert_eq!(*status, 401);
            assert!(!mensagem.contains(CHAVE));
        }
        other => panic!("esperava Recusado, veio {other:?}"),
    }
    assert!(!format!("{error} {error:?}").contains(CHAVE));
    assert_eq!(servidor.pedidos().len(), 1, "401 não pode ser retentado");
    assert_eq!(m.metricas.llm_errors.value(), 1);
}

#[tokio::test]
async fn exhausted_retries_return_unavailable() {
    let servidor = Servidor::iniciar(vec![Resposta::erro(500, "falhou")]).await;
    let m = montar(&servidor.url_base, 2, Duration::from_secs(5));

    let error = m.cliente.stream(pergunta("oi")).await.err().expect("falha");
    assert_eq!(
        error,
        LlmError::Unavailable {
            tentativas: 3,
            last_error: "HTTP 500".into()
        }
    );
    assert_eq!(servidor.pedidos().len(), 3);
}

#[tokio::test]
async fn timeout_returns_timed_out_without_retry() {
    let servidor = Servidor::iniciar(vec![
        Resposta::ok(sse(&["[DONE]"])).com_atraso(Duration::from_secs(3)),
        Resposta::ok(sse(&["[DONE]"])),
    ])
    .await;
    let m = montar(&servidor.url_base, 2, Duration::from_secs(1));

    let error = m
        .cliente
        .stream(pergunta("oi"))
        .await
        .err()
        .expect("expira");
    assert_eq!(error, LlmError::TimedOut { seconds: 1 });
    assert_eq!(
        servidor.pedidos().len(),
        1,
        "timeout não pode ser retentado"
    );
}

#[tokio::test]
async fn cancel_mid_stream_closes_and_releases_slots() {
    let servidor = Servidor::iniciar(vec![
        Resposta::ok(sse(&[
            r#"{"choices":[{"index":0,"delta":{"content":"Começando"},"finish_reason":null}]}"#,
        ]))
        .deixar_aberta(),
    ])
    .await;
    let m = montar(&servidor.url_base, 0, Duration::from_secs(30));
    let request = pergunta("conta uma história longa");
    let cancellation = request.cancellation.clone();

    let mut fluxo = m.cliente.stream(request).await.expect("abre");
    assert_eq!(
        fluxo.next().await.expect("há evento").expect("sem erro"),
        LlmEvent::Texto("Começando".into())
    );
    assert_eq!(
        m.limites.available_globally(),
        3,
        "a chamada segura uma vaga global"
    );

    cancellation.cancel();
    let next_item = tokio::time::timeout(Duration::from_secs(2), fluxo.next())
        .await
        .expect("cancelamento não pode ficar pendurado");
    assert_eq!(next_item, Some(Err(LlmError::Cancelled)));
    assert!(
        fluxo.next().await.is_none(),
        "depois do cancelamento o stream acaba"
    );
    assert_eq!(
        m.limites.available_globally(),
        4,
        "as vagas voltam depois do cancelamento"
    );
    assert_eq!(
        m.metricas.llm_errors.value(),
        0,
        "cancelamento não conta como erro"
    );
}

#[tokio::test]
async fn truncated_response_becomes_error_and_is_not_repeated() {
    let servidor = Servidor::iniciar(vec![Resposta::ok(sse(&[
        r#"{"choices":[{"index":0,"delta":{"content":"Oi"},"finish_reason":null}]}"#,
    ]))])
    .await;
    let m = montar(&servidor.url_base, 2, Duration::from_secs(5));

    let fluxo = m.cliente.stream(pergunta("oi")).await.expect("abre");
    let items = colher(fluxo).await;
    assert_eq!(items.len(), 2);
    assert_eq!(items[0], Ok(LlmEvent::Texto("Oi".into())));
    assert!(matches!(items[1], Err(LlmError::InvalidResponse(_))));
    assert_eq!(
        servidor.pedidos().len(),
        1,
        "depois de texto já enviado não pode repetir"
    );
}

#[tokio::test]
async fn request_ending_in_model_turn_leaves_when_rule_is_off() {
    let servidor = Servidor::iniciar(vec![Resposta::ok(sse(&[
        r#"{"choices":[{"index":0,"delta":{"content":"Oi"}}]}"#,
        r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        "[DONE]",
    ]))])
    .await;
    let m = montar_com_regra(&servidor.url_base, 0, Duration::from_secs(5), false);
    let request = pedido_com(vec![
        Message::User(vec![Part::Texto("oi".into())]),
        Message::Assistant {
            texto: "olá".into(),
            calls: vec![],
        },
    ]);

    let fluxo = m.cliente.stream(request).await.expect("pedido aceito");
    let events = so_eventos(colher(fluxo).await);
    assert!(events.contains(&LlmEvent::Texto("Oi".into())), "{events:?}");
    let sent = servidor.pedidos();
    assert_eq!(sent.len(), 1, "o pedido tem de chegar ao servidor");
    let messages = sent[0]["messages"].as_array().expect("lista");
    assert_eq!(messages.last().expect("última")["role"], "assistant");
}

#[tokio::test]
async fn request_ending_in_model_turn_does_not_leave_process() {
    let servidor = Servidor::iniciar(vec![Resposta::ok(sse(&["[DONE]"]))]).await;
    let m = montar(&servidor.url_base, 2, Duration::from_secs(5));
    let request = pedido_com(vec![
        Message::User(vec![Part::Texto("oi".into())]),
        Message::Assistant {
            texto: "olá".into(),
            calls: vec![],
        },
    ]);

    let error = m.cliente.stream(request).await.err().expect("recusado");
    assert!(matches!(error, LlmError::InvalidRequest(_)));
    assert!(
        servidor.pedidos().is_empty(),
        "nada pode ser enviado ao gateway"
    );
}
