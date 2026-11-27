use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordVerifier};
use poph_store::{NewAgent, Store};
use poph_types::{AdminTokenId, OrgId};
use serde_json::Value;

const UUID_V4: &str = "f47ac10b-58cc-4372-a567-0e02b2c3d479";

struct Scenario {
    dir: PathBuf,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("poph-cli-admin-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dados")).expect("cria a pasta de dados");
        Self { dir }
    }

    fn data_dir(&self) -> PathBuf {
        self.dir.join("dados")
    }

    fn harness(&self) -> PathBuf {
        self.data_dir().join("harness.db")
    }

    fn poph(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_poph"))
            .args(args)
            .current_dir(&self.dir)
            .env_clear()
            .env("POPH_DADOS_DIRETORIO", self.data_dir())
            .output()
            .expect("executa o poph")
    }

    fn store(&self) -> Store {
        Store::open_at(&self.harness(), &self.data_dir().join("rag.db")).expect("abre o banco")
    }

    fn create_org(&self, slug: &str) -> String {
        let output = self.poph(&[
            "--offline",
            "org",
            "create",
            "--name",
            &format!("Org {slug}"),
            "--slug",
            slug,
            "--json",
        ]);
        assert_eq!(exit_code(&output), 0, "{}", text(&output.stderr));
        json(&output)["org"]["id"]
            .as_str()
            .expect("id da org")
            .to_owned()
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn exit_code(output: &Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn json(output: &Output) -> Value {
    serde_json::from_str(&text(&output.stdout))
        .unwrap_or_else(|error| panic!("saída não é JSON ({error}): {}", text(&output.stdout)))
}

fn field(value: &Value, path: &str) -> String {
    let mut current = value;
    for part in path.split('.') {
        current = &current[part];
    }
    current
        .as_str()
        .unwrap_or_else(|| panic!("campo {path} ausente ou não é texto"))
        .to_owned()
}

#[test]
fn end_to_end_creates_and_lists_the_four() {
    let scenario = Scenario::new("ponta-a-ponta");

    let org = scenario.create_org("loja-teste");
    assert!(org.len() == 36, "id da org: {org}");

    let token = scenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "token",
        "create",
        "--label",
        "site",
        "--json",
    ]);
    assert_eq!(exit_code(&token), 0, "{}", text(&token.stderr));
    assert!(field(&json(&token), "valor_do_token").starts_with("poph_"));

    let category = scenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "category",
        "create",
        "--name",
        "Planos",
        "--description",
        "Planos de assinatura",
        "--json",
    ]);
    assert_eq!(exit_code(&category), 0, "{}", text(&category.stderr));
    let category_id = field(&json(&category), "category.id");

    let department = scenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "department",
        "create",
        "--name",
        "Vendas",
        "--json",
    ]);
    assert_eq!(exit_code(&department), 0, "{}", text(&department.stderr));
    let department_id = field(&json(&department), "department.id");

    let orgs = json(&scenario.poph(&["--offline", "org", "list", "--json"]));
    assert_eq!(orgs["total"], 1);
    let tokens = json(&scenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "token",
        "list",
        "--json",
    ]));
    assert_eq!(tokens["total"], 1);
    let categories = json(&scenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "category",
        "list",
        "--json",
    ]));
    assert_eq!(categories["total"], 1);
    let departments = json(&scenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "department",
        "list",
        "--json",
    ]));
    assert_eq!(departments["total"], 1);

    let show_org = scenario.poph(&["--offline", "org", "show", "loja-teste"]);
    assert_eq!(exit_code(&show_org), 0);
    assert!(text(&show_org.stdout).contains("Org loja-teste"));

    let show_category = scenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "category",
        "show",
        &category_id,
    ]);
    assert_eq!(exit_code(&show_category), 0);
    assert!(text(&show_category.stdout).contains("Planos"));

    let show_department = scenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "department",
        "show",
        &department_id,
        "--json",
    ]);
    assert_eq!(exit_code(&show_department), 0);
    assert_eq!(field(&json(&show_department), "department.name"), "Vendas");
}

#[test]
fn org_token_shows_once_and_stays_only_as_hash() {
    let scenario = Scenario::new("token-unico");
    let org = scenario.create_org("loja-token");

    let created = scenario.poph(&[
        "--offline",
        "--org",
        "loja-token",
        "token",
        "create",
        "--label",
        "integração",
        "--json",
    ]);
    assert_eq!(exit_code(&created), 0, "{}", text(&created.stderr));
    let value = field(&json(&created), "valor_do_token");
    let token_id = field(&json(&created), "token.id");
    assert!(value.len() > 60, "valor curto demais");

    let mut outputs = Vec::new();
    for arguments in [
        vec!["--offline", "--org", "loja-token", "token", "list"],
        vec![
            "--offline",
            "--org",
            "loja-token",
            "token",
            "list",
            "--json",
        ],
        vec![
            "--offline",
            "--org",
            "loja-token",
            "token",
            "show",
            token_id.as_str(),
        ],
        vec![
            "--offline",
            "--org",
            "loja-token",
            "token",
            "show",
            token_id.as_str(),
            "--json",
        ],
        vec!["--offline", "org", "show", org.as_str(), "--json"],
        vec!["--offline", "org", "list", "--json"],
    ] {
        let output = scenario.poph(&arguments);
        assert_eq!(
            exit_code(&output),
            0,
            "{arguments:?}: {}",
            text(&output.stderr)
        );
        outputs.push(text(&output.stdout));
        outputs.push(text(&output.stderr));
    }
    for output in &outputs {
        assert!(!output.contains(&value), "o valor do token vazou: {output}");
    }

    let show = scenario.poph(&[
        "--offline",
        "--org",
        "loja-token",
        "token",
        "show",
        token_id.as_str(),
    ]);
    assert!(
        text(&show.stdout).contains("valor: não disponível"),
        "mensagem: {}",
        text(&show.stdout)
    );

    for file in [
        scenario.harness(),
        scenario.data_dir().join("harness.db-wal"),
    ] {
        if let Ok(bytes) = fs::read(&file) {
            assert!(
                !bytes
                    .windows(value.len())
                    .any(|chunk| chunk == value.as_bytes()),
                "o valor em claro está no arquivo {}",
                file.display()
            );
        }
    }

    let store = scenario.store();
    let token = store
        .tokens()
        .get(
            org.parse::<OrgId>().expect("id da org"),
            token_id.parse::<AdminTokenId>().expect("id do token"),
        )
        .expect("token gravado");
    assert!(token.hash.starts_with("$argon2id$"), "hash: {}", token.hash);
    let rest = value.strip_prefix("poph_").expect("prefixo poph_");
    let (id_in_value, secret) = rest.split_once('_').expect("uuid e segredo no valor");
    assert_eq!(
        id_in_value, token_id,
        "o valor tem de carregar o id do token"
    );
    let parsed = PasswordHash::new(&token.hash).expect("hash no formato PHC");
    assert!(
        Argon2::default()
            .verify_password(secret.as_bytes(), &parsed)
            .is_ok(),
        "o hash não confere com o segredo"
    );
    assert!(
        Argon2::default()
            .verify_password(value.as_bytes(), &parsed)
            .is_err(),
        "o hash não pode conferir com o valor inteiro (o valor tem de ir por id)"
    );
}

#[test]
fn revoked_token_is_no_longer_active_and_repeating_revocation_is_fine() {
    let scenario = Scenario::new("token-revoga");
    scenario.create_org("loja-revoga");
    let created = json(&scenario.poph(&[
        "--offline",
        "--org",
        "loja-revoga",
        "token",
        "create",
        "--label",
        "antigo",
        "--json",
    ]));
    let token_id = field(&created, "token.id");

    let first = scenario.poph(&[
        "--offline",
        "--org",
        "loja-revoga",
        "token",
        "delete",
        &token_id,
        "--json",
    ]);
    assert_eq!(exit_code(&first), 0, "{}", text(&first.stderr));
    assert_eq!(json(&first)["ja_estava_revogado"], false);

    let show = json(&scenario.poph(&[
        "--offline",
        "--org",
        "loja-revoga",
        "token",
        "show",
        &token_id,
        "--json",
    ]));
    assert_eq!(field(&show, "token.status"), "revoked");

    let second = scenario.poph(&[
        "--offline",
        "--org",
        "loja-revoga",
        "token",
        "delete",
        &token_id,
        "--json",
    ]);
    assert_eq!(exit_code(&second), 0, "revogar de novo não é erro");
    assert_eq!(json(&second)["ja_estava_revogado"], true);
}

#[test]
fn org_with_agents_only_goes_with_force() {
    let scenario = Scenario::new("org-agentes");
    let org = scenario.create_org("org-com-agente");
    {
        let store = scenario.store();
        let id = org.parse::<OrgId>().expect("id da org");
        store
            .agents()
            .create(
                id,
                NewAgent {
                    name: "Bia".to_owned(),
                    slug: "bia".to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente");
    }

    let refused = scenario.poph(&["--offline", "org", "delete", "org-com-agente"]);
    assert_eq!(exit_code(&refused), 1, "sem --force a exclusão é recusada");
    let warning = text(&refused.stderr);
    assert!(warning.contains("--force"), "mensagem: {warning}");
    assert!(warning.contains("1 agente(s)"), "mensagem: {warning}");

    let still_exists = scenario.poph(&["--offline", "org", "show", "org-com-agente"]);
    assert_eq!(exit_code(&still_exists), 0);
    assert!(text(&still_exists.stdout).contains("agentes: 1"));

    let confirmed = scenario.poph(&[
        "--offline",
        "org",
        "delete",
        "org-com-agente",
        "--force",
        "--json",
    ]);
    assert_eq!(exit_code(&confirmed), 0, "{}", text(&confirmed.stderr));
    assert_eq!(json(&confirmed)["agentes_removidos"], 1);

    let gone = scenario.poph(&["--offline", "org", "show", "org-com-agente"]);
    assert_eq!(exit_code(&gone), 1);
    assert!(text(&gone.stderr).contains("não existe org com o slug `org-com-agente`"));
}

#[test]
fn empty_org_goes_without_force() {
    let scenario = Scenario::new("org-vazia");
    scenario.create_org("org-vazia");
    let output = scenario.poph(&["--offline", "org", "delete", "org-vazia"]);
    assert_eq!(exit_code(&output), 0, "{}", text(&output.stderr));
}

#[test]
fn org_with_category_also_asks_for_force() {
    let scenario = Scenario::new("org-conteudo");
    scenario.create_org("org-conteudo");
    let created = scenario.poph(&[
        "--offline",
        "--org",
        "org-conteudo",
        "category",
        "create",
        "--name",
        "Planos",
    ]);
    assert_eq!(exit_code(&created), 0);

    let refused = scenario.poph(&["--offline", "org", "delete", "org-conteudo"]);
    assert_eq!(exit_code(&refused), 1);
    assert!(text(&refused.stderr).contains("1 categoria(s)"));
}

#[test]
fn exit_codes_zero_one_and_two() {
    let scenario = Scenario::new("codigos");
    scenario.create_org("org-codigos");

    assert_eq!(exit_code(&scenario.poph(&["--offline", "org", "list"])), 0);
    assert_eq!(
        exit_code(&scenario.poph(&["--offline", "org", "show", "nao-existe"])),
        1
    );
    let without_offline = scenario.poph(&["org", "list"]);
    assert_eq!(exit_code(&without_offline), 1);
    assert!(text(&without_offline.stderr).contains("--offline"));
    assert_eq!(exit_code(&scenario.poph(&["--offline", "org"])), 2);
    assert_eq!(
        exit_code(&scenario.poph(&["--offline", "org", "update", "org-codigos", "--name", "X"])),
        2
    );
}

#[test]
fn error_messages_are_readable() {
    let scenario = Scenario::new("mensagens");
    scenario.create_org("org-msg");

    let bad_slug = scenario.poph(&[
        "--offline",
        "org",
        "create",
        "--name",
        "Teste",
        "--slug",
        "A_B",
    ]);
    assert_eq!(exit_code(&bad_slug), 1);
    assert!(text(&bad_slug.stderr).contains("slug `A_B` inválido"));

    let duplicate = scenario.poph(&[
        "--offline",
        "org",
        "create",
        "--name",
        "Outra",
        "--slug",
        "org-msg",
    ]);
    assert_eq!(exit_code(&duplicate), 1);
    assert!(text(&duplicate.stderr).contains("já existe uma org com o slug `org-msg`"));

    let v4 = scenario.poph(&["--offline", "org", "show", UUID_V4]);
    assert_eq!(exit_code(&v4), 1);
    assert!(
        text(&v4.stderr).contains("esperado UUID v7"),
        "{}",
        text(&v4.stderr)
    );

    let without_org = scenario.poph(&["--offline", "token", "list"]);
    assert_eq!(exit_code(&without_org), 1);
    assert!(text(&without_org.stderr).contains("informe --org"));

    let bad_category = scenario.poph(&["--offline", "--org", "org-msg", "category", "show", "abc"]);
    assert_eq!(exit_code(&bad_category), 1);
    assert!(text(&bad_category.stderr).contains("id de categoria de produto inválido"));

    let missing_token = scenario.poph(&[
        "--offline",
        "--org",
        "org-msg",
        "token",
        "show",
        "0190a000-0000-7000-8000-000000000000",
    ]);
    assert_eq!(exit_code(&missing_token), 1);
    assert!(text(&missing_token.stderr).contains("não existe token"));

    for name in ["Planos", "planos"] {
        let _ = scenario.poph(&[
            "--offline",
            "--org",
            "org-msg",
            "category",
            "create",
            "--name",
            name,
        ]);
    }
    let repeated = scenario.poph(&[
        "--offline",
        "--org",
        "org-msg",
        "category",
        "create",
        "--name",
        "PLANOS",
    ]);
    assert_eq!(exit_code(&repeated), 1);
    assert!(
        text(&repeated.stderr).contains("já existe um registro"),
        "{}",
        text(&repeated.stderr)
    );

    let nothing = scenario.poph(&["--offline", "org", "update", "org-msg", "--if-match", "1"]);
    assert_eq!(exit_code(&nothing), 1);
    assert!(text(&nothing.stderr).contains("nada a alterar"));
}

#[test]
fn change_requires_the_right_version() {
    let scenario = Scenario::new("versao");
    scenario.create_org("org-versao");
    let created = json(&scenario.poph(&[
        "--offline",
        "--org",
        "org-versao",
        "category",
        "create",
        "--name",
        "Planos",
        "--json",
    ]));
    let id = field(&created, "category.id");

    let ok = scenario.poph(&[
        "--offline",
        "--org",
        "org-versao",
        "category",
        "update",
        &id,
        "--if-match",
        "1",
        "--name",
        "Planos 2",
    ]);
    assert_eq!(exit_code(&ok), 0, "{}", text(&ok.stderr));

    let old = scenario.poph(&[
        "--offline",
        "--org",
        "org-versao",
        "category",
        "update",
        &id,
        "--if-match",
        "1",
        "--name",
        "Outro",
    ]);
    assert_eq!(exit_code(&old), 1);
    assert!(text(&old.stderr).contains("a versão não confere"));

    let org_ok = scenario.poph(&[
        "--offline",
        "org",
        "update",
        "org-versao",
        "--if-match",
        "1",
        "--name",
        "Nome novo",
    ]);
    assert_eq!(exit_code(&org_ok), 0, "{}", text(&org_ok.stderr));
    assert_eq!(
        json(&scenario.poph(&["--offline", "org", "show", "org-versao", "--json"]))["org"]["version"],
        2
    );
}

#[test]
fn category_only_goes_with_force_and_department_goes_directly() {
    let scenario = Scenario::new("exclusoes");
    scenario.create_org("org-exclui");
    let category = json(&scenario.poph(&[
        "--offline",
        "--org",
        "org-exclui",
        "category",
        "create",
        "--name",
        "Planos",
        "--json",
    ]));
    let category_id = field(&category, "category.id");

    let without_force = scenario.poph(&[
        "--offline",
        "--org",
        "org-exclui",
        "category",
        "delete",
        &category_id,
    ]);
    assert_eq!(exit_code(&without_force), 1);
    assert!(text(&without_force.stderr).contains("--force"));
    let list = json(&scenario.poph(&[
        "--offline",
        "--org",
        "org-exclui",
        "category",
        "list",
        "--json",
    ]));
    assert_eq!(list["total"], 1, "a recusa não pode apagar a categoria");

    let with_force = scenario.poph(&[
        "--offline",
        "--org",
        "org-exclui",
        "category",
        "delete",
        &category_id,
        "--force",
    ]);
    assert_eq!(exit_code(&with_force), 0, "{}", text(&with_force.stderr));

    let department = json(&scenario.poph(&[
        "--offline",
        "--org",
        "org-exclui",
        "department",
        "create",
        "--name",
        "Vendas",
        "--json",
    ]));
    let department_id = field(&department, "department.id");
    let leaves = scenario.poph(&[
        "--offline",
        "--org",
        "org-exclui",
        "department",
        "delete",
        &department_id,
    ]);
    assert_eq!(exit_code(&leaves), 0, "{}", text(&leaves.stderr));
    let gone = scenario.poph(&[
        "--offline",
        "--org",
        "org-exclui",
        "department",
        "show",
        &department_id,
    ]);
    assert_eq!(exit_code(&gone), 1);
    assert!(text(&gone.stderr).contains("não existe departamento"));
}

#[test]
fn data_from_one_org_does_not_show_in_another() {
    let scenario = Scenario::new("isolamento");
    scenario.create_org("org-a");
    scenario.create_org("org-b");

    let category = json(&scenario.poph(&[
        "--offline",
        "--org",
        "org-a",
        "category",
        "create",
        "--name",
        "Planos",
        "--json",
    ]));
    let category_id = field(&category, "category.id");
    let token = json(&scenario.poph(&[
        "--offline",
        "--org",
        "org-a",
        "token",
        "create",
        "--label",
        "a",
        "--json",
    ]));
    let token_id = field(&token, "token.id");

    let list_b =
        json(&scenario.poph(&["--offline", "--org", "org-b", "category", "list", "--json"]));
    assert_eq!(list_b["total"], 0, "org B não pode ver categoria da org A");

    let show_b = scenario.poph(&[
        "--offline",
        "--org",
        "org-b",
        "category",
        "show",
        &category_id,
    ]);
    assert_eq!(exit_code(&show_b), 1);

    let token_b = scenario.poph(&["--offline", "--org", "org-b", "token", "show", &token_id]);
    assert_eq!(
        exit_code(&token_b),
        1,
        "token da org A não aparece na org B"
    );
    let revoke_b = scenario.poph(&["--offline", "--org", "org-b", "token", "delete", &token_id]);
    assert_eq!(exit_code(&revoke_b), 1, "org B não revoga token da org A");
}
