#[path = "apoio/mod.rs"]
mod support;

use std::time::Duration;

use futures::StreamExt;
use poph_core::llm::{FinishReason, LlmError, LlmEvent, LlmProvider};
use support::{
    Resposta, Servidor, ServidorTls, certificados, colher, montar, pergunta, so_eventos, sse,
};

const HELLO_CHUNK: &str = r#"{"choices":[{"index":0,"delta":{"role":"assistant","content":"Olá"},"finish_reason":null}]}"#;
const FIM: &str = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}"#;

fn setup_trusting_certificate(url_base: &str, retentativas: u32) -> support::Montagem {
    let mut setup = montar(url_base, retentativas, Duration::from_secs(5));
    setup.cliente = setup
        .cliente
        .clone()
        .with_test_root(certificados::CA)
        .expect("raiz de teste aceita");
    setup
}

#[tokio::test]
async fn https_works_when_certificate_is_trusted() {
    let servidor = ServidorTls::iniciar(
        vec![Resposta::ok(sse(&[HELLO_CHUNK, FIM, "[DONE]"]))],
        certificados::SERVIDOR_IP,
    )
    .await;
    assert!(
        servidor.url_base.starts_with("https://"),
        "{}",
        servidor.url_base
    );
    let m = setup_trusting_certificate(&servidor.url_base, 2);

    let fluxo = m
        .cliente
        .stream(pergunta("oi"))
        .await
        .expect("abre por https");
    let events = so_eventos(colher(fluxo).await);

    assert_eq!(events[0], LlmEvent::Texto("Olá".into()));
    assert_eq!(events.last(), Some(&LlmEvent::Fim(FinishReason::Completed)));
    let body = &servidor.pedidos()[0];
    assert_eq!(body["model"], "gemini-3.8-flash-tiered");
    assert_eq!(body["messages"][1]["content"], "oi");
    assert_eq!(m.metricas.llm_calls.value(), 1);
    assert_eq!(servidor.conexoes(), 1);
}

#[tokio::test]
async fn unknown_ca_certificate_is_refused_without_retry() {
    let servidor = ServidorTls::iniciar(
        vec![Resposta::ok(sse(&[HELLO_CHUNK, "[DONE]"]))],
        certificados::SERVIDOR_IP,
    )
    .await;
    let m = montar(&servidor.url_base, 2, Duration::from_secs(5));

    let error = match m.cliente.stream(pergunta("oi")).await {
        Ok(_) => panic!("certificado de CA desconhecida foi aceito"),
        Err(error) => error,
    };

    let LlmError::Tls(detail) = &error else {
        panic!("esperava erro de TLS, veio {error:?}");
    };
    assert!(!detail.is_empty(), "detalhe vazio");
    let texto = error.to_string();
    assert!(texto.contains("TLS"), "mensagem: {texto}");
    assert!(
        !error.is_transient(),
        "erro de TLS não pode ser transitório"
    );
    assert_eq!(
        servidor.conexoes(),
        1,
        "o cliente tentou de novo uma falha de TLS"
    );
    assert!(
        servidor.pedidos().is_empty(),
        "o pedido não pode ter chegado"
    );
    assert!(
        !texto.contains(support::CHAVE),
        "chave vazou no erro: {texto}"
    );
    assert!(!format!("{error:?}").contains(support::CHAVE));
    assert_eq!(m.metricas.llm_errors.value(), 1);
}

#[tokio::test]
async fn server_name_mismatch_is_refused() {
    let servidor = ServidorTls::iniciar(
        vec![Resposta::ok(sse(&[HELLO_CHUNK, "[DONE]"]))],
        certificados::SERVIDOR_NOME,
    )
    .await;
    let m = setup_trusting_certificate(&servidor.url_base, 0);

    let error = match m.cliente.stream(pergunta("oi")).await {
        Ok(_) => panic!("certificado com nome errado foi aceito"),
        Err(error) => error,
    };

    assert!(matches!(error, LlmError::Tls(_)), "erro: {error:?}");
    assert!(!error.is_transient());
    assert!(servidor.pedidos().is_empty());
}

#[tokio::test]
async fn http_keeps_working() {
    let servidor = Servidor::iniciar(vec![Resposta::ok(sse(&[HELLO_CHUNK, FIM, "[DONE]"]))]).await;
    let m = montar(&servidor.url_base, 0, Duration::from_secs(5));

    let fluxo = m
        .cliente
        .stream(pergunta("oi"))
        .await
        .expect("abre por http");
    let events: Vec<LlmEvent> = colher(fluxo)
        .await
        .into_iter()
        .map(|item| item.expect("evento"))
        .collect();

    assert_eq!(events[0], LlmEvent::Texto("Olá".into()));
    assert_eq!(events.len(), 3);
}

#[tokio::test]
async fn tls_error_mid_stream_becomes_interrupted_connection() {
    let servidor =
        ServidorTls::iniciar(vec![Resposta::ok(String::new())], certificados::SERVIDOR_IP).await;
    let m = setup_trusting_certificate(&servidor.url_base, 0);

    let fluxo = m
        .cliente
        .stream(pergunta("oi"))
        .await
        .expect("abre por https");
    let items: Vec<_> = fluxo.collect().await;
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
