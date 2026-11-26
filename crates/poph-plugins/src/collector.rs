use std::sync::{Arc, Mutex};
use std::time::Duration;

use poph_types::{EventSink, InboundEvent, IntegrationEvent, StateEvent};

#[derive(Clone, Default)]
pub struct EventCollector {
    events: Arc<Mutex<Vec<IntegrationEvent>>>,
}

impl EventCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn all(&self) -> Vec<IntegrationEvent> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn inbound(&self) -> Vec<InboundEvent> {
        self.all()
            .into_iter()
            .filter_map(|event| match event {
                IntegrationEvent::Inbound(inbound) => Some(inbound),
                IntegrationEvent::Estado(_) => None,
            })
            .collect()
    }

    pub fn states(&self) -> Vec<StateEvent> {
        self.all()
            .into_iter()
            .filter_map(|event| match event {
                IntegrationEvent::Estado(state) => Some(state),
                IntegrationEvent::Inbound(_) => None,
            })
            .collect()
    }

    pub fn count(&self) -> usize {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    pub async fn wait_for(&self, minimum: usize, deadline: Duration) -> Vec<IntegrationEvent> {
        let limit = tokio::time::Instant::now() + deadline;
        loop {
            let events = self.all();
            if events.len() >= minimum || tokio::time::Instant::now() >= limit {
                return events;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

impl EventSink for EventCollector {
    fn receive(&self, event: IntegrationEvent) {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(event);
    }
}
