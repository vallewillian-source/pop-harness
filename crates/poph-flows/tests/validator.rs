use std::collections::BTreeSet;

use poph_flows::{
    Code, Departamento, Diagnostic, Ir, Limites, compile, resolve_departments, validar,
};
use serde_json::{Value, json};

fn golden(nome: &str) -> Value {
    let path = format!(
        "{}/../../tests/fluxos/ir/{nome}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let texto = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
    serde_json::from_str(&texto).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn ir(value: &Value) -> Ir {
    Ir::from_json(&value.to_string()).expect("a IR mutada continua sendo JSON válido")
}

fn codes(diagnostics: &[Diagnostic]) -> BTreeSet<Code> {
    diagnostics.iter().map(|d| d.code).collect()
}

#[test]
fn three_golden_irs_have_no_diagnostic() {
    for nome in ["agendar_visita", "triagem_suporte", "confirmar_entrega"] {
        let diagnostic = validar(&ir(&golden(nome)));
        assert!(
            diagnostic.is_empty(),
            "{nome} tem diagnóstico: {diagnostic:?}"
        );
    }
}

struct Case {
    nome: &'static str,
    base: &'static str,
    code: Code,
    mutation: fn(&mut Value),
    passage: &'static str,
}

fn campo(id: &str) -> Value {
    json!({"id": id, "rotulo": "Extra", "tipo": "texto", "obrigatorio": false, "opcoes": [], "ajuda": null})
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            nome: "duas etapas com o mesmo id",
            base: "confirmar_entrega",
            code: Code::DuplicateId,
            mutation: |v| v["etapas"][1]["id"] = json!("identificacao"),
            passage: "etapas[1]",
        },
        Case {
            nome: "dois campos com o mesmo id na etapa",
            base: "confirmar_entrega",
            code: Code::DuplicateId,
            mutation: |v| v["etapas"][2]["campos"] = json!([campo("extra"), campo("extra")]),
            passage: "campos[1]",
        },
        Case {
            nome: "dois fins com o mesmo id",
            base: "confirmar_entrega",
            code: Code::DuplicateId,
            mutation: |v| {
                let first_result = v["fins"][0].clone();
                v["fins"][1] = first_result;
            },
            passage: "fins[1]",
        },
        Case {
            nome: "ação que vai para etapa inexistente",
            base: "confirmar_entrega",
            code: Code::UnknownDestination,
            mutation: |v| v["etapas"][0]["acoes"][0]["vai_para"]["id"] = json!("nao_existe"),
            passage: "> vai_para",
        },
        Case {
            nome: "início que não é etapa",
            base: "confirmar_entrega",
            code: Code::UnknownDestination,
            mutation: |v| v["inicio"] = json!("nao_existe"),
            passage: "inicio",
        },
        Case {
            nome: "decisão que vai para decisão inexistente",
            base: "triagem_suporte",
            code: Code::UnknownDestination,
            mutation: |v| {
                v["decisoes"][0]["entao"] = json!({"tipo": "decisao", "id": "nao_existe"})
            },
            passage: "> entao",
        },
        Case {
            nome: "destino para fim que não está declarado em fins",
            base: "confirmar_entrega",
            code: Code::UnknownDestination,
            mutation: |v| v["etapas"][1]["acoes"][0]["vai_para"]["id"] = json!("outro_fim"),
            passage: "> vai_para",
        },
        Case {
            nome: "mesmo fim com resultado diferente do declarado",
            base: "confirmar_entrega",
            code: Code::DivergentEnd,
            mutation: |v| v["fins"][0]["resultado"] = json!("Outra coisa"),
            passage: "> vai_para",
        },
        Case {
            nome: "exige campo que a etapa não tem",
            base: "confirmar_entrega",
            code: Code::UnknownRequirement,
            mutation: |v| v["etapas"][0]["acoes"][0]["exige"] = json!(["cep"]),
            passage: "> exige",
        },
        Case {
            nome: "campo opcao com uma opção só",
            base: "confirmar_entrega",
            code: Code::InvalidOptions,
            mutation: |v| v["etapas"][1]["campos"][0]["opcoes"] = json!(["sim"]),
            passage: "campos[0]",
        },
        Case {
            nome: "campo de texto com opções",
            base: "confirmar_entrega",
            code: Code::InvalidOptions,
            mutation: |v| v["etapas"][0]["campos"][0]["opcoes"] = json!(["sim", "nao"]),
            passage: "campos[0]",
        },
        Case {
            nome: "ação de ator que a etapa não permite",
            base: "confirmar_entrega",
            code: Code::InvalidRuleOrActor,
            mutation: |v| v["etapas"][0]["acoes"][0]["ator"] = json!("atendente"),
            passage: "> ator",
        },
        Case {
            nome: "decisão que chama def inexistente",
            base: "triagem_suporte",
            code: Code::InvalidRuleOrActor,
            mutation: |v| v["decisoes"][0]["se"]["args"][1]["nome"] = json!("nao_existe"),
            passage: "decisoes[0] (id \"classificar\") > se",
        },
        Case {
            nome: "regra que compara campo sem etapa",
            base: "triagem_suporte",
            code: Code::MalformedCombinator,
            mutation: |v| {
                v["decisoes"][0]["se"] =
                    json!({"tipo": "igual", "campo": "sem_ponto", "valor": "sim"})
            },
            passage: "> se",
        },
        Case {
            nome: "regra que compara campo inexistente",
            base: "triagem_suporte",
            code: Code::UnknownField,
            mutation: |v| {
                v["decisoes"][0]["se"] =
                    json!({"tipo": "igual", "campo": "relato.inexistente", "valor": "sim"})
            },
            passage: "> se",
        },
        Case {
            nome: "decisão que aponta para si mesma",
            base: "triagem_suporte",
            code: Code::DecisionCycle,
            mutation: |v| {
                v["decisoes"][0]["senao"] = json!({"tipo": "decisao", "id": "classificar"})
            },
            passage: "decisoes[0] (id \"classificar\")",
        },
        Case {
            nome: "fluxo acima do teto de nós",
            base: "agendar_visita",
            code: Code::NodeLimit,
            mutation: |v| {
                let modelo = v["etapas"][0].clone();
                let list = v["etapas"].as_array_mut().expect("etapas é lista");
                for i in 0..60 {
                    let mut new_stage = modelo.clone();
                    new_stage["id"] = json!(format!("extra_{i}"));
                    list.push(new_stage);
                }
            },
            passage: "fluxo (id",
        },
        Case {
            nome: "etapa sem ação",
            base: "confirmar_entrega",
            code: Code::StageWithoutAction,
            mutation: |v| v["etapas"][2]["acoes"] = json!([]),
            passage: "etapas[2]",
        },
        Case {
            nome: "etapa inalcançável",
            base: "confirmar_entrega",
            code: Code::Unreachable,
            mutation: |v| {
                let mut new_stage = v["etapas"][0].clone();
                new_stage["id"] = json!("solta");
                new_stage["nome"] = json!("Solta");
                new_stage["acoes"][0]["id"] = json!("sair");
                new_stage["acoes"][0]["vai_para"] = json!({"tipo": "fim", "id": "concluido", "resultado": "Entrega confirmada", "encerra_atendimento": true});
                v["etapas"].as_array_mut().expect("lista").push(new_stage);
            },
            passage: "etapas[3]",
        },
        Case {
            nome: "campo que ninguém usa",
            base: "confirmar_entrega",
            code: Code::UnusedField,
            mutation: |v| v["etapas"][2]["campos"] = json!([campo("extra")]),
            passage: "campos[0]",
        },
        Case {
            nome: "comparação com valor fora das opções",
            base: "triagem_suporte",
            code: Code::OutsideOptions,
            mutation: |v| {
                v["decisoes"][0]["se"] =
                    json!({"tipo": "igual", "campo": "relato.afeta_agora", "valor": "talvez"})
            },
            passage: "> se",
        },
        Case {
            nome: "condição que sempre dá o mesmo resultado",
            base: "triagem_suporte",
            code: Code::ContradictoryCondition,
            mutation: |v| {
                v["decisoes"][0]["se"] = json!({
                    "tipo": "e",
                    "args": [
                        {"tipo": "igual", "campo": "relato.afeta_agora", "valor": "sim"},
                        {"tipo": "diferente", "campo": "relato.afeta_agora", "valor": "sim"}
                    ]
                })
            },
            passage: "> se",
        },
        Case {
            nome: "maior em campo que não é número",
            base: "triagem_suporte",
            code: Code::TextComparison,
            mutation: |v| {
                v["decisoes"][0]["se"] =
                    json!({"tipo": "maior", "campo": "relato.resumo", "valor": "10"})
            },
            passage: "> se",
        },
        Case {
            nome: "def que nenhuma decisão usa",
            base: "triagem_suporte",
            code: Code::UnusedDef,
            mutation: |v| {
                v["decisoes"][0]["se"] =
                    json!({"tipo": "igual", "campo": "relato.afeta_agora", "valor": "sim"})
            },
            passage: "defs[0] (nome \"problema_grave\")",
        },
        Case {
            nome: "mensagem com variável de formato inválido",
            base: "confirmar_entrega",
            code: Code::InvalidVariable,
            mutation: |v| v["etapas"][1]["ao_entrar"][0]["texto"] = json!("Olá {cliente.nome}"),
            passage: "etapas[1] (id \"confirmacao\") > ao_entrar[0] > texto",
        },
        Case {
            nome: "mensagem com variável de campo inexistente",
            base: "confirmar_entrega",
            code: Code::VariableWithoutField,
            mutation: |v| {
                v["etapas"][1]["ao_entrar"][0]["texto"] =
                    json!("Vou confirmar o pedido {identificacao.numero_pedido}.")
            },
            passage: "etapas[1] (id \"confirmacao\") > ao_entrar[0] > texto",
        },
        Case {
            nome: "ao_entrar com variável do próprio passo",
            base: "confirmar_entrega",
            code: Code::VariableFromOwnStep,
            mutation: |v| {
                v["etapas"][1]["ao_entrar"][0]["texto"] =
                    json!("A entrega foi recebida? {confirmacao.recebido}")
            },
            passage: "etapas[1] (id \"confirmacao\") > ao_entrar[0] > texto",
        },
        Case {
            nome: "combinador `nao` com duas condições",
            base: "triagem_suporte",
            code: Code::MalformedCombinator,
            mutation: |v| {
                v["decisoes"][0]["se"] = json!({
                    "tipo": "nao",
                    "args": [
                        {"tipo": "igual", "campo": "relato.afeta_agora", "valor": "sim"},
                        {"tipo": "igual", "campo": "relato.resumo", "valor": "x"}
                    ]
                })
            },
            passage: "decisoes[0] (id \"classificar\") > se",
        },
    ]
}

#[test]
fn each_code_semantic_catches_ir_outside() {
    let mut seen: BTreeSet<Code> = BTreeSet::new();
    let mut failures: Vec<String> = Vec::new();
    for case in cases() {
        let mut value = golden(case.base);
        (case.mutation)(&mut value);
        let diagnostic = validar(&ir(&value));
        let Some(finding) = diagnostic.iter().find(|d| d.code == case.code) else {
            failures.push(format!(
                "{}: {} não apareceu (tem {:?})",
                case.nome,
                case.code,
                codes(&diagnostic)
            ));
            continue;
        };
        let path = finding.path().unwrap_or("<sem caminho>");
        if !path.contains(case.passage) {
            failures.push(format!(
                "{}: caminho `{path}` não contém `{}`",
                case.nome, case.passage
            ));
        }
        if finding.line().is_some() || finding.column().is_some() {
            failures.push(format!(
                "{}: {} veio com linha {:?} e coluna {:?}",
                case.nome,
                case.code,
                finding.line(),
                finding.column()
            ));
        }
        seen.insert(case.code);
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    for required in [
        Code::DuplicateId,
        Code::UnknownDestination,
        Code::DivergentEnd,
        Code::UnknownRequirement,
        Code::InvalidOptions,
        Code::InvalidRuleOrActor,
        Code::MalformedCombinator,
        Code::UnknownField,
        Code::DecisionCycle,
        Code::NodeLimit,
        Code::StageWithoutAction,
        Code::Unreachable,
        Code::UnusedField,
        Code::OutsideOptions,
        Code::ContradictoryCondition,
        Code::TextComparison,
        Code::UnusedDef,
        Code::InvalidVariable,
        Code::VariableWithoutField,
        Code::VariableFromOwnStep,
    ] {
        assert!(
            seen.contains(&required),
            "{required} sem caso de IR de fora"
        );
    }
}

#[test]
fn diagnostics_and_nodes_order_does_not_change_between_calls() {
    let mut value = golden("confirmar_entrega");
    value["etapas"][0]["acoes"][0]["exige"] = json!(["pedido", "cep"]);
    value["etapas"][1]["campos"][0]["opcoes"] = json!(["sim"]);
    value["etapas"][2]["acoes"] = json!([]);

    let first_pass = validar(&ir(&value));
    let second_pass = validar(&ir(&value));
    assert_eq!(first_pass, second_pass, "mesma IR tem de dar a mesma lista");

    let codes: Vec<Code> = first_pass.iter().map(|d| d.code).collect();
    assert_eq!(
        codes,
        vec![
            Code::UnknownRequirement,
            Code::InvalidOptions,
            Code::StageWithoutAction,
        ],
        "os diagnósticos saem na ordem dos nós da IR: {first_pass:?}"
    );
    let paths: Vec<&str> = first_pass.iter().map(|d| d.path().unwrap_or("")).collect();
    assert!(paths[0].starts_with("etapas[0]"), "{paths:?}");
    assert!(paths[1].starts_with("etapas[1]"), "{paths:?}");
    assert!(paths[2].starts_with("etapas[2]"), "{paths:?}");
}

#[test]
fn validator_agrees_with_compiler_notices() {
    for (code, fonte) in [
        (
            Code::StageWithoutAction,
            script("", "[]", ""),
        ),
        (
            Code::UnusedField,
            script(
                "campo(\"nome\", \"Nome\"), campo(\"extra\", \"Extra\")",
                "[acao(\"seguir\", \"Seguir\", exige = [\"nome\"], vai_para = fim(\"pronto\"))]",
                "",
            ),
        ),
        (
            Code::OutsideOptions,
            [
                script(
                    "campo(\"periodo\", \"Período\", tipo = \"opcao\", opcoes = [\"manhã\", \"tarde\"])",
                    "[acao(\"seguir\", \"Seguir\", vai_para = fim(\"pronto\"))]",
                    "",
                ),
                decisao("igual(\"entrada.periodo\", \"noite\")"),
            ]
            .concat(),
        ),
        (
            Code::ContradictoryCondition,
            [
                script(
                    "campo(\"periodo\", \"Período\", tipo = \"opcao\", opcoes = [\"manhã\", \"tarde\"])",
                    "[acao(\"seguir\", \"Seguir\", vai_para = fim(\"pronto\"))]",
                    "",
                ),
                decisao(
                    "e(igual(\"entrada.periodo\", \"manhã\"), diferente(\"entrada.periodo\", \"manhã\"))",
                ),
            ]
            .concat(),
        ),
        (
            Code::TextComparison,
            [
                script(
                    "campo(\"nome\", \"Nome\")",
                    "[acao(\"seguir\", \"Seguir\", vai_para = fim(\"pronto\"))]",
                    "",
                ),
                decisao("maior(\"entrada.nome\", \"10\")"),
            ]
            .concat(),
        ),
        (
            Code::UnusedDef,
            format!(
                "{}def problema_grave(dados):\n    return True\n",
                script(
                    "campo(\"nome\", \"Nome\")",
                    "[acao(\"seguir\", \"Seguir\", exige = [\"nome\"], vai_para = fim(\"pronto\"))]",
                    "",
                )
            ),
        ),
        (
            Code::ActorWithoutAction,
            script(
                "campo(\"nome\", \"Nome\")",
                "[acao(\"seguir\", \"Seguir\", exige = [\"nome\"], ator = \"agente\", vai_para = fim(\"pronto\"))]",
                "atores = [\"agente\", \"atendente\"],",
            ),
        ),
        (
            Code::VariableWithoutField,
            script(
                "campo(\"nome\", \"Nome\")",
                "[acao(\"seguir\", \"Seguir\", exige = [\"nome\"], vai_para = fim(\"pronto\"))]",
                "ao_entrar = [mensagem_ao_cliente(\"Olá, {entrada.inexistente}.\")],",
            ),
        ),
        (
            Code::VariableFromOwnStep,
            script(
                "campo(\"nome\", \"Nome\")",
                "[acao(\"seguir\", \"Seguir\", exige = [\"nome\"], vai_para = fim(\"pronto\"))]",
                "ao_entrar = [mensagem_ao_cliente(\"Olá, {entrada.nome}.\")],",
            ),
        ),
        (
            Code::Unreachable,
            [
                script(
                    "campo(\"nome\", \"Nome\")",
                    "[acao(\"seguir\", \"Seguir\", exige = [\"nome\"], vai_para = fim(\"pronto\"))]",
                    "",
                ),
                "etapa(id = \"solta\", nome = \"Solta\", acoes = [acao(\"sair\", \"Sair\", vai_para = fim(\"pronto\"))])\n".to_owned(),
            ]
            .concat(),
        ),
    ] {
        let compiled = compile(&fonte, &Limites::default());
        assert!(
            compiled.ir.is_some(),
            "{code}: o script de teste tem de gerar IR; diagnósticos: {:?}",
            compiled.diagnostics
        );
        assert!(
            codes(&compiled.diagnostics).contains(&code),
            "{code}: o compilador não emitiu (tem {:?})",
            codes(&compiled.diagnostics)
        );
        let of_ir = validar(compiled.ir.as_ref().expect("IR"));
        assert!(
            codes(&of_ir).contains(&code),
            "{code}: o validador não emitiu sobre a IR (tem {:?})",
            codes(&of_ir)
        );
    }
}

fn script(campos: &str, actions: &str, extra_step: &str) -> String {
    format!(
        r#"fluxo(
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
    {extra_step}
    campos = [{campos}],
    acoes = {actions},
)
"#
    )
}

fn decisao(rule: &str) -> String {
    format!("decisao(id = \"escolha\", se = {rule}, entao = \"entrada\", senao = \"entrada\")\n")
}

fn forwards_for_department(nome: &str) -> Ir {
    let mut value = golden("confirmar_entrega");
    value["etapas"][2]["acoes"][0]["efeitos"][0]["departamento"] = json!(nome);
    ir(&value)
}

#[test]
fn maps_mentioned_department_to_org_id() {
    let fluxo = ir(&golden("confirmar_entrega"));
    let departments = vec![Departamento {
        id: "dep_logistica".to_owned(),
        nome: "Logística".to_owned(),
    }];
    let (resolutions, diagnostics) = resolve_departments(&fluxo, &departments);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(
        resolutions,
        vec![poph_flows::Resolution {
            nome: "Logística".to_owned(),
            department_id: Some("dep_logistica".to_owned()),
        }]
    );
}

#[test]
fn department_org_lacks_becomes_flx65_notice_without_id() {
    let fluxo = ir(&golden("confirmar_entrega"));
    let (resolutions, diagnostics) = resolve_departments(
        &fluxo,
        &[Departamento {
            id: "dep_x".to_owned(),
            nome: "Financeiro".to_owned(),
        }],
    );
    assert_eq!(resolutions.len(), 1);
    assert_eq!(resolutions[0].nome, "Logística");
    assert_eq!(resolutions[0].department_id, None);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, Code::UnknownDepartment);
    assert!(!diagnostics[0].is_error(), "FLX65 é aviso, não erro");
    assert!(
        diagnostics[0].path().is_some_and(
            |path| path.contains("etapas[2] (id \"extravio\") > acoes[0] > efeitos[0]")
        ),
        "{:?}",
        diagnostics[0].path()
    );
    assert!(diagnostics[0].mensagem.contains("Logística"));
}

#[test]
fn repeated_name_in_org_becomes_notice_and_does_not_resolve() {
    let fluxo = ir(&golden("confirmar_entrega"));
    let departments = vec![
        Departamento {
            id: "dep_1".to_owned(),
            nome: "Logística".to_owned(),
        },
        Departamento {
            id: "dep_2".to_owned(),
            nome: " logística".to_owned(),
        },
    ];
    let (resolutions, diagnostics) = resolve_departments(&fluxo, &departments);
    assert_eq!(resolutions[0].department_id, None);
    assert_eq!(diagnostics.len(), 1);
    assert!(
        diagnostics[0].mensagem.contains("2 departamentos"),
        "{:?}",
        diagnostics[0].mensagem
    );
}

#[test]
fn matching_ignores_case_and_spaces_but_not_accent() {
    let fluxo = forwards_for_department("  LOGÍSTICA ");
    let (resolutions, diagnostics) = resolve_departments(
        &fluxo,
        &[Departamento {
            id: "dep_1".to_owned(),
            nome: "Logística".to_owned(),
        }],
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(resolutions[0].department_id.as_deref(), Some("dep_1"));
    let fluxo = forwards_for_department("Logistica");
    let (resolutions, diagnostics) = resolve_departments(
        &fluxo,
        &[Departamento {
            id: "dep_1".to_owned(),
            nome: "Logística".to_owned(),
        }],
    );
    assert_eq!(resolutions[0].department_id, None);
    assert_eq!(diagnostics.len(), 1);
}

#[test]
fn flow_without_forwarding_has_no_effect_and_does_not_warn() {
    let (resolutions, diagnostics) = resolve_departments(&ir(&golden("agendar_visita")), &[]);
    assert!(resolutions.is_empty());
    assert!(diagnostics.is_empty());
}

#[test]
fn valid_variable_does_not_become_notice() {
    let mut value = golden("confirmar_entrega");
    value["etapas"][1]["ao_entrar"][0]["texto"] =
        json!("Pedido {identificacao.pedido} do cliente {cliente.peer} (ficha {ficha.id}).");
    value["etapas"][1]["acoes"][0]["efeitos"][0]["mensagem"] =
        json!("Pedido {identificacao.pedido} entregue para {cliente.peer}.");
    let diagnostic = validar(&ir(&value));
    assert!(diagnostic.is_empty(), "{diagnostic:?}");
}

#[test]
fn name_mentioned_twice_appears_once() {
    let mut value = golden("confirmar_entrega");
    value["etapas"][0]["ao_entrar"] = json!([{"tipo": "encaminhar", "departamento": "Logística"}]);
    let fluxo = ir(&value);
    let (resolutions, diagnostics) = resolve_departments(
        &fluxo,
        &[Departamento {
            id: "dep_1".to_owned(),
            nome: "Logística".to_owned(),
        }],
    );
    assert_eq!(resolutions.len(), 1, "{resolutions:?}");
    assert_eq!(resolutions[0].department_id.as_deref(), Some("dep_1"));
    assert!(diagnostics.is_empty());
    let (resolutions, _) = resolve_departments(&fluxo, &[]);
    assert_eq!(resolutions.len(), 1);
}

#[test]
fn all_codes_are_documented() {
    let path = format!(
        "{}/../../docs/FLUXOS_LINGUAGEM.md",
        env!("CARGO_MANIFEST_DIR")
    );
    let document = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
    let missing: Vec<&str> = Code::ALL
        .iter()
        .filter(|code| !document.contains(code.as_text()))
        .map(|code| code.as_text())
        .collect();
    assert!(
        missing.is_empty(),
        "código fora do §10 do FLUXOS_LINGUAGEM.md: {missing:?}"
    );
}

fn schedule_script() -> &'static str {
    r#"fluxo(
    id = "agenda",
    nome = "Agenda",
    inicio = "entrada",
    gatilho = gatilho(
        quando = "O cliente quer agendar.",
        nao_quando = "O cliente nao quer agendar.",
        exemplos = ["quero agendar"],
    ),
)

etapa(
    id = "entrada",
    nome = "Entrada",
    campos = [
        campo("data", "Data", obrigatorio = True),
        campo("fim", "Fim"),
        campo("endereco", "Endereço"),
        campo("executor", "Executor"),
    ],
    acoes = [acao(
        "seguir",
        "Seguir",
        exige = ["data", "fim", "endereco", "executor"],
        efeitos = [sincronizar_agenda(
            "create",
            inicio = "{entrada.data}",
            fim = "{entrada.fim}",
            titulo = "Visita {ficha.id}",
            local = "{entrada.endereco}",
            executor = "{entrada.executor}",
        )],
        vai_para = "remarcar",
    )],
)

etapa(
    id = "remarcar",
    nome = "Remarcar",
    ao_entrar = [sincronizar_agenda("update", inicio = "{entrada.data}")],
    acoes = [acao("fechar", "Fechar", efeitos = [sincronizar_agenda("remove")], vai_para = fim("pronto"))],
)
"#
}

fn schedule_value() -> Value {
    let compiled = compile(schedule_script(), &Limites::default());
    assert!(compiled.ir.is_some(), "{:?}", compiled.diagnostics);
    let ir = compiled.ir.expect("IR da agenda");
    serde_json::to_value(&ir).expect("IR em JSON")
}

#[test]
fn calendar_effect_valid_ir_has_no_diagnostic() {
    let diagnostic = validar(&ir(&schedule_value()));
    assert!(diagnostic.is_empty(), "{diagnostic:?}");
}

#[test]
fn calendar_effect_shape_is_validated_on_external_ir() {
    type Mutation = fn(&mut Value);
    let mutations: Vec<(&str, Mutation, Code)> = vec![
        (
            "operação desconhecida",
            |v| v["etapas"][0]["acoes"][0]["efeitos"][0]["operacao"] = json!("delete"),
            Code::InvalidEffect,
        ),
        (
            "create sem inicio",
            |v| v["etapas"][0]["acoes"][0]["efeitos"][0]["inicio"] = Value::Null,
            Code::InvalidEffect,
        ),
        (
            "inicio malformado",
            |v| v["etapas"][0]["acoes"][0]["efeitos"][0]["inicio"] = json!("22/11/2026 14:42"),
            Code::InvalidEffect,
        ),
        (
            "inicio sem fuso",
            |v| v["etapas"][0]["acoes"][0]["efeitos"][0]["inicio"] = json!("2027-01-10T18:00:00"),
            Code::InvalidEffect,
        ),
        (
            "fim antes do inicio",
            |v| {
                v["etapas"][0]["acoes"][0]["efeitos"][0]["inicio"] =
                    json!("2027-01-10T18:00:00-03:00");
                v["etapas"][0]["acoes"][0]["efeitos"][0]["fim"] =
                    json!("2027-01-10T17:00:00-03:00");
            },
            Code::InvalidEffect,
        ),
        (
            "remove com argumento",
            |v| {
                v["etapas"][0]["acoes"][0]["efeitos"][0]["operacao"] = json!("remove");
                v["etapas"][0]["acoes"][0]["efeitos"][0]["titulo"] = json!("Visita");
            },
            Code::InvalidEffect,
        ),
        (
            "variável inexistente no inicio",
            |v| v["etapas"][0]["acoes"][0]["efeitos"][0]["inicio"] = json!("{entrada.nao_existe}"),
            Code::VariableWithoutField,
        ),
        (
            "variável com formato inválido",
            |v| v["etapas"][0]["acoes"][0]["efeitos"][0]["inicio"] = json!("{cliente.nome}"),
            Code::InvalidVariable,
        ),
    ];
    for (nome, mutation, code) in mutations {
        let mut value = schedule_value();
        mutation(&mut value);
        let diagnostic = validar(&ir(&value));
        assert!(
            diagnostic.iter().any(|d| d.code == code),
            "{nome}: esperava {code}, veio {diagnostic:?}"
        );
    }
}
