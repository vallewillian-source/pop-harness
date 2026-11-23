//! Os tetos de compilação e de avaliação de regra vêm da configuração `[fluxos]` (FX-26b).
//!
//! Cada chave com efeito no compilador ou na avaliação tem um teste que muda o valor e prova que
//! o limite de fato muda (`Limites::da_config`, `LimitesRegra::da_config`, `compilar`).

use poph_fluxos::{Codigo, Limites, LimitesRegra, Severidade, compilar, formatar_com};
use poph_types::config::FluxosConfig;

/// Um dos exemplos do repositório: script válido que o compilador aceita com o padrão.
const EXEMPLO: &str = include_str!("../../../tests/fluxos/exemplos/agendar_visita.star");

/// Script de `tamanho` bytes: o exemplo válido seguido de um comentário longo no fim.
fn script_de_tamanho(tamanho: usize) -> String {
    let base = format!("{EXEMPLO}\n# ");
    let falta = tamanho.saturating_sub(base.len() + 1);
    format!("{base}{}\n", "x".repeat(falta))
}

/// Número de nós (etapas + decisões + fins) da IR de um script aceito.
fn nos_do_exemplo() -> usize {
    let ir = compilar(EXEMPLO, &Limites::default())
        .ir
        .expect("o exemplo compila com o padrão");
    ir.etapas.len() + ir.decisoes.len() + ir.fins.len()
}

fn erros_com_codigo(fonte: &str, limites: &Limites, codigo: Codigo) -> bool {
    compilar(fonte, limites)
        .diagnosticos
        .iter()
        .any(|d| d.codigo == codigo && d.severidade == Severidade::Erro)
}

#[test]
fn padroes_do_compilador_seguem_a_configuracao_padrao() {
    let padrao = FluxosConfig::default();
    let limites = Limites::default();
    assert_eq!(
        limites.max_fonte_bytes as u64, padrao.max_fonte_bytes,
        "tamanho do script: padrão do compilador e da configuração"
    );
    assert_eq!(
        limites.max_nos as u32, padrao.max_nos,
        "nós por fluxo: padrão do compilador e da configuração"
    );
    let regra = LimitesRegra::default();
    assert_eq!(regra.max_ticks, padrao.max_ticks_regra);
    assert_eq!(regra.max_heap_bytes as u64, padrao.max_heap_regra);
}

#[test]
fn da_config_mapeia_cada_campo_do_compilador() {
    // Cada campo com chave em `[fluxos]` muda o valor que o compilador recebe.
    let cfg = FluxosConfig {
        max_fonte_bytes: 40_960,
        max_nos: 17,
        ..FluxosConfig::default()
    };
    let limites = Limites::da_config(&cfg);
    assert_eq!(limites.max_fonte_bytes, 40_960);
    assert_eq!(limites.max_nos, 17);
    // Campos sem chave em `[fluxos]` continuam nos valores da carga do módulo.
    let padrao = Limites::default();
    assert_eq!(limites.max_ticks, padrao.max_ticks);
    assert_eq!(limites.max_heap_bytes, padrao.max_heap_bytes);
    assert_eq!(limites.max_callstack, padrao.max_callstack);
    assert_eq!(limites.prazo_de_carga, padrao.prazo_de_carga);
}

#[test]
fn da_config_mapeia_os_limites_de_regra() {
    let cfg = FluxosConfig {
        max_ticks_regra: 2_500,
        max_heap_regra: 262_144,
        ..FluxosConfig::default()
    };
    let regra = LimitesRegra::da_config(&cfg);
    assert_eq!(regra.max_ticks, 2_500);
    assert_eq!(regra.max_heap_bytes, 262_144);
    let padrao = LimitesRegra::default();
    assert_eq!(regra.max_callstack, padrao.max_callstack);
    assert_eq!(regra.prazo, padrao.prazo);
}

#[test]
fn script_de_40_kib_passa_quando_a_configuracao_sobe_o_teto_para_64_kib() {
    let fonte = script_de_tamanho(40 * 1024);
    assert!(fonte.len() > 32 * 1024 && fonte.len() < 64 * 1024);
    let cfg = FluxosConfig {
        max_fonte_bytes: 65_536,
        ..FluxosConfig::default()
    };
    let limites = Limites::da_config(&cfg);
    assert!(
        compilar(&fonte, &limites).ir.is_some(),
        "40 KiB com o teto de 64 KiB tem de compilar"
    );
}

#[test]
fn script_de_40_kib_e_recusado_com_o_padrao() {
    // Padrão de `fluxos.max_fonte_bytes` = 32 KiB (decisão D1): quem quer mais sobe a configuração.
    let fonte = script_de_tamanho(40 * 1024);
    let limites = Limites::da_config(&FluxosConfig::default());
    assert!(erros_com_codigo(&fonte, &limites, Codigo::FonteGrande));
}

#[test]
fn script_de_40_kib_e_recusado_com_teto_de_32_kib() {
    let fonte = script_de_tamanho(40 * 1024);
    let cfg = FluxosConfig {
        max_fonte_bytes: 32_768,
        ..FluxosConfig::default()
    };
    let limites = Limites::da_config(&cfg);
    assert!(
        erros_com_codigo(&fonte, &limites, Codigo::FonteGrande),
        "40 KiB com teto de 32 KiB tem de cair em FLX02"
    );
    assert!(compilar(&fonte, &limites).ir.is_none());
}

#[test]
fn teto_de_nos_da_configuracao_recusa_o_que_passa_dele() {
    let nos = nos_do_exemplo();
    let aperto = Limites::da_config(&FluxosConfig {
        max_nos: u32::try_from(nos - 1).unwrap(),
        ..FluxosConfig::default()
    });
    assert!(
        erros_com_codigo(EXEMPLO, &aperto, Codigo::NosDemais),
        "um nó a mais que o teto tem de cair em FLX27"
    );
    let no_limite = Limites::da_config(&FluxosConfig {
        max_nos: u32::try_from(nos).unwrap(),
        ..FluxosConfig::default()
    });
    assert!(
        compilar(EXEMPLO, &no_limite).ir.is_some(),
        "no limite exato o fluxo compila"
    );
}

#[test]
fn formatar_com_usa_o_mesmo_teto_do_compilador() {
    let fonte = script_de_tamanho(40 * 1024);
    let cfg = FluxosConfig {
        max_fonte_bytes: 32_768,
        ..FluxosConfig::default()
    };
    assert!(formatar_com(&fonte, &Limites::da_config(&cfg)).is_err());
    let subido = FluxosConfig {
        max_fonte_bytes: 65_536,
        ..FluxosConfig::default()
    };
    assert!(formatar_com(&fonte, &Limites::da_config(&subido)).is_ok());
}
