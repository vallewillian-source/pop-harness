use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream;

use super::error::LlmError;
use super::provedor::{LlmProvider, LlmStream, validate_request};
use super::types::{LlmEvent, Request};

#[derive(Debug, Clone)]
pub enum Script {
    Events(Vec<LlmEvent>),
    Failure(LlmError),
    Pending {
        events: Vec<LlmEvent>,
        after: Duration,
        error: LlmError,
    },
}

#[derive(Debug, Default)]
pub struct FakeProvider {
    scripts: Mutex<VecDeque<Script>>,
    received: Mutex<Vec<Request>>,
}

impl FakeProvider {
    pub fn with_scripts(scripts: Vec<Script>) -> Self {
        Self {
            scripts: Mutex::new(scripts.into()),
            received: Mutex::new(Vec::new()),
        }
    }

    pub fn received(&self) -> Vec<Request> {
        match self.received.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn next_script(&self) -> Option<Script> {
        match self.scripts.lock() {
            Ok(mut guard) => guard.pop_front(),
            Err(poisoned) => poisoned.into_inner().pop_front(),
        }
    }
}

impl LlmProvider for FakeProvider {
    fn stream<'a>(&'a self, request: Request) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            validate_request(&request, true)?;
            let cancellation = request.cancellation.clone();
            match self.received.lock() {
                Ok(mut guard) => guard.push(request),
                Err(poisoned) => poisoned.into_inner().push(request),
            }
            let script = self.next_script().ok_or_else(|| {
                LlmError::Configuracao("provedor falso sem roteiro para esta chamada".to_owned())
            })?;
            match script {
                Script::Failure(error) => Err(error),
                Script::Events(events) => {
                    let fluxo = stream::iter(events.into_iter().map(Ok));
                    Ok(Box::pin(fluxo) as LlmStream)
                }
                Script::Pending {
                    events,
                    after,
                    error,
                } => {
                    let wait = async move {
                        tokio::select! {
                            biased;
                            _ = cancellation.wait() => Err(LlmError::Cancelled),
                            _ = tokio::time::sleep(after) => Err(error),
                        }
                    };
                    let fluxo = stream::iter(events.into_iter().map(Ok)).chain(stream::once(wait));
                    Ok(Box::pin(fluxo) as LlmStream)
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use poph_types::OrgId;

    use super::*;
    use crate::llm::types::{FinishReason, Message, Part};

    #[tokio::test]
    async fn delivers_script_and_records_request() {
        let fake = FakeProvider::with_scripts(vec![Script::Events(vec![
            LlmEvent::Texto("oi".into()),
            LlmEvent::Fim(FinishReason::Completed),
        ])]);
        let request = Request::new(
            OrgId::generate(),
            vec![Message::User(vec![Part::Texto("olá".into())])],
        );
        let mut fluxo = fake.stream(request).await.expect("abre");
        let events: Vec<_> = fluxo.by_ref().collect().await;
        assert_eq!(events.len(), 2);
        assert_eq!(fake.received().len(), 1);
    }

    #[tokio::test]
    async fn script_failure_is_open_error() {
        let fake = FakeProvider::with_scripts(vec![Script::Failure(LlmError::Unavailable {
            tentativas: 3,
            last_error: "HTTP 503".into(),
        })]);
        let request = Request::new(
            OrgId::generate(),
            vec![Message::User(vec![Part::Texto("x".into())])],
        );
        assert!(matches!(
            fake.stream(request).await,
            Err(LlmError::Unavailable { .. })
        ));
    }

    #[tokio::test]
    async fn pending_respects_cancellation() {
        let fake = FakeProvider::with_scripts(vec![Script::Pending {
            events: vec![LlmEvent::Texto("parcial".into())],
            after: Duration::from_secs(30),
            error: LlmError::TimedOut { seconds: 30 },
        }]);
        let request = Request::new(
            OrgId::generate(),
            vec![Message::User(vec![Part::Texto("x".into())])],
        );
        let cancellation = request.cancellation.clone();
        let mut fluxo = fake.stream(request).await.expect("abre");
        assert!(matches!(fluxo.next().await, Some(Ok(LlmEvent::Texto(_)))));
        cancellation.cancel();
        assert_eq!(fluxo.next().await, Some(Err(LlmError::Cancelled)));
    }
}
