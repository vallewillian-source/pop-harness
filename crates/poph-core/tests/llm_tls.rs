#[path = "apoio/mod.rs"]
mod support;

use std::time::Duration;

use futures::StreamExt;
use poph_core::llm::{FinishReason, LlmError, LlmEvent, LlmProvider};
use support::{
    Reply, Server, ServerTls, assemble, certificates, collect_stream, only_events, question, sse,
};

const HELLO_CHUNK: &str = r#"{"choices":[{"index":0,"delta":{"role":"assistant","content":"Olá"},"finish_reason":null}]}"#;
const FIM: &str = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}"#;

fn setup_trusting_certificate(url_base: &str, retentativas: u32) -> support::Assembly {
    let mut setup = assemble(url_base, retentativas, Duration::from_secs(5));
    setup.client = setup
        .client
        .clone()
        .with_test_root(certificates::CA)
        .expect("raiz de teste aceita");
    setup
}

#[tokio::test]
async fn https_works_when_certificate_is_trusted() {
    let servidor = ServerTls::start(
        vec![Reply::ok(sse(&[HELLO_CHUNK, FIM, "[DONE]"]))],
        certificates::SERVER_IP,
    )
    .await;
    assert!(
        servidor.url_base.starts_with("https://"),
        "{}",
        servidor.url_base
    );
    let m = setup_trusting_certificate(&servidor.url_base, 2);

    let event_stream = m
        .client
        .stream(question("oi"))
        .await
        .expect("abre por https");
    let events = only_events(collect_stream(event_stream).await);

    assert_eq!(events[0], LlmEvent::Texto("Olá".into()));
    assert_eq!(events.last(), Some(&LlmEvent::Fim(FinishReason::Completed)));
    let body = &servidor.requests()[0];
    assert_eq!(body["model"], "gemini-3.8-flash-tiered");
    assert_eq!(body["messages"][1]["content"], "oi");
    assert_eq!(m.metrics.llm_calls.value(), 1);
    assert_eq!(servidor.connections(), 1);
}

#[tokio::test]
async fn unknown_ca_certificate_is_refused_without_retry() {
    let servidor = ServerTls::start(
        vec![Reply::ok(sse(&[HELLO_CHUNK, "[DONE]"]))],
        certificates::SERVER_IP,
    )
    .await;
    let m = assemble(&servidor.url_base, 2, Duration::from_secs(5));

    let error = match m.client.stream(question("oi")).await {
        Ok(_) => panic!("certificado de CA desconhecida foi aceito"),
        Err(error) => error,
    };

    let LlmError::Tls(detail) = &error else {
        panic!("esperava erro de TLS, veio {error:?}");
    };
    assert!(!detail.is_empty(), "detalhe vazio");
    let text = error.to_string();
    assert!(text.contains("TLS"), "mensagem: {text}");
    assert!(
        !error.is_transient(),
        "erro de TLS não pode ser transitório"
    );
    assert_eq!(
        servidor.connections(),
        1,
        "o cliente tentou de novo uma falha de TLS"
    );
    assert!(
        servidor.requests().is_empty(),
        "o pedido não pode ter chegado"
    );
    assert!(
        !text.contains(support::TEST_KEY),
        "chave vazou no erro: {text}"
    );
    assert!(!format!("{error:?}").contains(support::TEST_KEY));
    assert_eq!(m.metrics.llm_errors.value(), 1);
}

#[tokio::test]
async fn server_name_mismatch_is_refused() {
    let servidor = ServerTls::start(
        vec![Reply::ok(sse(&[HELLO_CHUNK, "[DONE]"]))],
        certificates::SERVER_NAME,
    )
    .await;
    let m = setup_trusting_certificate(&servidor.url_base, 0);

    let error = match m.client.stream(question("oi")).await {
        Ok(_) => panic!("certificado com nome errado foi aceito"),
        Err(error) => error,
    };

    assert!(matches!(error, LlmError::Tls(_)), "erro: {error:?}");
    assert!(!error.is_transient());
    assert!(servidor.requests().is_empty());
}

#[tokio::test]
async fn http_keeps_working() {
    let servidor = Server::start(vec![Reply::ok(sse(&[HELLO_CHUNK, FIM, "[DONE]"]))]).await;
    let m = assemble(&servidor.url_base, 0, Duration::from_secs(5));

    let event_stream = m
        .client
        .stream(question("oi"))
        .await
        .expect("abre por http");
    let events: Vec<LlmEvent> = collect_stream(event_stream)
        .await
        .into_iter()
        .map(|item| item.expect("evento"))
        .collect();

    assert_eq!(events[0], LlmEvent::Texto("Olá".into()));
    assert_eq!(events.len(), 3);
}

#[tokio::test]
async fn tls_error_mid_stream_becomes_interrupted_connection() {
    let servidor = ServerTls::start(vec![Reply::ok(String::new())], certificates::SERVER_IP).await;
    let m = setup_trusting_certificate(&servidor.url_base, 0);

    let event_stream = m
        .client
        .stream(question("oi"))
        .await
        .expect("abre por https");
    let items: Vec<_> = event_stream.collect().await;
    let error = items
        .into_iter()
        .find_map(|item| item.err())
        .expect("falha ao ler o corpo vazio");
    assert!(
        matches!(
            error,
            LlmError::InvalidResponse(_) | LlmError::ConnectionInterrupted(_)
        ),
        "erro: {error:?}"
    );
}
