use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use futures::stream;
use poph_types::{BaseMetrics, LlmConfig, Secret};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};

use super::base64;
use super::error::LlmError;
use super::limites::{LlmLimiter, LlmPermit};
use super::provedor::{LlmProvider, LlmStream, validate_request};
use super::sse::SseReader;
use super::tls;
use super::types::{
    Cancellation, FinishReason, LlmEvent, Message, Part, Request, Tool, ToolCall, Usage,
};

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
    pub base_wait: Duration,
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
            base_wait: Duration::from_secs(1),
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
    metrics: BaseMetrics,
}

#[derive(Clone)]
pub struct OpenAiCompatibleClient(Arc<Inner>);

impl OpenAiCompatibleClient {
    pub fn new(
        config: OpenAiCompatibleConfig,
        limites: LlmLimiter,
        metrics: BaseMetrics,
    ) -> Result<Self, LlmError> {
        let http = build_http(None)?;
        let endpoint = format!("{}/chat/completions", config.url_base);
        Ok(Self(Arc::new(Inner {
            http,
            endpoint,
            config,
            limites,
            metrics,
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
            metrics: self.0.metrics.clone(),
        })))
    }
}

fn build_http(extra_root: Option<&[u8]>) -> Result<reqwest::Client, LlmError> {
    let mut construtor = reqwest::Client::builder();
    if let Some(der) = extra_root {
        let certificate = reqwest::Certificate::from_der(der).map_err(|error| {
            LlmError::Configuracao(format!(
                "raiz de confiança inválida: {}",
                error.without_url()
            ))
        })?;
        construtor = construtor.add_root_certificate(certificate);
    }
    construtor
        .build()
        .map_err(|error| LlmError::Configuracao(format!("cliente HTTP: {}", error.without_url())))
}

impl LlmProvider for OpenAiCompatibleClient {
    fn stream<'a>(&'a self, request: Request) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(open_stream(Arc::clone(&self.0), request))
    }
}

async fn open_stream(inner: Arc<Inner>, request: Request) -> Result<LlmStream, LlmError> {
    validate_request(&request, inner.config.recusa_turno_modelo)
        .map_err(|error| inner.record_failure(error))?;
    inner.metrics.llm_calls.increment(1);

    let cancellation = request.cancellation.clone();
    let permit: LlmPermit = tokio::select! {
        biased;
        _ = cancellation.wait() => return Err(LlmError::Cancelled),
        resultado = inner.limites.acquire(request.org) => {
            resultado.map_err(|error| inner.record_failure(error))?
        }
    };

    let body = build_body(&request, &inner.config);
    let modelo = request
        .modelo
        .clone()
        .unwrap_or_else(|| inner.config.modelo.clone());
    let started = Instant::now();
    let response = send(&inner, &body, &cancellation)
        .await
        .map_err(|error| inner.record_failure(error))?;

    tracing::debug!(modelo = %modelo, "resposta do LLM começou a chegar");
    let estado = Estado {
        inner: Arc::clone(&inner),
        response,
        reader: SseReader::new(),
        partials: BTreeMap::new(),
        pending: VecDeque::new(),
        motivo: None,
        usage: None,
        closed: false,
        permit: Some(permit),
        cancellation,
        started,
        modelo,
        org: request.org,
    };
    Ok(Box::pin(stream::unfold(estado, advance)))
}

impl Inner {
    fn record_failure(&self, error: LlmError) -> LlmError {
        if error != LlmError::Cancelled {
            self.metrics.llm_errors.increment(1);
            tracing::warn!(error = %error, "chamada ao LLM falhou");
        }
        error
    }
}

async fn send(
    inner: &Inner,
    body: &Value,
    cancellation: &Cancellation,
) -> Result<reqwest::Response, LlmError> {
    let total = inner.config.retentativas + 1;
    let body_text = body.to_string();
    let mut last_error = String::from("sem resposta");

    for attempt in 1..=total {
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
            _ = cancellation.wait() => return Err(LlmError::Cancelled),
            resultado = sending => resultado,
        };

        match resultado {
            Ok(response) if response.status().is_success() => return Ok(response),
            Ok(response) => {
                let status = response.status().as_u16();
                let texto = response.text().await.unwrap_or_default();
                if status == 429 || (500..=599).contains(&status) {
                    last_error = format!("HTTP {status}");
                } else {
                    return Err(LlmError::Rejected {
                        status,
                        mensagem: summarize_error(&texto, inner.config.chave.expose()),
                    });
                }
            }
            Err(error) if error.is_timeout() => {
                return Err(LlmError::TimedOut {
                    seconds: inner.config.timeout.as_secs(),
                });
            }
            Err(error) => {
                if let Some(detail) = tls::tls_failure_detail(&error) {
                    return Err(LlmError::Tls(detail));
                }
                last_error = if error.is_connect() {
                    "conexão recusada".to_owned()
                } else {
                    "falha de rede".to_owned()
                };
            }
        }

        if attempt < total {
            let wait = retry_wait(inner.config.base_wait, attempt);
            tracing::info!(attempt, motivo = %last_error, "nova tentativa no LLM");
            let seguir = tokio::select! {
                biased;
                _ = cancellation.wait() => false,
                _ = tokio::time::sleep(wait) => true,
            };
            if !seguir {
                return Err(LlmError::Cancelled);
            }
        }
    }
    Err(LlmError::Unavailable {
        tentativas: total,
        last_error,
    })
}

fn retry_wait(base: Duration, attempt: u32) -> Duration {
    let factor = 1u32 << (attempt - 1).min(10);
    base.saturating_mul(factor).min(MAXIMUM_WAIT)
}

fn summarize_error(texto: &str, chave: &str) -> String {
    let mensagem = serde_json::from_str::<Value>(texto)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| texto.to_owned());
    let mut sanitized: String = mensagem.chars().take(300).collect();
    if !chave.is_empty() {
        sanitized = sanitized.replace(chave, "[oculto]");
    }
    sanitized
}

pub fn build_body(request: &Request, config: &OpenAiCompatibleConfig) -> Value {
    let modelo = request.modelo.as_deref().unwrap_or(&config.modelo);
    let messages: Vec<Value> = request.messages.iter().map(message_json).collect();
    let temperatura = request.temperatura.unwrap_or(config.temperatura);
    let max_tokens = request.max_tokens.unwrap_or(config.max_tokens_saida);
    let mut body = json!({
        "model": modelo,
        "stream": true,
        "temperature": temperatura,
        "max_tokens": max_tokens,
        "messages": messages,
    });
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(request.tools.iter().map(tool_json).collect());
    }
    body
}

fn message_json(mensagem: &Message) -> Value {
    match mensagem {
        Message::System(texto) => json!({"role": "system", "content": texto}),
        Message::User(parts) => {
            if let [Part::Texto(texto)] = parts.as_slice() {
                return json!({"role": "user", "content": texto});
            }
            let content: Vec<Value> = parts.iter().map(part_json).collect();
            json!({"role": "user", "content": content})
        }
        Message::Assistant { texto, calls } => {
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
        Message::Resultado { id, content } => {
            json!({"role": "tool", "tool_call_id": id, "content": content})
        }
    }
}

fn part_json(part: &Part) -> Value {
    match part {
        Part::Texto(texto) => json!({"type": "text", "text": texto}),
        Part::Image { mime, bytes } => json!({
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
            "parameters": tool.parameters,
        },
    })
}

#[derive(Default)]
struct Partial {
    id: Option<String>,
    nome: Option<String>,
    arguments: String,
}

struct Estado {
    inner: Arc<Inner>,
    response: reqwest::Response,
    reader: SseReader,
    partials: BTreeMap<u64, Partial>,
    pending: VecDeque<Result<LlmEvent, LlmError>>,
    motivo: Option<FinishReason>,
    usage: Option<Usage>,
    closed: bool,
    permit: Option<LlmPermit>,
    cancellation: Cancellation,
    started: Instant,
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
            _ = e.cancellation.wait() => Err(LlmError::Cancelled),
            chunk = e.response.chunk() => chunk.map_err(|error| {
                if error.is_timeout() {
                    LlmError::TimedOut { seconds: e.inner.config.timeout.as_secs() }
                } else {
                    LlmError::ConnectionInterrupted("a conexão caiu no meio da resposta".to_owned())
                }
            }),
        };

        match chunk {
            Err(error) => {
                e.closed = true;
                e.permit = None;
                let error = e.inner.record_failure(error);
                e.pending.push_back(Err(error));
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
                        let error = e.inner.record_failure(LlmError::InvalidResponse(
                            "a resposta terminou antes do fim (conexão fechada sem motivo de fim)"
                                .to_owned(),
                        ));
                        e.pending.push_back(Err(error));
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
    let Ok(value) = serde_json::from_str::<Value>(payload) else {
        tracing::debug!("evento do stream do LLM sem JSON válido, ignorado");
        return;
    };
    if !value["error"].is_null() {
        let mensagem = value["error"]["message"]
            .as_str()
            .map(|m| summarize_error(m, e.inner.config.chave.expose()))
            .unwrap_or_else(|| "erro sem detalhe".to_owned());
        let error = e.inner.record_failure(LlmError::StreamError(mensagem));
        e.pending.push_back(Err(error));
        e.closed = true;
        e.permit = None;
        return;
    }
    if let Some(usage) = value.get("usage").filter(|u| !u.is_null()) {
        e.usage = Some(Usage {
            input: usage["prompt_tokens"].as_u64().unwrap_or(0),
            saida: usage["completion_tokens"].as_u64().unwrap_or(0),
        });
    }
    let choice = &value["choices"][0];
    let delta = &choice["delta"];
    if let Some(texto) = delta["content"].as_str().filter(|t| !t.is_empty()) {
        e.pending.push_back(Ok(LlmEvent::Texto(texto.to_owned())));
    }
    if let Some(calls) = delta["tool_calls"].as_array() {
        for call in calls {
            let index = call["index"].as_u64().unwrap_or(0);
            let partial = e.partials.entry(index).or_default();
            if let Some(id) = call["id"].as_str().filter(|s| !s.is_empty()) {
                partial.id = Some(id.to_owned());
            }
            if let Some(nome) = call["function"]["name"].as_str().filter(|s| !s.is_empty()) {
                partial.nome = Some(nome.to_owned());
            }
            if let Some(chunk) = call["function"]["arguments"].as_str() {
                partial.arguments.push_str(chunk);
            }
        }
    }
    if let Some(motivo) = choice["finish_reason"].as_str() {
        e.motivo = Some(match motivo {
            "stop" => FinishReason::Completed,
            "tool_calls" | "function_call" => FinishReason::ToolCalls,
            "length" => FinishReason::TokenLimit,
            other => FinishReason::Other(other.to_owned()),
        });
    }
}

fn finish_response(e: &mut Estado) {
    if e.closed {
        return;
    }
    e.closed = true;
    e.permit = None;

    let partials = std::mem::take(&mut e.partials);
    let mut total_tools = 0usize;
    for (index, partial) in partials {
        let Some(nome) = partial.nome else {
            let error = e.inner.record_failure(LlmError::InvalidResponse(
                "chamada de ferramenta sem nome".to_owned(),
            ));
            e.pending.push_back(Err(error));
            return;
        };
        let arguments = if partial.arguments.trim().is_empty() {
            json!({})
        } else {
            match serde_json::from_str::<Value>(&partial.arguments) {
                Ok(value) => value,
                Err(_) => {
                    let error = e.inner.record_failure(LlmError::InvalidResponse(format!(
                        "argumentos da ferramenta {nome} não são JSON válido"
                    )));
                    e.pending.push_back(Err(error));
                    return;
                }
            }
        };
        let id = partial.id.unwrap_or_else(|| format!("chamada_{index}"));
        total_tools += 1;
        e.pending.push_back(Ok(LlmEvent::Tool(ToolCall {
            id,
            nome,
            arguments,
        })));
    }

    if let Some(usage) = e.usage {
        e.inner.metrics.tokens_total.increment(usage.total());
        e.pending.push_back(Ok(LlmEvent::Usage(usage)));
    }
    let motivo = match e.motivo.take() {
        Some(motivo) => motivo,
        None if total_tools > 0 => FinishReason::ToolCalls,
        None => FinishReason::Completed,
    };
    e.pending.push_back(Ok(LlmEvent::Fim(motivo)));

    tracing::info!(
        org = %e.org,
        modelo = %e.modelo,
        tools = total_tools,
        duration_ms = e.started.elapsed().as_millis() as u64,
        "resposta do LLM concluída"
    );
}

#[cfg(test)]
mod tests {
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
        let missing_scheme = config("exemplo.test/v1", "k").unwrap_err();
        assert!(
            matches!(missing_scheme, LlmError::Configuracao(m) if m.contains("http:// ou https://"))
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
    fn builds_body_with_image_tool_and_requested_model() {
        let cfg = config("http://127.0.0.1:8045/v1", "segredo-de-teste").expect("config");
        let mut request = Request::new(
            OrgId::generate(),
            vec![
                Message::System("instruções".into()),
                Message::User(vec![
                    Part::Texto("o que tem na foto?".into()),
                    Part::Image {
                        mime: "image/png".into(),
                        bytes: b"foo".to_vec(),
                    },
                ]),
            ],
        );
        request.modelo = Some("outro-modelo".into());
        request.tools = vec![Tool {
            nome: "buscar".into(),
            descricao: "busca".into(),
            parameters: json!({"type": "object"}),
        }];
        let body = build_body(&request, &cfg);
        assert_eq!(body["model"], "outro-modelo");
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"][0]["role"], "system");
        let parts = &body["messages"][1]["content"];
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,Zm9v");
        assert_eq!(body["tools"][0]["function"]["name"], "buscar");
        assert!(!body.to_string().contains("segredo-de-teste"));
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
        let body = r#"{"error":{"message":"chave inválida: abc-123-segredo"}}"#;
        let texto = summarize_error(body, "abc-123-segredo");
        assert!(!texto.contains("abc-123-segredo"));
        assert!(texto.contains("[oculto]"));
    }
}
