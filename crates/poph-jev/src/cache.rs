//! Cache de decisão do Jev, por hash do estado.
//!
//! Por que existe: a mesma pergunta sobre o mesmo estado não muda de resposta, e cada chamada custa
//! dinheiro (~255–315 ms no POP Atende). Um turno reexecutado, um retry do canal ou duas conversas
//! com a mesma fala reaproveitam a decisão.
//!
//! Regras:
//! - chave = gate + hash do estado; a validade é curta (padrão 5 min) porque o estado muda a cada
//!   mensagem;
//! - teto de entradas: ao encher, sai a mais antiga (FIFO) — o cache não cresce sem limite;
//! - guarda a **resposta do Jev**, não a interpretação: mudar limiar ou critério no código passa a
//!   valer na hora, com a mesma resposta gravada.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::tipos::Resposta;

/// Ajustes do cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpcoesCache {
    /// Quanto tempo uma resposta vale.
    pub validade: Duration,
    /// Quantas respostas cabem.
    pub capacidade: usize,
}

impl Default for OpcoesCache {
    fn default() -> Self {
        Self {
            // 5 min: cobre retry do canal e turno reexecutado sem guardar decisão velha.
            validade: Duration::from_secs(300),
            // 4096 respostas: ~40 gates por org ativa, com folga para várias orgs.
            capacidade: 4096,
        }
    }
}

#[derive(Debug)]
struct Entrada {
    resposta: Resposta,
    criada: Instant,
}

/// Cache em memória, seguro entre tarefas.
#[derive(Debug)]
pub struct CacheDecisoes {
    opcoes: OpcoesCache,
    entradas: Mutex<Estado>,
}

#[derive(Debug, Default)]
struct Estado {
    mapa: HashMap<u64, Entrada>,
    ordem: VecDeque<u64>,
}

impl CacheDecisoes {
    /// Cache com os ajustes dados.
    pub fn novo(opcoes: OpcoesCache) -> Self {
        Self {
            opcoes,
            entradas: Mutex::new(Estado::default()),
        }
    }

    /// Resposta guardada para a chave, se ainda valer.
    pub fn obter(&self, chave: u64) -> Option<Resposta> {
        let mut estado = self.travar();
        let entrada = estado.mapa.get(&chave)?;
        if entrada.criada.elapsed() > self.opcoes.validade {
            estado.mapa.remove(&chave);
            estado.ordem.retain(|item| *item != chave);
            return None;
        }
        Some(entrada.resposta.clone())
    }

    /// Guarda a resposta da chave, descartando o que venceu ou o mais antigo quando o teto estoura.
    pub fn guardar(&self, chave: u64, resposta: Resposta) {
        let mut estado = self.travar();
        self.descartar_vencidas(&mut estado);
        while estado.mapa.len() >= self.opcoes.capacidade.max(1) {
            match estado.ordem.pop_front() {
                Some(antiga) => {
                    estado.mapa.remove(&antiga);
                }
                None => break,
            }
        }
        if estado
            .mapa
            .insert(
                chave,
                Entrada {
                    resposta,
                    criada: Instant::now(),
                },
            )
            .is_none()
        {
            estado.ordem.push_back(chave);
        }
    }

    /// Quantas entradas válidas existem agora (teste e métrica).
    pub fn tamanho(&self) -> usize {
        self.travar().mapa.len()
    }

    /// Esvazia o cache.
    pub fn limpar(&self) {
        let mut estado = self.travar();
        estado.mapa.clear();
        estado.ordem.clear();
    }

    fn descartar_vencidas(&self, estado: &mut Estado) {
        let validade = self.opcoes.validade;
        estado
            .mapa
            .retain(|_, entrada| entrada.criada.elapsed() <= validade);
        let vivos: Vec<u64> = estado.mapa.keys().copied().collect();
        estado.ordem.retain(|chave| vivos.contains(chave));
    }

    fn travar(&self) -> std::sync::MutexGuard<'_, Estado> {
        match self.entradas.lock() {
            Ok(guarda) => guarda,
            Err(veneno) => veneno.into_inner(),
        }
    }
}

/// Chave do cache: o gate e o estado que ele julgou.
pub fn chave(gate: crate::tipos::IdGate, estado_hash: u64) -> u64 {
    // Mistura simples (xor + rotação) para o mesmo estado em gates diferentes não colidir.
    let base = estado_hash.rotate_left(7) ^ ((gate as u64) << 59);
    base.wrapping_mul(0x9e37_79b9_7f4a_7c15)
}
