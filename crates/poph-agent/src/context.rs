use std::sync::Arc;

use poph_store::{Mensagem, Store};
use poph_types::{AgentId, ConversationId, OrgId};

use crate::error::Resultado;

const CAP_OF_READ: u32 = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub messages: Vec<Mensagem>,
    pub discarded: usize,
    pub tokens_estimate: u64,
    pub budget: u64,
    pub last_truncate: bool,
}

impl Window {
    pub fn folga(&self) -> u64 {
        self.budget.saturating_sub(self.tokens_estimate)
    }
}

pub fn estimate_tokens(texto: &str) -> u64 {
    (texto.chars().count() as u64).div_ceil(4)
}

pub const COST_BY_MESSAGE: u64 = 4;

pub fn build(messages: Vec<Mensagem>, budget: u64, max_messages: u32) -> Window {
    let total = messages.len();
    let limit_quantity = if max_messages == 0 {
        usize::MAX
    } else {
        max_messages as usize
    };
    let mut chosen: Vec<Mensagem> = Vec::new();
    let mut gasto = 0u64;
    let mut last_truncate = false;

    for mensagem in messages.into_iter().rev() {
        if chosen.len() >= limit_quantity {
            break;
        }
        let cost = estimate_tokens(&mensagem.texto) + COST_BY_MESSAGE;
        if gasto + cost <= budget {
            gasto += cost;
            chosen.push(mensagem);
            continue;
        }
        if chosen.is_empty() {
            let available = budget.saturating_sub(COST_BY_MESSAGE);
            let mut cut = mensagem;
            cut.texto = cut_for_tokens(&cut.texto, available);
            gasto = estimate_tokens(&cut.texto) + COST_BY_MESSAGE;
            last_truncate = true;
            chosen.push(cut);
        }
        break;
    }

    chosen.reverse();
    let discarded = total.saturating_sub(chosen.len());
    Window {
        messages: chosen,
        discarded,
        tokens_estimate: gasto,
        budget,
        last_truncate,
    }
}

pub fn database(
    store: &Arc<Store>,
    org: OrgId,
    conversa: ConversationId,
    agente: AgentId,
) -> Resultado<Window> {
    let registro = store.agents().get_for_turn(org, agente)?;
    let limit = registro.max_context_messages.clamp(1, CAP_OF_READ);
    let messages = store.messages().last_set(org, conversa, limit)?;
    Ok(build(
        messages,
        u64::from(registro.max_context_tokens),
        registro.max_context_messages,
    ))
}

fn cut_for_tokens(texto: &str, tokens: u64) -> String {
    let maximum = usize::try_from(tokens.saturating_mul(4)).unwrap_or(usize::MAX);
    if texto.chars().count() <= maximum {
        return texto.to_owned();
    }
    let mut cut: String = texto.chars().take(maximum.saturating_sub(1)).collect();
    cut.push('…');
    cut
}
