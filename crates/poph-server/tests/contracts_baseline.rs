use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_fluxos::{Ir, Limites, compilar};
use poph_server::Estado;
use poph_store::{
    Direction, NewAgent, NewCategory, NewConversation, NewDepartment, NewFlow, NewKnowledgeGroup,
    NewKnowledgeItem, NewLibraryFile, NewMessage, NewOrg, NewProduct, NewRecord, Origin,
    RevisionPublication, Role, SpaceOrigin, Store,
};
use poph_types::HarnessConfig;
use poph_types::config::GlobalTokenConfig;
use rand_core::OsRng;
use reqwest::Client;
use serde_json::{Value, json};

const GLOBAL_TOKEN: &str = "pophg_contracts-baseline";
const FIXED_NOW_MS: i64 = 1_700_000_000_000;
const FLOW_SCRIPT: &str = include_str!("../../../tests/fluxos/exemplos/agendar_visita.star");

fn hash_argon2(value: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &salt)
        .expect("argon2 hash")
        .to_string()
}

fn output_dir() -> PathBuf {
    let path = std::env::var_os("POPH_CONTRACT_SNAPSHOT_DIR")
        .map(PathBuf::from)
        .expect("POPH_CONTRACT_SNAPSHOT_DIR is required");
    fs::create_dir_all(&path).expect("snapshot dir");
    path
}

fn work_dir() -> PathBuf {
    let path = std::env::temp_dir().join(format!("poph-contracts-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(path.join("dados")).expect("data dir");
    path
}

fn looks_like_uuid(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() != 36 {
        return false;
    }
    for (index, character) in chars.iter().enumerate() {
        match index {
            8 | 13 | 18 | 23 => {
                if *character != '-' {
                    return false;
                }
            }
            _ => {
                if !character.is_ascii_hexdigit() {
                    return false;
                }
            }
        }
    }
    true
}

fn looks_like_timestamp(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != 20 {
        return false;
    }
    let pattern = b"0000-00-00T00:00:00Z";
    for (index, character) in bytes.iter().enumerate() {
        let ok = match pattern[index] {
            b'0' => character.is_ascii_digit(),
            b'T' => *character == b'T' || *character == b' ',
            b'Z' => *character == b'Z',
            other => *character == other,
        };
        if !ok {
            return false;
        }
    }
    true
}

fn scrub(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if index + 36 <= chars.len() {
            let window: String = chars[index..index + 36].iter().collect();
            if looks_like_uuid(&window) {
                out.push_str("<uuid>");
                index += 36;
                continue;
            }
        }
        if index + 20 <= chars.len() {
            let window: String = chars[index..index + 20].iter().collect();
            if looks_like_timestamp(&window) {
                out.push_str("<timestamp>");
                index += 20;
                continue;
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

fn normalize(value: &mut Value, root: &str) {
    match value {
        Value::String(text) => {
            let replaced = if text.contains(root) {
                text.replace(root, "<data_dir>")
            } else {
                text.clone()
            };
            *text = scrub(&replaced);
        }
        Value::Array(items) => {
            for item in items {
                normalize(item, root);
            }
        }
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if key == "uptime_seconds" {
                    map.insert(key, json!(0));
                    continue;
                }
                if let Some(item) = map.get_mut(&key) {
                    normalize(item, root);
                }
            }
        }
        _ => {}
    }
}

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("snapshot folder");
    }
    let mut text = serde_json::to_string_pretty(value).expect("json");
    text.push('\n');
    fs::write(path, text).expect("write snapshot");
}

fn write_text(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("snapshot folder");
    }
    let mut complete = String::from(text.trim_end());
    complete.push('\n');
    fs::write(path, complete).expect("write snapshot");
}

struct Scenario {
    base: String,
    client: Client,
    folder: PathBuf,
    data_dir: PathBuf,
    blobs: PathBuf,
    store: Arc<Store>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

struct Seeds {
    org: poph_types::OrgId,
    agent: poph_types::AgentId,
    department: poph_types::DepartmentId,
    category: poph_types::ProductCategoryId,
    product: poph_types::ProductId,
    group: poph_types::KnowledgeGroupId,
    item: poph_types::KnowledgeItemId,
    file: poph_types::FileId,
    flow: poph_types::FlowId,
    conversation: poph_types::ConversationId,
    test_chat: poph_types::ConversationId,
    ficha: poph_types::RecordId,
    token: poph_types::AdminTokenId,
    integration: String,
}

impl Scenario {
    async fn new() -> Self {
        let folder = work_dir();
        let mut config = HarnessConfig::default();
        config.data.directory = folder.join("dados");
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "contracts".to_owned(),
            hash: hash_argon2(GLOBAL_TOKEN),
        }];
        config.validar().expect("valid config");
        let data_dir = config.data.directory.clone();
        let blobs = config.blobs();

        let store = Arc::new(
            Store::open(&config)
                .expect("open store")
                .with_clock(Arc::new(|| FIXED_NOW_MS)),
        );
        let state = Estado::novo(Arc::clone(&store), config).expect("server state");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("ephemeral port");
        let address = listener.local_addr().expect("local addr");
        let (stop, signal) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = poph_server::servir_em(listener, state, async {
                let _ = signal.await;
            })
            .await;
        });
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("http client");
        let scenario = Self {
            base: format!("http://{address}"),
            client,
            folder,
            data_dir,
            blobs,
            store,
            stop: Some(stop),
        };
        scenario.wait_until_up().await;
        scenario
    }

    async fn wait_until_up(&self) {
        for _ in 0..200 {
            if let Ok(response) = self
                .client
                .get(format!("{}/healthz", self.base))
                .send()
                .await
                && response.status().is_success()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("server did not start");
    }

    async fn capture(&self, name: &str, path: &str) {
        let response = self
            .client
            .get(format!("{}{path}", self.base))
            .bearer_auth(GLOBAL_TOKEN)
            .header("accept", "application/json")
            .send()
            .await
            .unwrap_or_else(|error| panic!("{path}: {error}"));
        let status = response.status().as_u16();
        let text = response.text().await.expect("body");
        let body: Value = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(_) => json!({ "non_json": scrub(&text) }),
        };
        let mut record = json!({ "status": status, "body": body });
        normalize(&mut record, &self.data_dir.display().to_string());
        let file = name.replace('/', "_");
        write_json(
            &output_dir().join("api").join(format!("{file}.json")),
            &record,
        );
        assert!(status < 500, "{path} answered {status}: {}", record["body"]);
    }

    fn seed(&self) -> Seeds {
        let store = &self.store;
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Contratos Ltda".to_owned(),
                slug: "contratos".to_owned(),
                config_json: String::new(),
            })
            .expect("org");
        let department = store
            .departments()
            .create(
                org.id,
                NewDepartment {
                    name: "Suporte".to_owned(),
                    description: "Atendimento de primeiro nivel".to_owned(),
                },
            )
            .expect("department");
        let agent = store
            .agents()
            .create(
                org.id,
                NewAgent {
                    name: "Ana".to_owned(),
                    slug: "ana".to_owned(),
                    description: "Agente de suporte".to_owned(),
                    default_department: Some(department.id),
                    rag_top_k: 4,
                    ..NewAgent::default()
                },
            )
            .expect("agent");
        store
            .instructions()
            .save(
                org.id,
                agent.id,
                0,
                r#"{"identity":{"name":"Ana","role":"Atendente"}}"#,
                "Voce e a Ana, atendente de suporte.",
                true,
            )
            .expect("instructions");
        let category = store
            .categories()
            .create(
                org.id,
                NewCategory {
                    name: "Planos".to_owned(),
                    description: "Planos comerciais".to_owned(),
                },
            )
            .expect("category");
        let product = store
            .products()
            .create(
                org.id,
                NewProduct {
                    category_id: Some(category.id),
                    name: "Plano Basico".to_owned(),
                    slug: Some("plano-basico".to_owned()),
                    aliases: vec!["basico".to_owned()],
                    description: "Plano de entrada".to_owned(),
                    price_text: "R$ 99,00".to_owned(),
                    price_cents: 9900,
                    unit: "mes".to_owned(),
                    includes: "Suporte por WhatsApp".to_owned(),
                    exclui: "Visitas".to_owned(),
                    conditions: "Contrato de 12 meses".to_owned(),
                    tags: vec!["plano".to_owned()],
                },
            )
            .expect("product");
        let group = store
            .knowledge()
            .create_group(
                org.id,
                NewKnowledgeGroup {
                    name: "Perguntas frequentes".to_owned(),
                    description: "Duvidas recorrentes".to_owned(),
                },
            )
            .expect("knowledge group");
        let item = store
            .knowledge()
            .create_item(
                org.id,
                NewKnowledgeItem {
                    group_id: group.id,
                    title: "Horario de atendimento".to_owned(),
                    content: "De segunda a sexta, das 8h as 18h.".to_owned(),
                    tags: vec!["horario".to_owned()],
                    position: 0,
                },
            )
            .expect("knowledge item");
        store
            .knowledge()
            .link_agent(org.id, agent.id, group.id)
            .expect("link group");
        let content = b"conteudo de exemplo";
        let blob = poph_files::blob::write(&self.blobs, org.id, content).expect("blob");
        let file = store
            .library()
            .create(
                org.id,
                NewLibraryFile {
                    agent_id: agent.id,
                    name: "exemplo.txt".to_owned(),
                    description: "Arquivo de exemplo".to_owned(),
                    mime: "text/plain".to_owned(),
                    size_bytes: content.len() as u64,
                    sha256: blob.sha256,
                    blob_path: format!("{}/{}", org.id, poph_files::blob::sha256_hex(content)),
                },
            )
            .expect("file");

        let compiled = compilar(FLOW_SCRIPT, &Limites::default());
        let ir: Ir = compiled.ir().expect("valid flow script").clone();
        let flow = store
            .flows()
            .create(
                org.id,
                NewFlow {
                    slug: "agendar_visita".to_owned(),
                    name: "Agendar visita".to_owned(),
                    description: "Marca a visita tecnica".to_owned(),
                    rascunho_fonte: String::new(),
                },
            )
            .expect("flow");
        let second_script = FLOW_SCRIPT.replace(
            "Marca uma visita técnica no endereço do cliente.",
            "Marca uma visita técnica no endereço do cliente, com confirmação.",
        );
        for (expected, script) in [(1u64, FLOW_SCRIPT.to_owned()), (2, second_script)] {
            store
                .revisions()
                .publish_revision(
                    org.id,
                    flow.id,
                    RevisionPublication {
                        expected,
                        fonte: script.clone(),
                        definicao_json: compilar(&script, &Limites::default())
                            .ir()
                            .expect("valid flow script")
                            .to_json(),
                        schema_versao: Some(ir.schema_versao),
                        resolucoes_json: String::new(),
                        ator_tipo: "cli".to_owned(),
                        actor_id: Some("contracts".to_owned()),
                    },
                )
                .expect("publish revision");
        }
        store
            .flows()
            .release(org.id, agent.id, flow.id)
            .expect("release flow");

        let conversation = store
            .conversations()
            .create(
                org.id,
                NewConversation {
                    agent_id: agent.id,
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some("5521000000000".to_owned()),
                    target_department: Some(department.id),
                },
            )
            .expect("conversation");
        let turn = store
            .turns()
            .start(org.id, conversation.id, Some(agent.id))
            .expect("turn");
        for (direction, role, origin, text) in [
            (Direction::Inbound, Role::User, Origin::Device, "Ola"),
            (
                Direction::Outbound,
                Role::Assistant,
                Origin::Agente,
                "Ola, como posso ajudar?",
            ),
        ] {
            store
                .messages()
                .write(
                    org.id,
                    NewMessage {
                        conversation_id: conversation.id,
                        direction,
                        role,
                        origin,
                        author_name: None,
                        texto: text.to_owned(),
                        attachments_json: "[]".to_owned(),
                        external_id: None,
                        turn_id: Some(turn.id),
                    },
                )
                .expect("message");
        }
        let test_chat = store
            .conversations()
            .create(
                org.id,
                NewConversation {
                    agent_id: agent.id,
                    integration_id: None,
                    canal: "test".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("test chat conversation");
        store
            .messages()
            .write(
                org.id,
                NewMessage {
                    conversation_id: test_chat.id,
                    direction: Direction::Inbound,
                    role: Role::User,
                    origin: Origin::Agente,
                    author_name: None,
                    texto: "Mensagem do chat de teste".to_owned(),
                    attachments_json: "[]".to_owned(),
                    external_id: None,
                    turn_id: None,
                },
            )
            .expect("test chat message");
        let ficha = store
            .cases()
            .by_conversation(org.id, conversation.id)
            .expect("ficha lookup")
            .unwrap_or_else(|| {
                store
                    .cases()
                    .create(
                        org.id,
                        NewRecord {
                            conversation_id: Some(conversation.id),
                            agent_id: agent.id,
                            canal: "whatsapp".to_owned(),
                            peer_externo: Some("5521000000000".to_owned()),
                        },
                    )
                    .expect("ficha")
            });
        let token = store
            .tokens()
            .create(Some(org.id), "leitura", &hash_argon2("poph_contracts"))
            .expect("token");
        let integration = store
            .create_integration(org.id, agent.id, "whatsapp".to_owned(), "active".to_owned())
            .expect("integration");
        store.matrix().enable(org.id).expect("matrix on");
        store
            .matrix()
            .ensure_space(
                org.id,
                department.id,
                "!espaco:contratos.test",
                "#suporte:contratos.test",
                SpaceOrigin::Automatic,
            )
            .expect("matrix space");

        Seeds {
            org: org.id,
            agent: agent.id,
            department: department.id,
            category: category.id,
            product: product.id,
            group: group.id,
            item: item.id,
            file: file.id,
            flow: flow.id,
            conversation: conversation.id,
            test_chat: test_chat.id,
            ficha: ficha.id,
            token: token.id,
            integration: integration.id.clone(),
        }
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let _ = fs::remove_dir_all(&self.folder);
    }
}

fn normalize_sql(sql: &str) -> String {
    let mut without_comments = String::with_capacity(sql.len());
    let mut inside_string = false;
    let mut chars = sql.chars().peekable();
    while let Some(character) = chars.next() {
        if inside_string {
            without_comments.push(character);
            if character == '\'' {
                inside_string = false;
            }
            continue;
        }
        if character == '\'' {
            inside_string = true;
            without_comments.push(character);
            continue;
        }
        if character == '-' && chars.peek() == Some(&'-') {
            for next in chars.by_ref() {
                if next == '\n' {
                    break;
                }
            }
            without_comments.push(' ');
            continue;
        }
        without_comments.push(character);
    }
    without_comments
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn dump_schema(path: &Path) -> String {
    let connection = rusqlite::Connection::open(path).expect("open database");
    let mut query = connection
        .prepare(
            "SELECT type, name, tbl_name, COALESCE(sql, '') FROM sqlite_master
             WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
        )
        .expect("prepare");
    let rows = query
        .query_map([], |row| {
            Ok(format!(
                "{}|{}|{}|{}",
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                normalize_sql(&row.get::<_, String>(3)?)
            ))
        })
        .expect("query");
    rows.collect::<rusqlite::Result<Vec<String>>>()
        .expect("collect")
        .join("\n")
}

fn dump_migrations(path: &Path) -> String {
    let connection = rusqlite::Connection::open(path).expect("open database");
    let mut query = connection
        .prepare("SELECT version, name FROM schema_migrations ORDER BY version")
        .expect("prepare");
    let rows = query
        .query_map([], |row| {
            Ok(format!(
                "{}|{}",
                row.get::<_, u32>(0)?,
                row.get::<_, String>(1)?
            ))
        })
        .expect("query");
    rows.collect::<rusqlite::Result<Vec<String>>>()
        .expect("collect")
        .join("\n")
}

const BOOTSTRAP: &str = "CREATE TABLE IF NOT EXISTS schema_migrations (
    version    INTEGER PRIMARY KEY NOT NULL,
    name       TEXT    NOT NULL,
    applied_at INTEGER NOT NULL
) STRICT;";

fn upgraded_schema(folder: &Path) -> PathBuf {
    let harness = folder.join("antigo").join("harness.db");
    let rag = folder.join("antigo").join("rag.db");
    fs::create_dir_all(harness.parent().expect("folder")).expect("old folder");
    let connection = rusqlite::Connection::open(&harness).expect("open old database");
    connection.execute_batch(BOOTSTRAP).expect("bootstrap");
    let first = &poph_store::HARNESS_MIGRATIONS[0];
    connection.execute_batch(first.sql).expect("migration 1");
    connection
        .execute(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![first.version, first.name, FIXED_NOW_MS],
        )
        .expect("migration row");
    drop(connection);
    Store::open_at(&harness, &rag).expect("upgrade through the runner");
    harness
}

#[tokio::test]
#[ignore = "capture tool: run through scripts/contracts.sh"]
async fn capture_contracts() {
    let scenario = Scenario::new().await;
    let seeds = scenario.seed();
    let org = seeds.org.to_string();
    let agent = seeds.agent.to_string();
    let base = format!("/admin/v1/orgs/{org}");
    let flow = seeds.flow.to_string();
    let ficha = seeds.ficha.to_string();
    let group = seeds.group.to_string();
    let item = seeds.item.to_string();
    let conversation = seeds.conversation.to_string();

    for (name, path) in [
        ("healthz", "/healthz".to_owned()),
        ("orgs", "/admin/v1/orgs".to_owned()),
        ("orgs-id", base.clone()),
        ("agents", format!("{base}/agents")),
        ("agents-id", format!("{base}/agents/{agent}")),
        ("agent-flows", format!("{base}/agents/{agent}/flows")),
        (
            "agent-instructions",
            format!("{base}/agents/{agent}/instructions"),
        ),
        (
            "agent-instructions-current",
            format!("{base}/agents/{agent}/instructions/current"),
        ),
        ("agent-files", format!("{base}/agents/{agent}/files")),
        (
            "agent-files-id",
            format!("{base}/agents/{agent}/files/{}", seeds.file),
        ),
        (
            "agent-files-content",
            format!("{base}/agents/{agent}/files/{}/content", seeds.file),
        ),
        (
            "agent-integrations",
            format!("{base}/agents/{agent}/integrations"),
        ),
        (
            "agent-integrations-id",
            format!("{base}/agents/{agent}/integrations/{}", seeds.integration),
        ),
        (
            "agent-test-chat",
            format!("{base}/agents/{agent}/test-chat"),
        ),
        (
            "agent-test-chat-id",
            format!("{base}/agents/{agent}/test-chat/{}", seeds.test_chat),
        ),
        (
            "agent-whatsapp-state",
            format!(
                "{base}/agents/{agent}/integrations/{}/whatsapp/state",
                seeds.integration
            ),
        ),
        ("flows", format!("{base}/flows")),
        ("flows-id", format!("{base}/flows/{flow}")),
        ("flows-revisions", format!("{base}/flows/{flow}/revisions")),
        (
            "flows-revisions-1",
            format!("{base}/flows/{flow}/revisions/1"),
        ),
        (
            "flows-diff",
            format!("{base}/flows/{flow}/diff?from=1&to=2"),
        ),
        ("flows-preview", format!("{base}/flows/{flow}/preview")),
        ("fichas", format!("{base}/fichas")),
        ("fichas-id", format!("{base}/fichas/{ficha}")),
        ("fichas-events", format!("{base}/fichas/{ficha}/events")),
        ("knowledge", format!("{base}/knowledge")),
        ("knowledge-group", format!("{base}/knowledge/{group}")),
        ("knowledge-items", format!("{base}/knowledge/{group}/items")),
        (
            "knowledge-items-id",
            format!("{base}/knowledge/{group}/items/{item}"),
        ),
        ("tokens", format!("{base}/tokens")),
        ("tokens-id", format!("{base}/tokens/{}", seeds.token)),
        ("categories", format!("{base}/categories")),
        (
            "categories-id",
            format!("{base}/categories/{}", seeds.category),
        ),
        ("products", format!("{base}/products")),
        ("products-id", format!("{base}/products/{}", seeds.product)),
        ("departments", format!("{base}/departments")),
        (
            "departments-id",
            format!("{base}/departments/{}", seeds.department),
        ),
        ("conversations", format!("{base}/conversations")),
        (
            "conversations-id",
            format!("{base}/conversations/{conversation}"),
        ),
        (
            "conversations-messages",
            format!("{base}/conversations/{conversation}/messages"),
        ),
        (
            "conversations-turns",
            format!("{base}/conversations/{conversation}/turns"),
        ),
        ("matrix", format!("{base}/matrix")),
        ("matrix-spaces", format!("{base}/matrix/spaces")),
        ("manifests", format!("{base}/manifests")),
        ("manifests-whatsapp", format!("{base}/manifests/whatsapp")),
        ("backup", format!("{base}/backup")),
    ] {
        scenario.capture(name, &path).await;
    }

    let output = output_dir();
    let harness = scenario.store.harness_path().to_path_buf();
    let rag = scenario.store.rag_path().to_path_buf();
    write_text(
        &output.join("db/harness-schema.txt"),
        &dump_schema(&harness),
    );
    write_text(
        &output.join("db/harness-migrations.txt"),
        &dump_migrations(&harness),
    );
    write_text(&output.join("db/rag-schema.txt"), &dump_schema(&rag));

    let old = upgraded_schema(&scenario.folder);
    let fresh = dump_schema(&harness);
    let upgraded = dump_schema(&old);
    write_text(&output.join("db/harness-schema-upgraded.txt"), &upgraded);
    assert_eq!(fresh, upgraded, "upgraded schema differs from fresh schema");
    write_text(
        &output.join("db/harness-migrations-upgraded.txt"),
        &dump_migrations(&old),
    );

    let metrics = scenario
        .client
        .get(format!("{}/metrics", scenario.base))
        .send()
        .await
        .expect("metrics")
        .text()
        .await
        .expect("metrics body");
    let mut names: Vec<String> = metrics
        .lines()
        .filter(|line| line.starts_with("# TYPE poph_"))
        .map(|line| line.trim_end().to_owned())
        .collect();
    names.sort();
    names.dedup();
    write_text(&output.join("metrics/types.txt"), &names.join("\n"));

    for example in ["agendar_visita", "confirmar_entrega", "triagem_suporte"] {
        let text = fs::read_to_string(format!(
            "{}/../../tests/fluxos/exemplos/{example}.star",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("flow example");
        let compiled = compilar(&text, &Limites::default());
        assert!(
            compiled.diagnosticos.is_empty(),
            "{example}: {:?}",
            compiled.diagnosticos
        );
        let ir = compiled.ir().expect("ir");
        let value: Value = serde_json::from_str(&ir.to_json()).expect("ir json");
        write_json(
            &output.join("flows").join(format!("{example}.ir.json")),
            &value,
        );
    }

    let config = toml::to_string_pretty(&HarnessConfig::default()).expect("default config");
    write_text(&output.join("config/default.toml"), &config);
}
