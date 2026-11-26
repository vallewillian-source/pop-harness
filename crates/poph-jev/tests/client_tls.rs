use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_jev::{
    BatchResponse, JevClient, JevClientConfig, JevError, JevRequest, JevTransport, Question,
};
use poph_types::{BaseMetrics, JevConfig, Registry, Secret};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

const CHAVE: &str = "chave-do-jev-https-nao-pode-vazar";

const CA: &[u8] = include_bytes!("fixtures/tls/ca.der");
const SERVER_IP: &[u8] = include_bytes!("fixtures/tls/servidor-ip.der");
const SERVER_NAME: &[u8] = include_bytes!("fixtures/tls/servidor-nome.der");
const SERVER_KEY: &[u8] = include_bytes!("fixtures/tls/servidor-chave.der");

const BODY: &str = r#"{"answers":{"g1_transferir":{"type":"choice","choice":"continuar_agente","probabilities":{"continuar_agente":0.95},"confidence":0.95}},"usage":{"input_tokens":20,"output_tokens":4}}"#;

fn metrics() -> BaseMetrics {
    BaseMetrics::register(&Registry::new()).expect("métricas base")
}

fn config(url_base: &str, retentativas: u32) -> JevClientConfig {
    let jev = JevConfig {
        enabled: true,
        url_base: url_base.to_owned(),
        model: "jev-latest".to_owned(),
        timeout_seconds: 8,
        retries: retentativas,
        ..JevConfig::default()
    };
    JevClientConfig::from_config(&jev, Secret::new(CHAVE.to_owned()))
        .expect("configuração do cliente")
}

fn sample_request() -> JevRequest {
    let mut rubric = BTreeMap::new();
    rubric.insert("continuar_agente".to_owned(), "o cliente segue".to_owned());
    rubric.insert("transferir_agora".to_owned(), "quer atendente".to_owned());
    let mut question_map = BTreeMap::new();
    question_map.insert(
        "g1_transferir".to_owned(),
        Question::choice("transfere?", rubric),
    );
    JevRequest {
        estado: serde_json::json!({"mensagens": [{"texto": "quero falar com alguém"}]}),
        modelo: "jev-latest".to_owned(),
        questions: question_map,
    }
}

fn tls_server(certificate: &'static [u8]) -> (String, Arc<AtomicUsize>) {
    let tls = Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(certificate)],
                PrivateKeyDer::Pkcs8(SERVER_KEY.into()),
            )
            .expect("configuração TLS de teste"),
    );
    let listener = TcpListener::bind("127.0.0.1:0").expect("porta efêmera");
    let porta = listener.local_addr().expect("endereço").port();
    let connections = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&connections);
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(connection) = connection else { return };
            counter.fetch_add(1, Ordering::SeqCst);
            let tls = Arc::clone(&tls);
            std::thread::spawn(move || serve_tls(tls, connection));
        }
    });
    (format!("https://127.0.0.1:{porta}"), connections)
}

fn serve_tls(tls: Arc<ServerConfig>, connection: TcpStream) {
    let Ok(session) = ServerConnection::new(tls) else {
        return;
    };
    let mut fluxo = StreamOwned::new(session, connection);
    if read_request(&mut fluxo).is_err() {
        return;
    }
    let answer = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{BODY}",
        BODY.len()
    );
    let _ = fluxo.write_all(answer.as_bytes());
    let _ = fluxo.flush();
}

fn read_request(fluxo: &mut StreamOwned<ServerConnection, TcpStream>) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    let start = loop {
        if let Some(position) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        let read_bytes = fluxo.read(&mut tmp)?;
        if read_bytes == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..read_bytes]);
    };
    let header = String::from_utf8_lossy(&buf[..start]).to_ascii_lowercase();
    let size = header
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buf.len() < start + size {
        let read_bytes = fluxo.read(&mut tmp)?;
        if read_bytes == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..read_bytes]);
    }
    Ok(())
}

#[tokio::test]
async fn https_works_when_the_certificate_is_trusted() {
    let (url, connections) = tls_server(SERVER_IP);
    assert!(url.starts_with("https://"));
    let client = JevClient::new(config(&url, 0), metrics())
        .expect("cliente")
        .with_test_root(CA)
        .expect("raiz de teste aceita");

    let batch: BatchResponse = client
        .judge(sample_request())
        .await
        .expect("responde por https");

    let answer = &batch.answers["g1_transferir"];
    assert_eq!(answer.choice.as_deref(), Some("continuar_agente"));
    assert!((answer.confidence - 0.95).abs() < 0.001);
    assert_eq!(batch.tokens, 24);
    assert_eq!(connections.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unknown_ca_certificate_is_refused_without_retry() {
    let (url, connections) = tls_server(SERVER_IP);
    let client = JevClient::new(config(&url, 1), metrics()).expect("cliente");

    let error = client
        .judge(sample_request())
        .await
        .expect_err("certificado não confiável");

    assert!(matches!(error, JevError::Tls(_)), "erro: {error:?}");
    let texto = error.to_string();
    assert!(texto.contains("TLS"), "mensagem: {texto}");
    assert!(
        !error.is_transient(),
        "erro de TLS não pode ser transitório"
    );
    assert_eq!(
        connections.load(Ordering::SeqCst),
        1,
        "o cliente tentou de novo uma falha de TLS"
    );
    assert!(!texto.contains(CHAVE), "chave vazou no erro: {texto}");
    assert!(!format!("{error:?}").contains(CHAVE));
}

#[tokio::test]
async fn server_name_mismatch_is_refused() {
    let (url, _connections) = tls_server(SERVER_NAME);
    let client = JevClient::new(config(&url, 0), metrics())
        .expect("cliente")
        .with_test_root(CA)
        .expect("raiz de teste aceita");

    let error = client
        .judge(sample_request())
        .await
        .expect_err("nome errado");

    assert!(matches!(error, JevError::Tls(_)), "erro: {error:?}");
    assert!(!error.is_transient());
}

#[tokio::test]
async fn http_still_works() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("porta efêmera");
    let porta = listener.local_addr().expect("endereço").port();
    std::thread::spawn(move || {
        if let Ok((mut connection, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = connection.read(&mut buf);
            let answer = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{BODY}",
                BODY.len()
            );
            let _ = connection.write_all(answer.as_bytes());
        }
    });
    let client = JevClient::new(config(&format!("http://127.0.0.1:{porta}"), 0), metrics())
        .expect("cliente");

    let batch = client
        .judge(sample_request())
        .await
        .expect("responde por http");

    assert_eq!(
        batch.answers["g1_transferir"].choice.as_deref(),
        Some("continuar_agente")
    );
    assert!(
        batch.answers["g1_transferir"]
            .probabilities
            .contains_key("continuar_agente")
    );
}
