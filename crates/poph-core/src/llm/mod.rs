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

pub use erro::ErroLlm;
pub use fake::{ProvedorFake, Roteiro};
pub use limites::{LimitadorLlm, PermissaoLlm};
pub use openai::{
    OpenAiCompatibleClient, OpenAiCompatibleConfig, PROVEDOR_ANTIGRAVITY,
    PROVEDOR_OPENAI_COMPATIVEL,
};
pub use provedor::{FluxoLlm, LlmProvider, MIMES_IMAGEM, validar_pedido};
pub use tipos::{
    Cancelamento, ChamadaFerramenta, EventoLlm, Ferramenta, Mensagem, MotivoFim, Parte, Pedido,
    PrecoModelo, Uso, custo_micros,
};
