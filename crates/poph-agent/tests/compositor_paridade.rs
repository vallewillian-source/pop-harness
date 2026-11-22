//! Paridade do compositor com o POP Atende, byte a byte.
//!
//! As fixturas são `<nome>.json` (entrada do formulário) e `<nome>.esperado.txt` (saída real de
//! `core/agent/instructions_composer.compose_instructions`). Elas foram geradas por
//! `fixtures/compositor/gerar_referencia.py`, que importa o compositor do backend e chama a mesma
//! função que o `PATCH /agents/{id}/instructions` chama. Nada do backend foi alterado.
//!
//! Se este teste falhar, o Harness mudou o texto do prompt do agente — o que quebra a paridade com
//! o POP Atende e o cache de prefixo do provedor.

use poph_agent::compositor::compor_instrucoes;
use poph_types::StructuredInstructions;

/// Pares (nome, JSON de entrada, texto esperado) versionados no repositório.
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
fn paridade_byte_a_byte_com_o_pop_atende() {
    for (nome, entrada, esperado) in FIXTURAS {
        let dados = StructuredInstructions::de_json_tolerante(entrada)
            .unwrap_or_else(|erro| panic!("fixtura `{nome}` não é JSON válido: {erro}"));
        let obtido = compor_instrucoes(&dados);
        assert_eq!(
            obtido, *esperado,
            "saída diferente da referência do POP Atende na fixtura `{nome}`\n\
             --- obtido ---\n{obtido}\n--- esperado ---\n{esperado}"
        );
    }
}

#[test]
fn exemplo_do_documento_de_paridade_tem_658_caracteres_e_677_bytes() {
    let (_nome, entrada, esperado) = FIXTURAS
        .iter()
        .find(|(nome, _, _)| *nome == "exemplo_bia")
        .expect("fixtura exemplo_bia");
    let texto = compor_instrucoes(&StructuredInstructions::de_json_tolerante(entrada).unwrap());
    assert_eq!(texto, *esperado);
    assert_eq!(texto.chars().count(), 658, "contagem de caracteres do §9.3");
    assert_eq!(texto.len(), 677, "contagem de bytes UTF-8 do §9.3");
    assert!(
        !texto.ends_with('\n'),
        "o texto do compositor não termina em \\n"
    );
}

#[test]
fn composicao_e_deterministica() {
    for (nome, entrada, _esperado) in FIXTURAS {
        let dados = StructuredInstructions::de_json_tolerante(entrada).unwrap();
        let primeira = compor_instrucoes(&dados);
        let segunda = compor_instrucoes(&dados);
        assert_eq!(
            primeira.as_bytes(),
            segunda.as_bytes(),
            "duas composições da fixtura `{nome}` divergiram"
        );
    }
}

#[test]
fn caso_de_borda_identity_sem_nome_nem_empresa_nao_emite_cabecalho() {
    // §9.3: o gate é `name or company`; só papel + descrição não emitem identidade.
    let dados = StructuredInstructions::de_json_tolerante(
        r#"{"identity":{"role":"atendente","company_description":"perdida"}}"#,
    )
    .unwrap();
    let texto = compor_instrucoes(&dados);
    assert!(texto.starts_with("\n## Tom e Comunicacao\n"), "{texto}");
    assert!(!texto.contains("perdida"));
    assert!(
        !texto.starts_with("# "),
        "não pode haver cabeçalho de identidade"
    );
}

#[test]
fn caso_de_borda_empresa_sem_nome_tem_dois_espacos_no_cabecalho() {
    let dados =
        StructuredInstructions::de_json_tolerante(r#"{"identity":{"company":"FixProLar"}}"#)
            .unwrap();
    let texto = compor_instrucoes(&dados);
    assert!(texto.starts_with("#  da FixProLar\n"), "{texto}");
}

#[test]
fn caso_de_borda_ferramenta_desabilitada_nao_entra_no_markdown() {
    let dados = StructuredInstructions::de_json_tolerante(
        r#"{"tools":[{"name":"enviar_arquivo","enabled":false,"when_to_use":"catálogo"}]}"#,
    )
    .unwrap();
    let texto = compor_instrucoes(&dados);
    assert!(!texto.contains("Ferramentas Disponiveis"));
    assert!(!texto.contains("enviar_arquivo"));
}

#[test]
fn caso_de_borda_flow_steps_nao_alteram_a_saida() {
    let sem_passos: StructuredInstructions =
        StructuredInstructions::de_json_tolerante(r#"{"scope":{"does":"x"}}"#).unwrap();
    let com_passos: StructuredInstructions = StructuredInstructions::de_json_tolerante(
        r#"{"scope":{"does":"x"},"flow_steps":[{"name":"A","description":"B"}]}"#,
    )
    .unwrap();
    assert_eq!(
        compor_instrucoes(&sem_passos),
        compor_instrucoes(&com_passos)
    );
}

#[test]
fn le_o_duplo_encoding_legado_do_jsonb() {
    let objeto = r#"{"identity":{"name":"Bia"}}"#;
    let duplo = serde_json::to_string(&serde_json::Value::String(objeto.to_owned())).unwrap();
    let dados = StructuredInstructions::de_json_tolerante(&duplo).expect("lê o duplo encoding");
    assert_eq!(dados.identity.name, "Bia");
}

#[test]
fn campo_desconhecido_e_ignorado_na_leitura() {
    // `linked_template_id` (Fichas) é fora da V1 e não pode fazer a leitura falhar.
    let dados = StructuredInstructions::de_json_tolerante(r#"{"linked_template_id":"tpl_1"}"#)
        .expect("ignora campo fora da V1");
    assert_eq!(dados, StructuredInstructions::default());
}

#[test]
fn nomes_antigos_de_ferramenta_sao_recusados_na_escrita() {
    // D-04: lista fechada. Quem importa do POP Atende (HR-52) precisa mapear os nomes antigos.
    let dados = StructuredInstructions::de_json_tolerante(
        r#"{"tools":[{"name":"search_knowledge_base","enabled":true}]}"#,
    )
    .unwrap();
    let erro = dados
        .validar()
        .expect_err("nome fora da V1 deve ser recusado");
    let mensagem = erro.to_string();
    assert!(mensagem.contains("search_knowledge_base"), "{mensagem}");
    assert!(mensagem.contains("consultar_base"), "{mensagem}");
}

#[test]
fn ferramentas_da_v1_sao_aceitas() {
    let dados = StructuredInstructions::de_json_tolerante(
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
    assert_eq!(dados.ferramentas_habilitadas().len(), 8);
}

#[test]
fn ferramenta_sem_enabled_vem_habilitada_como_no_contrato() {
    let dados =
        StructuredInstructions::de_json_tolerante(r#"{"tools":[{"name":"consultar_base"}]}"#)
            .unwrap();
    assert!(dados.tools[0].enabled);
}
