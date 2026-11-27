use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poph_matrix::attendant_asset::{self, NotSendReason};
use poph_matrix::{Action, AssetError, AssetKind, GhostIds, MatrixClient, MatrixModule, classify};
use poph_store::error::StoreError;
use poph_store::{HumanResponse, HumanResponseWithFile, NewHumanResponse, ResponseFile, Store};
use poph_types::{AgentId, ConversationId, OrgId};
use serde_json::{Value, json};

use crate::api::matrix_as::{DeliveryError, DeliveryFuture, EventDelivery};
use crate::matrix_metrics::MetricsMatrix;

pub const DEFAULT_ATTENDANT_NAME: &str = "Atendente";

pub const DEFAULT_NAME_TTL: Duration = Duration::from_secs(5 * 60);

pub const DEFAULT_NAME_CAP: usize = 1024;

pub const DEFAULT_ROOM_CAP: usize = 1024;

pub const CLOSED_CONVERSATION_NOTICE: &str =
    "Esta conversa já foi encerrada: a mensagem não foi enviada ao cliente.";

const CLOSED_NOTICE_TAG: &str = "fechada";

const DATABASE_REASON: &str = "banco indisponível na resposta do atendente";

const ASSET_REASON: &str = "download da mídia do atendente falhou";

const ATTENDANT_EXTRACTION_STATUS: &str = "skipped";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomConversation {
    pub org: OrgId,
    pub conversation: ConversationId,
}

struct EventFile<'a> {
    room: &'a str,
    event_id: &'a str,
    sender: &'a str,
    kind: AssetKind,
    url: Option<&'a str>,
    name: &'a str,
}

pub trait ResponsePort: Send + Sync {
    fn room_conversation(&self, room: &str) -> Result<Option<RoomConversation>, StoreError>;

    fn conversation_agent(&self, target: RoomConversation) -> Result<Option<AgentId>, StoreError>;

    fn record_response(
        &self,
        target: RoomConversation,
        fresh: NewHumanResponse<'_>,
    ) -> Result<HumanResponse, StoreError>;

    fn record_response_with_file(
        &self,
        target: RoomConversation,
        fresh: HumanResponseWithFile<'_>,
    ) -> Result<HumanResponse, StoreError>;
}

#[derive(Debug, Clone)]
pub struct HarnessResponsePort {
    store: Arc<Store>,
}

impl HarnessResponsePort {
    pub fn fresh(store: Arc<Store>) -> Self {
        Self { store }
    }
}

impl ResponsePort for HarnessResponsePort {
    fn room_conversation(&self, room: &str) -> Result<Option<RoomConversation>, StoreError> {
        for state in self.store.matrix().linked_orgs()? {
            if let Some(stored) = self.store.matrix().room_by_room_id(state.org_id, room)? {
                return Ok(Some(RoomConversation {
                    org: state.org_id,
                    conversation: stored.conversation,
                }));
            }
        }
        Ok(None)
    }

    fn conversation_agent(&self, target: RoomConversation) -> Result<Option<AgentId>, StoreError> {
        match self
            .store
            .conversations()
            .get(target.org, target.conversation)
        {
            Ok(conversation) => Ok(Some(conversation.agent_id)),
            Err(error) if error.is_not_found() => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn record_response(
        &self,
        target: RoomConversation,
        fresh: NewHumanResponse<'_>,
    ) -> Result<HumanResponse, StoreError> {
        self.store
            .messages()
            .register_response_human(target.org, target.conversation, fresh)
    }

    fn record_response_with_file(
        &self,
        target: RoomConversation,
        fresh: HumanResponseWithFile<'_>,
    ) -> Result<HumanResponse, StoreError> {
        self.store.messages().record_human_response_with_file(
            target.org,
            target.conversation,
            fresh,
        )
    }
}

#[derive(Debug, Default)]
pub struct ResponseCounters {
    pub recorded: AtomicU64,
    pub duplicates: AtomicU64,
    pub unknown_rooms: AtomicU64,
    pub closed: AtomicU64,
    pub asset_notices: AtomicU64,
    pub files_recorded: AtomicU64,
    pub unsigned_assets: AtomicU64,
}

impl ResponseCounters {
    fn increment(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

pub struct AttendantResponse {
    port: Arc<dyn ResponsePort>,
    client: Option<MatrixClient>,
    ids: GhostIds,
    rooms: Mutex<BoundedCache<RoomConversation>>,
    names: Mutex<NameCache>,
    name_ttl: Duration,
    counters: ResponseCounters,
    metrics: Arc<MetricsMatrix>,
    blob_folder: Option<PathBuf>,
}

impl AttendantResponse {
    pub fn new(port: Arc<dyn ResponsePort>, client: Option<MatrixClient>, ids: GhostIds) -> Self {
        Self::with_caps(
            port,
            client,
            ids,
            DEFAULT_ROOM_CAP,
            DEFAULT_NAME_CAP,
            DEFAULT_NAME_TTL,
        )
    }

    pub fn with_caps(
        port: Arc<dyn ResponsePort>,
        client: Option<MatrixClient>,
        ids: GhostIds,
        room_cap: usize,
        name_cap: usize,
        name_ttl: Duration,
    ) -> Self {
        Self {
            port,
            client,
            ids,
            rooms: Mutex::new(BoundedCache::new(room_cap)),
            names: Mutex::new(NameCache::new(name_cap, name_ttl)),
            name_ttl,
            counters: ResponseCounters::default(),
            metrics: MetricsMatrix::unregistered(),
            blob_folder: None,
        }
    }

    pub fn with_metrics(mut self, metrics: Arc<MetricsMatrix>) -> Self {
        self.metrics = metrics;
        self
    }

    pub fn with_blob_folder(mut self, folder: impl Into<PathBuf>) -> Self {
        self.blob_folder = Some(folder.into());
        self
    }

    fn count_error(&self, error: &poph_matrix::CallError) {
        match error.classification() {
            poph_matrix::error::Classification::Transient => {
                self.metrics.transient_errors.increment(1);
            }
            poph_matrix::error::Classification::Definitive => {
                self.metrics.definitive_errors.increment(1);
            }
        }
        if matches!(error, poph_matrix::CallError::Response { status: 429, .. }) {
            self.metrics.responses_429.increment(1);
        }
    }

    pub fn from_module(
        store: Arc<Store>,
        module: &MatrixModule,
    ) -> Result<Self, poph_matrix::CallError> {
        let client = MatrixClient::from_parts(&module.config, &module.secrets)?;
        let ids = GhostIds::of_config(&module.config).map_err(|error| {
            poph_matrix::CallError::Configuration(format!("ids dos usuários virtuais: {error}"))
        })?;
        Ok(Self::new(
            Arc::new(HarnessResponsePort::fresh(store)),
            Some(client),
            ids,
        ))
    }

    pub fn counters(&self) -> &ResponseCounters {
        &self.counters
    }

    pub fn cache(&self) -> ((usize, usize), (usize, usize)) {
        let rooms = {
            let cached = self.cached_rooms();
            (cached.map.len(), cached.cap)
        };
        let names = {
            let cached = self.cached_names();
            (cached.map.len(), cached.cap)
        };
        (rooms, names)
    }

    pub fn name_ttl(&self) -> Duration {
        self.name_ttl
    }

    fn cached_rooms(&self) -> std::sync::MutexGuard<'_, BoundedCache<RoomConversation>> {
        self.rooms
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn cached_names(&self) -> std::sync::MutexGuard<'_, NameCache> {
        self.names
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn forget_room(&self, room: &str) {
        self.cached_rooms().forget(room);
    }

    fn room_conversation(&self, room: &str) -> Result<Option<RoomConversation>, StoreError> {
        if let Some(target) = self.cached_rooms().get(room) {
            return Ok(Some(target));
        }
        let Some(target) = self.port.room_conversation(room)? else {
            return Ok(None);
        };
        self.cached_rooms().insert(room, target);
        Ok(Some(target))
    }

    fn room_signature(&self, target: RoomConversation) -> Result<Option<String>, StoreError> {
        let Some(agent) = self.port.conversation_agent(target)? else {
            return Ok(None);
        };
        Ok(self
            .ids
            .agent(&agent.to_string())
            .ok()
            .map(|ghost| ghost.user_id().to_owned()))
    }

    fn reported_signature(
        &self,
        target: RoomConversation,
        transaction: &str,
        event_id: &str,
    ) -> Result<Option<String>, StoreError> {
        match self.room_signature(target) {
            Ok(Some(signature)) => Ok(Some(signature)),
            Ok(None) => {
                ResponseCounters::increment(&self.counters.unsigned_assets);
                tracing::error!(
                    target: "poph_server::matrix_resposta",
                    org = %target.org,
                    conversation = %target.conversation,
                    transaction,
                    event = event_id,
                    reason = "fantasma_do_agente_indisponivel",
                    "a conversa da sala não tem agente para assinar: nada foi escrito na sala"
                );
                Ok(None)
            }
            Err(_) => {
                tracing::error!(
                    target: "poph_server::matrix_resposta",
                    org = %target.org,
                    conversation = %target.conversation,
                    transaction,
                    event = event_id,
                    reason = "banco_indisponivel",
                    "não consegui ler o agente da conversa agora"
                );
                Err(StoreError::Internal(
                    "não consegui ler o agente da conversa".to_owned(),
                ))
            }
        }
    }

    async fn attendant_name(&self, sender: &str) -> String {
        if let Some(name) = self.cached_names().get(sender) {
            return name;
        }
        let Some(client) = &self.client else {
            return DEFAULT_ATTENDANT_NAME.to_owned();
        };
        match client.profile(sender).await {
            Ok(profile) => {
                let name = usable_name(profile.displayname.as_deref());
                self.cached_names().insert(sender, &name);
                name
            }
            Err(error) => {
                self.count_error(&error);
                tracing::debug!(
                    target: "poph_server::matrix_resposta",
                    classification = ?error.classification(),
                    "não consegui ler o perfil do atendente; usando o nome padrão"
                );
                DEFAULT_ATTENDANT_NAME.to_owned()
            }
        }
    }

    async fn send_notice(
        &self,
        room: &str,
        signature: &str,
        tag: &str,
        event_id: &str,
        body: &str,
    ) {
        let content = json!({
            "msgtype": poph_matrix::events::MSGTYPE_NOTICE,
            "body": body,
        });
        self.send_notice_content(room, signature, &event_txn(tag, event_id), &content)
            .await;
    }

    async fn send_notice_content(&self, room: &str, signature: &str, txn: &str, content: &Value) {
        let Some(client) = &self.client else {
            return;
        };
        if let Err(error) = client
            .send_event(signature, room, "m.room.message", txn, content)
            .await
        {
            self.count_error(&error);
            tracing::warn!(
                target: "poph_server::matrix_resposta",
                reason = ?error.classification(),
                "não consegui escrever o aviso na sala: o atendente não fica sabendo"
            );
        }
    }

    async fn respond(
        &self,
        transaction: &str,
        room: &str,
        event_id: &str,
        sender: &str,
        text: &str,
    ) -> Result<(), DeliveryError> {
        let target = match self.room_conversation(room) {
            Ok(Some(target)) => target,
            Ok(None) => {
                ResponseCounters::increment(&self.counters.unknown_rooms);
                self.metrics.rooms_unknown.increment(1);
                tracing::info!(
                    target: "poph_server::matrix_resposta",
                    transaction,
                    room,
                    "evento de sala que nenhuma org ligada conhece: ignorado"
                );
                return Ok(());
            }
            Err(_) => {
                tracing::error!(
                    target: "poph_server::matrix_resposta",
                    transaction,
                    room,
                    "não consegui achar a conversa da sala agora"
                );
                return Err(DeliveryError::new(DATABASE_REASON));
            }
        };

        let name = self.attendant_name(sender).await;
        let sent = queued_text(&name, text);
        let fresh = NewHumanResponse {
            event_id,
            raw_text: text,
            sent_text: &sent,
            author_id: sender,
            author_name: &name,
        };

        match self.port.record_response(target, fresh) {
            Ok(HumanResponse::Stored(_)) => {
                ResponseCounters::increment(&self.counters.recorded);
                self.metrics.responses_queued.increment(1);
                tracing::info!(
                    target: "poph_server::matrix_resposta",
                    org = %target.org,
                    conversation = %target.conversation,
                    transaction,
                    room,
                    event = event_id,
                    "resposta do atendente enfileirada para o cliente"
                );
                Ok(())
            }
            Ok(HumanResponse::Duplicate) => {
                ResponseCounters::increment(&self.counters.duplicates);
                self.metrics.responses_duplicated.increment(1);
                tracing::debug!(
                    target: "poph_server::matrix_resposta",
                    transaction,
                    event = event_id,
                    "reenvio do Synapse de uma resposta já gravada"
                );
                Ok(())
            }
            Ok(HumanResponse::ConversationClosed) => {
                ResponseCounters::increment(&self.counters.closed);
                self.metrics.responses_conversation_closed.increment(1);
                tracing::info!(
                    target: "poph_server::matrix_resposta",
                    org = %target.org,
                    conversation = %target.conversation,
                    transaction,
                    room,
                    event = event_id,
                    "conversa encerrada: resposta não enviada"
                );
                if let Ok(Some(signature)) = self.reported_signature(target, transaction, event_id)
                {
                    self.send_notice(
                        room,
                        &signature,
                        CLOSED_NOTICE_TAG,
                        event_id,
                        CLOSED_CONVERSATION_NOTICE,
                    )
                    .await;
                }
                Ok(())
            }
            Ok(HumanResponse::ConversationMissing) => {
                self.forget_room(room);
                ResponseCounters::increment(&self.counters.unknown_rooms);
                self.metrics.rooms_unknown.increment(1);
                tracing::info!(
                    target: "poph_server::matrix_resposta",
                    transaction,
                    room,
                    "a sala aponta para uma conversa que não existe nesta org: ignorado"
                );
                Ok(())
            }
            Err(_) => {
                tracing::error!(
                    target: "poph_server::matrix_resposta",
                    org = %target.org,
                    conversation = %target.conversation,
                    transaction,
                    room,
                    reason = "banco_indisponivel",
                    "não consegui gravar a resposta do atendente agora"
                );
                Err(DeliveryError::new(DATABASE_REASON))
            }
        }
    }

    async fn send_file(
        &self,
        transaction: &str,
        event: EventFile<'_>,
    ) -> Result<(), DeliveryError> {
        let EventFile {
            room,
            event_id,
            sender,
            kind,
            url,
            name,
        } = event;
        let target = match self.room_conversation(room) {
            Ok(Some(target)) => target,
            Ok(None) => {
                ResponseCounters::increment(&self.counters.unknown_rooms);
                self.metrics.rooms_unknown.increment(1);
                tracing::info!(
                    target: "poph_server::matrix_resposta",
                    transaction,
                    room,
                    event = event_id,
                    "arquivo em sala que nenhuma org ligada conhece: ignorado"
                );
                return Ok(());
            }
            Err(_) => {
                tracing::error!(
                    target: "poph_server::matrix_resposta",
                    transaction,
                    room,
                    reason = "banco_indisponivel",
                    "não consegui achar a conversa da sala agora"
                );
                return Err(DeliveryError::new(DATABASE_REASON));
            }
        };

        let signature = match self.reported_signature(target, transaction, event_id) {
            Ok(Some(signature)) => signature,
            Ok(None) => return Ok(()),
            Err(_) => return Err(DeliveryError::new(DATABASE_REASON)),
        };

        let Some(folder) = self.blob_folder.clone() else {
            self.send_asset_notice(
                transaction,
                room,
                &signature,
                event_id,
                NotSendReason::NotStored,
            )
            .await;
            return Ok(());
        };
        let (Some(url), Some(client)) = (url, self.client.as_ref()) else {
            self.send_asset_notice(
                transaction,
                room,
                &signature,
                event_id,
                NotSendReason::WithoutFile,
            )
            .await;
            return Ok(());
        };

        let bytes = match client
            .download_asset(&signature, url, attendant_asset::ASSET_BYTES_LIMIT)
            .await
        {
            Ok(bytes) => bytes,
            Err(error) => {
                if error.transient() {
                    if let AssetError::Call(call) = &error {
                        self.count_error(call);
                    }
                    tracing::warn!(
                        target: "poph_server::matrix_resposta",
                        org = %target.org,
                        conversation = %target.conversation,
                        transaction,
                        room,
                        event = event_id,
                        kind = kind.msgtype(),
                        "não consegui baixar o arquivo do atendente agora: volta no reenvio do Synapse"
                    );
                    return Err(DeliveryError::new(ASSET_REASON));
                }
                if let AssetError::Call(call) = &error {
                    self.count_error(call);
                }
                self.send_asset_notice(
                    transaction,
                    room,
                    &signature,
                    event_id,
                    asset_failure_reason(&error),
                )
                .await;
                return Ok(());
            }
        };

        let kind_real = match poph_files::outbound::classify(&bytes, name) {
            Ok(kind) => kind,
            Err(_) => {
                self.send_asset_notice(
                    transaction,
                    room,
                    &signature,
                    event_id,
                    NotSendReason::RefusedKind,
                )
                .await;
                return Ok(());
            }
        };

        let org = target.org;
        let size_bytes = bytes.len() as u64;
        let output_folder = folder.clone();
        let stored = match tokio::task::spawn_blocking(move || {
            poph_files::blob::write(&output_folder, org, &bytes)
        })
        .await
        {
            Ok(Ok(stored)) => stored,
            _ => {
                self.send_asset_notice(
                    transaction,
                    room,
                    &signature,
                    event_id,
                    NotSendReason::NotStored,
                )
                .await;
                return Ok(());
            }
        };

        let file_name = attendant_asset::file_name(name, kind_real.default_name());
        let attendant_name = self.attendant_name(sender).await;
        let caption = file_caption(&attendant_name);
        let blob_ref = format!("{}/{}", target.org, stored.sha256);
        let fresh = HumanResponseWithFile {
            event_id,
            author_id: sender,
            author_name: &attendant_name,
            file: ResponseFile {
                name: &file_name,
                mime: kind_real.mime(),
                size_bytes,
                sha256: &stored.sha256,
                blob_path: &blob_ref,
                caption: &caption,
                extraction_status: ATTENDANT_EXTRACTION_STATUS,
            },
        };

        match self.port.record_response_with_file(target, fresh) {
            Ok(HumanResponse::Stored(_)) => {
                ResponseCounters::increment(&self.counters.files_recorded);
                self.metrics.responses_queued.increment(1);
                tracing::info!(
                    target: "poph_server::matrix_resposta",
                    org = %target.org,
                    conversation = %target.conversation,
                    transaction,
                    room,
                    event = event_id,
                    kind = kind.msgtype(),
                    mime = kind_real.mime(),
                    size_bytes,
                    "arquivo do atendente enfileirado para o cliente"
                );
                Ok(())
            }
            Ok(HumanResponse::Duplicate) => {
                ResponseCounters::increment(&self.counters.duplicates);
                self.metrics.responses_duplicated.increment(1);
                tracing::debug!(
                    target: "poph_server::matrix_resposta",
                    transaction,
                    event = event_id,
                    "reenvio do Synapse de um arquivo do atendente já gravado"
                );
                Ok(())
            }
            Ok(HumanResponse::ConversationClosed) => {
                ResponseCounters::increment(&self.counters.closed);
                self.metrics.responses_conversation_closed.increment(1);
                tracing::info!(
                    target: "poph_server::matrix_resposta",
                    org = %target.org,
                    conversation = %target.conversation,
                    transaction,
                    room,
                    event = event_id,
                    "conversa encerrada: arquivo não enviado ao cliente"
                );
                self.send_notice(
                    room,
                    &signature,
                    CLOSED_NOTICE_TAG,
                    event_id,
                    CLOSED_CONVERSATION_NOTICE,
                )
                .await;
                Ok(())
            }
            Ok(HumanResponse::ConversationMissing) => {
                self.forget_room(room);
                ResponseCounters::increment(&self.counters.unknown_rooms);
                self.metrics.rooms_unknown.increment(1);
                tracing::info!(
                    target: "poph_server::matrix_resposta",
                    transaction,
                    room,
                    "a sala aponta para uma conversa que não existe nesta org: arquivo ignorado"
                );
                Ok(())
            }
            Err(_) => {
                tracing::error!(
                    target: "poph_server::matrix_resposta",
                    org = %target.org,
                    conversation = %target.conversation,
                    transaction,
                    room,
                    reason = "banco_indisponivel",
                    "não consegui gravar o arquivo do atendente agora"
                );
                Err(DeliveryError::new(DATABASE_REASON))
            }
        }
    }

    async fn send_asset_notice(
        &self,
        transaction: &str,
        room: &str,
        signature: &str,
        event_id: &str,
        reason: NotSendReason,
    ) {
        ResponseCounters::increment(&self.counters.asset_notices);
        self.metrics.asset_notices.increment(1);
        tracing::info!(
            target: "poph_server::matrix_resposta",
            transaction,
            room,
            event = event_id,
            reason = reason.code(),
            "arquivo do atendente não foi ao cliente: avisando na sala"
        );
        let txn = attendant_asset::notice_txn(event_id, reason);
        let content = attendant_asset::notice_content(reason);
        self.send_notice_content(room, signature, &txn, &content)
            .await;
    }

    async fn process(&self, transaction: &str, events: Vec<Value>) -> Result<(), DeliveryError> {
        for event in &events {
            match classify(&self.ids, event) {
                Action::HumanResponse {
                    room,
                    event_id,
                    sender,
                    text,
                } => {
                    self.respond(transaction, &room, &event_id, &sender, &text)
                        .await?;
                }
                Action::AttendantAsset {
                    room,
                    event_id,
                    sender,
                    kind,
                    url,
                    name,
                } => {
                    self.send_file(
                        transaction,
                        EventFile {
                            room: &room,
                            event_id: &event_id,
                            sender: &sender,
                            kind,
                            url: url.as_deref(),
                            name: &name,
                        },
                    )
                    .await?;
                }
                Action::Ignore { reason } => {
                    tracing::debug!(
                        target: "poph_server::matrix_resposta",
                        transaction,
                        reason = reason.code(),
                        "evento do Matrix ignorado"
                    );
                }
            }
        }
        Ok(())
    }
}

impl EventDelivery for AttendantResponse {
    fn deliver(&self, transaction: &str, events: Vec<Value>) -> DeliveryFuture<'_> {
        let transaction = transaction.to_owned();
        Box::pin(async move { self.process(&transaction, events).await })
    }
}

impl std::fmt::Debug for AttendantResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RespostaDoAtendente")
            .field("cliente", &self.client)
            .field("ids", &self.ids)
            .field("validade_nome", &self.name_ttl)
            .finish_non_exhaustive()
    }
}

pub fn queued_text(name: &str, text: &str) -> String {
    format!("**[{}]**:\n{text}", prefix_name(name))
}

pub fn file_caption(name: &str) -> String {
    format!("**[{}]**", prefix_name(name))
}

fn asset_failure_reason(error: &AssetError) -> NotSendReason {
    match error {
        AssetError::Large { .. } => NotSendReason::Large,
        AssetError::Empty => NotSendReason::Empty,
        AssetError::Address(_) => NotSendReason::WithoutFile,
        AssetError::Call(_) => NotSendReason::Unavailable,
    }
}

fn prefix_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '*' | '_' | '~' | '`') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let clean = clean.trim();
    if clean.is_empty() {
        return DEFAULT_ATTENDANT_NAME.to_owned();
    }
    if clean.chars().count() > 80 {
        return clean
            .chars()
            .take(80)
            .collect::<String>()
            .trim_end()
            .to_owned();
    }
    clean.to_owned()
}

fn usable_name(name: Option<&str>) -> String {
    prefix_name(name.unwrap_or_default())
}

fn event_txn(tag: &str, event_id: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in event_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("ph_{tag}_{hash:016x}")
}

#[derive(Debug)]
struct BoundedCache<V> {
    cap: usize,
    order: VecDeque<String>,
    map: HashMap<String, V>,
}

impl<V: Copy> BoundedCache<V> {
    fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            order: VecDeque::new(),
            map: HashMap::new(),
        }
    }

    fn get(&self, key: &str) -> Option<V> {
        self.map.get(key).copied()
    }

    fn insert(&mut self, key: &str, value: V) {
        if self.map.contains_key(key) {
            return;
        }
        while self.order.len() >= self.cap {
            match self.order.pop_front() {
                Some(older) => {
                    self.map.remove(&older);
                }
                None => break,
            }
        }
        self.order.push_back(key.to_owned());
        self.map.insert(key.to_owned(), value);
    }

    fn forget(&mut self, key: &str) {
        if self.map.remove(key).is_some() {
            self.order.retain(|cached| cached != key);
        }
    }
}

#[derive(Debug)]
struct NameCache {
    cap: usize,
    ttl: Duration,
    order: VecDeque<String>,
    map: HashMap<String, (String, Instant)>,
}

impl NameCache {
    fn new(cap: usize, ttl: Duration) -> Self {
        Self {
            cap: cap.max(1),
            ttl,
            order: VecDeque::new(),
            map: HashMap::new(),
        }
    }

    fn get(&self, key: &str) -> Option<String> {
        let (name, when) = self.map.get(key)?;
        if when.elapsed() > self.ttl {
            return None;
        }
        Some(name.clone())
    }

    fn insert(&mut self, key: &str, name: &str) {
        if let Some((cached, when)) = self.map.get_mut(key) {
            *cached = name.to_owned();
            *when = Instant::now();
            return;
        }
        while self.order.len() >= self.cap {
            match self.order.pop_front() {
                Some(older) => {
                    self.map.remove(&older);
                }
                None => break,
            }
        }
        self.order.push_back(key.to_owned());
        self.map
            .insert(key.to_owned(), (name.to_owned(), Instant::now()));
    }
}

#[cfg(test)]
mod tests {
    use poph_evolution::to_whatsapp;

    use super::*;

    fn target() -> RoomConversation {
        RoomConversation {
            org: OrgId::generate(),
            conversation: ConversationId::generate(),
        }
    }

    #[test]
    fn room_cache_evicts_the_oldest_entry() {
        let mut cache = BoundedCache::new(2);
        cache.insert("!a", target());
        cache.insert("!b", target());
        cache.insert("!c", target());
        assert_eq!(cache.map.len(), 2);
        assert!(cache.get("!a").is_none(), "o mais antigo saiu");
        assert!(cache.get("!b").is_some());
        assert!(cache.get("!c").is_some());
    }

    #[test]
    fn room_cache_can_forget_a_room() {
        let mut cache = BoundedCache::new(4);
        cache.insert("!a", target());
        cache.forget("!a");
        assert!(cache.get("!a").is_none());
        assert!(cache.order.is_empty());
    }

    #[test]
    fn name_cache_respects_capacity() {
        let mut cache = NameCache::new(2, Duration::from_secs(60));
        cache.insert("@a:s", "Ana");
        cache.insert("@b:s", "Bia");
        cache.insert("@c:s", "Caio");
        assert_eq!(cache.map.len(), 2);
        assert!(cache.get("@a:s").is_none());
        assert_eq!(cache.get("@c:s").as_deref(), Some("Caio"));
    }

    #[test]
    fn name_cache_entries_expire() {
        let mut cache = NameCache::new(4, Duration::ZERO);
        cache.insert("@a:s", "Ana");
        assert!(cache.get("@a:s").is_none(), "validade zero vence na hora");
    }

    #[test]
    fn name_cache_updates_an_existing_entry() {
        let mut cache = NameCache::new(4, Duration::from_secs(60));
        cache.insert("@a:s", "Ana");
        cache.insert("@a:s", "Ana Souza");
        assert_eq!(cache.get("@a:s").as_deref(), Some("Ana Souza"));
        assert_eq!(cache.map.len(), 1);
    }

    #[test]
    fn customer_receives_prefix_and_body_converted_once() {
        let queue = queued_text("Ana", "olá");
        assert_eq!(queue, "**[Ana]**:\nolá");
        assert_eq!(to_whatsapp(&queue), "*[Ana]*:\nolá");
    }

    #[test]
    fn markdown_body_is_converted_once() {
        let queue = queued_text("Ana", "**atenção** e _ênfase_");
        let customer_tag = to_whatsapp(&queue);
        assert_eq!(customer_tag, "*[Ana]*:\n*atenção* e _ênfase_");
    }

    #[test]
    fn oversize_text_is_truncated_to_one_channel_message() {
        let enorme = "a".repeat(5000);
        let customer_tag = to_whatsapp(&queued_text("Ana", &enorme));
        assert!(customer_tag.chars().count() <= poph_evolution::TEXT_LIMIT);
        assert!(customer_tag.starts_with("*[Ana]*:\n"));
        assert!(
            customer_tag.ends_with('…'),
            "corta com reticências, não divide"
        );
    }

    #[test]
    fn prefix_name_preserves_bold_and_is_nonempty() {
        assert_eq!(usable_name(None), DEFAULT_ATTENDANT_NAME);
        assert_eq!(usable_name(Some("   ")), DEFAULT_ATTENDANT_NAME);
        assert_eq!(usable_name(Some("Ana\nSouza")), "Ana Souza");
        assert_eq!(
            to_whatsapp(&queued_text("Ana*Souza", "oi")),
            "*[Ana Souza]*:\noi"
        );
        let long = "n".repeat(200);
        assert_eq!(usable_name(Some(&long)).chars().count(), 80);
    }

    #[test]
    fn file_caption_has_the_prefix_without_a_colon() {
        assert_eq!(file_caption("Ana Atendente"), "**[Ana Atendente]**");
        assert_eq!(
            to_whatsapp(&file_caption("Ana Atendente")),
            "*[Ana Atendente]*"
        );
        assert_eq!(to_whatsapp(&file_caption("Ana*Souza")), "*[Ana Souza]*");
        assert_eq!(
            to_whatsapp(&file_caption("   ")),
            format!("*[{DEFAULT_ATTENDANT_NAME}]*")
        );
    }

    #[test]
    fn asset_notice_reflects_download_error() {
        use poph_matrix::CallError;
        use poph_matrix::attendant_asset::NotSendReason;

        assert_eq!(
            asset_failure_reason(&AssetError::Large { bytes: 20, cap: 16 }),
            NotSendReason::Large
        );
        assert_eq!(
            asset_failure_reason(&AssetError::Empty),
            NotSendReason::Empty
        );
        assert_eq!(
            asset_failure_reason(&AssetError::Address("sem mxc".to_owned())),
            NotSendReason::WithoutFile
        );
        assert_eq!(
            asset_failure_reason(&AssetError::Call(CallError::Network {
                operation: "download",
                reason: "caiu".to_owned()
            })),
            NotSendReason::Unavailable
        );
    }

    #[test]
    fn notice_txn_is_stable_and_varies_by_tag() {
        assert_eq!(event_txn("midia", "$a:s"), event_txn("midia", "$a:s"));
        assert_ne!(event_txn("midia", "$a:s"), event_txn("fechada", "$a:s"));
        assert_ne!(event_txn("midia", "$a:s"), event_txn("midia", "$b:s"));
    }
}
