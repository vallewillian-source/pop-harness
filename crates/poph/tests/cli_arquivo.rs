use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use poph_store::{NewAgent, NewOrg, Store};
use serde_json::Value;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89";

struct Scenario {
    dir: PathBuf,
    slug: String,
    agent: String,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("poph-cli-arquivo-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dados")).expect("cria a pasta de dados");
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
                    NewAgent {
                        name: "Bia".to_owned(),
                        slug: "bia".to_owned(),
                        ..NewAgent::default()
                    },
                )
                .expect("cria o agente");
        }
        Self {
            dir,
            slug,
            agent: "bia".to_owned(),
        }
    }

    fn poph(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_poph"));
        command
            .args(args)
            .current_dir(&self.dir)
            .env_clear()
            .env("POPH_DADOS_DIRETORIO", self.dir.join("dados"));
        command.output().expect("executa o poph")
    }

    fn ok(&self, args: &[&str]) -> String {
        let mut all: Vec<&str> = vec!["--offline", "--org", &self.slug];
        all.extend_from_slice(args);
        let output = self.poph(&all);
        assert!(
            output.status.success(),
            "`poph {}` falhou: {}",
            all.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn ok_json(&self, args: &[&str]) -> Value {
        let text = self.ok(args);
        serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("saída não é JSON ({error}): {text}"))
    }

    fn failed(&self, args: &[&str]) -> (i32, String) {
        let mut all: Vec<&str> = vec!["--offline", "--org", &self.slug];
        all.extend_from_slice(args);
        let output = self.poph(&all);
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.join(name);
        fs::write(&path, bytes).expect("grava o arquivo de teste");
        path
    }

    fn blobs(&self) -> Vec<String> {
        let dir = self.dir.join("dados").join("blobs");
        let Ok(orgs) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut names = Vec::new();
        for org in orgs.flatten() {
            if let Ok(files) = fs::read_dir(org.path()) {
                for file in files.flatten() {
                    names.push(file.file_name().to_string_lossy().into_owned());
                }
            }
        }
        names
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn upload_json(scenario: &Scenario, path: &Path, extra: &[&str]) -> Value {
    let path = path.to_string_lossy().into_owned();
    let mut args: Vec<&str> = vec![
        "file",
        "upload",
        &path,
        "--agent",
        &scenario.agent,
        "--json",
    ];
    args.extend_from_slice(extra);
    scenario.ok_json(&args)
}

#[test]
fn uploads_lists_shows_and_deletes_file() {
    let scenario = Scenario::new("crud");
    let path = scenario.write("logo.png", PNG);

    let created = upload_json(
        &scenario,
        &path,
        &[
            "--name",
            "Logo da empresa",
            "--description",
            "logotipo para o cliente",
        ],
    );
    let file = &created["arquivo"];
    let id = file["id"].as_str().expect("id no JSON").to_owned();
    assert_eq!(file["nome"], "Logo da empresa");
    assert_eq!(file["descricao"], "logotipo para o cliente");
    assert_eq!(file["mime"], "image/png");
    assert!(
        file["agent_id"].as_str().is_some(),
        "o dono do arquivo vem no JSON"
    );
    assert!(!file["blob_path"].as_str().unwrap().starts_with('/'));

    let list = scenario.ok_json(&["file", "list", "--agent", &scenario.agent, "--json"]);
    assert_eq!(list["total"], 1);
    assert_eq!(list["arquivos"][0]["id"], id.as_str());

    let shown = scenario.ok_json(&["file", "show", &id, "--agent", &scenario.agent, "--json"]);
    assert_eq!(shown["arquivo"]["id"], id.as_str());

    let deleted = scenario.ok_json(&["file", "delete", &id, "--agent", &scenario.agent, "--json"]);
    assert_eq!(deleted["excluido"]["id"], id.as_str());
    let list = scenario.ok_json(&["file", "list", "--agent", &scenario.agent, "--json"]);
    assert_eq!(list["total"], 0);

    assert_eq!(scenario.blobs().len(), 1);
}

#[test]
fn same_content_twice_yields_a_single_blob() {
    let scenario = Scenario::new("dedupe");
    let path = scenario.write("logo.png", PNG);
    let first = upload_json(&scenario, &path, &[]);
    let second = upload_json(&scenario, &path, &[]);
    assert_eq!(
        first["arquivo"]["sha256"], second["arquivo"]["sha256"],
        "mesmo conteúdo, mesmo hash"
    );
    assert_ne!(first["arquivo"]["id"], second["arquivo"]["id"]);
    assert_eq!(scenario.blobs().len(), 1, "um blob só por hash");
}

#[test]
fn file_above_16_mb_is_refused() {
    let scenario = Scenario::new("grande");
    let path = scenario.write("grande.pdf", &vec![0u8; 17 * 1024 * 1024]);
    let (exit_code, error) = scenario.failed(&[
        "file",
        "upload",
        &path.to_string_lossy(),
        "--agent",
        &scenario.agent,
    ]);
    assert_eq!(exit_code, 1, "{error}");
    assert!(
        error.contains("16"),
        "mensagem precisa dizer o limite: {error}"
    );
    let list = scenario.ok_json(&["file", "list", "--agent", &scenario.agent, "--json"]);
    assert_eq!(list["total"], 0);
    assert!(scenario.blobs().is_empty());
}

#[test]
fn misleading_extension_is_refused() {
    let scenario = Scenario::new("enganosa");
    let path = scenario.write("contrato.pdf", PNG);
    let (exit_code, error) = scenario.failed(&[
        "file",
        "upload",
        &path.to_string_lossy(),
        "--agent",
        &scenario.agent,
    ]);
    assert_eq!(exit_code, 1, "{error}");
    assert!(
        error.to_lowercase().contains("pdf") && error.to_lowercase().contains("png"),
        "{error}"
    );
    let list = scenario.ok_json(&["file", "list", "--agent", &scenario.agent, "--json"]);
    assert_eq!(list["total"], 0);
}

#[test]
fn missing_file_and_wrong_agent_fail_with_code_one() {
    let scenario = Scenario::new("erros");
    let (exit_code, error) = scenario.failed(&[
        "file",
        "upload",
        "/caminho/que/nao/existe.png",
        "--agent",
        &scenario.agent,
    ]);
    assert_eq!(exit_code, 1);
    assert!(error.contains("não consegui abrir"), "{error}");

    let path = scenario.write("logo.png", PNG);
    let (exit_code, error) = scenario.failed(&[
        "file",
        "upload",
        &path.to_string_lossy(),
        "--agent",
        "agente-que-nao-existe",
    ]);
    assert_eq!(exit_code, 1, "{error}");

    let (exit_code, error) =
        scenario.failed(&["file", "show", "nao-e-uuid", "--agent", &scenario.agent]);
    assert_eq!(exit_code, 1, "{error}");
}

#[test]
fn without_offline_the_command_refuses() {
    let scenario = Scenario::new("offline");
    let output = scenario.poph(&[
        "--org",
        &scenario.slug,
        "file",
        "list",
        "--agent",
        &scenario.agent,
    ]);
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("--offline"), "{error}");
}

#[test]
fn library_is_per_agent() {
    let scenario = Scenario::new("isolamento");
    {
        let store = Store::open_at(
            &scenario.dir.join("dados").join("harness.db"),
            &scenario.dir.join("dados").join("rag.db"),
        )
        .expect("abre o banco");
        let org = store.orgs().by_slug(&scenario.slug).expect("org criada").id;
        store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Caio".to_owned(),
                    slug: "caio".to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o segundo agente");
    }
    let path = scenario.write("logo.png", PNG);
    let created = upload_json(&scenario, &path, &[]);
    let id = created["arquivo"]["id"].as_str().expect("id").to_owned();

    let list = scenario.ok_json(&["file", "list", "--agent", "caio", "--json"]);
    assert_eq!(list["total"], 0);
    let (exit_code, _error) = scenario.failed(&["file", "show", &id, "--agent", "caio"]);
    assert_eq!(exit_code, 1);
    let (exit_code, _error) = scenario.failed(&["file", "delete", &id, "--agent", "caio"]);
    assert_eq!(exit_code, 1);
    let list = scenario.ok_json(&["file", "list", "--agent", &scenario.agent, "--json"]);
    assert_eq!(list["total"], 1);
}
