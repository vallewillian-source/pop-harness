//! Cliente HTTP do Jev (`POST /v1/systemone`) e transporte injetável para testes.
//!
//! Regras (as mesmas do cliente do LLM, HR-10):
//! - **Uma retentativa** só em 429, 5xx, queda de conexão ou prazo — nunca depois de a resposta
//!   começar a ser lida (aqui não há stream: é uma resposta só).
//! - **Prazo por tentativa** (`[jev].timeout_segundos`, padrão 8 s, PRD §8.5).
//! - **A chave nunca vaza**: `Debug` das opções e do pedido não a mostra, ela só entra no cabeçalho
//!   `Authorization`, e erro do provedor carrega o código HTTP, nunca o corpo (que pode ecoar a
//!   chave).
//! - **`http://` e `https://` são aceitos** (HR-10b): o TLS é `rustls` com as raízes do sistema.
//!   Falha de TLS (certificado não confiável, nome errado, handshake) é definitiva e não é
//!   retentada; queda de rede e prazo continuam transitórios.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use poph_types::{JevConfig, MetricasBase, Segredo};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};

use crate::tipos::{ErroJev, LoteResposta, Pergunta};

/// Espera antes da retentativa (o POP Atende usa 500 ms).
const ESPERA_RETENTATIVA: Duration = Duration::from_millis(500);

/// Parâmetros do cliente, já validados.
#[derive(Clone, Debug)]
pub struct ConfigJev {
    /// Endereço base, sem a barra final (ex.: `http://127.0.0.1:8050`).
    pub url_base: String,
    /// Modelo usado nas perguntas (`jev-latest`).
    pub modelo: String,
    /// Prazo de cada tentativa.
    pub timeout: Duration,
    /// Novas tentativas (0 ou 1; a configuração aceita até 3).
    pub retentativas: u32,
    /// Chave da API. Nunca aparece em log, erro ou `Debug`.
    chave: Segredo,
}

impl ConfigJev {
    /// Monta a configuração a partir do bloco `[jev]` e da chave lida do ambiente.
    pub fn da_config(jev: &JevConfig, chave: Segredo) -> Result<Self, ErroJev> {
        if chave.expor().trim().is_empty() {
            return Err(ErroJev::SemChave);
        }
        let url_base = jev.url_base.trim_end_matches('/').to_owned();
        // http (gateway local) e https (API real da TypeSafe) são aceitos; o TLS usa rustls com as
        // raízes do sistema (HR-10b).
        if !(url_base.starts_with("http://") || url_base.starts_with("https://")) {
            return Err(ErroJev::Configuracao(
                "o endereço do Jev deve começar com http:// ou https://".to_owned(),
            ));
        }
        if jev.modelo.trim().is_empty() {
            return Err(ErroJev::Configuracao("modelo do Jev vazio".to_owned()));
        }
        Ok(Self {
            url_base,
            modelo: jev.modelo.clone(),
            timeout: Duration::from_secs(u64::from(jev.timeout_segundos.max(1))),
            retentativas: jev.retentativas,
            chave,
        })
    }

    /// Troca o prazo por tentativa (teste de prazo e medição).
    pub fn com_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Endereço completo de uma pergunta.
    pub fn endpoint(&self) -> String {
        format!("{}/v1/systemone", self.url_base)
    }
}

/// Um pedido ao Jev: o estado, o modelo e as perguntas de todos os gates ligados.
#[derive(Debug, Clone, PartialEq)]
pub struct Pedido {
    /// Estado (conversa + política + capacidades).
    pub estado: Value,
    /// Modelo do Jev.
    pub modelo: String,
    /// Perguntas por id.
    pub perguntas: BTreeMap<String, Pergunta>,
}

impl Pedido {
    /// Corpo JSON do `POST /v1/systemone`.
    pub fn para_json(&self) -> Value {
        json!({
            "state": self.estado,
            "model": self.modelo,
            "questions": self.perguntas,
        })
    }
}

/// Quem entrega um pedido ao Jev. Existe como trait para o teste injetar um Jev falso local.
pub trait TransporteJev: Send + Sync {
    /// Faz a chamada e devolve as respostas por id.
    fn julgar<'a>(&'a self, pedido: Pedido) -> BoxFuture<'a, Result<LoteResposta, ErroJev>>;
}

struct Interno {
    http: reqwest::Client,
    endpoint: String,
    config: ConfigJev,
    metricas: MetricasBase,
}

/// Cliente HTTP do Jev. Clonar é barato (compartilha o `reqwest::Client`).
#[derive(Clone)]
pub struct ClienteJev(Arc<Interno>);

impl ClienteJev {
    /// Cria o cliente HTTP.
    pub fn novo(config: ConfigJev, metricas: MetricasBase) -> Result<Self, ErroJev> {
        let http = construir_http(&config, None)?;
        let endpoint = config.endpoint();
        Ok(Self(Arc::new(Interno {
            http,
            endpoint,
            config,
            metricas,
        })))
    }
}

impl ClienteJev {
    /// **Só para teste:** devolve um cliente igual a este que também confia numa raiz extra (DER).
    ///
    /// Serve para o servidor TLS local dos testes usar um certificado próprio. Não desliga a
    /// validação: é uma raiz a mais, e certificado, nome e cadeia continuam sendo conferidos.
    #[doc(hidden)]
    pub fn com_raiz_de_teste(self, raiz_der: &[u8]) -> Result<Self, ErroJev> {
        let http = construir_http(&self.0.config, Some(raiz_der))?;
        Ok(Self(Arc::new(Interno {
            http,
            endpoint: self.0.endpoint.clone(),
            config: self.0.config.clone(),
            metricas: self.0.metricas.clone(),
        })))
    }
}

/// Monta o cliente HTTP com o prazo configurado. Com `raiz_extra`, acrescenta uma raiz de confiança
/// (só teste).
fn construir_http(
    config: &ConfigJev,
    raiz_extra: Option<&[u8]>,
) -> Result<reqwest::Client, ErroJev> {
    let mut construtor = reqwest::Client::builder().timeout(config.timeout);
    if let Some(der) = raiz_extra {
        let certificado = reqwest::Certificate::from_der(der).map_err(|erro| {
            ErroJev::Configuracao(format!(
                "raiz de confiança inválida: {}",
                erro.without_url()
            ))
        })?;
        construtor = construtor.add_root_certificate(certificado);
    }
    construtor
        .build()
        .map_err(|erro| ErroJev::Rede(erro.without_url().to_string()))
}

impl TransporteJev for ClienteJev {
    fn julgar<'a>(&'a self, pedido: Pedido) -> BoxFuture<'a, Result<LoteResposta, ErroJev>> {
        Box::pin(async move {
            let interno = Arc::clone(&self.0);
            let corpo = serde_json::to_vec(&pedido.para_json()).map_err(|erro| {
                ErroJev::Resposta(format!("não consegui montar o corpo: {erro}"))
            })?;
            let mut ultimo: Option<ErroJev> = None;
            for tentativa in 0..=interno.config.retentativas {
                if tentativa > 0 {
                    tokio::time::sleep(ESPERA_RETENTATIVA).await;
                }
                interno.metricas.chamadas_jev.incrementar(1);
                match tentar(&interno, &corpo).await {
                    Ok(lote) => return Ok(lote),
                    Err(erro) => {
                        interno.metricas.erros_jev.incrementar(1);
                        if !erro.eh_transitorio() || tentativa == interno.config.retentativas {
                            return Err(erro);
                        }
                        tracing::debug!(erro = %erro, "Jev falhou; tentando de novo");
                        ultimo = Some(erro);
                    }
                }
            }
            Err(ultimo.unwrap_or_else(|| ErroJev::Rede("sem resposta do Jev".to_owned())))
        })
    }
}

async fn tentar(interno: &Interno, corpo: &[u8]) -> Result<LoteResposta, ErroJev> {
    let resposta = interno
        .http
        .post(&interno.endpoint)
        .header(CONTENT_TYPE, "application/json")
        .header(
            AUTHORIZATION,
            format!("Bearer {}", interno.config.chave.expor()),
        )
        .body(corpo.to_vec())
        .send()
        .await
        .map_err(|erro| {
            if erro.is_timeout() {
                return ErroJev::TempoEsgotado {
                    ms: u64::try_from(interno.config.timeout.as_millis()).unwrap_or(u64::MAX),
                };
            }
            // Falha de TLS é definitiva: repetir não conserta certificado nem nome errado.
            if let Some(detalhe) = detalhe_de_falha_tls(&erro) {
                return ErroJev::Tls(detalhe);
            }
            ErroJev::Rede(erro.without_url().to_string())
        })?;
    let codigo = resposta.status();
    if !codigo.is_success() {
        return Err(ErroJev::Status {
            codigo: codigo.as_u16(),
            motivo: codigo.canonical_reason().unwrap_or("sem motivo").to_owned(),
        });
    }
    let texto = resposta
        .text()
        .await
        .map_err(|erro| ErroJev::Rede(erro.without_url().to_string()))?;
    let corpo: Value = serde_json::from_str(&texto)
        .map_err(|erro| ErroJev::Resposta(format!("JSON inválido: {erro}")))?;
    LoteResposta::de_json(&corpo)
}

/// Marcadores (em minúsculas) de falha de TLS na cadeia de causas do erro do `reqwest`.
///
/// Mesma detecção do cliente do LLM (`crates/poph-core/src/llm/tls.rs`): o `rustls` não é dependência
/// deste crate, então o que distingue certificado recusado de porta fechada é o texto das causas.
const MARCAS_TLS: [&str; 10] = [
    "certificate",
    "certificado",
    "handshake",
    "tls",
    "unknownissuer",
    "notvalidfor",
    "invalid peer",
    "peer certificate",
    "invalidcertificate",
    "webpki",
];

/// Devolve o detalhe legível quando o erro é de TLS; `None` quando é outra falha de rede.
fn detalhe_de_falha_tls(erro: &dyn std::error::Error) -> Option<String> {
    let mut causas = Vec::new();
    let mut atual = Some(erro);
    while let Some(causa) = atual {
        causas.push(causa.to_string());
        atual = causa.source();
    }
    let marcada = |texto: &String| {
        let baixo = texto.to_ascii_lowercase();
        MARCAS_TLS.iter().any(|marca| baixo.contains(marca))
    };
    if !causas.iter().any(marcada) {
        return None;
    }
    let detalhe = causas
        .iter()
        .rev()
        .find(|texto| marcada(texto))
        .map(String::as_str)
        .unwrap_or("handshake TLS recusado");
    let sem_quebra: String = detalhe
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    Some(sem_quebra.trim().chars().take(200).collect())
}

// ───────────────────────────── Jev falso (testes e demonstração) ─────────────────────────────

/// Jev falso para teste e demonstração: conta chamadas, guarda pedidos e devolve o que foi roteirizado.
///
/// Nunca fala com a rede. É o que permite provar "3 gates ligados geram 1 requisição" sem chave real.
/// Função que responde um pedido no Jev falso.
type Roteiro = Box<dyn Fn(&Pedido) -> Result<LoteResposta, ErroJev> + Send + Sync>;

pub struct JevFalso {
    roteiro: Roteiro,
    atraso: Duration,
    chamadas: AtomicU64,
    pedidos: std::sync::Mutex<Vec<Pedido>>,
}

impl std::fmt::Debug for JevFalso {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JevFalso")
            .field("chamadas", &self.chamadas.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl JevFalso {
    /// Jev falso que responde com a função dada.
    pub fn novo(
        roteiro: impl Fn(&Pedido) -> Result<LoteResposta, ErroJev> + Send + Sync + 'static,
    ) -> Self {
        Self {
            roteiro: Box::new(roteiro),
            atraso: Duration::ZERO,
            chamadas: AtomicU64::new(0),
            pedidos: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Jev falso que responde cada pergunta com a escolha de mesmo nome no mapa, confiança 1,0.
    ///
    /// A chave do mapa é o id da pergunta (`g1_transferir`…); pergunta sem escolha no mapa volta
    /// sem `answers` e cai no fallback do gate.
    pub fn com_escolhas(escolhas: BTreeMap<String, String>) -> Self {
        Self::novo(move |pedido: &Pedido| {
            let mut respostas = BTreeMap::new();
            for id in pedido.perguntas.keys() {
                if let Some(escolha) = escolhas.get(id) {
                    let mut resposta = crate::tipos::Resposta {
                        escolha: Some(escolha.clone()),
                        confianca: 1.0,
                        ..crate::tipos::Resposta::default()
                    };
                    resposta.probabilidades.insert(escolha.clone(), 1.0);
                    respostas.insert(id.clone(), resposta);
                }
            }
            Ok(LoteResposta {
                respostas,
                tokens: 12,
                ..LoteResposta::default()
            })
        })
    }

    /// Jev falso que sempre falha com o erro dado.
    pub fn falhando(erro: ErroJev) -> Self {
        Self::novo(move |_| Err(erro.clone()))
    }

    /// Faz o Jev falso demorar antes de responder (para provar prazo e fallback).
    pub fn com_atraso(mut self, atraso: Duration) -> Self {
        self.atraso = atraso;
        self
    }

    /// Quantas chamadas o Jev falso recebeu.
    pub fn chamadas(&self) -> u64 {
        self.chamadas.load(Ordering::SeqCst)
    }

    /// Pedidos recebidos, em ordem.
    pub fn pedidos(&self) -> Vec<Pedido> {
        match self.pedidos.lock() {
            Ok(guarda) => guarda.clone(),
            Err(veneno) => veneno.into_inner().clone(),
        }
    }
}

impl TransporteJev for JevFalso {
    fn julgar<'a>(&'a self, pedido: Pedido) -> BoxFuture<'a, Result<LoteResposta, ErroJev>> {
        Box::pin(async move {
            if !self.atraso.is_zero() {
                tokio::time::sleep(self.atraso).await;
            }
            self.chamadas.fetch_add(1, Ordering::SeqCst);
            let resposta = (self.roteiro)(&pedido);
            match self.pedidos.lock() {
                Ok(mut guarda) => guarda.push(pedido),
                Err(veneno) => veneno.into_inner().push(pedido),
            }
            resposta
        })
    }
}
