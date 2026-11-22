//! Espera injetável da janela de agrupamento (PRD §6, passo 3).
//!
//! A janela de agrupamento é uma espera de tempo real em produção e precisa ser **instantânea** nos
//! testes, para provar "duas seguidas viram um turno, depois do teto vira outro" sem dormir. Por
//! isso o motor não chama `tokio::time::sleep` direto: ele pede a espera a um [`Pausa`].

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// Futuro de uma espera (sem depender de `async_trait`).
pub type FuturoPausa = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Quem sabe esperar. Produção usa [`PausaReal`]; teste usa uma pausa que só registra o pedido.
pub trait Pausa: Send + Sync + 'static {
    /// Espera o tempo pedido. Tempo zero devolve na hora.
    fn esperar(&self, duracao: Duration) -> FuturoPausa;
}

/// Espera de verdade, pelo relógio do tokio. Precisa de runtime tokio ativo.
#[derive(Debug, Default, Clone, Copy)]
pub struct PausaReal;

impl Pausa for PausaReal {
    fn esperar(&self, duracao: Duration) -> FuturoPausa {
        if duracao.is_zero() {
            return Box::pin(async {});
        }
        Box::pin(tokio::time::sleep(duracao))
    }
}
