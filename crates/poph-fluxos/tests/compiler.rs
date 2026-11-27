use std::time::{Duration, Instant};

use poph_fluxos::compilador::MAX_FONTE_BYTES;
use poph_fluxos::{Code, Compiled, Limites, LoadFailure, compile, load};

const BASE: &str = r#"fluxo(
    id = "teste",
    nome = "Teste",
    inicio = "entrada",
    gatilho = gatilho(
        quando = "O cliente quer testar.",
        nao_quando = "O cliente nao quer testar.",
        exemplos = ["quero testar"],
    ),
)

etapa(
    id = "entrada",
    nome = "Entrada",
    campos = [campo("nome", "Nome", obrigatorio = True)],
    acoes = [acao("seguir", "Seguir", exige = ["nome"], vai_para = fim("pronto", resultado = "Feito"))],
)
"#;

fn limites() -> Limites {
    Limites::default()
}

fn compiles(fonte: &str) -> Compiled {
    compile(fonte, &limites())
}

fn example(nome: &str) -> String {
    let path = format!(
        "{}/../../tests/fluxos/exemplos/{nome}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn with_rule(rule: &str) -> String {
    format!(
        "{BASE}\ndecisao(\n    id = \"escolha\",\n    se = {rule},\n    entao = \"entrada\",\n    senao = \"entrada\",\n)\n"
    )
}

struct Case {
    code: Code,
    fonte: String,
    limites: Limites,
    line: u32,
    column: u32,
}

fn positions() -> Vec<(Code, u32, u32)> {
    vec![
        (Code::Syntax, 3, 1),
        (Code::SourceTooLarge, 1, 1),
        (Code::Dialect, 1, 1),
        (Code::LoadLimit, 1, 1),
        (Code::InvalidTopLevel, 1, 1),
        (Code::WrongType, 15, 51),
        (Code::UnknownName, 20, 14),
        (Code::MissingFlow, 1, 1),
        (Code::MissingArgument, 1, 1),
        (Code::UnknownArgument, 15, 52),
        (Code::InvalidId, 2, 10),
        (Code::DuplicateId, 20, 10),
        (Code::UnknownDestination, 16, 68),
        (Code::DivergentEnd, 22, 46),
        (Code::UnknownRequirement, 16, 47),
        (Code::InvalidOptions, 15, 21),
        (Code::InvalidRuleOrActor, 15, 14),
        (Code::UnknownType, 15, 44),
        (Code::MalformedCombinator, 21, 16),
        (Code::UnknownField, 21, 16),
        (Code::DecisionCycle, 19, 1),
        (Code::InvalidDef, 19, 1),
        (Code::InvalidVariable, 15, 38),
        (Code::ForbiddenInterpolation, 15, 17),
        (Code::NodeLimit, 1, 1),
        (Code::InvalidNumber, 16, 94),
        (Code::StageWithoutAction, 12, 1),
        (Code::Unreachable, 19, 1),
        (Code::UnusedField, 15, 21),
        (Code::OutsideOptions, 21, 16),
        (Code::ContradictoryCondition, 21, 10),
        (Code::TriggerWithoutExamples, 5, 15),
        (Code::TriggerWithoutNegative, 5, 15),
        (Code::NotificationWithoutVariable, 16, 122),
        (Code::VariableWithoutField, 15, 38),
        (Code::TextComparison, 21, 16),
        (Code::UnusedDef, 19, 1),
        (Code::ActorWithoutAction, 15, 14),
        (Code::VariableFromOwnStep, 15, 38),
        (Code::LongText, 15, 17),
    ]
}

fn position_expected(code: Code) -> (u32, u32) {
    positions()
        .into_iter()
        .find(|(c, _, _)| *c == code)
        .map(|(_, line, column)| (line, column))
        .unwrap_or_else(|| panic!("{code} sem posição esperada"))
}

fn case(code: Code, fonte: impl Into<String>) -> Case {
    let (line, column) = position_expected(code);
    Case {
        code,
        fonte: fonte.into(),
        limites: limites(),
        line,
        column,
    }
}

fn cases() -> Vec<Case> {
    let (line_load, column_load) = position_expected(Code::LoadLimit);
    vec![
        case(Code::Syntax, "fluxo(\n    id = \"teste\",\n"),
        case(Code::SourceTooLarge, "#".repeat(MAX_FONTE_BYTES + 1)),
        case(
            Code::Dialect,
            format!("load(\"outro.star\", \"ajuda\")\n{BASE}"),
        ),
        Case {
            code: Code::LoadLimit,
            fonte: BASE.to_owned(),
            limites: Limites {
                max_heap_bytes: 1,
                ..limites()
            },
            line: line_load,
            column: column_load,
        },
        case(Code::InvalidTopLevel, format!("contador = 0\n{BASE}")),
        case(
            Code::WrongType,
            BASE.replace("obrigatorio = True", "obrigatorio = \"sim\""),
        ),
        case(
            Code::UnknownName,
            format!("{BASE}\ndef minha(dados):\n    aberto = open(\"x.txt\")\n    return len(aberto) > 0\n"),
        ),
        case(
            Code::MissingFlow,
            "etapa(\n    id = \"entrada\",\n    nome = \"Entrada\",\n)\n",
        ),
        case(
            Code::MissingArgument,
            BASE.replace("    inicio = \"entrada\",\n", ""),
        ),
        case(
            Code::UnknownArgument,
            BASE.replace("obrigatorio = True", "obrigatorioo = True"),
        ),
        case(
            Code::InvalidId,
            BASE.replace("id = \"teste\"", "id = \"Teste Ruim\""),
        ),
        case(
            Code::DuplicateId,
            format!("{BASE}\netapa(\n    id = \"entrada\",\n    nome = \"Repetida\",\n    acoes = [acao(\"outra\", \"Outra\", vai_para = fim(\"repetida\"))],\n)\n"),
        ),
        case(
            Code::UnknownDestination,
            BASE.replace(
                "vai_para = fim(\"pronto\", resultado = \"Feito\")",
                "vai_para = \"nao_existe\"",
            ),
        ),
        case(
            Code::DivergentEnd,
            format!("{BASE}\netapa(\n    id = \"outra\",\n    nome = \"Outra\",\n    acoes = [acao(\"sair\", \"Sair\", vai_para = fim(\"pronto\", resultado = \"Outro\"))],\n)\n"),
        ),
        case(
            Code::UnknownRequirement,
            BASE.replace("exige = [\"nome\"]", "exige = [\"cep\"]"),
        ),
        case(
            Code::InvalidOptions,
            BASE.replace(
                "campo(\"nome\", \"Nome\", obrigatorio = True)",
                "campo(\"nome\", \"Nome\", tipo = \"opcao\", obrigatorio = True)",
            ),
        ),
        case(
            Code::InvalidRuleOrActor,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    atores = [\"cliente\"],\n",
            ),
        ),
        case(
            Code::UnknownType,
            BASE.replace(
                "campo(\"nome\", \"Nome\", obrigatorio = True)",
                "campo(\"nome\", \"Nome\", tipo = \"telefone\", obrigatorio = True)",
            ),
        ),
        case(Code::MalformedCombinator, with_rule("igual(\"nome\", \"x\")")),
        case(Code::UnknownField, with_rule("igual(\"entrada.cep\", \"x\")")),
        case(
            Code::DecisionCycle,
            format!(
                "{BASE}\ndecisao(\n    id = \"a\",\n    se = igual(\"entrada.nome\", \"x\"),\n    entao = \"b\",\n    senao = \"b\",\n)\n\ndecisao(\n    id = \"b\",\n    se = igual(\"entrada.nome\", \"x\"),\n    entao = \"a\",\n    senao = \"a\",\n)\n"
            ),
        ),
        case(
            Code::InvalidDef,
            format!("{BASE}\ndef minha(dados, extra):\n    return True\n"),
        ),
        case(
            Code::InvalidVariable,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    ao_entrar = [mensagem_ao_cliente(\"Olá {cliente.nome}\")],\n",
            ),
        ),
        case(
            Code::ForbiddenInterpolation,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    instrucao = \"Veja {entrada.nome}\",\n",
            ),
        ),
        case(Code::NodeLimit, flow_with_61_nodes()),
        case(
            Code::InvalidNumber,
            BASE.replace(
                "exige = [\"nome\"], ",
                "exige = [\"nome\"], efeitos = [notificar_whatsapp(para = \"123\", mensagem = \"Pedido {entrada.nome}\")], ",
            ),
        ),
        case(
            Code::StageWithoutAction,
            BASE.replace(
                "    acoes = [acao(\"seguir\", \"Seguir\", exige = [\"nome\"], vai_para = fim(\"pronto\", resultado = \"Feito\"))],\n",
                "",
            ),
        ),
        case(
            Code::Unreachable,
            format!("{BASE}\netapa(\n    id = \"ilha\",\n    nome = \"Ilha\",\n    acoes = [acao(\"sair\", \"Sair\", vai_para = fim(\"ilha_fim\"))],\n)\n"),
        ),
        case(Code::UnusedField, BASE.replace("exige = [\"nome\"], ", "")),
        case(
            Code::OutsideOptions,
            format!(
                "{}\ndecisao(\n    id = \"escolha\",\n    se = igual(\"entrada.nome\", \"talvez\"),\n    entao = \"entrada\",\n    senao = \"entrada\",\n)\n",
                BASE.replace(
                    "campo(\"nome\", \"Nome\", obrigatorio = True)",
                    "campo(\"nome\", \"Nome\", tipo = \"opcao\", opcoes = [\"sim\", \"nao\"], obrigatorio = True)",
                )
            ),
        ),
        case(
            Code::ContradictoryCondition,
            with_rule("e(igual(\"entrada.nome\", \"x\"), diferente(\"entrada.nome\", \"x\"))"),
        ),
        case(
            Code::TriggerWithoutExamples,
            BASE.replace("        exemplos = [\"quero testar\"],\n", ""),
        ),
        case(
            Code::TriggerWithoutNegative,
            BASE.replace("        nao_quando = \"O cliente nao quer testar.\",\n", ""),
        ),
        case(
            Code::NotificationWithoutVariable,
            BASE.replace(
                "exige = [\"nome\"], ",
                "exige = [\"nome\"], efeitos = [notificar_whatsapp(para = \"5521999990000\", mensagem = \"Pedido entregue\")], ",
            ),
        ),
        case(
            Code::VariableWithoutField,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    ao_entrar = [mensagem_ao_cliente(\"Olá {entrada.cep}\")],\n",
            ),
        ),
        case(Code::TextComparison, with_rule("maior(\"entrada.nome\", \"10\")")),
        case(
            Code::UnusedDef,
            format!("{BASE}\ndef minha(dados):\n    return True\n"),
        ),
        case(
            Code::ActorWithoutAction,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    atores = [\"agente\", \"atendente\"],\n",
            )
            .replace(
                "acao(\"seguir\", \"Seguir\",",
                "acao(\"seguir\", \"Seguir\", ator = \"agente\",",
            ),
        ),
        case(
            Code::VariableFromOwnStep,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    ao_entrar = [mensagem_ao_cliente(\"Olá {entrada.nome}\")],\n",
            ),
        ),
        case(
            Code::LongText,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                &format!("    nome = \"Entrada\",\n    instrucao = \"{}\",\n", "a".repeat(1200)),
            ),
        ),
    ]
}

fn flow_with_61_nodes() -> String {
    let mut fonte = String::from(
        "fluxo(\n    id = \"grande\",\n    nome = \"Grande\",\n    inicio = \"e0\",\n    gatilho = gatilho(\n        quando = \"O cliente quer testar.\",\n        nao_quando = \"O cliente nao quer testar.\",\n        exemplos = [\"quero testar\"],\n    ),\n)\n\n",
    );
    for index in 0..31 {
        fonte.push_str(&format!(
            "etapa(\n    id = \"e{index}\",\n    nome = \"Etapa {index}\",\n    acoes = [acao(\"seguir\", \"Seguir\", vai_para = fim(\"f{index}\"))],\n)\n\n"
        ));
    }
    fonte
}

#[test]
fn def_with_name_declaration_and_refused_with_position() {
    for nome in ["etapa", "fluxo"] {
        let fonte = format!("{BASE}\ndef {nome}(dados):\n    return True\n");
        let compiled = compiles(&fonte);
        let diagnostic = compiled
            .diagnostics
            .iter()
            .find(|d| d.code == Code::InvalidDef)
            .unwrap_or_else(|| {
                panic!(
                    "`def {nome}(dados)` devia dar FLX24; diagnósticos: {:?}",
                    compiled.diagnostics
                )
            });
        assert_eq!(
            (diagnostic.line, diagnostic.column),
            (Some(19), Some(1)),
            "`def {nome}` tem de apontar o começo da declaração: {diagnostic}"
        );
        assert!(
            diagnostic.mensagem.contains(nome),
            "a mensagem tem de dizer qual é o nome reservado: {}",
            diagnostic.mensagem
        );
        assert!(compiled.ir.is_none(), "erro de forma não gera IR");
    }
}

#[test]
fn base_flow_has_no_diagnostic() {
    let compiled = compiles(BASE);
    assert!(
        compiled.diagnostics.is_empty(),
        "o fluxo base tem de passar limpo: {:?}",
        compiled.diagnostics
    );
    assert!(compiled.ir.is_some(), "o fluxo base tem de gerar IR");
}

#[test]
fn each_code_list_closed_has_case() {
    let mut seen: Vec<Code> = cases().iter().map(|case| case.code).collect();
    let total = seen.len();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), total, "dois casos para o mesmo código");
    assert_eq!(
        seen.len(),
        40,
        "a lista fechada tem 40 códigos do compilador"
    );
    for code in Code::from_compiler() {
        assert!(seen.contains(&code), "{code} sem caso que o dispare");
    }
}

#[test]
fn each_diagnostic_points_line_and_column_passage() {
    let mut failures: Vec<String> = Vec::new();
    for case in cases() {
        let compiled = compile(&case.fonte, &case.limites);
        let Some(diagnostic) = compiled.diagnostics.iter().find(|d| d.code == case.code) else {
            failures.push(format!(
                "{} não apareceu (diagnósticos: {:?})",
                case.code, compiled.diagnostics
            ));
            continue;
        };
        if (diagnostic.line, diagnostic.column) != (Some(case.line), Some(case.column)) {
            failures.push(format!(
                "{} em {:?}:{:?} (esperado {}:{}); trecho: {}",
                case.code,
                diagnostic.line,
                diagnostic.column,
                case.line,
                case.column,
                diagnostic.mensagem
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn base_flow_fires_none_of_40_codes() {
    let compiled = compiles(BASE);
    for code in Code::from_compiler() {
        assert!(
            !compiled.diagnostics.iter().any(|d| d.code == code),
            "{code} não devia aparecer num fluxo correto"
        );
    }
}

#[test]
fn error_blocks_ir_and_notice_does_not() {
    for case in cases() {
        let compiled = compile(&case.fonte, &case.limites);
        let error = !matches!(
            case.code,
            Code::StageWithoutAction
                | Code::Unreachable
                | Code::UnusedField
                | Code::OutsideOptions
                | Code::ContradictoryCondition
                | Code::TriggerWithoutExamples
                | Code::TriggerWithoutNegative
                | Code::NotificationWithoutVariable
                | Code::VariableWithoutField
                | Code::TextComparison
                | Code::UnusedDef
                | Code::ActorWithoutAction
                | Code::VariableFromOwnStep
                | Code::LongText
        );
        assert_eq!(
            compiled.has_error(),
            error,
            "{} deveria {}bloquear a publicação",
            case.code,
            if error { "" } else { "não " }
        );
        if error {
            assert!(
                compiled.ir.is_none(),
                "{} é erro, então não pode sair IR",
                case.code
            );
        }
    }
}

#[test]
#[ignore]
fn inventario() {
    for case in cases() {
        let compiled = compile(&case.fonte, &case.limites);
        match compiled.diagnostics.iter().find(|d| d.code == case.code) {
            Some(d) => {
                let line = case
                    .fonte
                    .lines()
                    .nth(d.line.unwrap_or(1) as usize - 1)
                    .unwrap_or("<fora do texto>");
                println!(
                    "{} => {:?}:{:?} | {}\n    {}",
                    case.code, d.line, d.column, d.mensagem, line
                );
            }
            None => println!(
                "{} => AUSENTE (tem {:?})",
                case.code,
                compiled
                    .diagnostics
                    .iter()
                    .map(|d| d.code)
                    .collect::<Vec<_>>()
            ),
        }
    }
}

#[test]
fn three_examples_compile_to_golden_ir() {
    for nome in [
        "agendar_visita.star",
        "triagem_suporte.star",
        "confirmar_entrega.star",
    ] {
        let fonte = example(nome);
        let compiled = compiles(&fonte);
        assert!(
            compiled.diagnostics.is_empty(),
            "{nome} tem diagnóstico: {:?}",
            compiled.diagnostics
        );
        let ir = compiled.ir.expect("o exemplo tem de gerar IR");
        let path = format!(
            "{}/../../tests/fluxos/ir/{}.json",
            env!("CARGO_MANIFEST_DIR"),
            nome.trim_end_matches(".star")
        );
        let golden = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("{path}: {error} — rode `cargo test -p poph-fluxos -- --ignored gera_douradas`")
        });
        let expected_ir = serde_json::from_str(&golden).expect("a dourada é JSON da IR");
        assert_eq!(ir, expected_ir, "a IR de {nome} não bate com a dourada");
        assert_eq!(
            ir.to_json(),
            expected_json(&path),
            "a forma compacta de {nome} não é a gravada"
        );
    }
}

fn expected_json(path: &str) -> String {
    let texto = std::fs::read_to_string(path).expect("dourada legível");
    let ir: poph_fluxos::Ir = serde_json::from_str(&texto).expect("dourada é IR");
    ir.to_json()
}

#[test]
#[ignore]
fn generates_golden() {
    for nome in [
        "agendar_visita.star",
        "triagem_suporte.star",
        "confirmar_entrega.star",
    ] {
        let fonte = example(nome);
        let compiled = compiles(&fonte);
        assert!(
            compiled.diagnostics.is_empty(),
            "{nome} tem diagnóstico: {:?}",
            compiled.diagnostics
        );
        let ir = compiled.ir.expect("o exemplo tem de gerar IR");
        let path = format!(
            "{}/../../tests/fluxos/ir/{}.json",
            env!("CARGO_MANIFEST_DIR"),
            nome.trim_end_matches(".star")
        );
        std::fs::create_dir_all(
            std::path::Path::new(&path)
                .parent()
                .expect("a dourada tem pasta"),
        )
        .expect("criar a pasta das douradas");
        std::fs::write(&path, serde_json::to_string_pretty(&ir).unwrap() + "\n")
            .unwrap_or_else(|error| panic!("{path}: {error}"));
        println!("gravada {path}");
    }
}

#[test]
fn two_compilations_produce_same_ir_byte_for_byte() {
    for nome in ["triagem_suporte.star", "confirmar_entrega.star"] {
        let fonte = example(nome);
        let first_pass = compiles(&fonte).ir.expect("IR").to_json();
        let second_pass = compiles(&fonte).ir.expect("IR").to_json();
        assert_eq!(first_pass, second_pass, "{nome} não é determinístico");
    }
}

#[test]
fn two_threads_compile_same_text_with_same_ir() {
    let fonte = example("triagem_suporte.star");
    let expected_json = compiles(&fonte).ir.expect("IR").to_json();
    let results: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let fonte = fonte.clone();
                scope.spawn(move || compiles(&fonte).ir.expect("IR").to_json())
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("thread sem pânico"))
            .collect()
    });
    for resultado in results {
        assert_eq!(resultado, expected_json, "a IR mudou entre threads");
    }
}

#[test]
fn pathological_script_ends_with_error_without_hanging() {
    let cases: Vec<(&str, String)> = vec![
        ("load", format!("load(\"outro.star\", \"x\")\n{BASE}")),
        (
            "while",
            format!("{BASE}\ndef gira(dados):\n    while True:\n        pass\n    return True\n"),
        ),
        (
            "recursao",
            format!("{BASE}\ndef gira(dados):\n    return gira(dados)\n"),
        ),
        (
            "alocacao monstro",
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    instrucao = \"x\" * 1000000000,\n",
            ),
        ),
        (
            "relogio",
            format!("{BASE}\ndef agora(dados):\n    return time.time() > 0\n"),
        ),
        (
            "arquivo",
            format!("{BASE}\ndef ler(dados):\n    return open(\"/etc/passwd\").read() != \"\"\n"),
        ),
        (
            "rede",
            format!("{BASE}\ndef baixar(dados):\n    return socket.gethostname() != \"\"\n"),
        ),
        (
            "aleatorio",
            format!("{BASE}\ndef sorte(dados):\n    return random.random() > 0.5\n"),
        ),
    ];
    for (nome, fonte) in cases {
        let beginning = Instant::now();
        let compiled = compiles(&fonte);
        let gasto = beginning.elapsed();
        assert!(
            compiled.has_error(),
            "o caso `{nome}` tinha de terminar com erro, e passou"
        );
        assert!(
            gasto < Duration::from_secs(2),
            "o caso `{nome}` demorou {gasto:?}"
        );
    }
}

#[test]
fn foreign_input_does_not_crash_compiler() {
    let cases = [
        "",
        "   \n\n\t\n",
        "# só comentário\n",
        "def minha(dados):\n    return True\n",
        "fluxo()\n",
        "fluxo(gatilho = \"x\")\n",
        "etapa()\n",
        "decisao()\n",
        "\\u{feff}fluxo(\n",
        "fluxo(\n    id = \"abc\",\n    nome = \"A\",\n    inicio = \"et\",\n    gatilho = gatilho(quando = \"q\"),\n)\netapa(\n    id = \"et\",\n    nome = \"E\",\n    acoes = [acao(\"ir\", \"Ir\", vai_para = fim(\"fim_ok\"))],\n)\n",
    ];
    for fonte in cases {
        let compiled = compiles(fonte);
        let _ = compiled.ir();
        if let Some(ir) = &compiled.ir {
            let read = poph_fluxos::Ir::from_json(&ir.to_json()).expect("a IR volta do JSON");
            assert_eq!(&read, ir, "ida e volta da IR em {fonte:?}");
        }
    }
    assert!(compiles(cases[cases.len() - 1]).ir.is_some());
    assert!(
        compiles("")
            .diagnostics
            .iter()
            .any(|d| d.code == Code::MissingFlow)
    );
}

#[test]
fn source_large_and_refused_before_compile() {
    let fonte = format!("{BASE}\n{}", "#".repeat(MAX_FONTE_BYTES));
    let compiled = compiles(&fonte);
    assert!(compiled.has_error());
    assert_eq!(compiled.diagnostics[0].code, Code::SourceTooLarge);
    assert!(compiled.ir.is_none());
}

#[test]
fn load_limits_are_really_enabled() {
    let tight = Limites {
        load_deadline: Duration::ZERO,
        ..Limites::default()
    };
    let compiled = compile(BASE, &tight);
    assert!(
        compiled
            .diagnostics
            .iter()
            .any(|d| d.code == Code::LoadLimit),
        "prazo zero tinha de estourar o limite de carga: {:?}",
        compiled.diagnostics
    );
    assert!(compiles(BASE).ir.is_some());
}

#[test]
fn load_accepts_ir_that_came_from_own_text() {
    let fonte = example("agendar_visita.star");
    let ir = compiles(&fonte).ir.expect("IR");
    let loaded = load(&ir, &fonte, &limites()).expect("a IR do texto tem de carregar");
    assert_eq!(loaded, ir);
}

#[test]
fn load_refuses_tampered_ir_with_divergent_ir() {
    let fonte = example("agendar_visita.star");
    let mut ir = compiles(&fonte).ir.expect("IR");
    ir.stages[1].actions[0].label = "Visita combinada (adulterado)".to_owned();
    match load(&ir, &fonte, &limites()) {
        Err(LoadFailure::Divergent { fluxo, motivo }) => {
            assert_eq!(fluxo, "agendar_visita");
            assert!(
                motivo.contains("as etapas"),
                "o motivo tem de dizer o que divergiu: {motivo}"
            );
        }
        other => panic!("devia falhar com ir_divergente, e voltou {other:?}"),
    }
}

#[test]
fn load_refuses_text_that_does_not_compile() {
    let fonte = example("agendar_visita.star");
    let ir = compiles(&fonte).ir.expect("IR");
    let quebrado = fonte.replace("inicio = \"coleta\",", "");
    match load(&ir, &quebrado, &limites()) {
        Err(LoadFailure::Form(diagnostics)) => {
            assert!(
                diagnostics.iter().any(|d| d.code == Code::MissingArgument),
                "esperava FLX11: {diagnostics:?}"
            );
            assert_eq!(LoadFailure::Form(diagnostics).code(), "fluxo_invalido");
        }
        other => panic!("devia falhar por forma, e voltou {other:?}"),
    }
}

#[test]
fn ir_ignores_unknown_field_on_read() {
    let fonte = example("agendar_visita.star");
    let ir = compiles(&fonte).ir.expect("IR");
    let mut json: serde_json::Value = serde_json::from_str(&ir.to_json()).expect("JSON");
    json["campo_do_futuro"] = serde_json::json!({"qualquer": "coisa"});
    let read =
        poph_fluxos::Ir::from_json(&json.to_string()).expect("campo desconhecido é ignorado");
    assert_eq!(read, ir);
}
