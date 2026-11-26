use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::types::JevResponse;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheOptions {
    pub ttl: Duration,
    pub capacity: usize,
}

impl Default for CacheOptions {
    fn default() -> Self {
        Self {
            ttl: Duration::from_secs(300),
            capacity: 4096,
        }
    }
}

#[derive(Debug)]
struct Entry {
    response: JevResponse,
    created: Instant,
}

#[derive(Debug)]
pub struct DecisionCache {
    options: CacheOptions,
    entries: Mutex<Estado>,
}

#[derive(Debug, Default)]
struct Estado {
    map: HashMap<u64, Entry>,
    ordem: VecDeque<u64>,
}

impl DecisionCache {
    pub fn new(options: CacheOptions) -> Self {
        Self {
            options,
            entries: Mutex::new(Estado::default()),
        }
    }

    pub fn get(&self, chave: u64) -> Option<JevResponse> {
        let mut estado = self.lock();
        let entry = estado.map.get(&chave)?;
        if entry.created.elapsed() > self.options.ttl {
            estado.map.remove(&chave);
            estado.ordem.retain(|item| *item != chave);
            return None;
        }
        Some(entry.response.clone())
    }

    pub fn insert(&self, chave: u64, response: JevResponse) {
        let mut estado = self.lock();
        self.remove_expired(&mut estado);
        while estado.map.len() >= self.options.capacity.max(1) {
            match estado.ordem.pop_front() {
                Some(oldest) => {
                    estado.map.remove(&oldest);
                }
                None => break,
            }
        }
        if estado
            .map
            .insert(
                chave,
                Entry {
                    response,
                    created: Instant::now(),
                },
            )
            .is_none()
        {
            estado.ordem.push_back(chave);
        }
    }

    pub fn len(&self) -> usize {
        self.lock().map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&self) {
        let mut estado = self.lock();
        estado.map.clear();
        estado.ordem.clear();
    }

    fn remove_expired(&self, estado: &mut Estado) {
        let ttl = self.options.ttl;
        estado.map.retain(|_, entry| entry.created.elapsed() <= ttl);
        let live_keys: Vec<u64> = estado.map.keys().copied().collect();
        estado.ordem.retain(|chave| live_keys.contains(chave));
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Estado> {
        match self.entries.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        }
    }
}

pub fn chave(gate: crate::types::IdGate, state_hash: u64) -> u64 {
    let base = state_hash.rotate_left(7) ^ ((gate as u64) << 59);
    base.wrapping_mul(0x9e37_79b9_7f4a_7c15)
}
