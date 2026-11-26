use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use moka::sync::Cache;
use poph_store::{Agente, Instructions, Store};
use poph_types::{AgentId, OrgId, StructuredInstructions};

use crate::error::{AgentError, Resultado};
use crate::metrics::MetricsMemory;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Chave {
    org: OrgId,
    agente: AgentId,
}

#[derive(Debug, Clone)]
pub struct HotConfigOptions {
    pub validity: Duration,
    pub cap_bytes: u64,
}

impl Default for HotConfigOptions {
    fn default() -> Self {
        Self {
            validity: Duration::from_secs(60),
            cap_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HotAgent {
    pub agente: Agente,
    pub instructions: Option<Instructions>,
    pub structured: Option<StructuredInstructions>,
    pub markdown: String,
    pub tools: Vec<String>,
    pub top_k: u32,
    pub cutoff: f64,
    pub version_instructions: u64,
    pub bytes: u64,
}

impl HotAgent {
    pub fn versao(&self) -> u64 {
        self.agente.version
    }

    pub fn version_composta(&self) -> (u64, u64) {
        (self.agente.version, self.version_instructions)
    }

    pub fn can_open_turn(&self) -> bool {
        self.agente.can_open_turn()
    }

    pub fn tool_enabled(&self, nome: &str) -> bool {
        self.tools.iter().any(|enabled| enabled == nome)
    }
}

pub struct ConfigHot {
    store: Arc<Store>,
    cache: Cache<Chave, Arc<HotAgent>>,
    metrics: MetricsMemory,
}

impl ConfigHot {
    pub fn new(store: Arc<Store>, options: HotConfigOptions, metrics: MetricsMemory) -> Self {
        let cap = options.cap_bytes.max(1024);
        let cache = Cache::builder()
            .max_capacity(cap)
            .weigher(|_key: &Chave, value: &Arc<HotAgent>| {
                u32::try_from(value.bytes.min(u64::from(u32::MAX))).unwrap_or(u32::MAX)
            })
            .time_to_live(options.validity)
            .build();
        Self {
            store,
            cache,
            metrics,
        }
    }

    pub fn load(&self, org: OrgId, agente: AgentId) -> Resultado<Arc<HotAgent>> {
        let chave = Chave { org, agente };
        if let Some(hot) = self.cache.get(&chave) {
            self.metrics.config_hits.increment(1);
            self.update_size();
            return Ok(hot);
        }
        self.reload(chave)
    }

    pub fn load_verify(&self, org: OrgId, agente: AgentId) -> Resultado<Arc<HotAgent>> {
        let chave = Chave { org, agente };
        let versao = self.store.agents().version(org, agente)?;
        let version_instructions = self.store.instructions().version(org, agente)?.unwrap_or(0);
        if let Some(hot) = self.cache.get(&chave)
            && hot.version_composta() == (versao, version_instructions)
        {
            self.metrics.config_hits.increment(1);
            self.update_size();
            return Ok(hot);
        }
        if self.cache.contains_key(&chave) {
            self.metrics.config_invalidadas.increment(1);
        }
        self.reload(chave)
    }

    pub fn invalidar(&self, org: OrgId, agente: AgentId) {
        self.cache.invalidate(&Chave { org, agente });
        self.update_size();
    }

    pub fn forget_org(&self, org: OrgId) {
        let keys: HashSet<Chave> = self.cache.iter().map(|(chave, _)| *chave).collect();
        for chave in keys {
            if chave.org == org {
                self.cache.invalidate(&chave);
            }
        }
        self.update_size();
    }

    pub fn synchronise(&self) {
        self.cache.run_pending_tasks();
        self.update_size();
    }

    pub fn size(&self) -> u64 {
        self.cache.entry_count()
    }

    pub fn bytes(&self) -> u64 {
        self.cache.weighted_size()
    }

    fn reload(&self, chave: Chave) -> Resultado<Arc<HotAgent>> {
        self.metrics.config_errors.increment(1);
        let agente = self.store.agents().get_for_turn(chave.org, chave.agente)?;
        let instructions = match self.store.instructions().get(chave.org, chave.agente) {
            Ok(instructions) => Some(instructions),
            Err(error) if error.is_not_found() => None,
            Err(error) => return Err(AgentError::Store(error)),
        };
        let hot = Arc::new(build(agente, instructions));
        self.cache.insert(chave, Arc::clone(&hot));
        self.update_size();
        Ok(hot)
    }

    fn update_size(&self) {
        let agents = self.cache.entry_count();
        self.metrics
            .agents_load
            .set(i64::try_from(agents).unwrap_or(i64::MAX));
        self.metrics
            .config_bytes
            .set(i64::try_from(self.cache.weighted_size()).unwrap_or(i64::MAX));
    }
}

fn build(agente: Agente, instructions: Option<Instructions>) -> HotAgent {
    let structured = instructions.as_ref().and_then(|instructions| {
        StructuredInstructions::from_tolerant_json(&instructions.structured_json).ok()
    });
    let tools: Vec<String> = structured
        .as_ref()
        .map(|dados| {
            dados
                .enabled_tools()
                .into_iter()
                .map(|tool| tool.name.clone())
                .collect()
        })
        .unwrap_or_default();
    let markdown = instructions
        .as_ref()
        .map(|instructions| instructions.markdown.clone())
        .unwrap_or_default();
    let top_k = agente.rag_top_k;
    let cutoff = agente.rag_cutoff;
    let version_instructions = instructions
        .as_ref()
        .map(|instructions| instructions.version)
        .unwrap_or(0);
    let bytes = estimate_bytes(
        &agente,
        instructions.as_ref(),
        &structured,
        &markdown,
        &tools,
    );
    HotAgent {
        agente,
        instructions,
        structured,
        markdown,
        tools,
        top_k,
        cutoff,
        version_instructions,
        bytes,
    }
}

fn estimate_bytes(
    agente: &Agente,
    instructions: Option<&Instructions>,
    structured: &Option<StructuredInstructions>,
    markdown: &str,
    tools: &[String],
) -> u64 {
    const FIXO: u64 = 512;
    let mut bytes = FIXO;
    bytes += agente.name.len() as u64
        + agente.slug.len() as u64
        + agente.description.len() as u64
        + agente.provedor.len() as u64
        + agente.modelo.len() as u64
        + agente.jev_json.len() as u64;
    bytes += markdown.len() as u64;
    if let Some(instructions) = instructions {
        bytes += instructions.structured_json.len() as u64;
    }
    if let Some(structured) = structured {
        bytes += structured.identity.name.len() as u64;
    }
    for tool in tools {
        bytes += tool.len() as u64;
    }
    bytes
}
