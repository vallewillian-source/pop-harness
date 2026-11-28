use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};

use argon2::password_hash::PasswordHash;
use poph_agent::entry_output::Fluxo;
use poph_agent::turno::TurnEngine;
use poph_files::FileConfig;
use poph_flows::{Compiled, Ir, Limites, LoadFailure, RevisionCache, RevisionKey, compile};
use poph_store::Store;
use poph_types::{BaseMetrics, ConversationId, HarnessConfig, OrgId, Registry};

use crate::cipher::CipherKey;
use crate::cluster::LiveCluster;
use crate::evolution::EvolutionChannel;
use crate::flow_metrics::FlowMetrics;
use crate::http_effects::HttpEffectRegistry;
use crate::manifests::ManifestCatalog;

#[derive(Debug, Clone)]
pub struct GlobalToken {
    pub label: String,
    pub hash: String,
}

#[derive(Clone)]
pub struct ServerState {
    pub store: Arc<Store>,
    pub config: Arc<HarnessConfig>,
    pub registry: Arc<Registry>,
    pub metrics: BaseMetrics,
    pub global_tokens: Vec<GlobalToken>,
    pub started_at: SystemTime,
    pub evolution_channels: Arc<Vec<EvolutionChannel>>,
    pub turn: Option<Fluxo>,
    pub turn_interval: Duration,
    pub manifests: Arc<ManifestCatalog>,
    pub encryption_key: Option<Arc<CipherKey>>,
    pub engine: Option<Arc<TurnEngine>>,
    pub(crate) site_workers: Arc<crate::site_chat::SiteWorkers>,
    pub(crate) adapter_workers: Arc<crate::adapter_chat::AdapterWorkers>,
    pub files: Arc<FileConfig>,
    pub active_test_turns: Arc<Mutex<HashMap<ConversationId, Instant>>>,
    pub reservation_ttl: Duration,
    pub matrix_as: Option<Arc<crate::api::matrix_as::AsReceiver>>,
    matrix_metrics: Arc<Mutex<Option<Arc<crate::matrix_metrics::MetricsMatrix>>>>,
    matrix_connection: Option<Arc<crate::matrix_beginning::ConnectionSignal>>,
    matrix_spaces: Option<Arc<crate::matrix_beginning::ModuleSpaces>>,
    pub flow_cache: Arc<RevisionCache>,
    pub http_effects: Arc<HttpEffectRegistry>,
    flow_metrics: Option<Arc<FlowMetrics>>,
    cluster: Arc<RwLock<LiveCluster>>,
    seen_nonces: Arc<Mutex<HashMap<String, Instant>>>,
}

pub const RESERVATION_TTL: Duration = Duration::from_secs(30 * 60);

impl ServerState {
    pub fn new(store: Arc<Store>, config: HarnessConfig) -> Result<Self, String> {
        let mut global_tokens = Vec::with_capacity(config.admin.tokens_globais.len());
        for token in &config.admin.tokens_globais {
            PasswordHash::new(&token.hash).map_err(|error| {
                format!(
                    "token global `{}`: hash argon2 inválido ({}); gere outro com `poph token global`",
                    token.label, error
                )
            })?;
            global_tokens.push(GlobalToken {
                label: token.label.clone(),
                hash: token.hash.clone(),
            });
        }
        if global_tokens.is_empty() {
            tracing::warn!(
                "nenhum token global configurado ([admin].tokens_globais): só tokens de org \
                 autenticam na API"
            );
        }
        let manifests = Arc::new(ManifestCatalog::new(&config.plugins));
        let files = Arc::new(FileConfig::default(config.blobs()));
        let registry = Arc::new(Registry::new());
        let metrics = BaseMetrics::register(&registry)
            .map_err(|error| format!("não consegui registrar as métricas: {error}"))?;
        let flow_cache = Arc::new(RevisionCache::new(
            usize::try_from(config.flows.revision_cache_mb).unwrap_or(usize::MAX),
        ));
        let flow_metrics = if config.flows.enabled {
            let metrics = FlowMetrics::register(&registry).map_err(|error| {
                format!("não consegui registrar as métricas de fluxos: {error}")
            })?;
            store.publish_execution_metrics(metrics.execution_metrics());
            Some(Arc::new(metrics))
        } else {
            None
        };
        let http_effects = Arc::new(HttpEffectRegistry::from_environment(&config.http_effects)?);
        Ok(Self {
            store,
            config: Arc::new(config),
            registry,
            metrics,
            global_tokens,
            started_at: SystemTime::now(),
            evolution_channels: Arc::new(Vec::new()),
            turn: None,
            turn_interval: Duration::from_millis(1000),
            manifests,
            encryption_key: CipherKey::from_environment().map(Arc::new),
            engine: None,
            site_workers: Arc::default(),
            adapter_workers: Arc::default(),
            files,
            active_test_turns: Arc::new(Mutex::new(HashMap::new())),
            reservation_ttl: RESERVATION_TTL,
            matrix_as: None,
            matrix_metrics: Arc::new(Mutex::new(None)),
            matrix_connection: None,
            matrix_spaces: None,
            cluster: Arc::new(RwLock::new(LiveCluster::single_instance(PathBuf::from(
                "cluster.toml",
            )))),
            seen_nonces: Arc::new(Mutex::new(HashMap::new())),
            flow_cache,
            http_effects,
            flow_metrics,
        })
    }

    pub fn flow_metrics(&self) -> Option<Arc<FlowMetrics>> {
        self.flow_metrics.clone()
    }

    pub fn compile_flow(&self, source: &str) -> Compiled {
        compile(source, &Limites::from_config(&self.config.flows))
    }

    pub fn load_flow_revision(
        &self,
        key: &RevisionKey,
        stored_ir: &Ir,
        source: &str,
    ) -> Result<Arc<Ir>, LoadFailure> {
        self.flow_cache.load(
            key,
            stored_ir,
            source,
            &Limites::from_config(&self.config.flows),
        )
    }

    pub fn register_nonce(&self, nonce: &str) -> bool {
        let mut seen = self
            .seen_nonces
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let window = crate::forwarding::REPLAY_WINDOW;
        seen.retain(|_, when| when.elapsed() < window);
        if seen.len() >= crate::forwarding::MAX_NONCES {
            tracing::warn!(
                stored_nonces = seen.len(),
                "memória de nonces do encaminhamento cheia: zerada para continuar aceitando pedidos"
            );
            seen.clear();
        }
        if seen.contains_key(nonce) {
            return false;
        }
        seen.insert(nonce.to_owned(), Instant::now());
        true
    }

    pub fn with_engine(mut self, engine: Arc<TurnEngine>) -> Self {
        self.engine = Some(engine);
        self
    }

    pub fn with_files(mut self, files: FileConfig) -> Self {
        self.files = Arc::new(files);
        self
    }

    pub fn reserve_test_turn(&self, conversation_id: ConversationId) -> bool {
        let mut reservations = self
            .active_test_turns
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(started_at) = reservations.get(&conversation_id) {
            if started_at.elapsed() < self.reservation_ttl {
                return false;
            }
            tracing::warn!(
                conversation_id = %conversation_id,
                ttl_ms = u64::try_from(self.reservation_ttl.as_millis()).unwrap_or(u64::MAX),
                "reserva de turno de teste vencida: a conversa volta a aceitar mensagem"
            );
        }
        reservations.insert(conversation_id, Instant::now());
        true
    }

    pub fn release_test_turn(&self, conversation_id: ConversationId) {
        self.active_test_turns
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&conversation_id);
    }

    pub fn with_reservation_ttl(mut self, ttl: Duration) -> Self {
        self.reservation_ttl = ttl;
        self
    }

    pub fn with_cluster(mut self, cluster: LiveCluster) -> Self {
        self.cluster = Arc::new(RwLock::new(cluster));
        self
    }

    pub fn cluster(&self) -> LiveCluster {
        self.cluster
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    pub fn replace_cluster(&self, new: LiveCluster) {
        *self
            .cluster
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = new;
    }

    pub fn with_evolution_channels(mut self, channels: Vec<EvolutionChannel>) -> Self {
        self.evolution_channels = Arc::new(channels);
        self
    }

    pub fn with_matrix_as(mut self, receiver: Arc<crate::api::matrix_as::AsReceiver>) -> Self {
        self.matrix_as = Some(receiver);
        self
    }

    pub(crate) fn matrix_blob_folder(&self) -> &std::path::Path {
        &self.files.blob_folder
    }

    pub fn set_matrix_as(&mut self, receiver: Arc<crate::api::matrix_as::AsReceiver>) {
        self.matrix_as = Some(receiver);
    }

    pub fn set_matrix_connection_signal(
        &mut self,
        signal: Arc<crate::matrix_beginning::ConnectionSignal>,
    ) {
        self.matrix_connection = Some(signal);
    }

    pub fn set_matrix_spaces(&mut self, spaces: Arc<crate::matrix_beginning::ModuleSpaces>) {
        self.matrix_spaces = Some(spaces);
    }

    pub fn matrix_spaces(&self) -> Option<Arc<crate::matrix_beginning::ModuleSpaces>> {
        self.matrix_spaces.as_ref().map(Arc::clone)
    }

    pub fn matrix_connected(&self) -> Option<bool> {
        self.matrix_connection
            .as_ref()
            .map(|signal| signal.connected())
    }

    pub fn matrix_metrics(
        &self,
    ) -> poph_types::DomainResult<Option<Arc<crate::matrix_metrics::MetricsMatrix>>> {
        if !self.matrix_module_enabled() {
            return Ok(None);
        }
        let mut guard = self
            .matrix_metrics
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(existing) = guard.as_ref() {
            return Ok(Some(Arc::clone(existing)));
        }
        let metrics = Arc::new(crate::matrix_metrics::MetricsMatrix::register(
            &self.registry,
        )?);
        *guard = Some(Arc::clone(&metrics));
        Ok(Some(metrics))
    }

    fn matrix_module_enabled(&self) -> bool {
        self.config
            .matrix
            .as_ref()
            .is_some_and(|matrix| matrix.enabled)
    }

    pub fn with_encryption_key(mut self, key: Option<CipherKey>) -> Self {
        self.encryption_key = key.map(Arc::new);
        self
    }

    pub fn with_turn(mut self, flow: Fluxo, interval: Duration) -> Self {
        self.turn = Some(flow);
        self.turn_interval = interval;
        self
    }

    pub fn evolution_channel(&self, org: &str, instance: &str) -> Option<&EvolutionChannel> {
        let org: OrgId = org.parse().ok()?;
        self.evolution_channels
            .iter()
            .find(|channel| channel.org == org && channel.instance == instance)
    }

    pub fn evolution_channel_for_integration(
        &self,
        org: OrgId,
        integration: poph_types::IntegrationId,
    ) -> Option<&EvolutionChannel> {
        self.evolution_channels
            .iter()
            .find(|channel| channel.org == org && channel.integration == integration)
    }
}

impl std::fmt::Debug for ServerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Estado")
            .field("tokens_globais", &self.global_tokens.len())
            .field("inicio", &self.started_at)
            .finish_non_exhaustive()
    }
}
