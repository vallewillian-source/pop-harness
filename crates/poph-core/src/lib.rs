//! Núcleo do POP Harness: armazenamento, RAG, LLM, Jev e motor do turno.
//!
//! Esqueleto (HR-04, parte A): só a fronteira do crate. As implementações entram nas
//! tarefas HR-06 em diante. Reexporta os tipos de domínio para quem depende só do núcleo.
//!
//! Desde a HR-10 há o módulo [`llm`] (provedor de LLM). O compositor das instruções saiu para o
//! `poph-agent` na HR-11 (D-08); o `poph-core` fica só com o LLM até o turno migrar na HR-13.

pub mod llm;

pub use poph_types as types;

/// Versão do pacote, usada pelo `poph --version` e pelo `/metrics` quando existirem.
pub const VERSAO: &str = env!("CARGO_PKG_VERSION");
