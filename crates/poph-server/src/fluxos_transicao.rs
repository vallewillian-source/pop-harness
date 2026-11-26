//! Ponte entre o motor de fluxos (`poph-fluxos`, puro) e a gravação atômica (`poph-store`, FX-20).
//!
//! A tradução mora em `poph-agent` ([`poph_agent::fluxos_transicao`]) desde a FX-23, porque **o
//! turno também a usa** para abrir a execução: um formato só, sem duplicata entre os dois crates.
//! Este módulo existe para os chamadores da API continuarem importando daqui.

pub use poph_agent::flows_transition::{
    effect_of_transition as efeito_da_transicao, state_for_database as estado_para_banco,
    transition_engine as transicao_do_motor,
};
