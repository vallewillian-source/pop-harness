//! https de verdade no cliente do provedor (HR-10b): servidor TLS local com certificado de fixtura.
//!
//! Prova, sem rede externa: (a) requisição https completada e stream lido; (b) certificado de uma CA
//! desconhecida é recusado com erro de TLS definitivo, uma única tentativa, mensagem legível e sem a
//! chave; (c) nome do servidor diferente do certificado também é recusado; (d) `http://` continua
//! funcionando.

mod apoio;

use std::time::Duration;

use apoio::{
    Resposta, Servidor, ServidorTls, certificados, colher, montar, pergunta, so_eventos, sse,
};
use futures::StreamExt;
use poph_core::llm::{EventoLlm, LlmError, LlmProvider, MotivoFim};

const TEXTO_OLA: &str = r#"{"choices":[{"index":0,"delta":{"role":"assistant","content":"Olá"},"finish_reason":null}]}"#;
const FIM: &str = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}"#;

/// Cliente do teste: o `montar` padrão mais a raiz extra (a CA de teste).
fn montar_confiando_no_certificado(url_base: &str, retentativas: u32) -> apoio::Montagem {
    let mut montagem = montar(url_base, retentativas, Duration::from_secs(5));
    montagem.cliente = montagem
        .cliente
        .clone()
        .com_raiz_de_teste(certificados::CA)
        .expect("raiz de teste aceita");
    montagem
}

#[tokio::test]
async fn https_funciona_quando_o_certificado_e_confiavel() {
    let servidor = ServidorTls::iniciar(
        vec![Resposta::ok(sse(&[TEXTO_OLA, FIM, "[DONE]"]))],
        certificados::SERVIDOR_IP,
    )
    .await;
    assert!(
        servidor.url_base.starts_with("https://"),
        "{}",
        servidor.url_base
    );
    let m = montar_confiando_no_certificado(&servidor.url_base, 2);

    let fluxo = m
        .cliente
        .stream(pergunta("oi"))
        .await
        .expect("abre por https");
    let eventos = so_eventos(colher(fluxo).await);

    assert_eq!(eventos[0], EventoLlm::Texto("Olá".into()));
    assert_eq!(eventos.last(), Some(&EventoLlm::Fim(MotivoFim::Concluido)));
    // O pedido chegou inteiro pelo túnel TLS.
    let corpo = &servidor.pedidos()[0];
    assert_eq!(corpo["model"], "gemini-3.8-flash-tiered");
    assert_eq!(corpo["messages"][1]["content"], "oi");
    assert_eq!(m.metricas.llm_calls.value(), 1);
    assert_eq!(servidor.conexoes(), 1);
}

#[tokio::test]
async fn certificado_de_ca_desconhecida_e_recusado_e_nao_retenta() {
    let servidor = ServidorTls::iniciar(
        vec![Resposta::ok(sse(&[TEXTO_OLA, "[DONE]"]))],
        certificados::SERVIDOR_IP,
    )
    .await;
    // Cliente padrão: confia só nas raízes do sistema, não na CA de teste.
    let m = montar(&servidor.url_base, 2, Duration::from_secs(5));

    let erro = match m.cliente.stream(pergunta("oi")).await {
        Ok(_) => panic!("certificado de CA desconhecida foi aceito"),
        Err(erro) => erro,
    };

    let LlmError::Tls(detalhe) = &erro else {
        panic!("esperava erro de TLS, veio {erro:?}");
    };
    assert!(!detalhe.is_empty(), "detalhe vazio");
    let texto = erro.to_string();
    assert!(texto.contains("TLS"), "mensagem: {texto}");
    // Erro definitivo: a política de retentativa não pode tentar de novo um certificado recusado.
    assert!(!erro.is_transient(), "erro de TLS não pode ser transitório");
    assert_eq!(
        servidor.conexoes(),
        1,
        "o cliente tentou de novo uma falha de TLS"
    );
    assert!(
        servidor.pedidos().is_empty(),
        "o pedido não pode ter chegado"
    );
    // A chave continua fora do erro, inclusive do erro novo.
    assert!(
        !texto.contains(apoio::CHAVE),
        "chave vazou no erro: {texto}"
    );
    assert!(!format!("{erro:?}").contains(apoio::CHAVE));
    assert_eq!(m.metricas.llm_errors.value(), 1);
}

#[tokio::test]
async fn nome_do_servidor_diferente_do_certificado_e_recusado() {
    // O servidor apresenta certificado para `localhost`; o cliente conecta em `127.0.0.1`.
    let servidor = ServidorTls::iniciar(
        vec![Resposta::ok(sse(&[TEXTO_OLA, "[DONE]"]))],
        certificados::SERVIDOR_NOME,
    )
    .await;
    let m = montar_confiando_no_certificado(&servidor.url_base, 0);

    let erro = match m.cliente.stream(pergunta("oi")).await {
        Ok(_) => panic!("certificado com nome errado foi aceito"),
        Err(erro) => erro,
    };

    assert!(matches!(erro, LlmError::Tls(_)), "erro: {erro:?}");
    assert!(!erro.is_transient());
    assert!(servidor.pedidos().is_empty());
}

#[tokio::test]
async fn http_continua_funcionando() {
    let servidor = Servidor::iniciar(vec![Resposta::ok(sse(&[TEXTO_OLA, FIM, "[DONE]"]))]).await;
    let m = montar(&servidor.url_base, 0, Duration::from_secs(5));

    let fluxo = m
        .cliente
        .stream(pergunta("oi"))
        .await
        .expect("abre por http");
    let eventos: Vec<EventoLlm> = colher(fluxo)
        .await
        .into_iter()
        .map(|item| item.expect("evento"))
        .collect();

    assert_eq!(eventos[0], EventoLlm::Texto("Olá".into()));
    assert_eq!(eventos.len(), 3);
}

#[tokio::test]
async fn erro_de_tls_no_meio_do_stream_vira_conexao_interrompida() {
    // Servidor que completa o handshake TLS e fecha sem responder o corpo: prova que ler depois do
    // handshake continua caindo no erro já existente, e não no erro de TLS novo.
    let servidor =
        ServidorTls::iniciar(vec![Resposta::ok(String::new())], certificados::SERVIDOR_IP).await;
    let m = montar_confiando_no_certificado(&servidor.url_base, 0);

    let fluxo = m
        .cliente
        .stream(pergunta("oi"))
        .await
        .expect("abre por https");
    let itens: Vec<_> = fluxo.collect().await;
    let erro = itens
        .into_iter()
        .find_map(|item| item.err())
        .expect("falha ao ler o corpo vazio");
    assert!(
        matches!(
            erro,
            LlmError::InvalidResponse(_) | LlmError::ConnectionInterrupted(_)
        ),
        "erro: {erro:?}"
    );
}
