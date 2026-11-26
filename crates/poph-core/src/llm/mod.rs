//! Provedor de LLM do POP Harness (HR-10).
//!
//! - [`LlmProvider`]: contrato do provedor (stream de eventos, cancelamento ao soltar o stream).
//! - [`OpenAiCompatibleClient`]: cliente OpenAI-compatível (SSE, ferramentas, imagens).
//! - [`ProvedorFake`]: provedor determinístico para testes e CI.
//! - [`LimitadorLlm`]: vagas de simultaneidade por org e globais.
//!
//! Armadilha do gateway: requisição que termina em turno do modelo recebe HTTP 400. Por isso
//! [`validar_pedido`] recusa esse formato antes do envio quando `llm.recusa_turno_modelo` está ligado.

pub mod base64;
pub mod erro;
pub mod fake;
pub mod limites;
pub mod openai;
pub mod provedor;
pub mod sse;
pub mod tipos;
pub mod tls;

pub use erro::LlmError;
pub use fake::{FakeProvider, Script};
pub use limites::{LlmLimiter, LlmPermit};
pub use openai::{
    ANTIGRAVITY_PROVIDER, OPENAI_COMPATIBLE_PROVIDER, OpenAiCompatibleClient,
    OpenAiCompatibleConfig,
};
pub use provedor::{IMAGE_MIMES, LlmProvider, LlmStream, validate_request};
pub use tipos::{
    Cancellation, FinishReason, LlmEvent, Message, ModelPrice, Part, Request, Tool, ToolCall,
    Usage, cost_micros,
};
