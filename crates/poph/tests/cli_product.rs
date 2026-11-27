use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use poph_store::{NewCategory, NewOrg, Store};
use serde_json::Value;

const CIPHER_SENTINEL: &str = "SENTINELA-CIFRA-hr18";
const LLM_SENTINEL: &str = "SENTINELA-LLM-hr18";

fn repo_models_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/modelos"))
}

struct Scenario {
    dir: PathBuf,
    slug: String,
}

impl Scenario {
    fn new(label: &str) -> Option<Self> {
        let models = repo_models_dir();
        if !models.join("model_int8.onnx").is_file() || !models.join("tokenizer.json").is_file() {
            eprintln!("PULADO ({label}): modelo não instalado; rode `poph model install`");
            return None;
        }
        let dir =
            std::env::temp_dir().join(format!("poph-cli-produto-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dados")).expect("cria a pasta de dados");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&models, dir.join("dados").join("modelos"))
            .expect("liga a pasta de modelos");
        let slug = format!("org-{label}");
        {
            let store = Store::open_at(
                &dir.join("dados").join("harness.db"),
                &dir.join("dados").join("rag.db"),
            )
            .expect("abre o banco");
            let org = store
                .orgs()
                .create(NewOrg {
                    name: format!("Org {label}"),
                    slug: slug.clone(),
                    config_json: "{}".to_owned(),
                })
                .expect("cria a org")
                .id;
            store
                .agents()
                .create(
                    org,
                    poph_store::NewAgent {
                        name: "Bia".to_owned(),
                        slug: "bia".to_owned(),
                        ..poph_store::NewAgent::default()
                    },
                )
                .expect("cria o agente");
            store
                .categories()
                .create(
                    org,
                    NewCategory {
                        name: "Pragas urbanas".to_owned(),
                        description: String::new(),
                    },
                )
                .expect("cria a categoria");
        }
        Some(Self { dir, slug })
    }

    fn poph(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_poph"));
        command
            .arg("--offline")
            .args(args)
            .current_dir(&self.dir)
            .env_clear()
            .env("POPH_DADOS_DIRETORIO", self.dir.join("dados"))
            .env("POPH_CHAVE_CIFRA", CIPHER_SENTINEL)
            .env("ANTIGRAVITY_API_KEY", LLM_SENTINEL);
        command.output().expect("executa o poph")
    }

    fn ok(&self, args: &[&str]) -> Value {
        let saida = self.poph(args);
        assert!(
            saida.status.success(),
            "`poph {}` falhou: {}",
            args.join(" "),
            String::from_utf8_lossy(&saida.stderr)
        );
        let texto = String::from_utf8_lossy(&saida.stdout);
        assert!(!texto.contains(CIPHER_SENTINEL) && !texto.contains(LLM_SENTINEL));
        serde_json::from_str(&texto).unwrap_or_else(|error| {
            panic!("saída não é JSON ({error}): {texto}");
        })
    }

    fn fail(&self, args: &[&str]) -> String {
        let saida = self.poph(args);
        assert!(
            !saida.status.success(),
            "`poph {}` devia falhar",
            args.join(" ")
        );
        String::from_utf8_lossy(&saida.stderr).into_owned()
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn cli_product_creates_shows_updates_and_deletes() {
    let Some(scenario) = Scenario::new("ciclo") else {
        return;
    };
    let org = scenario.slug.clone();

    let created = scenario.ok(&[
        "--org",
        &org,
        "product",
        "create",
        "--name",
        "Dedetização de baratas",
        "--category",
        "Pragas urbanas",
        "--alias",
        "baratas",
        "--alias",
        "barata",
        "--tag",
        "cozinha",
        "--price",
        "32000",
        "--price-text",
        "R$ 320,00",
        "--unit",
        "por visita",
        "--conditions",
        "Pagamento em até 3x",
        "--json",
    ]);
    let product = &created["product"];
    assert_eq!(product["name"], "Dedetização de baratas");
    assert_eq!(product["slug"], "dedetizacao-de-baratas");
    assert_eq!(product["price_cents"], 32_000);
    assert_eq!(product["aliases"][0], "baratas");
    assert_eq!(product["status"], "active");
    let agent = scenario.ok(&["--org", &org, "agent", "show", "bia", "--json"]);
    assert!(agent["agent"]["slug"] == "bia");
    let id = product["id"].as_str().expect("id").to_owned();

    let listing = scenario.ok(&["--org", &org, "product", "list", "--json"]);
    assert_eq!(listing["total"], 1);
    assert_eq!(listing["products"][0]["id"], id);

    let shown = scenario.ok(&["--org", &org, "product", "show", &id, "--json"]);
    assert_eq!(shown["product"]["version"], 1);

    let updated = scenario.ok(&[
        "--org",
        &org,
        "product",
        "update",
        &id,
        "--if-match",
        "1",
        "--price",
        "27990",
        "--price-text",
        "R$ 279,90",
        "--json",
    ]);
    assert_eq!(updated["product"]["price_cents"], 27_990);
    assert_eq!(updated["product"]["version"], 2);

    let error = scenario.fail(&[
        "--org",
        &org,
        "product",
        "update",
        &id,
        "--if-match",
        "1",
        "--price",
        "1000",
    ]);
    assert!(
        error.to_lowercase().contains("versão") || error.to_lowercase().contains("versao"),
        "erro de conflito de versão esperado: {error}"
    );

    let without_force = scenario.fail(&["--org", &org, "product", "delete", &id]);
    assert!(without_force.contains("--force"), "{without_force}");

    let deleted = scenario.ok(&["--org", &org, "product", "delete", &id, "--force", "--json"]);
    assert_eq!(deleted["deleted"], id);
    let listing = scenario.ok(&["--org", &org, "product", "list", "--json"]);
    assert_eq!(listing["total"], 0);
}

#[test]
fn cli_product_reindexes_and_respects_the_price_limit() {
    let Some(scenario) = Scenario::new("reindex") else {
        return;
    };
    let org = scenario.slug.clone();
    let created = scenario.ok(&[
        "--org",
        &org,
        "product",
        "create",
        "--name",
        "Desratização",
        "--category",
        "Pragas urbanas",
        "--alias",
        "ratos",
        "--price",
        "25000",
        "--json",
    ]);
    let id = created["product"]["id"].as_str().expect("id").to_owned();

    let reindexed = scenario.ok(&["--org", &org, "product", "reindex", "--json"]);
    let agents = reindexed["agents"].as_array().expect("agentes");
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0]["unchanged"], 1, "nada mudou desde a escrita");

    let error = scenario.fail(&[
        "--org",
        &org,
        "product",
        "update",
        &id,
        "--if-match",
        "1",
        "--price=-1",
    ]);
    assert!(error.contains("negativo"), "{error}");

    let error = scenario.fail(&[
        "--org",
        &org,
        "product",
        "create",
        "--name",
        "Produto sem categoria",
        "--category",
        "Não existe",
        "--price",
        "100",
    ]);
    assert!(error.contains("não existe categoria"), "{error}");
}
