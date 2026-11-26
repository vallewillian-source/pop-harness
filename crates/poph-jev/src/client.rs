use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use poph_types::{BaseMetrics, JevConfig, Secret};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};

use crate::types::{BatchResponse, JevError, Question};

const RETRY_WAIT: Duration = Duration::from_millis(500);

#[derive(Clone, Debug)]
pub struct JevClientConfig {
    pub url_base: String,
    pub modelo: String,
    pub timeout: Duration,
    pub retentativas: u32,
    chave: Secret,
}

impl JevClientConfig {
    pub fn from_config(jev: &JevConfig, chave: Secret) -> Result<Self, JevError> {
        if chave.expose().trim().is_empty() {
            return Err(JevError::MissingKey);
        }
        let url_base = jev.url_base.trim_end_matches('/').to_owned();
        if !(url_base.starts_with("http://") || url_base.starts_with("https://")) {
            return Err(JevError::InvalidConfiguration(
                "o endereço do Jev deve começar com http:// ou https://".to_owned(),
            ));
        }
        if jev.model.trim().is_empty() {
            return Err(JevError::InvalidConfiguration(
                "modelo do Jev vazio".to_owned(),
            ));
        }
        Ok(Self {
            url_base,
            modelo: jev.model.clone(),
            timeout: Duration::from_secs(u64::from(jev.timeout_seconds.max(1))),
            retentativas: jev.retries,
            chave,
        })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn endpoint(&self) -> String {
        format!("{}/v1/systemone", self.url_base)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JevRequest {
    pub estado: Value,
    pub modelo: String,
    pub questions: BTreeMap<String, Question>,
}

impl JevRequest {
    pub fn to_json(&self) -> Value {
        json!({
            "state": self.estado,
            "model": self.modelo,
            "questions": self.questions,
        })
    }
}

pub trait JevTransport: Send + Sync {
    fn judge<'a>(&'a self, request: JevRequest) -> BoxFuture<'a, Result<BatchResponse, JevError>>;
}

struct Inner {
    http: reqwest::Client,
    endpoint: String,
    config: JevClientConfig,
    metrics: BaseMetrics,
}

#[derive(Clone)]
pub struct JevClient(Arc<Inner>);

impl JevClient {
    pub fn new(config: JevClientConfig, metrics: BaseMetrics) -> Result<Self, JevError> {
        let http = build_http(&config, None)?;
        let endpoint = config.endpoint();
        Ok(Self(Arc::new(Inner {
            http,
            endpoint,
            config,
            metrics,
        })))
    }
}

impl JevClient {
    #[doc(hidden)]
    pub fn with_test_root(self, root_der: &[u8]) -> Result<Self, JevError> {
        let http = build_http(&self.0.config, Some(root_der))?;
        Ok(Self(Arc::new(Inner {
            http,
            endpoint: self.0.endpoint.clone(),
            config: self.0.config.clone(),
            metrics: self.0.metrics.clone(),
        })))
    }
}

fn build_http(
    config: &JevClientConfig,
    extra_root: Option<&[u8]>,
) -> Result<reqwest::Client, JevError> {
    let mut builder = reqwest::Client::builder().timeout(config.timeout);
    if let Some(der) = extra_root {
        let certificate = reqwest::Certificate::from_der(der).map_err(|error| {
            JevError::InvalidConfiguration(format!(
                "raiz de confiança inválida: {}",
                error.without_url()
            ))
        })?;
        builder = builder.add_root_certificate(certificate);
    }
    builder
        .build()
        .map_err(|error| JevError::Network(error.without_url().to_string()))
}

impl JevTransport for JevClient {
    fn judge<'a>(&'a self, request: JevRequest) -> BoxFuture<'a, Result<BatchResponse, JevError>> {
        Box::pin(async move {
            let inner = Arc::clone(&self.0);
            let body = serde_json::to_vec(&request.to_json()).map_err(|error| {
                JevError::InvalidResponse(format!("não consegui montar o corpo: {error}"))
            })?;
            let mut last: Option<JevError> = None;
            for attempt in 0..=inner.config.retentativas {
                if attempt > 0 {
                    tokio::time::sleep(RETRY_WAIT).await;
                }
                inner.metrics.jev_calls.increment(1);
                match send_once(&inner, &body).await {
                    Ok(batch) => return Ok(batch),
                    Err(error) => {
                        inner.metrics.jev_errors.increment(1);
                        if !error.is_transient() || attempt == inner.config.retentativas {
                            return Err(error);
                        }
                        tracing::debug!("erro" = %error, "Jev falhou; tentando de novo");
                        last = Some(error);
                    }
                }
            }
            Err(last.unwrap_or_else(|| JevError::Network("sem resposta do Jev".to_owned())))
        })
    }
}

async fn send_once(inner: &Inner, body: &[u8]) -> Result<BatchResponse, JevError> {
    let reply = inner
        .http
        .post(&inner.endpoint)
        .header(CONTENT_TYPE, "application/json")
        .header(
            AUTHORIZATION,
            format!("Bearer {}", inner.config.chave.expose()),
        )
        .body(body.to_vec())
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                return JevError::TimedOut {
                    ms: u64::try_from(inner.config.timeout.as_millis()).unwrap_or(u64::MAX),
                };
            }
            if let Some(detail) = tls_failure_detail(&error) {
                return JevError::Tls(detail);
            }
            JevError::Network(error.without_url().to_string())
        })?;
    let code = reply.status();
    if !code.is_success() {
        return Err(JevError::Status {
            code: code.as_u16(),
            motivo: code.canonical_reason().unwrap_or("sem motivo").to_owned(),
        });
    }
    let texto = reply
        .text()
        .await
        .map_err(|error| JevError::Network(error.without_url().to_string()))?;
    let parsed: Value = serde_json::from_str(&texto)
        .map_err(|error| JevError::InvalidResponse(format!("JSON inválido: {error}")))?;
    BatchResponse::from_json(&parsed)
}

const TLS_MARKERS: [&str; 10] = [
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

fn tls_failure_detail(error: &dyn std::error::Error) -> Option<String> {
    let mut causes = Vec::new();
    let mut current = Some(error);
    while let Some(cause) = current {
        causes.push(cause.to_string());
        current = cause.source();
    }
    let marked = |texto: &String| {
        let lower = texto.to_ascii_lowercase();
        TLS_MARKERS.iter().any(|marker| lower.contains(marker))
    };
    if !causes.iter().any(marked) {
        return None;
    }
    let detail = causes
        .iter()
        .rev()
        .find(|texto| marked(texto))
        .map(String::as_str)
        .unwrap_or("handshake TLS recusado");
    let no_breaks: String = detail
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    Some(no_breaks.trim().chars().take(200).collect())
}

type Script = Box<dyn Fn(&JevRequest) -> Result<BatchResponse, JevError> + Send + Sync>;

pub struct FakeJev {
    script: Script,
    delay: Duration,
    calls: AtomicU64,
    requests: std::sync::Mutex<Vec<JevRequest>>,
}

impl std::fmt::Debug for FakeJev {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JevFalso")
            .field("chamadas", &self.calls.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl FakeJev {
    pub fn new(
        script: impl Fn(&JevRequest) -> Result<BatchResponse, JevError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            script: Box::new(script),
            delay: Duration::ZERO,
            calls: AtomicU64::new(0),
            requests: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn with_choices(choices: BTreeMap<String, String>) -> Self {
        Self::new(move |request: &JevRequest| {
            let mut replies = BTreeMap::new();
            for id in request.questions.keys() {
                if let Some(choice) = choices.get(id) {
                    let mut reply = crate::types::JevResponse {
                        choice: Some(choice.clone()),
                        confidence: 1.0,
                        ..crate::types::JevResponse::default()
                    };
                    reply.probabilities.insert(choice.clone(), 1.0);
                    replies.insert(id.clone(), reply);
                }
            }
            Ok(BatchResponse {
                answers: replies,
                tokens: 12,
                ..BatchResponse::default()
            })
        })
    }

    pub fn failing(error: JevError) -> Self {
        Self::new(move |_| Err(error.clone()))
    }

    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn requests(&self) -> Vec<JevRequest> {
        match self.requests.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl JevTransport for FakeJev {
    fn judge<'a>(&'a self, request: JevRequest) -> BoxFuture<'a, Result<BatchResponse, JevError>> {
        Box::pin(async move {
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            self.calls.fetch_add(1, Ordering::SeqCst);
            let reply = (self.script)(&request);
            match self.requests.lock() {
                Ok(mut guard) => guard.push(request),
                Err(poisoned) => poisoned.into_inner().push(request),
            }
            reply
        })
    }
}
