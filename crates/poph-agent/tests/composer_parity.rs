use poph_agent::compositor::compose_instructions;
use poph_types::StructuredInstructions;

const FIXTURAS: &[(&str, &str, &str)] = &[
    (
        "exemplo_bia",
        include_str!("fixtures/compositor/exemplo_bia.json"),
        include_str!("fixtures/compositor/exemplo_bia.esperado.txt"),
    ),
    (
        "vazio",
        include_str!("fixtures/compositor/vazio.json"),
        include_str!("fixtures/compositor/vazio.esperado.txt"),
    ),
    (
        "so_empresa",
        include_str!("fixtures/compositor/so_empresa.json"),
        include_str!("fixtures/compositor/so_empresa.esperado.txt"),
    ),
    (
        "so_papel",
        include_str!("fixtures/compositor/so_papel.json"),
        include_str!("fixtures/compositor/so_papel.esperado.txt"),
    ),
    (
        "principios",
        include_str!("fixtures/compositor/principios.json"),
        include_str!("fixtures/compositor/principios.esperado.txt"),
    ),
    (
        "tom_estranho",
        include_str!("fixtures/compositor/tom_estranho.json"),
        include_str!("fixtures/compositor/tom_estranho.esperado.txt"),
    ),
    (
        "ferramentas",
        include_str!("fixtures/compositor/ferramentas.json"),
        include_str!("fixtures/compositor/ferramentas.esperado.txt"),
    ),
    (
        "escopo",
        include_str!("fixtures/compositor/escopo.json"),
        include_str!("fixtures/compositor/escopo.esperado.txt"),
    ),
    (
        "identidade_sem_descricao",
        include_str!("fixtures/compositor/identidade_sem_descricao.json"),
        include_str!("fixtures/compositor/identidade_sem_descricao.esperado.txt"),
    ),
];

#[test]
fn byte_for_byte_parity_with_pop_atende() {
    for (nome, entry, expected) in FIXTURAS {
        let dados = StructuredInstructions::from_tolerant_json(entry)
            .unwrap_or_else(|error| panic!("fixtura `{nome}` não é JSON válido: {error}"));
        let obtained = compose_instructions(&dados);
        assert_eq!(
            obtained, *expected,
            "saída diferente da referência do POP Atende na fixtura `{nome}`\n\
             --- obtido ---\n{obtained}\n--- esperado ---\n{expected}"
        );
    }
}

#[test]
fn example_document_parity_has_658_characters_677_bytes() {
    let (_name, entry, expected) = FIXTURAS
        .iter()
        .find(|(nome, _, _)| *nome == "exemplo_bia")
        .expect("fixtura exemplo_bia");
    let texto = compose_instructions(&StructuredInstructions::from_tolerant_json(entry).unwrap());
    assert_eq!(texto, *expected);
    assert_eq!(texto.chars().count(), 658, "contagem de caracteres do §9.3");
    assert_eq!(texto.len(), 677, "contagem de bytes UTF-8 do §9.3");
    assert!(
        !texto.ends_with('\n'),
        "o texto do compositor não termina em \\n"
    );
}

#[test]
fn composition_is_deterministic() {
    for (nome, entry, _expected) in FIXTURAS {
        let dados = StructuredInstructions::from_tolerant_json(entry).unwrap();
        let first = compose_instructions(&dados);
        let second_one = compose_instructions(&dados);
        assert_eq!(
            first.as_bytes(),
            second_one.as_bytes(),
            "duas composições da fixtura `{nome}` divergiram"
        );
    }
}

#[test]
fn case_edge_identity_without_name_nor_company_not_emits_header() {
    let dados = StructuredInstructions::from_tolerant_json(
        r#"{"identity":{"role":"atendente","company_description":"perdida"}}"#,
    )
    .unwrap();
    let texto = compose_instructions(&dados);
    assert!(texto.starts_with("\n## Tom e Comunicacao\n"), "{texto}");
    assert!(!texto.contains("perdida"));
    assert!(
        !texto.starts_with("# "),
        "não pode haver cabeçalho de identidade"
    );
}

#[test]
fn case_edge_company_without_name_has_two_spaces_header() {
    let dados =
        StructuredInstructions::from_tolerant_json(r#"{"identity":{"company":"FixProLar"}}"#)
            .unwrap();
    let texto = compose_instructions(&dados);
    assert!(texto.starts_with("#  da FixProLar\n"), "{texto}");
}

#[test]
fn case_edge_tool_disabled_not_enters_markdown() {
    let dados = StructuredInstructions::from_tolerant_json(
        r#"{"tools":[{"name":"enviar_arquivo","enabled":false,"when_to_use":"catálogo"}]}"#,
    )
    .unwrap();
    let texto = compose_instructions(&dados);
    assert!(!texto.contains("Ferramentas Disponiveis"));
    assert!(!texto.contains("enviar_arquivo"));
}

#[test]
fn case_edge_flow_steps_not_alter_output() {
    let without_steps: StructuredInstructions =
        StructuredInstructions::from_tolerant_json(r#"{"scope":{"does":"x"}}"#).unwrap();
    let with_steps: StructuredInstructions = StructuredInstructions::from_tolerant_json(
        r#"{"scope":{"does":"x"},"flow_steps":[{"name":"A","description":"B"}]}"#,
    )
    .unwrap();
    assert_eq!(
        compose_instructions(&without_steps),
        compose_instructions(&with_steps)
    );
}

#[test]
fn reads_legacy_double_encoding_from_jsonb() {
    let object = r#"{"identity":{"name":"Bia"}}"#;
    let duplo = serde_json::to_string(&serde_json::Value::String(object.to_owned())).unwrap();
    let dados = StructuredInstructions::from_tolerant_json(&duplo).expect("lê o duplo encoding");
    assert_eq!(dados.identity.name, "Bia");
}

#[test]
fn field_unknown_ignored_read() {
    let dados = StructuredInstructions::from_tolerant_json(r#"{"linked_template_id":"tpl_1"}"#)
        .expect("ignora campo fora da V1");
    assert_eq!(dados, StructuredInstructions::default());
}

#[test]
fn names_old_tool_are_refused_write() {
    let dados = StructuredInstructions::from_tolerant_json(
        r#"{"tools":[{"name":"search_knowledge_base","enabled":true}]}"#,
    )
    .unwrap();
    let error = dados
        .validar()
        .expect_err("nome fora da V1 deve ser recusado");
    let mensagem = error.to_string();
    assert!(mensagem.contains("search_knowledge_base"), "{mensagem}");
    assert!(mensagem.contains("consultar_base"), "{mensagem}");
}

#[test]
fn tools_v1_are_accepted() {
    let dados = StructuredInstructions::from_tolerant_json(
        r#"{"tools":[
            {"name":"consultar_base","enabled":true},
            {"name":"buscar_produto","enabled":true},
            {"name":"enviar_arquivo","enabled":true},
            {"name":"transferir","enabled":true},
            {"name":"cart_add_item","enabled":true},
            {"name":"cart_remove_item","enabled":true},
            {"name":"cart_summary","enabled":true},
            {"name":"cart_clear","enabled":true}
        ]}"#,
    )
    .unwrap();
    dados.validar().expect("toda ferramenta da V1 é aceita");
    assert_eq!(dados.enabled_tools().len(), 8);
}

#[test]
fn tool_without_enabled_comes_enabled_as_contract() {
    let dados =
        StructuredInstructions::from_tolerant_json(r#"{"tools":[{"name":"consultar_base"}]}"#)
            .unwrap();
    assert!(dados.tools[0].enabled);
}
