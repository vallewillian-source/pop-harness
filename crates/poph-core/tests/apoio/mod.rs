#![allow(dead_code, unused_imports)]
//! Auxiliares dos testes do cliente LLM: servidor HTTP falso e montagem do cliente.

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use poph_core::llm::base64::codificar;
use poph_core::llm::{
    ChamadaFerramenta, ErroLlm, EventoLlm, Ferramenta, FluxoLlm, LimitadorLlm, LlmProvider,
    Mensagem, MotivoFim, OpenAiCompatibleClient, OpenAiCompatibleConfig, Parte, Pedido, Uso,
};
use poph_types::{BaseMetrics, OrgId, Registry, Secret};
use serde_json::{Value, json};
use std::sync::atomic::AtomicUsize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// Chave de teste. Nenhum teste pode deixá-la aparecer em log, erro ou corpo de pedido de retorno.
pub const CHAVE: &str = "chave-de-teste-XYZ-987";

#[derive(Clone)]
pub struct Resposta {
    pub status: u16,
    pub corpo: String,
    pub atraso: Duration,
    pub deixar_aberta: bool,
}

impl Resposta {
    pub fn ok(corpo: String) -> Self {
        Self {
            status: 200,
            corpo,
            atraso: Duration::ZERO,
            deixar_aberta: false,
        }
    }

    pub fn erro(status: u16, corpo: &str) -> Self {
        Self {
            status,
            corpo: corpo.to_owned(),
            atraso: Duration::ZERO,
            deixar_aberta: false,
        }
    }

    pub fn com_atraso(mut self, atraso: Duration) -> Self {
        self.atraso = atraso;
        self
    }

    pub fn deixar_aberta(mut self) -> Self {
        self.deixar_aberta = true;
        self
    }
}

/// Monta um corpo SSE com uma linha `data:` por item.
pub fn sse(linhas: &[&str]) -> String {
    linhas.iter().map(|l| format!("data: {l}\n\n")).collect()
}

/// Servidor HTTP falso: responde na ordem do roteiro (repete o último quando acaba) e guarda os corpos recebidos.
pub struct Servidor {
    pub url_base: String,
    pedidos: Arc<Mutex<Vec<Value>>>,
}

impl Servidor {
    pub async fn iniciar(roteiro: Vec<Resposta>) -> Self {
        let ouvinte = TcpListener::bind("127.0.0.1:0").await.expect("porta livre");
        let porta = ouvinte.local_addr().expect("endereço").port();
        let pedidos = Arc::new(Mutex::new(Vec::new()));
        let fila = Arc::new(Mutex::new(VecDeque::from(roteiro)));
        let pedidos_do_servidor = Arc::clone(&pedidos);
        tokio::spawn(async move {
            while let Ok((conexao, _)) = ouvinte.accept().await {
                let pedidos = Arc::clone(&pedidos_do_servidor);
                let fila = Arc::clone(&fila);
                tokio::spawn(atender(conexao, pedidos, fila));
            }
        });
        Self {
            url_base: format!("http://127.0.0.1:{porta}/v1"),
            pedidos,
        }
    }

    pub fn pedidos(&self) -> Vec<Value> {
        self.pedidos.lock().expect("trava").clone()
    }
}

pub async fn atender(
    mut conexao: TcpStream,
    pedidos: Arc<Mutex<Vec<Value>>>,
    fila: Arc<Mutex<VecDeque<Resposta>>>,
) {
    let corpo = ler_requisicao(&mut conexao).await;
    pedidos
        .lock()
        .expect("trava")
        .push(serde_json::from_slice(&corpo).unwrap_or(Value::Null));

    let resposta = {
        let mut fila = fila.lock().expect("trava");
        if fila.len() > 1 {
            fila.pop_front().expect("há itens")
        } else {
            fila.front().cloned().expect("roteiro não vazio")
        }
    };
    if !resposta.atraso.is_zero() {
        tokio::time::sleep(resposta.atraso).await;
    }
    let cabecalho = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
        resposta.status,
        if resposta.status == 200 { "OK" } else { "ERRO" }
    );
    let _ = conexao.write_all(cabecalho.as_bytes()).await;
    let _ = conexao.write_all(resposta.corpo.as_bytes()).await;
    let _ = conexao.flush().await;
    if resposta.deixar_aberta {
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

/// Lê cabeçalhos e corpo (até `Content-Length`) de uma requisição HTTP simples.
pub async fn ler_requisicao(conexao: &mut TcpStream) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let inicio_do_corpo = loop {
        if let Some(posicao) = buf.windows(4).position(|janela| janela == b"\r\n\r\n") {
            break posicao + 4;
        }
        match conexao.read(&mut tmp).await {
            Ok(n) if n > 0 => buf.extend_from_slice(&tmp[..n]),
            _ => return Vec::new(),
        }
    };
    let cabecalho = String::from_utf8_lossy(&buf[..inicio_do_corpo]).to_ascii_lowercase();
    let tamanho = cabecalho
        .lines()
        .find_map(|linha| linha.strip_prefix("content-length:"))
        .and_then(|valor| valor.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buf.len() < inicio_do_corpo + tamanho {
        match conexao.read(&mut tmp).await {
            Ok(n) if n > 0 => buf.extend_from_slice(&tmp[..n]),
            _ => break,
        }
    }
    let fim = (inicio_do_corpo + tamanho).min(buf.len());
    buf[inicio_do_corpo..fim].to_vec()
}

pub struct Montagem {
    pub cliente: OpenAiCompatibleClient,
    pub limites: LimitadorLlm,
    pub metricas: BaseMetrics,
}

pub fn montar(url_base: &str, retentativas: u32, timeout: Duration) -> Montagem {
    montar_com_regra(url_base, retentativas, timeout, true)
}

pub fn montar_com_regra(
    url_base: &str,
    retentativas: u32,
    timeout: Duration,
    recusa_turno_modelo: bool,
) -> Montagem {
    let config = OpenAiCompatibleConfig {
        url_base: url_base.to_owned(),
        chave: Secret::new(CHAVE.to_owned()),
        modelo: "gemini-3.8-flash-tiered".to_owned(),
        timeout,
        retentativas,
        espera_base: Duration::from_millis(10),
        temperatura: 0.0,
        max_tokens_saida: 512,
        recusa_turno_modelo,
    };
    let limites = LimitadorLlm::new(4, 2).expect("limites válidos");
    let metricas = BaseMetrics::register(&Registry::new()).expect("catálogo");
    let cliente =
        OpenAiCompatibleClient::new(config, limites.clone(), metricas.clone()).expect("cliente");
    Montagem {
        cliente,
        limites,
        metricas,
    }
}

pub fn pedido_com(mensagens: Vec<Mensagem>) -> Pedido {
    Pedido::new(OrgId::generate(), mensagens)
}

pub fn pergunta(texto: &str) -> Pedido {
    pedido_com(vec![
        Mensagem::Sistema("Você é um assistente de atendimento.".to_owned()),
        Mensagem::Usuario(vec![Parte::Texto(texto.to_owned())]),
    ])
}

pub async fn colher(mut fluxo: FluxoLlm) -> Vec<Result<EventoLlm, ErroLlm>> {
    let mut itens = Vec::new();
    while let Some(item) = fluxo.next().await {
        itens.push(item);
    }
    itens
}

pub fn so_eventos(itens: Vec<Result<EventoLlm, ErroLlm>>) -> Vec<EventoLlm> {
    itens
        .into_iter()
        .map(|item| item.expect("evento sem erro"))
        .collect()
}

// ───────────────────────────── https: certificados e servidor TLS de teste (HR-10b) ─────────────────────────────

/// Certificados de teste do TLS (ver `tests/fixtures/tls/LEIAME.md`).
pub mod certificados {
    /// CA de teste: é a raiz extra que o cliente recebe no teste de https que deve funcionar.
    pub const CA: &[u8] = include_bytes!("../fixtures/tls/ca.der");
    /// Certificado de servidor para `IP:127.0.0.1`.
    pub const SERVIDOR_IP: &[u8] = include_bytes!("../fixtures/tls/servidor-ip.der");
    /// Certificado de servidor para `DNS:localhost` (usado no teste de nome errado).
    pub const SERVIDOR_NOME: &[u8] = include_bytes!("../fixtures/tls/servidor-nome.der");
    /// Chave privada (PKCS#8 DER) dos dois certificados de servidor.
    pub const SERVIDOR_CHAVE: &[u8] = include_bytes!("../fixtures/tls/servidor-chave.der");
}

/// Servidor HTTPS falso: mesma ideia do [`Servidor`], com handshake TLS de verdade.
pub struct ServidorTls {
    pub url_base: String,
    pedidos: Arc<Mutex<Vec<Value>>>,
    conexoes: Arc<AtomicUsize>,
}

impl ServidorTls {
    /// Sobe o servidor com o certificado dado (`certificado` em DER) e responde o roteiro.
    pub async fn iniciar(roteiro: Vec<Resposta>, certificado: &'static [u8]) -> Self {
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(certificado)],
                PrivateKeyDer::Pkcs8(certificados::SERVIDOR_CHAVE.into()),
            )
            .expect("configuração TLS de teste");
        let aceitador = TlsAcceptor::from(Arc::new(config));
        let ouvinte = TcpListener::bind("127.0.0.1:0").await.expect("porta livre");
        let porta = ouvinte.local_addr().expect("endereço").port();
        let pedidos = Arc::new(Mutex::new(Vec::new()));
        let conexoes = Arc::new(AtomicUsize::new(0));
        let fila = Arc::new(Mutex::new(VecDeque::from(roteiro)));
        let pedidos_do_servidor = Arc::clone(&pedidos);
        let conexoes_do_servidor = Arc::clone(&conexoes);
        tokio::spawn(async move {
            while let Ok((conexao, _)) = ouvinte.accept().await {
                conexoes_do_servidor.fetch_add(1, Ordering::SeqCst);
                let aceitador = aceitador.clone();
                let pedidos = Arc::clone(&pedidos_do_servidor);
                let fila = Arc::clone(&fila);
                tokio::spawn(async move {
                    // Handshake recusado pelo cliente é esperado em um dos testes: não é erro aqui.
                    let Ok(mut tls) = aceitador.accept(conexao).await else {
                        return;
                    };
                    atender_tls(&mut tls, pedidos, fila).await;
                });
            }
        });
        Self {
            url_base: format!("https://127.0.0.1:{porta}/v1"),
            pedidos,
            conexoes,
        }
    }

    pub fn pedidos(&self) -> Vec<Value> {
        self.pedidos.lock().expect("trava").clone()
    }

    /// Quantas conexões TCP o servidor aceitou (prova que não houve retentativa).
    pub fn conexoes(&self) -> usize {
        self.conexoes.load(Ordering::SeqCst)
    }
}

async fn atender_tls<S>(
    conexao: &mut S,
    pedidos: Arc<Mutex<Vec<Value>>>,
    fila: Arc<Mutex<VecDeque<Resposta>>>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let corpo = ler_requisicao_generica(conexao).await;
    pedidos
        .lock()
        .expect("trava")
        .push(serde_json::from_slice(&corpo).unwrap_or(Value::Null));
    let resposta = {
        let mut fila = fila.lock().expect("trava");
        if fila.len() > 1 {
            fila.pop_front().expect("há itens")
        } else {
            fila.front().cloned().expect("roteiro não vazio")
        }
    };
    if !resposta.atraso.is_zero() {
        tokio::time::sleep(resposta.atraso).await;
    }
    let cabecalho = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
        resposta.status,
        if resposta.status == 200 { "OK" } else { "ERRO" }
    );
    let _ = conexao.write_all(cabecalho.as_bytes()).await;
    let _ = conexao.write_all(resposta.corpo.as_bytes()).await;
    let _ = conexao.flush().await;
}

/// Igual a [`ler_requisicao`], mas para qualquer fluxo (TLS inclusive).
pub async fn ler_requisicao_generica<S>(conexao: &mut S) -> Vec<u8>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let inicio_do_corpo = loop {
        if let Some(posicao) = buf.windows(4).position(|janela| janela == b"\r\n\r\n") {
            break posicao + 4;
        }
        match conexao.read(&mut tmp).await {
            Ok(n) if n > 0 => buf.extend_from_slice(&tmp[..n]),
            _ => return Vec::new(),
        }
    };
    let cabecalho = String::from_utf8_lossy(&buf[..inicio_do_corpo]).to_ascii_lowercase();
    let tamanho = cabecalho
        .lines()
        .find_map(|linha| linha.strip_prefix("content-length:"))
        .and_then(|valor| valor.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buf.len() < inicio_do_corpo + tamanho {
        match conexao.read(&mut tmp).await {
            Ok(n) if n > 0 => buf.extend_from_slice(&tmp[..n]),
            _ => break,
        }
    }
    let fim = (inicio_do_corpo + tamanho).min(buf.len());
    buf[inicio_do_corpo..fim].to_vec()
}
