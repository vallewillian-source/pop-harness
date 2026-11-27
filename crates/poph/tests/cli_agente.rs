use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use poph_agent::compose_instructions;
use poph_store::{NewDepartment, NewOrg, Store};
use poph_types::StructuredInstructions;
use serde_json::Value;

const CIPHER_SENTINEL: &str = "SENTINELA-CIFRA-hr09";
const LLM_SENTINEL: &str = "SENTINELA-LLM-hr09";

struct Scenario {
    dir: PathBuf,
    slug: String,
    department: String,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("poph-cli-agente-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("cria a pasta do teste");
        let data_dir = dir.join("dados");
        fs::create_dir_all(&data_dir).expect("cria a pasta de dados");
        let slug = format!("org-{label}");
        let department = {
            let store = Store::open_at(&data_dir.join("harness.db"), &data_dir.join("rag.db"))
                .expect("abre o banco");
            let org = store
                .orgs()
                .create(NewOrg {
                    name: format!("Org {label}"),
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
        Self {
            dir,
            slug,
            department: department.id.to_string(),
        }
    }

    fn data_dir(&self) -> PathBuf {
        self.dir.join("dados")
    }

    fn create_org(&self, slug: &str) -> String {
        let store = Store::open_at(
            &self.data_dir().join("harness.db"),
            &self.data_dir().join("rag.db"),
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

    fn create_category(&self, org_slug: &str, name: &str) -> String {
        let store = Store::open_at(
            &self.data_dir().join("harness.db"),
            &self.data_dir().join("rag.db"),
        )
        .expect("abre o banco");
        let org = store.orgs().by_slug(org_slug).expect("org da categoria");
        store
            .categories()
            .create(
                org.id,
                poph_store::NewCategory {
                    name: name.to_owned(),
                    description: String::new(),
                },
            )
            .expect("cria a categoria")
            .id
            .to_string()
    }

    fn file(&self, name: &str, content: &str) -> PathBuf {
        let path = self.dir.join(name);
        fs::write(&path, content).expect("grava o arquivo do teste");
        path
    }

    fn poph(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_poph"));
        command
            .args(args)
            .current_dir(&self.dir)
            .env_clear()
            .env("POPH_DADOS_DIRETORIO", self.data_dir())
            .env("POPH_CHAVE_CIFRA", CIPHER_SENTINEL)
            .env("ANTIGRAVITY_API_KEY", LLM_SENTINEL);
        command.output().expect("executa o poph")
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn text(output: &[u8]) -> String {
    String::from_utf8_lossy(output).into_owned()
}

fn without_secret(output: &str) {
    for sentinel in [CIPHER_SENTINEL, LLM_SENTINEL] {
        assert!(
            !output.contains(sentinel),
            "segredo vazou na saída: {output}"
        );
    }
}

const INSTRUCTIONS: &str = r#"{
  "identity": {"name": "Bia", "role": "atendente", "company": "Clínica Sorriso Vivo", "company_description": "Clínica odontológica em Niterói."},
  "principles": [{"rule": "Nunca informar preços sem confirmar no sistema.", "fallback_action": "say_dont_know"}],
  "scope": {"does": "Agenda consultas e informa horários disponíveis.", "does_not": "Não dá orientação clínica.", "handoff": {"department_id": "", "reasons": ["pedido de humano"]}},
  "tone": {"language": "pt-BR", "style": "professional", "emoji_policy": "moderate", "format": "simple_lists", "extra_restrictions": "Nunca use tabelas."},
  "tools": [{"name": "consultar_base", "enabled": true, "when_to_use": "dúvidas da clínica"}, {"name": "buscar_produto", "enabled": false, "when_to_use": ""}],
  "extra_instructions": "Sempre confirme o nome do paciente."
}"#;

#[test]
fn cli_creates_agent_with_instructions_and_lists_showing_the_composed_text() {
    let scenario = Scenario::new("cria");
    let org = scenario.slug.clone();

    let created = scenario.poph(&[
        "--org",
        &org,
        "agent",
        "create",
        "--name",
        "Bia Atendente",
        "--department",
        &scenario.department,
        "--json",
    ]);
    assert!(
        created.status.success(),
        "create falhou: {}",
        text(&created.stderr)
    );
    without_secret(&text(&created.stdout));
    let json: Value = serde_json::from_slice(&created.stdout).expect("JSON do create");
    assert_eq!(json["agent"]["slug"], "bia-atendente");
    assert_eq!(json["agent"]["abre_turno"], true);
    assert_eq!(
        json["agent"]["default_department_id"],
        scenario.department.as_str()
    );
    assert_eq!(json["agent"]["version"], 1);

    let file = scenario.file("instrucoes.json", INSTRUCTIONS);
    let saved = scenario.poph(&[
        "--org",
        &org,
        "instructions",
        "create",
        "bia-atendente",
        "--file",
        file.to_str().expect("caminho utf-8"),
        "--json",
    ]);
    assert!(
        saved.status.success(),
        "instructions create falhou: {}",
        text(&saved.stderr)
    );
    let json: Value = serde_json::from_slice(&saved.stdout).expect("JSON do instructions");
    let markdown = json["instructions"]["markdown"]
        .as_str()
        .expect("markdown no JSON")
        .to_owned();
    let expected = compose_instructions(
        &StructuredInstructions::from_tolerant_json(INSTRUCTIONS).expect("JSON de instruções"),
    );
    assert_eq!(
        markdown, expected,
        "a CLI precisa gravar o texto do compositor"
    );
    assert!(markdown.contains("# Bia, atendente da Clínica Sorriso Vivo"));
    assert!(markdown.contains("- `consultar_base` — dúvidas da clínica"));
    assert!(
        !markdown.contains("buscar_produto"),
        "ferramenta desabilitada não entra"
    );

    let shown = scenario.poph(&["--org", &org, "agent", "show", "bia-atendente"]);
    assert!(shown.status.success());
    let output = text(&shown.stdout);
    assert!(
        output.contains(&markdown),
        "show não trouxe o texto composto"
    );
    without_secret(&output);

    let list = scenario.poph(&["--org", &org, "agent", "list"]);
    assert!(list.status.success());
    assert!(text(&list.stdout).contains("bia-atendente"));

    let list_json = scenario.poph(&["--org", &org, "agent", "list", "--json"]);
    let json: Value = serde_json::from_slice(&list_json.stdout).expect("JSON do list");
    assert_eq!(json["total"], 1);
    assert_eq!(json["agentes"][0]["slug"], "bia-atendente");

    let instructions = scenario.poph(&["--org", &org, "instructions", "list", "--json"]);
    let json: Value =
        serde_json::from_slice(&instructions.stdout).expect("JSON do instructions list");
    assert_eq!(json["total"], 1);
}

#[test]
fn stale_if_match_is_refused_and_version_rises_on_every_write() {
    let scenario = Scenario::new("if-match");
    let org = scenario.slug.clone();
    scenario.poph(&[
        "--org",
        &org,
        "agent",
        "create",
        "--name",
        "Agente If Match",
    ]);
    let file = scenario.file("instrucoes.json", INSTRUCTIONS);

    let first = scenario.poph(&[
        "--org",
        &org,
        "instructions",
        "create",
        "agente-if-match",
        "--file",
        file.to_str().unwrap(),
        "--json",
    ]);
    assert!(first.status.success(), "{}", text(&first.stderr));

    let second = scenario.poph(&[
        "--org",
        &org,
        "instructions",
        "update",
        "agente-if-match",
        "--if-match",
        "1",
        "--file",
        file.to_str().unwrap(),
        "--json",
    ]);
    assert!(second.status.success(), "{}", text(&second.stderr));
    let json: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(json["instructions"]["version"], 2);

    let conflict = scenario.poph(&[
        "--org",
        &org,
        "instructions",
        "update",
        "agente-if-match",
        "--if-match",
        "1",
        "--file",
        file.to_str().unwrap(),
    ]);
    assert_eq!(conflict.status.code(), Some(1));
    assert!(text(&conflict.stderr).contains("conflito de versão"));

    let updated = scenario.poph(&[
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
    assert!(updated.status.success(), "{}", text(&updated.stderr));
    let json: Value = serde_json::from_slice(&updated.stdout).unwrap();
    assert_eq!(json["agent"]["version"], 2);
    let conflict = scenario.poph(&[
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
    assert_eq!(conflict.status.code(), Some(1));
    assert!(text(&conflict.stderr).contains("conflito de versão"));

    let without_if_match = scenario.poph(&[
        "--org",
        &org,
        "instructions",
        "update",
        "agente-if-match",
        "--file",
        file.to_str().unwrap(),
    ]);
    assert_eq!(without_if_match.status.code(), Some(1));
    assert!(text(&without_if_match.stderr).contains("--if-match"));
}

#[test]
fn d05_inactive_agent_stops_opening_turn_via_cli() {
    let scenario = Scenario::new("d05");
    let org = scenario.slug.clone();
    scenario.poph(&["--org", &org, "agent", "create", "--name", "Agente Inativo"]);

    let deactivated = scenario.poph(&[
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
    assert!(
        deactivated.status.success(),
        "{}",
        text(&deactivated.stderr)
    );
    let json: Value = serde_json::from_slice(&deactivated.stdout).unwrap();
    assert_eq!(json["agent"]["status"], "inactive");
    assert_eq!(json["agent"]["abre_turno"], false);

    let shown = scenario.poph(&["--org", &org, "agent", "show", "agente-inativo"]);
    let output = text(&shown.stdout);
    assert!(output.contains("não abre turno (D-05)"), "{output}");

    let active = scenario.poph(&["--org", &org, "agent", "list", "--only-active", "--json"]);
    let json: Value = serde_json::from_slice(&active.stdout).unwrap();
    assert_eq!(json["total"], 0);

    let all = scenario.poph(&["--org", &org, "agent", "list", "--json"]);
    let json: Value = serde_json::from_slice(&all.stdout).unwrap();
    assert_eq!(json["total"], 1, "inativo continua listado para o Console");

    let wrong = scenario.poph(&[
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
    assert_eq!(wrong.status.code(), Some(1));
    assert!(text(&wrong.stderr).contains("status"));
}

#[test]
fn one_org_neither_sees_nor_deletes_another_org_agent() {
    let scenario = Scenario::new("org-a");
    let org_a = scenario.slug.clone();
    let org_b = scenario.create_org("org-b");
    scenario.poph(&[
        "--org",
        &org_a,
        "agent",
        "create",
        "--name",
        "Agente Exclusivo",
    ]);

    let seen = scenario.poph(&["--org", &org_b, "agent", "show", "agente-exclusivo"]);
    assert_eq!(seen.status.code(), Some(1));
    assert!(text(&seen.stderr).contains("não encontrado"));

    let list = scenario.poph(&["--org", &org_b, "agent", "list"]);
    assert!(list.status.success());
    assert!(!text(&list.stdout).contains("agente-exclusivo"));

    let id = {
        let list = scenario.poph(&["--org", &org_a, "agent", "list", "--json"]);
        let json: Value = serde_json::from_slice(&list.stdout).unwrap();
        json["agentes"][0]["id"].as_str().unwrap().to_owned()
    };
    let by_id = scenario.poph(&["--org", &org_b, "agent", "show", &id]);
    assert_eq!(by_id.status.code(), Some(1));
    assert!(text(&by_id.stderr).contains("não encontrado"));

    let deletion = scenario.poph(&["--org", &org_b, "agent", "delete", &id]);
    assert_eq!(deletion.status.code(), Some(1));
    let still_there = scenario.poph(&["--org", &org_a, "agent", "show", "agente-exclusivo"]);
    assert!(
        still_there.status.success(),
        "o agente da org A não pode ser apagado pela org B"
    );
}

#[test]
fn tool_outside_v1_is_refused_with_the_engine_list() {
    let scenario = Scenario::new("ferramenta");
    let org = scenario.slug.clone();
    scenario.poph(&["--org", &org, "agent", "create", "--name", "Agente Legado"]);
    let legacy = scenario.file(
        "legado.json",
        r#"{"tools":[{"name":"search_knowledge_base","enabled":true}]}"#,
    );
    let output = scenario.poph(&[
        "--org",
        &org,
        "instructions",
        "create",
        "agente-legado",
        "--file",
        legacy.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let error = text(&output.stderr);
    assert!(error.contains("search_knowledge_base"), "{error}");
    assert!(error.contains("consultar_base"), "{error}");

    let list = scenario.poph(&["--org", &org, "instructions", "list", "--json"]);
    let json: Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(json["total"], 0);
}

#[test]
fn hand_edited_markdown_is_preserved_without_recomposition() {
    let scenario = Scenario::new("manual");
    let org = scenario.slug.clone();
    scenario.poph(&["--org", &org, "agent", "create", "--name", "Agente Manual"]);
    let manual = scenario.file("manual.md", "# Prompt escrito a mao\n\nNao mexa.");
    let output = scenario.poph(&[
        "--org",
        &org,
        "instructions",
        "create",
        "agente-manual",
        "--markdown-file",
        manual.to_str().unwrap(),
        "--json",
    ]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["instructions"]["markdown"],
        "# Prompt escrito a mao\n\nNao mexa."
    );
    assert_eq!(json["instructions"]["derived_from_json"], false);

    let deleted = scenario.poph(&["--org", &org, "instructions", "delete", "agente-manual"]);
    assert!(deleted.status.success(), "{}", text(&deleted.stderr));
    let shown = scenario.poph(&["--org", &org, "agent", "show", "agente-manual"]);
    assert!(shown.status.success());
    assert!(text(&shown.stdout).contains("sem instruções gravadas"));
}

#[test]
fn without_org_the_command_fails_saying_what_is_missing() {
    let scenario = Scenario::new("sem-org");
    let output = scenario.poph(&["agent", "list"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).contains("--org"));
}

#[test]
fn help_lists_the_two_new_subcommands() {
    let scenario = Scenario::new("ajuda");
    let output = scenario.poph(&["--help"]);
    let help = text(&output.stdout);
    assert!(help.contains("agent"), "{help}");
    assert!(help.contains("instructions"), "{help}");
}

#[test]
fn product_category_link_stays_scoped_by_org() {
    let scenario = Scenario::new("categoria");
    let org_a = scenario.slug.clone();
    let org_b = scenario.create_org("org-b-categoria");
    scenario.poph(&[
        "--org",
        &org_a,
        "agent",
        "create",
        "--name",
        "Agente Categoria",
    ]);
    let category_a = scenario.create_category(&org_a, "Bebidas");
    let category_b = scenario.create_category(&org_b, "De Outra Org");

    let linked = scenario.poph(&[
        "--org",
        &org_a,
        "agent",
        "update",
        "agente-categoria",
        "--if-match",
        "1",
        "--category-id",
        &category_a,
        "--json",
    ]);
    assert!(linked.status.success(), "{}", text(&linked.stderr));

    let shown = scenario.poph(&[
        "--org",
        &org_a,
        "agent",
        "show",
        "agente-categoria",
        "--json",
    ]);
    let json: Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(
        json["agent"]["product_category_ids"],
        serde_json::json!([category_a])
    );

    let crossed = scenario.poph(&[
        "--org",
        &org_a,
        "agent",
        "update",
        "agente-categoria",
        "--if-match",
        "2",
        "--category-id",
        &category_b,
    ]);
    assert_eq!(crossed.status.code(), Some(1));
    assert!(text(&crossed.stderr).contains("não encontrado"));
}
