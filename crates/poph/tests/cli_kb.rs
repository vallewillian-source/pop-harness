use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use poph_store::{NewOrg, Store};
use serde_json::Value;

const CIPHER_SENTINEL: &str = "SENTINELA-CIFRA-hr17";
const LLM_SENTINEL: &str = "SENTINELA-LLM-hr17";

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
        let dir = std::env::temp_dir().join(format!("poph-cli-kb-{label}-{}", std::process::id()));
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
            store
                .orgs()
                .create(NewOrg {
                    name: format!("Org {label}"),
                    slug: slug.clone(),
                    config_json: "{}".to_owned(),
                })
                .expect("cria a org");
            store
                .agents()
                .create(
                    store.orgs().by_slug(&slug).expect("org").id,
                    poph_store::NewAgent {
                        name: "Bia".to_owned(),
                        slug: "bia".to_owned(),
                        ..poph_store::NewAgent::default()
                    },
                )
                .expect("cria o agente");
        }
        Some(Self { dir, slug })
    }

    fn poph(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_poph"));
        command
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
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn cli_kb_creates_indexes_searches_edits_and_deletes() {
    let Some(scenario) = Scenario::new("ciclo") else {
        return;
    };
    let org = scenario.slug.clone();

    let group = scenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "group",
        "create",
        "--name",
        "Políticas",
        "--json",
    ]);
    let group_id = group["group"]["id"]
        .as_str()
        .expect("id do grupo")
        .to_owned();

    let item = scenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "item",
        "create",
        "--group",
        &group_id,
        "--title",
        "Prazo de agendamento",
        "--content",
        "O prazo para agendar a dedetização é de 48 horas úteis.",
        "--json",
    ]);
    let item_id = item["item"]["id"].as_str().expect("id do item").to_owned();

    scenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "link",
        &group_id,
        "--agent",
        "bia",
        "--json",
    ]);

    let listing = scenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "item",
        "list",
        "--group",
        &group_id,
        "--json",
    ]);
    assert_eq!(listing["items"][0]["index"], "indexado", "{listing}");

    let found = scenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "search",
        "quanto tempo leva para agendar",
        "--agent",
        "bia",
        "--json",
    ]);
    assert_eq!(found["total"], 1, "{found}");
    assert_eq!(found["chunks"][0]["titulo"], "Prazo de agendamento");
    assert_eq!(found["chunks"][0]["grupo"], "Políticas");

    let versao = item["item"]["version"].as_u64().expect("versão");
    let edited = scenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "item",
        "update",
        &item_id,
        "--if-match",
        &versao.to_string(),
        "--content",
        "O agendamento agora é imediato pelo aplicativo.",
        "--json",
    ]);
    assert_eq!(edited["item"]["index"], "indexado", "{edited}");
    let searched = scenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "search",
        "agendamento pelo aplicativo",
        "--agent",
        "bia",
        "--json",
    ]);
    assert_eq!(searched["chunks"][0]["titulo"], "Prazo de agendamento");

    scenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "item",
        "delete",
        &item_id,
        "--json",
    ]);
    let afterwards = scenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "search",
        "agendamento aplicativo",
        "--agent",
        "bia",
        "--json",
    ]);
    assert_eq!(afterwards["total"], 0, "{afterwards}");
}

#[test]
fn cli_kb_refuses_item_above_one_thousand_characters() {
    let Some(scenario) = Scenario::new("limite") else {
        return;
    };
    let org = scenario.slug.clone();
    let group = scenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "group",
        "create",
        "--name",
        "G",
        "--json",
    ]);
    let group_id = group["group"]["id"].as_str().expect("id").to_owned();
    let large = "a".repeat(1001);
    let saida = scenario.poph(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "item",
        "create",
        "--group",
        &group_id,
        "--title",
        "Grande",
        "--content",
        &large,
    ]);
    assert!(!saida.status.success(), "1001 caracteres precisam falhar");
    let error = String::from_utf8_lossy(&saida.stderr);
    assert!(
        error.contains("1000") && error.contains("1001"),
        "o erro precisa dizer o limite: {error}"
    );
}

#[test]
fn repo_models_dir_exists() {
    let models: &Path = &repo_models_dir();
    if !models.is_dir() {
        eprintln!("modelo não instalado em {}", models.display());
    }
}

#[test]
fn cli_rag_reindex_is_idempotent() {
    let Some(scenario) = Scenario::new("reindex") else {
        return;
    };
    let org = scenario.slug.clone();
    let group = scenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "group",
        "create",
        "--name",
        "Prazo",
        "--json",
    ]);
    let group_id = group["group"]["id"].as_str().expect("grupo").to_owned();
    scenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "item",
        "create",
        "--group",
        &group_id,
        "--title",
        "Prazo de agendamento",
        "--content",
        "O prazo para agendar a dedetização é de 48 horas úteis.",
        "--json",
    ]);
    scenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "link",
        &group_id,
        "--agent",
        "bia",
        "--json",
    ]);

    let idle = scenario.ok(&["--offline", "--org", &org, "rag", "reindex", "--json"]);
    assert_eq!(idle["agents"][0]["indexed"]["embeddings"], 0, "{idle}");
    assert_eq!(idle["agents"][0]["indexed"]["items_unchanged"], 1, "{idle}");

    for suffix in ["rag.db", "rag.db-wal", "rag.db-shm"] {
        let _ = fs::remove_file(scenario.dir.join("dados").join(suffix));
    }
    let rebuilt = scenario.ok(&["--offline", "--org", &org, "rag", "reindex", "--json"]);
    assert_eq!(rebuilt["agents"][0]["indexed"]["items_new"], 1, "{rebuilt}");
    assert!(
        rebuilt["agents"][0]["indexed"]["embeddings"]
            .as_u64()
            .expect("embeddings")
            >= 1,
        "{rebuilt}"
    );

    let repeated = scenario.ok(&["--offline", "--org", &org, "rag", "reindex", "--json"]);
    assert_eq!(
        repeated["agents"][0]["indexed"]["embeddings"], 0,
        "{repeated}"
    );
    let found = scenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "search",
        "quanto tempo para agendar",
        "--agent",
        "bia",
        "--json",
    ]);
    assert_eq!(
        found["chunks"][0]["titulo"], "Prazo de agendamento",
        "{found}"
    );
}
