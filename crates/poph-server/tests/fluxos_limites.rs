use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use poph_flows::{Code, Limites, LoadFailure, RevisionKey, Severity, compile};
use poph_server::ServerState;
use poph_store::Store;
use poph_types::HarnessConfig;

const FLUXO: &str = include_str!("../../../tests/fluxos/exemplos/agendar_visita.star");

fn script_of_40_kib() -> String {
    let base = format!("{FLUXO}\n# ");
    let missing = 40 * 1024 - base.len() - 1;
    format!("{base}{}\n", "x".repeat(missing))
}

fn estado(label: &str, cfg: impl FnOnce(&mut HarnessConfig)) -> (ServerState, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "poph-fluxos-limites-{label}-{}",
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

fn with_source_cap(bytes: u64) -> impl FnOnce(&mut HarnessConfig) {
    move |config| config.flows.max_source_bytes = bytes
}

#[test]
fn server_accepts_40_kib_when_config_raises_cap_to_64_kib() {
    let (estado, dir) = estado("teto64", with_source_cap(65_536));
    let compiled = estado.compile_flow(&script_of_40_kib());
    let _ = fs::remove_dir_all(&dir);
    assert!(
        compiled.ir.is_some(),
        "com fluxos.max_fonte_bytes = 65536 o script de 40 KiB compila: {:?}",
        compiled.diagnostics
    );
}

#[test]
fn server_refuses_40_kib_with_default_32_kib() {
    let (estado, dir) = estado("padrao", |_| {});
    let compiled = estado.compile_flow(&script_of_40_kib());
    let _ = fs::remove_dir_all(&dir);
    assert!(compiled.ir.is_none());
    assert!(
        compiled
            .diagnostics
            .iter()
            .any(|d| d.code == Code::SourceTooLarge),
        "padrão de 32 KiB: tem de sair FLX02"
    );
}

#[test]
fn server_refuses_40_kib_when_config_says_32_kib() {
    let (estado, dir) = estado("teto32", with_source_cap(32_768));
    let compiled = estado.compile_flow(&script_of_40_kib());
    let _ = fs::remove_dir_all(&dir);
    assert!(compiled.ir.is_none(), "script acima do teto não tem IR");
    assert!(
        compiled
            .diagnostics
            .iter()
            .any(|d| d.code == Code::SourceTooLarge && d.severity == Severity::Error),
        "tem de sair FLX02: {:?}",
        compiled.diagnostics
    );
}

#[test]
fn server_compiles_normal_example_with_32_kib_cap() {
    let (estado, dir) = estado("exemplo32", with_source_cap(32_768));
    let compiled = estado.compile_flow(FLUXO);
    let _ = fs::remove_dir_all(&dir);
    assert!(compiled.ir.is_some(), "{:?}", compiled.diagnostics);
}

#[test]
fn cache_loads_40_kib_with_default_cap_and_refuses_32_kib() {
    let fonte = script_of_40_kib();
    let publication = Limites {
        max_fonte_bytes: 65_536,
        ..Limites::default()
    };
    let ir = compile(&fonte, &publication)
        .ir
        .expect("publicada com 64 KiB o script compila");
    let chave = RevisionKey::new("agendar_visita", 1, &fonte);

    let (cap64, dir_64) = estado("cache-64", with_source_cap(65_536));
    let loaded = cap64.load_flow_revision(&chave, &ir, &fonte);
    let _ = fs::remove_dir_all(&dir_64);
    assert!(loaded.is_ok(), "carga com 64 KiB: {loaded:?}");

    let (tight, dir_tight) = estado("cache-32", with_source_cap(32_768));
    let refused = tight.load_flow_revision(&chave, &ir, &fonte);
    let _ = fs::remove_dir_all(&dir_tight);
    match refused {
        Err(LoadFailure::Form(diagnostics)) => assert!(
            diagnostics.iter().any(|d| d.code == Code::SourceTooLarge),
            "a carga recusada tem de citar FLX02: {diagnostics:?}"
        ),
        another => panic!("com teto de 32 KiB a carga tem de falhar com FLX02, veio {another:?}"),
    }
    assert_eq!(
        tight.flow_cache.metrics().entries,
        0,
        "revisão recusada não entra no cache"
    );
}

#[test]
fn config_node_cap_applies_on_server() {
    let nodes = {
        let ir = compile(FLUXO, &Limites::default()).ir.expect("compila");
        ir.stages.len() + ir.decisions.len() + ir.ends.len()
    };
    let (tight, dir) = estado("nos", |config| {
        config.flows.max_nodes = u32::try_from(nodes - 1).unwrap();
    });
    let compiled = tight.compile_flow(FLUXO);
    let _ = fs::remove_dir_all(&dir);
    assert!(
        compiled
            .diagnostics
            .iter()
            .any(|d| d.code == Code::NodeLimit),
        "um nó acima do teto da configuração tem de cair em FLX27"
    );
}
