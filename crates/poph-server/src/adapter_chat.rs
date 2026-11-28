use std::collections::HashMap;
use std::sync::Mutex;

use poph_agent::turno::{Desfecho, TurnEvent, TurnInput, TurnObserver};
use poph_store::{AdapterTurnKey, AdapterTurnOutcome, AdapterTurnProgress, Store};
use poph_types::TurnId;

use crate::ServerState;

#[derive(Default)]
pub(crate) struct AdapterWorkers {
    active: Mutex<HashMap<TurnId, TurnInput>>,
}

impl AdapterWorkers {
    pub fn cancel(&self, key: AdapterTurnKey) {
        if let Some(entry) = self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&key.turn)
        {
            entry.cancellation.cancel();
        }
    }

    fn insert(&self, key: AdapterTurnKey, input: TurnInput) {
        self.active
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(key.turn, input);
    }

    fn remove(&self, turn: TurnId) {
        self.active
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&turn);
    }
}

pub(crate) fn recover(state: &ServerState) -> Result<(), poph_store::StoreError> {
    for key in state.store.adapter_chat().recover_turns()? {
        launch(state.clone(), key);
    }
    Ok(())
}

pub(crate) fn launch(state: ServerState, key: AdapterTurnKey) {
    tokio::spawn(async move {
        let input = match state.store.adapter_chat().claim_turn(key) {
            Ok(Some(claim)) => {
                TurnInput::new(key.org, key.conversation, claim.message).without_outbox()
            }
            Ok(None) => return,
            Err(error) => {
                tracing::error!(%error, turn_id = %key.turn, "adapter turn could not start");
                finish(&state, key, AdapterTurnOutcome::Failed("turn_failed"));
                return;
            }
        };
        state.adapter_workers.insert(key, input.clone());
        let result = match &state.engine {
            Some(engine) => {
                let mut observer = AdapterObserver {
                    key,
                    store: &state.store,
                };
                engine.run(input, &mut observer).await
            }
            None => {
                finish(&state, key, AdapterTurnOutcome::Failed("model_unavailable"));
                state.adapter_workers.remove(key.turn);
                return;
            }
        };
        let outcome = match result {
            Ok(summary) if summary.replied() => AdapterTurnOutcome::Success {
                engine_turn: summary.turno,
                input_tokens: summary.input_tokens,
                output_tokens: summary.output_tokens,
            },
            Ok(summary) => {
                if let Some(error) = summary.error.as_deref() {
                    tracing::warn!(turn_id = %key.turn, reason = %error, "adapter turn finished without a reply");
                }
                AdapterTurnOutcome::Failed("turn_failed")
            }
            Err(error) => {
                tracing::warn!(%error, turn_id = %key.turn, "adapter turn failed");
                AdapterTurnOutcome::Failed("turn_failed")
            }
        };
        finish(&state, key, outcome);
        state.adapter_workers.remove(key.turn);
    });
}

struct AdapterObserver<'a> {
    key: AdapterTurnKey,
    store: &'a Store,
}

impl TurnObserver for AdapterObserver<'_> {
    fn evento(&mut self, event: TurnEvent) {
        let chat = self.store.adapter_chat();
        let result = match event {
            TurnEvent::Started { turno, .. } => chat.record_engine_turn(self.key, turno),
            TurnEvent::Texto(text) => {
                chat.record_progress(self.key, AdapterTurnProgress::TextDelta(text))
            }
            TurnEvent::Tool { nome, .. } => {
                chat.record_progress(self.key, AdapterTurnProgress::ToolCall(nome))
            }
            _ => return,
        };
        self.log_persistence_error(result);
    }

    fn tool_result(&mut self, name: &str, outcome: Desfecho) {
        let refusal = match outcome {
            Desfecho::Executed => None,
            Desfecho::Refused(code) => Some(code),
        };
        self.log_persistence_error(self.store.adapter_chat().record_progress(
            self.key,
            AdapterTurnProgress::ToolResult {
                name: name.to_owned(),
                refusal,
            },
        ));
    }
}

impl AdapterObserver<'_> {
    fn log_persistence_error(&self, result: Result<(), poph_store::StoreError>) {
        if let Err(error) = result {
            tracing::warn!(%error, turn_id = %self.key.turn, "não foi possível gravar o progresso do turno de adaptador");
        }
    }
}

fn finish(state: &ServerState, key: AdapterTurnKey, outcome: AdapterTurnOutcome) {
    if let Err(error) = state.store.adapter_chat().finish_turn(key, outcome) {
        tracing::error!(%error, turn_id = %key.turn, "adapter turn could not be finalised");
    }
}
