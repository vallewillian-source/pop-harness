//! Tetos de fluxo vêm da configuração `[fluxos]`, pelo caminho do servidor (FX-26b).
//!
//! Os testes passam pelo `Estado` (a porta de compilação e a do cache de revisões), e não pelo
//! compilador direto: o que se prova é que o teto que vale é o da configuração do processo.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use poph_fluxos::{Code, FalhaCarregamento, Limites, RevisionKey, Severity, compilar};
use poph_server::Estado;
use poph_store::Store;
use poph_types::HarnessConfig;

/// Exemplo do repositório: script válido, que o compilador aceita com o padrão.
const FLUXO: &str = include_str!("../../../tests/fluxos/exemplos/agendar_visita.star");

/// Script de 40 KiB: o exemplo válido seguido de um comentário longo.
fn script_de_40_kib() -> String {
    let base = format!("{FLUXO}\n# ");
    let falta = 40 * 1024 - base.len() - 1;
    format!("{base}{}\n", "x".repeat(falta))
}

/// Estado do processo com a configuração dada, numa pasta própria deste teste.
fn estado(rotulo: &str, cfg: impl FnOnce(&mut HarnessConfig)) -> (Estado, PathBuf) {
    let pasta = std::env::temp_dir().join(format!(
        "poph-fluxos-limites-{rotulo}-{}",
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

fn com_teto_de_fonte(bytes: u64) -> impl FnOnce(&mut HarnessConfig) {
    move |config| config.flows.max_source_bytes = bytes
}

#[test]
fn servidor_aceita_40_kib_quando_a_configuracao_sobe_o_teto_para_64_kib() {
    let (estado, pasta) = estado("teto64", com_teto_de_fonte(65_536));
    let compilado = estado.compilar_fluxo(&script_de_40_kib());
    let _ = fs::remove_dir_all(&pasta);
    assert!(
        compilado.ir.is_some(),
        "com fluxos.max_fonte_bytes = 65536 o script de 40 KiB compila: {:?}",
        compilado.diagnosticos
    );
}

#[test]
fn servidor_recusa_40_kib_com_o_padrao_de_32_kib() {
    let (estado, pasta) = estado("padrao", |_| {});
    let compilado = estado.compilar_fluxo(&script_de_40_kib());
    let _ = fs::remove_dir_all(&pasta);
    assert!(compilado.ir.is_none());
    assert!(
        compilado
            .diagnosticos
            .iter()
            .any(|d| d.code == Code::SourceTooLarge),
        "padrão de 32 KiB: tem de sair FLX02"
    );
}

#[test]
fn servidor_recusa_40_kib_quando_a_configuracao_diz_32_kib() {
    let (estado, pasta) = estado("teto32", com_teto_de_fonte(32_768));
    let compilado = estado.compilar_fluxo(&script_de_40_kib());
    let _ = fs::remove_dir_all(&pasta);
    assert!(compilado.ir.is_none(), "script acima do teto não tem IR");
    assert!(
        compilado
            .diagnosticos
            .iter()
            .any(|d| d.code == Code::SourceTooLarge && d.severity == Severity::Error),
        "tem de sair FLX02: {:?}",
        compilado.diagnosticos
    );
}

#[test]
fn servidor_compila_o_exemplo_normal_com_o_teto_de_32_kib() {
    // O teto apertado não derruba o que cabe: o exemplo de ~2 KiB continua aceito.
    let (estado, pasta) = estado("exemplo32", com_teto_de_fonte(32_768));
    let compilado = estado.compilar_fluxo(FLUXO);
    let _ = fs::remove_dir_all(&pasta);
    assert!(compilado.ir.is_some(), "{:?}", compilado.diagnosticos);
}

#[test]
fn cache_carrega_40_kib_com_o_teto_padrao_e_recusa_com_32_kib() {
    let fonte = script_de_40_kib();
    // A revisão foi publicada com o teto de 64 KiB: a IR gravada vem dessa compilação.
    let publicacao = Limites {
        max_fonte_bytes: 65_536,
        ..Limites::default()
    };
    let ir = compilar(&fonte, &publicacao)
        .ir
        .expect("publicada com 64 KiB o script compila");
    let chave = RevisionKey::new("agendar_visita", 1, &fonte);

    let (teto64, pasta_64) = estado("cache-64", com_teto_de_fonte(65_536));
    let carregada = teto64.carregar_revisao_de_fluxo(&chave, &ir, &fonte);
    let _ = fs::remove_dir_all(&pasta_64);
    assert!(carregada.is_ok(), "carga com 64 KiB: {carregada:?}");

    let (apertado, pasta_apertado) = estado("cache-32", com_teto_de_fonte(32_768));
    let recusada = apertado.carregar_revisao_de_fluxo(&chave, &ir, &fonte);
    let _ = fs::remove_dir_all(&pasta_apertado);
    match recusada {
        Err(FalhaCarregamento::Forma(diagnosticos)) => assert!(
            diagnosticos.iter().any(|d| d.code == Code::SourceTooLarge),
            "a carga recusada tem de citar FLX02: {diagnosticos:?}"
        ),
        outro => panic!("com teto de 32 KiB a carga tem de falhar com FLX02, veio {outro:?}"),
    }
    assert_eq!(
        apertado.cache_fluxos.metrics().entries,
        0,
        "revisão recusada não entra no cache"
    );
}

#[test]
fn teto_de_nos_da_configuracao_vale_no_servidor() {
    let nos = {
        let ir = compilar(FLUXO, &Limites::default()).ir.expect("compila");
        ir.stages.len() + ir.decisions.len() + ir.ends.len()
    };
    let (apertado, pasta) = estado("nos", |config| {
        config.flows.max_nodes = u32::try_from(nos - 1).unwrap();
    });
    let compilado = apertado.compilar_fluxo(FLUXO);
    let _ = fs::remove_dir_all(&pasta);
    assert!(
        compilado
            .diagnosticos
            .iter()
            .any(|d| d.code == Code::NodeLimit),
        "um nó acima do teto da configuração tem de cair em FLX27"
    );
}
