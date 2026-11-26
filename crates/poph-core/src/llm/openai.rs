use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use futures::stream;
use poph_types::{BaseMetrics, LlmConfig, Secret};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};

use super::base64;
use super::erro::LlmError;
use super::limites::{LlmLimiter, LlmPermit};
use super::provedor::{LlmProvider, LlmStream, validate_request};
use super::sse::SseReader;
use super::tipos::{
    Cancellation, FinishReason, LlmEvent, Message, Part, Request, Tool, ToolCall, Usage,
};
use super::tls;

pub const OPENAI_COMPATIBLE_PROVIDER: &str = "openai_compativel";
pub const ANTIGRAVITY_PROVIDER: &str = "antigravity2api";

const MAXIMUM_WAIT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub struct OpenAiCompatibleConfig {
    pub url_base: String,
    pub chave: Secret,
    pub modelo: String,
    pub timeout: Duration,
    pub retentativas: u32,
    pub espera_base: Duration,
    pub temperatura: f32,
    pub max_tokens_saida: u32,
    pub recusa_turno_modelo: bool,
}

impl OpenAiCompatibleConfig {
    pub fn from_config(llm: &LlmConfig, chave: Secret) -> Result<Self, LlmError> {
        if !poph_types::llm_provider_accepted(&llm.provider) {
            return Err(LlmError::Configuracao(format!(
                "provedor {} não suportado; use {OPENAI_COMPATIBLE_PROVIDER} ou {ANTIGRAVITY_PROVIDER}",
                llm.provider
            )));
        }
        if chave.expose().trim().is_empty() {
            return Err(LlmError::Configuracao(
                "chave de API do provedor ausente".to_owned(),
            ));
        }
        let url_base = llm.url_base.trim_end_matches('/').to_owned();
        if !(url_base.starts_with("http://") || url_base.starts_with("https://")) {
            return Err(LlmError::Configuracao(
                "o endereço do provedor deve começar com http:// ou https://".to_owned(),
            ));
        }
        Ok(Self {
            url_base,
            chave,
            modelo: llm.model.clone(),
            timeout: Duration::from_secs(u64::from(llm.timeout_seconds)),
            retentativas: llm.retries,
            espera_base: Duration::from_secs(1),
            temperatura: llm.temperature,
            max_tokens_saida: llm.max_output_tokens,
            recusa_turno_modelo: llm.refuse_model_turn,
        })
    }
}

struct Inner {
    http: reqwest::Client,
    endpoint: String,
    config: OpenAiCompatibleConfig,
    limites: LlmLimiter,
    metricas: BaseMetrics,
}

#[derive(Clone)]
pub struct OpenAiCompatibleClient(Arc<Inner>);

impl OpenAiCompatibleClient {
    pub fn new(
        config: OpenAiCompatibleConfig,
        limites: LlmLimiter,
        metricas: BaseMetrics,
    ) -> Result<Self, LlmError> {
        let http = build_http(None)?;
        let endpoint = format!("{}/chat/completions", config.url_base);
        Ok(Self(Arc::new(Inner {
            http,
            endpoint,
            config,
            limites,
            metricas,
        })))
    }

    #[doc(hidden)]
    pub fn with_test_root(self, root_der: &[u8]) -> Result<Self, LlmError> {
        let http = build_http(Some(root_der))?;
        Ok(Self(Arc::new(Inner {
            http,
            endpoint: self.0.endpoint.clone(),
            config: self.0.config.clone(),
            limites: self.0.limites.clone(),
            metricas: self.0.metricas.clone(),
        })))
    }
}

fn build_http(extra_root: Option<&[u8]>) -> Result<reqwest::Client, LlmError> {
    let mut construtor = reqwest::Client::builder();
    if let Some(der) = extra_root {
        let certificate = reqwest::Certificate::from_der(der).map_err(|erro| {
            LlmError::Configuracao(format!(
                "raiz de confiança inválida: {}",
                erro.without_url()
            ))
        })?;
        construtor = construtor.add_root_certificate(certificate);
    }
    construtor
        .build()
        .map_err(|erro| LlmError::Configuracao(format!("cliente HTTP: {}", erro.without_url())))
}

impl LlmProvider for OpenAiCompatibleClient {
    fn stream<'a>(&'a self, pedido: Request) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(abrir(Arc::clone(&self.0), pedido))
    }
}

async fn abrir(inner: Arc<Inner>, pedido: Request) -> Result<LlmStream, LlmError> {
    validate_request(&pedido, inner.config.recusa_turno_modelo)
        .map_err(|erro| inner.record_failure(erro))?;
    inner.metricas.llm_calls.increment(1);

    let cancelamento = pedido.cancelamento.clone();
    let permit: LlmPermit = tokio::select! {
        biased;
        _ = cancelamento.wait() => return Err(LlmError::Cancelled),
        resultado = inner.limites.acquire(pedido.org) => {
            resultado.map_err(|erro| inner.record_failure(erro))?
        }
    };

    let corpo = build_body(&pedido, &inner.config);
    let modelo = pedido
        .modelo
        .clone()
        .unwrap_or_else(|| inner.config.modelo.clone());
    let inicio = Instant::now();
    let response = send(&inner, &corpo, &cancelamento)
        .await
        .map_err(|erro| inner.record_failure(erro))?;

    tracing::debug!(modelo = %modelo, "resposta do LLM começou a chegar");
    let estado = Estado {
        inner: Arc::clone(&inner),
        resposta: response,
        reader: SseReader::new(),
        parciais: BTreeMap::new(),
        pending: VecDeque::new(),
        motivo: None,
        usage: None,
        closed: false,
        permit: Some(permit),
        cancelamento,
        inicio,
        modelo,
        org: pedido.org,
    };
    Ok(Box::pin(stream::unfold(estado, advance)))
}

impl Inner {
    fn record_failure(&self, erro: LlmError) -> LlmError {
        if erro != LlmError::Cancelled {
            self.metricas.llm_errors.increment(1);
            tracing::warn!(erro = %erro, "chamada ao LLM falhou");
        }
        erro
    }
}

async fn send(
    inner: &Inner,
    corpo: &Value,
    cancelamento: &Cancellation,
) -> Result<reqwest::Response, LlmError> {
    let total = inner.config.retentativas + 1;
    let body_text = corpo.to_string();
    let mut ultimo = String::from("sem resposta");

    for tentativa in 1..=total {
        let sending = inner
            .http
            .post(&inner.endpoint)
            .timeout(inner.config.timeout)
            .header(
                AUTHORIZATION,
                format!("Bearer {}", inner.config.chave.expose()),
            )
            .header(CONTENT_TYPE, "application/json")
            .body(body_text.clone())
            .send();

        let resultado = tokio::select! {
            biased;
            _ = cancelamento.wait() => return Err(LlmError::Cancelled),
            resultado = sending => resultado,
        };

        match resultado {
            Ok(resposta) if resposta.status().is_success() => return Ok(resposta),
            Ok(resposta) => {
                let status = resposta.status().as_u16();
                let texto = resposta.text().await.unwrap_or_default();
                if status == 429 || (500..=599).contains(&status) {
                    ultimo = format!("HTTP {status}");
                } else {
                    return Err(LlmError::Rejected {
                        status,
                        mensagem: summarize_error(&texto, inner.config.chave.expose()),
                    });
                }
            }
            Err(erro) if erro.is_timeout() => {
                return Err(LlmError::TimedOut {
                    segundos: inner.config.timeout.as_secs(),
                });
            }
            Err(erro) => {
                if let Some(detail) = tls::tls_failure_detail(&erro) {
                    return Err(LlmError::Tls(detail));
                }
                ultimo = if erro.is_connect() {
                    "conexão recusada".to_owned()
                } else {
                    "falha de rede".to_owned()
                };
            }
        }

        if tentativa < total {
            let espera = retry_wait(inner.config.espera_base, tentativa);
            tracing::info!(tentativa, motivo = %ultimo, "nova tentativa no LLM");
            let seguir = tokio::select! {
                biased;
                _ = cancelamento.wait() => false,
                _ = tokio::time::sleep(espera) => true,
            };
            if !seguir {
                return Err(LlmError::Cancelled);
            }
        }
    }
    Err(LlmError::Unavailable {
        tentativas: total,
        ultimo,
    })
}

fn retry_wait(base: Duration, tentativa: u32) -> Duration {
    let factor = 1u32 << (tentativa - 1).min(10);
    base.saturating_mul(factor).min(MAXIMUM_WAIT)
}

fn summarize_error(texto: &str, chave: &str) -> String {
    let mensagem = serde_json::from_str::<Value>(texto)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| texto.to_owned());
    let mut limpo: String = mensagem.chars().take(300).collect();
    if !chave.is_empty() {
        limpo = limpo.replace(chave, "[oculto]");
    }
    limpo
}

pub fn build_body(pedido: &Request, config: &OpenAiCompatibleConfig) -> Value {
    let modelo = pedido.modelo.as_deref().unwrap_or(&config.modelo);
    let messages: Vec<Value> = pedido.mensagens.iter().map(message_json).collect();
    let temperatura = pedido.temperatura.unwrap_or(config.temperatura);
    let max_tokens = pedido.max_tokens.unwrap_or(config.max_tokens_saida);
    let mut corpo = json!({
        "model": modelo,
        "stream": true,
        "temperature": temperatura,
        "max_tokens": max_tokens,
        "messages": messages,
    });
    if !pedido.tools.is_empty() {
        corpo["tools"] = Value::Array(pedido.tools.iter().map(tool_json).collect());
    }
    corpo
}

fn message_json(mensagem: &Message) -> Value {
    match mensagem {
        Message::Sistema(texto) => json!({"role": "system", "content": texto}),
        Message::Usuario(parts) => {
            if let [Part::Texto(texto)] = parts.as_slice() {
                return json!({"role": "user", "content": texto});
            }
            let conteudo: Vec<Value> = parts.iter().map(part_json).collect();
            json!({"role": "user", "content": conteudo})
        }
        Message::Assistente {
            texto,
            chamadas: calls,
        } => {
            let mut obj = json!({"role": "assistant"});
            obj["content"] = if texto.is_empty() && !calls.is_empty() {
                Value::Null
            } else {
                Value::String(texto.clone())
            };
            if !calls.is_empty() {
                obj["tool_calls"] = Value::Array(
                    calls
                        .iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "type": "function",
                                "function": {"name": c.nome, "arguments": c.arguments.to_string()},
                            })
                        })
                        .collect(),
                );
            }
            obj
        }
        Message::Resultado { id, conteudo } => {
            json!({"role": "tool", "tool_call_id": id, "content": conteudo})
        }
    }
}

fn part_json(parte: &Part) -> Value {
    match parte {
        Part::Texto(texto) => json!({"type": "text", "text": texto}),
        Part::Imagem { mime, bytes } => json!({
            "type": "image_url",
            "image_url": {"url": format!("data:{mime};base64,{}", base64::encode(bytes))},
        }),
    }
}

fn tool_json(tool: &Tool) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.nome,
            "description": tool.descricao,
            "parameters": tool.parametros,
        },
    })
}

#[derive(Default)]
struct Partial {
    id: Option<String>,
    nome: Option<String>,
    argumentos: String,
}

struct Estado {
    inner: Arc<Inner>,
    resposta: reqwest::Response,
    reader: SseReader,
    parciais: BTreeMap<u64, Partial>,
    pending: VecDeque<Result<LlmEvent, LlmError>>,
    motivo: Option<FinishReason>,
    usage: Option<Usage>,
    closed: bool,
    permit: Option<LlmPermit>,
    cancelamento: Cancellation,
    inicio: Instant,
    modelo: String,
    org: poph_types::OrgId,
}

async fn advance(mut e: Estado) -> Option<(Result<LlmEvent, LlmError>, Estado)> {
    loop {
        if let Some(item) = e.pending.pop_front() {
            if item.is_err() {
                e.closed = true;
                e.permit = None;
            }
            return Some((item, e));
        }
        if e.closed {
            return None;
        }

        let chunk = tokio::select! {
            biased;
            _ = e.cancelamento.wait() => Err(LlmError::Cancelled),
            pedaco = e.resposta.chunk() => pedaco.map_err(|erro| {
                if erro.is_timeout() {
                    LlmError::TimedOut { segundos: e.inner.config.timeout.as_secs() }
                } else {
                    LlmError::ConnectionInterrupted("a conexão caiu no meio da resposta".to_owned())
                }
            }),
        };

        match chunk {
            Err(erro) => {
                e.closed = true;
                e.permit = None;
                let erro = e.inner.record_failure(erro);
                e.pending.push_back(Err(erro));
            }
            Ok(Some(bytes)) => {
                for payload in e.reader.push(&bytes) {
                    if e.closed {
                        break;
                    }
                    process_payload(&mut e, &payload);
                }
            }
            Ok(None) => {
                for payload in e.reader.finish() {
                    if e.closed {
                        break;
                    }
                    process_payload(&mut e, &payload);
                }
                if !e.closed {
                    if e.motivo.is_none() {
                        let erro = e.inner.record_failure(LlmError::InvalidResponse(
                            "a resposta terminou antes do fim (conexão fechada sem motivo de fim)"
                                .to_owned(),
                        ));
                        e.pending.push_back(Err(erro));
                        e.closed = true;
                        e.permit = None;
                    } else {
                        finish_response(&mut e);
                    }
                }
            }
        }
    }
}

fn process_payload(e: &mut Estado, payload: &str) {
    if payload == "[DONE]" {
        finish_response(e);
        return;
    }
    let Ok(valor) = serde_json::from_str::<Value>(payload) else {
        tracing::debug!("evento do stream do LLM sem JSON válido, ignorado");
        return;
    };
    if !valor["error"].is_null() {
        let mensagem = valor["error"]["message"]
            .as_str()
            .map(|m| summarize_error(m, e.inner.config.chave.expose()))
            .unwrap_or_else(|| "erro sem detalhe".to_owned());
        let erro = e.inner.record_failure(LlmError::StreamError(mensagem));
        e.pending.push_back(Err(erro));
        e.closed = true;
        e.permit = None;
        return;
    }
    if let Some(uso) = valor.get("usage").filter(|u| !u.is_null()) {
        e.usage = Some(Usage {
            entrada: uso["prompt_tokens"].as_u64().unwrap_or(0),
            saida: uso["completion_tokens"].as_u64().unwrap_or(0),
        });
    }
    let escolha = &valor["choices"][0];
    let delta = &escolha["delta"];
    if let Some(texto) = delta["content"].as_str().filter(|t| !t.is_empty()) {
        e.pending.push_back(Ok(LlmEvent::Texto(texto.to_owned())));
    }
    if let Some(chamadas) = delta["tool_calls"].as_array() {
        for call in chamadas {
            let index = call["index"].as_u64().unwrap_or(0);
            let partial = e.parciais.entry(index).or_default();
            if let Some(id) = call["id"].as_str().filter(|s| !s.is_empty()) {
                partial.id = Some(id.to_owned());
            }
            if let Some(nome) = call["function"]["name"].as_str().filter(|s| !s.is_empty()) {
                partial.nome = Some(nome.to_owned());
            }
            if let Some(pedaco) = call["function"]["arguments"].as_str() {
                partial.argumentos.push_str(pedaco);
            }
        }
    }
    if let Some(motivo) = escolha["finish_reason"].as_str() {
        e.motivo = Some(match motivo {
            "stop" => FinishReason::Concluido,
            "tool_calls" | "function_call" => FinishReason::ChamadasFerramenta,
            "length" => FinishReason::LimiteDeTokens,
            outro => FinishReason::Outro(outro.to_owned()),
        });
    }
}

fn finish_response(e: &mut Estado) {
    if e.closed {
        return;
    }
    e.closed = true;
    e.permit = None;

    let parciais = std::mem::take(&mut e.parciais);
    let mut total_ferramentas = 0usize;
    for (indice, partial) in parciais {
        let Some(nome) = partial.nome else {
            let erro = e.inner.record_failure(LlmError::InvalidResponse(
                "chamada de ferramenta sem nome".to_owned(),
            ));
            e.pending.push_back(Err(erro));
            return;
        };
        let argumentos = if partial.argumentos.trim().is_empty() {
            json!({})
        } else {
            match serde_json::from_str::<Value>(&partial.argumentos) {
                Ok(valor) => valor,
                Err(_) => {
                    let erro = e.inner.record_failure(LlmError::InvalidResponse(format!(
                        "argumentos da ferramenta {nome} não são JSON válido"
                    )));
                    e.pending.push_back(Err(erro));
                    return;
                }
            }
        };
        let id = partial.id.unwrap_or_else(|| format!("chamada_{indice}"));
        total_ferramentas += 1;
        e.pending.push_back(Ok(LlmEvent::Tool(ToolCall {
            id,
            nome,
            arguments: argumentos,
        })));
    }

    if let Some(uso) = e.usage {
        e.inner.metricas.tokens_total.increment(uso.total());
        e.pending.push_back(Ok(LlmEvent::Usage(uso)));
    }
    let motivo = match e.motivo.take() {
        Some(motivo) => motivo,
        None if total_ferramentas > 0 => FinishReason::ChamadasFerramenta,
        None => FinishReason::Concluido,
    };
    e.pending.push_back(Ok(LlmEvent::Fim(motivo)));

    tracing::info!(
        org = %e.org,
        modelo = %e.modelo,
        ferramentas = total_ferramentas,
        duracao_ms = e.inicio.elapsed().as_millis() as u64,
        "resposta do LLM concluída"
    );
}

#[cfg(test)]
mod testes {
    use poph_types::OrgId;

    use super::*;

    fn config(url: &str, chave: &str) -> Result<OpenAiCompatibleConfig, LlmError> {
        let llm = LlmConfig {
            provider: OPENAI_COMPATIBLE_PROVIDER.to_owned(),
            url_base: url.to_owned(),
            api_key_env: "ANTIGRAVITY_API_KEY".to_owned(),
            refuse_model_turn: true,
            model: "gemini-3.8-flash-tiered".to_owned(),
            timeout_seconds: 10,
            retries: 2,
            concurrent_calls: 4,
            temperature: 0.0,
            max_output_tokens: 512,
        };
        OpenAiCompatibleConfig::from_config(&llm, Secret::new(chave.to_owned()))
    }

    #[test]
    fn accepts_https_rejects_invalid_address_and_empty_key() {
        let https = config("https://exemplo.test/v1", "k").expect("https aceito");
        assert_eq!(https.url_base, "https://exemplo.test/v1");
        let sem_esquema = config("exemplo.test/v1", "k").unwrap_err();
        assert!(
            matches!(sem_esquema, LlmError::Configuracao(m) if m.contains("http:// ou https://"))
        );
        assert!(config("http://127.0.0.1:8045/v1/", "").is_err());
        let llm = LlmConfig {
            provider: "outro".to_owned(),
            url_base: "http://127.0.0.1:8045/v1".to_owned(),
            api_key_env: "ANTIGRAVITY_API_KEY".to_owned(),
            refuse_model_turn: true,
            model: "m".to_owned(),
            timeout_seconds: 10,
            retries: 0,
            concurrent_calls: 1,
            temperature: 0.0,
            max_output_tokens: 1,
        };
        assert!(OpenAiCompatibleConfig::from_config(&llm, Secret::new("k".into())).is_err());
    }

    #[test]
    fn monta_corpo_com_imagem_ferramenta_e_modelo_do_pedido() {
        let cfg = config("http://127.0.0.1:8045/v1", "segredo-de-teste").expect("config");
        let mut pedido = Request::new(
            OrgId::generate(),
            vec![
                Message::Sistema("instruções".into()),
                Message::Usuario(vec![
                    Part::Texto("o que tem na foto?".into()),
                    Part::Imagem {
                        mime: "image/png".into(),
                        bytes: b"foo".to_vec(),
                    },
                ]),
            ],
        );
        pedido.modelo = Some("outro-modelo".into());
        pedido.tools = vec![Tool {
            nome: "buscar".into(),
            descricao: "busca".into(),
            parametros: json!({"type": "object"}),
        }];
        let corpo = build_body(&pedido, &cfg);
        assert_eq!(corpo["model"], "outro-modelo");
        assert_eq!(corpo["stream"], true);
        assert_eq!(corpo["messages"][0]["role"], "system");
        let partes = &corpo["messages"][1]["content"];
        assert_eq!(partes[1]["image_url"]["url"], "data:image/png;base64,Zm9v");
        assert_eq!(corpo["tools"][0]["function"]["name"], "buscar");
        assert!(!corpo.to_string().contains("segredo-de-teste"));
    }

    #[test]
    fn wait_doubles_with_a_cap() {
        let base = Duration::from_secs(1);
        assert_eq!(retry_wait(base, 1), Duration::from_secs(1));
        assert_eq!(retry_wait(base, 2), Duration::from_secs(2));
        assert_eq!(retry_wait(base, 3), Duration::from_secs(4));
        assert_eq!(retry_wait(base, 30), MAXIMUM_WAIT);
    }

    #[test]
    fn provider_error_does_not_echo_key() {
        let corpo = r#"{"error":{"message":"chave inválida: abc-123-segredo"}}"#;
        let texto = summarize_error(corpo, "abc-123-segredo");
        assert!(!texto.contains("abc-123-segredo"));
        assert!(texto.contains("[oculto]"));
    }
}
