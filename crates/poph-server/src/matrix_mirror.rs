use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use poph_matrix::asset::{self, Asset, AttachmentForAsset, RefusalReason};
use poph_matrix::client::MatrixClient;
use poph_matrix::error::{CallError, Classification};
use poph_matrix::format::{
    self, Decision, Meaning, MessageForMirror, MessageOrigin, MessageRole, NotMirrorReason,
    Outbound,
};
use poph_matrix::rooms::{
    ConversationToRoom, PortError, Provisioner, RoomError, RoomStored, RoomsPort, StoredState,
};
use poph_matrix::spaces::{DepartmentSpace, Position, RoomForSpace, Spaces, resolve_department};
use poph_matrix::users::{Ghost, GhostIds, GhostsError};
use poph_store::{
    AttachmentRecord, CategoryAsset, ConversationStatus, Direction, MessageSince, NewAsset, Origin,
    Role, StateRoom, Store, StoreError,
};
use poph_types::{AgentId, ConversationId, MatrixConfig, MessageId, OrgId};
use serde_json::json;

use crate::matrix_beginning::SpaceStorePort;
use crate::matrix_metrics::MetricsMatrix;

pub const PAGE: u32 = 200;

pub const MAX_PAGES_PER_CYCLE: u32 = 25;

pub const DEFINITIVE_ATTEMPTS: u32 = 3;

pub const MAX_DELAY: Duration = Duration::from_secs(60);

const FORMAT_REASON: &str = "mensagem_invalida";
const ROOM_REASON: &str = "sala_rejeitada";
const SEND_REASON: &str = "envio_rejeitado";
const DEVICE_REASON: &str = "aparelho_fora_da_sala";
const ASSET_REASON: &str = "midia_invalida";
const REASON_LIMIT: usize = 64;

pub trait MatrixTarget: Send + Sync {
    fn ensure_room(
        &self,
        data: &ConversationToRoom,
    ) -> impl Future<Output = Result<String, RoomError>> + Send;

    fn ensure_device(
        &self,
        data: &ConversationToRoom,
        room: &str,
        device: &Ghost,
    ) -> impl Future<Output = Result<(), RoomError>> + Send;

    fn send(
        &self,
        room: &str,
        send: &Outbound,
    ) -> impl Future<Output = Result<(), CallError>> + Send;

    fn send_asset(
        &self,
        room: &str,
        send: &AssetSend,
    ) -> impl Future<Output = Result<PublishedAsset, CallError>> + Send;

    fn send_notice(
        &self,
        room: &str,
        notice: &RoomNotice,
    ) -> impl Future<Output = Result<(), CallError>> + Send;
}

pub struct AssetSend {
    pub conversation: ConversationId,
    pub sender: Ghost,
    pub asset: Asset,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedAsset {
    pub event: String,
    pub mxc: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomNotice {
    pub conversation: ConversationId,
    pub sender: Ghost,
    pub txn_id: String,
    pub reason: RefusalReason,
}

pub struct SynapseMirrorTarget {
    client: MatrixClient,
    provisioner: Provisioner<RoomStorePort>,
    store: Arc<Store>,
    spaces: Option<Spaces<SpaceStorePort>>,
    metrics: Arc<MetricsMatrix>,
}

impl SynapseMirrorTarget {
    pub fn assemble(
        store: Arc<Store>,
        client: MatrixClient,
        config: &MatrixConfig,
    ) -> Result<Self, GhostsError> {
        let provisioner =
            Provisioner::of_config(client.clone(), RoomStorePort(Arc::clone(&store)), config)?;
        Ok(Self {
            client,
            provisioner,
            store,
            spaces: None,
            metrics: MetricsMatrix::unregistered(),
        })
    }

    pub fn with_spaces(mut self, spaces: Spaces<SpaceStorePort>) -> Self {
        self.spaces = Some(spaces);
        self
    }

    pub fn with_metrics(mut self, metrics: Arc<MetricsMatrix>) -> Self {
        self.metrics = metrics;
        self
    }
    async fn place_new_room(&self, data: &ConversationToRoom, room: &str) {
        let Some(spaces) = &self.spaces else {
            return;
        };
        let department = match self.conversation_department(data) {
            Ok(department) => department,
            Err(reason) => {
                tracing::warn!(
                    target: "poph_server::matrix_espelho",
                    org = %data.org,
                    conversation = %data.conversation,
                    reason,
                    "não consegui ler o departamento da conversa; a sala fica sem espaço por ora"
                );
                return;
            }
        };
        let Some(department) = department else {
            return;
        };
        let agent = match self.provisioner.agent_ghost(data).await {
            Ok(agent) => agent,
            Err(_) => {
                tracing::warn!(
                    target: "poph_server::matrix_espelho",
                    org = %data.org,
                    conversation = %data.conversation,
                    reason = "fantasma_do_agente_indisponivel",
                    "sala criada mas não posicionada no espaço do departamento"
                );
                return;
            }
        };
        let request = RoomForSpace {
            org: data.org,
            conversation: data.conversation,
            room: room.to_owned(),
            agent,
            department: Some(department),
            existing: None,
        };
        let started_at = Instant::now();
        let result = spaces.place(&request).await;
        self.metrics
            .latency_synapse_seconds
            .observe(started_at.elapsed().as_secs_f64());
        match result {
            Ok(Position::Positioned {
                failed_invitations, ..
            }) if failed_invitations > 0 => {
                tracing::warn!(
                    target: "poph_server::matrix_espelho",
                    org = %data.org,
                    conversation = %data.conversation,
                    failed_invitations,
                    reason = "convite_pendente_no_espaco",
                    "sala criada e posta no espaço, mas com convite pendente"
                );
            }
            Ok(_) => {
                tracing::info!(
                    target: "poph_server::matrix_espelho",
                    org = %data.org,
                    conversation = %data.conversation,
                    "sala criada já dentro do espaço do departamento"
                );
            }
            Err(error) => {
                match error.classification() {
                    Classification::Transient => self.metrics.transient_errors.increment(1),
                    Classification::Definitive => self.metrics.definitive_errors.increment(1),
                }
                tracing::warn!(
                    target: "poph_server::matrix_espelho",
                    org = %data.org,
                    conversation = %data.conversation,
                    reason = "sala_nao_posicionada_no_espaco",
                    "sala criada mas o espaço do departamento não aceitou a sala; tenta na próxima alteração da conversa"
                );
            }
        }
    }

    fn conversation_department(
        &self,
        data: &ConversationToRoom,
    ) -> Result<Option<DepartmentSpace>, &'static str> {
        let conversation = self
            .store
            .conversations()
            .get(data.org, data.conversation)
            .map_err(|_| "conversa_indisponivel")?;
        let defaults = self
            .store
            .agents()
            .get(data.org, conversation.agent_id)
            .ok()
            .and_then(|agent| agent.default_department);
        let Some(chosen) = resolve_department(conversation.target_department, defaults) else {
            return Ok(None);
        };
        match self.store.departments().get(data.org, chosen) {
            Ok(department) => Ok(Some(DepartmentSpace {
                id: department.id,
                name: department.name,
            })),
            Err(_) => Err("departamento_indisponivel"),
        }
    }
}

impl MatrixTarget for SynapseMirrorTarget {
    async fn ensure_room(&self, data: &ConversationToRoom) -> Result<String, RoomError> {
        let started_at = Instant::now();
        let result = self.provisioner.provision(data).await;
        self.metrics
            .latency_synapse_seconds
            .observe(started_at.elapsed().as_secs_f64());
        let ready = result?;
        if !ready.was_already_ready {
            self.metrics.ready_rooms.increment(1);
            self.place_new_room(data, &ready.room_id).await;
        }
        Ok(ready.room_id)
    }

    async fn ensure_device(
        &self,
        data: &ConversationToRoom,
        room: &str,
        device: &Ghost,
    ) -> Result<(), RoomError> {
        self.provisioner
            .ensure_device_in_room(data, room, device)
            .await
    }

    async fn send(&self, room: &str, send: &Outbound) -> Result<(), CallError> {
        let body = json!({ "msgtype": send.msgtype, "body": send.body });
        let started_at = Instant::now();
        let result = self
            .client
            .send_event(
                send.sender.user_id(),
                room,
                send.event_kind,
                &send.txn_id,
                &body,
            )
            .await
            .map(|_| ());
        self.metrics
            .latency_synapse_seconds
            .observe(started_at.elapsed().as_secs_f64());
        result
    }

    async fn send_asset(&self, room: &str, send: &AssetSend) -> Result<PublishedAsset, CallError> {
        let started_at = Instant::now();
        let result = asset::send(&self.client, &send.sender, room, &send.asset, &send.content)
            .await
            .map(|published| PublishedAsset {
                event: published.event,
                mxc: published.mxc,
            });
        self.metrics
            .latency_synapse_seconds
            .observe(started_at.elapsed().as_secs_f64());
        result
    }

    async fn send_notice(&self, room: &str, notice: &RoomNotice) -> Result<(), CallError> {
        let body = asset::notice_content(notice.reason);
        let started_at = Instant::now();
        let result = self
            .client
            .send_event(
                notice.sender.user_id(),
                room,
                format::EVENT_KIND,
                &notice.txn_id,
                &body,
            )
            .await
            .map(|_| ());
        self.metrics
            .latency_synapse_seconds
            .observe(started_at.elapsed().as_secs_f64());
        result
    }
}

pub struct RoomStorePort(pub Arc<Store>);

impl RoomsPort for RoomStorePort {
    fn room(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> impl Future<Output = Result<Option<RoomStored>, PortError>> + Send {
        let store = Arc::clone(&self.0);
        async move {
            let room = store.matrix().room(org, conversation).map_err(port_error)?;
            Ok(room.map(|room| RoomStored {
                room_id: room.room_id,
                state: stored_room_state(room.state),
            }))
        }
    }

    fn ensure_room(
        &self,
        org: OrgId,
        conversation: ConversationId,
        room_id: &str,
    ) -> impl Future<Output = Result<RoomStored, PortError>> + Send {
        let store = Arc::clone(&self.0);
        let room_id = room_id.to_owned();
        async move {
            let room = store
                .matrix()
                .ensure_room(org, conversation, &room_id, None)
                .map_err(port_error)?;
            Ok(RoomStored {
                room_id: room.room_id,
                state: stored_room_state(room.state),
            })
        }
    }

    fn mark_ready(
        &self,
        org: OrgId,
        conversation: ConversationId,
        mirrored_name: Option<&str>,
    ) -> impl Future<Output = Result<RoomStored, PortError>> + Send {
        let store = Arc::clone(&self.0);
        let name = mirrored_name.map(str::to_owned);
        async move {
            let room = store
                .matrix()
                .mark_room_ready_with_name(org, conversation, name.as_deref())
                .map_err(port_error)?;
            Ok(RoomStored {
                room_id: room.room_id,
                state: stored_room_state(room.state),
            })
        }
    }

    fn had_messages_before(
        &self,
        org: OrgId,
        conversation: ConversationId,
        cutoff: i64,
    ) -> impl Future<Output = Result<bool, PortError>> + Send {
        let store = Arc::clone(&self.0);
        async move {
            store
                .messages()
                .had_messages_before(org, conversation, cutoff)
                .map_err(port_error)
        }
    }
}

fn port_error(_: StoreError) -> PortError {
    PortError::fresh("armazenamento do módulo Matrix indisponível")
}

fn stored_room_state(state: StateRoom) -> StoredState {
    match state {
        StateRoom::Creating => StoredState::Creating,
        StateRoom::Ready => StoredState::Ready,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MirrorError {
    #[error("armazenamento do espelho do Matrix falhou: {0}")]
    Armazenamento(#[from] StoreError),
}

impl MirrorError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Armazenamento(_) => "armazenamento_indisponivel",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MirrorReport {
    pub sent: u32,
    pub assets_sent: u32,
    pub asset_notices: u32,
    pub not_mirrored: u32,
    pub already_mirrored: u32,
    pub from_disabled_org: u32,
    pub before_cutoff: u32,
    pub isolated: u32,
    pub pending: u32,
    pub rewinds: u32,
    pub pages: u32,
    pub largest_page: u32,
}

struct EnabledOrg {
    cutoff: i64,
    cursor: (u64, String),
}

#[derive(Clone)]
struct ConversationData {
    identifier: String,
    agent: String,
    integration: Option<String>,
    room: ConversationToRoom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MirrorProgress {
    Resolved,
    Pending,
}

enum DeliveryOutcome {
    Sent,
    Pending,
    Isolate { reason: String, attempts: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepKind {
    Text,
    Asset,
    Notice,
}

enum MirrorStep {
    Text(Outbound),
    Asset { send: AssetSend, notice: RoomNotice },
    Notice(RoomNotice),
}

impl MirrorStep {
    fn kind(&self) -> StepKind {
        match self {
            Self::Text(_) => StepKind::Text,
            Self::Asset { .. } => StepKind::Asset,
            Self::Notice(_) => StepKind::Notice,
        }
    }
}

struct GroupItem {
    index: usize,
    org: OrgId,
    id: MessageId,
    steps: Vec<MirrorStep>,
}

struct RoomGroup {
    data: ConversationToRoom,
    items: Vec<GroupItem>,
}

struct GroupOutcome {
    index: usize,
    org: OrgId,
    id: MessageId,
    kind: StepKind,
    outcome: DeliveryOutcome,
}

struct PageOutcome {
    prefix: Option<(u64, MessageId)>,
    pending: bool,
}

pub struct MatrixMirror<D: MatrixTarget> {
    store: Arc<Store>,
    destination: D,
    ids: GhostIds,
    concurrency: usize,
    interval: Duration,
    page: u32,
    definitive_failures: Mutex<HashMap<MessageId, u32>>,
    consecutive_failures: AtomicU32,
    stop: tokio::sync::watch::Sender<bool>,
    metrics: Arc<MetricsMatrix>,
    now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
    blob_folder: Option<PathBuf>,
}

impl<D: MatrixTarget> MatrixMirror<D> {
    pub fn new(
        store: Arc<Store>,
        destination: D,
        config: &MatrixConfig,
    ) -> Result<Self, GhostsError> {
        let ids = GhostIds::of_config(config)?;
        let (stop, _) = tokio::sync::watch::channel(false);
        Ok(Self {
            store,
            destination,
            ids,
            concurrency: (config.concurrency as usize).max(1),
            interval: Duration::from_millis(config.interval_ms.max(1)),
            page: PAGE,
            definitive_failures: Mutex::new(HashMap::new()),
            consecutive_failures: AtomicU32::new(0),
            stop,
            metrics: MetricsMatrix::unregistered(),
            now_ms: Arc::new(system_clock),
            blob_folder: None,
        })
    }

    pub fn with_metrics(mut self, metrics: Arc<MetricsMatrix>) -> Self {
        self.metrics = metrics;
        self
    }

    pub fn with_clock(mut self, now_ms: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        self.now_ms = now_ms;
        self
    }

    pub fn with_blob_folder(mut self, folder: impl Into<PathBuf>) -> Self {
        self.blob_folder = Some(folder.into());
        self
    }

    pub fn request_stop(&self) {
        self.stop.send_replace(true);
    }

    pub fn delay_after(&self, consecutive_failures: u32) -> Duration {
        if consecutive_failures == 0 {
            return self.interval;
        }
        let factor = 1u32
            .checked_shl(consecutive_failures.min(10))
            .unwrap_or(u32::MAX);
        self.interval.saturating_mul(factor).min(MAX_DELAY)
    }

    pub async fn run(self: Arc<Self>) {
        let mut stop = self.stop.subscribe();
        loop {
            if *stop.borrow() {
                break;
            }
            let failures = match self.cycle().await {
                Ok(report) => {
                    tracing::debug!(
                        target: "poph_server::matrix_espelho",
                        sent = report.sent,
                        pending = report.pending,
                        isolated = report.isolated,
                        "ciclo do espelho do Matrix"
                    );
                    if report.pending > 0 {
                        self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1
                    } else {
                        self.consecutive_failures.store(0, Ordering::Relaxed);
                        0
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        target: "poph_server::matrix_espelho",
                        reason = error.code(),
                        "ciclo do espelho do Matrix falhou; segue no próximo"
                    );
                    self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1
                }
            };
            tokio::select! {
                _ = tokio::time::sleep(self.delay_after(failures)) => {}
                _ = stop.changed() => break,
            }
        }
    }

    pub async fn cycle(&self) -> Result<MirrorReport, MirrorError> {
        let mut report = MirrorReport::default();
        let enabled = self.store.matrix().linked_orgs()?;
        if enabled.is_empty() {
            self.metrics.cursor_age_seconds.set(0);
            return Ok(report);
        }

        let mut orgs: HashMap<OrgId, EnabledOrg> = HashMap::new();
        for state in enabled.iter().filter(|state| state.linked) {
            let cursors = self.store.matrix().read_cursors(state.org_id)?;
            orgs.insert(
                state.org_id,
                EnabledOrg {
                    cutoff: state.linked_at.unwrap_or(0),
                    cursor: (cursors.messages, cursors.message_ids),
                },
            );
        }
        if orgs.is_empty() {
            self.metrics.cursor_age_seconds.set(0);
            return Ok(report);
        }

        let (highest_rowid, highest_id) = match self.store.messages().last_rowid()? {
            Some((rowid, id)) => (rowid, id.to_string()),
            None => (0, String::new()),
        };
        for (org, info) in orgs.iter_mut() {
            if info.cursor.0 > highest_rowid {
                self.store
                    .matrix()
                    .rewind_cursor_messages(*org, highest_rowid, &highest_id)?;
                info.cursor = (highest_rowid, highest_id.clone());
                report.rewinds += 1;
            }
        }

        let lowest_rowid = orgs.values().map(|info| info.cursor.0).min().unwrap_or(0);
        let mut position = lowest_rowid.saturating_sub(1);

        let mut pages = 0u32;
        while pages < MAX_PAGES_PER_CYCLE && !*self.stop.borrow() {
            let page = self.store.messages().since(position, self.page)?;
            if page.is_empty() {
                break;
            }
            pages += 1;
            report.pages += 1;
            report.largest_page = report.largest_page.max(page.len() as u32);

            let last = page.last().map(|m| m.rowid).unwrap_or(position);
            let last_page = page.len() < self.page as usize;

            let result = self.process_page(&orgs, page, &mut report).await?;

            if let Some((rowid, id)) = &result.prefix {
                let text_id = id.to_string();
                for (org, info) in orgs.iter_mut() {
                    if *rowid > info.cursor.0 {
                        self.store
                            .matrix()
                            .advance_messages_with_id(*org, *rowid, &text_id)?;
                        info.cursor = (*rowid, text_id.clone());
                    }
                }
            }

            if result.pending {
                break;
            }
            position = last;
            if last_page {
                break;
            }
        }
        self.measure_delay(&orgs)?;
        Ok(report)
    }

    fn measure_delay(&self, orgs: &HashMap<OrgId, EnabledOrg>) -> Result<(), MirrorError> {
        let Some(lowest_rowid) = orgs.values().map(|info| info.cursor.0).min() else {
            self.metrics.cursor_age_seconds.set(0);
            return Ok(());
        };
        let delay_ms = match self.store.messages().since(lowest_rowid, 1)?.first() {
            Some(first) => ((self.now_ms)() - first.created_at).max(0),
            None => 0,
        };
        self.metrics.cursor_age_seconds.set(delay_ms / 1000);
        Ok(())
    }

    async fn process_page(
        &self,
        orgs: &HashMap<OrgId, EnabledOrg>,
        page: Vec<MessageSince>,
        report: &mut MirrorReport,
    ) -> Result<PageOutcome, MirrorError> {
        let mut states = vec![MirrorProgress::Resolved; page.len()];
        let mut groups: Vec<RoomGroup> = Vec::new();
        let mut group_index: HashMap<ConversationId, usize> = HashMap::new();
        let mut conversations: HashMap<ConversationId, Option<ConversationData>> = HashMap::new();

        for (i, message) in page.iter().enumerate() {
            let Some(org) = orgs.get(&message.org_id) else {
                report.from_disabled_org += 1;
                continue;
            };
            if message.rowid < org.cursor.0
                || (message.rowid == org.cursor.0 && message.id.to_string() == org.cursor.1)
            {
                report.already_mirrored += 1;
                continue;
            }
            if message.created_at < org.cutoff {
                report.before_cutoff += 1;
                continue;
            }

            let loaded = conversations
                .entry(message.conversation_id)
                .or_insert_with(|| {
                    self.conversation_data(message.org_id, message.conversation_id, org.cutoff)
                        .ok()
                })
                .clone();
            let Some(data) = loaded else {
                states[i] = MirrorProgress::Pending;
                continue;
            };

            if matches!(data.room.channel.as_str(), "site" | "popbot") {
                continue;
            }

            let attachments = match self.customer_attachments(message) {
                Ok(attachments) => attachments,
                Err(_) => {
                    states[i] = MirrorProgress::Pending;
                    continue;
                }
            };
            let mut steps: Vec<MirrorStep> = Vec::new();
            let mut invalid_asset = false;
            for attachment in &attachments {
                match self.asset_step(message.org_id, &data, attachment).await {
                    Ok(step) => steps.push(step),
                    Err(_) => {
                        invalid_asset = true;
                        break;
                    }
                }
            }
            if invalid_asset {
                steps.clear();
            }

            let decision = format::decide(
                &self.ids,
                &MessageForMirror {
                    id: message.id,
                    conversation: message.conversation_id,
                    origin: message_origin(message.origin),
                    meaning: message_direction(message.direction),
                    role: message_role(message.role),
                    text: &message.texto,
                    client_identifier: &data.identifier,
                    id_agent: &data.agent,
                    id_integration: data.integration.as_deref(),
                },
            );
            let failure_reason = match decision {
                Decision::NotMirror(NotMirrorReason::TextEmpty) if !steps.is_empty() => None,
                Decision::NotMirror(_) => {
                    steps.clear();
                    report.not_mirrored += 1;
                    None
                }
                Decision::Error(_) => {
                    steps.clear();
                    Some(if invalid_asset {
                        ASSET_REASON
                    } else {
                        FORMAT_REASON
                    })
                }
                Decision::Mirror(send) => {
                    steps.push(MirrorStep::Text(send));
                    None
                }
            };
            if let Some(reason) = failure_reason {
                match self
                    .store
                    .matrix()
                    .register_failure(message.org_id, message.id, reason, 1)
                {
                    Ok(_) => {
                        report.isolated += 1;
                        self.metrics.isolated_messages.increment(1);
                        tracing::warn!(
                            target: "poph_server::matrix_espelho",
                            org = %message.org_id,
                            conversation = %message.conversation_id,
                            reason,
                            "mensagem fora do formato do espelho: isolada em matrix_falhas"
                        );
                    }
                    Err(_) => states[i] = MirrorProgress::Pending,
                }
                continue;
            }
            if steps.is_empty() {
                continue;
            }
            let position = match group_index.get(&message.conversation_id) {
                Some(position) => *position,
                None => {
                    groups.push(RoomGroup {
                        data: data.room.clone(),
                        items: Vec::new(),
                    });
                    group_index.insert(message.conversation_id, groups.len() - 1);
                    groups.len() - 1
                }
            };
            groups[position].items.push(GroupItem {
                index: i,
                org: message.org_id,
                id: message.id,
                steps,
            });
        }

        let outcomes: Vec<Vec<GroupOutcome>> = futures::stream::iter(groups)
            .map(|group| self.deliver_group(group))
            .buffer_unordered(self.concurrency)
            .collect()
            .await;

        for outcome in outcomes.into_iter().flatten() {
            let GroupOutcome {
                index,
                org,
                id,
                kind,
                outcome,
            } = outcome;
            match outcome {
                DeliveryOutcome::Sent => {
                    states[index] = MirrorProgress::Resolved;
                    match kind {
                        StepKind::Text => {
                            report.sent += 1;
                            self.metrics.mirrored_messages.increment(1);
                        }
                        StepKind::Asset => {
                            report.assets_sent += 1;
                            self.metrics.mirrored_assets.increment(1);
                        }
                        StepKind::Notice => report.asset_notices += 1,
                    }
                }
                DeliveryOutcome::Pending => {
                    states[index] = MirrorProgress::Pending;
                }
                DeliveryOutcome::Isolate { reason, attempts } => match self
                    .store
                    .matrix()
                    .register_failure(org, id, &reason, attempts)
                {
                    Ok(_) => {
                        states[index] = MirrorProgress::Resolved;
                        report.isolated += 1;
                        self.metrics.isolated_messages.increment(1);
                    }
                    Err(_) => states[index] = MirrorProgress::Pending,
                },
            }
        }

        let mut prefix = None;
        let mut pending = false;
        for (i, message) in page.iter().enumerate() {
            if states[i] == MirrorProgress::Pending {
                pending = true;
                break;
            }
            prefix = Some((message.rowid, message.id));
        }
        report.pending += states
            .iter()
            .filter(|e| **e == MirrorProgress::Pending)
            .count() as u32;

        Ok(PageOutcome { prefix, pending })
    }

    async fn deliver_group(&self, group: RoomGroup) -> Vec<GroupOutcome> {
        let mut output = Vec::with_capacity(group.items.len());
        let mut items = group.items.into_iter();

        let conversation = group.data.conversation;
        let conversation_agent = group.data.agent.to_string();
        self.metrics.attempts.increment(1);
        let room = match self.destination.ensure_room(&group.data).await {
            Ok(room) => room,
            Err(error) => {
                if room_call_error(&error).is_some_and(is_rate_limited) {
                    self.metrics.responses_429.increment(1);
                }
                let reason = ROOM_REASON.to_owned();
                let first = items.next();
                if let Some(item) = first {
                    let kind = item
                        .steps
                        .first()
                        .map(MirrorStep::kind)
                        .unwrap_or(StepKind::Text);
                    let outcome = self.outcome_for(
                        item.org,
                        conversation,
                        &item.id,
                        error.classification(),
                        reason,
                    );
                    output.push(GroupOutcome {
                        index: item.index,
                        org: item.org,
                        id: item.id,
                        kind,
                        outcome,
                    });
                }
                for item in items {
                    output.push(GroupOutcome {
                        index: item.index,
                        org: item.org,
                        id: item.id,
                        kind: StepKind::Text,
                        outcome: DeliveryOutcome::Pending,
                    });
                }
                return output;
            }
        };

        let mut device_in_room = false;
        while let Some(item) = items.next() {
            for step in item.steps {
                self.metrics.attempts.increment(1);
                let kind = step.kind();
                if let MirrorStep::Text(send) = &step
                    && send.origin == MessageOrigin::Device
                    && !device_in_room
                {
                    match self
                        .destination
                        .ensure_device(&group.data, &room, &send.sender)
                        .await
                    {
                        Ok(()) => device_in_room = true,
                        Err(error) => {
                            let outcome = self.outcome_for(
                                item.org,
                                conversation,
                                &item.id,
                                error.classification(),
                                DEVICE_REASON.to_owned(),
                            );
                            output.push(GroupOutcome {
                                index: item.index,
                                org: item.org,
                                id: item.id,
                                kind,
                                outcome,
                            });
                            self.remaining_pending(&mut items, &mut output);
                            return output;
                        }
                    }
                }

                let result = match &step {
                    MirrorStep::Text(send) => {
                        self.destination.send(&room, send).await.map(|()| None)
                    }
                    MirrorStep::Asset { send, .. } => {
                        self.destination.send_asset(&room, send).await.map(Some)
                    }
                    MirrorStep::Notice(notice) => self
                        .destination
                        .send_notice(&room, notice)
                        .await
                        .map(|()| None),
                };

                match result {
                    Ok(published) => {
                        if let (MirrorStep::Asset { send, .. }, Some(published)) =
                            (&step, &published)
                        {
                            self.record_asset(
                                item.org,
                                &conversation_agent,
                                &room,
                                send,
                                published,
                            );
                        }
                        self.forget(&item.id);
                        output.push(GroupOutcome {
                            index: item.index,
                            org: item.org,
                            id: item.id,
                            kind,
                            outcome: DeliveryOutcome::Sent,
                        });
                    }
                    Err(error) => {
                        if is_rate_limited(&error) {
                            self.metrics.responses_429.increment(1);
                        }
                        let reason = send_failure_reason(&error);
                        let outcome = self.outcome_for(
                            item.org,
                            conversation,
                            &item.id,
                            error.classification(),
                            reason,
                        );
                        if let (DeliveryOutcome::Isolate { .. }, MirrorStep::Asset { notice, .. }) =
                            (&outcome, &step)
                        {
                            self.write_notice(&room, notice).await;
                        }
                        output.push(GroupOutcome {
                            index: item.index,
                            org: item.org,
                            id: item.id,
                            kind,
                            outcome,
                        });
                        self.remaining_pending(&mut items, &mut output);
                        return output;
                    }
                }
            }
        }
        output
    }

    fn remaining_pending(
        &self,
        items: &mut std::vec::IntoIter<GroupItem>,
        output: &mut Vec<GroupOutcome>,
    ) {
        for item in items.by_ref() {
            output.push(GroupOutcome {
                index: item.index,
                org: item.org,
                id: item.id,
                kind: StepKind::Text,
                outcome: DeliveryOutcome::Pending,
            });
        }
    }

    async fn write_notice(&self, room: &str, notice: &RoomNotice) {
        if let Err(error) = self.destination.send_notice(room, notice).await {
            tracing::warn!(
                target: "poph_server::matrix_espelho",
                conversation = %notice.conversation,
                reason = notice.reason.code(),
                classification = %error.classification(),
                "aviso de mídia recusada não entrou na sala; a mensagem foi isolada do mesmo jeito"
            );
        }
    }

    fn outcome_for(
        &self,
        org: OrgId,
        conversation: ConversationId,
        id: &MessageId,
        classification: Classification,
        reason: String,
    ) -> DeliveryOutcome {
        match classification {
            Classification::Transient => {
                self.metrics.transient_errors.increment(1);
                tracing::debug!(
                    target: "poph_server::matrix_espelho",
                    org = %org,
                    conversation = %conversation,
                    reason = %reason,
                    classification = "transitoria",
                    "falha no espelho do Matrix: a mensagem fica pendente"
                );
                DeliveryOutcome::Pending
            }
            Classification::Definitive => {
                self.metrics.definitive_errors.increment(1);
                let attempts = {
                    let mut map = self
                        .definitive_failures
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    let count = map.entry(*id).or_insert(0);
                    *count += 1;
                    *count
                };
                if attempts >= DEFINITIVE_ATTEMPTS {
                    self.forget(id);
                    tracing::warn!(
                        target: "poph_server::matrix_espelho",
                        org = %org,
                        conversation = %conversation,
                        reason = %reason,
                        attempts,
                        classification = "definitiva",
                        "falha definitiva repetida no espelho do Matrix: mensagem isolada"
                    );
                    DeliveryOutcome::Isolate { reason, attempts }
                } else {
                    tracing::info!(
                        target: "poph_server::matrix_espelho",
                        org = %org,
                        conversation = %conversation,
                        reason = %reason,
                        attempts,
                        classification = "definitiva",
                        "falha definitiva no espelho do Matrix: nova tentativa no próximo ciclo"
                    );
                    DeliveryOutcome::Pending
                }
            }
        }
    }

    fn forget(&self, id: &MessageId) {
        let mut map = self
            .definitive_failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        map.remove(id);
    }

    fn customer_attachments(
        &self,
        message: &MessageSince,
    ) -> Result<Vec<AttachmentRecord>, StoreError> {
        if message.origin != Origin::Agente
            || message.role != Role::User
            || message.direction != Direction::Inbound
        {
            return Ok(Vec::new());
        }
        let ids: Vec<String> =
            serde_json::from_str(&message.attachments_json).map_err(|error| {
                StoreError::Internal(format!(
                    "lista de anexos da mensagem não lê como JSON: {error}"
                ))
            })?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.store
            .messages()
            .attachments(message.org_id, message.id)
    }

    async fn asset_step(
        &self,
        org: OrgId,
        data: &ConversationData,
        attachment: &AttachmentRecord,
    ) -> Result<MirrorStep, ()> {
        let id_attachment = attachment.id.to_string();
        let sender = self.ids.client(&data.identifier).map_err(|_| ())?;
        let decided = asset::decide(&AttachmentForAsset {
            id: &id_attachment,
            name: &attachment.name,
            mime: &attachment.mime,
            size_bytes: attachment.size_bytes,
            source: &data.room.channel,
        });
        let decided = match decided {
            Ok(decided) => decided,
            Err(reason) => {
                return Ok(MirrorStep::Notice(self.asset_notice(
                    data,
                    &id_attachment,
                    reason,
                )?));
            }
        };
        let content = match self.read_blob(org, attachment).await {
            Ok(content) => content,
            Err(reason) => {
                return Ok(MirrorStep::Notice(self.asset_notice(
                    data,
                    &id_attachment,
                    reason,
                )?));
            }
        };
        Ok(MirrorStep::Asset {
            send: AssetSend {
                conversation: data.room.conversation,
                sender,
                asset: decided,
                content,
            },
            notice: self.asset_notice(data, &id_attachment, RefusalReason::Rejeitada)?,
        })
    }

    fn record_asset(
        &self,
        org: OrgId,
        agent: &str,
        room: &str,
        send: &AssetSend,
        published: &PublishedAsset,
    ) {
        let asset = &send.asset;
        let record = NewAsset {
            id: asset
                .id_attachment
                .parse()
                .unwrap_or_else(|_| poph_types::AttachmentId::generate()),
            conversation: send.conversation,
            agent: agent.to_owned(),
            room: room.to_owned(),
            evento: published.event.clone(),
            mxc: published.mxc.clone(),
            nome: asset.name.clone(),
            mime: asset.mime.clone(),
            category: if asset.msgtype == asset::MSGTYPE_IMAGE {
                CategoryAsset::Image
            } else {
                CategoryAsset::Document
            },
            size_bytes: asset.size_bytes,
            sent_at: (self.now_ms)(),
        };
        if let Err(error) = self.store.matrix_asset().register(org, &record) {
            tracing::warn!(
                target: "poph_server::matrix_espelho",
                org = %org,
                conversation = %send.conversation,
                reason = "midia_nao_registrada",
                "mídia entrou na sala mas não foi registrada para expirar: o arquivo fica no Synapse"
            );
            let _ = error;
        }
    }

    fn asset_notice(
        &self,
        data: &ConversationData,
        id_attachment: &str,
        reason: RefusalReason,
    ) -> Result<RoomNotice, ()> {
        let sender = self.ids.agent(&data.agent).map_err(|_| ())?;
        Ok(RoomNotice {
            conversation: data.room.conversation,
            sender,
            txn_id: asset::notice_txn(id_attachment, reason),
            reason,
        })
    }

    async fn read_blob(
        &self,
        org: OrgId,
        attachment: &AttachmentRecord,
    ) -> Result<Vec<u8>, RefusalReason> {
        let Some(folder) = self.blob_folder.clone() else {
            return Err(RefusalReason::Unreadable);
        };
        let sha256 = attachment.sha256.clone();
        let limit = asset::ASSET_BYTES_LIMIT;
        tokio::task::spawn_blocking(move || {
            poph_files::blob::read(&folder, org, &sha256, limit)
                .map_err(|_| RefusalReason::Unreadable)
        })
        .await
        .map_err(|_| RefusalReason::Unreadable)?
    }

    fn conversation_data(
        &self,
        org: OrgId,
        conversation: ConversationId,
        cutoff: i64,
    ) -> Result<ConversationData, StoreError> {
        let loaded_conversation = self.store.conversations().get(org, conversation)?;
        let identifier = loaded_conversation.identifier_client();
        let agent_name = self
            .store
            .agents()
            .get(org, loaded_conversation.agent_id)
            .ok()
            .map(|agent| agent.name);
        let agent: AgentId = loaded_conversation.agent_id;
        let room = ConversationToRoom {
            org,
            conversation,
            client: identifier.clone(),
            client_name: loaded_conversation.customer_name.clone(),
            agent,
            agent_name,
            channel: loaded_conversation.canal.clone(),
            department: loaded_conversation.target_department,
            closed: loaded_conversation.status == ConversationStatus::Closed,
            backfill_cutoff: Some(cutoff),
        };
        Ok(ConversationData {
            identifier,
            agent: agent.to_string(),
            integration: loaded_conversation.integration_id.map(|id| id.to_string()),
            room,
        })
    }
}

fn message_origin(origin: Origin) -> MessageOrigin {
    match origin {
        Origin::Agente => MessageOrigin::Agent,
        Origin::Human => MessageOrigin::Human,
        Origin::Device => MessageOrigin::Device,
    }
}

fn message_direction(direction: Direction) -> Meaning {
    match direction {
        Direction::Inbound => Meaning::Entry,
        Direction::Outbound => Meaning::Output,
    }
}

fn message_role(role: Role) -> MessageRole {
    match role {
        Role::User => MessageRole::Client,
        Role::Assistant => MessageRole::Agent,
        Role::Tool | Role::System => MessageRole::Other,
    }
}

fn send_failure_reason(error: &CallError) -> String {
    match error {
        CallError::Response {
            errcode: Some(code),
            ..
        } if safe_code(code) => code.clone(),
        _ => SEND_REASON.to_owned(),
    }
}

fn room_call_error(error: &RoomError) -> Option<&CallError> {
    match error {
        RoomError::Call(call) | RoomError::Ghosts(GhostsError::Call(call)) => Some(call),
        _ => None,
    }
}

fn is_rate_limited(error: &CallError) -> bool {
    matches!(error, CallError::Response { status: 429, .. })
}

pub(crate) fn system_clock() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn safe_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= REASON_LIMIT
        && code
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}
