use std::time::{Duration, Instant};

use poph_flows::compilador::Limites;
use poph_flows::ir::Rule;
use poph_flows::rules::{
    DEADLINE_TOLERANCE, Dados, MAX_NESTING, RULE_DEADLINE, RuleError, RuleEvaluator, RuleLimits,
    evaluate, snapshot_from_json,
};
use poph_types::DomainError;

const SCRIPT: &str = r#"
fluxo(
    id = "teste_de_regras",
    nome = "Teste das regras",
    inicio = "relato",
    gatilho = gatilho(quando = "o cliente relata um problema", exemplos = ["deu erro"]),
)

etapa(
    id = "relato",
    nome = "Relato",
    campos = [campo("resumo", "O que aconteceu", tipo = "texto")],
    acoes = [acao("ok", "Registrado", vai_para = fim("concluido"))],
)

def grave(dados):
    resumo = dados.get("relato", {}).get("resumo", "").lower()
    return "parado" in resumo

def viu_anexo(dados):
    return dados.get("relato", {}).get("anexo") != None

def devolve_texto(dados):
    return "sim"

def devolve_nada(dados):
    pass

def estoura_ticks(dados):
    total = 0
    for i in range(200000):
        total = total + i
    return total > 0

def le_ausente(dados):
    return dados["relato"]["nao_existe"] == 1

def recursiva(dados):
    return recursiva(dados)

def aloca_gigante(dados):
    # O tamanho vem dos dados, não do script: multiplicação de **constantes** é feita na carga do
    # script (e o prazo do Starlark só a percebe depois); com o valor vindo dos dados, o custo cai
    # na avaliação da regra, que é o que o prazo de 200 ms protege.
    return len("x" * int(dados["relato"]["n"])) > 0

def ecoa(dados):
    return dados["relato"]["resumo"] == "__import__('os').system('ls')"
"#;

fn dados(json: &str) -> Dados {
    snapshot_from_json(json).expect("dados de teste válidos")
}

fn evaluator() -> RuleEvaluator {
    RuleEvaluator::new(SCRIPT).expect("o script de apoio carrega")
}

fn code(error: &RuleError) -> &'static str {
    error.api_code()
}

#[test]
fn equal_compares_text_number_boolean_and_attachment() {
    let base = dados(
        r#"{"relato": {"nome": "  Maria  ", "idade": 42, "aceitou": true, "outro": false,
                        "periodo": "tarde", "anexo": {"nome": "foto.jpg", "tipo": "image/jpeg"}}}"#,
    );
    let cases = [
        (
            Rule::Igual {
                campo: "relato.nome".into(),
                value: "Maria".into(),
            },
            true,
        ),
        (
            Rule::Igual {
                campo: "relato.nome".into(),
                value: " maria ".into(),
            },
            false,
        ),
        (
            Rule::Igual {
                campo: "relato.idade".into(),
                value: "42".into(),
            },
            true,
        ),
        (
            Rule::Igual {
                campo: "relato.idade".into(),
                value: "42.0".into(),
            },
            false,
        ),
        (
            Rule::Igual {
                campo: "relato.aceitou".into(),
                value: "sim".into(),
            },
            true,
        ),
        (
            Rule::Igual {
                campo: "relato.aceitou".into(),
                value: "true".into(),
            },
            true,
        ),
        (
            Rule::Igual {
                campo: "relato.aceitou".into(),
                value: "1".into(),
            },
            true,
        ),
        (
            Rule::Igual {
                campo: "relato.outro".into(),
                value: "nao".into(),
            },
            true,
        ),
        (
            Rule::Igual {
                campo: "relato.outro".into(),
                value: "0".into(),
            },
            true,
        ),
        (
            Rule::Igual {
                campo: "relato.aceitou".into(),
                value: "nao".into(),
            },
            false,
        ),
        (
            Rule::Igual {
                campo: "relato.aceitou".into(),
                value: "talvez".into(),
            },
            false,
        ),
        (
            Rule::Igual {
                campo: "relato.periodo".into(),
                value: "tarde".into(),
            },
            true,
        ),
        (
            Rule::Igual {
                campo: "relato.periodo".into(),
                value: "Tarde".into(),
            },
            false,
        ),
        (
            Rule::Igual {
                campo: "relato.anexo".into(),
                value: "foto.jpg".into(),
            },
            true,
        ),
    ];
    for (rule, expected_json) in cases {
        assert_eq!(
            evaluate(&rule, &base).unwrap(),
            expected_json,
            "regra {rule:?}"
        );
    }
}

#[test]
fn different_is_inverse_of_equal() {
    let base = dados(r#"{"relato": {"periodo": "manha"}}"#);
    assert!(
        evaluate(
            &Rule::Diferente {
                campo: "relato.periodo".into(),
                value: "tarde".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        !evaluate(
            &Rule::Diferente {
                campo: "relato.periodo".into(),
                value: "manha".into()
            },
            &base
        )
        .unwrap()
    );
}

#[test]
fn greater_and_less_compare_number_and_numeric_text() {
    let base =
        dados(r#"{"coleta": {"dias": 12, "dias_texto": "12", "nome": "doze", "nulo": null}}"#);
    assert!(
        evaluate(
            &Rule::Maior {
                campo: "coleta.dias".into(),
                value: "10".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        !evaluate(
            &Rule::Menor {
                campo: "coleta.dias".into(),
                value: "10".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        evaluate(
            &Rule::Maior {
                campo: "coleta.dias_texto".into(),
                value: "9".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        !evaluate(
            &Rule::Maior {
                campo: "coleta.nome".into(),
                value: "9".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        !evaluate(
            &Rule::Maior {
                campo: "coleta.nulo".into(),
                value: "9".into()
            },
            &base
        )
        .unwrap()
    );
    let decimal = dados(r#"{"coleta": {"dias": 10.5}}"#);
    assert!(
        evaluate(
            &Rule::Maior {
                campo: "coleta.dias".into(),
                value: "10.25".into()
            },
            &decimal
        )
        .unwrap()
    );
}

#[test]
fn compare_with_non_numeric_literal_is_invalid_rule() {
    let base = dados(r#"{"coleta": {"dias": 12}}"#);
    let error = evaluate(
        &Rule::Maior {
            campo: "coleta.dias".into(),
            value: "dez".into(),
        },
        &base,
    )
    .expect_err("literal não numérico tem de ser recusado");
    assert_eq!(code(&error), "regra_invalida");
    assert!(error.to_string().contains("dez"), "{error}");
}

#[test]
fn contains_ignores_case_and_accent_in_text_and_in_list() {
    let base = dados(
        r#"{"relato": {"resumo": "O sistema NÃO ABRE desde ontem", "tags": ["urgente", "Fatura"]}}"#,
    );
    assert!(
        evaluate(
            &Rule::Contem {
                campo: "relato.resumo".into(),
                value: "nao abre".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        evaluate(
            &Rule::Contem {
                campo: "relato.resumo".into(),
                value: "NÃO".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        !evaluate(
            &Rule::Contem {
                campo: "relato.resumo".into(),
                value: "fechou".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        evaluate(
            &Rule::Contem {
                campo: "relato.tags".into(),
                value: "urgen".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        evaluate(
            &Rule::Contem {
                campo: "relato.tags".into(),
                value: "fatura".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        !evaluate(
            &Rule::Contem {
                campo: "relato.tags".into(),
                value: "cobranca".into()
            },
            &base
        )
        .unwrap()
    );
}

#[test]
fn empty_covers_missing_null_spaces_and_list() {
    let base = dados(
        r#"{"relato": {"vazio": "", "espacos": "   ", "nulo": null, "lista": [], "numero": 0, "texto": "x"}}"#,
    );
    let cases = [
        ("relato.vazio", true),
        ("relato.espacos", true),
        ("relato.nulo", true),
        ("relato.lista", true),
        ("relato.numero", false),
        ("relato.texto", false),
        ("relato.nao_existe", true),
    ];
    for (campo, expected_json) in cases {
        assert_eq!(
            evaluate(
                &Rule::Vazio {
                    campo: campo.into()
                },
                &base
            )
            .unwrap(),
            expected_json,
            "campo {campo}"
        );
    }
}

#[test]
fn and_or_not_nested() {
    let base =
        dados(r#"{"relato": {"afeta_agora": "sim", "resumo": "está parado", "aceitou": true}}"#);
    let afeta = Rule::Igual {
        campo: "relato.afeta_agora".into(),
        value: "sim".into(),
    };
    let vazio = Rule::Vazio {
        campo: "relato.resumo".into(),
    };
    let stopped = Rule::Contem {
        campo: "relato.resumo".into(),
        value: "parado".into(),
    };
    let aceitou = Rule::Igual {
        campo: "relato.aceitou".into(),
        value: "sim".into(),
    };

    let rule = Rule::E {
        args: vec![
            afeta,
            Rule::Nao { args: vec![vazio] },
            Rule::Ou {
                args: vec![stopped, aceitou],
            },
        ],
    };
    assert!(evaluate(&rule, &base).unwrap());

    let r#false = Rule::E {
        args: vec![
            Rule::Igual {
                campo: "relato.resumo".into(),
                value: "outro".into(),
            },
            Rule::Vazio {
                campo: "relato.resumo".into(),
            },
        ],
    };
    assert!(!evaluate(&r#false, &base).unwrap());

    let none = Rule::Ou {
        args: vec![
            Rule::Vazio {
                campo: "relato.resumo".into(),
            },
            Rule::Igual {
                campo: "relato.aceitou".into(),
                value: "nao".into(),
            },
        ],
    };
    assert!(!evaluate(&none, &base).unwrap());

    let malformed = Rule::Nao {
        args: vec![
            Rule::Vazio {
                campo: "relato.resumo".into(),
            },
            Rule::Vazio {
                campo: "relato.aceitou".into(),
            },
        ],
    };
    assert_eq!(
        code(&evaluate(&malformed, &base).unwrap_err()),
        "regra_invalida"
    );
}

#[test]
fn field_missing_follows_table_document() {
    let base = dados(r#"{"relato": {}}"#);
    let cases = [
        (
            Rule::Igual {
                campo: "relato.x".into(),
                value: "y".into(),
            },
            false,
        ),
        (
            Rule::Igual {
                campo: "relato.x".into(),
                value: "".into(),
            },
            false,
        ),
        (
            Rule::Diferente {
                campo: "relato.x".into(),
                value: "y".into(),
            },
            true,
        ),
        (
            Rule::Diferente {
                campo: "relato.x".into(),
                value: "".into(),
            },
            true,
        ),
        (
            Rule::Maior {
                campo: "relato.x".into(),
                value: "1".into(),
            },
            false,
        ),
        (
            Rule::Menor {
                campo: "relato.x".into(),
                value: "1".into(),
            },
            false,
        ),
        (
            Rule::Contem {
                campo: "relato.x".into(),
                value: "".into(),
            },
            false,
        ),
        (
            Rule::Vazio {
                campo: "relato.x".into(),
            },
            true,
        ),
    ];
    for (rule, expected_json) in cases {
        assert_eq!(
            evaluate(&rule, &base).unwrap(),
            expected_json,
            "regra {rule:?}"
        );
    }
}

#[test]
fn nested_rule_beyond_cap_refused_without_crashing_stack() {
    let mut rule = Rule::Vazio {
        campo: "relato.x".into(),
    };
    for _ in 0..(MAX_NESTING + 5) {
        rule = Rule::Nao { args: vec![rule] };
    }
    let error = evaluate(&rule, &dados(r#"{"relato": {"x": "1"}}"#))
        .expect_err("aninhamento acima do teto tem de ser recusado");
    assert_eq!(code(&error), "regra_invalida");
    assert!(error.to_string().contains("aninhamento"), "{error}");
}

#[test]
fn function_reads_data_and_decides() {
    let evaluator = evaluator();
    let urgente = dados(r#"{"relato": {"resumo": "o sistema está PARADO"}}"#);
    let normal = dados(r#"{"relato": {"resumo": "gostaria de um orçamento"}}"#);
    let rule = Rule::Funcao {
        nome: "grave".into(),
    };
    assert!(evaluator.evaluate(&rule, &urgente).unwrap());
    assert!(!evaluator.evaluate(&rule, &normal).unwrap());
}

#[test]
fn function_receives_whole_dictionary_including_null_attachment() {
    let evaluator = evaluator();
    let rule = Rule::Funcao {
        nome: "viu_anexo".into(),
    };
    assert!(
        evaluator
            .evaluate(
                &rule,
                &dados(r#"{"relato": {"anexo": {"nome": "nota.pdf"}}}"#)
            )
            .unwrap()
    );
    assert!(
        !evaluator
            .evaluate(&rule, &dados(r#"{"relato": {"anexo": null}}"#))
            .unwrap()
    );
    assert!(!evaluator.evaluate(&rule, &dados("{}")).unwrap());
}

#[test]
fn function_that_returns_text_and_rule_invalid() {
    let evaluator = evaluator();
    let error = evaluator
        .evaluate(
            &Rule::Funcao {
                nome: "devolve_texto".into(),
            },
            &dados("{}"),
        )
        .expect_err("devolver texto tem de ser erro");
    assert_eq!(code(&error), "regra_invalida");
    assert!(error.to_string().contains("True"), "{error}");
}

#[test]
fn function_returning_none_is_invalid_rule() {
    let evaluator = evaluator();
    let error = evaluator
        .evaluate(
            &Rule::Funcao {
                nome: "devolve_nada".into(),
            },
            &dados("{}"),
        )
        .expect_err("devolver None tem de ser erro");
    assert_eq!(code(&error), "regra_invalida");
}

#[test]
fn function_reading_missing_value_fails_without_panic() {
    let evaluator = evaluator();
    let error = evaluator
        .evaluate(
            &Rule::Funcao {
                nome: "le_ausente".into(),
            },
            &dados(r#"{"relato": {}}"#),
        )
        .expect_err("chave ausente tem de virar erro tipado");
    assert_eq!(code(&error), "regra_invalida");
}

#[test]
fn function_exceeding_ticks_is_rule_limit() {
    let evaluator = evaluator();
    let beginning = Instant::now();
    let error = evaluator
        .evaluate(
            &Rule::Funcao {
                nome: "estoura_ticks".into(),
            },
            &dados("{}"),
        )
        .expect_err("laço longo tem de ser cortado");
    let gasto = beginning.elapsed();
    assert_eq!(code(&error), "regra_limite", "{error}");
    assert!(gasto < RULE_DEADLINE, "levou {gasto:?}");
}

#[test]
fn function_recursive_and_rule_limit() {
    let evaluator = evaluator();
    let beginning = Instant::now();
    let error = evaluator
        .evaluate(
            &Rule::Funcao {
                nome: "recursiva".into(),
            },
            &dados("{}"),
        )
        .expect_err("recursão tem de ser cortada");
    let gasto = beginning.elapsed();
    assert_eq!(code(&error), "regra_limite", "{error}");
    assert!(gasto < RULE_DEADLINE, "levou {gasto:?}");
}

#[test]
fn function_allocating_huge_answers_within_deadline() {
    let evaluator = evaluator();
    for n in [100_000_000_u64, 1_000_000_000_u64] {
        let base = dados(&format!(r#"{{"relato": {{"n": {n}}}}}"#));
        let beginning = Instant::now();
        let error = evaluator
            .evaluate(
                &Rule::Funcao {
                    nome: "aloca_gigante".into(),
                },
                &base,
            )
            .expect_err("alocação gigante tem de ser cortada");
        let gasto = beginning.elapsed();
        assert_eq!(code(&error), "regra_limite", "n={n}: {error}");
        assert!(
            gasto <= RULE_DEADLINE + DEADLINE_TOLERANCE + Duration::from_millis(250),
            "n={n}: a regra respondeu em {gasto:?}, acima do prazo"
        );
    }
}

#[test]
fn script_load_with_huge_constant_answers_within_deadline() {
    let fonte = r#"
fluxo(
    id = "constante_gigante",
    nome = "Constante gigante",
    inicio = "relato",
    gatilho = gatilho(quando = "o cliente relata", nao_quando = "o cliente agradece", exemplos = ["deu erro"]),
)

etapa(
    id = "relato",
    nome = "Relato",
    campos = [campo("resumo", "O que aconteceu", tipo = "texto")],
    acoes = [acao("ok", "Registrado", vai_para = fim("concluido"))],
)

def aloca_gigante(dados):
    return len("x" * 100000000) > 0
"#;
    let beginning = Instant::now();
    let error =
        RuleEvaluator::new(fonte).expect_err("carga com constante gigante tem de ser cortada");
    let gasto = beginning.elapsed();
    assert_eq!(code(&error), "regra_limite", "{error}");
    assert!(
        gasto <= Duration::from_secs(2),
        "a carga respondeu em {gasto:?}, acima do prazo da carga mais a margem"
    );
}

#[test]
fn limits_load_become_rule_limit() {
    let tight = Limites {
        max_heap_bytes: 1,
        ..Limites::default()
    };
    let error = RuleEvaluator::new_with(SCRIPT, &tight, RuleLimits::default())
        .expect_err("heap de 1 byte não carrega o script");
    assert_eq!(code(&error), "regra_limite", "{error}");
}

#[test]
fn function_without_def_and_rule_invalid() {
    let evaluator = evaluator();
    let error = evaluator
        .evaluate(
            &Rule::Funcao {
                nome: "nao_existe".into(),
            },
            &dados("{}"),
        )
        .expect_err("def ausente tem de ser erro");
    assert_eq!(code(&error), "regra_invalida");
    assert!(error.to_string().contains("nao_existe"), "{error}");
}

#[test]
fn function_without_loaded_module_is_invalid_rule() {
    let error = evaluate(
        &Rule::Funcao {
            nome: "grave".into(),
        },
        &dados("{}"),
    )
    .expect_err("sem módulo não há como rodar def");
    assert_eq!(code(&error), "regra_invalida");
    assert!(error.to_string().contains("Avaliador::novo"), "{error}");
}

#[test]
fn rule_limits_are_really_enabled() {
    let tight = RuleLimits {
        max_ticks: 100,
        ..RuleLimits::default()
    };
    let evaluator = RuleEvaluator::new_with(SCRIPT, &Limites::default(), tight)
        .expect("o script carrega com os limites de carga");
    let error = evaluator
        .evaluate(
            &Rule::Funcao {
                nome: "estoura_ticks".into(),
            },
            &dados("{}"),
        )
        .expect_err("o laço não cabe em 100 ticks");
    assert_eq!(code(&error), "regra_limite", "{error}");

    let without_heap = RuleLimits {
        max_heap_bytes: 1,
        ..RuleLimits::default()
    };
    let evaluator =
        RuleEvaluator::new_with(SCRIPT, &Limites::default(), without_heap).expect("carrega");
    let error = evaluator
        .evaluate(
            &Rule::Funcao {
                nome: "grave".into(),
            },
            &dados("{}"),
        )
        .expect_err("heap de 1 byte não dá para nada");
    assert_eq!(code(&error), "regra_limite", "{error}");

    let without_stack = RuleLimits {
        max_callstack: 2,
        ..RuleLimits::default()
    };
    let evaluator =
        RuleEvaluator::new_with(SCRIPT, &Limites::default(), without_stack).expect("carrega");
    let error = evaluator
        .evaluate(
            &Rule::Funcao {
                nome: "recursiva".into(),
            },
            &dados("{}"),
        )
        .expect_err("recursão com pilha de 2 tem de ser cortada");
    assert_eq!(code(&error), "regra_limite", "{error}");
}

#[test]
fn script_that_does_not_load_is_invalid_rule() {
    let error = RuleEvaluator::new("fluxo(\n  id = \n").expect_err("sintaxe quebrada não carrega");
    assert_eq!(code(&error), "regra_invalida");
}

#[test]
fn customer_data_enters_as_value_never_as_code() {
    let evaluator = evaluator();
    let base = dados(r#"{"relato": {"resumo": "__import__('os').system('ls')"}}"#);
    assert!(
        evaluator
            .evaluate(
                &Rule::Funcao {
                    nome: "ecoa".into()
                },
                &base
            )
            .unwrap()
    );
    assert!(
        evaluate(
            &Rule::Contem {
                campo: "relato.resumo".into(),
                value: "system('ls')".into()
            },
            &base
        )
        .unwrap()
    );
    let perigoso = dados(r#"{"relato": {"resumo": "load(\"x.star\", \"y\")"}}"#);
    assert!(
        evaluator
            .evaluate(
                &Rule::Funcao {
                    nome: "ecoa".into()
                },
                &perigoso
            )
            .is_ok()
    );
}

#[test]
fn same_rule_with_same_data_same_result() {
    let evaluator = evaluator();
    let base = dados(r#"{"relato": {"resumo": "está parado", "aceitou": true}}"#);
    let rule = Rule::E {
        args: vec![
            Rule::Funcao {
                nome: "grave".into(),
            },
            Rule::Igual {
                campo: "relato.aceitou".into(),
                value: "sim".into(),
            },
        ],
    };
    let first_result = evaluator.evaluate(&rule, &base).unwrap();
    for _ in 0..5 {
        assert_eq!(evaluator.evaluate(&rule, &base).unwrap(), first_result);
    }
}

#[test]
fn p95_of_combinators_within_target() {
    let base = dados(r#"{"relato": {"resumo": "está parado", "aceitou": true, "dias": 12}}"#);
    let rule = Rule::E {
        args: vec![
            Rule::Igual {
                campo: "relato.aceitou".into(),
                value: "sim".into(),
            },
            Rule::Contem {
                campo: "relato.resumo".into(),
                value: "parado".into(),
            },
            Rule::Ou {
                args: vec![
                    Rule::Maior {
                        campo: "relato.dias".into(),
                        value: "10".into(),
                    },
                    Rule::Vazio {
                        campo: "relato.resumo".into(),
                    },
                ],
            },
        ],
    };
    for _ in 0..1000 {
        assert!(evaluate(&rule, &base).unwrap());
    }
    let mut times = Vec::with_capacity(20_000);
    for _ in 0..20_000 {
        let beginning = Instant::now();
        let _ = evaluate(&rule, &base).unwrap();
        times.push(beginning.elapsed());
    }
    times.sort_unstable();
    let p50 = times[times.len() / 2];
    let p95 = times[times.len() * 95 / 100];
    println!("combinadores: p50 {p50:?}, p95 {p95:?} (build de teste)");
    assert!(p95 < Duration::from_micros(200), "p95 {p95:?}");
}

#[test]
fn p95_of_function_within_deadline() {
    let evaluator = evaluator();
    let base = dados(r#"{"relato": {"resumo": "está parado"}}"#);
    let rule = Rule::Funcao {
        nome: "grave".into(),
    };
    for _ in 0..50 {
        assert!(evaluator.evaluate(&rule, &base).unwrap());
    }
    let mut times = Vec::with_capacity(300);
    for _ in 0..300 {
        let beginning = Instant::now();
        let _ = evaluator.evaluate(&rule, &base).unwrap();
        times.push(beginning.elapsed());
    }
    times.sort_unstable();
    let p50 = times[times.len() / 2];
    let p95 = times[times.len() * 95 / 100];
    println!("função (com thread e prazo): p50 {p50:?}, p95 {p95:?} (build de teste)");
    assert!(p95 < RULE_DEADLINE, "p95 {p95:?}");
}

#[test]
fn each_error_rule_has_code_api() {
    let base = dados(r#"{"relato": {}}"#);
    let invalid = evaluate(
        &Rule::Maior {
            campo: "relato.x".into(),
            value: "abc".into(),
        },
        &base,
    )
    .unwrap_err();
    assert_eq!(invalid.api_code(), "regra_invalida");
    assert_eq!(invalid.http_status(), 400);

    let limit_error = DomainError::RuleLimit {
        reason: "a função passou do prazo".to_owned(),
    };
    assert_eq!(limit_error.api_code(), "regra_limite");
    assert_eq!(limit_error.http_status(), 400);
}
