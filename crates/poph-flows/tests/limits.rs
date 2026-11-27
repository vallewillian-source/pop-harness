use poph_flows::{Code, Limites, RuleLimits, Severity, compile, format_with};
use poph_types::config::FlowsConfig;

const EXAMPLE: &str = include_str!("../../../tests/fluxos/exemplos/agendar_visita.star");

fn script_size(size: usize) -> String {
    let base = format!("{EXAMPLE}\n# ");
    let missing = size.saturating_sub(base.len() + 1);
    format!("{base}{}\n", "x".repeat(missing))
}

fn nodes_of_example() -> usize {
    let ir = compile(EXAMPLE, &Limites::default())
        .ir
        .expect("o exemplo compila com o padrão");
    ir.stages.len() + ir.decisions.len() + ir.ends.len()
}

fn errors_with_code(fonte: &str, limites: &Limites, code: Code) -> bool {
    compile(fonte, limites)
        .diagnostics
        .iter()
        .any(|d| d.code == code && d.severity == Severity::Error)
}

#[test]
fn compiler_defaults_follow_default_configuration() {
    let default = FlowsConfig::default();
    let limites = Limites::default();
    assert_eq!(
        limites.max_fonte_bytes as u64, default.max_source_bytes,
        "tamanho do script: padrão do compilador e da configuração"
    );
    assert_eq!(
        limites.max_nos as u32, default.max_nodes,
        "nós por fluxo: padrão do compilador e da configuração"
    );
    let rule = RuleLimits::default();
    assert_eq!(rule.max_ticks, default.max_rule_ticks);
    assert_eq!(rule.max_heap_bytes as u64, default.max_rule_heap);
}

#[test]
fn from_config_maps_each_compiler_field() {
    let cfg = FlowsConfig {
        max_source_bytes: 40_960,
        max_nodes: 17,
        ..FlowsConfig::default()
    };
    let limites = Limites::from_config(&cfg);
    assert_eq!(limites.max_fonte_bytes, 40_960);
    assert_eq!(limites.max_nos, 17);
    let default = Limites::default();
    assert_eq!(limites.max_ticks, default.max_ticks);
    assert_eq!(limites.max_heap_bytes, default.max_heap_bytes);
    assert_eq!(limites.max_callstack, default.max_callstack);
    assert_eq!(limites.load_deadline, default.load_deadline);
}

#[test]
fn from_config_maps_rule_limits() {
    let cfg = FlowsConfig {
        max_rule_ticks: 2_500,
        max_rule_heap: 262_144,
        ..FlowsConfig::default()
    };
    let rule = RuleLimits::from_config(&cfg);
    assert_eq!(rule.max_ticks, 2_500);
    assert_eq!(rule.max_heap_bytes, 262_144);
    let default = RuleLimits::default();
    assert_eq!(rule.max_callstack, default.max_callstack);
    assert_eq!(rule.deadline, default.deadline);
}

#[test]
fn script_of_40_kib_passes_when_configuration_raises_cap_to_64_kib() {
    let fonte = script_size(40 * 1024);
    assert!(fonte.len() > 32 * 1024 && fonte.len() < 64 * 1024);
    let cfg = FlowsConfig {
        max_source_bytes: 65_536,
        ..FlowsConfig::default()
    };
    let limites = Limites::from_config(&cfg);
    assert!(
        compile(&fonte, &limites).ir.is_some(),
        "40 KiB com o teto de 64 KiB tem de compilar"
    );
}

#[test]
fn script_of_40_kib_refused_with_default() {
    let fonte = script_size(40 * 1024);
    let limites = Limites::from_config(&FlowsConfig::default());
    assert!(errors_with_code(&fonte, &limites, Code::SourceTooLarge));
}

#[test]
fn script_of_40_kib_refused_with_cap_of_32_kib() {
    let fonte = script_size(40 * 1024);
    let cfg = FlowsConfig {
        max_source_bytes: 32_768,
        ..FlowsConfig::default()
    };
    let limites = Limites::from_config(&cfg);
    assert!(
        errors_with_code(&fonte, &limites, Code::SourceTooLarge),
        "40 KiB com teto de 32 KiB tem de cair em FLX02"
    );
    assert!(compile(&fonte, &limites).ir.is_none());
}

#[test]
fn node_cap_from_configuration_refuses_what_exceeds_it() {
    let nodes = nodes_of_example();
    let aperto = Limites::from_config(&FlowsConfig {
        max_nodes: u32::try_from(nodes - 1).unwrap(),
        ..FlowsConfig::default()
    });
    assert!(
        errors_with_code(EXAMPLE, &aperto, Code::NodeLimit),
        "um nó a mais que o teto tem de cair em FLX27"
    );
    let limit = Limites::from_config(&FlowsConfig {
        max_nodes: u32::try_from(nodes).unwrap(),
        ..FlowsConfig::default()
    });
    assert!(
        compile(EXAMPLE, &limit).ir.is_some(),
        "no limite exato o fluxo compila"
    );
}

#[test]
fn format_with_uses_same_cap_as_compiler() {
    let fonte = script_size(40 * 1024);
    let cfg = FlowsConfig {
        max_source_bytes: 32_768,
        ..FlowsConfig::default()
    };
    assert!(format_with(&fonte, &Limites::from_config(&cfg)).is_err());
    let raised = FlowsConfig {
        max_source_bytes: 65_536,
        ..FlowsConfig::default()
    };
    assert!(format_with(&fonte, &Limites::from_config(&raised)).is_ok());
}
