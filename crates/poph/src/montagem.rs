//! Montagem do turno, compartilhada por `poph chat` e `poph serve` (HR-26h).
//!
//! As duas pontas usam a mesma peça: o motor (modelo, memória, ferramentas, base, produtos, Jev e
//! transferência), o processador de turno, os atores por conversa e o fluxo de entrada e saída.
//! Quem chama escolhe duas coisas: as chaves (o modelo é obrigatório; o Jev é opcional) e as
//! métricas, que o servidor publica no `/metrics` e o chat mantém próprias.

use std::sync::Arc;

use poph_agent::atores::{Atores, OpcoesAtores};
use poph_agent::base_conhecimento::BaseConhecimento;
use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::entrada_saida::{Fluxo, OpcoesFluxo};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasFluxo, MetricasMemoria, MetricasTurno};
use poph_agent::turno::{AjustesDoFluxo, Motor, OpcoesTurno, ProcessadorTurno, RegistroPadrao};
use poph_agent::{BuscadorBase, BuscadorProdutos, CatalogoProdutos, ConsultarBase, registro_com};
use poph_core::llm::{ClienteAntigravity, ConfigAntigravity, LimitadorLlm};
use poph_fluxos::cache::CacheRevisoes;
use poph_store::Store;
use poph_types::{HarnessConfig, MetricasBase, Registro, Segredo};

/// Chaves que a montagem usa. A do modelo é obrigatória; a do Jev só liga o Jev quando existe.
pub struct Chaves {
    /// Chave do provedor de LLM (`ANTIGRAVITY_API_KEY`).
    pub llm: Segredo,
    /// Chave do Jev (`TYPESAFE_API_KEY`), quando há.
    pub jev: Option<Segredo>,
}

/// Métricas que a montagem publica. O servidor passa as dele; o chat usa as próprias.
pub struct Metricas {
    base: MetricasBase,
    turno: MetricasTurno,
    fluxo: MetricasFluxo,
    memoria: MetricasMemoria,
}

impl Metricas {
    /// Métricas próprias do processo, que não aparecem em `/metrics` (o `poph chat`).
    pub fn proprias(base: MetricasBase) -> Self {
        Self {
            base,
            turno: MetricasTurno::proprio(),
            fluxo: MetricasFluxo::proprio(),
            memoria: MetricasMemoria::proprio(),
        }
    }

    /// Métricas publicadas no registro dado (o `poph serve` usa o registro do `/metrics`).
    pub fn registradas(base: MetricasBase, registro: &Registro) -> Result<Self, String> {
        Ok(Self {
            base,
            turno: MetricasTurno::registrar(registro)
                .map_err(|erro| format!("métricas do turno: {erro}"))?,
            fluxo: MetricasFluxo::registrar(registro)
                .map_err(|erro| format!("métricas da fila: {erro}"))?,
            memoria: MetricasMemoria::registrar(registro)
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
    g9: Option<poph_agent::metricas::MetricasG9>,
}

impl FluxosDoProcesso {
    /// Os do servidor: o cache da API de fluxos e os contadores que o `/metrics` publica.
    pub fn do_servidor(estado: &poph_server::Estado) -> Self {
        let g9 = estado
            .metricas_fluxos()
            .map(|metricas| poph_agent::metricas::MetricasG9 {
                iniciou: metricas.g9_iniciou.clone(),
                nenhum: metricas.g9_nenhum.clone(),
                abaixo_do_limiar: metricas.g9_abaixo_do_limiar.clone(),
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
    pub motor: Arc<Motor>,
    /// Fluxo de entrada e saída, com o processador de turno ligado.
    pub fluxo: Fluxo,
    /// Base de conhecimento (RAG). `None` quando o índice não abre: o turno roda sem RAG.
    pub base: Option<Arc<BaseConhecimento>>,
    /// Catálogo de produtos, no mesmo índice da base.
    pub catalogo: Option<Arc<CatalogoProdutos>>,
}

/// Ferramentas que dependem da base de conhecimento (índice de trechos + modelo de embedding).
///
/// Sem o índice aberto elas não são registradas nem oferecidas (HR-46f). A lista existe para o
/// aviso de subida dizer exatamente o que ficou de fora.
const FERRAMENTAS_DA_BASE: [&str; 2] = [
    poph_agent::base_conhecimento::NOME_FERRAMENTA,
    poph_agent::produtos::NOME_FERRAMENTA,
];

/// Ferramentas que só usam o banco: carrinho de orçamento (HR-19) e envio de arquivo (HR-24).
///
/// Elas não dependem do índice de embedding, então valem mesmo numa instalação sem
/// `poph model install` — é o que garante que o agente continua podendo enviar arquivo (HR-46f).
fn ferramentas_do_banco(store: Arc<Store>) -> RegistroPadrao {
    let registro = poph_agent::carrinho::registrar(RegistroPadrao::novo(), Arc::clone(&store));
    poph_agent::biblioteca::registrar(registro, store)
}

/// O registro que o motor recebe: o banco sempre; `consultar_base` e `buscar_produto` só com a base.
///
/// Com `base` presente a composição é a mesma de antes da HR-46f: `consultar_base`, `buscar_produto`,
/// as quatro do carrinho e `enviar_arquivo`. Sem ela, ficam as do banco.
fn registro_do_turno(
    store: Arc<Store>,
    quente: Arc<ConfigQuente>,
    base: Option<(&Arc<BaseConhecimento>, &Arc<CatalogoProdutos>)>,
) -> RegistroPadrao {
    let Some((base, catalogo)) = base else {
        return ferramentas_do_banco(store);
    };
    let busca: Arc<dyn BuscadorBase> = Arc::clone(base) as Arc<dyn BuscadorBase>;
    let produtos: Arc<dyn BuscadorProdutos> = Arc::clone(catalogo) as Arc<dyn BuscadorProdutos>;
    let registro = ConsultarBase::registro(Arc::clone(&quente), busca);
    registro_com(registro, quente, produtos)
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
) -> Result<Montagem, String> {
    let cliente = ConfigAntigravity::da_config(&configuracao.llm, chaves.llm)
        .map_err(|erro| format!("configuração do provedor: {erro}"))?;
    // O Jev (HR-28) usa o mesmo catálogo de métricas; a cópia é feita antes de entregar ao provedor.
    let metricas_jev = metricas.base.clone();
    let limites_llm = LimitadorLlm::new(
        configuracao.llm.simultaneos as usize,
        configuracao.llm.simultaneos as usize,
    )
    .map_err(|erro| format!("limites do provedor: {erro}"))?;
    let provedor = Arc::new(
        ClienteAntigravity::new(cliente, limites_llm, metricas.base)
            .map_err(|erro| format!("cliente do provedor: {erro}"))?,
    );

    // Base de conhecimento (HR-17): sem o modelo instalado, o turno continua, só sem RAG.
    let base = match crate::rag::abrir_base(configuracao, Arc::clone(&store)) {
        Ok(base) => Some(base),
        Err(erro) => {
            eprintln!("[aviso] busca na base desligada: {erro}");
            None
        }
    };

    let mut motor = Motor::novo(
        Arc::clone(&store),
        ConfigQuente::nova(
            Arc::clone(&store),
            OpcoesConfigQuente::default(),
            metricas.memoria.clone(),
        ),
        Arc::new(Limitador::novo(
            OpcoesLimites::da_config(&configuracao.limites, &configuracao.llm),
            metricas.memoria.clone(),
        )),
        provedor,
        OpcoesTurno::default(),
        metricas.turno,
    )
    .com_modelo_padrao(configuracao.llm.modelo.clone())
    // Arquivos do cliente (HR-23b): pasta dos blobs e limites, para a imagem do anexo chegar ao
    // modelo como parte multimodal. Sem isto, só o texto extraído entra no prompt.
    .com_arquivos(poph_files::ConfigArquivos::padrao(configuracao.blobs()))
    // Transferência para humano (HR-29): textos da orientação e mensagem automática vêm do
    // `[handoff]` do `poph.toml` — trocar o texto não exige recompilar.
    .com_handoff(poph_jev::Orientacao::da_config(&configuracao.handoff))
    // Fluxos e Fichas (FX-23): o turno escolhe (G9), valida e inicia a execução na Ficha. Desligado
    // em `[fluxos] ativo`, nada disso acontece e a Ficha continua.
    .com_fluxos(&configuracao.fluxos);
    // O que o processo já tem do módulo é reaproveitado: mesmo cache de revisões da API e os
    // mesmos contadores `poph_fluxos_g9_*` do `/metrics`.
    if let Some(cache) = fluxos.cache.clone() {
        motor = motor.com_cache_de_fluxos(cache);
    }
    if let Some(g9) = fluxos.g9 {
        motor = motor.com_metricas_g9(g9);
    }
    // Jev (HR-28): só entra quando está ligado na configuração e a chave está no ambiente. Sem ele,
    // o turno roda exatamente como antes da HR-28 (nenhum gate é perguntado).
    if configuracao.jev.ativo {
        match chaves.jev {
            None => eprintln!(
                "[aviso] Jev ligado na configuração, mas sem TYPESAFE_API_KEY: seguindo sem Jev"
            ),
            Some(chave) => match poph_jev::ConfigJev::da_config(&configuracao.jev, chave)
                .and_then(|cliente| poph_jev::ClienteJev::novo(cliente, metricas_jev))
            {
                Ok(transporte) => {
                    let avaliador = poph_jev::MotorJev::novo(
                        Arc::new(transporte),
                        configuracao.jev.gates,
                        configuracao.jev.modelo.clone(),
                    );
                    motor = motor.com_jev(Arc::new(avaliador));
                }
                Err(erro) => eprintln!("[aviso] Jev desligado: {erro}"),
            },
        }
    }
    // Catálogo de produtos (HR-18): mesmo índice, ferramenta `buscar_produto`.
    let catalogo = base
        .as_ref()
        .map(|base| CatalogoProdutos::novo(Arc::clone(&store), Arc::clone(base.indice())));
    // HR-46f: o registro de ferramentas **não** depende da base de conhecimento. As ferramentas que
    // só usam o banco (`enviar_arquivo` e o carrinho) entram sempre; `consultar_base` e
    // `buscar_produto` só quando o índice abriu. Antes, tudo era registrado dentro do bloco da base:
    // numa instalação sem `poph model install` o pedido ao modelo saía com `tools: []`, o
    // `enviar_arquivo` não existia e o sintoma era silencioso (tudo terminava `done`).
    let registro = registro_do_turno(
        Arc::clone(&store),
        Arc::clone(motor.config_quente()),
        base.as_ref().zip(catalogo.as_ref()),
    );
    // FX-24: as três ferramentas do fluxo (ação, abandono e o início da reserva). Registradas só
    // com `[fluxos] ativo = true`, com o mesmo cache de revisões da API quando o processo tem um.
    let ajustes_do_fluxo = match fluxos.cache.clone() {
        Some(cache) => AjustesDoFluxo::da_config(&configuracao.fluxos).com_cache(cache),
        None => AjustesDoFluxo::da_config(&configuracao.fluxos),
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
            let busca: Arc<dyn BuscadorBase> = Arc::clone(base) as Arc<dyn BuscadorBase>;
            let produtos: Arc<dyn BuscadorProdutos> =
                Arc::clone(catalogo) as Arc<dyn BuscadorProdutos>;
            // HR-33: a mesma busca de produtos alimenta o G5 (candidatos do turno), além da
            // ferramenta `buscar_produto`.
            motor = motor.com_produtos(produtos).com_busca(busca);
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
    motor = motor.com_ferramentas(Arc::new(registro));
    let motor = Arc::new(motor);
    let processador = Arc::new(ProcessadorTurno::novo(Arc::clone(&motor)));
    let atores = Atores::novo(
        processador.clone(),
        OpcoesAtores::default(),
        metricas.memoria,
    );
    // Falha do modelo (HR-26i): a política vem do `[turno]` do `poph.toml`.
    let opcoes_fluxo = OpcoesFluxo::default().com_falha_modelo(
        configuracao.turno.tentativas_falha_modelo,
        std::time::Duration::from_millis(configuracao.turno.atraso_falha_modelo_ms),
    );
    let fluxo = Fluxo::com_metricas(Arc::clone(&store), atores, opcoes_fluxo, metricas.fluxo);
    processador.ligar_fluxo(fluxo.clone());

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

    use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
    use poph_agent::metricas::MetricasMemoria;
    use poph_rag::{ConfigIndice, Embedder, Indice};

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
            Store::abrir_em(
                &pasta.join("dados").join("harness.db"),
                &pasta.join("dados").join("rag.db"),
            )
            .expect("abre o banco de teste"),
        )
    }

    fn quente(store: &Arc<Store>) -> Arc<ConfigQuente> {
        Arc::new(ConfigQuente::nova(
            Arc::clone(store),
            OpcoesConfigQuente::default(),
            MetricasMemoria::proprio(),
        ))
    }

    /// Base e catálogo de teste sobre um índice com embedder falso: nada de modelo instalado.
    fn base_e_catalogo(
        store: &Arc<Store>,
        pasta: &std::path::Path,
    ) -> (Arc<BaseConhecimento>, Arc<CatalogoProdutos>) {
        let abrir = || {
            let embedder: Arc<dyn Embedder> = Arc::new(poph_rag::EmbedderFake::new());
            Indice::abrir(
                &pasta.join("dados").join("rag.db"),
                embedder,
                ConfigIndice::default(),
            )
            .expect("abre o índice de teste")
            .0
        };
        let base = BaseConhecimento::nova(Arc::clone(store), Arc::new(abrir()));
        let catalogo = CatalogoProdutos::novo(Arc::clone(store), Arc::new(abrir()));
        (base, catalogo)
    }

    #[test]
    fn sem_base_ficam_enviar_arquivo_e_carrinho_sem_consultar_base_nem_produto() {
        let pasta = pasta("sem-base");
        let store = store(&pasta);
        let nomes = ferramentas_do_banco(Arc::clone(&store)).nomes();
        // HR-46f: sem `poph model install` o modelo continua recebendo a ferramenta de arquivo.
        assert!(
            nomes.contains(&poph_agent::NOME_ENVIAR_ARQUIVO.to_owned()),
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
        let nomes = ferramentas_do_banco(Arc::clone(&store)).nomes();
        assert_eq!(
            nomes,
            vec![
                "cart_add_item".to_owned(),
                "cart_clear".to_owned(),
                "cart_remove_item".to_owned(),
                "cart_summary".to_owned(),
                poph_agent::NOME_ENVIAR_ARQUIVO.to_owned(),
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
        let nomes = registro.nomes();
        assert_eq!(
            nomes,
            vec![
                "buscar_produto".to_owned(),
                "cart_add_item".to_owned(),
                "cart_clear".to_owned(),
                "cart_remove_item".to_owned(),
                "cart_summary".to_owned(),
                "consultar_base".to_owned(),
                poph_agent::NOME_ENVIAR_ARQUIVO.to_owned(),
            ],
            "a lista com base ligada mudou; era a de antes da HR-46f"
        );
    }

    #[test]
    fn registro_do_turno_sem_base_nao_toca_no_indice() {
        let pasta = pasta("sem-base-helper");
        let store = store(&pasta);
        let nomes = registro_do_turno(Arc::clone(&store), quente(&store), None).nomes();
        assert_eq!(nomes.len(), 5, "lista sem base: {nomes:?}");
        assert!(nomes.contains(&poph_agent::NOME_ENVIAR_ARQUIVO.to_owned()));
        assert!(!nomes.contains(&"consultar_base".to_owned()));
    }
}
