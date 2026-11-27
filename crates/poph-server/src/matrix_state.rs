use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poph_matrix::RoomMember;
use poph_matrix::client::MatrixClient;
use poph_matrix::error::{CallError, Classification};
use poph_matrix::rooms::{ConversationToRoom, METADATA_KIND, NAME_KIND, RoomMetadata};
use poph_matrix::spaces::{
    DepartmentSpace, Position, RoomForSpace, SpaceError, Spaces, SpacesPort, resolve_department,
};
use poph_matrix::users::{GhostIds, GhostsError, VirtualUsers};
use poph_store::error::StoreError;
use poph_store::{ConversationStatus, StateRoom, Store};
use poph_types::{AgentId, ConversationId, MatrixConfig, OrgId};
use serde_json::json;

use crate::matrix_metrics::MetricsMatrix;

pub const PAGE: u32 = 200;

pub const MAX_PAGES_PER_CYCLE: u32 = 20;

pub const MAX_DELAY: Duration = Duration::from_secs(60);

pub const DEFAULT_SENDER_CAP: usize = 1024;

pub const NOTICE_TRANSFERRED: &str =
    "Atendimento humano assumiu a conversa: o agente automático está pausado até ela ser reaberta.";

pub const NOTICE_REOPENED: &str = "O agente automático retomou o atendimento desta conversa.";

pub const NOTICE_CLOSED: &str =
    "Conversa encerrada. Uma nova mensagem do cliente abre uma conversa nova.";

const MSGTYPE_NOTICE: &str = "m.notice";

const NOTICE_MARK: &str = "ph_est";

const MIN_ID: &str = "00000000-0000-0000-0000-000000000000";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrgState {
    pub org: OrgId,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversationChanged {
    pub org: OrgId,
    pub conversation: ConversationId,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredCursor {
    pub updated_at: i64,
    pub id: Option<ConversationId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationState {
    pub org: OrgId,
    pub conversation: ConversationId,
    pub status: ConversationStatus,
    pub agent: AgentId,
    pub agent_name: Option<String>,
    pub channel: String,
    pub client: String,
    pub client_name: Option<String>,
    pub department_target: Option<DepartmentSpace>,
    pub department_default: Option<DepartmentSpace>,
    pub updated_at: i64,
}

impl ConversationState {
    pub fn department(&self) -> Option<DepartmentSpace> {
        let target = self
            .department_target
            .as_ref()
            .map(|department| department.id);
        let default = self
            .department_default
            .as_ref()
            .map(|department| department.id);
        let chosen = resolve_department(target, default)?;
        self.department_target
            .iter()
            .chain(self.department_default.iter())
            .find(|department| department.id == chosen)
            .cloned()
    }

    pub fn to_room(&self, department: Option<DepartmentSpace>) -> ConversationToRoom {
        ConversationToRoom {
            org: self.org,
            conversation: self.conversation,
            client: self.client.clone(),
            client_name: self.client_name.clone(),
            agent: self.agent,
            agent_name: self.agent_name.clone(),
            channel: self.channel.clone(),
            department: department.map(|department| department.id),
            closed: self.status == ConversationStatus::Closed,
            backfill_cutoff: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomState {
    pub room_id: String,
    pub space: Option<String>,
    pub last_status: Option<ConversationStatus>,
    pub mirrored_name: Option<String>,
    pub ready: bool,
}

pub trait StatePort: Send + Sync {
    fn orgs(&self) -> impl Future<Output = Result<Vec<OrgState>, StoreError>> + Send;

    fn cursors(&self, org: OrgId) -> impl Future<Output = Result<StoredCursor, StoreError>> + Send;

    fn changed_items(
        &self,
        after: Option<(i64, ConversationId)>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<ConversationChanged>, StoreError>> + Send;

    fn conversation(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> impl Future<Output = Result<Option<ConversationState>, StoreError>> + Send;

    fn room(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> impl Future<Output = Result<Option<RoomState>, StoreError>> + Send;

    fn define_last_status(
        &self,
        org: OrgId,
        conversation: ConversationId,
        status: ConversationStatus,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    fn define_name_mirrored(
        &self,
        org: OrgId,
        conversation: ConversationId,
        name: &str,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    fn advance_conversations(
        &self,
        org: OrgId,
        updated_at: i64,
        id: &str,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

#[derive(Debug, Clone)]
pub struct StateStorePort {
    store: Arc<Store>,
}

impl StateStorePort {
    pub fn fresh(store: Arc<Store>) -> Self {
        Self { store }
    }
}

impl StatePort for StateStorePort {
    fn orgs(&self) -> impl Future<Output = Result<Vec<OrgState>, StoreError>> + Send {
        let store = Arc::clone(&self.store);
        async move {
            let states = store.matrix().linked_orgs()?;
            Ok(states
                .into_iter()
                .map(|state| OrgState {
                    org: state.org_id,
                    enabled: state.linked,
                })
                .collect())
        }
    }

    fn cursors(&self, org: OrgId) -> impl Future<Output = Result<StoredCursor, StoreError>> + Send {
        let store = Arc::clone(&self.store);
        async move {
            let cursors = store.matrix().read_cursors(org)?.conversations;
            Ok(StoredCursor {
                updated_at: cursors.updated_at,
                id: cursors.id.parse::<ConversationId>().ok(),
            })
        }
    }

    fn changed_items(
        &self,
        after: Option<(i64, ConversationId)>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<ConversationChanged>, StoreError>> + Send {
        let store = Arc::clone(&self.store);
        async move {
            let conversations = store.conversations().changed_since(after, limit)?;
            Ok(conversations
                .into_iter()
                .map(|conversation| ConversationChanged {
                    org: conversation.org_id,
                    conversation: conversation.id,
                    updated_at: conversation.updated_at,
                })
                .collect())
        }
    }

    fn conversation(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> impl Future<Output = Result<Option<ConversationState>, StoreError>> + Send {
        let store = Arc::clone(&self.store);
        async move {
            let read = match store.conversations().get(org, conversation) {
                Ok(read) => read,
                Err(error) if error.is_not_found() => return Ok(None),
                Err(error) => return Err(error),
            };
            let agent = store.agents().get(org, read.agent_id).ok();
            let department = |id: poph_types::DepartmentId| {
                store
                    .departments()
                    .get(org, id)
                    .ok()
                    .map(|department| DepartmentSpace {
                        id: department.id,
                        name: department.name,
                    })
            };
            let identifier = read.identifier_client();
            Ok(Some(ConversationState {
                org,
                conversation,
                status: read.status,
                agent: read.agent_id,
                agent_name: agent.as_ref().map(|agent| agent.name.clone()),
                channel: read.canal.clone(),
                client: identifier,
                client_name: read.customer_name.clone(),
                department_target: read.target_department.and_then(department),
                department_default: agent
                    .as_ref()
                    .and_then(|agent| agent.default_department)
                    .and_then(department),
                updated_at: read.updated_at,
            }))
        }
    }

    fn room(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> impl Future<Output = Result<Option<RoomState>, StoreError>> + Send {
        let store = Arc::clone(&self.store);
        async move {
            let room = store.matrix().room(org, conversation)?;
            Ok(room.map(|room| RoomState {
                room_id: room.room_id,
                space: room.space,
                last_status: room.last_status,
                mirrored_name: room.name_client_mirrored,
                ready: room.state == StateRoom::Ready,
            }))
        }
    }

    fn define_last_status(
        &self,
        org: OrgId,
        conversation: ConversationId,
        status: ConversationStatus,
    ) -> impl Future<Output = Result<(), StoreError>> + Send {
        let store = Arc::clone(&self.store);
        async move {
            store
                .matrix()
                .define_last_status(org, conversation, status)
                .map(|_| ())
        }
    }

    fn define_name_mirrored(
        &self,
        org: OrgId,
        conversation: ConversationId,
        name: &str,
    ) -> impl Future<Output = Result<(), StoreError>> + Send {
        let store = Arc::clone(&self.store);
        let name = name.to_owned();
        async move {
            store
                .matrix()
                .set_mirrored_customer_name(org, conversation, &name)
                .map(|_| ())
        }
    }

    fn advance_conversations(
        &self,
        org: OrgId,
        updated_at: i64,
        id: &str,
    ) -> impl Future<Output = Result<(), StoreError>> + Send {
        let store = Arc::clone(&self.store);
        let id = id.to_owned();
        async move {
            store
                .matrix()
                .advance_conversations(org, updated_at, &id)
                .map(|_| ())
        }
    }
}

pub trait StateTarget: Send + Sync {
    fn notify(
        &self,
        room: &str,
        agent: AgentId,
        txn: &str,
        text: &str,
    ) -> impl Future<Output = Result<(), CallError>> + Send;

    fn write_metadata(
        &self,
        room: &str,
        agent: AgentId,
        metadata: &RoomMetadata,
    ) -> impl Future<Output = Result<(), CallError>> + Send;

    fn place(
        &self,
        data: RoomForSpace,
    ) -> impl Future<Output = Result<Position, SpaceError>> + Send;

    fn rename(
        &self,
        room: &str,
        agent: AgentId,
        name: &str,
    ) -> impl Future<Output = Result<(), CallError>> + Send;

    fn rename_ghost(
        &self,
        client: &str,
        name: &str,
    ) -> impl Future<Output = Result<(), CallError>> + Send;
}

pub fn choose_sender(members: &BTreeMap<String, RoomMember>, bot: &str, agent: &str) -> String {
    if members.contains_key(bot) {
        bot.to_owned()
    } else {
        agent.to_owned()
    }
}

pub struct SynapseTarget<P: SpacesPort + Send + Sync> {
    client: MatrixClient,
    users: Arc<VirtualUsers>,
    ids: GhostIds,
    spaces: Spaces<P>,
    senders: Mutex<SenderCache>,
}

impl<P: SpacesPort + Send + Sync> SynapseTarget<P> {
    pub fn new(client: MatrixClient, users: Arc<VirtualUsers>, spaces: Spaces<P>) -> Self {
        let ids = users.ids().clone();
        Self {
            client,
            users,
            ids,
            spaces,
            senders: Mutex::new(SenderCache::new(DEFAULT_SENDER_CAP)),
        }
    }

    pub fn with_cap_of_senders(mut self, cap: usize) -> Self {
        self.senders = Mutex::new(SenderCache::new(cap));
        self
    }

    pub fn bot(&self) -> String {
        self.ids.bot().user_id().to_owned()
    }

    async fn sender(&self, room: &str, agent: AgentId) -> Option<String> {
        if let Some(kept) = self.cache_senders().get(room) {
            return Some(kept);
        }
        let Ok(ghost) = self.ids.agent(&agent.to_string()) else {
            return None;
        };
        let chosen = match self.client.members(ghost.user_id(), room).await {
            Ok(members) => choose_sender(&members, &self.bot(), ghost.user_id()),
            Err(_) => ghost.user_id().to_owned(),
        };
        self.cache_senders().keep(room, &chosen);
        Some(chosen)
    }

    fn cache_senders(&self) -> std::sync::MutexGuard<'_, SenderCache> {
        self.senders
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

impl<P: SpacesPort + Send + Sync> StateTarget for SynapseTarget<P> {
    async fn notify(
        &self,
        room: &str,
        agent: AgentId,
        txn: &str,
        text: &str,
    ) -> Result<(), CallError> {
        let Some(sender) = self.sender(room, agent).await else {
            return Err(CallError::Configuration(
                "id de agente fora do padrão do módulo Matrix".to_owned(),
            ));
        };
        let body = json!({ "msgtype": MSGTYPE_NOTICE, "body": text });
        self.client
            .send_event(&sender, room, "m.room.message", txn, &body)
            .await
            .map(|_| ())
    }

    async fn write_metadata(
        &self,
        room: &str,
        agent: AgentId,
        metadata: &RoomMetadata,
    ) -> Result<(), CallError> {
        let Ok(ghost) = self.ids.agent(&agent.to_string()) else {
            return Err(CallError::Configuration(
                "id de agente fora do padrão do módulo Matrix".to_owned(),
            ));
        };
        let body = serde_json::to_value(metadata).map_err(|error| CallError::InvalidResponse {
            operation: "state (metadata)",
            reason: format!("o metadata da sala não serializou: {error}"),
        })?;
        self.client
            .send_state(ghost.user_id(), room, METADATA_KIND, None, &body)
            .await
            .map(|_| ())
    }

    async fn place(&self, data: RoomForSpace) -> Result<Position, SpaceError> {
        self.spaces.place(&data).await
    }

    async fn rename(&self, room: &str, agent: AgentId, name: &str) -> Result<(), CallError> {
        let Ok(ghost) = self.ids.agent(&agent.to_string()) else {
            return Err(CallError::Configuration(
                "id de agente fora do padrão do módulo Matrix".to_owned(),
            ));
        };
        let body = json!({ "name": name });
        self.client
            .send_state(ghost.user_id(), room, NAME_KIND, None, &body)
            .await
            .map(|_| ())
    }

    async fn rename_ghost(&self, client: &str, name: &str) -> Result<(), CallError> {
        self.users
            .ensure_client(client, name)
            .await
            .map(|_| ())
            .map_err(ghost_call)
    }
}

fn ghost_call(error: GhostsError) -> CallError {
    match error {
        GhostsError::Call(call) => call,
        other => CallError::Configuration(other.to_string()),
    }
}

impl<P: SpacesPort + Send + Sync> std::fmt::Debug for SynapseTarget<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DestinoDoSynapse")
            .field("bot", &self.bot())
            .finish_non_exhaustive()
    }
}

struct SenderCache {
    cap: usize,
    items: BTreeMap<String, String>,
    order: VecDeque<String>,
}

impl SenderCache {
    fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            items: BTreeMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&self, room: &str) -> Option<String> {
        self.items.get(room).cloned()
    }

    fn keep(&mut self, room: &str, sender: &str) {
        if self.items.contains_key(room) {
            return;
        }
        while self.items.len() >= self.cap {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            self.items.remove(&old);
        }
        self.items.insert(room.to_owned(), sender.to_owned());
        self.order.push_back(room.to_owned());
    }
}

pub fn notice_of_status(
    status: ConversationStatus,
    department: Option<&DepartmentSpace>,
) -> String {
    match status {
        ConversationStatus::Transferred => match department {
            Some(department) => format!(
                "Atendimento humano assumiu a conversa (departamento {}): o agente automático está pausado até ela ser reaberta.",
                department.space_name()
            ),
            None => NOTICE_TRANSFERRED.to_owned(),
        },
        ConversationStatus::Open => NOTICE_REOPENED.to_owned(),
        ConversationStatus::Closed => NOTICE_CLOSED.to_owned(),
    }
}

pub fn notice_txn(conversation: &ConversationState) -> String {
    let body: String = conversation
        .conversation
        .to_string()
        .chars()
        .filter(char::is_ascii_hexdigit)
        .collect();
    format!(
        "{NOTICE_MARK}_{body}_{}_{}",
        conversation.updated_at,
        conversation.status.as_text()
    )
}

fn call_reason(error: &CallError) -> String {
    match error {
        CallError::Response {
            status, errcode, ..
        } => match errcode {
            Some(code) => format!("{status}_{code}"),
            None => format!("http_{status}"),
        },
        CallError::Network { .. } => "rede".to_owned(),
        CallError::Deadline { .. } => "prazo".to_owned(),
        CallError::Configuration(_) => "configuracao".to_owned(),
        CallError::InvalidResponse { .. } => "resposta_invalida".to_owned(),
    }
}

fn space_reason(error: &SpaceError) -> &'static str {
    match error {
        SpaceError::Call(_) => "chamada",
        SpaceError::Armazenamento(_) => "armazenamento",
        SpaceError::WithoutPermission { .. } => "sem_permissao",
        SpaceError::InvalidSpace { .. } => "espaco_invalido",
        SpaceError::Internal { .. } => "interno",
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("armazenamento do estado do Matrix falhou: {0}")]
    Armazenamento(#[from] StoreError),
}

impl StateError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Armazenamento(_) => "armazenamento_indisponivel",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CycleReport {
    pub checked: u32,
    pub notified: u32,
    pub positioned: u32,
    pub renamed: u32,
    pub without_room: u32,
    pub without_change: u32,
    pub deferred: u32,
    pub skipped: u32,
    pub pages: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveryOutcome {
    Done,
    Deferred,
}

struct CursorOrg {
    updated_at: i64,
    id: String,
    bloqueada: bool,
    did_change: bool,
}

pub struct ConversationStates<D: StateTarget, P: StatePort> {
    port: P,
    destination: D,
    ids: GhostIds,
    interval: Duration,
    page: u32,
    consecutive_failures: AtomicU32,
    stop: tokio::sync::watch::Sender<bool>,
    metrics: Arc<MetricsMatrix>,
}

impl<D: StateTarget, P: StatePort> ConversationStates<D, P> {
    pub fn new(port: P, destination: D, config: &MatrixConfig) -> Result<Self, GhostsError> {
        let ids = GhostIds::of_config(config)?;
        let (stop, _) = tokio::sync::watch::channel(false);
        Ok(Self {
            port,
            destination,
            ids,
            interval: Duration::from_millis(config.interval_ms.max(1)),
            page: PAGE,
            consecutive_failures: AtomicU32::new(0),
            stop,
            metrics: MetricsMatrix::unregistered(),
        })
    }

    pub fn with_metrics(mut self, metrics: Arc<MetricsMatrix>) -> Self {
        self.metrics = metrics;
        self
    }

    pub fn with_page(mut self, page: u32) -> Self {
        self.page = page.max(1);
        self
    }

    pub fn ask_stop(&self) {
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
                        target: "poph_server::matrix_estado",
                        checked = report.checked,
                        notified = report.notified,
                        deferred = report.deferred,
                        "passagem do estado das conversas no Matrix"
                    );
                    if report.deferred > 0 {
                        self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1
                    } else {
                        self.consecutive_failures.store(0, Ordering::Relaxed);
                        0
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        target: "poph_server::matrix_estado",
                        reason = error.code(),
                        "passagem do estado das conversas no Matrix falhou; segue na próxima"
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

    pub async fn cycle(&self) -> Result<CycleReport, StateError> {
        let mut report = CycleReport::default();

        let mut orgs: BTreeMap<OrgId, CursorOrg> = BTreeMap::new();
        for state in self.port.orgs().await? {
            if !state.enabled {
                continue;
            }
            let cursors = self.port.cursors(state.org).await?;
            orgs.insert(
                state.org,
                CursorOrg {
                    updated_at: cursors.updated_at,
                    id: cursors
                        .id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| MIN_ID.to_owned()),
                    bloqueada: false,
                    did_change: false,
                },
            );
        }
        if orgs.is_empty() {
            return Ok(report);
        }

        let mut after = orgs
            .values()
            .map(|cursor| (cursor.updated_at, cursor.id.clone()))
            .min()
            .unwrap_or((0, MIN_ID.to_owned()));

        let mut pages = 0u32;
        while pages < MAX_PAGES_PER_CYCLE && !*self.stop.borrow() {
            let beginning = pair_conversation(&after);
            let page = self.port.changed_items(beginning, self.page).await?;
            if page.is_empty() {
                break;
            }
            pages += 1;
            report.pages += 1;
            let last_page = page.len() < self.page as usize;

            for changed in &page {
                let Some(cursor) = orgs.get_mut(&changed.org) else {
                    continue;
                };
                if cursor.bloqueada {
                    continue;
                }
                let pair = (changed.updated_at, changed.conversation.to_string());
                if pair <= (cursor.updated_at, cursor.id.clone()) {
                    continue;
                }
                let Some(conversation) = self
                    .port
                    .conversation(changed.org, changed.conversation)
                    .await?
                else {
                    cursor.updated_at = pair.0;
                    cursor.id = pair.1;
                    cursor.did_change = true;
                    continue;
                };
                report.checked += 1;
                match self.mirror(&conversation, &mut report).await? {
                    DeliveryOutcome::Done => {
                        cursor.updated_at = pair.0;
                        cursor.id = pair.1;
                        cursor.did_change = true;
                    }
                    DeliveryOutcome::Deferred => cursor.bloqueada = true,
                }
            }

            let Some(last) = page.last() else {
                break;
            };
            after = (last.updated_at, last.conversation.to_string());
            if last_page {
                break;
            }
        }

        for (org, cursor) in &orgs {
            if !cursor.did_change {
                continue;
            }
            self.port
                .advance_conversations(*org, cursor.updated_at, &cursor.id)
                .await?;
        }
        Ok(report)
    }

    async fn mirror(
        &self,
        target: &ConversationState,
        report: &mut CycleReport,
    ) -> Result<DeliveryOutcome, StateError> {
        let Some(room) = self.port.room(target.org, target.conversation).await? else {
            report.without_room += 1;
            return Ok(DeliveryOutcome::Done);
        };
        if !room.ready {
            report.deferred += 1;
            return Ok(DeliveryOutcome::Deferred);
        }

        let mirrored = room.last_status.unwrap_or(ConversationStatus::Open);
        let mut fresca: Option<ConversationState> = None;

        if mirrored != target.status {
            let Some(current) = self
                .port
                .conversation(target.org, target.conversation)
                .await?
            else {
                report.without_room += 1;
                return Ok(DeliveryOutcome::Done);
            };
            if current.status != target.status {
                report.deferred += 1;
                return Ok(DeliveryOutcome::Deferred);
            }

            let department = current.department();
            let text = notice_of_status(current.status, department.as_ref());
            let txn = notice_txn(&current);

            let beginning = Instant::now();
            let result = self
                .destination
                .notify(&room.room_id, current.agent, &txn, &text)
                .await;
            self.measure_synapse(beginning.elapsed().as_secs_f64());
            if let Some(outcome) = self.handle(result, &current, "aviso", report) {
                return Ok(outcome);
            }

            let metadata = RoomMetadata::of_conversation(&current.to_room(department));
            let beginning = Instant::now();
            let result = self
                .destination
                .write_metadata(&room.room_id, current.agent, &metadata)
                .await;
            self.measure_synapse(beginning.elapsed().as_secs_f64());
            if let Some(outcome) = self.handle(result, &current, "metadata", report) {
                return Ok(outcome);
            }

            self.port
                .define_last_status(current.org, current.conversation, current.status)
                .await?;
            report.notified += 1;
            tracing::info!(
                target: "poph_server::matrix_estado",
                org = %current.org,
                conversation = %current.conversation,
                status = current.status.as_text(),
                "estado da conversa espelhado na sala"
            );
            fresca = Some(current);
        } else {
            report.without_change += 1;
        }

        let origin = fresca.as_ref().unwrap_or(target);

        let metadata_already_writing = fresca.is_some();
        if let Some(outcome) = self
            .rename_if_needed(origin, &room, metadata_already_writing, report)
            .await?
        {
            return Ok(outcome);
        }

        self.place(origin, &room, report).await;
        Ok(DeliveryOutcome::Done)
    }

    async fn rename_if_needed(
        &self,
        conversation: &ConversationState,
        room: &RoomState,
        metadata_already_writing: bool,
        report: &mut CycleReport,
    ) -> Result<Option<DeliveryOutcome>, StateError> {
        let Some(name) = client_name(conversation.client_name.as_deref()) else {
            return Ok(None);
        };
        if room.mirrored_name.as_deref() == Some(name.as_str()) {
            return Ok(None);
        }

        let Some(current) = self
            .port
            .conversation(conversation.org, conversation.conversation)
            .await?
        else {
            return Ok(Some(DeliveryOutcome::Done));
        };
        let Some(name) = client_name(current.client_name.as_deref()) else {
            return Ok(None);
        };
        if room.mirrored_name.as_deref() == Some(name.as_str()) {
            return Ok(None);
        }
        let metadata_already_writing = metadata_already_writing && current == *conversation;

        let beginning = Instant::now();
        let result = self
            .destination
            .rename(&room.room_id, current.agent, &name)
            .await;
        self.measure_synapse(beginning.elapsed().as_secs_f64());
        if let Some(outcome) = self.handle(result, &current, "nome da sala", report) {
            return Ok(Some(outcome));
        }

        if !metadata_already_writing {
            let metadata = RoomMetadata::of_conversation(&current.to_room(current.department()));
            let beginning = Instant::now();
            let result = self
                .destination
                .write_metadata(&room.room_id, current.agent, &metadata)
                .await;
            self.measure_synapse(beginning.elapsed().as_secs_f64());
            if let Some(outcome) = self.handle(result, &current, "metadata do nome", report) {
                return Ok(Some(outcome));
            }
        }

        let beginning = Instant::now();
        let result = self.destination.rename_ghost(&current.client, &name).await;
        self.measure_synapse(beginning.elapsed().as_secs_f64());
        if let Some(outcome) = self.handle(result, &current, "nome do fantasma", report) {
            return Ok(Some(outcome));
        }

        self.port
            .define_name_mirrored(current.org, current.conversation, &name)
            .await?;
        report.renamed += 1;
        tracing::info!(
            target: "poph_server::matrix_estado",
            org = %current.org,
            conversation = %current.conversation,
            "nome do cliente atualizado na sala (sala, metadata e fantasma)"
        );
        Ok(None)
    }

    fn handle(
        &self,
        result: Result<(), CallError>,
        conversation: &ConversationState,
        step: &'static str,
        report: &mut CycleReport,
    ) -> Option<DeliveryOutcome> {
        let error = result.err()?;
        let reason = call_reason(&error);
        match error.classification() {
            Classification::Transient => {
                self.metrics.transient_errors.increment(1);
                if error.retry_after_ms().is_some() {
                    self.metrics.responses_429.increment(1);
                }
                report.deferred += 1;
                tracing::warn!(
                    target: "poph_server::matrix_estado",
                    org = %conversation.org,
                    conversation = %conversation.conversation,
                    step,
                    reason,
                    "estado não espelhado agora; a passagem seguinte tenta de novo"
                );
                Some(DeliveryOutcome::Deferred)
            }
            Classification::Definitive => {
                self.metrics.definitive_errors.increment(1);
                report.skipped += 1;
                tracing::warn!(
                    target: "poph_server::matrix_estado",
                    org = %conversation.org,
                    conversation = %conversation.conversation,
                    step,
                    reason,
                    "estado recusado pelo Synapse; a conversa volta na próxima alteração"
                );
                Some(DeliveryOutcome::Done)
            }
        }
    }

    async fn place(
        &self,
        conversation: &ConversationState,
        room: &RoomState,
        report: &mut CycleReport,
    ) {
        let department = conversation.department();
        if department.is_none() && room.space.is_none() {
            return;
        }
        let Ok(agent) = self.ids.agent(&conversation.agent.to_string()) else {
            tracing::warn!(
                target: "poph_server::matrix_estado",
                org = %conversation.org,
                conversation = %conversation.conversation,
                reason = "id_de_agente_invalido",
                "sala não posicionada no espaço do departamento"
            );
            return;
        };
        let data = RoomForSpace {
            org: conversation.org,
            conversation: conversation.conversation,
            room: room.room_id.clone(),
            agent,
            department,
            existing: None,
        };
        let beginning = Instant::now();
        let result = self.destination.place(data).await;
        self.measure_synapse(beginning.elapsed().as_secs_f64());
        match result {
            Ok(Position::Positioned { moved, .. }) if moved => {
                report.positioned += 1;
                tracing::info!(
                    target: "poph_server::matrix_estado",
                    org = %conversation.org,
                    conversation = %conversation.conversation,
                    "sala moved para o espaço do departamento novo"
                );
            }
            Ok(Position::Positioned {
                failed_invitations, ..
            }) if failed_invitations > 0 => {
                tracing::warn!(
                    target: "poph_server::matrix_estado",
                    org = %conversation.org,
                    conversation = %conversation.conversation,
                    failed_invitations,
                    reason = "convite_pendente_no_espaco",
                    "sala não ficou gravada no espaço porque há convite pendente; tenta na próxima alteração"
                );
            }
            Ok(_) => {}
            Err(error) => {
                match error.classification() {
                    Classification::Transient => self.metrics.transient_errors.increment(1),
                    Classification::Definitive => self.metrics.definitive_errors.increment(1),
                }
                tracing::warn!(
                    target: "poph_server::matrix_estado",
                    org = %conversation.org,
                    conversation = %conversation.conversation,
                    reason = space_reason(&error),
                    "sala não entrou no espaço do departamento; tenta na próxima alteração"
                );
            }
        }
    }

    fn measure_synapse(&self, seconds: f64) {
        self.metrics.attempts.increment(1);
        self.metrics.latency_synapse_seconds.observe(seconds);
    }
}

fn client_name(name: Option<&str>) -> Option<String> {
    name.map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn pair_conversation(pair: &(i64, String)) -> Option<(i64, ConversationId)> {
    let id = pair.1.parse::<ConversationId>().ok()?;
    Some((pair.0, id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use poph_types::DepartmentId;

    fn department(name: &str) -> DepartmentSpace {
        DepartmentSpace {
            id: DepartmentId::generate(),
            name: name.to_owned(),
        }
    }

    fn conversation(status: ConversationStatus, updated_at: i64) -> ConversationState {
        ConversationState {
            org: OrgId::generate(),
            conversation: ConversationId::generate(),
            status,
            agent: AgentId::generate(),
            agent_name: Some("Bia".to_owned()),
            channel: "whatsapp".to_owned(),
            client: "5511999990000".to_owned(),
            client_name: None,
            department_target: None,
            department_default: None,
            updated_at,
        }
    }

    #[test]
    fn notice_per_situation_is_short_and_has_no_client_data() {
        assert_eq!(
            notice_of_status(ConversationStatus::Open, None),
            NOTICE_REOPENED
        );
        assert_eq!(
            notice_of_status(ConversationStatus::Closed, None),
            NOTICE_CLOSED
        );
        assert_eq!(
            notice_of_status(ConversationStatus::Transferred, None),
            NOTICE_TRANSFERRED
        );
        let with_department = notice_of_status(
            ConversationStatus::Transferred,
            Some(&department("Suporte técnico")),
        );
        assert!(with_department.contains("departamento Suporte técnico"));
        for text in [NOTICE_REOPENED, NOTICE_TRANSFERRED, NOTICE_CLOSED] {
            assert!(!text.contains('@'), "{text}");
            assert!(!text.chars().any(|c| c.is_ascii_digit()), "{text}");
        }
    }

    #[test]
    fn notice_txn_changes_with_instant_and_situation() {
        let mut a = conversation(ConversationStatus::Transferred, 1_000);
        let first = notice_txn(&a);
        assert_eq!(first, notice_txn(&a), "repetir dá a mesma chave");
        assert!(first.starts_with("ph_est_"));
        assert!(!first.contains('-'), "{first}");
        a.updated_at = 2_000;
        assert_ne!(first, notice_txn(&a), "outra transição, outra chave");
        a.status = ConversationStatus::Closed;
        assert_ne!(first, notice_txn(&a));
        let another = conversation(ConversationStatus::Transferred, 1_000);
        assert_ne!(first, notice_txn(&another), "outra conversa, outra chave");
    }

    #[test]
    fn client_name_ignores_empty_and_space() {
        assert_eq!(client_name(None), None);
        assert_eq!(client_name(Some("")), None);
        assert_eq!(client_name(Some("   \n ")), None);
        assert_eq!(
            client_name(Some("  Ana Souza ")),
            Some("Ana Souza".to_owned())
        );
    }

    #[test]
    fn department_picks_target_before_default() {
        let default = department("Geral");
        let target = department("Suporte");
        let mut a = conversation(ConversationStatus::Open, 1);
        a.department_default = Some(default.clone());
        assert_eq!(a.department(), Some(default));
        a.department_target = Some(target.clone());
        assert_eq!(a.department(), Some(target));
        a.department_default = None;
        a.department_target = None;
        assert_eq!(a.department(), None);
    }

    #[test]
    fn notice_sender_is_bot_when_in_room() {
        let bot = "@ph_as_bot:matrix-dev.local";
        let agent = "@ph_ag_1:matrix-dev.local";
        let mut members = BTreeMap::new();
        members.insert(
            agent.to_owned(),
            RoomMember {
                display_name: None,
                avatar_url: None,
            },
        );
        assert_eq!(choose_sender(&members, bot, agent), agent);
        members.insert(
            bot.to_owned(),
            RoomMember {
                display_name: None,
                avatar_url: None,
            },
        );
        assert_eq!(choose_sender(&members, bot, agent), bot);
    }

    #[test]
    fn sender_cache_respects_cap() {
        let mut cache = SenderCache::new(2);
        cache.keep("!a", "@bot");
        cache.keep("!b", "@bot");
        cache.keep("!c", "@bot");
        assert_eq!(cache.get("!a"), None, "o mais antigo saiu");
        assert_eq!(cache.get("!b").as_deref(), Some("@bot"));
        assert_eq!(cache.get("!c").as_deref(), Some("@bot"));
    }

    #[test]
    fn delay_doubles_with_failures_and_has_cap() {
        let config = MatrixConfig {
            enabled: true,
            homeserver_url: "http://127.0.0.1:1".to_owned(),
            server_name: "matrix-dev.local".to_owned(),
            prefix: "ph".to_owned(),
            concurrency: 4,
            interval_ms: 1_000,
            ..MatrixConfig::default()
        };
        let worker = ConversationStates::new(NullPort, NullTarget, &config).expect("monta o laço");
        assert_eq!(worker.delay_after(0), Duration::from_secs(1));
        assert_eq!(worker.delay_after(1), Duration::from_secs(2));
        assert_eq!(worker.delay_after(3), Duration::from_secs(8));
        assert_eq!(worker.delay_after(9), MAX_DELAY);
    }

    struct NullPort;

    impl StatePort for NullPort {
        async fn orgs(&self) -> Result<Vec<OrgState>, StoreError> {
            Ok(Vec::new())
        }

        async fn cursors(&self, _org: OrgId) -> Result<StoredCursor, StoreError> {
            Ok(StoredCursor {
                updated_at: 0,
                id: None,
            })
        }

        async fn changed_items(
            &self,
            _after: Option<(i64, ConversationId)>,
            _limit: u32,
        ) -> Result<Vec<ConversationChanged>, StoreError> {
            Ok(Vec::new())
        }

        async fn conversation(
            &self,
            _org: OrgId,
            _conversation: ConversationId,
        ) -> Result<Option<ConversationState>, StoreError> {
            Ok(None)
        }

        async fn room(
            &self,
            _org: OrgId,
            _conversation: ConversationId,
        ) -> Result<Option<RoomState>, StoreError> {
            Ok(None)
        }

        async fn define_last_status(
            &self,
            _org: OrgId,
            _conversation: ConversationId,
            _status: ConversationStatus,
        ) -> Result<(), StoreError> {
            Ok(())
        }

        async fn define_name_mirrored(
            &self,
            _org: OrgId,
            _conversation: ConversationId,
            _name: &str,
        ) -> Result<(), StoreError> {
            Ok(())
        }

        async fn advance_conversations(
            &self,
            _org: OrgId,
            _updated_at: i64,
            _id: &str,
        ) -> Result<(), StoreError> {
            Ok(())
        }
    }

    struct NullTarget;

    impl StateTarget for NullTarget {
        async fn notify(
            &self,
            _room: &str,
            _agent: AgentId,
            _txn: &str,
            _text: &str,
        ) -> Result<(), CallError> {
            Ok(())
        }

        async fn write_metadata(
            &self,
            _room: &str,
            _agent: AgentId,
            _metadata: &RoomMetadata,
        ) -> Result<(), CallError> {
            Ok(())
        }

        async fn place(&self, _data: RoomForSpace) -> Result<Position, SpaceError> {
            Ok(Position::WithoutDepartment)
        }

        async fn rename(&self, _room: &str, _agent: AgentId, _name: &str) -> Result<(), CallError> {
            Ok(())
        }

        async fn rename_ghost(&self, _customer: &str, _name: &str) -> Result<(), CallError> {
            Ok(())
        }
    }
}
