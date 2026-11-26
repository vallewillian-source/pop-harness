//! https de verdade no cliente do Jev (HR-10b): servidor TLS local com certificado de fixtura.
//!
//! Prova, sem rede externa: (a) requisição https completada e respostas lidas; (b) certificado de
//! CA desconhecida é recusado com erro de TLS definitivo (uma única conexão, sem retentativa) e sem
//! a chave no texto; (c) nome do servidor diferente do certificado também é recusado; (d) `http://`
//! continua funcionando.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_jev::{BatchResponse, ClienteJev, ConfigJev, JevError, Pedido, Question, TransporteJev};
use poph_types::{BaseMetrics, JevConfig, Registry, Secret};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

const CHAVE: &str = "chave-do-jev-https-nao-pode-vazar";

/// Certificados de teste (ver `tests/fixtures/tls/LEIAME.md`).
const CA: &[u8] = include_bytes!("fixtures/tls/ca.der");
const SERVIDOR_IP: &[u8] = include_bytes!("fixtures/tls/servidor-ip.der");
const SERVIDOR_NOME: &[u8] = include_bytes!("fixtures/tls/servidor-nome.der");
const SERVIDOR_CHAVE: &[u8] = include_bytes!("fixtures/tls/servidor-chave.der");

const CORPO: &str = r#"{"answers":{"g1_transferir":{"type":"choice","choice":"continuar_agente","probabilities":{"continuar_agente":0.95},"confidence":0.95}},"usage":{"input_tokens":20,"output_tokens":4}}"#;

fn metricas() -> BaseMetrics {
    BaseMetrics::register(&Registry::new()).expect("métricas base")
}

fn config(url_base: &str, retentativas: u32) -> ConfigJev {
    let jev = JevConfig {
        enabled: true,
        url_base: url_base.to_owned(),
        model: "jev-latest".to_owned(),
        timeout_seconds: 8,
        retries: retentativas,
        ..JevConfig::default()
    };
    ConfigJev::da_config(&jev, Secret::new(CHAVE.to_owned())).expect("configuração do cliente")
}

fn pedido() -> Pedido {
    let mut criterios = BTreeMap::new();
    criterios.insert("continuar_agente".to_owned(), "o cliente segue".to_owned());
    criterios.insert("transferir_agora".to_owned(), "quer atendente".to_owned());
    let mut perguntas = BTreeMap::new();
    perguntas.insert(
        "g1_transferir".to_owned(),
        Question::choice("transfere?", criterios),
    );
    Pedido {
        estado: serde_json::json!({"mensagens": [{"texto": "quero falar com alguém"}]}),
        modelo: "jev-latest".to_owned(),
        perguntas,
    }
}

/// Sobe um servidor HTTPS (rustls bloqueante em thread) e devolve a URL e o contador de conexões.
fn servidor_tls(certificado: &'static [u8]) -> (String, Arc<AtomicUsize>) {
    let tls = Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(certificado)],
                PrivateKeyDer::Pkcs8(SERVIDOR_CHAVE.into()),
            )
            .expect("configuração TLS de teste"),
    );
    let ouvinte = TcpListener::bind("127.0.0.1:0").expect("porta efêmera");
    let porta = ouvinte.local_addr().expect("endereço").port();
    let conexoes = Arc::new(AtomicUsize::new(0));
    let contador = Arc::clone(&conexoes);
    std::thread::spawn(move || {
        for conexao in ouvinte.incoming() {
            let Ok(conexao) = conexao else { return };
            contador.fetch_add(1, Ordering::SeqCst);
            let tls = Arc::clone(&tls);
            std::thread::spawn(move || atender_tls(tls, conexao));
        }
    });
    (format!("https://127.0.0.1:{porta}"), conexoes)
}

fn atender_tls(tls: Arc<ServerConfig>, conexao: TcpStream) {
    let Ok(sessao) = ServerConnection::new(tls) else {
        return;
    };
    let mut fluxo = StreamOwned::new(sessao, conexao);
    // Handshake recusado pelo cliente é esperado em um dos testes: para aqui sem erro.
    if ler_pedido(&mut fluxo).is_err() {
        return;
    }
    let resposta = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{CORPO}",
        CORPO.len()
    );
    let _ = fluxo.write_all(resposta.as_bytes());
    let _ = fluxo.flush();
}

/// Lê cabeçalhos + corpo de uma requisição HTTP/1.1.
fn ler_pedido(fluxo: &mut StreamOwned<ServerConnection, TcpStream>) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    let inicio = loop {
        if let Some(posicao) = buf.windows(4).position(|janela| janela == b"\r\n\r\n") {
            break posicao + 4;
        }
        let lidos = fluxo.read(&mut tmp)?;
        if lidos == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..lidos]);
    };
    let cabecalho = String::from_utf8_lossy(&buf[..inicio]).to_ascii_lowercase();
    let tamanho = cabecalho
        .lines()
        .find_map(|linha| linha.strip_prefix("content-length:"))
        .and_then(|valor| valor.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buf.len() < inicio + tamanho {
        let lidos = fluxo.read(&mut tmp)?;
        if lidos == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..lidos]);
    }
    Ok(())
}

#[tokio::test]
async fn https_funciona_quando_o_certificado_e_confiavel() {
    let (url, conexoes) = servidor_tls(SERVIDOR_IP);
    assert!(url.starts_with("https://"));
    let cliente = ClienteJev::novo(config(&url, 0), metricas())
        .expect("cliente")
        .com_raiz_de_teste(CA)
        .expect("raiz de teste aceita");

    let lote: BatchResponse = cliente.julgar(pedido()).await.expect("responde por https");

    let resposta = &lote.answers["g1_transferir"];
    assert_eq!(resposta.choice.as_deref(), Some("continuar_agente"));
    assert!((resposta.confidence - 0.95).abs() < 0.001);
    assert_eq!(lote.tokens, 24);
    assert_eq!(conexoes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn certificado_de_ca_desconhecida_e_recusado_sem_retentativa() {
    let (url, conexoes) = servidor_tls(SERVIDOR_IP);
    // Cliente padrão: confia só nas raízes do sistema, não na CA de teste.
    let cliente = ClienteJev::novo(config(&url, 1), metricas()).expect("cliente");

    let erro = cliente
        .julgar(pedido())
        .await
        .expect_err("certificado não confiável");

    assert!(matches!(erro, JevError::Tls(_)), "erro: {erro:?}");
    let texto = erro.to_string();
    assert!(texto.contains("TLS"), "mensagem: {texto}");
    // Definitivo: a política de retentativa não pode repetir certificado recusado.
    assert!(!erro.is_transient(), "erro de TLS não pode ser transitório");
    assert_eq!(
        conexoes.load(Ordering::SeqCst),
        1,
        "o cliente tentou de novo uma falha de TLS"
    );
    assert!(!texto.contains(CHAVE), "chave vazou no erro: {texto}");
    assert!(!format!("{erro:?}").contains(CHAVE));
}

#[tokio::test]
async fn nome_do_servidor_diferente_do_certificado_e_recusado() {
    // O servidor apresenta certificado para `localhost`; o cliente conecta em `127.0.0.1`.
    let (url, _conexoes) = servidor_tls(SERVIDOR_NOME);
    let cliente = ClienteJev::novo(config(&url, 0), metricas())
        .expect("cliente")
        .com_raiz_de_teste(CA)
        .expect("raiz de teste aceita");

    let erro = cliente.julgar(pedido()).await.expect_err("nome errado");

    assert!(matches!(erro, JevError::Tls(_)), "erro: {erro:?}");
    assert!(!erro.is_transient());
}

#[tokio::test]
async fn http_continua_funcionando() {
    // O gateway local em http segue atendendo (o endereço do [jev] pode ser http ou https).
    let ouvinte = TcpListener::bind("127.0.0.1:0").expect("porta efêmera");
    let porta = ouvinte.local_addr().expect("endereço").port();
    std::thread::spawn(move || {
        if let Ok((mut conexao, _)) = ouvinte.accept() {
            let mut buf = [0u8; 4096];
            let _ = conexao.read(&mut buf);
            let resposta = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{CORPO}",
                CORPO.len()
            );
            let _ = conexao.write_all(resposta.as_bytes());
        }
    });
    let cliente = ClienteJev::novo(config(&format!("http://127.0.0.1:{porta}"), 0), metricas())
        .expect("cliente");

    let lote = cliente.julgar(pedido()).await.expect("responde por http");

    assert_eq!(
        lote.answers["g1_transferir"].choice.as_deref(),
        Some("continuar_agente")
    );
    assert!(
        lote.answers["g1_transferir"]
            .probabilities
            .contains_key("continuar_agente")
    );
}
