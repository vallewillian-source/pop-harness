
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

fn estado(rotulo: &str, cfg: impl FnOnce(&mut HarnessConfig)) -> (ServerState, PathBuf) {
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
    let estado = ServerState::new(store, config).expect("estado do servidor");
    (estado, pasta)
}

async fn metricas_http(estado: ServerState) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = listener.local_addr().expect("endereço");
    let (desligar, sinal) = tokio::sync::oneshot::channel::<()>();
    let servidor = tokio::spawn(async move {
        let _ = poph_server::serve_at(listener, estado, async {
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

fn carregar(estado: &ServerState, fluxo: &str, numero: u32) {
    let limites = Limites::default();
    let compilado = compile(FLUXO, &limites);
    let ir = compilado
        .ir()
        .unwrap_or_else(|diagnosticos| panic!("exemplo não compila: {diagnosticos:?}"))
        .clone();
    let chave = RevisionKey::new(fluxo, numero, FLUXO);
    estado
        .flow_cache
        .load(&chave, &ir, FLUXO, &limites)
        .unwrap_or_else(|falha| panic!("carga falhou: {falha:?}"));
}

#[tokio::test]
async fn series_de_fluxos_aparecem_uma_vez_com_o_motor_ligado() {
    let (estado, pasta) = estado("ligado", |_| {});
    assert!(
        estado.flow_metrics().is_some(),
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
    assert_eq!(padrao.flow_cache.byte_cap(), 16 * 1024 * 1024);
    let _ = fs::remove_dir_all(&pasta_padrao);

    let (trocado, pasta_trocado) = estado("teto-3", |cfg| cfg.flows.revision_cache_mb = 3);
    assert_eq!(trocado.flow_cache.byte_cap(), 3 * 1024 * 1024);
    let _ = fs::remove_dir_all(&pasta_trocado);
}

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
        estado.flow_metrics().is_none(),
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
