//! Janela de contexto recente (PRD §6.2).
//!
//! Monta a lista de mensagens que vai ao LLM respeitando dois tetos do agente:
//! `max_context_messages` (quantidade) e `max_context_tokens` (orçamento). O excedente **não** é
//! resumido aqui: o resumo rolante é a HR-14. Esta camada só decide o que cabe, sempre preservando
//! a ordem de chegada e as mensagens mais recentes.
//!
//! A contagem é uma estimativa (≈ 4 caracteres por token, mais um custo fixo por mensagem), porque
//! o tokenizador real é do provedor. O PRD já prevê isso: a HR-13 desconta do orçamento o que o
//! provedor informar de uso e o resumo da HR-14.

use std::sync::Arc;

use poph_store::{Mensagem, Store};
use poph_types::{AgentId, ConversationId, OrgId};

use crate::erro::Resultado;

/// Teto absoluto de mensagens lidas do banco, para a consulta não varrer histórico enorme.
const TETO_DE_LEITURA: u32 = 200;

/// Resultado da montagem da janela.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Janela {
    /// Mensagens que vão ao modelo, em ordem de chegada.
    pub mensagens: Vec<Mensagem>,
    /// Quantas mensagens antigas ficaram de fora.
    pub descartadas: usize,
    /// Tokens estimados da janela montada.
    pub tokens_estimados: u64,
    /// Orçamento que foi usado.
    pub orcamento: u64,
    /// `true` quando a mensagem mais recente precisou ser cortada para caber.
    pub ultima_truncada: bool,
}

impl Janela {
    /// Sobrou orçamento? (A HR-14 usa para decidir se cabe o resumo.)
    pub fn folga(&self) -> u64 {
        self.orcamento.saturating_sub(self.tokens_estimados)
    }
}

/// Custo estimado de um texto, em tokens.
///
/// 4 caracteres por token é a média prática de modelos com tokenizador parecido com o do Gemini;
/// o custo fixo por mensagem cobre os delimitadores de papel que o provedor acrescenta.
pub fn estimar_tokens(texto: &str) -> u64 {
    (texto.chars().count() as u64).div_ceil(4)
}

/// Custo fixo estimado de uma mensagem (papel + delimitadores).
pub const CUSTO_POR_MENSAGEM: u64 = 4;

/// Monta a janela a partir de mensagens em ordem de chegada.
///
/// - Percorre do fim para o começo, incluindo o que cabe.
/// - A mensagem mais recente **sempre** entra; se ela sozinha passar do orçamento, o texto é
///   cortado (e `ultima_truncada` fica `true`), porque um turno sem a fala do cliente não existe.
/// - `max_mensagens = 0` significa "sem teto de quantidade".
pub fn montar(mensagens: Vec<Mensagem>, orcamento: u64, max_mensagens: u32) -> Janela {
    let total = mensagens.len();
    let limite_quantidade = if max_mensagens == 0 {
        usize::MAX
    } else {
        max_mensagens as usize
    };
    let mut escolhidas: Vec<Mensagem> = Vec::new();
    let mut gasto = 0u64;
    let mut ultima_truncada = false;

    for mensagem in mensagens.into_iter().rev() {
        if escolhidas.len() >= limite_quantidade {
            break;
        }
        let custo = estimar_tokens(&mensagem.texto) + CUSTO_POR_MENSAGEM;
        if gasto + custo <= orcamento {
            gasto += custo;
            escolhidas.push(mensagem);
            continue;
        }
        if escolhidas.is_empty() {
            // A mensagem mais recente não cabe: corta o texto para o orçamento disponível.
            let disponivel = orcamento.saturating_sub(CUSTO_POR_MENSAGEM);
            let mut cortada = mensagem;
            cortada.texto = cortar_para_tokens(&cortada.texto, disponivel);
            gasto = estimar_tokens(&cortada.texto) + CUSTO_POR_MENSAGEM;
            ultima_truncada = true;
            escolhidas.push(cortada);
        }
        // Qualquer outra que não caiba é descartada (vira resumo na HR-14).
        break;
    }

    escolhidas.reverse();
    let descartadas = total.saturating_sub(escolhidas.len());
    Janela {
        mensagens: escolhidas,
        descartadas,
        tokens_estimados: gasto,
        orcamento,
        ultima_truncada,
    }
}

/// Lê as últimas mensagens do banco e monta a janela com os tetos do agente.
pub fn do_banco(
    store: &Arc<Store>,
    org: OrgId,
    conversa: ConversationId,
    agente: AgentId,
) -> Resultado<Janela> {
    let registro = store.agents().obter_para_turno(org, agente)?;
    let limite = registro.max_context_messages.clamp(1, TETO_DE_LEITURA);
    let mensagens = store.messages().ultimas(org, conversa, limite)?;
    Ok(montar(
        mensagens,
        u64::from(registro.max_context_tokens),
        registro.max_context_messages,
    ))
}

/// Corta o texto para caber em `tokens` estimados, marcando o corte com `…`.
fn cortar_para_tokens(texto: &str, tokens: u64) -> String {
    let maximo = usize::try_from(tokens.saturating_mul(4)).unwrap_or(usize::MAX);
    if texto.chars().count() <= maximo {
        return texto.to_owned();
    }
    let mut cortado: String = texto.chars().take(maximo.saturating_sub(1)).collect();
    cortado.push('…');
    cortado
}
