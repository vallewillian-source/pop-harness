//! Testes de ponta a ponta de `poph agent` e `poph instructions`: processo de verdade, banco de
//! verdade em pasta temporária, ambiente controlado.
//!
//! A org é semeada pelo armazenamento (o `poph org` é a HR-08 e ainda não existe). Os segredos
//! usados aqui são sentinelas: os testes conferem que eles não aparecem na saída.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use poph_agent::compor_instrucoes;
use poph_store::{NewDepartment, NewOrg, Store};
use poph_types::StructuredInstructions;
use serde_json::Value;

const SENTINELA_CIFRA: &str = "SENTINELA-CIFRA-hr09";
const SENTINELA_LLM: &str = "SENTINELA-LLM-hr09";

/// Pasta temporária com um banco já migrado e uma org pronta para a CLI.
struct Cenario {
    pasta: PathBuf,
    slug: String,
    departamento: String,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta =
            std::env::temp_dir().join(format!("poph-cli-agente-{rotulo}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("cria a pasta do teste");
        let dados = pasta.join("dados");
        fs::create_dir_all(&dados).expect("cria a pasta de dados");
        let slug = format!("org-{rotulo}");
        let departamento = {
            let store = Store::open_at(&dados.join("harness.db"), &dados.join("rag.db"))
                .expect("abre o banco");
            let org = store
                .orgs()
                .create(NewOrg {
                    name: format!("Org {rotulo}"),
                    slug: slug.clone(),
                    config_json: String::new(),
                })
                .expect("cria a org");
            store
                .departments()
                .create(
                    org.id,
                    NewDepartment {
                        name: "Atendimento".to_owned(),
                        description: String::new(),
                    },
                )
                .expect("cria o departamento")
        };
        // O `Store` acima é fechado aqui (o escritor dedicado encerra) antes da CLI abrir o banco.
        Self {
            pasta,
            slug,
            departamento: departamento.id.to_string(),
        }
    }

    fn dados(&self) -> PathBuf {
        self.pasta.join("dados")
    }

    /// Cria uma segunda org na mesma pasta e devolve o slug.
    fn criar_org(&self, slug: &str) -> String {
        let store = Store::open_at(
            &self.dados().join("harness.db"),
            &self.dados().join("rag.db"),
        )
        .expect("abre o banco");
        store
            .orgs()
            .create(NewOrg {
                name: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org");
        slug.to_owned()
    }

    /// Cria uma categoria de produto na org informada e devolve o id.
    fn criar_categoria(&self, org_slug: &str, nome: &str) -> String {
        let store = Store::open_at(
            &self.dados().join("harness.db"),
            &self.dados().join("rag.db"),
        )
        .expect("abre o banco");
        let org = store.orgs().by_slug(org_slug).expect("org da categoria");
        store
            .categories()
            .create(
                org.id,
                poph_store::NewCategory {
                    name: nome.to_owned(),
                    description: String::new(),
                },
            )
            .expect("cria a categoria")
            .id
            .to_string()
    }

    fn arquivo(&self, nome: &str, conteudo: &str) -> PathBuf {
        let caminho = self.pasta.join(nome);
        fs::write(&caminho, conteudo).expect("grava o arquivo do teste");
        caminho
    }

    fn poph(&self, args: &[&str]) -> Output {
        let mut comando = Command::new(env!("CARGO_BIN_EXE_poph"));
        comando
            .args(args)
            .current_dir(&self.pasta)
            .env_clear()
            .env("POPH_DADOS_DIRETORIO", self.dados())
            .env("POPH_CHAVE_CIFRA", SENTINELA_CIFRA)
            .env("ANTIGRAVITY_API_KEY", SENTINELA_LLM);
        comando.output().expect("executa o poph")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn texto(saida: &[u8]) -> String {
    String::from_utf8_lossy(saida).into_owned()
}

fn sem_segredo(saida: &str) {
    for sentinela in [SENTINELA_CIFRA, SENTINELA_LLM] {
        assert!(
            !saida.contains(sentinela),
            "segredo vazou na saída: {saida}"
        );
    }
}

/// JSON de instruções com nomes de ferramenta da V1.
const INSTRUCOES: &str = r#"{
  "identity": {"name": "Bia", "role": "atendente", "company": "Clínica Sorriso Vivo", "company_description": "Clínica odontológica em Niterói."},
  "principles": [{"rule": "Nunca informar preços sem confirmar no sistema.", "fallback_action": "say_dont_know"}],
  "scope": {"does": "Agenda consultas e informa horários disponíveis.", "does_not": "Não dá orientação clínica.", "handoff": {"department_id": "", "reasons": ["pedido de humano"]}},
  "tone": {"language": "pt-BR", "style": "professional", "emoji_policy": "moderate", "format": "simple_lists", "extra_restrictions": "Nunca use tabelas."},
  "tools": [{"name": "consultar_base", "enabled": true, "when_to_use": "dúvidas da clínica"}, {"name": "buscar_produto", "enabled": false, "when_to_use": ""}],
  "extra_instructions": "Sempre confirme o nome do paciente."
}"#;

#[test]
fn cli_cria_agente_com_instrucoes_e_lista_mostrando_o_texto_composto() {
    let cenario = Cenario::novo("cria");
    let org = cenario.slug.clone();

    let criado = cenario.poph(&[
        "--org",
        &org,
        "agent",
        "create",
        "--name",
        "Bia Atendente",
        "--department",
        &cenario.departamento,
        "--json",
    ]);
    assert!(
        criado.status.success(),
        "create falhou: {}",
        texto(&criado.stderr)
    );
    sem_segredo(&texto(&criado.stdout));
    let json: Value = serde_json::from_slice(&criado.stdout).expect("JSON do create");
    assert_eq!(json["agent"]["slug"], "bia-atendente");
    assert_eq!(json["agent"]["abre_turno"], true);
    assert_eq!(
        json["agent"]["default_department_id"],
        cenario.departamento.as_str()
    );
    assert_eq!(json["agent"]["version"], 1);

    let arquivo = cenario.arquivo("instrucoes.json", INSTRUCOES);
    let gravado = cenario.poph(&[
        "--org",
        &org,
        "instructions",
        "create",
        "bia-atendente",
        "--file",
        arquivo.to_str().expect("caminho utf-8"),
        "--json",
    ]);
    assert!(
        gravado.status.success(),
        "instructions create falhou: {}",
        texto(&gravado.stderr)
    );
    let json: Value = serde_json::from_slice(&gravado.stdout).expect("JSON do instructions");
    let markdown = json["instructions"]["markdown"]
        .as_str()
        .expect("markdown no JSON")
        .to_owned();
    let esperado = compor_instrucoes(
        &StructuredInstructions::from_tolerant_json(INSTRUCOES).expect("JSON de instruções"),
    );
    assert_eq!(
        markdown, esperado,
        "a CLI precisa gravar o texto do compositor"
    );
    assert!(markdown.contains("# Bia, atendente da Clínica Sorriso Vivo"));
    assert!(markdown.contains("- `consultar_base` — dúvidas da clínica"));
    assert!(
        !markdown.contains("buscar_produto"),
        "ferramenta desabilitada não entra"
    );

    // `show` mostra o texto composto; `list` mostra o agente.
    let mostrado = cenario.poph(&["--org", &org, "agent", "show", "bia-atendente"]);
    assert!(mostrado.status.success());
    let saida = texto(&mostrado.stdout);
    assert!(
        saida.contains(&markdown),
        "show não trouxe o texto composto"
    );
    sem_segredo(&saida);

    let lista = cenario.poph(&["--org", &org, "agent", "list"]);
    assert!(lista.status.success());
    assert!(texto(&lista.stdout).contains("bia-atendente"));

    let lista_json = cenario.poph(&["--org", &org, "agent", "list", "--json"]);
    let json: Value = serde_json::from_slice(&lista_json.stdout).expect("JSON do list");
    assert_eq!(json["total"], 1);
    assert_eq!(json["agentes"][0]["slug"], "bia-atendente");

    let instrucoes = cenario.poph(&["--org", &org, "instructions", "list", "--json"]);
    let json: Value =
        serde_json::from_slice(&instrucoes.stdout).expect("JSON do instructions list");
    assert_eq!(json["total"], 1);
}

#[test]
fn if_match_velho_e_recusado_e_versao_sobe_a_cada_escrita() {
    let cenario = Cenario::novo("if-match");
    let org = cenario.slug.clone();
    cenario.poph(&[
        "--org",
        &org,
        "agent",
        "create",
        "--name",
        "Agente If Match",
    ]);
    let arquivo = cenario.arquivo("instrucoes.json", INSTRUCOES);

    let primeira = cenario.poph(&[
        "--org",
        &org,
        "instructions",
        "create",
        "agente-if-match",
        "--file",
        arquivo.to_str().unwrap(),
        "--json",
    ]);
    assert!(primeira.status.success(), "{}", texto(&primeira.stderr));

    let segunda = cenario.poph(&[
        "--org",
        &org,
        "instructions",
        "update",
        "agente-if-match",
        "--if-match",
        "1",
        "--file",
        arquivo.to_str().unwrap(),
        "--json",
    ]);
    assert!(segunda.status.success(), "{}", texto(&segunda.stderr));
    let json: Value = serde_json::from_slice(&segunda.stdout).unwrap();
    assert_eq!(json["instructions"]["version"], 2);

    let conflito = cenario.poph(&[
        "--org",
        &org,
        "instructions",
        "update",
        "agente-if-match",
        "--if-match",
        "1",
        "--file",
        arquivo.to_str().unwrap(),
    ]);
    assert_eq!(conflito.status.code(), Some(1));
    assert!(texto(&conflito.stderr).contains("conflito de versão"));

    // Agente: versão 1 → 2, e a versão 1 deixa de valer.
    let alterado = cenario.poph(&[
        "--org",
        &org,
        "agent",
        "update",
        "agente-if-match",
        "--if-match",
        "1",
        "--description",
        "nova descrição",
        "--json",
    ]);
    assert!(alterado.status.success(), "{}", texto(&alterado.stderr));
    let json: Value = serde_json::from_slice(&alterado.stdout).unwrap();
    assert_eq!(json["agent"]["version"], 2);
    let conflito = cenario.poph(&[
        "--org",
        &org,
        "agent",
        "update",
        "agente-if-match",
        "--if-match",
        "1",
        "--description",
        "de novo",
    ]);
    assert_eq!(conflito.status.code(), Some(1));
    assert!(texto(&conflito.stderr).contains("conflito de versão"));

    // Sem `--if-match` no update de instruções: erro claro, sem tocar no banco.
    let sem_if_match = cenario.poph(&[
        "--org",
        &org,
        "instructions",
        "update",
        "agente-if-match",
        "--file",
        arquivo.to_str().unwrap(),
    ]);
    assert_eq!(sem_if_match.status.code(), Some(1));
    assert!(texto(&sem_if_match.stderr).contains("--if-match"));
}

#[test]
fn d05_agente_inativo_para_de_abrir_turno_pela_cli() {
    let cenario = Cenario::novo("d05");
    let org = cenario.slug.clone();
    cenario.poph(&["--org", &org, "agent", "create", "--name", "Agente Inativo"]);

    let inativado = cenario.poph(&[
        "--org",
        &org,
        "agent",
        "update",
        "agente-inativo",
        "--if-match",
        "1",
        "--status",
        "inactive",
        "--json",
    ]);
    assert!(inativado.status.success(), "{}", texto(&inativado.stderr));
    let json: Value = serde_json::from_slice(&inativado.stdout).unwrap();
    assert_eq!(json["agent"]["status"], "inactive");
    assert_eq!(json["agent"]["abre_turno"], false);

    let mostrado = cenario.poph(&["--org", &org, "agent", "show", "agente-inativo"]);
    let saida = texto(&mostrado.stdout);
    assert!(saida.contains("não abre turno (D-05)"), "{saida}");

    let ativos = cenario.poph(&["--org", &org, "agent", "list", "--only-active", "--json"]);
    let json: Value = serde_json::from_slice(&ativos.stdout).unwrap();
    assert_eq!(json["total"], 0);

    let todos = cenario.poph(&["--org", &org, "agent", "list", "--json"]);
    let json: Value = serde_json::from_slice(&todos.stdout).unwrap();
    assert_eq!(json["total"], 1, "inativo continua listado para o Console");

    // Status desconhecido é recusado.
    let errado = cenario.poph(&[
        "--org",
        &org,
        "agent",
        "update",
        "agente-inativo",
        "--if-match",
        "2",
        "--status",
        "dormindo",
    ]);
    assert_eq!(errado.status.code(), Some(1));
    assert!(texto(&errado.stderr).contains("status"));
}

#[test]
fn uma_org_nao_ve_nem_apaga_o_agente_de_outra() {
    let cenario = Cenario::novo("org-a");
    let org_a = cenario.slug.clone();
    let org_b = cenario.criar_org("org-b");
    cenario.poph(&[
        "--org",
        &org_a,
        "agent",
        "create",
        "--name",
        "Agente Exclusivo",
    ]);

    let visto = cenario.poph(&["--org", &org_b, "agent", "show", "agente-exclusivo"]);
    assert_eq!(visto.status.code(), Some(1));
    assert!(texto(&visto.stderr).contains("não encontrado"));

    let lista = cenario.poph(&["--org", &org_b, "agent", "list"]);
    assert!(lista.status.success());
    assert!(!texto(&lista.stdout).contains("agente-exclusivo"));

    // Pelo id também não: a leitura é escopada por org.
    let id = {
        let lista = cenario.poph(&["--org", &org_a, "agent", "list", "--json"]);
        let json: Value = serde_json::from_slice(&lista.stdout).unwrap();
        json["agentes"][0]["id"].as_str().unwrap().to_owned()
    };
    let por_id = cenario.poph(&["--org", &org_b, "agent", "show", &id]);
    assert_eq!(por_id.status.code(), Some(1));
    assert!(texto(&por_id.stderr).contains("não encontrado"));

    let exclusao = cenario.poph(&["--org", &org_b, "agent", "delete", &id]);
    assert_eq!(exclusao.status.code(), Some(1));
    let ainda_la = cenario.poph(&["--org", &org_a, "agent", "show", "agente-exclusivo"]);
    assert!(
        ainda_la.status.success(),
        "o agente da org A não pode ser apagado pela org B"
    );
}

#[test]
fn ferramenta_fora_da_v1_e_recusada_com_a_lista_do_motor() {
    let cenario = Cenario::novo("ferramenta");
    let org = cenario.slug.clone();
    cenario.poph(&["--org", &org, "agent", "create", "--name", "Agente Legado"]);
    let legado = cenario.arquivo(
        "legado.json",
        r#"{"tools":[{"name":"search_knowledge_base","enabled":true}]}"#,
    );
    let saida = cenario.poph(&[
        "--org",
        &org,
        "instructions",
        "create",
        "agente-legado",
        "--file",
        legado.to_str().unwrap(),
    ]);
    assert_eq!(saida.status.code(), Some(1));
    let erro = texto(&saida.stderr);
    assert!(erro.contains("search_knowledge_base"), "{erro}");
    assert!(erro.contains("consultar_base"), "{erro}");

    // Nada foi gravado.
    let lista = cenario.poph(&["--org", &org, "instructions", "list", "--json"]);
    let json: Value = serde_json::from_slice(&lista.stdout).unwrap();
    assert_eq!(json["total"], 0);
}

#[test]
fn markdown_editado_a_mao_e_preservado_sem_recomposicao() {
    let cenario = Cenario::novo("manual");
    let org = cenario.slug.clone();
    cenario.poph(&["--org", &org, "agent", "create", "--name", "Agente Manual"]);
    let manual = cenario.arquivo("manual.md", "# Prompt escrito a mao\n\nNao mexa.");
    let saida = cenario.poph(&[
        "--org",
        &org,
        "instructions",
        "create",
        "agente-manual",
        "--markdown-file",
        manual.to_str().unwrap(),
        "--json",
    ]);
    assert!(saida.status.success(), "{}", texto(&saida.stderr));
    let json: Value = serde_json::from_slice(&saida.stdout).unwrap();
    assert_eq!(
        json["instructions"]["markdown"],
        "# Prompt escrito a mao\n\nNao mexa."
    );
    assert_eq!(json["instructions"]["derived_from_json"], false);

    // Apagar as instruções não apaga o agente.
    let apagado = cenario.poph(&["--org", &org, "instructions", "delete", "agente-manual"]);
    assert!(apagado.status.success(), "{}", texto(&apagado.stderr));
    let mostrado = cenario.poph(&["--org", &org, "agent", "show", "agente-manual"]);
    assert!(mostrado.status.success());
    assert!(texto(&mostrado.stdout).contains("sem instruções gravadas"));
}

#[test]
fn sem_org_o_comando_falha_dizendo_o_que_falta() {
    let cenario = Cenario::novo("sem-org");
    let saida = cenario.poph(&["agent", "list"]);
    assert_eq!(saida.status.code(), Some(1));
    assert!(texto(&saida.stderr).contains("--org"));
}

#[test]
fn ajuda_lista_os_dois_novos_subcomandos() {
    let cenario = Cenario::novo("ajuda");
    let saida = cenario.poph(&["--help"]);
    let ajuda = texto(&saida.stdout);
    assert!(ajuda.contains("agent"), "{ajuda}");
    assert!(ajuda.contains("instructions"), "{ajuda}");
}

#[test]
fn vinculo_de_categorias_de_produto_fica_escopado_por_org() {
    let cenario = Cenario::novo("categoria");
    let org_a = cenario.slug.clone();
    let org_b = cenario.criar_org("org-b-categoria");
    cenario.poph(&[
        "--org",
        &org_a,
        "agent",
        "create",
        "--name",
        "Agente Categoria",
    ]);
    let categoria_a = cenario.criar_categoria(&org_a, "Bebidas");
    let categoria_b = cenario.criar_categoria(&org_b, "De Outra Org");

    let vinculado = cenario.poph(&[
        "--org",
        &org_a,
        "agent",
        "update",
        "agente-categoria",
        "--if-match",
        "1",
        "--category-id",
        &categoria_a,
        "--json",
    ]);
    assert!(vinculado.status.success(), "{}", texto(&vinculado.stderr));

    let mostrado = cenario.poph(&[
        "--org",
        &org_a,
        "agent",
        "show",
        "agente-categoria",
        "--json",
    ]);
    let json: Value = serde_json::from_slice(&mostrado.stdout).unwrap();
    assert_eq!(
        json["agent"]["product_category_ids"],
        serde_json::json!([categoria_a])
    );

    // Categoria de outra org é recusada (o vínculo não atravessa o isolamento).
    let cruzado = cenario.poph(&[
        "--org",
        &org_a,
        "agent",
        "update",
        "agente-categoria",
        "--if-match",
        "2",
        "--category-id",
        &categoria_b,
    ]);
    assert_eq!(cruzado.status.code(), Some(1));
    assert!(texto(&cruzado.stderr).contains("não encontrado"));
}
