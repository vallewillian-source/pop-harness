//! Logs do cliente LLM: a chave nunca aparece no que é escrito, nem em erro.
//!
//! Fica num binário próprio de propósito: o cache de interesse do `tracing` é global, e os
//! outros testes já disparam os mesmos pontos de log sem assinante. Aqui só há este teste.

mod apoio;

use std::io;
use std::sync::{Arc, Mutex};

use std::time::Duration;

use apoio::*;
use poph_core::llm::LlmProvider;

/// Escritor que guarda tudo em memória, para ler o que o tracing escreveu.
#[derive(Clone, Default)]
struct Captura(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captura {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("trava").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn chave_nao_aparece_em_log_nem_em_erro() {
    let captura = Captura::default();
    let assinante = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer({
            let captura = captura.clone();
            move || captura.clone()
        })
        .finish();
    let _guarda = tracing::subscriber::set_default(assinante);
    // Outros testes deste binário já dispararam os mesmos pontos de log sem assinante; sem isto o
    // cache de interesse do tracing os marca como "ignorar" e o log não aparece.
    tracing::callsite::rebuild_interest_cache();

    let corpo = format!(r#"{{"error":{{"message":"chave inválida {CHAVE}"}}}}"#);
    let servidor = Servidor::iniciar(vec![
        Resposta::ok(sse(&[
            r#"{"choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ])),
        Resposta::erro(401, &corpo),
    ])
    .await;
    let m = montar(&servidor.url_base, 0, Duration::from_secs(5));

    let fluxo = m.cliente.stream(pergunta("oi")).await.expect("abre");
    let _ = colher(fluxo).await;
    let erro = m
        .cliente
        .stream(pergunta("oi"))
        .await
        .err()
        .expect("recusado");

    let registro = String::from_utf8(captura.0.lock().expect("trava").clone()).expect("utf8");
    assert!(
        registro.contains("resposta do LLM concluída"),
        "o log de sucesso foi escrito."
    );
    assert!(
        registro.contains("chamada ao LLM falhou"),
        "o log de falha foi escrito"
    );
    assert!(!registro.contains(CHAVE), "a chave não pode ir para o log");
    assert!(
        !format!("{erro} {erro:?}").contains(CHAVE),
        "nem para o erro"
    );

    // Mesma checagem no formato legível, o que aparece no terminal.
    let captura_legivel = Captura::default();
    let assinante_legivel = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer({
            let captura_legivel = captura_legivel.clone();
            move || captura_legivel.clone()
        })
        .finish();
    let _guarda_legivel = tracing::subscriber::set_default(assinante_legivel);
    tracing::callsite::rebuild_interest_cache();

    let erro_legivel = m
        .cliente
        .stream(pergunta("oi"))
        .await
        .err()
        .expect("recusado");
    let registro_legivel =
        String::from_utf8(captura_legivel.0.lock().expect("trava").clone()).expect("utf8");
    assert!(
        registro_legivel.contains("chamada ao LLM falhou"),
        "o log legível de falha foi escrito"
    );
    assert!(
        !registro_legivel.contains(CHAVE),
        "a chave não pode ir para o log legível"
    );
    assert!(!format!("{erro_legivel} {erro_legivel:?}").contains(CHAVE));
}
