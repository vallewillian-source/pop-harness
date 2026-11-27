mod console;
mod department;
mod flow;
mod flow_tools;
mod http_tools;
mod pause;
mod prompt;
mod site_chat;
mod tools;

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use poph_core::llm::{
    Cancellation, FinishReason, LlmError, LlmEvent, LlmProvider, Message, Part, Request, ToolCall,
    Usage,
};
use poph_jev::{
    G1_TRANSFER, G3_NOTHING, G6_APPROVED, G7_DEFAULT_REPLY, G7Action, G8_DEFAULT_FAREWELL,
    G8_ENDED, GateEvaluator, GateOverrides, GuidanceResult, HandoffGuidance, IdGate, JevEngine,
    JevMessage, JevResult, JevState,
};
use poph_store::{
    Direction, Mensagem as StoredMessage, NewAuditEntry, NewMessage, NewOutboxItem, Page, Role,
    Store, TurnFinalization, TurnState,
};
use poph_types::{
    AgentId, ConversationId, DomainError, JevGates, LlmConfig, MessageId, OrgId, TurnId,
};
use serde_json::json;

use crate::actors::{BoxFuture, Processor, Task};
use crate::base_knowledge::{KnowledgeSearcher, RecoveredPassage, SearchParameters};
use crate::config_hot::{ConfigHot, HotAgent};
use crate::entry_output::{Fluxo, InputEvent};
use crate::error::{AgentError, Resultado};
use crate::limites::Limiter;
use crate::metrics::{MetricsG9, MetricsTurn};
use crate::products::{FoundProduct, ProductParameters, ProductSearcher};
use crate::providers::{Providers, ResolvedProvider};
use crate::summary::{self, Resumidor, SummaryOptions};

pub use console::{TransportConsole, text_item};
pub use flow::{ABRE_FLOWS, AdjustmentsFlow, CLOSES_FLOWS, NAME_TOOL_START};
pub use flow_tools::{
    NAME_ABANDON, NAME_RUN_ACTION, is_flow_tool, register as register_flow_tools,
    register_closure as register_closing_tool, register_with as register_flow_tools_with,
};
pub use http_tools::HttpToolRegistry;
use http_tools::{HttpToolCalls, HttpToolContext};
pub use pause::{FuturePause, Pause, PauseReal};
pub use prompt::{
    ABRE_BASE, ABRE_CONTEXT, CLOSES_BASE, CLOSES_CONTEXT, NOTICE_BASE, Prompt, block_base,
    block_volatile, block_volatile_with_base, data_utc,
};
pub use site_chat::{SiteRun, SiteTurnError};
pub use tools::{
    CAP_SUMMARY_TOOL, CallRecorded, DefaultRegistry, Desfecho, InternalTool, MARK_OF_CUTOFF,
    NoTools, ProductUsed, REASON_ARGUMENT_INVALID, REASON_ERROR_OF_EXECUTION,
    REASON_LIMIT_OF_CALLS, REASON_NOT_ENABLED, REASON_OUTSIDE_DO_TURN, REASON_UNKNOWN, RecordCalls,
    ToolContext, ToolFuture, ToolRegistry, UseTools, cut, offered, record_turn,
    summary_of_arguments,
};

const MESSAGE_READ_LIMIT: u32 = 200;

const COMPLETION_BATCH_SIZE: u32 = 500;

pub const MAX_IMAGES_PER_TURN: usize = 4;

pub const MAX_BYTES_PER_IMAGE: u64 = 5 * 1024 * 1024;

pub const MAX_IMAGE_BYTES_PER_TURN: u64 = 8 * 1024 * 1024;

#[derive(Debug, Default)]
struct ImageBudget {
    count: usize,
    bytes: u64,
}

impl ImageBudget {
    fn fits(&self, size: u64) -> bool {
        size > 0
            && size <= MAX_BYTES_PER_IMAGE
            && self.count < MAX_IMAGES_PER_TURN
            && self.bytes.saturating_add(size) <= MAX_IMAGE_BYTES_PER_TURN
    }

    fn consume(&mut self, size: u64) {
        self.count += 1;
        self.bytes = self.bytes.saturating_add(size);
    }
}

pub const CONFIRMATION_WINDOW: Duration = Duration::from_millis(250);

pub fn window_with_g2(
    base: Duration,
    cap: Duration,
    confirmation: Duration,
    decisao: Option<&str>,
) -> Duration {
    match decisao {
        Some(poph_jev::G2_WAIT) => base.saturating_mul(2).min(cap),
        Some(poph_jev::G2_PROCESS) => base.min(confirmation),
        _ => base,
    }
}

#[derive(Debug, Clone)]
pub struct TurnOptions {
    pub default_window: Duration,
    pub max_window: Duration,
    pub confirmation_window: Duration,
    pub slot_wait: Duration,
    pub turn_timeout: Duration,
    pub message_read_limit: u32,
}

impl Default for TurnOptions {
    fn default() -> Self {
        Self {
            default_window: Duration::from_secs(2),
            max_window: Duration::from_secs(8),
            confirmation_window: CONFIRMATION_WINDOW,
            slot_wait: Duration::from_secs(5),
            turn_timeout: Duration::from_secs(180),
            message_read_limit: MESSAGE_READ_LIMIT,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TurnInput {
    pub org: OrgId,
    pub conversa: ConversationId,
    pub mensagem: MessageId,
    pub cancellation: Cancellation,
}

impl TurnInput {
    pub fn new(org: OrgId, conversa: ConversationId, mensagem: MessageId) -> Self {
        Self {
            org,
            conversa,
            mensagem,
            cancellation: Cancellation::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnEvent {
    Started {
        turno: TurnId,
        agente: AgentId,
        conversa: ConversationId,
    },
    Texto(String),
    Base {
        sources: Vec<String>,
    },
    Tool {
        nome: String,
        round: u32,
    },
    Handoff {
        departamento: Option<poph_types::DepartmentId>,
        risk_kind: Option<String>,
        guidance: bool,
    },
    Fim {
        estado: String,
        motivo: Option<String>,
    },
}

pub trait TurnObserver: Send {
    fn evento(&mut self, evento: TurnEvent);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoopObserver;

impl TurnObserver for NoopObserver {
    fn evento(&mut self, _event: TurnEvent) {}
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnTimings {
    pub queue_ms: u64,
    pub assembly_ms: u64,
    pub first_text_ms: Option<u64>,
    pub total_ms: u64,
    pub tool_rounds: u32,
}

#[derive(Debug, Clone)]
pub struct TurnSummary {
    pub turno: TurnId,
    pub estado: TurnState,
    pub texto: String,
    pub messages: Vec<MessageId>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub timings: TurnTimings,
    pub modelo: String,
    pub motivo_fim: Option<String>,
    pub error: Option<String>,
    pub model_failure: bool,
    pub jev: Option<JevResult>,
}

impl TurnSummary {
    pub fn replied(&self) -> bool {
        self.estado == TurnState::Completed && !self.texto.trim().is_empty()
    }
}

struct PreparedTurn {
    trigger_message_id: MessageId,
    messages: Vec<Message>,
    usage: Arc<UseTools>,
    calls: Arc<RecordCalls>,
    skip_knowledge_and_products: bool,
    skip_product_search: bool,
    active_flow: bool,
    flow_fallback: bool,
}

impl PreparedTurn {
    fn retain_flow_tools(&self, offered: &mut Vec<poph_core::llm::Tool>) {
        offered.retain(|tool| match tool.nome.as_str() {
            flow_tools::NAME_START => self.flow_fallback,
            flow_tools::NAME_RUN_ACTION | flow_tools::NAME_ABANDON => self.active_flow,
            _ => true,
        });
    }
}

struct G5Context<'a> {
    org: OrgId,
    conversa: &'a poph_store::Conversa,
    hot_agent: &'a HotAgent,
    recent_messages: &'a [StoredMessage],
    group: &'a [StoredMessage],
    g3_nothing: bool,
}

struct G6Context<'a> {
    org: OrgId,
    hot_agent: &'a HotAgent,
    recent_messages: &'a [StoredMessage],
    conversa: &'a poph_store::Conversa,
    messages: &'a [Message],
    products: Vec<ProductUsed>,
    modelo: String,
    cancellation: Cancellation,
}

#[derive(Debug, Clone, Default)]
struct G6Review {
    choice: Option<String>,
    confidence: f64,
    second_choice: Option<String>,
    did_regenerate: bool,
    safe_reply: bool,
}

impl G6Review {
    fn took_place(&self) -> bool {
        self.choice.is_some()
    }

    fn to_json(&self) -> serde_json::Value {
        json!({
            "gate": IdGate::G6.as_text(),
            "escolha": self.choice,
            "confianca": self.confidence,
            "regenerou": self.did_regenerate,
            "segunda_escolha": self.second_choice,
            "resposta_segura": self.safe_reply,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TurnTarget {
    org: OrgId,
    conversa: ConversationId,
    turno: TurnId,
}

#[derive(Clone)]
pub struct TurnEngine {
    store: Arc<Store>,
    config: Arc<ConfigHot>,
    limiter: Arc<Limiter>,
    provedor: Arc<dyn LlmProvider>,
    providers: Option<Arc<Providers>>,
    tools: Arc<dyn ToolRegistry>,
    http_tools: Arc<HttpToolRegistry>,
    search: Option<Arc<dyn KnowledgeSearcher>>,
    products: Option<Arc<dyn ProductSearcher>>,
    options: TurnOptions,
    metrics: MetricsTurn,
    pause: Arc<dyn Pause>,
    default_model: String,
    cancellation: Option<Cancellation>,
    summarizer: Arc<Resumidor>,
    jev: Option<Arc<dyn GateEvaluator>>,
    handoff: HandoffGuidance,
    files: Option<poph_files::FileConfig>,
    fluxos: Option<AdjustmentsFlow>,
    g9_metrics: MetricsG9,
}

impl TurnEngine {
    pub fn with_http_tools(mut self, registry: HttpToolRegistry) -> Self {
        self.http_tools = Arc::new(registry);
        self
    }

    pub fn new(
        store: Arc<Store>,
        config: ConfigHot,
        limiter: Arc<Limiter>,
        provedor: Arc<dyn LlmProvider>,
        options: TurnOptions,
        metrics: MetricsTurn,
    ) -> Self {
        let summarizer = Arc::new(Resumidor::new(
            Arc::clone(&store),
            Arc::clone(&provedor),
            Arc::clone(&limiter),
            SummaryOptions::default(),
        ));
        Self {
            store,
            config: Arc::new(config),
            limiter,
            provedor,
            providers: None,
            tools: Arc::new(NoTools),
            http_tools: Arc::default(),
            search: None,
            products: None,
            options,
            metrics,
            pause: Arc::new(PauseReal),
            default_model: LlmConfig::default().model,
            cancellation: None,
            summarizer,
            jev: None,
            handoff: HandoffGuidance::default(),
            files: None,
            fluxos: None,
            g9_metrics: MetricsG9::own(),
        }
    }

    pub fn with_summary(mut self, options: SummaryOptions) -> Self {
        self.summarizer = Arc::new(Resumidor::new(
            Arc::clone(&self.store),
            Arc::clone(&self.provedor),
            Arc::clone(&self.limiter),
            options,
        ));
        self
    }

    pub async fn wait_for_summaries(&self, limit: Duration) -> bool {
        self.summarizer.wait_ocioso(limit).await
    }

    pub fn with_cancellation(mut self, cancellation: Cancellation) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    pub fn with_default_model(mut self, modelo: impl Into<String>) -> Self {
        let modelo = modelo.into();
        if !modelo.trim().is_empty() {
            self.default_model = modelo;
        }
        self
    }

    pub fn with_providers(mut self, providers: Arc<Providers>) -> Self {
        self.providers = Some(providers);
        self
    }

    pub fn with_jev(mut self, evaluator: Arc<dyn GateEvaluator>) -> Self {
        self.jev = Some(evaluator);
        self
    }

    pub fn with_handoff(mut self, guidance: HandoffGuidance) -> Self {
        self.handoff = guidance;
        self
    }

    pub fn with_tools(mut self, registro: Arc<dyn ToolRegistry>) -> Self {
        self.tools = registro;
        self
    }

    pub fn with_search(mut self, search: Arc<dyn KnowledgeSearcher>) -> Self {
        self.search = Some(search);
        self
    }

    pub fn with_products(mut self, products: Arc<dyn ProductSearcher>) -> Self {
        self.products = Some(products);
        self
    }

    pub fn with_files(mut self, files: poph_files::FileConfig) -> Self {
        self.files = Some(files);
        self
    }

    pub fn with_flows(mut self, config: &poph_types::FlowsConfig) -> Self {
        self.fluxos = Some(AdjustmentsFlow::of_config(config));
        self
    }

    pub fn with_flow_cache(mut self, cache: Arc<poph_fluxos::cache::RevisionCache>) -> Self {
        let Some(settings) = self.fluxos.take() else {
            return self;
        };
        self.fluxos = Some(settings.with_cache(cache));
        self
    }

    pub fn with_g9_metrics(mut self, metrics: MetricsG9) -> Self {
        self.g9_metrics = metrics;
        self
    }

    pub fn with_pause(mut self, pause: Arc<dyn Pause>) -> Self {
        self.pause = pause;
        self
    }

    pub fn hot_config(&self) -> &Arc<ConfigHot> {
        &self.config
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    pub fn metrics(&self) -> &MetricsTurn {
        &self.metrics
    }

    pub fn options(&self) -> &TurnOptions {
        &self.options
    }

    pub async fn run(
        &self,
        input: TurnInput,
        observer: &mut dyn TurnObserver,
    ) -> Resultado<TurnSummary> {
        let started_at = Instant::now();
        let org = input.org;
        let conversation_id = input.conversa;
        let cancellation = input.cancellation.clone();

        let conversa = self.store.conversations().get(org, conversation_id)?;
        let agent_id = conversa.agent_id;
        if conversa.canal == "site" {
            return Err(AgentError::Internal(
                "conversa do site exige execução própria".to_owned(),
            ));
        }
        if self
            .cancellation
            .as_ref()
            .is_some_and(Cancellation::cancelled)
        {
            cancellation.cancel();
        }

        let hot_agent = match self.config.load_verify(org, agent_id) {
            Ok(hot_agent) => hot_agent,
            Err(AgentError::Domain(DomainError::AgentInactive { status, .. })) => {
                let motivo =
                    format!("agente inativo ({status}): mensagem registrada e não respondida");
                return self.finish_skipped(
                    org,
                    conversation_id,
                    Some(agent_id),
                    motivo,
                    started_at,
                );
            }
            Err(error) => return Err(error),
        };

        if conversa.status != poph_store::ConversationStatus::Open {
            let motivo = format!(
                "conversa {} ({}): o agente não responde até a conversa ser reaberta",
                conversation_id,
                conversa.status.as_text()
            );
            return self.finish_skipped(org, conversation_id, Some(agent_id), motivo, started_at);
        }

        if let Some(integracao) = conversa.integration_id {
            let situacao = self.store.channel_integration(org, integracao)?;
            let active = situacao
                .as_ref()
                .is_some_and(|canal| canal.status == "active");
            if !active {
                let motivo = match situacao {
                    Some(canal) => format!(
                        "integração do canal ({}): mensagem registrada e não respondida",
                        canal.status
                    ),
                    None => {
                        "integração do canal não encontrada: mensagem registrada e não respondida"
                            .to_owned()
                    }
                };
                return self.finish_skipped(
                    org,
                    conversation_id,
                    Some(agent_id),
                    motivo,
                    started_at,
                );
            }
        }

        let gatilho = match self
            .store
            .messages()
            .get(org, conversation_id, input.mensagem)
        {
            Ok(mensagem) => mensagem,
            Err(error) if error.is_not_found() => {
                let motivo = format!(
                    "mensagem {} não existe na conversa {}",
                    input.mensagem, conversation_id
                );
                return self.finish_skipped(
                    org,
                    conversation_id,
                    Some(agent_id),
                    motivo,
                    started_at,
                );
            }
            Err(error) => return Err(error.into()),
        };

        let window = self.agent_window(&hot_agent.agente);
        let elapsed = || {
            Duration::from_millis(
                u64::try_from(self.store.now().saturating_sub(gatilho.created_at)).unwrap_or(0),
            )
        };

        let flow_preparation = self.fluxos.as_ref().and_then(|settings| {
            flow::prepare(settings, &self.store, org, conversation_id, agent_id)
        });
        let g9_flows: Vec<poph_jev::JevFlow> = flow_preparation
            .as_ref()
            .filter(|prepared| prepared.active.is_none())
            .map(|prepared| prepared.released.clone())
            .unwrap_or_default();
        let g9_flows = (!g9_flows.is_empty()).then_some(g9_flows);
        let g9_flows = g9_flows.as_deref();

        let mut arrival_jev: Option<JevResult> = None;
        let mut batch_started_at: Option<MessageId> = None;
        let mut wait = window.saturating_sub(elapsed());
        if self.g2_enabled(&hot_agent.agente) {
            let on_arrival = self.read_recent_messages(org, conversation_id)?;
            batch_started_at = last_customer_input(&on_arrival);
            if let Some(future) = self.start_jev(org, &hot_agent, &on_arrival, &conversa, g9_flows)
            {
                arrival_jev = Some(future.await);
            }
            let decisao = arrival_jev
                .as_ref()
                .and_then(|resultado| resultado.reliable_choice(IdGate::G2));
            let adjusted_window = window_with_g2(
                window,
                self.options.max_window,
                self.options.confirmation_window,
                decisao,
            );
            match decisao {
                Some(poph_jev::G2_WAIT) if adjusted_window > window => {
                    self.metrics.window_esticadas.increment(1);
                }
                Some(poph_jev::G2_PROCESS) if adjusted_window < window => {
                    self.metrics.window_direct.increment(1);
                }
                _ => {}
            }
            wait = adjusted_window.saturating_sub(elapsed());
        }
        self.pause.wait(wait).await;
        self.metrics
            .wait_grouping_ms
            .observe(wait.as_secs_f64() * 1000.0);

        let recent_messages = self.read_recent_messages(org, conversation_id)?;
        let (situacao, group) = analyze_group(&recent_messages, input.mensagem);
        let can_end = group.len() == 1;
        let queue_ms = queue_delay(self.store.now(), gatilho.created_at);
        match situacao {
            Situacao::NoNewMessages => {
                let motivo = "nenhuma mensagem nova para responder".to_owned();
                return self.finish_skipped(
                    org,
                    conversation_id,
                    Some(agent_id),
                    motivo,
                    started_at,
                );
            }
            Situacao::AlreadyAnswered => {
                let motivo = "mensagem já respondida por um turno anterior (reentrega ou retomada)"
                    .to_owned();
                return self.finish_skipped(
                    org,
                    conversation_id,
                    Some(agent_id),
                    motivo,
                    started_at,
                );
            }
            Situacao::Process => {}
        }
        let group_ids: Vec<MessageId> = group.iter().map(|mensagem| mensagem.id).collect();
        let ready_jev =
            arrival_jev.filter(|_| last_customer_input(&recent_messages) == batch_started_at);

        let turno = self
            .store
            .turns()
            .start(org, conversation_id, Some(agent_id))?;
        self.metrics.turns_started.increment(1);
        observer.evento(TurnEvent::Started {
            turno: turno.id,
            agente: agent_id,
            conversa: conversation_id,
        });

        let slot = match self
            .limiter
            .acquire_turn(org, agent_id, self.options.slot_wait)
            .await
        {
            Ok(slot) => slot,
            Err(error) => {
                return self.fail(
                    org,
                    turno.id,
                    &hot_agent.agente.modelo,
                    group_ids,
                    started_at,
                    queue_ms,
                    0,
                    None,
                    format!("sem vaga para o turno: {error}"),
                    FailureClass::NoSlot,
                    None,
                    observer,
                );
            }
        };

        let assembly_started_at = Instant::now();
        let search_future = self.start_search(org, agent_id, &group, &hot_agent);
        let jev_future = if ready_jev.is_some() {
            None
        } else {
            self.start_jev(org, &hot_agent, &recent_messages, &conversa, g9_flows)
        };
        let limit = hot_agent
            .agente
            .max_context_messages
            .clamp(1, self.options.message_read_limit.max(1));
        let conversation_summary = summary::Summary::decode(conversa.summary.as_deref());
        let history_assembly = summary::build(
            &recent_messages,
            &conversation_summary,
            u64::from(hot_agent.agente.max_context_tokens),
            limit,
        );
        let mut history: Vec<Message> = Vec::new();
        let mut images = ImageBudget::default();
        for mensagem in &history_assembly.messages {
            let parts = if mensagem.role == Role::User {
                self.attachment_parts(org, mensagem, &mut images)
            } else {
                Vec::new()
            };
            let empty_text = mensagem.role == Role::User && mensagem.texto.trim().is_empty();
            if let Some(item) = prompt::of_message_stored(mensagem)
                && !(empty_text && !parts.is_empty())
            {
                history.push(item);
            }
            if !parts.is_empty() {
                history.push(Message::User(parts));
            }
        }
        let (search_result, post_window_jev) = match (search_future, jev_future) {
            (Some(search), Some(jev)) => match futures::future::select(search, jev).await {
                futures::future::Either::Left((resultado, pending_jev)) => {
                    (Some(resultado), Some(pending_jev.await))
                }
                futures::future::Either::Right((jev, pending_search)) => {
                    let skips_search = jev.reliable_choice(IdGate::G3) == Some(G3_NOTHING)
                        || (can_end && jev.reliable_choice(IdGate::G8) == Some(G8_ENDED));
                    if skips_search {
                        (None, Some(jev))
                    } else {
                        (Some(pending_search.await), Some(jev))
                    }
                }
            },
            (Some(search), None) => (Some(search.await), None),
            (None, Some(jev)) => (None, Some(jev.await)),
            (None, None) => (None, None),
        };
        let mut jev = ready_jev.or(post_window_jev);
        let g3_nothing = jev
            .as_ref()
            .and_then(|resultado| resultado.reliable_choice(IdGate::G3))
            == Some(G3_NOTHING);
        let mut chunks = match search_result {
            Some(Ok(chunks)) => chunks,
            Some(Err(error)) => {
                tracing::warn!(error = %error, "busca na base falhou; turno segue sem trechos");
                self.metrics.search_failures.increment(1);
                Vec::new()
            }
            None => Vec::new(),
        };
        if g3_nothing {
            if !chunks.is_empty() {
                tracing::debug!(
                    discarded = chunks.len(),
                    "G3 classificou a mensagem como nada; trechos descartados"
                );
            }
            chunks.clear();
            self.metrics.searches_puladas_g3.increment(1);
        }
        if let Some(jev) = &jev
            && (jev.failed() || !jev.decisions.is_empty())
        {
            tracing::debug!(
                failure = ?jev.call.failure,
                decisions = ?jev.summary(),
                "Jev do turno"
            );
        }

        if jev
            .as_ref()
            .and_then(|resultado| resultado.reliable_choice(IdGate::G1))
            == Some(G1_TRANSFER)
        {
            let jev_result = jev.expect("a transferência só existe com o resultado do Jev");
            return self
                .transfer_to_human(
                    TurnTarget {
                        org,
                        conversa: conversation_id,
                        turno: turno.id,
                    },
                    &hot_agent,
                    &recent_messages,
                    &conversa,
                    &jev_result,
                    IdGate::G1,
                    group_ids,
                    (
                        queue_ms,
                        u64::try_from(assembly_started_at.elapsed().as_millis())
                            .unwrap_or(u64::MAX),
                    ),
                    started_at,
                    observer,
                )
                .await;
        }
        let g7_action = poph_jev::g7_action_from_agent(&hot_agent.agente.jev_json);
        if jev
            .as_ref()
            .and_then(|resultado| resultado.reliable_choice(IdGate::G7))
            .is_some_and(poph_jev::g7_requests_action)
        {
            let jev_result = jev.as_ref().expect("o G7 só decide com o resultado do Jev");
            self.metrics.g7_detectados.increment(1);
            tracing::warn!(
                conversation_id = %conversation_id,
                classification = jev
                    .as_ref()
                    .and_then(|resultado| resultado.reliable_choice(IdGate::G7))
                    .unwrap_or("—"),
                action = g7_action.as_text(),
                "G7: mensagem classificada como injeção ou abuso"
            );
            match g7_action {
                G7Action::Record => {}
                G7Action::DefaultReply => {
                    return self.send_g7_reply(
                        TurnTarget {
                            org,
                            conversa: conversation_id,
                            turno: turno.id,
                        },
                        &hot_agent,
                        jev_result,
                        group_ids,
                        (
                            queue_ms,
                            u64::try_from(assembly_started_at.elapsed().as_millis())
                                .unwrap_or(u64::MAX),
                        ),
                        started_at,
                        observer,
                    );
                }
                G7Action::Transfer => {
                    self.metrics.g7_transfer.increment(1);
                    return self
                        .transfer_to_human(
                            TurnTarget {
                                org,
                                conversa: conversation_id,
                                turno: turno.id,
                            },
                            &hot_agent,
                            &recent_messages,
                            &conversa,
                            jev_result,
                            IdGate::G7,
                            group_ids,
                            (
                                queue_ms,
                                u64::try_from(assembly_started_at.elapsed().as_millis())
                                    .unwrap_or(u64::MAX),
                            ),
                            started_at,
                            observer,
                        )
                        .await;
                }
            }
        }
        if can_end
            && jev
                .as_ref()
                .and_then(|resultado| resultado.reliable_choice(IdGate::G8))
                == Some(G8_ENDED)
        {
            let jev_result = jev.expect("o encerramento só existe com o resultado do Jev");
            return self.handle_g8_farewell(
                TurnTarget {
                    org,
                    conversa: conversation_id,
                    turno: turno.id,
                },
                &hot_agent,
                &jev_result,
                group_ids,
                (
                    queue_ms,
                    u64::try_from(assembly_started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
                ),
                started_at,
                observer,
            );
        }
        let g5_product = self
            .g5_product(
                G5Context {
                    org,
                    conversa: &conversa,
                    hot_agent: &hot_agent,
                    recent_messages: &recent_messages,
                    group: &group,
                    g3_nothing,
                },
                &mut jev,
            )
            .await;
        if let Some(jev) = &jev
            && jev.decision(IdGate::G5).is_some()
        {
            tracing::debug!(
                decision = ?jev.decision(IdGate::G5).map(|decisao| decisao.to_json()),
                "G5 do turno"
            );
        }

        let g9_enabled = self.jev.is_some() && self.agent_gates(&hot_agent.agente).g9;
        let turn_flow = match self.fluxos.as_ref() {
            Some(settings) => flow::decide(
                settings,
                &self.store,
                org,
                flow_preparation.as_ref(),
                jev.as_ref(),
                g9_enabled,
                self.tools.tool(flow::NAME_TOOL_START).is_some()
                    && hot_agent.tool_enabled(flow::NAME_TOOL_START),
            ),
            None => Default::default(),
        };
        self.record_g9_result(&turn_flow);

        if !chunks.is_empty() {
            self.metrics
                .passages_recovered
                .increment(chunks.len() as u64);
            observer.evento(TurnEvent::Base {
                sources: chunks
                    .iter()
                    .map(|chunk| format!("{} ({})", chunk.title, chunk.group))
                    .collect(),
            });
        }
        let block = prompt::BlockVolatile {
            now_ms: self.store.now(),
            canal: &conversa.canal,
            status: conversation_status(conversa.status),
            conversa: &conversa.id.to_string(),
            summary: history_assembly.summary.as_deref(),
            passages: &chunks,
            catalog_of_products: !g3_nothing && hot_agent.tool_enabled(crate::products::NAME_TOOL),
            product_chosen: g5_product.as_ref(),
            fluxos: turn_flow.texto.as_deref(),
        }
        .build();
        let messages = Prompt::new(hot_agent.markdown.clone())
            .with_volatile(block)
            .messages(history);
        let assembly_ms =
            u64::try_from(assembly_started_at.elapsed().as_millis()).unwrap_or(u64::MAX);

        let usage = UseTools::new();
        let calls = RecordCalls::new();
        if let Some(product) = &g5_product {
            usage.register_product(ProductUsed {
                product: product.product,
                nome: product.nome.clone(),
                price_cents: product.price_cents,
                price_text: product.price_text.clone(),
                unit: product.unit.clone(),
                conditions: product.conditions.clone(),
            });
            self.metrics.products_chosen_g5.increment(1);
        }
        let prepared = PreparedTurn {
            trigger_message_id: input.mensagem,
            messages,
            usage: Arc::clone(&usage),
            calls: Arc::clone(&calls),
            skip_knowledge_and_products: g3_nothing,
            skip_product_search: g5_product.is_some(),
            active_flow: turn_flow.in_progress,
            flow_fallback: turn_flow.reservation,
        };
        let run_model_loop = self.run_model_loop(
            TurnTarget {
                org,
                conversa: conversation_id,
                turno: turno.id,
            },
            &hot_agent,
            prepared,
            &cancellation,
            observer,
            started_at,
        );
        let (mut reply, final_conversation) =
            match tokio::time::timeout(self.options.turn_timeout, run_model_loop).await {
                Ok(Ok(pair)) => pair,
                Ok(Err(error)) => {
                    let texto = format!("erro do provedor: {error}");
                    return self.fail(
                        org,
                        turno.id,
                        &hot_agent.agente.modelo,
                        group_ids,
                        started_at,
                        queue_ms,
                        assembly_ms,
                        None,
                        texto,
                        llm_error_class(&error),
                        Some(&calls),
                        observer,
                    );
                }
                Err(_) => {
                    let texto = format!(
                        "tempo máximo do turno esgotado ({} s)",
                        self.options.turn_timeout.as_secs()
                    );
                    return self.fail(
                        org,
                        turno.id,
                        &hot_agent.agente.modelo,
                        group_ids,
                        started_at,
                        queue_ms,
                        assembly_ms,
                        None,
                        texto,
                        FailureClass::Modelo,
                        Some(&calls),
                        observer,
                    );
                }
            };
        drop(slot);

        let review = self
            .review_g6(
                G6Context {
                    org,
                    hot_agent: &hot_agent,
                    recent_messages: &recent_messages,
                    conversa: &conversa,
                    messages: &final_conversation,
                    products: usage.products(),
                    modelo: self.selected_model(&hot_agent),
                    cancellation: cancellation.clone(),
                },
                &mut reply,
                &mut jev,
            )
            .await;

        let timings = TurnTimings {
            queue_ms,
            assembly_ms,
            first_text_ms: reply.first_text_ms,
            total_ms: u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
            tool_rounds: reply.rounds,
        };

        let texto = reply.texto.trim().to_owned();
        if !texto.is_empty() {
            self.store.messages().write_with_output(
                org,
                NewMessage {
                    conversation_id,
                    direction: Direction::Outbound,
                    role: Role::Assistant,
                    texto: texto.clone(),
                    attachments_json: "[]".to_owned(),
                    external_id: None,
                    turn_id: Some(turno.id),
                    origin: poph_store::Origin::Agente,
                    author_name: None,
                },
                NewOutboxItem {
                    conversation_id,
                    turn_id: turno.id,
                    seq: 0,
                    tipo: "text".to_owned(),
                    content_json: json!({ "texto": texto }).to_string(),
                },
            )?;
            self.metrics.responses_stored.increment(1);
        }
        self.store
            .conversations()
            .touch(org, conversation_id, self.store.now())?;

        let modelo = self.selected_model(&hot_agent);
        let jev_json = match (&jev, &turn_flow.outcome_record) {
            (Some(item), _) if !item.decisions.is_empty() => {
                let mut record = item.to_json();
                if review.took_place() {
                    record["revisao"] = review.to_json();
                }
                if let Some(fluxo) = &turn_flow.outcome_record {
                    record["fluxo"] = fluxo.clone();
                }
                Some(record.to_string())
            }
            (_, Some(fluxo)) => Some(json!({ "fluxo": fluxo }).to_string()),
            _ => None,
        };
        let fim = TurnFinalization {
            state: Some(TurnState::Completed),
            jev_json,
            times_json: Some(timings_json(&timings, &modelo, reply.motivo.as_ref())),
            passages_json: Some(retrieved_json(&chunks, &usage.products())),
            tools_json: Some(record_turn(&reply.tools, &calls)),
            input_tokens: Some(u32::try_from(reply.usage.input).unwrap_or(u32::MAX)),
            output_tokens: Some(u32::try_from(reply.usage.saida).unwrap_or(u32::MAX)),
            ..TurnFinalization::default()
        };
        self.store.turns().finalise(org, turno.id, fim)?;
        self.metrics.turns_finished.increment(1);
        self.metrics.tokens_entry.increment(reply.usage.input);
        self.metrics.tokens_output.increment(reply.usage.saida);
        self.metrics
            .duration_turn_ms
            .observe(timings.total_ms as f64);
        if let Some(first) = timings.first_text_ms {
            self.metrics.first_text_ms.observe(first as f64);
        }
        observer.evento(TurnEvent::Fim {
            estado: TurnState::Completed.as_text().to_owned(),
            motivo: reply.motivo.as_ref().map(finish_reason_text),
        });

        self.summarizer
            .schedule(org, conversation_id, modelo.clone());

        Ok(TurnSummary {
            turno: turno.id,
            estado: TurnState::Completed,
            texto,
            messages: group_ids,
            input_tokens: u32::try_from(reply.usage.input).unwrap_or(u32::MAX),
            output_tokens: u32::try_from(reply.usage.saida).unwrap_or(u32::MAX),
            timings,
            modelo,
            motivo_fim: reply.motivo.as_ref().map(finish_reason_text),
            error: None,
            model_failure: false,
            jev,
        })
    }

    async fn run_model_loop(
        &self,
        target: TurnTarget,
        hot_agent: &HotAgent,
        prepared: PreparedTurn,
        cancellation: &Cancellation,
        observer: &mut dyn TurnObserver,
        started_at: Instant,
    ) -> Result<(ModelReply, Vec<Message>), LlmError> {
        let max_rounds = hot_agent.agente.max_tool_rounds.max(1);
        let max_per_message = hot_agent.agente.max_tools_per_message.max(1);
        let mut offered = tools::offered(self.tools.as_ref(), &hot_agent.tools);
        offered.extend(hot_agent.tools.iter().filter_map(|name| {
            self.http_tools
                .tool(target.org, name)
                .map(|tool| tool.definition(name))
        }));
        let http_context = HttpToolContext {
            org_id: target.org,
            agent_id: hot_agent.agente.id,
            conversation_id: target.conversa,
            turn_id: target.turno,
            message_id: prepared.trigger_message_id,
        };
        let mut http_calls = HttpToolCalls::default();
        if prepared.skip_knowledge_and_products {
            offered.retain(|tool| !is_knowledge_or_product_tool(&tool.nome));
        } else if prepared.skip_product_search {
            offered.retain(|tool| tool.nome != crate::products::NAME_TOOL);
        }
        prepared.retain_flow_tools(&mut offered);
        crate::library::attach_library(&mut offered, &self.store, target.org, hot_agent.agente.id);
        let modelo = self.selected_model(hot_agent);
        let turn_provider = self.turn_provider(hot_agent)?;

        let usage = &prepared.usage;
        let turn_calls = &prepared.calls;
        let mut conversa = prepared.messages;
        let mut reply = ModelReply::default();
        for round in 0..=max_rounds {
            let list = if round == max_rounds {
                Vec::new()
            } else {
                offered.clone()
            };
            let mut request = Request::new(target.org, conversa.clone());
            request.tools = list;
            request.modelo = Some(modelo.clone());
            request.cancellation = cancellation.clone();
            request.temperatura = hot_agent.agente.temperatura.map(|value| value as f32);
            request.max_tokens = hot_agent
                .agente
                .max_tokens_saida
                .and_then(|value| u32::try_from(value).ok());

            let provider_slot = self
                .limiter
                .acquire_provider(turn_provider.nome(), self.options.slot_wait)
                .await
                .map_err(provider_slot_error)?;

            let mut fluxo = turn_provider.client().stream(request).await?;
            let mut round_text = String::new();
            let mut calls: Vec<ToolCall> = Vec::new();
            while let Some(evento) = self.next_event(&mut fluxo, cancellation).await? {
                match evento? {
                    LlmEvent::Texto(chunk) => {
                        if reply.first_text_ms.is_none() {
                            reply.first_text_ms = Some(
                                u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
                            );
                        }
                        round_text.push_str(&chunk);
                        observer.evento(TurnEvent::Texto(chunk));
                    }
                    LlmEvent::Tool(call) => calls.push(call),
                    LlmEvent::Usage(usage) => reply.usage = add_usage(reply.usage, usage),
                    LlmEvent::Fim(motivo) => reply.motivo = Some(motivo),
                }
            }
            drop(provider_slot);
            reply.texto.push_str(&round_text);

            if calls.is_empty() {
                break;
            }
            reply.rounds += 1;
            conversa.push(Message::Assistant {
                texto: round_text,
                calls: calls.clone(),
            });
            for (index, call) in calls.iter().enumerate() {
                let http_tool = self.http_tools.tool(target.org, &call.nome);
                let audit_arguments = if http_tool.is_some() {
                    &serde_json::Value::Null
                } else {
                    &call.arguments
                };
                if index >= max_per_message as usize {
                    let content = format!(
                        "erro: limite de {max_per_message} chamadas de ferramenta por \
                         mensagem excedido"
                    );
                    turn_calls.register(
                        &call.nome,
                        round,
                        Desfecho::Refused(REASON_LIMIT_OF_CALLS),
                        Duration::ZERO,
                        audit_arguments,
                        &content,
                    );
                    conversa.push(Message::Resultado {
                        id: call.id.clone(),
                        content,
                    });
                    continue;
                }
                observer.evento(TurnEvent::Tool {
                    nome: call.nome.clone(),
                    round,
                });
                reply.tools.push(call.nome.clone());
                let call_started_at = Instant::now();
                let desfecho = if prepared.skip_knowledge_and_products
                    && is_knowledge_or_product_tool(&call.nome)
                {
                    ToolOutcome::rejected(
                        REASON_OUTSIDE_DO_TURN,
                        format!(
                            "erro: a ferramenta `{}` não faz parte deste turno (a mensagem não \
                             exige consulta)",
                            call.nome
                        ),
                    )
                } else if call.nome == flow_tools::NAME_START && !prepared.flow_fallback {
                    ToolOutcome::rejected(
                        REASON_OUTSIDE_DO_TURN,
                        "erro: iniciar fluxo não é sua decisão neste turno; siga a orientação do bloco <fluxo_em_andamento>"
                            .to_owned(),
                    )
                } else if prepared.skip_product_search && call.nome == crate::products::NAME_TOOL {
                    ToolOutcome::rejected(
                        REASON_OUTSIDE_DO_TURN,
                        "erro: o catálogo já foi consultado e o produto do cliente está no bloco \
                         <produtos_encontrados> deste turno; não chame buscar_produto de novo"
                            .to_owned(),
                    )
                } else if let Some(tool) = http_tool
                    && round < max_rounds
                    && hot_agent.tool_enabled(&call.nome)
                {
                    match http_calls.execute(tool, http_context, call).await {
                        Ok(text) => ToolOutcome::Executed(text),
                        Err(error) => ToolOutcome::rejected(
                            tool_error_reason(&error),
                            format!("erro: {error}"),
                        ),
                    }
                } else {
                    self.execute_tool(target, hot_agent, usage, call).await
                };
                let duration = call_started_at.elapsed();
                turn_calls.register(
                    &call.nome,
                    round,
                    desfecho.for_record(),
                    duration,
                    audit_arguments,
                    if http_tool.is_some() {
                        ""
                    } else {
                        desfecho.texto()
                    },
                );
                conversa.push(Message::Resultado {
                    id: call.id.clone(),
                    content: desfecho.into_text(),
                });
            }
        }
        Ok((reply, conversa))
    }

    fn start_search<'a>(
        &'a self,
        org: OrgId,
        agente: AgentId,
        group: &[StoredMessage],
        hot_agent: &HotAgent,
    ) -> Option<crate::base_knowledge::SearchFuture<'a>> {
        let base = self.search.as_ref()?;
        let query = group_query(group);
        if query.is_empty() {
            return None;
        }
        Some(base.search(
            org,
            agente,
            query,
            SearchParameters {
                top_k: usize::try_from(hot_agent.top_k).unwrap_or(3).max(1),
                cutoff: hot_agent.cutoff as f32,
            },
        ))
    }

    async fn g5_product(
        &self,
        context: G5Context<'_>,
        jev: &mut Option<JevResult>,
    ) -> Option<FoundProduct> {
        let G5Context {
            org,
            conversa,
            hot_agent,
            recent_messages,
            group,
            g3_nothing,
        } = context;
        let evaluator = Arc::clone(self.jev.as_ref()?);
        if g3_nothing
            || !self.agent_gates(&hot_agent.agente).g5
            || !hot_agent.tool_enabled(crate::products::NAME_TOOL)
        {
            return None;
        }
        let products = self.products.as_ref()?;
        let query = group_query(group);
        if query.is_empty() {
            return None;
        }
        let parameters = ProductParameters {
            top_k: usize::try_from(hot_agent.top_k).unwrap_or(3).clamp(1, 10),
            cutoff: hot_agent.cutoff as f32,
        };
        let candidates = match products
            .search(org, hot_agent.agente.id, query, None, parameters)
            .await
        {
            Ok(candidates) => candidates,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "G5: a busca de produtos falhou; turno segue como sem o gate"
                );
                return None;
            }
        };
        if candidates.is_empty() {
            return None;
        }
        let state_tools: Vec<poph_jev::ProductCandidate> =
            candidates.iter().map(g5_candidate).collect();
        let gates = self.agent_gates(&hot_agent.agente);
        let estado = self
            .jev_state(org, hot_agent, recent_messages, conversa, gates, None)?
            .with_product_candidates(state_tools);
        let resultado = evaluator
            .evaluate(estado, Some(poph_jev::GateOverrides::only(IdGate::G5)))
            .await;
        let choice = resultado.reliable_choice(IdGate::G5).map(str::to_owned);
        match jev {
            Some(current) => {
                current.decisions.extend(resultado.decisions);
                current.raw.extend(resultado.raw);
                current.call.questions += resultado.call.questions;
                current.call.ms = current.call.ms.saturating_add(resultado.call.ms);
                current.call.tokens = current.call.tokens.saturating_add(resultado.call.tokens);
                current.call.usage = current.call.usage.plus(resultado.call.usage);
                if current.call.failure.is_none() {
                    current.call.failure = resultado.call.failure;
                }
            }
            None => *jev = Some(resultado),
        }
        let choice = choice?;
        candidates
            .into_iter()
            .find(|product| product.product.to_string() == choice)
    }

    async fn review_g6(
        &self,
        context: G6Context<'_>,
        reply: &mut ModelReply,
        jev: &mut Option<JevResult>,
    ) -> G6Review {
        let G6Context {
            org,
            hot_agent,
            recent_messages,
            conversa,
            messages,
            products,
            modelo,
            cancellation,
        } = context;
        let mut review = G6Review::default();
        let Some(evaluator) = self.jev.as_ref().map(Arc::clone) else {
            return review;
        };
        if !self.agent_gates(&hot_agent.agente).g6 {
            return review;
        }
        if reply.texto.trim().is_empty() || messages.is_empty() {
            return review;
        }

        let Some(choice) = self
            .review_text(
                org,
                hot_agent,
                recent_messages,
                conversa,
                &products,
                &reply.texto,
                evaluator.as_ref(),
                jev,
            )
            .await
        else {
            return review;
        };
        review.choice = Some(choice.clone());
        review.confidence = jev
            .as_ref()
            .and_then(|resultado| resultado.decision(IdGate::G6))
            .map(|decisao| decisao.confidence)
            .unwrap_or(0.0);
        if choice == G6_APPROVED {
            return review;
        }

        self.metrics.reprovadas_g6.increment(1);
        let guidance = poph_jev::g6_guidance(&choice);
        let regenerated = match self
            .regenerate(
                org,
                hot_agent,
                messages,
                &reply.texto,
                guidance,
                &modelo,
                &cancellation,
            )
            .await
        {
            Ok(regenerated) => regenerated,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "G6: a regeneração falhou; o turno segue com a resposta original"
                );
                return review;
            }
        };
        reply.usage = add_usage(reply.usage, regenerated.usage);
        if regenerated.motivo.is_some() {
            reply.motivo = regenerated.motivo;
        }
        let new = regenerated.texto.trim().to_owned();
        if new.is_empty() {
            tracing::warn!("G6: a regeneração voltou vazia; o turno segue com a resposta original");
            return review;
        }
        reply.texto = new;
        review.did_regenerate = true;

        let second_choice = self
            .review_text(
                org,
                hot_agent,
                recent_messages,
                conversa,
                &products,
                &reply.texto,
                evaluator.as_ref(),
                jev,
            )
            .await;
        review.second_choice = second_choice.clone();
        if second_choice.as_deref() == Some(G6_APPROVED) {
            return review;
        }
        if let Some(safe_reply) = poph_jev::safe_response_from_agent(&hot_agent.agente.jev_json) {
            reply.texto = safe_reply;
            review.safe_reply = true;
            self.metrics.responses_safe_g6.increment(1);
        }
        review
    }

    #[allow(clippy::too_many_arguments)]
    async fn review_text(
        &self,
        org: OrgId,
        hot_agent: &HotAgent,
        recent_messages: &[StoredMessage],
        conversa: &poph_store::Conversa,
        products: &[ProductUsed],
        texto: &str,
        evaluator: &dyn GateEvaluator,
        jev: &mut Option<JevResult>,
    ) -> Option<String> {
        let gates = self.agent_gates(&hot_agent.agente);
        let candidates: Vec<poph_jev::ProductCandidate> = products
            .iter()
            .map(|product| poph_jev::ProductCandidate {
                id: product.product.to_string(),
                nome: product.nome.clone(),
                detail: format!(
                    "{} {} · {}",
                    product.price_text, product.unit, product.conditions
                ),
            })
            .collect();
        let estado = self
            .jev_state(org, hot_agent, recent_messages, conversa, gates, None)?
            .with_candidate_response(texto)
            .with_product_candidates(candidates);
        let resultado = evaluator
            .evaluate(estado, Some(GateOverrides::only(IdGate::G6)))
            .await;
        let choice = resultado.reliable_choice(IdGate::G6).map(str::to_owned);
        merge_jev_result(jev, resultado);
        self.metrics.revisions_g6.increment(1);
        choice
    }

    #[allow(clippy::too_many_arguments)]
    async fn regenerate(
        &self,
        org: OrgId,
        hot_agent: &HotAgent,
        messages: &[Message],
        draft: &str,
        guidance: &str,
        modelo: &str,
        cancellation: &Cancellation,
    ) -> Result<RegeneratedReply, LlmError> {
        let mut conversa = messages.to_vec();
        conversa.push(Message::Assistant {
            texto: draft.to_owned(),
            calls: Vec::new(),
        });
        conversa.push(Message::User(vec![Part::Texto(guidance.to_owned())]));
        let mut request = Request::new(org, conversa);
        request.modelo = Some(modelo.to_owned());
        request.cancellation = cancellation.clone();
        request.temperatura = hot_agent.agente.temperatura.map(|value| value as f32);
        request.max_tokens = hot_agent
            .agente
            .max_tokens_saida
            .and_then(|value| u32::try_from(value).ok());
        let turn_provider = self.turn_provider(hot_agent)?;
        let _slot = self
            .limiter
            .acquire_provider(turn_provider.nome(), self.options.slot_wait)
            .await
            .map_err(provider_slot_error)?;
        let mut fluxo = turn_provider.client().stream(request).await?;
        let mut regenerated = RegeneratedReply::default();
        while let Some(evento) = self.next_event(&mut fluxo, cancellation).await? {
            match evento? {
                LlmEvent::Texto(chunk) => regenerated.texto.push_str(&chunk),
                LlmEvent::Tool(_) => {}
                LlmEvent::Usage(usage) => regenerated.usage = add_usage(regenerated.usage, usage),
                LlmEvent::Fim(motivo) => regenerated.motivo = Some(motivo),
            }
        }
        Ok(regenerated)
    }

    fn start_jev(
        &self,
        org: OrgId,
        hot_agent: &HotAgent,
        recent_messages: &[StoredMessage],
        conversa: &poph_store::Conversa,
        fluxos: Option<&[poph_jev::JevFlow]>,
    ) -> Option<futures::future::BoxFuture<'static, JevResult>> {
        let evaluator = Arc::clone(self.jev.as_ref()?);
        let override_config: Option<GateOverrides> =
            poph_jev::agent_overrides(&hot_agent.agente.jev_json);
        let gates: JevGates = evaluator.effective_gates(override_config);
        if JevEngine::none_enabled(gates) {
            return None;
        }
        let estado = self.jev_state(org, hot_agent, recent_messages, conversa, gates, fluxos)?;
        Some(Box::pin(async move {
            evaluator.evaluate(estado, override_config).await
        }))
    }

    fn jev_state(
        &self,
        org: OrgId,
        hot_agent: &HotAgent,
        recent_messages: &[StoredMessage],
        conversa: &poph_store::Conversa,
        gates: JevGates,
        fluxos: Option<&[poph_jev::JevFlow]>,
    ) -> Option<JevState> {
        let messages: Vec<JevMessage> = recent_messages.iter().filter_map(jev_message).collect();
        if messages.is_empty() {
            return None;
        }
        let mut estado = JevState::new(conversa.canal.clone())
            .with_messages(messages)
            .with_handoff(
                poph_jev::handoff_situations_from_agent(&hot_agent.agente.jev_json),
                hot_agent.agente.handoff_active,
            )
            .with_capabilities(
                self.search.is_some(),
                hot_agent.tool_enabled(crate::products::NAME_TOOL),
            );
        if gates.g4 {
            estado = estado.with_departments(self.active_department_names(org));
        }
        if gates.g9
            && let Some(fluxos) = fluxos
        {
            estado = estado.with_released_flows(fluxos.iter().cloned());
        }
        if let Some(texto) = self.attachment_text(org, recent_messages) {
            estado = estado.with_file_text(texto);
        }
        Some(estado)
    }

    fn attachment_parts(
        &self,
        org: OrgId,
        mensagem: &StoredMessage,
        images: &mut ImageBudget,
    ) -> Vec<Part> {
        let attachment_json = mensagem.attachments_json.trim();
        if attachment_json.is_empty() || attachment_json == "[]" {
            return Vec::new();
        }
        let anexos = match self.store.messages().attachments(org, mensagem.id) {
            Ok(anexos) => anexos,
            Err(error) => {
                tracing::warn!(error = %error, "não consegui ler os anexos da mensagem");
                return Vec::new();
            }
        };
        let mut parts: Vec<Part> = Vec::new();
        let mut texts: Vec<String> = Vec::new();
        for attachment in &anexos {
            if poph_core::llm::IMAGE_MIMES.contains(&attachment.mime.as_str()) {
                if let Some(part) = self.attachment_image(org, attachment, images) {
                    parts.push(part);
                }
                continue;
            }
            if let Some(texto) = &attachment.extracted_text {
                let texto = texto.trim();
                if !texto.is_empty() {
                    texts.push(texto.to_owned());
                }
            }
        }
        if !texts.is_empty() {
            parts.insert(
                0,
                Part::Texto(poph_files::envelope::UNTRUSTED_NOTICE.to_owned()),
            );
            parts.insert(1, Part::Texto(texts.join("\n\n")));
        }
        parts
    }

    fn attachment_image(
        &self,
        org: OrgId,
        attachment: &poph_store::AttachmentRecord,
        images: &mut ImageBudget,
    ) -> Option<Part> {
        let Some(files) = self.files.as_ref() else {
            tracing::debug!(
                attachment_id = %attachment.id,
                "sem configuração de arquivos no turno: imagem do anexo não vai ao modelo"
            );
            return None;
        };
        if !images.fits(attachment.size_bytes) {
            tracing::warn!(
                attachment_id = %attachment.id,
                size_bytes = attachment.size_bytes,
                "imagem omitida do turno: passa do teto de imagens por turno"
            );
            return None;
        }
        match poph_files::blob::read(
            &files.blob_folder,
            org,
            &attachment.sha256,
            MAX_BYTES_PER_IMAGE,
        ) {
            Ok(bytes) if !bytes.is_empty() => {
                images.consume(bytes.len() as u64);
                Some(Part::Image {
                    mime: attachment.mime.clone(),
                    bytes,
                })
            }
            Ok(_) => {
                tracing::warn!(attachment_id = %attachment.id, "imagem vazia no blob; turno segue sem ela");
                None
            }
            Err(error) => {
                tracing::warn!(
                    attachment_id = %attachment.id,
                    error = %error,
                    "não consegui ler a imagem do anexo; turno segue sem ela"
                );
                None
            }
        }
    }

    fn attachment_text(&self, org: OrgId, messages: &[StoredMessage]) -> Option<String> {
        let mut texts: Vec<String> = Vec::new();
        for mensagem in messages {
            if mensagem.role != Role::User {
                continue;
            }
            let anexos = mensagem.attachments_json.trim();
            if anexos.is_empty() || anexos == "[]" {
                continue;
            }
            match self.store.messages().attachments(org, mensagem.id) {
                Ok(anexos) => {
                    for attachment in anexos {
                        if let Some(texto) = attachment.extracted_text {
                            let texto = texto.trim();
                            if !texto.is_empty() {
                                texts.push(texto.to_owned());
                            }
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(error = %error, "não consegui ler os anexos da mensagem");
                }
            }
        }
        if texts.is_empty() {
            None
        } else {
            Some(texts.join("\n\n"))
        }
    }

    fn active_department_names(&self, org: OrgId) -> Vec<String> {
        self.active_departments_with_id(org)
            .into_iter()
            .map(|candidate| candidate.nome)
            .collect()
    }

    fn active_departments_with_id(&self, org: OrgId) -> Vec<department::Candidate> {
        match self.store.departments().list(org, Page::with_limit(200)) {
            Ok(departments) => departments
                .iter()
                .filter(|departamento| departamento.status == poph_store::Status::Active)
                .map(department::Candidate::de)
                .collect(),
            Err(error) => {
                tracing::warn!(error = %error, "não consegui listar departamentos para o G4");
                Vec::new()
            }
        }
    }

    async fn next_event(
        &self,
        fluxo: &mut poph_core::llm::LlmStream,
        cancellation: &Cancellation,
    ) -> Result<Option<Result<LlmEvent, LlmError>>, LlmError> {
        match &self.cancellation {
            Some(shared) => tokio::select! {
                evento = fluxo.next() => Ok(evento),
                _ = shared.wait() => {
                    cancellation.cancel();
                    Err(LlmError::Cancelled)
                }
            },
            None => Ok(fluxo.next().await),
        }
    }

    async fn execute_tool(
        &self,
        target: TurnTarget,
        hot_agent: &HotAgent,
        usage: &Arc<UseTools>,
        call: &ToolCall,
    ) -> ToolOutcome {
        if !hot_agent.tool_enabled(&call.nome) {
            return ToolOutcome::rejected(
                REASON_NOT_ENABLED,
                format!(
                    "erro: a ferramenta `{}` não está habilitada para este agente",
                    call.nome
                ),
            );
        }
        let Some(tool) = self.tools.tool(&call.nome) else {
            return ToolOutcome::rejected(
                REASON_UNKNOWN,
                format!(
                    "erro: a ferramenta `{}` não está disponível neste motor",
                    call.nome
                ),
            );
        };
        let context = ToolContext {
            org: target.org,
            conversa: target.conversa,
            agente: hot_agent.agente.id,
            turno: target.turno,
            usage: Arc::clone(usage),
        };
        match tool.run(context, call.arguments.clone()).await {
            Ok(texto) => ToolOutcome::Executed(texto),
            Err(error) => {
                ToolOutcome::rejected(tool_error_reason(&error), format!("erro: {error}"))
            }
        }
    }

    pub fn agent_window(&self, agente: &poph_store::Agente) -> Duration {
        let agent = Duration::from_millis(u64::from(agente.grouping_window_ms));
        let window = if agent.is_zero() {
            self.options.default_window
        } else {
            agent
        };
        window.min(self.options.max_window)
    }

    fn read_recent_messages(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> Resultado<Vec<StoredMessage>> {
        Ok(self
            .store
            .messages()
            .last_set(org, conversa, self.options.message_read_limit.max(1))?)
    }

    fn record_g9_result(&self, desfecho: &flow::FlowOutcome) {
        let Some(record) = desfecho.outcome_record.as_ref() else {
            return;
        };
        match record.get("resultado").and_then(serde_json::Value::as_str) {
            Some("iniciou" | "iniciou_sem_bloco") => self.g9_metrics.start.increment(1),
            Some("nenhum") => self.g9_metrics.none.increment(1),
            Some("abaixo_do_limiar") => self.g9_metrics.below_threshold.increment(1),
            Some("fallback" | "falhou" | "nao_iniciou") => self.g9_metrics.fallback.increment(1),
            _ => {}
        }
    }

    fn agent_gates(&self, agente: &poph_store::Agente) -> JevGates {
        let Some(evaluator) = self.jev.as_ref() else {
            return JevGates::default();
        };
        evaluator.effective_gates(poph_jev::agent_overrides(&agente.jev_json))
    }

    fn g2_enabled(&self, agente: &poph_store::Agente) -> bool {
        self.jev.is_some() && self.agent_gates(agente).g2
    }

    fn selected_model(&self, hot_agent: &HotAgent) -> String {
        let modelo = hot_agent.agente.modelo.trim();
        if !modelo.is_empty() {
            return modelo.to_owned();
        }
        match self.turn_provider(hot_agent) {
            Ok(provedor) => provedor.modelo().to_owned(),
            Err(_) => self.default_model.clone(),
        }
    }

    fn turn_provider(&self, hot_agent: &HotAgent) -> Result<ResolvedProvider, LlmError> {
        let Some(providers) = &self.providers else {
            return Ok(ResolvedProvider::new(
                hot_agent.agente.provedor.clone(),
                Arc::clone(&self.provedor),
                self.default_model.clone(),
            ));
        };
        providers.resolver(&hot_agent.agente.provedor)
    }

    #[allow(clippy::too_many_arguments)]
    async fn transfer_to_human(
        &self,
        target: TurnTarget,
        hot_agent: &HotAgent,
        recent_messages: &[StoredMessage],
        conversa: &poph_store::Conversa,
        jev: &JevResult,
        origin: IdGate,
        group_ids: Vec<MessageId>,
        (queue_ms, assembly_ms): (u64, u64),
        started_at: Instant,
        observer: &mut dyn TurnObserver,
    ) -> Resultado<TurnSummary> {
        let org = target.org;
        let conversation_id = target.conversa;
        let turno = target.turno;

        let risk = self
            .classify_risk(org, hot_agent, recent_messages, conversa)
            .await;
        let tipo = self.handoff.reliable_type(&risk);
        let texto = self.handoff.compose(tipo.as_deref());

        if texto.trim().is_empty() {
            tracing::warn!(
                conversation_id = %conversation_id,
                "transferência sem mensagem automática configurada: cliente não recebe texto"
            );
        } else {
            self.store.messages().write_with_output(
                org,
                NewMessage {
                    conversation_id,
                    direction: Direction::Outbound,
                    role: Role::Assistant,
                    texto: texto.clone(),
                    attachments_json: "[]".to_owned(),
                    external_id: None,
                    turn_id: Some(turno),
                    origin: poph_store::Origin::Agente,
                    author_name: None,
                },
                NewOutboxItem {
                    conversation_id,
                    turn_id: turno,
                    seq: 0,
                    tipo: "text".to_owned(),
                    content_json: json!({ "texto": texto }).to_string(),
                },
            )?;
            self.metrics.responses_stored.increment(1);
        }

        let g4_choice = jev.reliable_choice(IdGate::G4);
        let abstention_reason = jev
            .decision(IdGate::G4)
            .and_then(|decisao| decisao.motivo.clone());
        let candidates = if g4_choice.is_some() {
            self.active_departments_with_id(org)
        } else {
            Vec::new()
        };
        let destination = department::resolver(
            g4_choice,
            &candidates,
            hot_agent.agente.handoff_department,
            abstention_reason.as_deref(),
        );
        if let Some(motivo) = &destination.motivo {
            tracing::info!(
                conversation_id = %conversation_id,
                jev_choice = ?g4_choice,
                reason = %motivo,
                "departamento do G4 não aplicado; usando o padrão do agente"
            );
        }

        self.store
            .conversations()
            .touch(org, conversation_id, self.store.now())?;
        let departamento = destination.departamento;
        let detail = json!({
            "gate": origin.as_text(),
            "decisao": jev.choice(origin),
            "confianca": jev.decision(origin).map(|decisao| decisao.confidence),
            "g1_decisao": jev.choice(IdGate::G1),
            "risco": tipo,
            "orientacao": tipo.is_some(),
            "departamento": departamento.map(|dep| dep.to_string()),
            "departamento_origem": destination.origin.as_text(),
            "departamento_escolhido_pelo_jev": g4_choice,
            "departamento_motivo": destination.motivo,
        })
        .to_string();
        self.store.conversations().transfer(
            org,
            conversation_id,
            departamento,
            Some("turno".to_owned()),
            detail,
        )?;

        let timings = TurnTimings {
            queue_ms,
            assembly_ms,
            first_text_ms: None,
            total_ms: u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
            tool_rounds: 0,
        };
        let modelo = self.selected_model(hot_agent);
        let mut registro = jev.to_json();
        registro["risco"] = risk.to_json();
        registro["transferencia"] = json!({
            "gate": origin.as_text(),
            "departamento": departamento.map(|dep| dep.to_string()),
            "origem": destination.origin.as_text(),
            "escolha_do_jev": g4_choice,
            "motivo": destination.motivo,
        });
        self.store.turns().finalise(
            org,
            turno,
            TurnFinalization {
                state: Some(TurnState::Completed),
                jev_json: Some(registro.to_string()),
                times_json: Some(timings_json(&timings, &modelo, None)),
                ..TurnFinalization::default()
            },
        )?;
        self.metrics.turns_finished.increment(1);
        self.metrics
            .duration_turn_ms
            .observe(timings.total_ms as f64);
        tracing::info!(
            conversation_id = %conversation_id,
            department_id = ?departamento,
            department_source = destination.origin.as_text(),
            guidance_included = tipo.is_some(),
            "conversa transferida para humano (G1)"
        );
        observer.evento(TurnEvent::Handoff {
            departamento,
            risk_kind: tipo.clone(),
            guidance: tipo.is_some(),
        });
        observer.evento(TurnEvent::Texto(texto.clone()));
        observer.evento(TurnEvent::Fim {
            estado: TurnState::Completed.as_text().to_owned(),
            motivo: Some("transferida para humano".to_owned()),
        });

        Ok(TurnSummary {
            turno,
            estado: TurnState::Completed,
            texto,
            messages: group_ids,
            input_tokens: 0,
            output_tokens: 0,
            timings,
            modelo,
            motivo_fim: Some("transferida para humano".to_owned()),
            error: None,
            model_failure: false,
            jev: Some(jev.clone()),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn send_g7_reply(
        &self,
        target: TurnTarget,
        hot_agent: &HotAgent,
        jev: &JevResult,
        group_ids: Vec<MessageId>,
        (queue_ms, assembly_ms): (u64, u64),
        started_at: Instant,
        observer: &mut dyn TurnObserver,
    ) -> Resultado<TurnSummary> {
        let org = target.org;
        let conversation_id = target.conversa;
        let turno = target.turno;

        let texto = poph_jev::g7_response_from_agent(&hot_agent.agente.jev_json)
            .unwrap_or_else(|| G7_DEFAULT_REPLY.to_owned());
        self.store.messages().write_with_output(
            org,
            NewMessage {
                conversation_id,
                direction: Direction::Outbound,
                role: Role::Assistant,
                texto: texto.clone(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: Some(turno),
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
            NewOutboxItem {
                conversation_id,
                turn_id: turno,
                seq: 0,
                tipo: "text".to_owned(),
                content_json: json!({ "texto": texto }).to_string(),
            },
        )?;
        self.metrics.responses_stored.increment(1);
        self.store
            .conversations()
            .touch(org, conversation_id, self.store.now())?;

        let timings = TurnTimings {
            queue_ms,
            assembly_ms,
            first_text_ms: None,
            total_ms: u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
            tool_rounds: 0,
        };
        let modelo = self.selected_model(hot_agent);
        let motivo = "mensagem classificada como injeção ou abuso (G7)".to_owned();
        let mut registro = jev.to_json();
        registro["classificacao"] = json!({
            "gate": IdGate::G7.as_text(),
            "escolha": jev.choice(IdGate::G7),
            "confianca": jev.decision(IdGate::G7).map(|decisao| decisao.confidence),
            "acao": G7Action::DefaultReply.as_text(),
        });
        self.store.turns().finalise(
            org,
            turno,
            TurnFinalization {
                state: Some(TurnState::Completed),
                jev_json: Some(registro.to_string()),
                times_json: Some(timings_json(&timings, &modelo, None)),
                ..TurnFinalization::default()
            },
        )?;
        self.metrics.turns_finished.increment(1);
        self.metrics.g7_responses_default.increment(1);
        self.metrics
            .duration_turn_ms
            .observe(timings.total_ms as f64);
        tracing::warn!(
            conversation_id = %conversation_id,
            "G7: resposta neutra enviada sem chamar o modelo de texto"
        );
        observer.evento(TurnEvent::Texto(texto.clone()));
        observer.evento(TurnEvent::Fim {
            estado: TurnState::Completed.as_text().to_owned(),
            motivo: Some(motivo.clone()),
        });

        Ok(TurnSummary {
            turno,
            estado: TurnState::Completed,
            texto,
            messages: group_ids,
            input_tokens: 0,
            output_tokens: 0,
            timings,
            modelo,
            motivo_fim: Some(motivo),
            error: None,
            model_failure: false,
            jev: Some(jev.clone()),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_g8_farewell(
        &self,
        target: TurnTarget,
        hot_agent: &HotAgent,
        jev: &JevResult,
        group_ids: Vec<MessageId>,
        (queue_ms, assembly_ms): (u64, u64),
        started_at: Instant,
        observer: &mut dyn TurnObserver,
    ) -> Resultado<TurnSummary> {
        let org = target.org;
        let conversation_id = target.conversa;
        let turno = target.turno;

        let texto = poph_jev::farewell_from_agent(&hot_agent.agente.jev_json)
            .unwrap_or_else(|| G8_DEFAULT_FAREWELL.to_owned())
            .trim()
            .to_owned();
        if !texto.is_empty() {
            self.store.messages().write_with_output(
                org,
                NewMessage {
                    conversation_id,
                    direction: Direction::Outbound,
                    role: Role::Assistant,
                    texto: texto.clone(),
                    attachments_json: "[]".to_owned(),
                    external_id: None,
                    turn_id: Some(turno),
                    origin: poph_store::Origin::Agente,
                    author_name: None,
                },
                NewOutboxItem {
                    conversation_id,
                    turn_id: turno,
                    seq: 0,
                    tipo: "text".to_owned(),
                    content_json: json!({ "texto": texto }).to_string(),
                },
            )?;
            self.metrics.responses_stored.increment(1);
        }
        self.store
            .conversations()
            .touch(org, conversation_id, self.store.now())?;

        let timings = TurnTimings {
            queue_ms,
            assembly_ms,
            first_text_ms: None,
            total_ms: u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
            tool_rounds: 0,
        };
        let modelo = self.selected_model(hot_agent);
        let motivo = "conversa encerrada pelo cliente (G8)".to_owned();
        let mut registro = jev.to_json();
        registro["encerramento"] = json!({
            "gate": IdGate::G8.as_text(),
            "confianca": jev.decision(IdGate::G8).map(|decisao| decisao.confidence),
            "despedida": !texto.is_empty(),
        });
        self.store.turns().finalise(
            org,
            turno,
            TurnFinalization {
                state: Some(TurnState::Completed),
                jev_json: Some(registro.to_string()),
                times_json: Some(timings_json(&timings, &modelo, None)),
                ..TurnFinalization::default()
            },
        )?;
        self.metrics.turns_finished.increment(1);
        self.metrics.closed_g8.increment(1);
        self.metrics
            .duration_turn_ms
            .observe(timings.total_ms as f64);
        tracing::info!(
            conversation_id = %conversation_id,
            farewell_sent = !texto.is_empty(),
            "conversa encerrada pelo cliente (G8)"
        );
        if !texto.is_empty() {
            observer.evento(TurnEvent::Texto(texto.clone()));
        }
        observer.evento(TurnEvent::Fim {
            estado: TurnState::Completed.as_text().to_owned(),
            motivo: Some(motivo.clone()),
        });

        Ok(TurnSummary {
            turno,
            estado: TurnState::Completed,
            texto,
            messages: group_ids,
            input_tokens: 0,
            output_tokens: 0,
            timings,
            modelo,
            motivo_fim: Some(motivo),
            error: None,
            model_failure: false,
            jev: Some(jev.clone()),
        })
    }

    async fn classify_risk(
        &self,
        org: OrgId,
        hot_agent: &HotAgent,
        recent_messages: &[StoredMessage],
        conversa: &poph_store::Conversa,
    ) -> GuidanceResult {
        if !self.handoff.ativo {
            return GuidanceResult::not_queried("orientação desligada na configuração");
        }
        let Some(evaluator) = self.jev.as_ref() else {
            return GuidanceResult::not_queried("Jev desligado no motor");
        };
        let override_config: Option<GateOverrides> =
            poph_jev::agent_overrides(&hot_agent.agente.jev_json);
        let gates: JevGates = evaluator.effective_gates(override_config);
        let Some(estado) = self.jev_state(org, hot_agent, recent_messages, conversa, gates, None)
        else {
            return GuidanceResult::not_queried("sem mensagem para julgar");
        };
        let resultado = evaluator.orientar(estado, self.handoff.timeout).await;
        if resultado.failed() {
            tracing::warn!(
                failure = ?resultado.failure,
                "pergunta de risco da transferência falhou; segue só com a mensagem automática"
            );
        }
        resultado
    }

    fn finish_skipped(
        &self,
        org: OrgId,
        conversa: ConversationId,
        agente: Option<AgentId>,
        motivo: String,
        started_at: Instant,
    ) -> Resultado<TurnSummary> {
        let turno = self.store.turns().start(org, conversa, agente)?;
        let timings = TurnTimings {
            total_ms: u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
            ..TurnTimings::default()
        };
        self.store.turns().finalise(
            org,
            turno.id,
            TurnFinalization {
                state: Some(TurnState::Skipped),
                times_json: Some(timings_json(&timings, "", None)),
                error: Some(motivo.clone()),
                ..TurnFinalization::default()
            },
        )?;
        self.metrics.turns_ignored.increment(1);
        if let Some(agente) = agente {
            let _ = self.store.audit().register(
                Some(org),
                NewAuditEntry {
                    ator_tipo: "system".to_owned(),
                    actor_id: Some("turno".to_owned()),
                    acao: "skip".to_owned(),
                    entity: "turn".to_owned(),
                    entity_id: Some(turno.id.to_string()),
                    detail_json: json!({
                        "motivo": motivo,
                        "conversa": conversa.to_string(),
                        "agente": agente.to_string(),
                    })
                    .to_string(),
                },
            );
        }
        Ok(TurnSummary {
            turno: turno.id,
            estado: TurnState::Skipped,
            texto: String::new(),
            messages: Vec::new(),
            input_tokens: 0,
            output_tokens: 0,
            timings,
            modelo: String::new(),
            motivo_fim: None,
            error: Some(motivo),
            model_failure: false,
            jev: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn fail(
        &self,
        org: OrgId,
        turno: TurnId,
        modelo: &str,
        messages: Vec<MessageId>,
        started_at: Instant,
        queue_ms: u64,
        assembly_ms: u64,
        first_text_ms: Option<u64>,
        error: String,
        class: FailureClass,
        calls: Option<&RecordCalls>,
        observer: &mut dyn TurnObserver,
    ) -> Resultado<TurnSummary> {
        let timings = TurnTimings {
            queue_ms,
            assembly_ms,
            first_text_ms,
            total_ms: u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
            tool_rounds: 0,
        };
        self.store.turns().finalise(
            org,
            turno,
            TurnFinalization {
                state: Some(TurnState::Failed),
                times_json: Some(timings_json(&timings, modelo, None)),
                tools_json: calls.map(|registro| record_turn(&[], registro)),
                error: Some(error.clone()),
                ..TurnFinalization::default()
            },
        )?;
        self.metrics.turns_faulty.increment(1);
        if class == FailureClass::ProviderRejected {
            self.metrics.failures_model_refused.increment(1);
            self.metrics.failures_model_definitive.increment(1);
            tracing::warn!(
                target: "poph_agent::turno",
                org = %org,
                turn_id = %turno,
                model = modelo,
                reason = %log_safe_reason(&error),
                "provedor recusou o pedido: mensagem concluída sem repetir; confira a chave e o modelo do agente"
            );
        }
        observer.evento(TurnEvent::Fim {
            estado: TurnState::Failed.as_text().to_owned(),
            motivo: Some(error.clone()),
        });
        Ok(TurnSummary {
            turno,
            estado: TurnState::Failed,
            texto: String::new(),
            messages,
            input_tokens: 0,
            output_tokens: 0,
            timings,
            modelo: modelo.to_owned(),
            motivo_fim: None,
            error: Some(error),
            model_failure: matches!(class, FailureClass::Modelo | FailureClass::NoSlot),
            jev: None,
        })
    }
}

fn jev_message(mensagem: &StoredMessage) -> Option<JevMessage> {
    match mensagem.direction {
        poph_store::Direction::Inbound => Some(JevMessage::from_customer(mensagem.texto.clone())),
        poph_store::Direction::Outbound => Some(JevMessage::from_agent(mensagem.texto.clone())),
    }
    .filter(|_| mensagem.role != Role::Tool && mensagem.role != Role::System)
}

pub struct TurnProcessor {
    engine: Arc<TurnEngine>,
    fluxo: Arc<std::sync::OnceLock<Fluxo>>,
}

impl TurnProcessor {
    pub fn new(engine: Arc<TurnEngine>) -> Self {
        Self {
            engine,
            fluxo: Arc::new(std::sync::OnceLock::new()),
        }
    }

    pub fn attach_io(&self, fluxo: Fluxo) {
        let _ = self.fluxo.set(fluxo);
    }

    pub fn engine(&self) -> &Arc<TurnEngine> {
        &self.engine
    }
}

impl Processor for TurnProcessor {
    fn process(&self, task: Task) -> BoxFuture {
        let engine = Arc::clone(&self.engine);
        let fluxo = self.fluxo.get().cloned();
        Box::pin(async move {
            let Some(fluxo) = fluxo else {
                return Err(AgentError::Internal(
                    "processador do turno sem fluxo ligado (ligar_fluxo não foi chamado)"
                        .to_owned(),
                ));
            };
            let input = TurnInput::new(task.org, task.conversa, task.mensagem);
            let summary = engine.run(input, &mut NoopObserver).await;
            log_turn_end(&task, &summary);
            let (error, messages, model_failure, turno) = match &summary {
                Ok(summary) => (
                    summary.error.clone(),
                    summary.messages.clone(),
                    summary.model_failure,
                    Some(summary.turno),
                ),
                Err(error) => (Some(error.to_string()), Vec::new(), false, None),
            };
            if let (true, Some(turno)) = (model_failure, turno) {
                handle_model_failure(
                    engine.metrics(),
                    &fluxo,
                    &task,
                    turno,
                    &messages,
                    error.as_deref().unwrap_or("falha do modelo"),
                )?;
            } else {
                complete_inputs(
                    &fluxo,
                    task.org,
                    task.conversa,
                    task.entry,
                    &messages,
                    error.as_deref(),
                )?;
            }
            Ok(())
        })
    }
}

fn log_turn_end(task: &Task, summary: &Resultado<TurnSummary>) {
    match summary {
        Ok(summary) => {
            let estado = summary.estado.as_text();
            match summary.estado {
                TurnState::Completed => tracing::info!(
                    target: "poph_agent::turno",
                    org = %task.org,
                    conversation_id = %task.conversa,
                    turn_id = %summary.turno,
                    state = estado,
                    duration_ms = summary.timings.total_ms,
                    input_tokens = summary.input_tokens,
                    output_tokens = summary.output_tokens,
                    "turno concluído"
                ),
                TurnState::Skipped => tracing::info!(
                    target: "poph_agent::turno",
                    org = %task.org,
                    conversation_id = %task.conversa,
                    turn_id = %summary.turno,
                    state = estado,
                    reason = %log_safe_reason(summary.error.as_deref().unwrap_or("")),
                    "turno sem resposta: a mensagem ficou registrada"
                ),
                _ => tracing::warn!(
                    target: "poph_agent::turno",
                    org = %task.org,
                    conversation_id = %task.conversa,
                    turn_id = %summary.turno,
                    state = estado,
                    duration_ms = summary.timings.total_ms,
                    reason = %log_safe_reason(summary.error.as_deref().unwrap_or("")),
                    "turno falhou"
                ),
            }
        }
        Err(error) => tracing::warn!(
            target: "poph_agent::turno",
            org = %task.org,
            conversation_id = %task.conversa,
            reason = %log_safe_reason(&error.to_string()),
            "turno não pôde começar"
        ),
    }
}

fn log_safe_reason(texto: &str) -> String {
    let sanitized = texto
        .split_whitespace()
        .map(|word| {
            let digits = word.chars().filter(char::is_ascii_digit).count();
            if word.contains('@') || digits >= 7 {
                "[oculto]"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    sanitized.chars().take(160).collect()
}

fn complete_inputs(
    fluxo: &Fluxo,
    org: OrgId,
    conversa: ConversationId,
    own: Option<poph_types::InboxId>,
    messages: &[MessageId],
    error: Option<&str>,
) -> Resultado<()> {
    let pending = fluxo
        .store()
        .inbox()
        .list_opens(org, COMPLETION_BATCH_SIZE)?;
    let mut completed = Vec::new();
    for item in pending {
        let own = Some(item.id) == own;
        let group_tools = messages.iter().any(|mensagem| {
            InputEvent::of_json(&item.event_json)
                .map(|evento| evento.conversa == conversa && evento.mensagem == *mensagem)
                .unwrap_or(false)
        });
        if own || group_tools {
            completed.push(item.id);
        }
    }
    for id in completed {
        fluxo.concluir_entry(org, id, error)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureClass {
    Permanent,
    ProviderRejected,
    NoSlot,
    Modelo,
}

fn llm_error_class(error: &LlmError) -> FailureClass {
    match error {
        LlmError::Configuracao(_) | LlmError::InvalidRequest(_) | LlmError::Cancelled => {
            FailureClass::Permanent
        }
        LlmError::Rejected { .. } => FailureClass::ProviderRejected,
        _ => FailureClass::Modelo,
    }
}

fn provider_slot_error(error: AgentError) -> LlmError {
    match error {
        AgentError::WithoutSlotProvider { provedor, wait_ms } => LlmError::Unavailable {
            tentativas: 0,
            last_error: format!("sem vaga de chamada ao provedor `{provedor}` em {wait_ms} ms"),
        },
        other => LlmError::Configuracao(other.to_string()),
    }
}

fn turn_items(
    fluxo: &Fluxo,
    org: OrgId,
    conversa: ConversationId,
    own: Option<poph_types::InboxId>,
    messages: &[MessageId],
) -> Resultado<Vec<poph_types::InboxId>> {
    let open = fluxo
        .store()
        .inbox()
        .list_opens(org, COMPLETION_BATCH_SIZE)?;
    Ok(open
        .into_iter()
        .filter(|item| {
            Some(item.id) == own
                || messages.iter().any(|mensagem| {
                    InputEvent::of_json(&item.event_json)
                        .map(|evento| evento.conversa == conversa && evento.mensagem == *mensagem)
                        .unwrap_or(false)
                })
        })
        .map(|item| item.id)
        .collect())
}

fn handle_model_failure(
    metrics: &MetricsTurn,
    fluxo: &Fluxo,
    task: &Task,
    turno: TurnId,
    messages: &[MessageId],
    error: &str,
) -> Resultado<()> {
    let org = task.org;
    let motivo = log_safe_reason(error);
    let ids = turn_items(fluxo, org, task.conversa, task.entry, messages)?;
    let already_queued = fluxo.store().outbox().count_turn(org, turno)? > 0;
    let policy = fluxo.options();
    let maximum = policy.tentativas_falha_modelo.max(1);

    if ids.is_empty() {
        tracing::warn!(
            target: "poph_agent::turno",
            org = %org,
            conversation_id = %task.conversa,
            turn_id = %turno,
            reason = %motivo,
            "falha do modelo sem item na fila para repetir"
        );
        return Ok(());
    }
    if already_queued {
        for id in &ids {
            fluxo.concluir_entry(org, *id, Some(motivo.as_str()))?;
        }
        metrics.failures_model_definitive.increment(1);
        tracing::warn!(
            target: "poph_agent::turno",
            org = %org,
            conversation_id = %task.conversa,
            turn_id = %turno,
            reason = %motivo,
            "falha do modelo depois de saída já enfileirada: não repetido para não duplicar"
        );
        return Ok(());
    }

    let tentativas = fluxo
        .store()
        .inbox()
        .attempts(org, &ids)?
        .into_iter()
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    if tentativas >= maximum {
        for id in &ids {
            fluxo.concluir_entry(org, *id, Some(motivo.as_str()))?;
        }
        metrics.failures_model_definitive.increment(1);
        tracing::warn!(
            target: "poph_agent::turno",
            org = %org,
            conversation_id = %task.conversa,
            turn_id = %turno,
            attempts = tentativas,
            reason = %motivo,
            "falha do modelo: tentativas esgotadas; mensagem concluída sem resposta"
        );
        return Ok(());
    }

    let wait = policy.delay_failure_model.wait(tentativas);
    let next_attempt_at = fluxo
        .store()
        .now()
        .saturating_add(i64::try_from(wait.as_millis()).unwrap_or(i64::MAX));
    fluxo.adiar_entries(org, &ids, next_attempt_at, &motivo)?;
    metrics.failures_model_deferred.increment(1);
    tracing::warn!(
        target: "poph_agent::turno",
        org = %org,
        conversation_id = %task.conversa,
        turn_id = %turno,
        attempt = tentativas,
        max_attempts = maximum,
        wait_seconds = wait.as_secs(),
        reason = %motivo,
        "falha do modelo: mensagem adiada para nova tentativa"
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Situacao {
    Process,
    NoNewMessages,
    AlreadyAnswered,
}

fn analyze_group(
    recent_messages: &[StoredMessage],
    gatilho: MessageId,
) -> (Situacao, Vec<StoredMessage>) {
    if recent_messages.is_empty() {
        return (Situacao::NoNewMessages, Vec::new());
    }
    let mut group = Vec::new();
    for mensagem in recent_messages.iter().rev() {
        if mensagem.role == Role::User && mensagem.direction == Direction::Inbound {
            group.push(mensagem.clone());
        } else {
            break;
        }
    }
    if group.is_empty() {
        return (Situacao::AlreadyAnswered, Vec::new());
    }
    group.reverse();
    if group.iter().any(|mensagem| mensagem.id == gatilho) {
        (Situacao::Process, group)
    } else {
        (Situacao::AlreadyAnswered, Vec::new())
    }
}

#[derive(Debug)]
enum ToolOutcome {
    Executed(String),
    Rejected { motivo: &'static str, texto: String },
}

impl ToolOutcome {
    fn rejected(motivo: &'static str, texto: String) -> Self {
        Self::Rejected { motivo, texto }
    }

    fn texto(&self) -> &str {
        match self {
            Self::Executed(texto) => texto,
            Self::Rejected { texto, .. } => texto,
        }
    }

    fn for_record(&self) -> Desfecho {
        match self {
            Self::Executed(_) => Desfecho::Executed,
            Self::Rejected { motivo, .. } => Desfecho::Refused(motivo),
        }
    }

    fn into_text(self) -> String {
        match self {
            Self::Executed(texto) => texto,
            Self::Rejected { texto, .. } => texto,
        }
    }
}

fn tool_error_reason(error: &str) -> &'static str {
    let normalized_error = error.to_lowercase();
    if normalized_error.contains("argumento") || normalized_error.contains("não é um id") {
        REASON_ARGUMENT_INVALID
    } else {
        REASON_ERROR_OF_EXECUTION
    }
}

#[derive(Debug, Default)]
struct ModelReply {
    texto: String,
    usage: Usage,
    motivo: Option<FinishReason>,
    first_text_ms: Option<u64>,
    rounds: u32,
    tools: Vec<String>,
}

#[derive(Debug, Default)]
struct RegeneratedReply {
    texto: String,
    usage: Usage,
    motivo: Option<FinishReason>,
}

fn merge_jev_result(jev: &mut Option<JevResult>, resultado: JevResult) {
    match jev {
        Some(current) => {
            current.decisions.extend(resultado.decisions);
            current.raw.extend(resultado.raw);
            current.call.questions += resultado.call.questions;
            current.call.decided_by_cache += resultado.call.decided_by_cache;
            current.call.ms = current.call.ms.saturating_add(resultado.call.ms);
            current.call.tokens = current.call.tokens.saturating_add(resultado.call.tokens);
            current.call.usage = current.call.usage.plus(resultado.call.usage);
            if current.call.failure.is_none() {
                current.call.failure = resultado.call.failure;
            }
        }
        None => *jev = Some(resultado),
    }
}

fn add_usage(total: Usage, other: Usage) -> Usage {
    Usage {
        input: total.input.saturating_add(other.input),
        saida: total.saida.saturating_add(other.saida),
    }
}

fn queue_delay(now: i64, arrival: i64) -> u64 {
    u64::try_from(now.saturating_sub(arrival)).unwrap_or(0)
}

fn last_customer_input(recent_messages: &[StoredMessage]) -> Option<MessageId> {
    recent_messages
        .iter()
        .rev()
        .find(|mensagem| mensagem.role == Role::User && mensagem.direction == Direction::Inbound)
        .map(|mensagem| mensagem.id)
}

fn conversation_status(status: poph_store::ConversationStatus) -> &'static str {
    match status {
        poph_store::ConversationStatus::Open => "aberta",
        poph_store::ConversationStatus::Transferred => "transferida para humano",
        poph_store::ConversationStatus::Closed => "fechada",
    }
}

fn finish_reason_text(motivo: &FinishReason) -> String {
    match motivo {
        FinishReason::Completed => "concluido".to_owned(),
        FinishReason::ToolCalls => "chamadas_ferramenta".to_owned(),
        FinishReason::TokenLimit => "limite_de_tokens".to_owned(),
        FinishReason::Other(texto) => texto.clone(),
    }
}

const MAX_QUERY_CHARS: usize = 2000;

fn g5_candidate(product: &FoundProduct) -> poph_jev::ProductCandidate {
    let mut parts: Vec<String> = Vec::new();
    if !product.category.trim().is_empty() {
        parts.push(format!("categoria: {}", product.category.trim()));
    }
    if !product.price_text.trim().is_empty() {
        parts.push(format!("preço: {}", product.price_text.trim()));
    }
    if !product.unit.trim().is_empty() {
        parts.push(format!("unidade: {}", product.unit.trim()));
    }
    poph_jev::ProductCandidate {
        id: product.product.to_string(),
        nome: product.nome.clone(),
        detail: parts.join("; "),
    }
}

fn group_query(group: &[StoredMessage]) -> String {
    let mut texto = String::new();
    for mensagem in group {
        if mensagem.role != Role::User {
            continue;
        }
        let spoken_text = mensagem.texto.trim();
        if spoken_text.is_empty() {
            continue;
        }
        if !texto.is_empty() {
            texto.push('\n');
        }
        texto.push_str(spoken_text);
        if texto.chars().count() >= MAX_QUERY_CHARS {
            break;
        }
    }
    texto.chars().take(MAX_QUERY_CHARS).collect()
}

fn retrieved_json(chunks: &[RecoveredPassage], products: &[ProductUsed]) -> String {
    let mut items: Vec<serde_json::Value> = Vec::with_capacity(chunks.len() + products.len());
    for chunk in chunks {
        let mut value = serde_json::to_value(chunk).unwrap_or(serde_json::Value::Null);
        if let Some(object) = value.as_object_mut() {
            object.insert("tipo".to_owned(), json!("trecho"));
        }
        items.push(value);
    }
    for product in products {
        let mut value = serde_json::to_value(product).unwrap_or(serde_json::Value::Null);
        if let Some(object) = value.as_object_mut() {
            object.insert("tipo".to_owned(), json!("produto"));
        }
        items.push(value);
    }
    serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_owned())
}

fn is_knowledge_or_product_tool(nome: &str) -> bool {
    nome == crate::base_knowledge::NAME_TOOL || nome == crate::products::NAME_TOOL
}

fn timings_json(timings: &TurnTimings, modelo: &str, motivo: Option<&FinishReason>) -> String {
    json!({
        "fila_ms": timings.queue_ms,
        "montagem_ms": timings.assembly_ms,
        "primeiro_texto_ms": timings.first_text_ms,
        "total_ms": timings.total_ms,
        "rodadas_ferramenta": timings.tool_rounds,
        "modelo": modelo,
        "motivo_fim": motivo.map(finish_reason_text),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mensagem(role: Role, direction: Direction, id: MessageId) -> StoredMessage {
        StoredMessage {
            id,
            org_id: OrgId::generate(),
            conversation_id: ConversationId::generate(),
            direction,
            role,
            origin: poph_store::Origin::Agente,
            author_name: None,
            texto: String::new(),
            attachments_json: "[]".to_owned(),
            external_id: None,
            turn_id: None,
            created_at: 0,
        }
    }

    #[test]
    fn group_is_the_unanswered_customer_tail() {
        let (a, b, c) = (
            MessageId::generate(),
            MessageId::generate(),
            MessageId::generate(),
        );
        let recent_messages = vec![
            mensagem(Role::User, Direction::Inbound, a),
            mensagem(Role::Assistant, Direction::Outbound, b),
            mensagem(Role::User, Direction::Inbound, c),
        ];
        let (situacao, group) = analyze_group(&recent_messages, c);
        assert_eq!(situacao, Situacao::Process);
        assert_eq!(group.len(), 1);
        assert_eq!(group[0].id, c);
    }

    #[test]
    fn answered_message_does_not_start_turn() {
        let (a, b) = (MessageId::generate(), MessageId::generate());
        let recent_messages = vec![
            mensagem(Role::User, Direction::Inbound, a),
            mensagem(Role::Assistant, Direction::Outbound, b),
        ];
        let (situacao, group) = analyze_group(&recent_messages, a);
        assert_eq!(situacao, Situacao::AlreadyAnswered);
        assert!(group.is_empty());
    }

    #[test]
    fn empty_conversation_has_no_new_messages() {
        let (situacao, group) = analyze_group(&[], MessageId::generate());
        assert_eq!(situacao, Situacao::NoNewMessages);
        assert!(group.is_empty());
    }
}
