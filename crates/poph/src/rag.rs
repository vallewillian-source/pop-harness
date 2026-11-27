use std::sync::Arc;

use poph_agent::{KnowledgeBase, ProductCatalog};
use poph_rag::{Embedder, Index, IndexConfig, ModelConfig, OrtEmbedder, QueueConfig};
use poph_store::Store;
use poph_types::HarnessConfig;

pub fn open_index(config: &HarnessConfig, store: &Arc<Store>) -> Result<Index, String> {
    let modelo = ModelConfig {
        diretorio: config.models(),
        ..ModelConfig::default()
    };
    let embedder = OrtEmbedder::open_model(&modelo, None).map_err(|error| {
        format!(
            "modelo de embedding indisponível em {} ({error}); rode `poph model install`",
            modelo.diretorio.display()
        )
    })?;
    let embedder: Arc<dyn Embedder> = Arc::new(embedder);
    let (index, opening) = Index::open(store.rag_path(), embedder, IndexConfig::default())
        .map_err(|error| format!("não consegui abrir o índice: {error}"))?;
    if opening.cleaned_by_divergence {
        eprintln!(
            "[aviso] o índice foi limpo: os vetores eram de outro modelo ou de outro divisor de \
             trechos. Rode `poph kb reindex` para reindexar."
        );
    }
    Ok(index)
}

pub fn open_knowledge_base(
    config: &HarnessConfig,
    store: Arc<Store>,
) -> Result<Arc<KnowledgeBase>, String> {
    let index = open_index(config, &store)?;
    Ok(KnowledgeBase::new(store, Arc::new(index)))
}

pub fn open_catalog(
    config: &HarnessConfig,
    store: Arc<Store>,
) -> Result<Arc<ProductCatalog>, String> {
    let index = open_index(config, &store)?;
    Ok(ProductCatalog::new(store, Arc::new(index)))
}

#[derive(Debug, clap::Subcommand)]
pub enum RagAction {
    #[command(
        about = "Deixa o índice igual à base ativa do(s) agente(s). Mesmo comando do `poph kb reindex`"
    )]
    Reindex(crate::kb::ReindexArgs),
}

pub fn run(
    flag: Option<std::path::PathBuf>,
    environment: &crate::config::Environment,
    offline: bool,
    org: Option<&str>,
    acao: RagAction,
) -> std::process::ExitCode {
    match acao {
        RagAction::Reindex(args) => crate::kb::run(
            flag,
            environment,
            offline,
            org,
            crate::kb::KbAction::Reindex(args),
        ),
    }
}

pub fn open_queued_knowledge_base(
    config: &HarnessConfig,
    store: Arc<Store>,
    org: poph_types::OrgId,
) -> Result<Arc<KnowledgeBase>, String> {
    let modelo = ModelConfig {
        diretorio: config.models(),
        ..ModelConfig::default()
    };
    let base = OrtEmbedder::open_model(&modelo, None).map_err(|error| {
        format!(
            "modelo de embedding indisponível em {} ({error}); rode `poph model install`",
            modelo.diretorio.display()
        )
    })?;
    let identity = base.identity().clone();
    let embedder: Arc<dyn Embedder> = Arc::new(base);

    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("não consegui iniciar o runtime da fila: {error}"))?,
    );
    let queue = {
        let _guard = runtime.enter();
        Arc::new(
            poph_rag::EmbeddingQueue::start(Arc::clone(&embedder), QueueConfig::default(), None)
                .map_err(|error| format!("não consegui iniciar a fila de embedding: {error}"))?,
        )
    };
    let org_embedder = poph_rag::QueuedEmbedder::with_runtime(queue, org, identity, runtime);
    let (index, opening) = Index::open(
        store.rag_path(),
        Arc::new(org_embedder),
        IndexConfig::default(),
    )
    .map_err(|error| format!("não consegui abrir o índice: {error}"))?;
    if opening.cleaned_by_divergence {
        eprintln!(
            "[aviso] o índice foi limpo: os vetores eram de outro modelo ou de outro divisor de \
             trechos. Rode `poph kb reindex` para reindexar."
        );
    }
    Ok(KnowledgeBase::new(store, Arc::new(index)))
}
