//! Métricas `poph_fluxos_*` e teto do cache de revisões no processo (FX-26).
//!
//! Servidor HTTP de verdade (o mesmo `servir_em` do teste de métricas do Matrix), para o `/metrics`.
//! O cache é carregado pelo caminho que o motor vai usar (`CacheRevisoes::carregar`), e o teto vem
//! da configuração `[fluxos]`, não de um número escrito no teste.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_flows::{Limites, RevisionKey, compile};
use poph_server::Estado;
use poph_store::Store;
use poph_types::HarnessConfig;

/// Um dos exemplos de fluxo do repositório: script válido, o mesmo que o compilador da FX-10 aceita.
const FLUXO: &str = include_str!("../../../tests/fluxos/exemplos/agendar_visita.star");

/// Todas as séries `poph_fluxos_*` do catálogo (FX-26).
const SERIES: [&str; 22] = [
    "poph_fluxos_cache_acertos_total",
    "poph_fluxos_cache_erros_total",
    "poph_fluxos_cache_cargas_total",
    "poph_fluxos_cache_esperas_total",
    "poph_fluxos_expulsoes_total",
    "poph_fluxos_cache_em_uso_poupadas_total",
    "poph_fluxos_cache_nao_cacheadas_total",
    "poph_fluxos_revisoes_em_cache",
    "poph_fluxos_revisoes_em_uso",
    "poph_fluxos_cache_bytes",
    "poph_fluxos_execucoes_iniciadas_total",
    "poph_fluxos_execucoes_concluidas_total",
    "poph_fluxos_execucoes_abandonadas_total",
    "poph_fluxos_execucoes_falhas_total",
    "poph_fluxos_g9_iniciou_total",
    "poph_fluxos_g9_nenhum_total",
    "poph_fluxos_g9_abaixo_do_limiar_total",
    "poph_fluxos_g9_fallback_total",
    "poph_fluxos_efeitos_pendentes_total",
    "poph_fluxos_efeitos_enviados_total",
    "poph_fluxos_efeitos_falhas_total",
    "poph_fluxos_erros_regra_total",
];

/// Estado do processo com a configuração dada, numa pasta própria do teste.
fn estado(rotulo: &str, cfg: impl FnOnce(&mut HarnessConfig)) -> (Estado, PathBuf) {
    let pasta = std::env::temp_dir().join(format!(
        "poph-fluxos-metricas-{rotulo}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");
    let mut config = HarnessConfig::default();
    config.data.directory = pasta.join("dados");
    cfg(&mut config);
    let store = Arc::new(Store::open(&config).expect("abre o banco"));
    let estado = Estado::novo(store, config).expect("estado do servidor");
    (estado, pasta)
}

/// Sobe o servidor com o estado dado e devolve o texto de `/metrics`.
async fn metricas_http(estado: Estado) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = listener.local_addr().expect("endereço");
    let (desligar, sinal) = tokio::sync::oneshot::channel::<()>();
    let servidor = tokio::spawn(async move {
        let _ = poph_server::servir_em(listener, estado, async {
            let _ = sinal.await;
        })
        .await;
    });
    let cliente = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("cliente HTTP");
    let mut corpo = None;
    for _ in 0..100 {
        if let Ok(resposta) = cliente
            .get(format!("http://{endereco}/metrics"))
            .send()
            .await
        {
            corpo = Some(resposta.text().await.expect("corpo do /metrics"));
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let _ = desligar.send(());
    let _ = servidor.await;
    corpo.expect("o /metrics respondeu")
}

/// Carrega a revisão pelo cache, como o motor faz: a IR vem da compilação do script.
fn carregar(estado: &Estado, fluxo: &str, numero: u32) {
    let limites = Limites::default();
    let compilado = compile(FLUXO, &limites);
    let ir = compilado
        .ir()
        .unwrap_or_else(|diagnosticos| panic!("exemplo não compila: {diagnosticos:?}"))
        .clone();
    let chave = RevisionKey::new(fluxo, numero, FLUXO);
    estado
        .cache_fluxos
        .load(&chave, &ir, FLUXO, &limites)
        .unwrap_or_else(|falha| panic!("carga falhou: {falha:?}"));
}

#[tokio::test]
async fn series_de_fluxos_aparecem_uma_vez_com_o_motor_ligado() {
    let (estado, pasta) = estado("ligado", |_| {});
    assert!(
        estado.metricas_fluxos().is_some(),
        "motor ligado declara as métricas"
    );
    let texto = metricas_http(estado).await;
    let _ = fs::remove_dir_all(&pasta);
    for nome in SERIES {
        assert_eq!(
            texto.matches(&format!("# HELP {nome} ")).count(),
            1,
            "{nome} deve aparecer exatamente uma vez no /metrics"
        );
    }
    // Sem rótulo nenhum: nada de id de Ficha, telefone ou texto na série.
    for linha in texto.lines().filter(|l| l.starts_with("poph_fluxos_")) {
        assert!(!linha.contains('{'), "série com rótulo: {linha}");
    }
}

#[tokio::test]
async fn revisao_carregada_pelo_motor_aparece_nas_series_do_cache() {
    let (estado, pasta) = estado("cache", |_| {});
    carregar(&estado, "agendar_visita", 1);
    carregar(&estado, "agendar_visita", 1);
    let texto = metricas_http(estado).await;
    let _ = fs::remove_dir_all(&pasta);
    assert!(
        texto.contains("poph_fluxos_cache_cargas_total 1\n"),
        "{texto}"
    );
    assert!(
        texto.contains("poph_fluxos_cache_acertos_total 1\n"),
        "{texto}"
    );
    assert!(
        texto.contains("poph_fluxos_revisoes_em_cache 1\n"),
        "{texto}"
    );
    assert!(
        !texto.contains("poph_fluxos_cache_bytes 0\n"),
        "o peso da revisão aparece"
    );
}

#[tokio::test]
async fn teto_do_cache_vem_da_configuracao() {
    let (padrao, pasta_padrao) = estado("teto-padrao", |_| {});
    assert_eq!(padrao.cache_fluxos.byte_cap(), 16 * 1024 * 1024);
    let _ = fs::remove_dir_all(&pasta_padrao);

    let (trocado, pasta_trocado) = estado("teto-3", |cfg| cfg.flows.revision_cache_mb = 3);
    assert_eq!(trocado.cache_fluxos.byte_cap(), 3 * 1024 * 1024);
    let _ = fs::remove_dir_all(&pasta_trocado);
}

/// Com `cache_revisoes_mb = 0` nada fica guardado: a carga conta como "não cacheada" e a execução segue.
#[tokio::test]
async fn cache_zero_nao_guarda_e_conta_nao_cacheada() {
    let (estado, pasta) = estado("teto-zero", |cfg| cfg.flows.revision_cache_mb = 0);
    carregar(&estado, "agendar_visita", 1);
    let texto = metricas_http(estado).await;
    let _ = fs::remove_dir_all(&pasta);
    assert!(
        texto.contains("poph_fluxos_cache_nao_cacheadas_total 1\n"),
        "{texto}"
    );
    assert!(
        texto.contains("poph_fluxos_revisoes_em_cache 0\n"),
        "{texto}"
    );
}

#[tokio::test]
async fn motor_desligado_nao_declara_nenhuma_serie_de_fluxos() {
    let (estado, pasta) = estado("desligado", |cfg| cfg.flows.enabled = false);
    assert!(
        estado.metricas_fluxos().is_none(),
        "motor desligado não declara as métricas"
    );
    let texto = metricas_http(estado).await;
    let _ = fs::remove_dir_all(&pasta);
    assert!(
        texto.contains("poph_memoria_processo_bytes"),
        "sanidade: o /metrics responde"
    );
    assert!(
        !texto.contains("poph_fluxos_"),
        "com o motor desligado, nenhuma série de fluxos:\n{texto}"
    );
}
