//! Provedor falso e determinístico, para testes e para o CI (HR-13 em diante).
//!
//! Cada chamada a `stream` consome o próximo roteiro da fila. Ele registra o pedido recebido,
//! para o teste conferir o que foi enviado (ex.: a imagem chegou, a ferramenta estava na lista).
//! Não aplica retentativa: o cliente real é que faz isso, e tem testes próprios para isso.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream;

use super::erro::LlmError;
use super::provedor::{LlmProvider, LlmStream, validate_request};
use super::tipos::{LlmEvent, Request};

/// Resultado previsto de uma chamada ao provedor falso.
#[derive(Debug, Clone)]
pub enum Script {
    /// Eventos na ordem, terminando no `Fim` que estiver na lista.
    Eventos(Vec<LlmEvent>),
    /// Falha antes de a resposta começar (ex.: 429 ou 5xx já esgotados).
    Falha(LlmError),
    /// Entrega os eventos e depois fica parado por `depois`. Se a chamada for cancelada
    /// antes, devolve `Cancelada`; se o tempo passar, devolve `erro` (ex.: `TempoEsgotado`).
    Pendurado {
        eventos: Vec<LlmEvent>,
        depois: Duration,
        erro: LlmError,
    },
}

/// Provedor falso. Compartilhável entre tarefas.
#[derive(Debug, Default)]
pub struct FakeProvider {
    roteiros: Mutex<VecDeque<Script>>,
    received: Mutex<Vec<Request>>,
}

impl FakeProvider {
    /// Provedor com os roteiros na ordem em que serão usados.
    pub fn with_scripts(roteiros: Vec<Script>) -> Self {
        Self {
            roteiros: Mutex::new(roteiros.into()),
            received: Mutex::new(Vec::new()),
        }
    }

    /// Pedidos recebidos até agora, na ordem.
    pub fn received(&self) -> Vec<Request> {
        match self.received.lock() {
            Ok(guarda) => guarda.clone(),
            Err(veneno) => veneno.into_inner().clone(),
        }
    }

    fn proximo(&self) -> Option<Script> {
        match self.roteiros.lock() {
            Ok(mut guarda) => guarda.pop_front(),
            Err(veneno) => veneno.into_inner().pop_front(),
        }
    }
}

impl LlmProvider for FakeProvider {
    fn stream<'a>(&'a self, pedido: Request) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            validate_request(&pedido, true)?;
            let cancelamento = pedido.cancelamento.clone();
            match self.received.lock() {
                Ok(mut guarda) => guarda.push(pedido),
                Err(veneno) => veneno.into_inner().push(pedido),
            }
            let roteiro = self.proximo().ok_or_else(|| {
                LlmError::Configuracao("provedor falso sem roteiro para esta chamada".to_owned())
            })?;
            match roteiro {
                Script::Falha(erro) => Err(erro),
                Script::Eventos(eventos) => {
                    let fluxo = stream::iter(eventos.into_iter().map(Ok));
                    Ok(Box::pin(fluxo) as LlmStream)
                }
                Script::Pendurado {
                    eventos,
                    depois,
                    erro,
                } => {
                    let espera = async move {
                        tokio::select! {
                            biased;
                            _ = cancelamento.aguardar() => Err(LlmError::Cancelled),
                            _ = tokio::time::sleep(depois) => Err(erro),
                        }
                    };
                    let fluxo =
                        stream::iter(eventos.into_iter().map(Ok)).chain(stream::once(espera));
                    Ok(Box::pin(fluxo) as LlmStream)
                }
            }
        })
    }
}

#[cfg(test)]
mod testes {
    use poph_types::OrgId;

    use super::*;
    use crate::llm::tipos::{FinishReason, Message, Part};

    #[tokio::test]
    async fn delivers_script_and_records_request() {
        let fake = FakeProvider::with_scripts(vec![Script::Eventos(vec![
            LlmEvent::Texto("oi".into()),
            LlmEvent::Fim(FinishReason::Concluido),
        ])]);
        let pedido = Request::new(
            OrgId::generate(),
            vec![Message::Usuario(vec![Part::Texto("olá".into())])],
        );
        let mut fluxo = fake.stream(pedido).await.expect("abre");
        let eventos: Vec<_> = fluxo.by_ref().collect().await;
        assert_eq!(eventos.len(), 2);
        assert_eq!(fake.received().len(), 1);
    }

    #[tokio::test]
    async fn script_failure_is_open_error() {
        let fake = FakeProvider::with_scripts(vec![Script::Falha(LlmError::Unavailable {
            tentativas: 3,
            ultimo: "HTTP 503".into(),
        })]);
        let pedido = Request::new(
            OrgId::generate(),
            vec![Message::Usuario(vec![Part::Texto("x".into())])],
        );
        assert!(matches!(
            fake.stream(pedido).await,
            Err(LlmError::Unavailable { .. })
        ));
    }

    #[tokio::test]
    async fn pending_respects_cancellation() {
        let fake = FakeProvider::with_scripts(vec![Script::Pendurado {
            eventos: vec![LlmEvent::Texto("parcial".into())],
            depois: Duration::from_secs(30),
            erro: LlmError::TimedOut { segundos: 30 },
        }]);
        let pedido = Request::new(
            OrgId::generate(),
            vec![Message::Usuario(vec![Part::Texto("x".into())])],
        );
        let cancelamento = pedido.cancelamento.clone();
        let mut fluxo = fake.stream(pedido).await.expect("abre");
        assert!(matches!(fluxo.next().await, Some(Ok(LlmEvent::Texto(_)))));
        cancelamento.cancelar();
        assert_eq!(fluxo.next().await, Some(Err(LlmError::Cancelled)));
    }
}
