use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use poph_store::{
    Conversa, ConversationStatus, NewConversation, OutboxItem, Page, Received, Store,
};
use poph_types::{
    AgentId, ConversationId, EventSink, InboundEvent, Integration, IntegrationError,
    IntegrationEvent, IntegrationId, OrgId, OutboundCommand, Presence, SendFile, SendText,
    SetPresence, StateEvent,
};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::entry_output::{Fluxo, OutputTransport, ReceivedMessage, SendFailure, SendFuture};
use crate::error::Resultado;

const QUEUE_EVENTS: usize = 1024;

const PAGES_SEARCH: u32 = 50;

const CAP_CACHE: usize = 4096;

pub struct SendByChannel {
    store: Arc<Store>,
    canal: Arc<dyn Integration>,
}

impl SendByChannel {
    pub fn new(store: Arc<Store>, canal: Arc<dyn Integration>) -> Self {
        Self { store, canal }
    }

    pub fn canal(&self) -> &Arc<dyn Integration> {
        &self.canal
    }
}

impl OutputTransport for SendByChannel {
    fn send(&self, item: OutboxItem) -> SendFuture {
        let store = Arc::clone(&self.store);
        let canal = Arc::clone(&self.canal);
        Box::pin(async move {
            let peer = peer_of_conversation(&store, &item)?;
            let idempotency = chave_idempotencia(&item);
            let command = command_item(&item, &peer, &idempotency)?;
            let resultado = match &command {
                OutboundCommand::Text(c) => canal.send_text(c.clone()).await,
                OutboundCommand::File(c) => canal.send_file(c.clone()).await,
                OutboundCommand::Presence(c) => canal.set_presence(c.clone()).await,
            };
            resultado.map_err(|error| failure(&error))
        })
    }
}

pub fn chave_idempotencia(item: &OutboxItem) -> String {
    format!("{}:{}", item.turn_id, item.seq)
}

fn peer_of_conversation(
    store: &Store,
    item: &OutboxItem,
) -> std::result::Result<String, SendFailure> {
    let conversa = store
        .conversations()
        .get(item.org_id, item.conversation_id)
        .map_err(|error| {
            SendFailure::definitive(format!("não consegui ler a conversa: {error}"))
        })?;
    conversa.peer_externo.clone().ok_or_else(|| {
        SendFailure::definitive(format!(
            "a conversa `{}` não tem identificador do cliente no canal (peer)",
            item.conversation_id
        ))
    })
}

pub fn command_item(
    item: &OutboxItem,
    peer: &str,
    idempotency: &str,
) -> std::result::Result<OutboundCommand, SendFailure> {
    let value: Value = serde_json::from_str(&item.content_json).map_err(|error| {
        SendFailure::definitive(format!("payload do item de saída inválido: {error}"))
    })?;
    match item.tipo.as_str() {
        "text" => {
            let texto = value
                .get("texto")
                .and_then(Value::as_str)
                .ok_or_else(|| SendFailure::definitive("item de saída `text` sem `texto`"))?;
            Ok(OutboundCommand::Text(SendText {
                idempotency: idempotency.to_owned(),
                peer: peer.to_owned(),
                text: texto.to_owned(),
            }))
        }
        "file" => {
            let campo = |nome: &str| {
                value
                    .get(nome)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        SendFailure::definitive(format!("item de saída `file` sem `{nome}`"))
                    })
            };
            let size_bytes = value
                .get("tamanho_bytes")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    SendFailure::definitive("item de saída `file` sem `tamanho_bytes`")
                })?;
            Ok(OutboundCommand::File(SendFile {
                idempotency: idempotency.to_owned(),
                peer: peer.to_owned(),
                name: campo("nome")?,
                mime: campo("mime")?,
                size_bytes,
                sha256: campo("sha256")?,
                blob_ref: campo("blob_ref")?,
                caption: value
                    .get("legenda")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            }))
        }
        "presence" => {
            let estado = match value.get("estado").and_then(Value::as_str) {
                Some("pausado") => Presence::Paused,
                Some("digitando") => Presence::Typing,
                other => {
                    return Err(SendFailure::definitive(format!(
                        "item de saída `presence` com estado desconhecido: {other:?}"
                    )));
                }
            };
            Ok(OutboundCommand::Presence(SetPresence {
                peer: peer.to_owned(),
                state: estado,
            }))
        }
        other => Err(SendFailure::definitive(format!(
            "item de saída com tipo desconhecido: `{other}`"
        ))),
    }
}

fn failure(error: &IntegrationError) -> SendFailure {
    if error.is_transient() {
        SendFailure::transient(error.to_string())
    } else {
        SendFailure::definitive(error.to_string())
    }
}

pub trait ResolverConversation: Send + Sync + 'static {
    fn resolver(&self, org: OrgId, evento: &InboundEvent) -> Resultado<ConversationId>;
}

pub struct ConversationChannel {
    store: Arc<Store>,
    agente: AgentId,
    canal: String,
    integracao: Option<IntegrationId>,
    cache: Mutex<HashMap<String, ConversationId>>,
}

impl ConversationChannel {
    pub fn new(
        store: Arc<Store>,
        agente: AgentId,
        canal: impl Into<String>,
        integracao: Option<IntegrationId>,
    ) -> Self {
        Self {
            store,
            agente,
            canal: canal.into(),
            integracao,
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn em_cache(&self) -> usize {
        self.cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .len()
    }

    fn do_cache(&self, org: OrgId, peer: &str) -> Option<ConversationId> {
        let lembrada = {
            let cache = self
                .cache
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            cache.get(peer).copied()
        }?;
        let conversa = self.store.conversations().get(org, lembrada).ok();
        let vale = conversa.is_some_and(|conversa| {
            conversa.canal == self.canal
                && conversa.peer_externo.as_deref() == Some(peer)
                && conversa.status == ConversationStatus::Open
        });
        if vale {
            return Some(lembrada);
        }
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove(peer);
        }
        None
    }

    fn lembrar(&self, peer: &str, conversa: ConversationId) {
        let Ok(mut cache) = self.cache.lock() else {
            return;
        };
        if cache.len() >= CAP_CACHE {
            cache.clear();
        }
        cache.insert(peer.to_owned(), conversa);
    }
}

impl ResolverConversation for ConversationChannel {
    fn resolver(&self, org: OrgId, evento: &InboundEvent) -> Resultado<ConversationId> {
        if let Some(conversa) = self.do_cache(org, &evento.peer) {
            return Ok(conversa);
        }
        if let Some(conversa) = self.procurar(org, &evento.peer)? {
            self.lembrar(&evento.peer, conversa.id);
            return Ok(conversa.id);
        }
        let new = self.store.conversations().create(
            org,
            NewConversation {
                agent_id: self.agente,
                integration_id: self.integracao,
                canal: self.canal.clone(),
                peer_externo: Some(evento.peer.clone()),
                target_department: None,
            },
        )?;
        self.lembrar(&evento.peer, new.id);
        Ok(new.id)
    }
}

impl ConversationChannel {
    fn procurar(&self, org: OrgId, peer: &str) -> Resultado<Option<Conversa>> {
        let mut after = None;
        let mut found: Option<Conversa> = None;
        for _ in 0..PAGES_SEARCH {
            let page = Page { after, limit: 200 };
            let conversas = self.store.conversations().list(org, None, page)?;
            if conversas.is_empty() {
                break;
            }
            after = conversas.last().map(|conversa| conversa.id.as_uuid());
            for conversa in conversas {
                if conversa.canal != self.canal || conversa.peer_externo.as_deref() != Some(peer) {
                    continue;
                }
                if conversa.status == ConversationStatus::Open {
                    return Ok(Some(conversa));
                }
                found.get_or_insert(conversa);
            }
            if after.is_none() {
                break;
            }
        }
        Ok(found)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EntryReport {
    pub received: u64,
    pub stored: u64,
    pub duplicated: u64,
    pub refused: u64,
    pub failures: u64,
    pub states: u64,
    pub with_attachment: u64,
}

#[derive(Debug, Default)]
struct Counters {
    received: AtomicU64,
    stored: AtomicU64,
    duplicated: AtomicU64,
    refused: AtomicU64,
    failures: AtomicU64,
    states: AtomicU64,
    with_attachment: AtomicU64,
}

impl Counters {
    fn report(&self) -> EntryReport {
        EntryReport {
            received: self.received.load(Ordering::Relaxed),
            stored: self.stored.load(Ordering::Relaxed),
            duplicated: self.duplicated.load(Ordering::Relaxed),
            refused: self.refused.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
            states: self.states.load(Ordering::Relaxed),
            with_attachment: self.with_attachment.load(Ordering::Relaxed),
        }
    }
}

pub struct InboxChannel {
    fluxo: Fluxo,
    org: OrgId,
    resolver: Arc<dyn ResolverConversation>,
    sender: mpsc::Sender<InboundEvent>,
    counters: Arc<Counters>,
    last_state: Mutex<Option<StateEvent>>,
}

impl InboxChannel {
    pub fn start(fluxo: Fluxo, org: OrgId, resolver: Arc<dyn ResolverConversation>) -> Arc<Self> {
        let (sender, mut receiver) = mpsc::channel(QUEUE_EVENTS);
        let counters = Arc::new(Counters::default());
        let channel = Arc::new(Self {
            fluxo,
            org,
            resolver,
            sender,
            counters: Arc::clone(&counters),
            last_state: Mutex::new(None),
        });
        let gravador = Arc::clone(&channel);
        tokio::spawn(async move {
            while let Some(evento) = receiver.recv().await {
                gravador.write(evento);
            }
        });
        channel
    }

    pub fn counters(&self) -> EntryReport {
        self.counters.report()
    }

    pub fn last_state(&self) -> Option<StateEvent> {
        self.last_state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    pub fn write(&self, evento: InboundEvent) -> Option<Received> {
        let conversa = match self.resolver.resolver(self.org, &evento) {
            Ok(conversa) => conversa,
            Err(error) => {
                self.counters.failures.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    org = %self.org,
                    integracao = %evento.integration,
                    id_externo = %evento.id_externo,
                    %error,
                    "não consegui resolver a conversa do evento de entrada"
                );
                return None;
            }
        };
        if !evento.attachments.is_empty() {
            self.counters
                .with_attachment
                .fetch_add(1, Ordering::Relaxed);
        }
        let received = ReceivedMessage {
            integration_id: evento.integration,
            id_externo: evento.id_externo.clone(),
            conversa,
            texto: evento.text.clone(),
            attachments_json: attachments_json(&evento),
        };
        match self.fluxo.receive_message(self.org, received) {
            Ok(stored) => match stored.situation {
                Received::New => {
                    self.counters.stored.fetch_add(1, Ordering::Relaxed);
                    Some(Received::New)
                }
                Received::Duplicate => {
                    self.counters.duplicated.fetch_add(1, Ordering::Relaxed);
                    Some(Received::Duplicate)
                }
            },
            Err(error) => {
                self.counters.failures.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    org = %self.org,
                    integracao = %evento.integration,
                    id_externo = %evento.id_externo,
                    %error,
                    "não consegui gravar o evento de entrada na inbox"
                );
                None
            }
        }
    }
}

impl EventSink for InboxChannel {
    fn receive(&self, evento: IntegrationEvent) {
        match evento {
            IntegrationEvent::Inbound(entry) => match self.sender.try_send(entry) {
                Ok(()) => {
                    self.counters.received.fetch_add(1, Ordering::Relaxed);
                }
                Err(error) => {
                    self.counters.refused.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(%error, "fila de entrada cheia: evento do canal recusado");
                }
            },
            IntegrationEvent::Estado(estado) => {
                self.counters.states.fetch_add(1, Ordering::Relaxed);
                *self
                    .last_state
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = Some(estado);
            }
        }
    }
}

fn attachments_json(evento: &InboundEvent) -> String {
    if evento.attachments.is_empty() {
        return "[]".to_owned();
    }
    serde_json::to_string(&evento.attachments).unwrap_or_else(|_| "[]".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use poph_store::OutboxState;
    use poph_types::{OutboxId, TurnId};

    fn item(tipo: &str, content: Value) -> OutboxItem {
        OutboxItem {
            id: OutboxId::generate(),
            org_id: OrgId::generate(),
            conversation_id: ConversationId::generate(),
            turn_id: TurnId::generate(),
            seq: 3,
            tipo: tipo.to_owned(),
            content_json: content.to_string(),
            state: OutboxState::Pending,
            attempts: 0,
            ultimo_erro: None,
            created_at: 0,
            enviado_em: None,
        }
    }

    #[test]
    fn text_becomes_send_text_with_a_key_of_outbox() {
        let item = item("text", serde_json::json!({"texto": "olá"}));
        let command = command_item(&item, "5511", &chave_idempotencia(&item)).unwrap();
        let OutboundCommand::Text(texto) = command else {
            panic!("esperava enviar_texto");
        };
        assert_eq!(texto.peer, "5511");
        assert_eq!(texto.text, "olá");
        assert_eq!(texto.idempotency, format!("{}:3", item.turn_id));
    }

    #[test]
    fn file_becomes_send_file_with_o_blob() {
        let item = item(
            "file",
            serde_json::json!({
                "blob_ref": "org/sha",
                "arquivo_id": "01a1f6a5-cd9b-7000-8000-000000000000",
                "nome": "manual.pdf",
                "mime": "application/pdf",
                "tamanho_bytes": 1234,
                "sha256": "a".repeat(64),
                "legenda": "segue"
            }),
        );
        let command = command_item(&item, "5511", "t:1").unwrap();
        let OutboundCommand::File(file) = command else {
            panic!("esperava enviar_arquivo");
        };
        assert_eq!(file.name, "manual.pdf");
        assert_eq!(file.blob_ref, "org/sha");
        assert_eq!(file.size_bytes, 1234);
        assert_eq!(file.caption.as_deref(), Some("segue"));
    }

    #[test]
    fn presence_kind_unknown() {
        let command = command_item(
            &item("presence", serde_json::json!({"estado": "digitando"})),
            "5511",
            "t:2",
        )
        .unwrap();
        assert!(matches!(command, OutboundCommand::Presence(_)));

        let error = command_item(&item("texto", serde_json::json!({})), "5511", "t:2").unwrap_err();
        assert!(!error.transient, "tipo desconhecido não deve ser retentado");
    }

    #[test]
    fn text_without_field_failure_definitive() {
        let error = command_item(
            &item("text", serde_json::json!({"outro": 1})),
            "5511",
            "t:1",
        )
        .unwrap_err();
        assert!(!error.transient);
        assert!(error.motivo.contains("`texto`"), "{}", error.motivo);
    }

    #[test]
    fn attachment_event_stays_descrito_towards_message() {
        let evento = InboundEvent {
            integration: IntegrationId::generate(),
            id_externo: "abc".to_owned(),
            peer: "5511".to_owned(),
            remetente: None,
            text: "segue".to_owned(),
            attachments: vec![poph_types::InboundAttachment::Blob {
                name: "manual.pdf".to_owned(),
                mime: "application/pdf".to_owned(),
                sha256: "a".repeat(64),
                size_bytes: 10,
            }],
            occurred_at: 1,
            metadata: Value::Null,
        };
        let json = attachments_json(&evento);
        assert!(json.contains("manual.pdf"), "{json}");

        let without = InboundEvent {
            attachments: Vec::new(),
            ..evento
        };
        assert_eq!(attachments_json(&without), "[]");
    }
}
