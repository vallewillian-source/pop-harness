//! Abertura do índice de trechos e da base de conhecimento pela CLI (HR-17).
//!
//! O modelo de embedding é a única parte cara: `EmbedderOrt::abrir` confere os sha256 dos arquivos
//! (bloqueante) e a sessão ONNX só nasce no primeiro uso. Quem chama isto é o `chat` e o `kb`.

use std::sync::Arc;

use poph_agent::{BaseConhecimento, CatalogoProdutos};
use poph_rag::{ConfigIndice, ConfigModelo, Embedder, EmbedderOrt, Indice, QueueConfig};
use poph_store::Store;
use poph_types::HarnessConfig;

/// Abre o índice de trechos sobre o `rag.db` do store.
pub fn abrir_indice(config: &HarnessConfig, store: &Arc<Store>) -> Result<Indice, String> {
    let modelo = ConfigModelo {
        diretorio: config.models(),
        ..ConfigModelo::default()
    };
    let embedder = EmbedderOrt::abrir(&modelo, None).map_err(|erro| {
        format!(
            "modelo de embedding indisponível em {} ({erro}); rode `poph model install`",
            modelo.diretorio.display()
        )
    })?;
    let embedder: Arc<dyn Embedder> = Arc::new(embedder);
    let (indice, abertura) = Indice::abrir(store.rag_path(), embedder, ConfigIndice::default())
        .map_err(|erro| format!("não consegui abrir o índice: {erro}"))?;
    if abertura.limpo_por_divergencia {
        eprintln!(
            "[aviso] o índice foi limpo: os vetores eram de outro modelo ou de outro divisor de \
             trechos. Rode `poph kb reindex` para reindexar."
        );
    }
    Ok(indice)
}

/// Abre a base de conhecimento (store + índice) pronta para buscar e sincronizar.
pub fn abrir_base(
    config: &HarnessConfig,
    store: Arc<Store>,
) -> Result<Arc<BaseConhecimento>, String> {
    let indice = abrir_indice(config, &store)?;
    Ok(BaseConhecimento::nova(store, Arc::new(indice)))
}

/// Abre o catálogo de produtos (store + o mesmo índice de embedding da base).
pub fn abrir_catalogo(
    config: &HarnessConfig,
    store: Arc<Store>,
) -> Result<Arc<CatalogoProdutos>, String> {
    let indice = abrir_indice(config, &store)?;
    Ok(CatalogoProdutos::novo(store, Arc::new(indice)))
}

/// Subcomandos de `poph rag` (o índice da busca semântica).
#[derive(Debug, clap::Subcommand)]
pub enum AcaoRag {
    /// Deixa o índice igual à base ativa do(s) agente(s). Mesmo comando do `poph kb reindex`.
    Reindex(crate::kb::ReindexArgs),
}

/// Executa um subcomando de `poph rag`.
///
/// `poph rag reindex` é o nome que o PRD usa para a reindexação em lote (HR-17). A lógica mora no
/// `poph kb`, porque é lá que a base é mantida: este comando só encaminha, para não existirem duas
/// implementações do mesmo índice.
pub fn executar(
    flag: Option<std::path::PathBuf>,
    ambiente: &crate::config::Ambiente,
    offline: bool,
    org: Option<&str>,
    acao: AcaoRag,
) -> std::process::ExitCode {
    match acao {
        AcaoRag::Reindex(args) => crate::kb::executar(
            flag,
            ambiente,
            offline,
            org,
            crate::kb::AcaoKb::Reindex(args),
        ),
    }
}

/// Abre a base de conhecimento para reindexação, com o embedding passando pela fila da HR-15.
///
/// A diferença para [`abrir_base`]: o índice que a reindexação usa embrulha o modelo na
/// [`poph_rag::EmbedderDaFila`], então cada trecho entra na vaga da org (limite de pedidos
/// simultâneos e rodízio entre orgs). Numa reindexação grande, uma org não ocupa o modelo de todas
/// as outras. A busca de uma consulta continua indo direto ao modelo ([`abrir_base`]).
pub fn abrir_base_com_fila(
    config: &HarnessConfig,
    store: Arc<Store>,
    org: poph_types::OrgId,
) -> Result<Arc<BaseConhecimento>, String> {
    let modelo = ConfigModelo {
        diretorio: config.models(),
        ..ConfigModelo::default()
    };
    let base = EmbedderOrt::abrir(&modelo, None).map_err(|erro| {
        format!(
            "modelo de embedding indisponível em {} ({erro}); rode `poph model install`",
            modelo.diretorio.display()
        )
    })?;
    let identidade = base.identity().clone();
    let embedder: Arc<dyn Embedder> = Arc::new(base);

    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|erro| format!("não consegui iniciar o runtime da fila: {erro}"))?,
    );
    let fila = {
        let _guard = runtime.enter();
        Arc::new(
            poph_rag::EmbeddingQueue::start(Arc::clone(&embedder), QueueConfig::default(), None)
                .map_err(|erro| format!("não consegui iniciar a fila de embedding: {erro}"))?,
        )
    };
    let da_org = poph_rag::QueuedEmbedder::with_runtime(fila, org, identidade, runtime);
    let (indice, abertura) =
        Indice::abrir(store.rag_path(), Arc::new(da_org), ConfigIndice::default())
            .map_err(|erro| format!("não consegui abrir o índice: {erro}"))?;
    if abertura.limpo_por_divergencia {
        eprintln!(
            "[aviso] o índice foi limpo: os vetores eram de outro modelo ou de outro divisor de \
             trechos. Rode `poph kb reindex` para reindexar."
        );
    }
    Ok(BaseConhecimento::nova(store, Arc::new(indice)))
}
