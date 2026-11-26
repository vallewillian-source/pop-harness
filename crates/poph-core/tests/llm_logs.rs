#[path = "apoio/mod.rs"]
mod support;

use std::io;
use std::sync::{Arc, Mutex};

use std::time::Duration;

use poph_core::llm::LlmProvider;
use support::*;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("trava").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn key_stays_out_of_log_and_error() {
    let capture = Capture::default();
    let assinante = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer({
            let capture_writer = capture.clone();
            move || capture_writer.clone()
        })
        .finish();
    let _guard = tracing::subscriber::set_default(assinante);
    tracing::callsite::rebuild_interest_cache();

    let body = format!(r#"{{"error":{{"message":"chave inválida {TEST_KEY}"}}}}"#);
    let servidor = Server::start(vec![
        Reply::ok(sse(&[
            r#"{"choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ])),
        Reply::error(401, &body),
    ])
    .await;
    let m = assemble(&servidor.url_base, 0, Duration::from_secs(5));

    let event_stream = m.client.stream(question("oi")).await.expect("abre");
    let _ = collect_stream(event_stream).await;
    let error = m
        .client
        .stream(question("oi"))
        .await
        .err()
        .expect("recusado");

    let registro = String::from_utf8(capture.0.lock().expect("trava").clone()).expect("utf8");
    assert!(
        registro.contains("resposta do LLM concluída"),
        "o log de sucesso foi escrito."
    );
    assert!(
        registro.contains("chamada ao LLM falhou"),
        "o log de falha foi escrito"
    );
    assert!(
        !registro.contains(TEST_KEY),
        "a chave não pode ir para o log"
    );
    assert!(
        !format!("{error} {error:?}").contains(TEST_KEY),
        "nem para o erro"
    );

    let readable_capture = Capture::default();
    let readable_subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer({
            let readable_capture_writer = readable_capture.clone();
            move || readable_capture_writer.clone()
        })
        .finish();
    let _readable_guard = tracing::subscriber::set_default(readable_subscriber);
    tracing::callsite::rebuild_interest_cache();

    let readable_error = m
        .client
        .stream(question("oi"))
        .await
        .err()
        .expect("recusado");
    let readable_record =
        String::from_utf8(readable_capture.0.lock().expect("trava").clone()).expect("utf8");
    assert!(
        readable_record.contains("chamada ao LLM falhou"),
        "o log legível de falha foi escrito"
    );
    assert!(
        !readable_record.contains(TEST_KEY),
        "a chave não pode ir para o log legível"
    );
    assert!(!format!("{readable_error} {readable_error:?}").contains(TEST_KEY));
}
