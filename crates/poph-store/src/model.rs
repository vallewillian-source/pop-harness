use poph_types::{
    ActionActor, AgentId, ClosedBy, ConversationId, DepartmentId, EffectId, EffectState,
    EventActor, ExecutionId, ExecutionStatus, FileId, FlowId, IntegrationId, KnowledgeGroupId,
    KnowledgeItemId, MessageId, OrgId, ProductCategoryId, ProductId, RecordEventId,
    RecordEventKind, RecordId, RecordStatus, RevisionId, StepStatus, TurnId,
};

use crate::error::{Resultado, StoreError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Active,
    Inactive,
    Archived,
}

impl Status {
    pub fn as_text(self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::Inactive => "inactive",
            Status::Archived => "archived",
        }
    }

    pub fn from_text(value: &str) -> Resultado<Self> {
        match value {
            "active" => Ok(Status::Active),
            "inactive" => Ok(Status::Inactive),
            "archived" => Ok(Status::Archived),
            other => Err(StoreError::Internal(format!(
                "status desconhecido no banco: `{other}`"
            ))),
        }
    }

    pub fn is_active(self) -> bool {
        self == Status::Active
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub after: Option<uuid::Uuid>,
    pub limit: u32,
}

impl Page {
    pub fn with_limit(limit: u32) -> Self {
        Self {
            after: None,
            limit: limit.clamp(1, 200),
        }
    }
}

impl Default for Page {
    fn default() -> Self {
        Self::with_limit(50)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Org {
    pub id: OrgId,
    pub name: String,
    pub slug: String,
    pub status: Status,
    pub config_json: String,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default)]
pub struct NewOrg {
    pub name: String,
    pub slug: String,
    pub config_json: String,
}

#[derive(Debug, Clone, Default)]
pub struct OrgChange {
    pub name: Option<String>,
    pub status: Option<Status>,
    pub config_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenAdmin {
    pub id: poph_types::AdminTokenId,
    pub org_id: Option<OrgId>,
    pub label: String,
    pub hash: String,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductCategory {
    pub id: ProductCategoryId,
    pub org_id: OrgId,
    pub name: String,
    pub description: String,
    pub status: Status,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default)]
pub struct NewCategory {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Default)]
pub struct CategoryChange {
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: Option<Status>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Product {
    pub id: ProductId,
    pub org_id: OrgId,
    pub category_id: ProductCategoryId,
    pub name: String,
    pub slug: String,
    pub aliases: Vec<String>,
    pub description: String,
    pub price_text: String,
    pub price_cents: i64,
    pub unit: String,
    pub includes: String,
    pub exclui: String,
    pub conditions: String,
    pub tags: Vec<String>,
    pub status: Status,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default)]
pub struct NewProduct {
    pub category_id: Option<ProductCategoryId>,
    pub name: String,
    pub slug: Option<String>,
    pub aliases: Vec<String>,
    pub description: String,
    pub price_text: String,
    pub price_cents: i64,
    pub unit: String,
    pub includes: String,
    pub exclui: String,
    pub conditions: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ProductChange {
    pub category_id: Option<ProductCategoryId>,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub aliases: Option<Vec<String>>,
    pub description: Option<String>,
    pub price_text: Option<String>,
    pub price_cents: Option<i64>,
    pub unit: Option<String>,
    pub includes: Option<String>,
    pub exclui: Option<String>,
    pub conditions: Option<String>,
    pub tags: Option<Vec<String>>,
    pub status: Option<Status>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductForIndex {
    pub id: ProductId,
    pub name: String,
    pub category: String,
    pub texto: String,
    pub version: u64,
}

pub fn slugificar(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_trace = false;
    for character in name
        .chars()
        .flat_map(char::to_lowercase)
        .map(without_accent)
    {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            last_trace = false;
        } else if !last_trace && !slug.is_empty() {
            slug.push('-');
            last_trace = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug.truncate(64);
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

fn without_accent(character: char) -> char {
    match character {
        'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        other => other,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Departamento {
    pub id: DepartmentId,
    pub org_id: OrgId,
    pub name: String,
    pub description: String,
    pub status: Status,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default)]
pub struct NewDepartment {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Default)]
pub struct DepartmentChange {
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: Option<Status>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Agente {
    pub id: AgentId,
    pub org_id: OrgId,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub status: Status,
    pub agent_type: String,
    pub category: String,
    pub default_department: Option<DepartmentId>,
    pub handoff_active: bool,
    pub handoff_department: Option<DepartmentId>,
    pub provedor: String,
    pub modelo: String,
    pub temperatura: Option<f64>,
    pub max_tokens_saida: Option<i64>,
    pub max_context_messages: u32,
    pub max_context_tokens: u32,
    pub grouping_window_ms: u32,
    pub max_tool_rounds: u32,
    pub max_tools_per_message: u32,
    pub rag_top_k: u32,
    pub rag_cutoff: f64,
    pub jev_json: String,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Agente {
    pub fn can_open_turn(&self) -> bool {
        self.status.is_active()
    }
}

#[derive(Debug, Clone)]
pub struct NewAgent {
    pub name: String,
    pub slug: String,
    pub description: String,
    pub agent_type: String,
    pub category: String,
    pub default_department: Option<DepartmentId>,
    pub provedor: String,
    pub modelo: String,
    pub temperatura: Option<f64>,
    pub max_tokens_saida: Option<i64>,
    pub max_context_tokens: u32,
    pub grouping_window_ms: u32,
    pub max_tool_rounds: u32,
    pub max_tools_per_message: u32,
    pub rag_top_k: u32,
    pub rag_cutoff: f64,
}

impl Default for NewAgent {
    fn default() -> Self {
        Self {
            name: String::new(),
            slug: String::new(),
            description: String::new(),
            agent_type: "custom".to_owned(),
            category: "generic".to_owned(),
            default_department: None,
            provedor: "antigravity2api".to_owned(),
            modelo: "gemini-3.8-flash-tiered".to_owned(),
            temperatura: None,
            max_tokens_saida: None,
            max_context_tokens: 64_000,
            grouping_window_ms: 2_000,
            max_tool_rounds: 4,
            max_tools_per_message: 10,
            rag_top_k: 3,
            rag_cutoff: 0.0,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AgentChange {
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: Option<Status>,
    pub default_department: Option<DepartmentId>,
    pub handoff_active: Option<bool>,
    pub handoff_department: Option<DepartmentId>,
    pub provedor: Option<String>,
    pub modelo: Option<String>,
    pub temperatura: Option<f64>,
    pub max_tokens_saida: Option<i64>,
    pub max_context_tokens: Option<u32>,
    pub grouping_window_ms: Option<u32>,
    pub max_tool_rounds: Option<u32>,
    pub max_tools_per_message: Option<u32>,
    pub rag_top_k: Option<u32>,
    pub rag_cutoff: Option<f64>,
    pub jev_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instructions {
    pub agent_id: AgentId,
    pub org_id: OrgId,
    pub structured_json: String,
    pub markdown: String,
    pub derived_from_json: bool,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationStatus {
    Open,
    Transferred,
    Closed,
}

impl ConversationStatus {
    pub fn as_text(self) -> &'static str {
        match self {
            ConversationStatus::Open => "open",
            ConversationStatus::Transferred => "transferred",
            ConversationStatus::Closed => "closed",
        }
    }

    pub fn from_text(value: &str) -> Resultado<Self> {
        match value {
            "open" => Ok(ConversationStatus::Open),
            "transferred" => Ok(ConversationStatus::Transferred),
            "closed" => Ok(ConversationStatus::Closed),
            other => Err(StoreError::Internal(format!(
                "status de conversa desconhecido no banco: `{other}`"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversa {
    pub id: ConversationId,
    pub org_id: OrgId,
    pub agent_id: AgentId,
    pub integration_id: Option<IntegrationId>,
    pub canal: String,
    pub peer_externo: Option<String>,
    pub status: ConversationStatus,
    pub target_department: Option<DepartmentId>,
    pub summary: Option<String>,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_message_at: Option<i64>,
    pub customer_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewConversation {
    pub agent_id: AgentId,
    pub integration_id: Option<IntegrationId>,
    pub canal: String,
    pub peer_externo: Option<String>,
    pub target_department: Option<DepartmentId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Inbound,
    Outbound,
}

impl Direction {
    pub fn as_text(self) -> &'static str {
        match self {
            Direction::Inbound => "in",
            Direction::Outbound => "out",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    Tool,
    System,
}

impl Role {
    pub fn as_text(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
            Role::System => "system",
        }
    }

    pub fn from_text(value: &str) -> Resultado<Self> {
        match value {
            "user" => Ok(Role::User),
            "assistant" => Ok(Role::Assistant),
            "tool" => Ok(Role::Tool),
            "system" => Ok(Role::System),
            other => Err(StoreError::Internal(format!(
                "papel de mensagem desconhecido no banco: `{other}`"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Agente,
    Human,
    Device,
}

impl Origin {
    pub fn as_text(self) -> &'static str {
        match self {
            Origin::Agente => "agent",
            Origin::Human => "human",
            Origin::Device => "device",
        }
    }

    pub fn from_text(value: &str) -> Resultado<Self> {
        match value {
            "agent" => Ok(Origin::Agente),
            "human" => Ok(Origin::Human),
            "device" => Ok(Origin::Device),
            other => Err(StoreError::Internal(format!(
                "origem de mensagem desconhecida no banco: `{other}`"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mensagem {
    pub id: MessageId,
    pub org_id: OrgId,
    pub conversation_id: ConversationId,
    pub direction: Direction,
    pub role: Role,
    pub origin: Origin,
    pub author_name: Option<String>,
    pub texto: String,
    pub attachments_json: String,
    pub id_externo: Option<String>,
    pub turn_id: Option<TurnId>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct NewHumanResponse<'a> {
    pub event_id: &'a str,
    pub raw_text: &'a str,
    pub sent_text: &'a str,
    pub author_id: &'a str,
    pub author_name: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub struct ResponseFile<'a> {
    pub name: &'a str,
    pub mime: &'a str,
    pub size_bytes: u64,
    pub sha256: &'a str,
    pub blob_path: &'a str,
    pub caption: &'a str,
    pub extraction_status: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct HumanResponseWithFile<'a> {
    pub event_id: &'a str,
    pub author_id: &'a str,
    pub author_name: &'a str,
    pub file: ResponseFile<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanRecording {
    pub mensagem: Mensagem,
    pub saida: OutboxItem,
    pub just_transferred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HumanResponse {
    Stored(Box<HumanRecording>),
    Duplicate,
    ConversationClosed,
    ConversationMissing,
}

#[derive(Debug, Clone)]
pub struct NewMessage {
    pub conversation_id: ConversationId,
    pub direction: Direction,
    pub role: Role,
    pub origin: Origin,
    pub author_name: Option<String>,
    pub texto: String,
    pub attachments_json: String,
    pub id_externo: Option<String>,
    pub turn_id: Option<TurnId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ficha {
    pub id: RecordId,
    pub org_id: OrgId,
    pub conversation_id: Option<ConversationId>,
    pub agent_id: AgentId,
    pub canal: String,
    pub peer_externo: Option<String>,
    pub status: RecordStatus,
    pub aberta_em: i64,
    pub fechada_em: Option<i64>,
    pub fechada_por: Option<ClosedBy>,
    pub fechada_motivo: Option<String>,
    pub version: u64,
}

#[derive(Debug, Clone)]
pub struct NewRecord {
    pub conversation_id: Option<ConversationId>,
    pub agent_id: AgentId,
    pub canal: String,
    pub peer_externo: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordEvent {
    pub id: RecordEventId,
    pub org_id: OrgId,
    pub ficha_id: RecordId,
    pub execucao_id: Option<ExecutionId>,
    pub seq: i64,
    pub tipo: RecordEventKind,
    pub ator_tipo: EventActor,
    pub ator_ref: Option<String>,
    pub payload_json: String,
    pub chave_idempotencia: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewRecordEvent<'a> {
    pub execucao_id: Option<ExecutionId>,
    pub tipo: RecordEventKind,
    pub ator_tipo: EventActor,
    pub ator_ref: Option<&'a str>,
    pub payload_json: String,
    pub chave_idempotencia: Option<&'a str>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordFilter {
    pub status: Option<RecordStatus>,
    pub agente: Option<AgentId>,
    pub canal: Option<String>,
    pub desde: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fluxo {
    pub id: FlowId,
    pub org_id: OrgId,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub status: Status,
    pub rascunho_fonte: String,
    pub revisao_atual: Option<u64>,
    pub version: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewFlow {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub rascunho_fonte: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowChange {
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: Option<Status>,
    pub rascunho_fonte: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowFilter {
    pub status: Option<Status>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowRevision {
    pub id: RevisionId,
    pub org_id: OrgId,
    pub fluxo_id: FlowId,
    pub numero: u64,
    pub fonte: String,
    pub fonte_sha256: String,
    pub definicao_json: String,
    pub schema_versao: u32,
    pub publicada_em: i64,
    pub publicada_por: String,
    pub resolucoes_json: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RevisionPublication {
    pub expected: u64,
    pub fonte: String,
    pub definicao_json: String,
    pub schema_versao: Option<u32>,
    pub resolucoes_json: String,
    pub ator_tipo: String,
    pub actor_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Execution {
    pub id: ExecutionId,
    pub org_id: OrgId,
    pub ficha_id: RecordId,
    pub fluxo_id: FlowId,
    pub revisao_id: RevisionId,
    pub status: ExecutionStatus,
    pub no_atual: String,
    pub dados_json: String,
    pub result: Option<String>,
    pub motivo_fim: Option<String>,
    pub iniciada_em: i64,
    pub finalizada_em: Option<i64>,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionStep {
    pub id: String,
    pub org_id: OrgId,
    pub execucao_id: ExecutionId,
    pub no_id: String,
    pub ordem: i64,
    pub status: StepStatus,
    pub dados_json: String,
    pub acting_actors: Vec<ActionActor>,
    pub entrou_em: i64,
    pub concluiu_em: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowEffect {
    pub id: EffectId,
    pub org_id: OrgId,
    pub ficha_id: RecordId,
    pub execucao_id: ExecutionId,
    pub key: String,
    pub tipo: String,
    pub payload_json: String,
    pub estado: EffectState,
    pub tentativas: u32,
    pub ultimo_erro: Option<String>,
    pub proxima_tentativa: Option<i64>,
    pub enviado_em: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionState {
    pub status: ExecutionStatus,
    pub no_atual: String,
    pub dados_json: String,
    pub acting_actors: Vec<ActionActor>,
    pub result: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionEvent {
    pub tipo: RecordEventKind,
    pub ator_tipo: EventActor,
    pub payload_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionEffect {
    pub key: String,
    pub tipo: String,
    pub payload_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowTransition {
    pub new_state: ExecutionState,
    pub events: Vec<TransitionEvent>,
    pub effects: Vec<TransitionEffect>,
    pub closes_service: bool,
    pub repeated: bool,
    pub rule_error: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JevDecision<'a> {
    pub choice: &'a str,
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct ExecutionStart<'a> {
    pub ficha: RecordId,
    pub fluxo: FlowId,
    pub revision: RevisionId,
    pub actor: EventActor,
    pub ator_ref: Option<&'a str>,
    pub jev_decision: Option<JevDecision<'a>>,
    pub initial_state: ExecutionState,
    pub inbound_effects: Vec<TransitionEffect>,
    pub chave_idempotencia: Option<&'a str>,
}

#[derive(Debug, Clone, Default)]
pub struct TransitionExtras<'a> {
    pub ator_ref: Option<&'a str>,
    pub fechada_por: Option<ClosedBy>,
    pub close_reason: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredExecution {
    pub execution: Execution,
    pub events: Vec<RecordEvent>,
    pub effects: Vec<FlowEffect>,
    pub closed_service: bool,
    pub repeated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxItem {
    pub id: poph_types::InboxId,
    pub org_id: OrgId,
    pub integration_id: IntegrationId,
    pub id_externo: String,
    pub event_json: String,
    pub received_at: i64,
    pub processed_at: Option<i64>,
    pub tentativas: u32,
    pub ultimo_erro: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewInboxItem {
    pub integration_id: IntegrationId,
    pub id_externo: String,
    pub event_json: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Received {
    New,
    Duplicate,
}

#[derive(Debug, Clone)]
pub struct NewInboxItemWithMessage {
    pub integration_id: IntegrationId,
    pub id_externo: String,
    pub event_json: String,
    pub conversation_id: ConversationId,
    pub message_id: MessageId,
    pub texto: String,
    pub attachments_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedWithMessage {
    pub situacao: Received,
    pub item: InboxItem,
    pub mensagem: Mensagem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxState {
    Pending,
    Sending,
    Sent,
    Failed,
}

impl OutboxState {
    pub fn as_text(self) -> &'static str {
        match self {
            OutboxState::Pending => "pending",
            OutboxState::Sending => "sending",
            OutboxState::Sent => "sent",
            OutboxState::Failed => "failed",
        }
    }

    pub fn from_text(value: &str) -> Resultado<Self> {
        match value {
            "pending" => Ok(OutboxState::Pending),
            "sending" => Ok(OutboxState::Sending),
            "sent" => Ok(OutboxState::Sent),
            "failed" => Ok(OutboxState::Failed),
            other => Err(StoreError::Internal(format!(
                "estado de saída desconhecido no banco: `{other}`"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxItem {
    pub id: poph_types::OutboxId,
    pub org_id: OrgId,
    pub conversation_id: ConversationId,
    pub turn_id: TurnId,
    pub seq: u32,
    pub tipo: String,
    pub content_json: String,
    pub estado: OutboxState,
    pub tentativas: u32,
    pub ultimo_erro: Option<String>,
    pub created_at: i64,
    pub enviado_em: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct NewOutboxItem {
    pub conversation_id: ConversationId,
    pub turn_id: TurnId,
    pub seq: u32,
    pub tipo: String,
    pub content_json: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnState {
    Running,
    Completed,
    Failed,
    Skipped,
}

impl TurnState {
    pub fn as_text(self) -> &'static str {
        match self {
            TurnState::Running => "running",
            TurnState::Completed => "done",
            TurnState::Failed => "failed",
            TurnState::Skipped => "skipped",
        }
    }

    pub fn from_text(value: &str) -> Resultado<Self> {
        match value {
            "running" => Ok(TurnState::Running),
            "done" => Ok(TurnState::Completed),
            "failed" => Ok(TurnState::Failed),
            "skipped" => Ok(TurnState::Skipped),
            other => Err(StoreError::Internal(format!(
                "estado de turno desconhecido no banco: `{other}`"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turno {
    pub id: TurnId,
    pub org_id: OrgId,
    pub conversation_id: ConversationId,
    pub agent_id: Option<AgentId>,
    pub estado: TurnState,
    pub jev_json: String,
    pub passages_json: String,
    pub tools_json: String,
    pub times_json: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_micros: i64,
    pub error: Option<String>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub id: poph_types::AuditId,
    pub org_id: Option<OrgId>,
    pub ator_tipo: String,
    pub actor_id: Option<String>,
    pub acao: String,
    pub entity: String,
    pub entity_id: Option<String>,
    pub detail_json: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Default)]
pub struct TurnFinalization {
    pub estado: Option<TurnState>,
    pub jev_json: Option<String>,
    pub passages_json: Option<String>,
    pub tools_json: Option<String>,
    pub times_json: Option<String>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub cost_micros: Option<i64>,
    pub error: Option<String>,
}

impl TurnFinalization {
    pub fn completed(times_json: &str) -> Self {
        Self {
            estado: Some(TurnState::Completed),
            times_json: Some(times_json.to_owned()),
            ..Self::default()
        }
    }

    pub fn failed(error: &str) -> Self {
        Self {
            estado: Some(TurnState::Failed),
            error: Some(error.to_owned()),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NewAuditEntry {
    pub ator_tipo: String,
    pub actor_id: Option<String>,
    pub acao: String,
    pub entity: String,
    pub entity_id: Option<String>,
    pub detail_json: String,
}

pub const MAX_ITEM_CONTENT: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeGroup {
    pub id: KnowledgeGroupId,
    pub org_id: OrgId,
    pub name: String,
    pub description: String,
    pub status: Status,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewKnowledgeGroup {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KnowledgeGroupChange {
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: Option<Status>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeItem {
    pub id: KnowledgeItemId,
    pub org_id: OrgId,
    pub group_id: KnowledgeGroupId,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub position: i64,
    pub status: Status,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewKnowledgeItem {
    pub group_id: KnowledgeGroupId,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub position: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KnowledgeItemChange {
    pub group_id: Option<KnowledgeGroupId>,
    pub title: Option<String>,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    pub position: Option<i64>,
    pub status: Option<Status>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexItem {
    pub id: KnowledgeItemId,
    pub group_id: KnowledgeGroupId,
    pub group: String,
    pub title: String,
    pub content: String,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryFile {
    pub id: FileId,
    pub org_id: OrgId,
    pub agent_id: AgentId,
    pub name: String,
    pub description: String,
    pub mime: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub blob_path: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLibraryFile {
    pub agent_id: AgentId,
    pub name: String,
    pub description: String,
    pub mime: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub blob_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewChannelAttachment {
    pub name: String,
    pub mime: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub blob_path: String,
    pub extracted_text: Option<String>,
    pub extraction_status: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewChannelInboxItem {
    pub integration_id: poph_types::IntegrationId,
    pub agent_id: poph_types::AgentId,
    pub canal: String,
    pub id_externo: String,
    pub peer: String,
    pub event_json: String,
    pub texto: String,
    pub anexos: Vec<NewChannelAttachment>,
    pub customer_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDeviceMessage {
    pub integration_id: poph_types::IntegrationId,
    pub agent_id: poph_types::AgentId,
    pub canal: String,
    pub peer: String,
    pub id_externo: String,
    pub texto: String,
}

#[derive(Debug, Clone)]
pub struct ReceivedFromDevice {
    pub situacao: Received,
    pub conversa: ConversationId,
    pub mensagem: Mensagem,
    pub just_transferred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentRecord {
    pub id: poph_types::AttachmentId,
    pub message_id: poph_types::MessageId,
    pub name: String,
    pub mime: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub blob_path: String,
    pub extraction_status: String,
    pub extracted_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelIntegration {
    pub id: poph_types::IntegrationId,
    pub agent_id: poph_types::AgentId,
    pub kind: String,
    pub status: String,
}
