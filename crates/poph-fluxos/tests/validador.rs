//! Testes do validador semântico (FX-11): revalidação de IR que **não** veio do compilador (com o
//! caminho do nó, sem linha e coluna), resolução dos departamentos de `encaminhar` e a conferência
//! de que as duas direções (compilador e validador) concordam.
//!
//! As IRs usadas aqui são as douradas do FX-10 (`tests/fluxos/ir/*.json`), mutadas **no JSON à mão**:
//! é a melhor imitação de uma importação ou de uma revisão antiga que existe sem banco.

use std::collections::BTreeSet;

use poph_fluxos::{
    Code, Departamento, Diagnostic, Ir, Limites, compilar, resolver_departamentos, validar,
};
use serde_json::{Value, json};

// ------------------------------------------------------------------ auxiliares

fn dourada(nome: &str) -> Value {
    let caminho = format!(
        "{}/../../tests/fluxos/ir/{nome}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let texto =
        std::fs::read_to_string(&caminho).unwrap_or_else(|erro| panic!("{caminho}: {erro}"));
    serde_json::from_str(&texto).unwrap_or_else(|erro| panic!("{caminho}: {erro}"))
}

fn ir(valor: &Value) -> Ir {
    Ir::from_json(&valor.to_string()).expect("a IR mutada continua sendo JSON válido")
}

fn codigos(diagnosticos: &[Diagnostic]) -> BTreeSet<Code> {
    diagnosticos.iter().map(|d| d.code).collect()
}

// ------------------------------------------------------------------ a IR boa passa limpa

#[test]
fn as_tres_irs_douradas_nao_tem_nenhum_diagnostico() {
    // O outro lado de cada caso: a IR que veio do compilador é aceita pelo validador sem nenhum
    // diagnóstico. Se as duas direções divergirem, este teste cai.
    for nome in ["agendar_visita", "triagem_suporte", "confirmar_entrega"] {
        let diagnostico = validar(&ir(&dourada(nome)));
        assert!(
            diagnostico.is_empty(),
            "{nome} tem diagnóstico: {diagnostico:?}"
        );
    }
}

// ------------------------------------------------------------------ IR de fora, um caso por código

/// Um caso de revalidação: a dourada de origem, a mutação no JSON e o código que tem de aparecer.
struct Caso {
    nome: &'static str,
    base: &'static str,
    codigo: Code,
    mutacao: fn(&mut Value),
    /// Trecho que o caminho do nó tem de conter.
    trecho: &'static str,
}

fn campo(id: &str) -> Value {
    json!({"id": id, "rotulo": "Extra", "tipo": "texto", "obrigatorio": false, "opcoes": [], "ajuda": null})
}

fn casos() -> Vec<Caso> {
    vec![
        Caso {
            nome: "duas etapas com o mesmo id",
            base: "confirmar_entrega",
            codigo: Code::DuplicateId,
            mutacao: |v| v["etapas"][1]["id"] = json!("identificacao"),
            trecho: "etapas[1]",
        },
        Caso {
            nome: "dois campos com o mesmo id na etapa",
            base: "confirmar_entrega",
            codigo: Code::DuplicateId,
            mutacao: |v| v["etapas"][2]["campos"] = json!([campo("extra"), campo("extra")]),
            trecho: "campos[1]",
        },
        Caso {
            nome: "dois fins com o mesmo id",
            base: "confirmar_entrega",
            codigo: Code::DuplicateId,
            mutacao: |v| {
                let primeiro = v["fins"][0].clone();
                v["fins"][1] = primeiro;
            },
            trecho: "fins[1]",
        },
        Caso {
            nome: "ação que vai para etapa inexistente",
            base: "confirmar_entrega",
            codigo: Code::UnknownDestination,
            mutacao: |v| v["etapas"][0]["acoes"][0]["vai_para"]["id"] = json!("nao_existe"),
            trecho: "> vai_para",
        },
        Caso {
            nome: "início que não é etapa",
            base: "confirmar_entrega",
            codigo: Code::UnknownDestination,
            mutacao: |v| v["inicio"] = json!("nao_existe"),
            trecho: "inicio",
        },
        Caso {
            nome: "decisão que vai para decisão inexistente",
            base: "triagem_suporte",
            codigo: Code::UnknownDestination,
            mutacao: |v| v["decisoes"][0]["entao"] = json!({"tipo": "decisao", "id": "nao_existe"}),
            trecho: "> entao",
        },
        Caso {
            nome: "destino para fim que não está declarado em fins",
            base: "confirmar_entrega",
            codigo: Code::UnknownDestination,
            mutacao: |v| v["etapas"][1]["acoes"][0]["vai_para"]["id"] = json!("outro_fim"),
            trecho: "> vai_para",
        },
        Caso {
            nome: "mesmo fim com resultado diferente do declarado",
            base: "confirmar_entrega",
            codigo: Code::DivergentEnd,
            mutacao: |v| v["fins"][0]["resultado"] = json!("Outra coisa"),
            trecho: "> vai_para",
        },
        Caso {
            nome: "exige campo que a etapa não tem",
            base: "confirmar_entrega",
            codigo: Code::UnknownRequirement,
            mutacao: |v| v["etapas"][0]["acoes"][0]["exige"] = json!(["cep"]),
            trecho: "> exige",
        },
        Caso {
            nome: "campo opcao com uma opção só",
            base: "confirmar_entrega",
            codigo: Code::InvalidOptions,
            mutacao: |v| v["etapas"][1]["campos"][0]["opcoes"] = json!(["sim"]),
            trecho: "campos[0]",
        },
        Caso {
            nome: "campo de texto com opções",
            base: "confirmar_entrega",
            codigo: Code::InvalidOptions,
            mutacao: |v| v["etapas"][0]["campos"][0]["opcoes"] = json!(["sim", "nao"]),
            trecho: "campos[0]",
        },
        Caso {
            nome: "ação de ator que a etapa não permite",
            base: "confirmar_entrega",
            codigo: Code::InvalidRuleOrActor,
            mutacao: |v| v["etapas"][0]["acoes"][0]["ator"] = json!("atendente"),
            trecho: "> ator",
        },
        Caso {
            nome: "decisão que chama def inexistente",
            base: "triagem_suporte",
            codigo: Code::InvalidRuleOrActor,
            mutacao: |v| v["decisoes"][0]["se"]["args"][1]["nome"] = json!("nao_existe"),
            trecho: "decisoes[0] (id \"classificar\") > se",
        },
        Caso {
            nome: "regra que compara campo sem etapa",
            base: "triagem_suporte",
            codigo: Code::MalformedCombinator,
            mutacao: |v| {
                v["decisoes"][0]["se"] =
                    json!({"tipo": "igual", "campo": "sem_ponto", "valor": "sim"})
            },
            trecho: "> se",
        },
        Caso {
            nome: "regra que compara campo inexistente",
            base: "triagem_suporte",
            codigo: Code::UnknownField,
            mutacao: |v| {
                v["decisoes"][0]["se"] =
                    json!({"tipo": "igual", "campo": "relato.inexistente", "valor": "sim"})
            },
            trecho: "> se",
        },
        Caso {
            nome: "decisão que aponta para si mesma",
            base: "triagem_suporte",
            codigo: Code::DecisionCycle,
            mutacao: |v| {
                v["decisoes"][0]["senao"] = json!({"tipo": "decisao", "id": "classificar"})
            },
            trecho: "decisoes[0] (id \"classificar\")",
        },
        Caso {
            nome: "fluxo acima do teto de nós",
            base: "agendar_visita",
            codigo: Code::NodeLimit,
            mutacao: |v| {
                let modelo = v["etapas"][0].clone();
                let lista = v["etapas"].as_array_mut().expect("etapas é lista");
                for i in 0..60 {
                    let mut nova = modelo.clone();
                    nova["id"] = json!(format!("extra_{i}"));
                    lista.push(nova);
                }
            },
            trecho: "fluxo (id",
        },
        Caso {
            nome: "etapa sem ação",
            base: "confirmar_entrega",
            codigo: Code::StageWithoutAction,
            mutacao: |v| v["etapas"][2]["acoes"] = json!([]),
            trecho: "etapas[2]",
        },
        Caso {
            nome: "etapa inalcançável",
            base: "confirmar_entrega",
            codigo: Code::Unreachable,
            mutacao: |v| {
                let mut nova = v["etapas"][0].clone();
                nova["id"] = json!("solta");
                nova["nome"] = json!("Solta");
                nova["acoes"][0]["id"] = json!("sair");
                nova["acoes"][0]["vai_para"] = json!({"tipo": "fim", "id": "concluido", "resultado": "Entrega confirmada", "encerra_atendimento": true});
                v["etapas"].as_array_mut().expect("lista").push(nova);
            },
            trecho: "etapas[3]",
        },
        Caso {
            nome: "campo que ninguém usa",
            base: "confirmar_entrega",
            codigo: Code::UnusedField,
            mutacao: |v| v["etapas"][2]["campos"] = json!([campo("extra")]),
            trecho: "campos[0]",
        },
        Caso {
            nome: "comparação com valor fora das opções",
            base: "triagem_suporte",
            codigo: Code::OutsideOptions,
            mutacao: |v| {
                v["decisoes"][0]["se"] =
                    json!({"tipo": "igual", "campo": "relato.afeta_agora", "valor": "talvez"})
            },
            trecho: "> se",
        },
        Caso {
            nome: "condição que sempre dá o mesmo resultado",
            base: "triagem_suporte",
            codigo: Code::ContradictoryCondition,
            mutacao: |v| {
                v["decisoes"][0]["se"] = json!({
                    "tipo": "e",
                    "args": [
                        {"tipo": "igual", "campo": "relato.afeta_agora", "valor": "sim"},
                        {"tipo": "diferente", "campo": "relato.afeta_agora", "valor": "sim"}
                    ]
                })
            },
            trecho: "> se",
        },
        Caso {
            nome: "maior em campo que não é número",
            base: "triagem_suporte",
            codigo: Code::TextComparison,
            mutacao: |v| {
                v["decisoes"][0]["se"] =
                    json!({"tipo": "maior", "campo": "relato.resumo", "valor": "10"})
            },
            trecho: "> se",
        },
        Caso {
            nome: "def que nenhuma decisão usa",
            base: "triagem_suporte",
            codigo: Code::UnusedDef,
            mutacao: |v| {
                v["decisoes"][0]["se"] =
                    json!({"tipo": "igual", "campo": "relato.afeta_agora", "valor": "sim"})
            },
            trecho: "defs[0] (nome \"problema_grave\")",
        },
        Caso {
            nome: "mensagem com variável de formato inválido",
            base: "confirmar_entrega",
            codigo: Code::InvalidVariable,
            mutacao: |v| v["etapas"][1]["ao_entrar"][0]["texto"] = json!("Olá {cliente.nome}"),
            trecho: "etapas[1] (id \"confirmacao\") > ao_entrar[0] > texto",
        },
        Caso {
            nome: "mensagem com variável de campo inexistente",
            base: "confirmar_entrega",
            codigo: Code::VariableWithoutField,
            mutacao: |v| {
                v["etapas"][1]["ao_entrar"][0]["texto"] =
                    json!("Vou confirmar o pedido {identificacao.numero_pedido}.")
            },
            trecho: "etapas[1] (id \"confirmacao\") > ao_entrar[0] > texto",
        },
        Caso {
            nome: "ao_entrar com variável do próprio passo",
            base: "confirmar_entrega",
            codigo: Code::VariableFromOwnStep,
            mutacao: |v| {
                v["etapas"][1]["ao_entrar"][0]["texto"] =
                    json!("A entrega foi recebida? {confirmacao.recebido}")
            },
            trecho: "etapas[1] (id \"confirmacao\") > ao_entrar[0] > texto",
        },
        Caso {
            nome: "combinador `nao` com duas condições",
            base: "triagem_suporte",
            codigo: Code::MalformedCombinator,
            mutacao: |v| {
                v["decisoes"][0]["se"] = json!({
                    "tipo": "nao",
                    "args": [
                        {"tipo": "igual", "campo": "relato.afeta_agora", "valor": "sim"},
                        {"tipo": "igual", "campo": "relato.resumo", "valor": "x"}
                    ]
                })
            },
            trecho: "decisoes[0] (id \"classificar\") > se",
        },
    ]
}

#[test]
fn cada_codigo_semantico_pega_uma_ir_de_fora() {
    let mut vistos: BTreeSet<Code> = BTreeSet::new();
    let mut falhas: Vec<String> = Vec::new();
    for caso in casos() {
        let mut valor = dourada(caso.base);
        (caso.mutacao)(&mut valor);
        let diagnostico = validar(&ir(&valor));
        let Some(achado) = diagnostico.iter().find(|d| d.code == caso.codigo) else {
            falhas.push(format!(
                "{}: {} não apareceu (tem {:?})",
                caso.nome,
                caso.codigo,
                codigos(&diagnostico)
            ));
            continue;
        };
        let caminho = achado.path().unwrap_or("<sem caminho>");
        if !caminho.contains(caso.trecho) {
            falhas.push(format!(
                "{}: caminho `{caminho}` não contém `{}`",
                caso.nome, caso.trecho
            ));
        }
        // Diagnóstico de IR não fala de linha: não existe texto para apontar.
        if achado.line().is_some() || achado.column().is_some() {
            falhas.push(format!(
                "{}: {} veio com linha {:?} e coluna {:?}",
                caso.nome,
                caso.codigo,
                achado.line(),
                achado.column()
            ));
        }
        vistos.insert(caso.codigo);
    }
    assert!(falhas.is_empty(), "\n{}", falhas.join("\n"));
    // Os códigos que a revalidação de IR precisa cobrir: sem estes, uma IR de fora passaria torta.
    for exigido in [
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
            vistos.contains(&exigido),
            "{exigido} sem caso de IR de fora"
        );
    }
}

#[test]
fn a_ordem_dos_diagnosticos_e_a_dos_nos_e_nao_muda_entre_chamadas() {
    let mut valor = dourada("confirmar_entrega");
    // Três problemas em nós diferentes, sem mexer no grafo (para o alcance não entrar na conta):
    // uma ação que exige campo inexistente na etapa 0, uma opção só na etapa 1, etapa sem ação na 2.
    valor["etapas"][0]["acoes"][0]["exige"] = json!(["pedido", "cep"]);
    valor["etapas"][1]["campos"][0]["opcoes"] = json!(["sim"]);
    valor["etapas"][2]["acoes"] = json!([]);

    let primeira = validar(&ir(&valor));
    let segunda = validar(&ir(&valor));
    assert_eq!(primeira, segunda, "mesma IR tem de dar a mesma lista");

    let codigos: Vec<Code> = primeira.iter().map(|d| d.code).collect();
    assert_eq!(
        codigos,
        vec![
            Code::UnknownRequirement,
            Code::InvalidOptions,
            Code::StageWithoutAction,
        ],
        "os diagnósticos saem na ordem dos nós da IR: {primeira:?}"
    );
    let caminhos: Vec<&str> = primeira.iter().map(|d| d.path().unwrap_or("")).collect();
    assert!(caminhos[0].starts_with("etapas[0]"), "{caminhos:?}");
    assert!(caminhos[1].starts_with("etapas[1]"), "{caminhos:?}");
    assert!(caminhos[2].starts_with("etapas[2]"), "{caminhos:?}");
}

#[test]
fn o_validador_concorda_com_os_avisos_do_compilador() {
    // As duas direções sobre o mesmo fluxo: o compilador acha o aviso no texto, o validador acha o
    // mesmo aviso na IR que ele gerou. É a guarda contra as duas implementações divergirem.
    for (codigo, fonte) in [
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
        let compilado = compilar(&fonte, &Limites::default());
        assert!(
            compilado.ir.is_some(),
            "{codigo}: o script de teste tem de gerar IR; diagnósticos: {:?}",
            compilado.diagnosticos
        );
        assert!(
            codigos(&compilado.diagnosticos).contains(&codigo),
            "{codigo}: o compilador não emitiu (tem {:?})",
            codigos(&compilado.diagnosticos)
        );
        let da_ir = validar(compilado.ir.as_ref().expect("IR"));
        assert!(
            codigos(&da_ir).contains(&codigo),
            "{codigo}: o validador não emitiu sobre a IR (tem {:?})",
            codigos(&da_ir)
        );
    }
}

/// Fluxo mínimo com uma etapa `entrada`, para o teste de concordância.
fn script(campos: &str, acoes: &str, extra_etapa: &str) -> String {
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
    {extra_etapa}
    campos = [{campos}],
    acoes = {acoes},
)
"#
    )
}

fn decisao(regra: &str) -> String {
    format!("decisao(id = \"escolha\", se = {regra}, entao = \"entrada\", senao = \"entrada\")\n")
}

// ------------------------------------------------------------------ departamentos

fn encaminha_para_departamento(nome: &str) -> Ir {
    let mut valor = dourada("confirmar_entrega");
    valor["etapas"][2]["acoes"][0]["efeitos"][0]["departamento"] = json!(nome);
    ir(&valor)
}

#[test]
fn resolve_o_departamento_citado_para_o_id_da_organizacao() {
    let fluxo = ir(&dourada("confirmar_entrega"));
    let departamentos = vec![Departamento {
        id: "dep_logistica".to_owned(),
        nome: "Logística".to_owned(),
    }];
    let (resolucoes, diagnosticos) = resolver_departamentos(&fluxo, &departamentos);
    assert!(diagnosticos.is_empty(), "{diagnosticos:?}");
    assert_eq!(
        resolucoes,
        vec![poph_fluxos::Resolucao {
            nome: "Logística".to_owned(),
            departamento_id: Some("dep_logistica".to_owned()),
        }]
    );
}

#[test]
fn departamento_que_a_organizacao_nao_tem_vira_aviso_flx65_sem_id() {
    let fluxo = ir(&dourada("confirmar_entrega"));
    let (resolucoes, diagnosticos) = resolver_departamentos(
        &fluxo,
        &[Departamento {
            id: "dep_x".to_owned(),
            nome: "Financeiro".to_owned(),
        }],
    );
    assert_eq!(resolucoes.len(), 1);
    assert_eq!(resolucoes[0].nome, "Logística");
    assert_eq!(resolucoes[0].departamento_id, None);
    assert_eq!(diagnosticos.len(), 1);
    assert_eq!(diagnosticos[0].code, Code::UnknownDepartment);
    assert!(!diagnosticos[0].is_error(), "FLX65 é aviso, não erro");
    assert!(
        diagnosticos[0].path().is_some_and(
            |caminho| caminho.contains("etapas[2] (id \"extravio\") > acoes[0] > efeitos[0]")
        ),
        "{:?}",
        diagnosticos[0].path()
    );
    assert!(diagnosticos[0].mensagem.contains("Logística"));
}

#[test]
fn nome_repetido_na_organizacao_vira_aviso_e_nao_resolve() {
    let fluxo = ir(&dourada("confirmar_entrega"));
    let departamentos = vec![
        Departamento {
            id: "dep_1".to_owned(),
            nome: "Logística".to_owned(),
        },
        Departamento {
            id: "dep_2".to_owned(),
            nome: " logística".to_owned(),
        },
    ];
    let (resolucoes, diagnosticos) = resolver_departamentos(&fluxo, &departamentos);
    assert_eq!(resolucoes[0].departamento_id, None);
    assert_eq!(diagnosticos.len(), 1);
    assert!(
        diagnosticos[0].mensagem.contains("2 departamentos"),
        "{:?}",
        diagnosticos[0].mensagem
    );
}

#[test]
fn o_casamento_ignora_caixa_e_espacos_mas_nao_acento() {
    // Mesmo nome com outra caixa e com espaço nas pontas casa.
    let fluxo = encaminha_para_departamento("  LOGÍSTICA ");
    let (resolucoes, diagnosticos) = resolver_departamentos(
        &fluxo,
        &[Departamento {
            id: "dep_1".to_owned(),
            nome: "Logística".to_owned(),
        }],
    );
    assert!(diagnosticos.is_empty(), "{diagnosticos:?}");
    assert_eq!(resolucoes[0].departamento_id.as_deref(), Some("dep_1"));
    // Acento conta: `Logistica` não é `Logística`.
    let fluxo = encaminha_para_departamento("Logistica");
    let (resolucoes, diagnosticos) = resolver_departamentos(
        &fluxo,
        &[Departamento {
            id: "dep_1".to_owned(),
            nome: "Logística".to_owned(),
        }],
    );
    assert_eq!(resolucoes[0].departamento_id, None);
    assert_eq!(diagnosticos.len(), 1);
}

#[test]
fn fluxo_sem_encaminhamento_nao_resolve_nada_e_nao_avisa() {
    let (resolucoes, diagnosticos) = resolver_departamentos(&ir(&dourada("agendar_visita")), &[]);
    assert!(resolucoes.is_empty());
    assert!(diagnosticos.is_empty());
}

#[test]
fn variavel_valida_nao_vira_aviso() {
    let mut valor = dourada("confirmar_entrega");
    // `{ficha.id}` já aparece no exemplo; `{cliente.peer}` e uma variável de campo existente têm de
    // passar limpas, inclusive dentro de uma ação e falando de outra etapa.
    valor["etapas"][1]["ao_entrar"][0]["texto"] =
        json!("Pedido {identificacao.pedido} do cliente {cliente.peer} (ficha {ficha.id}).");
    valor["etapas"][1]["acoes"][0]["efeitos"][0]["mensagem"] =
        json!("Pedido {identificacao.pedido} entregue para {cliente.peer}.");
    let diagnostico = validar(&ir(&valor));
    assert!(diagnostico.is_empty(), "{diagnostico:?}");
}

#[test]
fn o_nome_citado_duas_vezes_aparece_uma_vez_so() {
    let mut valor = dourada("confirmar_entrega");
    // O mesmo departamento no `ao_entrar` de uma etapa e nos efeitos de uma ação.
    valor["etapas"][0]["ao_entrar"] = json!([{"tipo": "encaminhar", "departamento": "Logística"}]);
    let fluxo = ir(&valor);
    let (resolucoes, diagnosticos) = resolver_departamentos(
        &fluxo,
        &[Departamento {
            id: "dep_1".to_owned(),
            nome: "Logística".to_owned(),
        }],
    );
    assert_eq!(resolucoes.len(), 1, "{resolucoes:?}");
    assert_eq!(resolucoes[0].departamento_id.as_deref(), Some("dep_1"));
    assert!(diagnosticos.is_empty());
    // A ordem é a da primeira citação na IR: `ao_entrar` da etapa 0 vem antes do efeito da ação.
    let (resolucoes, _) = resolver_departamentos(&fluxo, &[]);
    assert_eq!(resolucoes.len(), 1);
}

// ------------------------------------------------------------------ a lista fechada na doc

#[test]
fn todos_os_codigos_estao_documentados() {
    let caminho = format!(
        "{}/../../docs/FLUXOS_LINGUAGEM.md",
        env!("CARGO_MANIFEST_DIR")
    );
    let documento =
        std::fs::read_to_string(&caminho).unwrap_or_else(|erro| panic!("{caminho}: {erro}"));
    let faltando: Vec<&str> = Code::ALL
        .iter()
        .filter(|codigo| !documento.contains(codigo.as_text()))
        .map(|codigo| codigo.as_text())
        .collect();
    assert!(
        faltando.is_empty(),
        "código fora do §10 do FLUXOS_LINGUAGEM.md: {faltando:?}"
    );
}
