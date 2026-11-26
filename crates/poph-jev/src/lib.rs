//! Jev (TypeSafe System One) no POP Harness — HR-28.
//!
//! O Jev decide o que é escolha entre opções conhecidas (transferir, esperar, precisa de base,
//! departamento) em ~255–315 ms e custo de entrada, sem gastar o modelo de texto. Aqui ficam:
//!
//! - [`cliente`]: `POST /v1/systemone` com uma retentativa, prazo por tentativa e a chave só no
//!   cabeçalho — mais o [`cliente::JevFalso`] que os testes usam;
//! - [`estado`]: o que o Jev enxerga (conversa recente, política da empresa, capacidades) e o hash
//!   que identifica a decisão no cache;
//! - [`gate`]: o molde `Gate` (pergunta, aplicabilidade, limiares, interpretação, fallback);
//! - [`gates`]: G1 a G9 (G1–G4 obrigatórios da V1, G5 na HR-33, G6 na HR-34, G7 na HR-35, G8 na
//!   HR-36 e G9 na FX-22, que escolhe o fluxo a iniciar);
//! - [`motor`]: [`motor::MotorJev`], que manda **um lote só** com as perguntas de todos os gates
//!   ligados, distribui as respostas, guarda no cache e nunca derruba o turno;
//! - [`handoff`]: a segunda pergunta do G1 (tipo de risco) e o texto de orientação ao cliente
//!   antes da transferência (HR-29);
//! - [`flags`]: flags global e por agente (`agents.jev_json`);
//! - [`eval`]: conjunto rotulado e medição (`poph jev eval`).
//!
//! O que o gate faz é **decidir**. Transferir, esperar, pular a busca e escolher o departamento são
//! das tarefas HR-29 a HR-36, que medem antes de ligar.

pub mod cache;
pub mod cliente;
pub mod eval;
pub mod flags;
pub mod gate;
pub mod gates;
pub mod handoff;
pub mod motor;
pub mod state;
pub mod types;

pub use cache::{CacheOptions, DecisionCache};
pub use cliente::{ClienteJev, ConfigJev, JevFalso, Pedido, TransporteJev};
pub use eval::{
    CasoRotulado, DetalheCaso, ErroConjunto, RelatorioEval, avaliar_conjunto, ler_conjunto,
};
pub use flags::{
    G7Action, GateOverrides, farewell_from_agent, from_agent as ajuste_do_agente,
    g7_action_from_agent, g7_response_from_agent, handoff_situations_from_agent,
    safe_response_from_agent,
};
pub use gate::{DEFAULT_CONFIDENCE, DEFAULT_MARGIN, Decisao, DecisionOrigin, Gate, Thresholds};
pub use gates::{
    G1, G1_CONTINUE, G1_TRANSFER, G2, G2_PROCESS, G2_WAIT, G3, G3_ALL, G3_BASE, G3_BOTH,
    G3_NOTHING, G3_PRODUCT, G4, G5, G5_NONE, G6, G6_APPROVED, G6_REVIEW, G6_REVIEW_LONG,
    G6_REVIEW_PRICE, G6_REVIEW_SCOPE, G6_REVIEW_TONE, G6_REVIEW_TRANSFER, G7, G7_ABUSE,
    G7_DEFAULT_REPLY, G7_INJECTION, G7_NORMAL, G8, G8_CONTINUES, G8_DEFAULT_FAREWELL, G8_ENDED, G9,
    G9_NONE, flow_policy, g6_guidance, g6_needs_review, g7_requests_action, match_candidate,
};
pub use handoff::{
    Orientacao, PRAZO_PADRAO, RISCO_ANIMAL, RISCO_INGESTAO, RISCO_NENHUM, RISCO_OUTRO,
    RISCO_PERGUNTA_ID, ResultadoOrientacao, TIPOS_DE_RISCO, pergunta_risco,
};
pub use motor::{
    AvaliadorGates, MotorJev, OpcoesJev, RelatoChamada, ResultadoJev, TEMPO_MAXIMO_PADRAO,
};
pub use state::{
    JevFlow, JevMessage, JevRole, JevState, MAX_CANDIDATE_RESPONSE_CHARS, MAX_FILE_TEXT_CHARS,
    MAX_MESSAGE_CHARS, MAX_MESSAGES, MAX_PRODUCT_CANDIDATES, MAX_RELEASED_FLOWS, MAX_TRIGGER_CHARS,
    ProductCandidate,
};
pub use types::{
    BatchResponse, IdGate, JevError, JevResponse, Question, QuestionType, UsageTokens,
};
