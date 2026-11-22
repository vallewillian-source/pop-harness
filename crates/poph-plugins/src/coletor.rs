//! Coletor de eventos para testes e para quem só quer observar a integração.
//!
//! Implementa [`Receptor`] guardando os eventos em memória. Serve ao CI (é onde os testes leem o
//! que o plugin entregou) e a qualquer consumidor que ainda não tenha um destino próprio.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use poph_types::{EventoEstado, EventoIntegracao, InboundEvent, Receptor};

/// Receptor que guarda tudo o que chega.
#[derive(Clone, Default)]
pub struct ColetorEventos {
    eventos: Arc<Mutex<Vec<EventoIntegracao>>>,
}

impl ColetorEventos {
    /// Coletor vazio.
    pub fn novo() -> Self {
        Self::default()
    }

    /// Tudo o que chegou, na ordem.
    pub fn todos(&self) -> Vec<EventoIntegracao> {
        self.eventos
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .clone()
    }

    /// Só os eventos de entrada.
    pub fn entradas(&self) -> Vec<InboundEvent> {
        self.todos()
            .into_iter()
            .filter_map(|evento| match evento {
                EventoIntegracao::Entrada(entrada) => Some(entrada),
                EventoIntegracao::Estado(_) => None,
            })
            .collect()
    }

    /// Só os eventos de estado.
    pub fn estados(&self) -> Vec<EventoEstado> {
        self.todos()
            .into_iter()
            .filter_map(|evento| match evento {
                EventoIntegracao::Estado(estado) => Some(estado),
                EventoIntegracao::Entrada(_) => None,
            })
            .collect()
    }

    /// Quantos eventos já chegaram.
    pub fn quantidade(&self) -> usize {
        self.eventos
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .len()
    }

    /// Espera até chegar o `minimo`-ésimo evento (ou estourar o prazo).
    pub async fn esperar(&self, minimo: usize, prazo: Duration) -> Vec<EventoIntegracao> {
        let limite = tokio::time::Instant::now() + prazo;
        loop {
            let eventos = self.todos();
            if eventos.len() >= minimo || tokio::time::Instant::now() >= limite {
                return eventos;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

impl Receptor for ColetorEventos {
    fn receber(&self, evento: EventoIntegracao) {
        self.eventos
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .push(evento);
    }
}
