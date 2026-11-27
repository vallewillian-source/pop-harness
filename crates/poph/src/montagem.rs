use std::collections::BTreeMap;
use std::sync::Arc;

use poph_agent::actors::{ActorOptions, Actors};
use poph_agent::base_knowledge::KnowledgeBase;
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::entry_output::{FlowOptions, Fluxo};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsFlow, MetricsMemory, MetricsTurn};
use poph_agent::providers::{Providers, ResolvedProvider};
use poph_agent::turno::{AdjustmentsFlow, DefaultRegistry, TurnEngine, TurnOptions, TurnProcessor};
use poph_agent::{KnowledgeSearcher, ProductCatalog, ProductSearcher, QueryKnowledge, record_with};
use poph_core::llm::{LlmLimiter, LlmProvider, OpenAiCompatibleClient, OpenAiCompatibleConfig};
use poph_flows::cache::RevisionCache;
use poph_store::Store;
use poph_types::{BaseMetrics, HarnessConfig, LlmConfig, Registry, Secret};

use crate::config;

pub struct Keys {
    pub llm: Secret,
    pub jev: Option<Secret>,
}

pub struct Metrics {
    base: BaseMetrics,
    turn: MetricsTurn,
    flow: MetricsFlow,
    memory: MetricsMemory,
}

impl Metrics {
    pub fn own(base: BaseMetrics) -> Self {
        Self {
            base,
            turn: MetricsTurn::own(),
            flow: MetricsFlow::own(),
            memory: MetricsMemory::own(),
        }
    }

    pub fn registered(base: BaseMetrics, registry: &Registry) -> Result<Self, String> {
        Ok(Self {
            base,
            turn: MetricsTurn::register(registry)
                .map_err(|error| format!("métricas do turno: {error}"))?,
            flow: MetricsFlow::register(registry)
                .map_err(|error| format!("métricas da fila: {error}"))?,
            memory: MetricsMemory::register(registry)
                .map_err(|error| format!("métricas de memória: {error}"))?,
        })
    }
}

#[derive(Default)]
pub struct ProcessFlows {
    cache: Option<Arc<RevisionCache>>,
    g9: Option<poph_agent::metrics::MetricsG9>,
}

impl ProcessFlows {
    pub fn from_server(estado: &poph_server::ServerState) -> Self {
        let g9 = estado
            .flow_metrics()
            .map(|metrics| poph_agent::metrics::MetricsG9 {
                start: metrics.g9_started.clone(),
                none: metrics.g9_none.clone(),
                below_threshold: metrics.g9_below_threshold.clone(),
                fallback: metrics.g9_fallback.clone(),
            });
        Self {
            cache: Some(Arc::clone(&estado.flow_cache)),
            g9,
        }
    }
}

pub struct Assembly {
    pub engine: Arc<TurnEngine>,
    pub flow: Fluxo,
    pub base: Option<Arc<KnowledgeBase>>,
    pub catalog: Option<Arc<ProductCatalog>>,
}

const BASE_TOOLS: [&str; 2] = [
    poph_agent::base_knowledge::NAME_TOOL,
    poph_agent::products::NAME_TOOL,
];

fn database_tools(store: Arc<Store>) -> DefaultRegistry {
    let registry = poph_agent::cart::register(DefaultRegistry::new(), Arc::clone(&store));
    poph_agent::library::register(registry, store)
}

fn turn_registry(
    store: Arc<Store>,
    hot: Arc<ConfigHot>,
    base: Option<(&Arc<KnowledgeBase>, &Arc<ProductCatalog>)>,
) -> DefaultRegistry {
    let Some((base, catalog)) = base else {
        return database_tools(store);
    };
    let search: Arc<dyn KnowledgeSearcher> = Arc::clone(base) as Arc<dyn KnowledgeSearcher>;
    let products: Arc<dyn ProductSearcher> = Arc::clone(catalog) as Arc<dyn ProductSearcher>;
    let registry = QueryKnowledge::registro(Arc::clone(&hot), search);
    record_with(registry, hot, products)
}

fn llm_client(
    llm: &LlmConfig,
    key: Secret,
    metrics: BaseMetrics,
    label: &str,
) -> Result<Arc<dyn LlmProvider>, String> {
    let client = OpenAiCompatibleConfig::from_config(llm, key)
        .map_err(|error| format!("configuração do provedor {label}: {error}"))?;
    let llm_limits = LlmLimiter::new(llm.concurrent_calls as usize, llm.concurrent_calls as usize)
        .map_err(|error| format!("limites do provedor {label}: {error}"))?;
    let client = OpenAiCompatibleClient::new(client, llm_limits, metrics)
        .map_err(|error| format!("cliente do provedor {label}: {error}"))?;
    Ok(Arc::new(client))
}

fn provider_registry(
    configuration: &HarnessConfig,
    environment: &config::Environment,
    metrics: BaseMetrics,
    default_provider: &Arc<dyn LlmProvider>,
) -> Result<(Providers, BTreeMap<String, u32>), String> {
    let default_provider = ResolvedProvider::new(
        configuration.llm.provider.as_str(),
        Arc::clone(default_provider),
        configuration.llm.model.clone(),
    );
    let mut named = BTreeMap::new();
    let mut caps = BTreeMap::new();
    for (name, override_config) in &configuration.providers {
        let effective = override_config.effective(&configuration.llm);
        let Some(key) = config::llm_key(environment, &effective.api_key_env) else {
            eprintln!(
                "[aviso] provedor `{name}` sem a variável {} no ambiente: agentes que apontam para ele não atendem",
                effective.api_key_env
            );
            continue;
        };
        let client = llm_client(&effective, key, metrics.clone(), &format!("`{name}`"))?;
        caps.insert(name.clone(), effective.concurrent_calls);
        named.insert(
            name.clone(),
            ResolvedProvider::new(name.as_str(), client, effective.model.clone()),
        );
    }
    Ok((Providers::new(default_provider, named), caps))
}

pub fn assemble(
    configuration: &HarnessConfig,
    store: Arc<Store>,
    keys: Keys,
    metrics: Metrics,
    process_flows: ProcessFlows,
    environment: &config::Environment,
) -> Result<Assembly, String> {
    let jev_metrics = metrics.base.clone();
    let provider = llm_client(&configuration.llm, keys.llm, metrics.base.clone(), "padrão")?;
    let (providers, caps) = provider_registry(configuration, environment, metrics.base, &provider)?;

    let base = match crate::rag::abrir_base(configuration, Arc::clone(&store)) {
        Ok(base) => Some(base),
        Err(error) => {
            eprintln!("[aviso] busca na base desligada: {error}");
            None
        }
    };

    let http_tools =
        poph_agent::turno::HttpToolRegistry::from_config(&configuration.http_tools, |name| {
            config::llm_key(environment, name)
        })?;
    let mut engine = TurnEngine::new(
        Arc::clone(&store),
        ConfigHot::new(
            Arc::clone(&store),
            HotConfigOptions::default(),
            metrics.memory.clone(),
        ),
        Arc::new(Limiter::new(
            LimitOptions::of_config(&configuration.limits, &configuration.llm)
                .with_cap_by_provider(caps),
            metrics.memory.clone(),
        )),
        provider,
        TurnOptions::default(),
        metrics.turn,
    )
    .with_http_tools(http_tools)
    .with_default_model(configuration.llm.model.clone())
    .with_providers(Arc::new(providers))
    .with_files(poph_files::FileConfig::default(configuration.blobs()))
    .with_handoff(poph_jev::HandoffGuidance::from_config(
        &configuration.handoff,
    ))
    .with_flows(&configuration.flows);
    if let Some(cache) = process_flows.cache.clone() {
        engine = engine.with_flow_cache(cache);
    }
    if let Some(g9) = process_flows.g9 {
        engine = engine.with_g9_metrics(g9);
    }
    if configuration.jev.enabled {
        match keys.jev {
            None => eprintln!(
                "[aviso] Jev ligado na configuração, mas sem TYPESAFE_API_KEY: seguindo sem Jev"
            ),
            Some(key) => match poph_jev::JevClientConfig::from_config(&configuration.jev, key)
                .and_then(|client| poph_jev::JevClient::new(client, jev_metrics))
            {
                Ok(transport) => {
                    let evaluator = poph_jev::JevEngine::new(
                        Arc::new(transport),
                        configuration.jev.gates,
                        configuration.jev.model.clone(),
                    );
                    engine = engine.with_jev(Arc::new(evaluator));
                }
                Err(error) => eprintln!("[aviso] Jev desligado: {error}"),
            },
        }
    }
    let catalog = base
        .as_ref()
        .map(|base| ProductCatalog::new(Arc::clone(&store), Arc::clone(base.index())));
    let registry = turn_registry(
        Arc::clone(&store),
        Arc::clone(engine.hot_config()),
        base.as_ref().zip(catalog.as_ref()),
    );
    let flow_adjustments = match process_flows.cache.clone() {
        Some(cache) => AdjustmentsFlow::of_config(&configuration.flows).with_cache(cache),
        None => AdjustmentsFlow::of_config(&configuration.flows),
    };
    let registry =
        poph_agent::turno::register_flow_tools_with(registry, Arc::clone(&store), flow_adjustments);
    let registry = poph_agent::turno::register_closing_tool(registry, Arc::clone(&store));
    match (&base, &catalog) {
        (Some(base), Some(catalog)) => {
            let search: Arc<dyn KnowledgeSearcher> = Arc::clone(base) as Arc<dyn KnowledgeSearcher>;
            let products: Arc<dyn ProductSearcher> =
                Arc::clone(catalog) as Arc<dyn ProductSearcher>;
            engine = engine.with_products(products).with_search(search);
        }
        _ => {
            eprintln!(
                "[aviso] {} ferramentas dependem da base de conhecimento e não serão oferecidas sem ela: {}",
                BASE_TOOLS.len(),
                BASE_TOOLS.join(", ")
            );
        }
    }
    engine = engine.with_tools(Arc::new(registry));
    let engine = Arc::new(engine);
    let processor = Arc::new(TurnProcessor::new(Arc::clone(&engine)));
    let actors = Actors::new(processor.clone(), ActorOptions::default(), metrics.memory);
    let flow_options = FlowOptions::default().with_failure_model(
        configuration.turn.model_failure_attempts,
        std::time::Duration::from_millis(configuration.turn.model_failure_delay_ms),
    );
    let flow = Fluxo::with_metrics(Arc::clone(&store), actors, flow_options, metrics.flow);
    processor.attach_io(flow.clone());

    Ok(Assembly {
        engine,
        flow,
        base,
        catalog,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
    use poph_agent::metrics::MetricsMemory;
    use poph_rag::{Embedder, Index, IndexConfig};

    use super::*;

    fn dir(label: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("poph-montagem-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("cria a pasta de teste");
        path
    }

    fn store(dir: &std::path::Path) -> Arc<Store> {
        Arc::new(
            Store::open_at(
                &dir.join("dados").join("harness.db"),
                &dir.join("dados").join("rag.db"),
            )
            .expect("abre o banco de teste"),
        )
    }

    fn hot(store: &Arc<Store>) -> Arc<ConfigHot> {
        Arc::new(ConfigHot::new(
            Arc::clone(store),
            HotConfigOptions::default(),
            MetricsMemory::own(),
        ))
    }

    fn base_and_catalog(
        store: &Arc<Store>,
        dir: &std::path::Path,
    ) -> (Arc<KnowledgeBase>, Arc<ProductCatalog>) {
        let open_index = || {
            let embedder: Arc<dyn Embedder> = Arc::new(poph_rag::FakeEmbedder::new());
            Index::open(
                &dir.join("dados").join("rag.db"),
                embedder,
                IndexConfig::default(),
            )
            .expect("abre o índice de teste")
            .0
        };
        let base = KnowledgeBase::new(Arc::clone(store), Arc::new(open_index()));
        let catalog = ProductCatalog::new(Arc::clone(store), Arc::new(open_index()));
        (base, catalog)
    }

    #[test]
    fn without_base_send_file_and_cart_stay_without_knowledge_or_product() {
        let folder = dir("sem-base");
        let store = store(&folder);
        let names = database_tools(Arc::clone(&store)).names();
        assert!(
            names.contains(&poph_agent::NAME_SEND_FILE.to_owned()),
            "enviar_arquivo fora da lista: {names:?}"
        );
        for cart in [
            "cart_add_item",
            "cart_remove_item",
            "cart_summary",
            "cart_clear",
        ] {
            assert!(
                names.contains(&cart.to_owned()),
                "{cart} fora da lista: {names:?}"
            );
        }
        for dependent in BASE_TOOLS {
            assert!(
                !names.contains(&dependent.to_owned()),
                "{dependent} não deveria existir sem base: {names:?}"
            );
        }
    }

    #[test]
    fn without_base_engine_gets_no_duplicate_file_tool() {
        let folder = dir("sem-base-contagem");
        let store = store(&folder);
        let names = database_tools(Arc::clone(&store)).names();
        assert_eq!(
            names,
            vec![
                "cart_add_item".to_owned(),
                "cart_clear".to_owned(),
                "cart_remove_item".to_owned(),
                "cart_summary".to_owned(),
                poph_agent::NAME_SEND_FILE.to_owned(),
            ]
        );
    }

    #[test]
    fn with_base_list_stays_as_before_hr_46f() {
        let folder = dir("com-base");
        let store = store(&folder);
        let (base, catalog) = base_and_catalog(&store, &folder);
        let registry = turn_registry(Arc::clone(&store), hot(&store), Some((&base, &catalog)));
        let names = registry.names();
        assert_eq!(
            names,
            vec![
                "buscar_produto".to_owned(),
                "cart_add_item".to_owned(),
                "cart_clear".to_owned(),
                "cart_remove_item".to_owned(),
                "cart_summary".to_owned(),
                "consultar_base".to_owned(),
                poph_agent::NAME_SEND_FILE.to_owned(),
            ],
            "a lista com base ligada mudou; era a de antes da HR-46f"
        );
    }

    #[test]
    fn turn_registry_without_base_does_not_touch_index() {
        let folder = dir("sem-base-helper");
        let store = store(&folder);
        let names = turn_registry(Arc::clone(&store), hot(&store), None).names();
        assert_eq!(names.len(), 5, "lista sem base: {names:?}");
        assert!(names.contains(&poph_agent::NAME_SEND_FILE.to_owned()));
        assert!(!names.contains(&"consultar_base".to_owned()));
    }
}
