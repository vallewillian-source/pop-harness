use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::compilador::{Limites, LoadFailure, load as load_revision};
use crate::ir::Ir;

pub const DEFAULT_CAP_MB: usize = 16;

pub const ENTRY_FIXED_COST: usize = 256;

pub const WEIGHT_FACTOR: usize = 2;

pub const MAX_BATCH_WAIT: Duration = Duration::from_secs(30);

pub fn sha256_of_source(fonte: &str) -> String {
    let digest = Sha256::digest(fonte.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RevisionKey {
    fluxo_id: String,
    numero: u32,
    hash: String,
}

impl RevisionKey {
    pub fn new(fluxo_id: impl Into<String>, numero: u32, fonte: &str) -> Self {
        Self {
            fluxo_id: fluxo_id.into(),
            numero,
            hash: sha256_of_source(fonte),
        }
    }

    pub fn with_hash(fluxo_id: impl Into<String>, numero: u32, hash: impl Into<String>) -> Self {
        Self {
            fluxo_id: fluxo_id.into(),
            numero,
            hash: hash.into(),
        }
    }

    pub fn fluxo_id(&self) -> &str {
        &self.fluxo_id
    }

    pub fn numero(&self) -> u32 {
        self.numero
    }

    pub fn hash(&self) -> &str {
        &self.hash
    }
}

pub fn entry_weight(ir: &Ir) -> usize {
    ir.to_json()
        .len()
        .saturating_mul(WEIGHT_FACTOR)
        .saturating_add(ENTRY_FIXED_COST)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Metrics {
    pub hits: u64,
    pub misses: u64,
    pub loads: u64,
    pub waits: u64,
    pub evictions: u64,
    pub in_use_spared: u64,
    pub not_cached: u64,
    pub entries: usize,
    pub in_use: usize,
    pub bytes: usize,
}

struct InFlight {
    resultado: Mutex<Option<Result<Arc<Ir>, LoadFailure>>>,
    signal: Condvar,
}

impl InFlight {
    fn new() -> Self {
        Self {
            resultado: Mutex::new(None),
            signal: Condvar::new(),
        }
    }

    fn publicar(&self, resultado: Result<Arc<Ir>, LoadFailure>) {
        let mut guard = self.lock();
        *guard = Some(resultado);
        drop(guard);
        self.signal.notify_all();
    }

    fn wait_result(&self) -> Option<Result<Arc<Ir>, LoadFailure>> {
        let start = Instant::now();
        let mut guard = self.lock();
        loop {
            if let Some(resultado) = guard.as_ref() {
                return Some(resultado.clone());
            }
            let restante = MAX_BATCH_WAIT.checked_sub(start.elapsed())?;
            guard = match self.signal.wait_timeout(guard, restante) {
                Ok((new, _)) => new,
                Err(envenenado) => envenenado.into_inner().0,
            };
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<Result<Arc<Ir>, LoadFailure>>> {
        self.resultado
            .lock()
            .unwrap_or_else(|envenenado| envenenado.into_inner())
    }
}

struct Entry {
    ir: Arc<Ir>,
    bytes: usize,
    used_at: u64,
}

#[derive(Default)]
struct Counters {
    hits: u64,
    misses: u64,
    loads: u64,
    waits: u64,
    evictions: u64,
    in_use_spared: u64,
    not_cached: u64,
}

struct Inner {
    entries: HashMap<RevisionKey, Entry>,
    in_flight: HashMap<RevisionKey, Arc<InFlight>>,
    bytes: usize,
    clock: u64,
    counters: Counters,
}

impl Inner {
    fn touch(&mut self, chave: &RevisionKey) -> Option<Arc<Ir>> {
        self.clock += 1;
        let stamp = self.clock;
        let entry = self.entries.get_mut(chave)?;
        entry.used_at = stamp;
        Some(Arc::clone(&entry.ir))
    }

    fn store(&mut self, chave: &RevisionKey, ir: Arc<Ir>, cap: usize) {
        if self.entries.contains_key(chave) {
            return;
        }
        let bytes = entry_weight(&ir);
        if bytes > cap || !self.has_room(bytes, cap) {
            self.counters.not_cached += 1;
            return;
        }
        self.open_space(bytes, cap);
        self.clock += 1;
        let used_at = self.clock;
        self.entries
            .insert(chave.clone(), Entry { ir, bytes, used_at });
        self.bytes += bytes;
    }

    fn has_room(&self, needed: usize, cap: usize) -> bool {
        let free = cap.saturating_sub(self.bytes);
        if free >= needed {
            return true;
        }
        let expulsavel: usize = self
            .entries
            .values()
            .filter(|entry| Arc::strong_count(&entry.ir) == 1)
            .map(|entry| entry.bytes)
            .sum();
        free + expulsavel >= needed
    }

    fn open_space(&mut self, needed: usize, cap: usize) {
        while self.bytes + needed > cap {
            let Some(victim) = self.victim() else {
                return;
            };
            if let Some(entry) = self.entries.remove(&victim) {
                self.bytes = self.bytes.saturating_sub(entry.bytes);
                self.counters.evictions += 1;
            } else {
                return;
            }
        }
    }

    fn victim(&mut self) -> Option<RevisionKey> {
        let mut chosen: Option<(RevisionKey, u64)> = None;
        let mut spared = 0u64;
        for (chave, entry) in &self.entries {
            if Arc::strong_count(&entry.ir) > 1 {
                spared += 1;
                continue;
            }
            let melhor = chosen
                .as_ref()
                .is_none_or(|(_, used)| entry.used_at < *used);
            if melhor {
                chosen = Some((chave.clone(), entry.used_at));
            }
        }
        self.counters.in_use_spared += spared;
        chosen.map(|(chave, _)| chave)
    }

    fn metrics(&self) -> Metrics {
        Metrics {
            hits: self.counters.hits,
            misses: self.counters.misses,
            loads: self.counters.loads,
            waits: self.counters.waits,
            evictions: self.counters.evictions,
            in_use_spared: self.counters.in_use_spared,
            not_cached: self.counters.not_cached,
            entries: self.entries.len(),
            in_use: self
                .entries
                .values()
                .filter(|entry| Arc::strong_count(&entry.ir) > 1)
                .count(),
            bytes: self.bytes,
        }
    }
}

#[derive(Clone)]
pub struct RevisionCache {
    inner: Arc<Mutex<Inner>>,
    byte_cap: usize,
}

impl RevisionCache {
    pub fn new(cap_mb: usize) -> Self {
        Self::with_byte_cap(cap_mb.saturating_mul(1024 * 1024))
    }

    pub fn with_byte_cap(byte_cap: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                entries: HashMap::new(),
                in_flight: HashMap::new(),
                bytes: 0,
                clock: 0,
                counters: Counters::default(),
            })),
            byte_cap,
        }
    }

    pub fn byte_cap(&self) -> usize {
        self.byte_cap
    }

    pub fn metrics(&self) -> Metrics {
        self.lock().metrics()
    }

    pub fn contem(&self, chave: &RevisionKey) -> bool {
        self.lock().entries.contains_key(chave)
    }

    pub fn get(&self, chave: &RevisionKey) -> Option<Arc<Ir>> {
        self.lock().touch(chave)
    }

    pub fn clear(&self) -> usize {
        let mut inner = self.lock();
        let outputs = inner.entries.len();
        inner.entries.clear();
        inner.bytes = 0;
        outputs
    }

    pub fn load(
        &self,
        chave: &RevisionKey,
        stored_ir: &Ir,
        fonte: &str,
        limites: &Limites,
    ) -> Result<Arc<Ir>, LoadFailure> {
        let wait = {
            let mut inner = self.lock();
            if let Some(ir) = inner.touch(chave) {
                inner.counters.hits += 1;
                return Ok(ir);
            }
            inner.counters.misses += 1;
            let in_flight = inner.in_flight.get(chave).map(Arc::clone);
            match in_flight {
                Some(in_flight) => {
                    inner.counters.waits += 1;
                    Some(in_flight)
                }
                None => {
                    inner
                        .in_flight
                        .insert(chave.clone(), Arc::new(InFlight::new()));
                    None
                }
            }
        };

        if let Some(in_flight) = wait {
            if let Some(resultado) = in_flight.wait_result() {
                return resultado;
            }
            let resultado = self.load_alone(stored_ir, fonte, limites);
            self.concluir(chave, &resultado);
            return resultado;
        }

        let resultado = self.load_alone(stored_ir, fonte, limites);
        self.concluir(chave, &resultado);
        resultado
    }

    fn load_alone(
        &self,
        stored_ir: &Ir,
        fonte: &str,
        limites: &Limites,
    ) -> Result<Arc<Ir>, LoadFailure> {
        {
            let mut inner = self.lock();
            inner.counters.loads += 1;
        }
        load_revision(stored_ir, fonte, limites).map(Arc::new)
    }

    fn concluir(&self, chave: &RevisionKey, resultado: &Result<Arc<Ir>, LoadFailure>) {
        let in_flight = {
            let mut inner = self.lock();
            if let Ok(ir) = resultado {
                inner.store(chave, Arc::clone(ir), self.byte_cap);
            }
            inner.in_flight.remove(chave)
        };
        if let Some(in_flight) = in_flight {
            in_flight.publicar(resultado.clone());
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|envenenado| envenenado.into_inner())
    }
}

impl std::fmt::Debug for RevisionCache {
    fn fmt(&self, saida: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let metrics = self.metrics();
        saida
            .debug_struct("CacheRevisoes")
            .field("teto_bytes", &self.byte_cap)
            .field("entradas", &metrics.entries)
            .field("bytes", &metrics.bytes)
            .field("acertos", &metrics.hits)
            .field("erros", &metrics.misses)
            .field("expulsoes", &metrics.evictions)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{RevisionKey, sha256_of_source};

    #[test]
    fn sha256_of_source_is_lowercase_hex() {
        assert_eq!(
            sha256_of_source("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn new_key_loads_the_hash_of_the_text() {
        let chave = RevisionKey::new("agendar_visita", 3, "abc");
        assert_eq!(chave.fluxo_id(), "agendar_visita");
        assert_eq!(chave.numero(), 3);
        assert_eq!(chave.hash(), sha256_of_source("abc"));
        assert_ne!(chave, RevisionKey::new("agendar_visita", 3, "abd"));
    }

    #[test]
    fn key_with_hash_uses_the_value_stored_on_publish() {
        let chave = RevisionKey::with_hash("triagem", 1, "abc123");
        assert_eq!(chave.hash(), "abc123");
    }
}
