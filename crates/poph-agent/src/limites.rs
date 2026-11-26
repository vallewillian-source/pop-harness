use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poph_types::{AgentId, LimitsConfig, LlmConfig, OrgId};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::error::{AgentError, Resultado};
use crate::metrics::MetricsMemory;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitOptions {
    pub turns_global: u32,
    pub turns_by_org: u32,
    pub turns_by_agent: u32,
    pub calls_by_provider: u32,
    pub cap_by_provider: BTreeMap<String, u32>,
    pub taxa_por_org_por_minuto: u32,
}

impl Default for LimitOptions {
    fn default() -> Self {
        let limites = LimitsConfig::default();
        Self::of_limits(&limites)
    }
}

impl LimitOptions {
    pub fn of_limits(limites: &LimitsConfig) -> Self {
        Self {
            turns_global: limites.concurrent_turns.max(1),
            turns_by_org: limites
                .concurrent_turns_per_org
                .clamp(1, limites.concurrent_turns.max(1)),
            turns_by_agent: 0,
            calls_by_provider: 32,
            cap_by_provider: BTreeMap::new(),
            taxa_por_org_por_minuto: limites.rate_per_org_per_minute,
        }
    }

    pub fn of_config(limites: &LimitsConfig, llm: &LlmConfig) -> Self {
        Self::of_limits(limites).with_provider(llm.concurrent_calls)
    }

    pub fn with_provider(mut self, simultaneous: u32) -> Self {
        self.calls_by_provider = simultaneous.max(1);
        self
    }

    pub fn with_cap_by_provider(mut self, cap: BTreeMap<String, u32>) -> Self {
        self.cap_by_provider = cap;
        self
    }

    pub fn with_agent(mut self, simultaneos: u32) -> Self {
        self.turns_by_agent = simultaneos;
        self
    }
}

#[derive(Debug)]
pub struct Slot {
    _global: OwnedSemaphorePermit,
    _org: OwnedSemaphorePermit,
    _agent: Option<OwnedSemaphorePermit>,
}

#[derive(Debug)]
pub struct ProviderSlot {
    _permit: OwnedSemaphorePermit,
}

pub struct Limiter {
    options: LimitOptions,
    global: Arc<Semaphore>,
    by_org: Mutex<HashMap<OrgId, Arc<Semaphore>>>,
    by_agent: Mutex<HashMap<(OrgId, AgentId), Arc<Semaphore>>>,
    by_provider: Mutex<HashMap<String, Arc<Semaphore>>>,
    taxas: Mutex<HashMap<OrgId, Balde>>,
    metrics: MetricsMemory,
}

#[derive(Debug)]
struct Balde {
    tokens: f64,
    last: Instant,
}

impl Limiter {
    pub fn new(options: LimitOptions, metrics: MetricsMemory) -> Self {
        Self {
            global: Arc::new(Semaphore::new(options.turns_global as usize)),
            by_org: Mutex::new(HashMap::new()),
            by_agent: Mutex::new(HashMap::new()),
            by_provider: Mutex::new(HashMap::new()),
            taxas: Mutex::new(HashMap::new()),
            options,
            metrics,
        }
    }

    pub fn options(&self) -> &LimitOptions {
        &self.options
    }

    pub async fn acquire_turn(
        &self,
        org: OrgId,
        agente: AgentId,
        wait: Duration,
    ) -> Resultado<Slot> {
        let beginning = Instant::now();
        let global = self.acquire(&self.global, "instância", wait).await?;
        let org_semaphore = self.semaphore_of_org(org);
        let by_org = self.acquire(&org_semaphore, "org", wait).await?;
        let by_agent = if self.options.turns_by_agent == 0 {
            None
        } else {
            let semaphore = self.semaphore_agent(org, agente);
            Some(self.acquire(&semaphore, "agente", wait).await?)
        };
        self.metrics
            .wait_slot_seconds
            .observe(beginning.elapsed().as_secs_f64());
        Ok(Slot {
            _global: global,
            _org: by_org,
            _agent: by_agent,
        })
    }

    pub async fn acquire_provider(
        &self,
        provedor: &str,
        wait: Duration,
    ) -> Resultado<ProviderSlot> {
        let semaphore = self.semaphore_provider(provedor);
        let permit = match tokio::time::timeout(wait, semaphore.acquire_owned()).await {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => {
                return Err(AgentError::Internal(format!(
                    "semáforo do provedor `{provedor}` fechado inesperadamente"
                )));
            }
            Err(_) => {
                return Err(AgentError::WithoutSlotProvider {
                    provedor: provedor.to_owned(),
                    wait_ms: wait.as_millis() as u64,
                });
            }
        };
        Ok(ProviderSlot { _permit: permit })
    }

    pub fn consumir_cota(&self, org: OrgId) -> Resultado<()> {
        let limit = self.options.taxa_por_org_por_minuto;
        if limit == 0 {
            return Ok(());
        }
        let mut taxas = match self.taxas.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        };
        let now = Instant::now();
        let balde = taxas.entry(org).or_insert(Balde {
            tokens: f64::from(limit),
            last: now,
        });
        let pass = now.duration_since(balde.last).as_secs_f64();
        balde.tokens = (balde.tokens + pass * (f64::from(limit) / 60.0)).min(f64::from(limit));
        balde.last = now;
        if balde.tokens >= 1.0 {
            balde.tokens -= 1.0;
            return Ok(());
        }
        let missing = 1.0 - balde.tokens;
        let wait_ms = (missing * 60.0 / f64::from(limit) * 1000.0).ceil() as u64;
        Err(AgentError::CotaExhausted { limit, wait_ms })
    }

    async fn acquire(
        &self,
        semaphore: &Arc<Semaphore>,
        scope: &str,
        wait: Duration,
    ) -> Resultado<OwnedSemaphorePermit> {
        match tokio::time::timeout(wait, Arc::clone(semaphore).acquire_owned()).await {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) => Err(AgentError::Internal(format!(
                "semáforo de {scope} fechado inesperadamente"
            ))),
            Err(_) => Err(AgentError::WithoutSlot {
                scope: scope.to_owned(),
                wait_ms: wait.as_millis() as u64,
            }),
        }
    }

    fn semaphore_of_org(&self, org: OrgId) -> Arc<Semaphore> {
        let mut map = self
            .by_org
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        Arc::clone(
            map.entry(org)
                .or_insert_with(|| Arc::new(Semaphore::new(self.options.turns_by_org as usize))),
        )
    }

    fn semaphore_agent(&self, org: OrgId, agente: AgentId) -> Arc<Semaphore> {
        let maximum = self.options.turns_by_agent as usize;
        let mut map = self
            .by_agent
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        Arc::clone(
            map.entry((org, agente))
                .or_insert_with(|| Arc::new(Semaphore::new(maximum))),
        )
    }

    fn semaphore_provider(&self, provedor: &str) -> Arc<Semaphore> {
        let maximum = self
            .options
            .cap_by_provider
            .get(provedor)
            .copied()
            .unwrap_or(self.options.calls_by_provider)
            .max(1) as usize;
        let mut map = self
            .by_provider
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        Arc::clone(
            map.entry(provedor.to_owned())
                .or_insert_with(|| Arc::new(Semaphore::new(maximum))),
        )
    }
}
