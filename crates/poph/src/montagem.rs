//! Montagem do turno, compartilhada por `poph chat` e `poph serve` (HR-26h).
//!
//! As duas pontas usam a mesma peça: o motor (modelo, memória, ferramentas, base, produtos, Jev e
//! transferência), o processador de turno, os atores por conversa e o fluxo de entrada e saída.
//! Quem chama escolhe duas coisas: as chaves (o modelo é obrigatório; o Jev é opcional) e as
//! métricas, que o servidor publica no `/metrics` e o chat mantém próprias.

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
use poph_fluxos::cache::CacheRevisoes;
use poph_store::Store;
use poph_types::{BaseMetrics, HarnessConfig, LlmConfig, Registry, Secret};

use crate::config;

/// Chaves que a montagem usa. A do modelo é obrigatória; a do Jev só liga o Jev quando existe.
pub struct Chaves {
    /// Chave do provedor de LLM (`ANTIGRAVITY_API_KEY`).
    pub llm: Secret,
    /// Chave do Jev (`TYPESAFE_API_KEY`), quando há.
    pub jev: Option<Secret>,
}

/// Métricas que a montagem publica. O servidor passa as dele; o chat usa as próprias.
pub struct Metricas {
    base: BaseMetrics,
    turno: MetricsTurn,
    fluxo: MetricsFlow,
    memoria: MetricsMemory,
}

impl Metricas {
    /// Métricas próprias do processo, que não aparecem em `/metrics` (o `poph chat`).
    pub fn proprias(base: BaseMetrics) -> Self {
        Self {
            base,
            turno: MetricsTurn::own(),
            fluxo: MetricsFlow::own(),
            memoria: MetricsMemory::own(),
        }
    }

    /// Métricas publicadas no registro dado (o `poph serve` usa o registro do `/metrics`).
    pub fn registradas(base: BaseMetrics, registro: &Registry) -> Result<Self, String> {
        Ok(Self {
            base,
            turno: MetricsTurn::register(registro)
                .map_err(|erro| format!("métricas do turno: {erro}"))?,
            fluxo: MetricsFlow::register(registro)
                .map_err(|erro| format!("métricas da fila: {erro}"))?,
            memoria: MetricsMemory::register(registro)
                .map_err(|erro| format!("métricas de memória: {erro}"))?,
        })
    }
}

/// O que o processo já tem do módulo de fluxos e o turno deve **reaproveitar** (FX-23).
///
/// O `/metrics` e a API de fluxos do servidor já têm o cache de revisões e os contadores do G9. O
/// turno usa os mesmos: um cache só por processo (nenhuma revisão compilada duas vezes) e as séries
/// `poph_fluxos_g9_*` publicadas são as que o turno incrementa. O `poph chat` monta sem nada disso e
/// o turno cria o próprio cache e os próprios contadores.
#[derive(Default)]
pub struct FluxosDoProcesso {
    cache: Option<Arc<CacheRevisoes>>,
    g9: Option<poph_agent::metrics::MetricsG9>,
}

impl FluxosDoProcesso {
    /// Os do servidor: o cache da API de fluxos e os contadores que o `/metrics` publica.
    pub fn do_servidor(estado: &poph_server::Estado) -> Self {
        let g9 = estado
            .metricas_fluxos()
            .map(|metricas| poph_agent::metrics::MetricsG9 {
                start: metricas.g9_iniciou.clone(),
                none: metricas.g9_nenhum.clone(),
                below_threshold: metricas.g9_abaixo_do_limiar.clone(),
                fallback: metricas.g9_fallback.clone(),
            });
        Self {
            cache: Some(Arc::clone(&estado.cache_fluxos)),
            g9,
        }
    }
}

/// O que a montagem entrega a quem chama.
pub struct Montagem {
    /// Motor do turno (usado direto pelo chat).
    pub motor: Arc<TurnEngine>,
    /// Fluxo de entrada e saída, com o processador de turno ligado.
    pub fluxo: Fluxo,
    /// Base de conhecimento (RAG). `None` quando o índice não abre: o turno roda sem RAG.
    pub base: Option<Arc<KnowledgeBase>>,
    /// Catálogo de produtos, no mesmo índice da base.
    pub catalogo: Option<Arc<ProductCatalog>>,
}

/// Ferramentas que dependem da base de conhecimento (índice de trechos + modelo de embedding).
///
/// Sem o índice aberto elas não são registradas nem oferecidas (HR-46f). A lista existe para o
/// aviso de subida dizer exatamente o que ficou de fora.
const FERRAMENTAS_DA_BASE: [&str; 2] = [
    poph_agent::base_knowledge::NAME_TOOL,
    poph_agent::products::NAME_TOOL,
];

/// Ferramentas que só usam o banco: carrinho de orçamento (HR-19) e envio de arquivo (HR-24).
///
/// Elas não dependem do índice de embedding, então valem mesmo numa instalação sem
/// `poph model install` — é o que garante que o agente continua podendo enviar arquivo (HR-46f).
fn ferramentas_do_banco(store: Arc<Store>) -> DefaultRegistry {
    let registro = poph_agent::cart::register(DefaultRegistry::new(), Arc::clone(&store));
    poph_agent::library::register(registro, store)
}

/// O registro que o motor recebe: o banco sempre; `consultar_base` e `buscar_produto` só com a base.
///
/// Com `base` presente a composição é a mesma de antes da HR-46f: `consultar_base`, `buscar_produto`,
/// as quatro do carrinho e `enviar_arquivo`. Sem ela, ficam as do banco.
fn registro_do_turno(
    store: Arc<Store>,
    quente: Arc<ConfigHot>,
    base: Option<(&Arc<KnowledgeBase>, &Arc<ProductCatalog>)>,
) -> DefaultRegistry {
    let Some((base, catalogo)) = base else {
        return ferramentas_do_banco(store);
    };
    let busca: Arc<dyn KnowledgeSearcher> = Arc::clone(base) as Arc<dyn KnowledgeSearcher>;
    let produtos: Arc<dyn ProductSearcher> = Arc::clone(catalogo) as Arc<dyn ProductSearcher>;
    let registro = QueryKnowledge::registro(Arc::clone(&quente), busca);
    record_with(registro, quente, produtos)
}

fn cliente_llm(
    llm: &LlmConfig,
    chave: Secret,
    metricas: BaseMetrics,
    rotulo: &str,
) -> Result<Arc<dyn LlmProvider>, String> {
    let cliente = OpenAiCompatibleConfig::from_config(llm, chave)
        .map_err(|erro| format!("configuração do provedor {rotulo}: {erro}"))?;
    let limites_llm = LlmLimiter::new(llm.concurrent_calls as usize, llm.concurrent_calls as usize)
        .map_err(|erro| format!("limites do provedor {rotulo}: {erro}"))?;
    let cliente = OpenAiCompatibleClient::new(cliente, limites_llm, metricas)
        .map_err(|erro| format!("cliente do provedor {rotulo}: {erro}"))?;
    Ok(Arc::new(cliente))
}

fn registro_de_provedores(
    configuracao: &HarnessConfig,
    ambiente: &config::Ambiente,
    metricas: BaseMetrics,
    padrao: &Arc<dyn LlmProvider>,
) -> Result<(Providers, BTreeMap<String, u32>), String> {
    let padrao = ResolvedProvider::new(
        configuracao.llm.provider.as_str(),
        Arc::clone(padrao),
        configuracao.llm.model.clone(),
    );
    let mut nomeados = BTreeMap::new();
    let mut tetos = BTreeMap::new();
    for (nome, sobreposicao) in &configuracao.providers {
        let efetivo = sobreposicao.effective(&configuracao.llm);
        let Some(chave) = config::chave_llm(ambiente, &efetivo.api_key_env) else {
            eprintln!(
                "[aviso] provedor `{nome}` sem a variável {} no ambiente: agentes que apontam para ele não atendem",
                efetivo.api_key_env
            );
            continue;
        };
        let cliente = cliente_llm(&efetivo, chave, metricas.clone(), &format!("`{nome}`"))?;
        tetos.insert(nome.clone(), efetivo.concurrent_calls);
        nomeados.insert(
            nome.clone(),
            ResolvedProvider::new(nome.as_str(), cliente, efetivo.model.clone()),
        );
    }
    Ok((Providers::new(padrao, nomeados), tetos))
}

/// Monta o turno completo a partir da configuração e do banco.
///
/// Os avisos de base e de Jev vão para o erro padrão, como sempre foram no chat. Erro devolvido é
/// só de configuração do provedor ou de métricas.
pub fn montar(
    configuracao: &HarnessConfig,
    store: Arc<Store>,
    chaves: Chaves,
    metricas: Metricas,
    fluxos: FluxosDoProcesso,
    ambiente: &config::Ambiente,
) -> Result<Montagem, String> {
    // O Jev (HR-28) usa o mesmo catálogo de métricas; a cópia é feita antes de entregar ao provedor.
    let metricas_jev = metricas.base.clone();
    let provedor = cliente_llm(
        &configuracao.llm,
        chaves.llm,
        metricas.base.clone(),
        "padrão",
    )?;
    let (provedores, tetos) =
        registro_de_provedores(configuracao, ambiente, metricas.base, &provedor)?;

    // Base de conhecimento (HR-17): sem o modelo instalado, o turno continua, só sem RAG.
    let base = match crate::rag::abrir_base(configuracao, Arc::clone(&store)) {
        Ok(base) => Some(base),
        Err(erro) => {
            eprintln!("[aviso] busca na base desligada: {erro}");
            None
        }
    };

    let http_tools =
        poph_agent::turno::HttpToolRegistry::from_config(&configuracao.http_tools, |name| {
            config::chave_llm(ambiente, name)
        })?;
    let mut motor = TurnEngine::new(
        Arc::clone(&store),
        ConfigHot::new(
            Arc::clone(&store),
            HotConfigOptions::default(),
            metricas.memoria.clone(),
        ),
        Arc::new(Limiter::new(
            LimitOptions::of_config(&configuracao.limits, &configuracao.llm)
                .with_cap_by_provider(tetos),
            metricas.memoria.clone(),
        )),
        provedor,
        TurnOptions::default(),
        metricas.turno,
    )
    .with_http_tools(http_tools)
    .with_default_model(configuracao.llm.model.clone())
    .with_providers(Arc::new(provedores))
    // Arquivos do cliente (HR-23b): pasta dos blobs e limites, para a imagem do anexo chegar ao
    // modelo como parte multimodal. Sem isto, só o texto extraído entra no prompt.
    .with_files(poph_files::FileConfig::default(configuracao.blobs()))
    // Transferência para humano (HR-29): textos da orientação e mensagem automática vêm do
    // `[handoff]` do `poph.toml` — trocar o texto não exige recompilar.
    .with_handoff(poph_jev::HandoffGuidance::from_config(
        &configuracao.handoff,
    ))
    // Fluxos e Fichas (FX-23): o turno escolhe (G9), valida e inicia a execução na Ficha. Desligado
    // em `[fluxos] ativo`, nada disso acontece e a Ficha continua.
    .with_flows(&configuracao.flows);
    // O que o processo já tem do módulo é reaproveitado: mesmo cache de revisões da API e os
    // mesmos contadores `poph_fluxos_g9_*` do `/metrics`.
    if let Some(cache) = fluxos.cache.clone() {
        motor = motor.with_flow_cache(cache);
    }
    if let Some(g9) = fluxos.g9 {
        motor = motor.with_g9_metrics(g9);
    }
    // Jev (HR-28): só entra quando está ligado na configuração e a chave está no ambiente. Sem ele,
    // o turno roda exatamente como antes da HR-28 (nenhum gate é perguntado).
    if configuracao.jev.enabled {
        match chaves.jev {
            None => eprintln!(
                "[aviso] Jev ligado na configuração, mas sem TYPESAFE_API_KEY: seguindo sem Jev"
            ),
            Some(chave) => match poph_jev::JevClientConfig::from_config(&configuracao.jev, chave)
                .and_then(|cliente| poph_jev::JevClient::new(cliente, metricas_jev))
            {
                Ok(transporte) => {
                    let avaliador = poph_jev::JevEngine::new(
                        Arc::new(transporte),
                        configuracao.jev.gates,
                        configuracao.jev.model.clone(),
                    );
                    motor = motor.with_jev(Arc::new(avaliador));
                }
                Err(erro) => eprintln!("[aviso] Jev desligado: {erro}"),
            },
        }
    }
    // Catálogo de produtos (HR-18): mesmo índice, ferramenta `buscar_produto`.
    let catalogo = base
        .as_ref()
        .map(|base| ProductCatalog::new(Arc::clone(&store), Arc::clone(base.index())));
    // HR-46f: o registro de ferramentas **não** depende da base de conhecimento. As ferramentas que
    // só usam o banco (`enviar_arquivo` e o carrinho) entram sempre; `consultar_base` e
    // `buscar_produto` só quando o índice abriu. Antes, tudo era registrado dentro do bloco da base:
    // numa instalação sem `poph model install` o pedido ao modelo saía com `tools: []`, o
    // `enviar_arquivo` não existia e o sintoma era silencioso (tudo terminava `done`).
    let registro = registro_do_turno(
        Arc::clone(&store),
        Arc::clone(motor.hot_config()),
        base.as_ref().zip(catalogo.as_ref()),
    );
    // FX-24: as três ferramentas do fluxo (ação, abandono e o início da reserva). Registradas só
    // com `[fluxos] ativo = true`, com o mesmo cache de revisões da API quando o processo tem um.
    let ajustes_do_fluxo = match fluxos.cache.clone() {
        Some(cache) => AdjustmentsFlow::of_config(&configuracao.flows).with_cache(cache),
        None => AdjustmentsFlow::of_config(&configuracao.flows),
    };
    let registro = poph_agent::turno::registrar_ferramentas_do_fluxo_com(
        registro,
        Arc::clone(&store),
        ajustes_do_fluxo,
    );
    let registro =
        poph_agent::turno::registrar_ferramenta_encerramento(registro, Arc::clone(&store));
    match (&base, &catalogo) {
        (Some(base), Some(catalogo)) => {
            let busca: Arc<dyn KnowledgeSearcher> = Arc::clone(base) as Arc<dyn KnowledgeSearcher>;
            let produtos: Arc<dyn ProductSearcher> =
                Arc::clone(catalogo) as Arc<dyn ProductSearcher>;
            // HR-33: a mesma busca de produtos alimenta o G5 (candidatos do turno), além da
            // ferramenta `buscar_produto`.
            motor = motor.with_products(produtos).with_search(busca);
        }
        _ => {
            // Aviso uma vez, na subida: sem base, estas ferramentas que dependem do índice não
            // são oferecidas a nenhum agente. O resto (banco) continua valendo.
            eprintln!(
                "[aviso] {} ferramentas dependem da base de conhecimento e não serão oferecidas sem ela: {}",
                FERRAMENTAS_DA_BASE.len(),
                FERRAMENTAS_DA_BASE.join(", ")
            );
        }
    }
    motor = motor.with_tools(Arc::new(registro));
    let motor = Arc::new(motor);
    let processador = Arc::new(TurnProcessor::new(Arc::clone(&motor)));
    let atores = Actors::new(
        processador.clone(),
        ActorOptions::default(),
        metricas.memoria,
    );
    // Falha do modelo (HR-26i): a política vem do `[turno]` do `poph.toml`.
    let opcoes_fluxo = FlowOptions::default().with_failure_model(
        configuracao.turn.model_failure_attempts,
        std::time::Duration::from_millis(configuracao.turn.model_failure_delay_ms),
    );
    let fluxo = Fluxo::with_metrics(Arc::clone(&store), atores, opcoes_fluxo, metricas.fluxo);
    processador.attach_io(fluxo.clone());

    Ok(Montagem {
        motor,
        fluxo,
        base,
        catalogo,
    })
}

#[cfg(test)]
mod testes {
    use std::path::PathBuf;

    use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
    use poph_agent::metrics::MetricsMemory;
    use poph_rag::{Embedder, Index, IndexConfig};

    use super::*;

    /// Pasta descartável por teste.
    fn pasta(rotulo: &str) -> PathBuf {
        let caminho =
            std::env::temp_dir().join(format!("poph-montagem-{rotulo}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&caminho);
        std::fs::create_dir_all(&caminho).expect("cria a pasta de teste");
        caminho
    }

    fn store(pasta: &std::path::Path) -> Arc<Store> {
        Arc::new(
            Store::open_at(
                &pasta.join("dados").join("harness.db"),
                &pasta.join("dados").join("rag.db"),
            )
            .expect("abre o banco de teste"),
        )
    }

    fn quente(store: &Arc<Store>) -> Arc<ConfigHot> {
        Arc::new(ConfigHot::new(
            Arc::clone(store),
            HotConfigOptions::default(),
            MetricsMemory::own(),
        ))
    }

    /// Base e catálogo de teste sobre um índice com embedder falso: nada de modelo instalado.
    fn base_e_catalogo(
        store: &Arc<Store>,
        pasta: &std::path::Path,
    ) -> (Arc<KnowledgeBase>, Arc<ProductCatalog>) {
        let abrir = || {
            let embedder: Arc<dyn Embedder> = Arc::new(poph_rag::FakeEmbedder::new());
            Index::open(
                &pasta.join("dados").join("rag.db"),
                embedder,
                IndexConfig::default(),
            )
            .expect("abre o índice de teste")
            .0
        };
        let base = KnowledgeBase::new(Arc::clone(store), Arc::new(abrir()));
        let catalogo = ProductCatalog::new(Arc::clone(store), Arc::new(abrir()));
        (base, catalogo)
    }

    #[test]
    fn sem_base_ficam_enviar_arquivo_e_carrinho_sem_consultar_base_nem_produto() {
        let pasta = pasta("sem-base");
        let store = store(&pasta);
        let nomes = ferramentas_do_banco(Arc::clone(&store)).names();
        // HR-46f: sem `poph model install` o modelo continua recebendo a ferramenta de arquivo.
        assert!(
            nomes.contains(&poph_agent::NAME_SEND_FILE.to_owned()),
            "enviar_arquivo fora da lista: {nomes:?}"
        );
        for carrinho in [
            "cart_add_item",
            "cart_remove_item",
            "cart_summary",
            "cart_clear",
        ] {
            assert!(
                nomes.contains(&carrinho.to_owned()),
                "{carrinho} fora da lista: {nomes:?}"
            );
        }
        for dependente in FERRAMENTAS_DA_BASE {
            assert!(
                !nomes.contains(&dependente.to_owned()),
                "{dependente} não deveria existir sem base: {nomes:?}"
            );
        }
    }

    #[test]
    fn sem_base_o_motor_nao_recebe_a_ferramenta_de_arquivo_duplicada() {
        // O registro sem base tem exatamente as cinco ferramentas do banco (4 do carrinho + arquivo).
        let pasta = pasta("sem-base-contagem");
        let store = store(&pasta);
        let nomes = ferramentas_do_banco(Arc::clone(&store)).names();
        assert_eq!(
            nomes,
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
    fn com_base_a_lista_continua_a_de_antes_da_hr_46f() {
        let pasta = pasta("com-base");
        let store = store(&pasta);
        let (base, catalogo) = base_e_catalogo(&store, &pasta);
        let registro =
            registro_do_turno(Arc::clone(&store), quente(&store), Some((&base, &catalogo)));
        let nomes = registro.names();
        assert_eq!(
            nomes,
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
    fn registro_do_turno_sem_base_nao_toca_no_indice() {
        let pasta = pasta("sem-base-helper");
        let store = store(&pasta);
        let nomes = registro_do_turno(Arc::clone(&store), quente(&store), None).names();
        assert_eq!(nomes.len(), 5, "lista sem base: {nomes:?}");
        assert!(nomes.contains(&poph_agent::NAME_SEND_FILE.to_owned()));
        assert!(!nomes.contains(&"consultar_base".to_owned()));
    }
}
