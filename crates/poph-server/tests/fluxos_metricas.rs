use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_flows::{Limites, RevisionKey, compile};
use poph_server::ServerState;
use poph_store::Store;
use poph_types::HarnessConfig;

const FLUXO: &str = include_str!("../../../tests/fluxos/exemplos/agendar_visita.star");

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

fn estado(label: &str, cfg: impl FnOnce(&mut HarnessConfig)) -> (ServerState, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "poph-fluxos-metricas-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("dados")).expect("pasta de dados");
    let mut config = HarnessConfig::default();
    config.data.directory = dir.join("dados");
    cfg(&mut config);
    let store = Arc::new(Store::open(&config).expect("abre o banco"));
    let estado = ServerState::new(store, config).expect("estado do servidor");
    (estado, dir)
}

async fn metrics_http(estado: ServerState) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = listener.local_addr().expect("endereço");
    let (desligar, signal) = tokio::sync::oneshot::channel::<()>();
    let servidor = tokio::spawn(async move {
        let _ = poph_server::serve_at(listener, estado, async {
            let _ = signal.await;
        })
        .await;
    });
    let customer = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("cliente HTTP");
    let mut body = None;
    for _ in 0..100 {
        if let Ok(reply) = customer
            .get(format!("http://{endereco}/metrics"))
            .send()
            .await
        {
            body = Some(reply.text().await.expect("corpo do /metrics"));
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let _ = desligar.send(());
    let _ = servidor.await;
    body.expect("o /metrics respondeu")
}

fn load(estado: &ServerState, fluxo: &str, numero: u32) {
    let limites = Limites::default();
    let compiled = compile(FLUXO, &limites);
    let ir = compiled
        .ir()
        .unwrap_or_else(|diagnostics| panic!("exemplo não compila: {diagnostics:?}"))
        .clone();
    let chave = RevisionKey::new(fluxo, numero, FLUXO);
    estado
        .flow_cache
        .load(&chave, &ir, FLUXO, &limites)
        .unwrap_or_else(|failure| panic!("carga falhou: {failure:?}"));
}

#[tokio::test]
async fn flow_series_appear_once_with_engine_on() {
    let (estado, dir) = estado("ligado", |_| {});
    assert!(
        estado.flow_metrics().is_some(),
        "motor ligado declara as métricas"
    );
    let texto = metrics_http(estado).await;
    let _ = fs::remove_dir_all(&dir);
    for nome in SERIES {
        assert_eq!(
            texto.matches(&format!("# HELP {nome} ")).count(),
            1,
            "{nome} deve aparecer exatamente uma vez no /metrics"
        );
    }
    for line in texto.lines().filter(|l| l.starts_with("poph_fluxos_")) {
        assert!(!line.contains('{'), "série com rótulo: {line}");
    }
}

#[tokio::test]
async fn revision_loaded_by_engine_appears_in_cache_series() {
    let (estado, dir) = estado("cache", |_| {});
    load(&estado, "agendar_visita", 1);
    load(&estado, "agendar_visita", 1);
    let texto = metrics_http(estado).await;
    let _ = fs::remove_dir_all(&dir);
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
async fn cache_cap_comes_from_config() {
    let (standard, dir_standard) = estado("teto-padrao", |_| {});
    assert_eq!(standard.flow_cache.byte_cap(), 16 * 1024 * 1024);
    let _ = fs::remove_dir_all(&dir_standard);

    let (changed, dir_changed) = estado("teto-3", |cfg| cfg.flows.revision_cache_mb = 3);
    assert_eq!(changed.flow_cache.byte_cap(), 3 * 1024 * 1024);
    let _ = fs::remove_dir_all(&dir_changed);
}

#[tokio::test]
async fn cache_zero_does_not_store_and_counts_uncached() {
    let (estado, dir) = estado("teto-zero", |cfg| cfg.flows.revision_cache_mb = 0);
    load(&estado, "agendar_visita", 1);
    let texto = metrics_http(estado).await;
    let _ = fs::remove_dir_all(&dir);
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
async fn engine_off_declares_no_flow_series() {
    let (estado, dir) = estado("desligado", |cfg| cfg.flows.enabled = false);
    assert!(
        estado.flow_metrics().is_none(),
        "motor desligado não declara as métricas"
    );
    let texto = metrics_http(estado).await;
    let _ = fs::remove_dir_all(&dir);
    assert!(
        texto.contains("poph_memoria_processo_bytes"),
        "sanidade: o /metrics responde"
    );
    assert!(
        !texto.contains("poph_fluxos_"),
        "com o motor desligado, nenhuma série de fluxos:\n{texto}"
    );
}
