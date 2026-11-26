pub mod actors;
pub mod base_knowledge;
pub mod cart;
pub mod compositor;
pub mod config_hot;
pub mod context;
pub mod entry_output;
pub mod error;
pub mod flows_transition;
pub mod library;
pub mod limites;
pub mod metrics;
pub mod products;
pub mod providers;
pub mod queue_integration;
pub mod summary;
pub mod turno;

pub use actors::{ActorOptions, Actors, BoxFuture, Processor, Task};
pub use base_knowledge::{
    KnowledgeBase, KnowledgeSearcher, QueryKnowledge, RecoveredPassage, SearchFuture,
    SearchParameters, format_passages,
};
pub use compositor::compose_instructions;
pub use config_hot::{ConfigHot, HotAgent, HotConfigOptions};
pub use context::{Window, build as build_window, estimate_tokens};
pub use entry_output::{
    Backoff, DispatchReport, FlowOptions, Fluxo, InputEvent, Loop, OutputTransport,
    ReceivedMessage, ResumeReport, SendFailure, SendFuture, SendReport, Statistics,
};
pub use error::{AgentError, Resultado};
pub use library::{NAME_TOOL as NAME_SEND_FILE, SendFile, attach_library};
pub use limites::{LimitOptions, Limiter, ProviderSlot, Slot};
pub use metrics::MetricsG9;
pub use metrics::{MetricsFlow, MetricsMemory, MetricsTurn};
pub use products::{
    FoundProduct, ProductCatalog, ProductParameters, ProductSearcher, SearchProduct,
    format_products, price_text, record_with,
};
pub use providers::{Providers, ResolvedProvider};
pub use queue_integration::{
    ConversationChannel, EntryReport, InboxChannel, ResolverConversation, SendByChannel,
    chave_idempotencia, command_item,
};
pub use summary::{Summary, SummaryOptions};
pub use turno::{
    InternalTool, NoTools, NoopObserver, Pause, PauseReal, ToolContext, ToolRegistry,
    TransportConsole, TurnEngine, TurnEvent, TurnInput, TurnObserver, TurnOptions, TurnProcessor,
    TurnSummary, TurnTimings,
};
