#![allow(dead_code, unused_imports)]

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use poph_core::llm::base64::encode;
use poph_core::llm::{
    FinishReason, LlmError, LlmEvent, LlmLimiter, LlmProvider, LlmStream, Message,
    OpenAiCompatibleClient, OpenAiCompatibleConfig, Part, Request, Tool, ToolCall, Usage,
};
use poph_types::{BaseMetrics, OrgId, Registry, Secret};
use serde_json::{Value, json};
use std::sync::atomic::AtomicUsize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

pub const TEST_KEY: &str = "chave-de-teste-XYZ-987";

#[derive(Clone)]
pub struct Reply {
    pub status: u16,
    pub body: String,
    pub delay: Duration,
    pub keep_open: bool,
}

impl Reply {
    pub fn ok(body: String) -> Self {
        Self {
            status: 200,
            body,
            delay: Duration::ZERO,
            keep_open: false,
        }
    }

    pub fn error(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_owned(),
            delay: Duration::ZERO,
            keep_open: false,
        }
    }

    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn keep_open(mut self) -> Self {
        self.keep_open = true;
        self
    }
}

pub fn sse(lines: &[&str]) -> String {
    lines.iter().map(|l| format!("data: {l}\n\n")).collect()
}

pub struct Server {
    pub url_base: String,
    requests: Arc<Mutex<Vec<Value>>>,
}

impl Server {
    pub async fn start(script: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("porta livre");
        let port = listener.local_addr().expect("endereço").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let queue = Arc::new(Mutex::new(VecDeque::from(script)));
        let server_requests = Arc::clone(&requests);
        tokio::spawn(async move {
            while let Ok((connection, _)) = listener.accept().await {
                let requests = Arc::clone(&server_requests);
                let queue = Arc::clone(&queue);
                tokio::spawn(serve(connection, requests, queue));
            }
        });
        Self {
            url_base: format!("http://127.0.0.1:{port}/v1"),
            requests,
        }
    }

    pub fn requests(&self) -> Vec<Value> {
        self.requests.lock().expect("trava").clone()
    }
}

pub async fn serve(
    mut connection: TcpStream,
    requests: Arc<Mutex<Vec<Value>>>,
    queue: Arc<Mutex<VecDeque<Reply>>>,
) {
    let body = read_request(&mut connection).await;
    requests
        .lock()
        .expect("trava")
        .push(serde_json::from_slice(&body).unwrap_or(Value::Null));

    let reply = {
        let mut queue = queue.lock().expect("trava");
        if queue.len() > 1 {
            queue.pop_front().expect("há itens")
        } else {
            queue.front().cloned().expect("roteiro não vazio")
        }
    };
    if !reply.delay.is_zero() {
        tokio::time::sleep(reply.delay).await;
    }
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
        reply.status,
        if reply.status == 200 { "OK" } else { "ERRO" }
    );
    let _ = connection.write_all(header.as_bytes()).await;
    let _ = connection.write_all(reply.body.as_bytes()).await;
    let _ = connection.flush().await;
    if reply.keep_open {
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

pub async fn read_request(connection: &mut TcpStream) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let body_start = loop {
        if let Some(index) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        match connection.read(&mut tmp).await {
            Ok(n) if n > 0 => buf.extend_from_slice(&tmp[..n]),
            _ => return Vec::new(),
        }
    };
    let header = String::from_utf8_lossy(&buf[..body_start]).to_ascii_lowercase();
    let size = header
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buf.len() < body_start + size {
        match connection.read(&mut tmp).await {
            Ok(n) if n > 0 => buf.extend_from_slice(&tmp[..n]),
            _ => break,
        }
    }
    let fim = (body_start + size).min(buf.len());
    buf[body_start..fim].to_vec()
}

pub struct Assembly {
    pub client: OpenAiCompatibleClient,
    pub limiter: LlmLimiter,
    pub metrics: BaseMetrics,
}

pub fn assemble(url_base: &str, retries: u32, timeout: Duration) -> Assembly {
    assemble_with_rule(url_base, retries, timeout, true)
}

pub fn assemble_with_rule(
    url_base: &str,
    retries: u32,
    timeout: Duration,
    refuses_model_turn: bool,
) -> Assembly {
    let config = OpenAiCompatibleConfig {
        url_base: url_base.to_owned(),
        chave: Secret::new(TEST_KEY.to_owned()),
        modelo: "gemini-3.8-flash-tiered".to_owned(),
        timeout,
        retentativas: retries,
        base_wait: Duration::from_millis(10),
        temperatura: 0.0,
        max_tokens_saida: 512,
        recusa_turno_modelo: refuses_model_turn,
    };
    let limiter = LlmLimiter::new(4, 2).expect("limites válidos");
    let metrics = BaseMetrics::register(&Registry::new()).expect("catálogo");
    let client =
        OpenAiCompatibleClient::new(config, limiter.clone(), metrics.clone()).expect("cliente");
    Assembly {
        client,
        limiter,
        metrics,
    }
}

pub fn request_with(messages: Vec<Message>) -> Request {
    Request::new(OrgId::generate(), messages)
}

pub fn question(text: &str) -> Request {
    request_with(vec![
        Message::System("Você é um assistente de atendimento.".to_owned()),
        Message::User(vec![Part::Texto(text.to_owned())]),
    ])
}

pub async fn collect_stream(mut event_stream: LlmStream) -> Vec<Result<LlmEvent, LlmError>> {
    let mut items = Vec::new();
    while let Some(item) = event_stream.next().await {
        items.push(item);
    }
    items
}

pub fn only_events(items: Vec<Result<LlmEvent, LlmError>>) -> Vec<LlmEvent> {
    items
        .into_iter()
        .map(|item| item.expect("evento sem erro"))
        .collect()
}

pub mod certificates {
    pub const CA: &[u8] = include_bytes!("../fixtures/tls/ca.der");
    pub const SERVER_IP: &[u8] = include_bytes!("../fixtures/tls/servidor-ip.der");
    pub const SERVER_NAME: &[u8] = include_bytes!("../fixtures/tls/servidor-nome.der");
    pub const SERVER_KEY: &[u8] = include_bytes!("../fixtures/tls/servidor-chave.der");
}

pub struct ServerTls {
    pub url_base: String,
    requests: Arc<Mutex<Vec<Value>>>,
    connections: Arc<AtomicUsize>,
}

impl ServerTls {
    pub async fn start(script: Vec<Reply>, certificate: &'static [u8]) -> Self {
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(certificate)],
                PrivateKeyDer::Pkcs8(certificates::SERVER_KEY.into()),
            )
            .expect("configuração TLS de teste");
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("porta livre");
        let port = listener.local_addr().expect("endereço").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let queue = Arc::new(Mutex::new(VecDeque::from(script)));
        let server_requests = Arc::clone(&requests);
        let server_connections = Arc::clone(&connections);
        tokio::spawn(async move {
            while let Ok((connection, _)) = listener.accept().await {
                server_connections.fetch_add(1, Ordering::SeqCst);
                let acceptor = acceptor.clone();
                let requests = Arc::clone(&server_requests);
                let queue = Arc::clone(&queue);
                tokio::spawn(async move {
                    let Ok(mut tls) = acceptor.accept(connection).await else {
                        return;
                    };
                    serve_tls(&mut tls, requests, queue).await;
                });
            }
        });
        Self {
            url_base: format!("https://127.0.0.1:{port}/v1"),
            requests,
            connections,
        }
    }

    pub fn requests(&self) -> Vec<Value> {
        self.requests.lock().expect("trava").clone()
    }

    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

async fn serve_tls<S>(
    connection: &mut S,
    requests: Arc<Mutex<Vec<Value>>>,
    queue: Arc<Mutex<VecDeque<Reply>>>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let body = read_request_generic(connection).await;
    requests
        .lock()
        .expect("trava")
        .push(serde_json::from_slice(&body).unwrap_or(Value::Null));
    let reply = {
        let mut queue = queue.lock().expect("trava");
        if queue.len() > 1 {
            queue.pop_front().expect("há itens")
        } else {
            queue.front().cloned().expect("roteiro não vazio")
        }
    };
    if !reply.delay.is_zero() {
        tokio::time::sleep(reply.delay).await;
    }
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
        reply.status,
        if reply.status == 200 { "OK" } else { "ERRO" }
    );
    let _ = connection.write_all(header.as_bytes()).await;
    let _ = connection.write_all(reply.body.as_bytes()).await;
    let _ = connection.flush().await;
}

pub async fn read_request_generic<S>(connection: &mut S) -> Vec<u8>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let body_start = loop {
        if let Some(index) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        match connection.read(&mut tmp).await {
            Ok(n) if n > 0 => buf.extend_from_slice(&tmp[..n]),
            _ => return Vec::new(),
        }
    };
    let header = String::from_utf8_lossy(&buf[..body_start]).to_ascii_lowercase();
    let size = header
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buf.len() < body_start + size {
        match connection.read(&mut tmp).await {
            Ok(n) if n > 0 => buf.extend_from_slice(&tmp[..n]),
            _ => break,
        }
    }
    let fim = (body_start + size).min(buf.len());
    buf[body_start..fim].to_vec()
}
