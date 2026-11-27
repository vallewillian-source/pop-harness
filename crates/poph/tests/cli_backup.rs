use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use poph_files::FileConfig;
use poph_store::backup::OrgSnapshot;
use poph_store::{
    Direction, NewChannelAttachment, NewConversation, NewMessage, NewOutboxItem, Role, Store,
    TurnFinalization,
};
use poph_types::{AgentId, OrgId};
use serde_json::Value;

const SECRET_SENTINEL: &str = "SENTINELA-SEGREDO-hr53";
const CIPHER_KEY: &str = "chave-de-teste-hr53";
const LLM_SENTINEL: &str = "SENTINELA-LLM-hr53";

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89";

fn models_repo_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/modelos"))
}

fn model_installed() -> bool {
    let models = models_repo_dir();
    models.join("model_int8.onnx").is_file() && models.join("tokenizer.json").is_file()
}

struct Scenario {
    dir: PathBuf,
    slug: String,
}

impl Scenario {
    fn new(label: &str) -> Self {
        Self::with_model(label, false)
    }

    fn with_model(label: &str, with_model: bool) -> Self {
        let dir =
            std::env::temp_dir().join(format!("poph-cli-backup-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dados")).expect("cria a pasta de dados");
        if with_model && model_installed() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(models_repo_dir(), dir.join("dados").join("modelos"))
                .expect("liga a pasta de modelos");
        }
        Self {
            dir,
            slug: format!("org-{label}"),
        }
    }

    fn data_dir(&self) -> PathBuf {
        self.dir.join("dados")
    }

    fn poph(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_poph"))
            .args(args)
            .current_dir(&self.dir)
            .env_clear()
            .env("POPH_DADOS_DIRETORIO", self.data_dir())
            .env("POPH_CHAVE_CIFRA", CIPHER_KEY)
            .env("ANTIGRAVITY_API_KEY", LLM_SENTINEL)
            .output()
            .expect("executa o poph")
    }

    fn ok_json(&self, args: &[&str]) -> Value {
        let saida = self.poph(args);
        assert!(
            saida.status.success(),
            "`poph {}` falhou: {}",
            args.join(" "),
            String::from_utf8_lossy(&saida.stderr)
        );
        let texto = String::from_utf8_lossy(&saida.stdout);
        serde_json::from_str(&texto).unwrap_or_else(|error| {
            panic!("saída não é JSON ({error}): {texto}");
        })
    }

    fn failure(&self, args: &[&str]) -> String {
        let saida = self.poph(args);
        assert!(
            !saida.status.success(),
            "`poph {}` devia falhar",
            args.join(" ")
        );
        String::from_utf8_lossy(&saida.stderr).into_owned()
    }

    fn open_store(&self) -> Store {
        Store::open_at(
            &self.data_dir().join("harness.db"),
            &self.data_dir().join("rag.db"),
        )
        .expect("abre o banco")
    }

    fn org_by_slug(&self, slug: &str) -> OrgId {
        self.open_store()
            .orgs()
            .by_slug(slug)
            .expect("org do cenário")
            .id
    }
}

fn seed_origin(scenario: &Scenario) {
    scenario.ok_json(&[
        "--offline",
        "org",
        "create",
        "--name",
        "Org Origem",
        "--slug",
        &scenario.slug,
        "--json",
    ]);
    scenario.ok_json(&[
        "--offline",
        "org",
        "update",
        &scenario.slug,
        "--if-match",
        "1",
        "--config-json",
        "{\"retencao\":{\"dias\":7}}",
        "--json",
    ]);
    scenario.ok_json(&[
        "--offline",
        "--org",
        &scenario.slug,
        "agent",
        "create",
        "--name",
        "Bia",
        "--slug",
        "bia",
        "--json",
    ]);
    scenario.ok_json(&[
        "--offline",
        "--org",
        &scenario.slug,
        "department",
        "create",
        "--name",
        "Atendimento",
        "--description",
        "primeiro contato",
        "--json",
    ]);
    scenario.ok_json(&[
        "--offline",
        "--org",
        &scenario.slug,
        "category",
        "create",
        "--name",
        "Pragas",
        "--json",
    ]);
    scenario.ok_json(&[
        "--offline",
        "--org",
        &scenario.slug,
        "product",
        "create",
        "--name",
        "Dedetização de baratas",
        "--slug",
        "dedetizacao-de-baratas",
        "--category",
        "Pragas",
        "--price",
        "45000",
        "--alias",
        "baratas",
        "--json",
    ]);
    let group = scenario.ok_json(&[
        "--offline",
        "--org",
        &scenario.slug,
        "kb",
        "group",
        "create",
        "--name",
        "Atendimento",
        "--json",
    ]);
    let group_id = group["group"]["id"].as_str().expect("id do grupo");
    scenario.ok_json(&[
        "--offline",
        "--org",
        &scenario.slug,
        "kb",
        "item",
        "create",
        "--group",
        group_id,
        "--title",
        "Prazo de entrega",
        "--content",
        "O prazo é de 5 dias úteis.",
        "--json",
    ]);
    scenario.ok_json(&[
        "--offline",
        "--org",
        &scenario.slug,
        "kb",
        "link",
        group_id,
        "--agent",
        "bia",
        "--json",
    ]);

    let file = scenario.dir.join("logo.png");
    fs::write(&file, PNG).expect("grava o PNG");
    scenario.ok_json(&[
        "--offline",
        "--org",
        &scenario.slug,
        "file",
        "upload",
        file.to_str().expect("caminho"),
        "--agent",
        "bia",
        "--name",
        "Logo da empresa",
        "--description",
        "logo",
        "--json",
    ]);
    let prompt = scenario.dir.join("prompt.md");
    fs::write(
        &prompt,
        "# Identidade\n\nVocê é a Bia, da Dedetizadora Exemplo.\n",
    )
    .expect("grava o prompt");
    scenario.ok_json(&[
        "--offline",
        "--org",
        &scenario.slug,
        "instructions",
        "create",
        "bia",
        "--markdown-file",
        prompt.to_str().expect("caminho"),
        "--json",
    ]);

    let store = scenario.open_store();
    let org = store.orgs().by_slug(&scenario.slug).unwrap().id;
    let agent = store.agents().by_slug(org, "bia").unwrap().id;
    let key = poph_server::CipherKey::derive(CIPHER_KEY).expect("deriva a chave");
    let mut secrets = std::collections::BTreeMap::new();
    secrets.insert("token".to_owned(), SECRET_SENTINEL.to_owned());
    let blob = key.cifrar(&secrets).expect("cifra o segredo");
    store
        .create_integration_with_config(
            org,
            agent,
            "whatsapp".to_owned(),
            "active".to_owned(),
            serde_json::json!({"instancia": "bia"}),
            Some(blob),
        )
        .expect("cria a integração");

    let conversation = store
        .conversations()
        .create(
            org,
            NewConversation {
                agent_id: agent,
                integration_id: None,
                canal: "test".to_owned(),
                peer_externo: None,
                target_department: None,
            },
        )
        .expect("cria a conversa");
    let attachment_content = [PNG, b"-anexo-do-cliente"].concat();
    let attachment = poph_files::blob::write(
        &FileConfig::default(scenario.data_dir().join("blobs")).blob_folder,
        org,
        &attachment_content,
    )
    .expect("grava o blob do anexo");
    store
        .messages()
        .write_with_attachments(
            org,
            NewMessage {
                conversation_id: conversation.id,
                direction: Direction::Inbound,
                role: Role::User,
                texto: "Qual o prazo? Segue o comprovante.".to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
            &[NewChannelAttachment {
                name: "comprovante.png".to_owned(),
                mime: "image/png".to_owned(),
                size_bytes: attachment_content.len() as u64,
                sha256: attachment.sha256.clone(),
                blob_path: format!("{org}/{}", attachment.sha256),
                extracted_text: None,
                extraction_status: "skipped",
            }],
        )
        .expect("grava a mensagem com anexo");
    let turn = store
        .turns()
        .start(org, conversation.id, Some(agent))
        .expect("abre o turno");
    store
        .turns()
        .finalise(
            org,
            turn.id,
            TurnFinalization::completed("{\"total_ms\":12}"),
        )
        .expect("fecha o turno");
    store
        .messages()
        .write(
            org,
            NewMessage {
                conversation_id: conversation.id,
                direction: Direction::Outbound,
                role: Role::Assistant,
                texto: "O prazo é de 5 dias úteis.".to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: Some(turn.id),
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
        )
        .expect("grava a resposta");
    store
        .outbox()
        .enqueue(
            org,
            NewOutboxItem {
                conversation_id: conversation.id,
                turn_id: turn.id,
                seq: 1,
                tipo: "text".to_owned(),
                content_json: "{\"text\":\"O prazo é de 5 dias úteis.\"}".to_owned(),
            },
        )
        .expect("enfileira a saída");
}

fn compare_snapshots(origin: &OrgSnapshot, destination: &OrgSnapshot) {
    assert_eq!(origin.org, destination.org, "linha da org");
    assert_eq!(
        origin.tables.keys().collect::<Vec<_>>(),
        destination.tables.keys().collect::<Vec<_>>(),
        "tabelas do retrato"
    );
    for (table, rows) in &origin.tables {
        let expected: Vec<Value> = rows.iter().map(|row| normalize(table, row)).collect();
        let obtained: Vec<Value> = destination.tables[table]
            .iter()
            .map(|row| normalize(table, row))
            .collect();
        assert_eq!(expected, obtained, "conteúdo da tabela `{table}`");
    }
}

fn normalize(table: &str, row: &Value) -> Value {
    if table != "integrations" {
        return row.clone();
    }
    let mut row = row.clone();
    if let Some(object) = row.as_object_mut() {
        for field in ["tem_segredo", "segredos_campos"] {
            object.remove(field);
        }
    }
    row
}

fn read_snapshot(scenario: &Scenario, org: OrgId) -> OrgSnapshot {
    scenario
        .open_store()
        .org_snapshot(org, true)
        .expect("retrato")
}

fn compare_blobs(
    origin: &Scenario,
    destination: &Scenario,
    origin_org: OrgId,
    destination_org: OrgId,
) {
    let snapshot = read_snapshot(origin, origin_org);
    let mut hashes = Vec::new();
    for table in ["file_library", "attachments"] {
        for row in snapshot.tables.get(table).into_iter().flatten() {
            if let Some(sha) = row.get("sha256").and_then(Value::as_str) {
                hashes.push(sha.to_owned());
            }
        }
    }
    assert!(!hashes.is_empty(), "o cenário devia ter blobs");
    for sha in hashes {
        let original = fs::read(
            origin
                .data_dir()
                .join("blobs")
                .join(origin_org.to_string())
                .join(&sha),
        )
        .expect("lê o blob original");
        let copy = fs::read(
            destination
                .data_dir()
                .join("blobs")
                .join(destination_org.to_string())
                .join(&sha),
        )
        .expect("lê o blob importado");
        assert_eq!(original, copy, "conteúdo do blob {sha}");
        assert_eq!(
            poph_files::blob::sha256_hex(&copy),
            sha,
            "sha256 do blob importado"
        );
    }
}

fn agent_by_slug(store: &Store, org: OrgId, slug: &str) -> AgentId {
    store.agents().by_slug(org, slug).expect("agente").id
}

#[test]
fn export_and_import_rebuild_org_in_a_single_file() {
    let origin = Scenario::new("origem");
    seed_origin(&origin);
    let origin_org = origin.org_by_slug(&origin.slug);
    let file = origin.dir.join("backup.json");

    let report = origin.ok_json(&[
        "--offline",
        "backup",
        "export",
        &origin.slug,
        "--out",
        file.to_str().expect("caminho"),
        "--com-conversas",
        "--json",
    ]);
    assert_eq!(report["versao_relatorio"], 1);
    assert_eq!(report["com_conversas"], true);
    assert!(report["blobs"].as_u64().unwrap() >= 2, "biblioteca + anexo");

    let texto = fs::read_to_string(&file).expect("lê o backup");
    assert!(
        !texto.contains(SECRET_SENTINEL),
        "segredo em claro no arquivo"
    );
    assert!(!texto.contains(LLM_SENTINEL), "segredo do LLM no arquivo");
    assert!(!texto.contains("blob_path"), "caminho de blob no arquivo");
    assert!(!texto.contains("secrets_enc"), "segredo cifrado no arquivo");
    let document: Value = serde_json::from_str(&texto).expect("JSON do backup");
    assert_eq!(document["formato"], "poph-backup-org");
    assert_eq!(document["versao_formato"], 2);
    assert_eq!(document["org_origem"]["slug"], origin.slug);
    let integration = &document["tabelas"]["integrations"][0];
    assert_eq!(
        integration["tem_segredo"], true,
        "a marca de que havia segredo"
    );
    assert_eq!(
        integration["segredos_campos"],
        serde_json::json!(["token"]),
        "o nome do campo, sem o valor"
    );

    let destination = Scenario::with_model("destino", true);
    let import_report = destination.ok_json(&[
        "--offline",
        "backup",
        "import",
        file.to_str().expect("caminho"),
        "--json",
    ]);
    assert_eq!(import_report["org"]["criada"], true);
    assert_eq!(import_report["segredos_a_redigitar"], 1);
    assert_eq!(import_report["org"]["id"], origin_org.to_string());
    compare_snapshots(
        &read_snapshot(&origin, origin_org),
        &read_snapshot(&destination, origin_org),
    );
    compare_blobs(&origin, &destination, origin_org, origin_org);

    {
        let store = destination.open_store();
        let agent = agent_by_slug(&store, origin_org, "bia");
        let integrations = store
            .list_integrations(origin_org, agent, poph_store::Page::default())
            .expect("lista integrações");
        assert_eq!(integrations.len(), 1);
        assert!(
            !integrations[0].has_secret,
            "o segredo não pode ser importado"
        );
        assert_eq!(integrations[0].configuracao["instancia"], "bia");
    }

    if model_installed() {
        assert!(
            import_report["aviso"].is_null(),
            "com o modelo instalado não deveria haver aviso: {import_report}"
        );
        let search = destination.ok_json(&[
            "--offline",
            "--org",
            &origin.slug,
            "kb",
            "search",
            "prazo de entrega",
            "--agent",
            "bia",
            "--json",
        ]);
        assert!(
            search["chunks"]
                .as_array()
                .is_some_and(|list| !list.is_empty()),
            "a base importada devia ser buscável: {search}"
        );
    } else {
        assert!(
            import_report["aviso"].is_string(),
            "sem modelo, o import termina com aviso: {import_report}"
        );
    }

    let refusal = destination.failure(&[
        "--offline",
        "backup",
        "import",
        file.to_str().expect("caminho"),
        "--json",
    ]);
    assert!(refusal.contains("--sobrescrever"), "{refusal}");
    assert!(refusal.contains("a importação atualizaria"), "{refusal}");
    assert!(refusal.contains("agents "), "{refusal}");
    assert!(
        refusal.contains(&format!(
            "{} linha(s)",
            import_report["total"].as_u64().expect("total") + 1
        )),
        "a conta inclui a linha da org: {refusal}"
    );
    let state_after_refusal = read_snapshot(&destination, origin_org);
    compare_snapshots(&read_snapshot(&origin, origin_org), &state_after_refusal);

    let second = destination.ok_json(&[
        "--offline",
        "backup",
        "import",
        file.to_str().expect("caminho"),
        "--sobrescrever",
        "--json",
    ]);
    assert_eq!(second["org"]["criada"], false);
    assert_eq!(second["total"], import_report["total"]);
    for (table, saved) in second["tabelas"].as_object().expect("tabelas") {
        assert_eq!(
            saved["inseridas"], 0,
            "`{table}` duplicou na segunda importação"
        );
    }
    compare_snapshots(
        &read_snapshot(&origin, origin_org),
        &read_snapshot(&destination, origin_org),
    );
    let agents = destination
        .open_store()
        .agents()
        .list(origin_org, poph_store::Page::default());
    assert_eq!(agents.expect("lista").len(), 1);
}

#[test]
fn importing_into_existing_org_does_not_touch_neighbor_org() {
    let origin = Scenario::new("origem-org");
    seed_origin(&origin);
    let file = origin.dir.join("backup.json");
    origin.ok_json(&[
        "--offline",
        "backup",
        "export",
        &origin.slug,
        "--out",
        file.to_str().expect("caminho"),
        "--json",
    ]);

    let destination = Scenario::new("destino-org");
    destination.ok_json(&[
        "--offline",
        "org",
        "create",
        "--name",
        "Vizinha",
        "--slug",
        "vizinha",
        "--json",
    ]);
    destination.ok_json(&[
        "--offline",
        "--org",
        "vizinha",
        "agent",
        "create",
        "--name",
        "Vizinho",
        "--slug",
        "vizinho",
        "--json",
    ]);
    let neighbor = destination.org_by_slug("vizinha");
    let before = read_snapshot(&destination, neighbor);

    let import_report = destination.ok_json(&[
        "--offline",
        "backup",
        "import",
        file.to_str().expect("caminho"),
        "--org",
        "vizinha",
        "--json",
    ]);
    assert_eq!(import_report["org"]["id"], neighbor.to_string());
    assert_eq!(import_report["org"]["criada"], false);

    let error = destination.failure(&["--offline", "org", "show", &origin.slug, "--json"]);
    assert!(error.contains("não existe org"), "{error}");

    let after = read_snapshot(&destination, neighbor);
    assert_eq!(
        after.tables["agents"].len(),
        before.tables["agents"].len() + 1,
        "um agente importado além do próprio"
    );
    let store = destination.open_store();
    assert!(store.agents().by_slug(neighbor, "vizinho").is_ok());
    assert!(store.agents().by_slug(neighbor, "bia").is_ok());
    assert_eq!(
        store
            .products()
            .list(neighbor, poph_store::Page::default())
            .expect("produtos")
            .len(),
        1
    );
    let own = before.tables["agents"]
        .iter()
        .find(|row| row["slug"] == "vizinho")
        .expect("agente próprio")
        .clone();
    let same = after.tables["agents"]
        .iter()
        .find(|row| row["slug"] == "vizinho")
        .expect("agente próprio depois");
    assert_eq!(&own, same);
}

#[test]
fn corrupted_or_wrong_version_file_is_refused_without_writing() {
    let origin = Scenario::new("corrompido");
    seed_origin(&origin);
    let file = origin.dir.join("backup.json");
    origin.ok_json(&[
        "--offline",
        "backup",
        "export",
        &origin.slug,
        "--out",
        file.to_str().expect("caminho"),
        "--json",
    ]);
    let texto = fs::read_to_string(&file).expect("lê");

    let destination = Scenario::new("corrompido-destino");
    let truncated = destination.dir.join("truncado.json");
    fs::write(&truncated, &texto[..texto.len() / 2]).expect("grava o truncado");
    let error = destination.failure(&[
        "--offline",
        "backup",
        "import",
        truncated.to_str().expect("caminho"),
        "--json",
    ]);
    assert!(error.contains("JSON inválido"), "{error}");

    let mut document: Value = serde_json::from_str(&texto).expect("JSON");
    document["versao_formato"] = serde_json::json!(99);
    let future = destination.dir.join("futuro.json");
    fs::write(&future, document.to_string()).expect("grava");
    let error = destination.failure(&[
        "--offline",
        "backup",
        "import",
        future.to_str().expect("caminho"),
        "--json",
    ]);
    assert!(error.contains("versão de formato"), "{error}");

    assert!(
        destination
            .open_store()
            .orgs()
            .list(poph_store::Page::default())
            .expect("orgs")
            .is_empty()
    );
}

#[test]
fn export_refuses_overwrite_without_flag() {
    let origin = Scenario::new("sobrescrever");
    seed_origin(&origin);
    let file = origin.dir.join("backup.json");
    fs::write(&file, b"nao-apague-por-engano").expect("grava o arquivo existente");
    let error = origin.failure(&[
        "--offline",
        "backup",
        "export",
        &origin.slug,
        "--out",
        file.to_str().expect("caminho"),
        "--json",
    ]);
    assert!(error.contains("--sobrescrever"), "{error}");
    assert_eq!(
        fs::read_to_string(&file).expect("lê"),
        "nao-apague-por-engano"
    );
    origin.ok_json(&[
        "--offline",
        "backup",
        "export",
        &origin.slug,
        "--out",
        file.to_str().expect("caminho"),
        "--sobrescrever",
        "--json",
    ]);
    assert!(
        fs::read_to_string(&file)
            .expect("lê")
            .contains("poph-backup-org")
    );
}

#[test]
fn export_refuses_blob_missing_on_disk() {
    let origin = Scenario::new("sem-blob");
    seed_origin(&origin);
    let org = origin.org_by_slug(&origin.slug);
    let snapshot = origin
        .open_store()
        .org_snapshot(org, false)
        .expect("retrato");
    let sha = snapshot.tables["file_library"][0]["sha256"]
        .as_str()
        .expect("sha do arquivo da biblioteca")
        .to_owned();
    fs::remove_file(
        origin
            .data_dir()
            .join("blobs")
            .join(org.to_string())
            .join(&sha),
    )
    .expect("apaga o blob");

    let file = origin.dir.join("nao-deve-existir.json");
    let error = origin.failure(&[
        "--offline",
        "backup",
        "export",
        &origin.slug,
        "--out",
        file.to_str().expect("caminho"),
        "--json",
    ]);
    assert!(error.contains("Logo da empresa"), "{error}");
    assert!(error.contains(&sha), "{error}");
    assert!(!file.exists(), "não pode deixar arquivo pela metade");
}

#[test]
fn imported_blob_lands_in_destination_org_folder() {
    let origin = Scenario::new("pasta");
    seed_origin(&origin);
    let origin_org = origin.org_by_slug(&origin.slug);
    let file = origin.dir.join("backup.json");
    origin.ok_json(&[
        "--offline",
        "backup",
        "export",
        &origin.slug,
        "--out",
        file.to_str().expect("caminho"),
        "--json",
    ]);
    let destination = Scenario::new("pasta-destino");
    destination.ok_json(&[
        "--offline",
        "org",
        "create",
        "--name",
        "Alvo",
        "--slug",
        "alvo",
        "--json",
    ]);
    let target = destination.org_by_slug("alvo");
    destination.ok_json(&[
        "--offline",
        "backup",
        "import",
        file.to_str().expect("caminho"),
        "--org",
        "alvo",
        "--json",
    ]);
    let snapshot = read_snapshot(&destination, target);
    for table in ["file_library", "attachments"] {
        for row in snapshot.tables.get(table).into_iter().flatten() {
            let sha = row["sha256"].as_str().expect("sha");
            assert!(
                destination
                    .data_dir()
                    .join("blobs")
                    .join(target.to_string())
                    .join(sha)
                    .is_file(),
                "o blob {sha} devia estar na pasta da org de destino"
            );
        }
    }
    assert!(
        !Path::new(
            &destination
                .data_dir()
                .join("blobs")
                .join(origin_org.to_string())
        )
        .exists()
    );
}
