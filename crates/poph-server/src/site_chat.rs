use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use poph_agent::turno::{SiteRun, SiteTurnError, TurnEvent, TurnInput, TurnObserver};
use poph_store::{SiteOutcome, SiteTurnKey, Store};
use poph_types::{ConversationId, OrgId, TurnId};
use tokio::sync::broadcast;

use crate::ServerState;

pub(crate) struct SiteWorkers {
    active: Mutex<HashMap<TurnId, TurnInput>>,
    pub changes: broadcast::Sender<(OrgId, ConversationId)>,
}

impl Default for SiteWorkers {
    fn default() -> Self {
        Self {
            active: Mutex::default(),
            changes: broadcast::channel(256).0,
        }
    }
}

impl SiteWorkers {
    pub fn signal(&self, key: SiteTurnKey) {
        let _ = self.changes.send((key.org, key.conversation));
    }

    pub fn cancel(&self, key: SiteTurnKey) {
        if let Some(entry) = self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&key.turn)
        {
            entry.cancellation.cancel();
        }
        self.signal(key);
    }
}

pub(crate) fn recover(state: &ServerState) -> Result<(), poph_store::StoreError> {
    for key in state.store.site_chat().recover()? {
        launch(state.clone(), key);
    }
    Ok(())
}

pub(crate) fn launch(state: ServerState, key: SiteTurnKey) {
    state.site_workers.signal(key);
    tokio::spawn(async move {
        let (job, entry) = {
            let mut active = state
                .site_workers
                .active
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match state.store.site_chat().claim(key) {
                Ok(Some((agent, message))) => {
                    let entry = TurnInput::new(key.org, key.conversation, message);
                    active.insert(key.turn, entry.clone());
                    (
                        SiteRun {
                            key,
                            agent,
                            message,
                        },
                        entry,
                    )
                }
                Ok(None) => return,
                Err(error) => {
                    tracing::error!(%error, turn_id = %key.turn, "falha ao iniciar turno do site");
                    finish(&state, key, SiteOutcome::Failed("turn_failed"));
                    return;
                }
            }
        };
        state.site_workers.signal(key);
        let mut output = SiteObserver {
            key,
            store: &state.store,
            workers: &state.site_workers,
            entry: &entry,
            failed: false,
        };
        let result = match &state.engine {
            Some(engine) => {
                engine
                    .run_site(
                        job,
                        &state.config.site_chat,
                        &entry.cancellation,
                        &mut output,
                    )
                    .await
            }
            None => Err(SiteTurnError::ModelUnavailable),
        };
        let outcome = match result {
            _ if output.failed => SiteOutcome::Failed("turn_failed"),
            Ok(reply) => SiteOutcome::Success(reply),
            Err(SiteTurnError::Cancelled) => SiteOutcome::Cancelled,
            Err(error) => SiteOutcome::Failed(error.code()),
        };
        finish(&state, key, outcome);
        state
            .site_workers
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&key.turn);
    });
}

fn finish(state: &ServerState, key: SiteTurnKey, outcome: SiteOutcome) {
    match state.store.site_chat().finish(key, outcome) {
        Ok(_) => state.site_workers.signal(key),
        Err(error) => {
            tracing::error!(%error, turn_id = %key.turn, "falha ao finalizar turno do site")
        }
    }
}

struct SiteObserver<'a> {
    key: SiteTurnKey,
    store: &'a Arc<Store>,
    workers: &'a SiteWorkers,
    entry: &'a TurnInput,
    failed: bool,
}

impl TurnObserver for SiteObserver<'_> {
    fn evento(&mut self, event: TurnEvent) {
        if let TurnEvent::Texto(text) = event {
            match self.store.site_chat().delta(self.key, text) {
                Ok(()) => self.workers.signal(self.key),
                Err(error) => {
                    self.failed = true;
                    self.entry.cancellation.cancel();
                    tracing::error!(%error, turn_id = %self.key.turn, "falha ao gravar trecho do site");
                }
            }
        }
    }
}
