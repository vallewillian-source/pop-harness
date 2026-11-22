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

use super::erro::ErroLlm;
use super::provedor::{FluxoLlm, LlmProvider, validar_pedido};
use super::tipos::{EventoLlm, Pedido};

/// Resultado previsto de uma chamada ao provedor falso.
#[derive(Debug, Clone)]
pub enum Roteiro {
    /// Eventos na ordem, terminando no `Fim` que estiver na lista.
    Eventos(Vec<EventoLlm>),
    /// Falha antes de a resposta começar (ex.: 429 ou 5xx já esgotados).
    Falha(ErroLlm),
    /// Entrega os eventos e depois fica parado por `depois`. Se a chamada for cancelada
    /// antes, devolve `Cancelada`; se o tempo passar, devolve `erro` (ex.: `TempoEsgotado`).
    Pendurado {
        eventos: Vec<EventoLlm>,
        depois: Duration,
        erro: ErroLlm,
    },
}

/// Provedor falso. Compartilhável entre tarefas.
#[derive(Debug, Default)]
pub struct ProvedorFake {
    roteiros: Mutex<VecDeque<Roteiro>>,
    recebidos: Mutex<Vec<Pedido>>,
}

impl ProvedorFake {
    /// Provedor com os roteiros na ordem em que serão usados.
    pub fn com_roteiros(roteiros: Vec<Roteiro>) -> Self {
        Self {
            roteiros: Mutex::new(roteiros.into()),
            recebidos: Mutex::new(Vec::new()),
        }
    }

    /// Pedidos recebidos até agora, na ordem.
    pub fn recebidos(&self) -> Vec<Pedido> {
        match self.recebidos.lock() {
            Ok(guarda) => guarda.clone(),
            Err(veneno) => veneno.into_inner().clone(),
        }
    }

    fn proximo(&self) -> Option<Roteiro> {
        match self.roteiros.lock() {
            Ok(mut guarda) => guarda.pop_front(),
            Err(veneno) => veneno.into_inner().pop_front(),
        }
    }
}

impl LlmProvider for ProvedorFake {
    fn stream<'a>(&'a self, pedido: Pedido) -> BoxFuture<'a, Result<FluxoLlm, ErroLlm>> {
        Box::pin(async move {
            validar_pedido(&pedido)?;
            let cancelamento = pedido.cancelamento.clone();
            match self.recebidos.lock() {
                Ok(mut guarda) => guarda.push(pedido),
                Err(veneno) => veneno.into_inner().push(pedido),
            }
            let roteiro = self.proximo().ok_or_else(|| {
                ErroLlm::Configuracao("provedor falso sem roteiro para esta chamada".to_owned())
            })?;
            match roteiro {
                Roteiro::Falha(erro) => Err(erro),
                Roteiro::Eventos(eventos) => {
                    let fluxo = stream::iter(eventos.into_iter().map(Ok));
                    Ok(Box::pin(fluxo) as FluxoLlm)
                }
                Roteiro::Pendurado {
                    eventos,
                    depois,
                    erro,
                } => {
                    let espera = async move {
                        tokio::select! {
                            biased;
                            _ = cancelamento.aguardar() => Err(ErroLlm::Cancelada),
                            _ = tokio::time::sleep(depois) => Err(erro),
                        }
                    };
                    let fluxo =
                        stream::iter(eventos.into_iter().map(Ok)).chain(stream::once(espera));
                    Ok(Box::pin(fluxo) as FluxoLlm)
                }
            }
        })
    }
}

#[cfg(test)]
mod testes {
    use poph_types::OrgId;

    use super::*;
    use crate::llm::tipos::{Mensagem, MotivoFim, Parte};

    #[tokio::test]
    async fn entrega_o_roteiro_e_registra_o_pedido() {
        let fake = ProvedorFake::com_roteiros(vec![Roteiro::Eventos(vec![
            EventoLlm::Texto("oi".into()),
            EventoLlm::Fim(MotivoFim::Concluido),
        ])]);
        let pedido = Pedido::new(
            OrgId::generate(),
            vec![Mensagem::Usuario(vec![Parte::Texto("olá".into())])],
        );
        let mut fluxo = fake.stream(pedido).await.expect("abre");
        let eventos: Vec<_> = fluxo.by_ref().collect().await;
        assert_eq!(eventos.len(), 2);
        assert_eq!(fake.recebidos().len(), 1);
    }

    #[tokio::test]
    async fn falha_de_roteiro_chega_como_erro_ao_abrir() {
        let fake = ProvedorFake::com_roteiros(vec![Roteiro::Falha(ErroLlm::Indisponivel {
            tentativas: 3,
            ultimo: "HTTP 503".into(),
        })]);
        let pedido = Pedido::new(
            OrgId::generate(),
            vec![Mensagem::Usuario(vec![Parte::Texto("x".into())])],
        );
        assert!(matches!(
            fake.stream(pedido).await,
            Err(ErroLlm::Indisponivel { .. })
        ));
    }

    #[tokio::test]
    async fn pendurado_respeita_cancelamento() {
        let fake = ProvedorFake::com_roteiros(vec![Roteiro::Pendurado {
            eventos: vec![EventoLlm::Texto("parcial".into())],
            depois: Duration::from_secs(30),
            erro: ErroLlm::TempoEsgotado { segundos: 30 },
        }]);
        let pedido = Pedido::new(
            OrgId::generate(),
            vec![Mensagem::Usuario(vec![Parte::Texto("x".into())])],
        );
        let cancelamento = pedido.cancelamento.clone();
        let mut fluxo = fake.stream(pedido).await.expect("abre");
        assert!(matches!(fluxo.next().await, Some(Ok(EventoLlm::Texto(_)))));
        cancelamento.cancelar();
        assert_eq!(fluxo.next().await, Some(Err(ErroLlm::Cancelada)));
    }
}
