use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use poph_retention::{
    DatabaseSweep, HarnessDatabase, Options, OrgReport, RetentionError, Situacao, run_all, run_org,
};
use poph_store::{
    ExecutionStart, ExecutionState, NewConversation, NewFlow, RevisionPublication, Store,
    TransitionEffect,
};
use poph_types::{AgentId, EventActor, ExecutionStatus, OrgId};
use rusqlite::{Connection, params, types::Value};
use tempfile::TempDir;

const DAY_MS: i64 = 86_400_000;
const HOUR_MS: i64 = 3_600_000;
const NOW_MS: i64 = 1_795_131_511_000;
const RECENT_MS: i64 = NOW_MS - HOUR_MS;

const ORG_TABLES: &[&str] = &[
    "agents",
    "agent_instructions",
    "agent_product_categories",
    "departments",
    "product_categories",
    "products",
    "knowledge_groups",
    "knowledge_items",
    "agent_knowledge_groups",
    "file_library",
    "integrations",
    "conversations",
    "messages",
    "attachments",
    "carts",
    "cart_items",
    "inbox",
    "outbox",
    "turns",
];

const PROTECTED_TABLES: &[&str] = &[
    "agents",
    "agent_instructions",
    "agent_product_categories",
    "departments",
    "product_categories",
    "products",
    "knowledge_groups",
    "knowledge_items",
    "agent_knowledge_groups",
    "file_library",
    "integrations",
];

fn sha(n: u64) -> String {
    format!("{n:064x}")
}

fn new_id() -> String {
    OrgId::generate().to_string()
}

struct Scenario {
    store: Store,
    harness: PathBuf,
    blobs: PathBuf,
    _folder: TempDir,
}

impl Scenario {
    fn new() -> Self {
        let folder = TempDir::new().expect("pasta temporária");
        let data = folder.path().join("dados");
        let harness = data.join("harness.db");
        let rag = data.join("rag.db");
        let store = Store::open_at(&harness, &rag).expect("abre o banco de teste");
        Self {
            store,
            blobs: data.join("blobs"),
            harness,
            _folder: folder,
        }
    }

    fn connection(&self) -> Connection {
        let conn = Connection::open(&self.harness).expect("abre o banco");
        conn.pragma_update(None, "foreign_keys", "ON")
            .expect("chaves estrangeiras");
        conn.busy_timeout(Duration::from_secs(5))
            .expect("tempo de espera");
        conn
    }

    fn warehouse(&self) -> HarnessDatabase<'_> {
        HarnessDatabase::new(&self.store, &self.blobs)
    }

    fn org_folder(&self, org: &str) -> PathBuf {
        self.blobs.join(org)
    }
}

fn org(c: &Connection, slug: &str, config: &str) -> String {
    let id = new_id();
    c.execute(
        "INSERT INTO orgs (id, name, slug, status, config_json, version, created_at, updated_at)
         VALUES (?1, ?2, ?2, 'active', ?3, 1, ?4, ?4)",
        params![id, slug, config, NOW_MS - 400 * DAY_MS],
    )
    .expect("org");
    id
}

fn insert_agent(c: &Connection, org: &str) -> String {
    let id = new_id();
    c.execute(
        "INSERT INTO agents (id, org_id, name, slug, provider, model, created_at, updated_at)
         VALUES (?1, ?2, 'Bia', ?3, 'antigravity2api', 'modelo-teste', ?4, ?4)",
        params![id, org, format!("bia-{id}"), NOW_MS - 400 * DAY_MS],
    )
    .expect("agente");
    id
}

fn insert_conversation(c: &Connection, org: &str, agent: &str, created: i64) -> String {
    let id = new_id();
    c.execute(
        "INSERT INTO conversations (id, org_id, agent_id, channel, status, version, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'cli', 'open', 1, ?4, ?4)",
        params![id, org, agent, created],
    )
    .expect("conversa");
    id
}

fn insert_message(
    c: &Connection,
    org: &str,
    conversation: &str,
    created: i64,
    turn: Option<&str>,
) -> String {
    let id = new_id();
    c.execute(
        "INSERT INTO messages (id, org_id, conversation_id, direction, role, text, created_at, turn_id)
         VALUES (?1, ?2, ?3, 'in', 'user', 'oi', ?4, ?5)",
        params![id, org, conversation, created, turn],
    )
    .expect("mensagem");
    id
}

fn insert_attachment(c: &Connection, org: &str, message: &str, sha256: &str, created: i64) {
    c.execute(
        "INSERT INTO attachments (id, org_id, message_id, name, mime, size_bytes, sha256, blob_path, created_at)
         VALUES (?1, ?2, ?3, 'doc.pdf', 'application/pdf', 10, ?4, ?5, ?6)",
        params![new_id(), org, message, sha256, format!("{org}/{sha256}"), created],
    )
    .expect("anexo");
}

fn insert_turn(
    c: &Connection,
    org: &str,
    conversation: &str,
    agent: &str,
    state: &str,
    started: i64,
    finished: Option<i64>,
) -> String {
    let id = new_id();
    c.execute(
        "INSERT INTO turns (id, org_id, conversation_id, agent_id, state, started_at, finished_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, org, conversation, agent, state, started, finished],
    )
    .expect("turno");
    id
}

fn insert_output(
    c: &Connection,
    org: &str,
    conversation: &str,
    turn: &str,
    state: &str,
    created: i64,
    sent: Option<i64>,
) {
    let seq: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM outbox WHERE turn_id = ?1",
            params![turn],
            |row| row.get(0),
        )
        .expect("seq");
    c.execute(
        "INSERT INTO outbox (id, org_id, conversation_id, turn_id, seq, kind, payload_json, state, attempts, created_at, sent_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'text', '{}', ?6, 0, ?7, ?8)",
        params![new_id(), org, conversation, turn, seq, state, created, sent],
    )
    .expect("saída");
}

fn insert_input(c: &Connection, org: &str, received: i64, processed: Option<i64>) {
    c.execute(
        "INSERT INTO inbox (id, org_id, integration_id, external_message_id, payload_json, received_at, processed_at, attempts)
         VALUES (?1, ?2, ?3, ?4, '{}', ?5, ?6, 0)",
        params![new_id(), org, new_id(), new_id(), received, processed],
    )
    .expect("entrada");
}

fn insert_library_file(c: &Connection, org: &str, agent: &str, sha256: &str) {
    c.execute(
        "INSERT INTO file_library (id, org_id, agent_id, name, mime, size_bytes, sha256, blob_path, created_at)
         VALUES (?1, ?2, ?3, 'tabela.xlsx', 'application/vnd.ms-excel', 10, ?4, ?5, ?6)",
        params![new_id(), org, agent, sha256, format!("{org}/{sha256}"), NOW_MS - 400 * DAY_MS],
    )
    .expect("biblioteca");
}

fn insert_knowledge_and_products(c: &Connection, org: &str) {
    let group = new_id();
    c.execute(
        "INSERT INTO knowledge_groups (id, org_id, name, created_at, updated_at) VALUES (?1, ?2, 'Horários', ?3, ?3)",
        params![group, org, NOW_MS - 400 * DAY_MS],
    )
    .expect("grupo da base");
    c.execute(
        "INSERT INTO knowledge_items (id, org_id, group_id, title, content, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'Seg a sex', 'Das 8h às 18h', ?4, ?4)",
        params![new_id(), org, group, NOW_MS - 400 * DAY_MS],
    )
    .expect("item da base");
    let category = new_id();
    c.execute(
        "INSERT INTO product_categories (id, org_id, name, created_at, updated_at) VALUES (?1, ?2, 'Planos', ?3, ?3)",
        params![category, org, NOW_MS - 400 * DAY_MS],
    )
    .expect("categoria");
    c.execute(
        "INSERT INTO products (id, org_id, category_id, name, slug, price_text, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'Plano A', ?4, 'R$ 10', ?5, ?5)",
        params![new_id(), org, category, format!("plano-{category}"), NOW_MS - 400 * DAY_MS],
    )
    .expect("produto");
}

fn seed(c: &Connection, slug: &str, config: &str, old_age_days: i64) -> String {
    let old_ms = NOW_MS - old_age_days * DAY_MS;
    let org = org(c, slug, config);
    let agent_id = insert_agent(c, &org);

    insert_conversation(c, &org, &agent_id, old_ms);
    let only_old = insert_conversation(c, &org, &agent_id, old_ms);
    let mixed = insert_conversation(c, &org, &agent_id, old_ms);
    let recent = insert_conversation(c, &org, &agent_id, RECENT_MS);

    let m1 = insert_message(c, &org, &only_old, old_ms, None);
    insert_attachment(c, &org, &m1, &sha(1), old_ms);
    insert_message(c, &org, &only_old, old_ms + 1, None);
    let m3 = insert_message(c, &org, &mixed, old_ms, None);
    insert_attachment(c, &org, &m3, &sha(2), old_ms);
    let n1 = insert_message(c, &org, &mixed, RECENT_MS, None);
    insert_attachment(c, &org, &n1, &sha(3), RECENT_MS);
    insert_message(c, &org, &recent, RECENT_MS, None);

    let done = insert_turn(
        c,
        &org,
        &recent,
        &agent_id,
        "done",
        old_ms,
        Some(old_ms + 5),
    );
    insert_turn(c, &org, &recent, &agent_id, "running", old_ms, None);
    let pending = insert_turn(
        c,
        &org,
        &recent,
        &agent_id,
        "done",
        old_ms,
        Some(old_ms + 5),
    );
    let today = insert_turn(
        c,
        &org,
        &recent,
        &agent_id,
        "done",
        RECENT_MS,
        Some(RECENT_MS + 5),
    );
    insert_output(c, &org, &recent, &done, "sent", old_ms, Some(old_ms + 5));
    insert_output(c, &org, &recent, &pending, "pending", old_ms, None);
    insert_output(c, &org, &recent, &pending, "failed", old_ms, None);
    insert_output(c, &org, &recent, &today, "sent", RECENT_MS, Some(RECENT_MS));

    insert_input(c, &org, old_ms, Some(old_ms + 1));
    insert_input(c, &org, old_ms, None);
    insert_input(c, &org, RECENT_MS, Some(RECENT_MS));

    insert_library_file(c, &org, &agent_id, &sha(9));
    insert_knowledge_and_products(c, &org);
    org
}

fn snapshot(c: &Connection, org: &str, tables: &[&str]) -> BTreeMap<String, Vec<String>> {
    let mut map = BTreeMap::new();
    for table in tables {
        let mut query = c
            .prepare(&format!(
                "SELECT * FROM {table} WHERE org_id = ?1 ORDER BY rowid"
            ))
            .expect("consulta da foto");
        let columns = query.column_count();
        let rows = query
            .query_map(params![org], |row| {
                let mut parts = Vec::with_capacity(columns);
                for i in 0..columns {
                    let value: Value = row.get(i)?;
                    parts.push(format!("{value:?}"));
                }
                Ok(parts.join("|"))
            })
            .expect("foto");
        let rows: Vec<String> = rows.map(|row| row.expect("linha da foto")).collect();
        map.insert((*table).to_owned(), rows);
    }
    map
}

fn count_rows(c: &Connection, table: &str, org: &str) -> i64 {
    c.query_row(
        &format!("SELECT COUNT(*) FROM {table} WHERE org_id = ?1"),
        params![org],
        |row| row.get(0),
    )
    .expect("contagem")
}

fn audit_entries(c: &Connection, org: &str) -> Vec<serde_json::Value> {
    let mut query = c
        .prepare(
            "SELECT detail_json FROM audit_log
              WHERE org_id = ?1 AND action = 'retention' ORDER BY created_at, id",
        )
        .expect("consulta da auditoria");
    let rows = query
        .query_map(params![org], |row| row.get::<_, String>(0))
        .expect("auditoria");
    rows.map(|row| {
        let text = row.expect("linha da auditoria");
        serde_json::from_str(&text).expect("detalhe em JSON")
    })
    .collect()
}

fn files_in(folder: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut map = BTreeMap::new();
    let Ok(entries) = fs::read_dir(folder) else {
        return map;
    };
    for entry in entries {
        let path = entry.expect("entrada").path();
        if path.is_file() {
            let file_name = path
                .file_name()
                .expect("nome")
                .to_string_lossy()
                .into_owned();
            map.insert(file_name, fs::read(&path).expect("lê o arquivo"));
        }
    }
    map
}

fn write_blob_file(folder: &Path, file_name: &str, age_ms: i64) {
    fs::create_dir_all(folder).expect("pasta da org");
    let path = folder.join(file_name);
    fs::write(&path, file_name.as_bytes()).expect("grava o arquivo");
    let file = fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("abre o arquivo");
    file.set_modified(UNIX_EPOCH + Duration::from_millis((NOW_MS - age_ms) as u64))
        .expect("fixa a data do arquivo");
}

fn options_for(default_days: u32, simulate: bool) -> Options {
    Options::new(NOW_MS, default_days, simulate, "system")
}

fn org_id(text: &str) -> OrgId {
    text.parse().expect("id de org válido")
}

fn database_of(report: &OrgReport) -> DatabaseSweep {
    report
        .database
        .clone()
        .expect("org varrida tem contagem do banco")
}

#[test]
fn sweeps_only_the_expired_with_exact_counts() {
    let scenario = Scenario::new();
    let c = scenario.connection();
    let org = seed(&c, "principal", "{}", 40);
    let warehouse = scenario.warehouse();

    let report = run_org(&warehouse, org_id(&org), &options_for(30, false)).expect("varredura");

    assert_eq!(report.situacao, Situacao::Swept, "{:?}", report.motivo);
    let database = database_of(&report);
    assert_eq!(database.messages, 3, "mensagens antigas");
    assert_eq!(database.anexos, 2, "anexos das mensagens antigas");
    assert_eq!(database.turns, 1, "só o turno encerrado sem saída pendente");
    assert_eq!(database.outbox, 1, "só a saída enviada e antiga");
    assert_eq!(database.inbox, 1, "só a entrada processada e antiga");
    assert_eq!(database.conversas, 2, "a vazia e a que ficou sem mensagem");
    assert_eq!(
        database.conversation_ids.len(),
        3,
        "as três conversas afetadas"
    );
    assert_eq!(
        database.referenced_hashes,
        vec![sha(3), sha(9)],
        "anexo vivo e biblioteca continuam com dono"
    );

    assert_eq!(count_rows(&c, "messages", &org), 2);
    assert_eq!(count_rows(&c, "attachments", &org), 1);
    assert_eq!(count_rows(&c, "conversations", &org), 2);
    assert_eq!(
        count_rows(&c, "turns", &org),
        3,
        "rodando, pendente e recente"
    );
    assert_eq!(
        count_rows(&c, "outbox", &org),
        3,
        "pendente, com falha e enviada recente"
    );
    assert_eq!(
        count_rows(&c, "inbox", &org),
        2,
        "não processada e processada recente"
    );

    let again = run_org(&warehouse, org_id(&org), &options_for(30, false)).expect("de novo");
    let database2 = database_of(&again);
    assert_eq!(
        database2.messages
            + database2.anexos
            + database2.turns
            + database2.inbox
            + database2.outbox
            + database2.conversas,
        0
    );
}

#[test]
fn zero_retention_deletes_nothing_even_with_ten_year_old_data() {
    let scenario = Scenario::new();
    let c = scenario.connection();
    let org = seed(&c, "guardada", r#"{"retencao":{"dias":0}}"#, 3650);
    let before = snapshot(&c, &org, ORG_TABLES);

    let report =
        run_org(&scenario.warehouse(), org_id(&org), &options_for(30, false)).expect("varredura");

    assert_eq!(report.situacao, Situacao::Skipped);
    assert_eq!(report.database, None);
    assert_eq!(
        snapshot(&c, &org, ORG_TABLES),
        before,
        "nada mudou no banco"
    );
    assert!(
        audit_entries(&c, &org).is_empty(),
        "org pulada não gera auditoria"
    );
}

#[test]
fn knowledge_products_and_library_stay_intact_under_the_most_aggressive_sweep() {
    let scenario = Scenario::new();
    let c = scenario.connection();
    let org = seed(&c, "agressiva", "{}", 400);
    let before = snapshot(&c, &org, PROTECTED_TABLES);
    assert!(!before["knowledge_items"].is_empty() && !before["products"].is_empty());

    run_org(&scenario.warehouse(), org_id(&org), &options_for(1, false)).expect("varredura");

    assert_eq!(
        snapshot(&c, &org, PROTECTED_TABLES),
        before,
        "cadastro e biblioteca intactos"
    );
    assert!(
        count_rows(&c, "conversations", &org) >= 1,
        "a conversa recente continua"
    );
}

#[test]
fn simulation_changes_neither_database_nor_folder_and_counts_what_would_go() {
    let scenario = Scenario::new();
    let c = scenario.connection();
    let org = seed(&c, "simulada", "{}", 40);
    let folder = scenario.org_folder(&org);
    write_blob_file(&folder, &sha(1), 40 * DAY_MS);
    write_blob_file(&folder, &sha(2), 40 * DAY_MS);
    write_blob_file(&folder, &sha(3), 40 * DAY_MS);
    write_blob_file(&folder, &sha(7), 40 * DAY_MS);
    write_blob_file(&folder, "pendente.tmp", 40 * DAY_MS);
    let before_database = snapshot(&c, &org, ORG_TABLES);
    let before_files = files_in(&folder);
    let warehouse = scenario.warehouse();

    let simulated = run_org(&warehouse, org_id(&org), &options_for(30, true)).expect("simulação");

    assert_eq!(simulated.situacao, Situacao::Swept);
    assert_eq!(simulated.deleted_files, 0);
    assert_eq!(
        snapshot(&c, &org, ORG_TABLES),
        before_database,
        "banco idêntico após a simulação"
    );
    assert_eq!(
        files_in(&folder),
        before_files,
        "pasta de blobs idêntica após a simulação"
    );
    let audit = audit_entries(&c, &org);
    assert_eq!(audit.len(), 1, "a simulação deixa uma linha de auditoria");
    assert_eq!(audit[0]["simulacao"], true, "marcada como simulação");
    assert_eq!(audit[0]["mensagens"], 3, "a auditoria traz o que sairia");

    let real = run_org(&warehouse, org_id(&org), &options_for(30, false)).expect("real");
    assert_eq!(
        database_of(&real),
        database_of(&simulated),
        "mesmas contagens"
    );
    assert_eq!(
        real.orphan_files, simulated.orphan_files,
        "mesmos arquivos órfãos"
    );
    assert_eq!(real.deleted_files, 4, "três blobs órfãos e um temporário");
    let remaining: Vec<String> = files_in(&folder).into_keys().collect();
    assert_eq!(remaining, vec![sha(3)], "só o blob vivo fica");
}

#[test]
fn orphan_blobs_go_referenced_stay_and_the_other_org_is_untouched() {
    let scenario = Scenario::new();
    let c = scenario.connection();
    let a = seed(&c, "a", "{}", 40);
    let b = seed(&c, "b", "{}", 40);
    let folder_a = scenario.org_folder(&a);
    write_blob_file(&folder_a, &sha(1), 40 * DAY_MS);
    write_blob_file(&folder_a, &sha(3), 40 * DAY_MS);
    write_blob_file(&folder_a, &sha(9), 40 * DAY_MS);
    write_blob_file(&folder_a, &sha(5), HOUR_MS / 2);
    write_blob_file(&folder_a, "sobra.tmp", 40 * DAY_MS);
    write_blob_file(&folder_a, "notas.txt", 40 * DAY_MS);
    let folder_b = scenario.org_folder(&b);
    write_blob_file(&folder_b, &sha(4), 40 * DAY_MS);
    let database_b_before = snapshot(&c, &b, ORG_TABLES);
    let files_b_before = files_in(&folder_b);

    let report =
        run_org(&scenario.warehouse(), org_id(&a), &options_for(30, false)).expect("varredura");

    assert_eq!(report.deleted_files, 2);
    assert_eq!(report.orphan_files, vec![sha(1), "sobra.tmp".to_owned()]);
    let remaining: Vec<String> = files_in(&folder_a).into_keys().collect();
    assert_eq!(
        remaining,
        vec![sha(3), sha(5), sha(9), "notas.txt".to_owned()],
        "vivos, recente e não-blob ficam"
    );
    assert_eq!(
        files_in(&folder_b),
        files_b_before,
        "pasta da outra org intacta"
    );
    assert_eq!(
        snapshot(&c, &b, ORG_TABLES),
        database_b_before,
        "banco da outra org intacto"
    );
}

#[test]
fn isolation_between_orgs_in_a_global_sweep() {
    let scenario = Scenario::new();
    let c = scenario.connection();
    let ok = seed(&c, "ok", "{}", 40);
    let bad = seed(&c, "ruim", r#"{"retencao":{"dias":-5}}"#, 40);
    let before_bad = snapshot(&c, &bad, ORG_TABLES);

    let report = run_all(&scenario.warehouse(), &options_for(30, false)).expect("varredura geral");

    let ok_report = report.orgs.iter().find(|r| r.org_id == ok).expect("org ok");
    let bad_report = report
        .orgs
        .iter()
        .find(|r| r.org_id == bad)
        .expect("org ruim");
    assert_eq!(ok_report.situacao, Situacao::Swept);
    assert_eq!(database_of(ok_report).messages, 3);
    assert_eq!(bad_report.situacao, Situacao::Failed);
    assert!(
        bad_report
            .motivo
            .as_deref()
            .unwrap_or("")
            .contains("Nada foi apagado"),
        "{:?}",
        bad_report.motivo
    );
    assert_eq!(
        snapshot(&c, &bad, ORG_TABLES),
        before_bad,
        "org com config inválida não é tocada"
    );
    assert_eq!(count_rows(&c, "messages", &ok), 2, "a org boa foi varrida");
}

#[test]
fn unknown_org_id_deletes_nothing() {
    let scenario = Scenario::new();
    let c = scenario.connection();
    let org = seed(&c, "existe", "{}", 40);
    let before = snapshot(&c, &org, ORG_TABLES);

    let error = run_org(
        &scenario.warehouse(),
        OrgId::generate(),
        &options_for(30, false),
    )
    .expect_err("org inexistente tem de falhar");

    assert!(matches!(error, RetentionError::OrgNotFound(_)), "{error:?}");
    assert_eq!(snapshot(&c, &org, ORG_TABLES), before);
}

#[test]
fn pending_items_are_never_deleted() {
    let scenario = Scenario::new();
    let c = scenario.connection();
    let org = seed(&c, "pendentes", "{}", 400);
    let agent_id = insert_agent(&c, &org);
    let conversation_id = insert_conversation(&c, &org, &agent_id, NOW_MS - 400 * DAY_MS);
    let t_running = insert_turn(
        &c,
        &org,
        &conversation_id,
        &agent_id,
        "running",
        NOW_MS - 400 * DAY_MS,
        None,
    );
    insert_output(
        &c,
        &org,
        &conversation_id,
        &t_running,
        "sending",
        NOW_MS - 400 * DAY_MS,
        None,
    );
    insert_input(&c, &org, NOW_MS - 400 * DAY_MS, None);
    let t_failed = insert_turn(
        &c,
        &org,
        &conversation_id,
        &agent_id,
        "failed",
        NOW_MS - 400 * DAY_MS,
        Some(NOW_MS - 400 * DAY_MS),
    );
    insert_output(
        &c,
        &org,
        &conversation_id,
        &t_failed,
        "failed",
        NOW_MS - 400 * DAY_MS,
        None,
    );
    let not_sent = |c: &Connection| -> i64 {
        c.query_row(
            "SELECT COUNT(*) FROM outbox WHERE org_id = ?1 AND state <> 'sent'",
            params![org],
            |row| row.get(0),
        )
        .expect("saídas não enviadas")
    };
    let unprocessed = |c: &Connection| -> i64 {
        c.query_row(
            "SELECT COUNT(*) FROM inbox WHERE org_id = ?1 AND processed_at IS NULL",
            params![org],
            |row| row.get(0),
        )
        .expect("entradas não processadas")
    };
    let running = |c: &Connection| -> i64 {
        c.query_row(
            "SELECT COUNT(*) FROM turns WHERE org_id = ?1 AND state = 'running'",
            params![org],
            |row| row.get(0),
        )
        .expect("turnos rodando")
    };
    let before = (not_sent(&c), unprocessed(&c), running(&c));
    assert_eq!(before, (4, 2, 2), "cenário de partida");

    run_org(&scenario.warehouse(), org_id(&org), &options_for(1, false)).expect("varredura");

    let after = (not_sent(&c), unprocessed(&c), running(&c));
    assert_eq!(
        after, before,
        "saídas pendentes, entradas e turnos rodando continuam"
    );
}

#[test]
fn audit_matches_what_was_deleted() {
    let scenario = Scenario::new();
    let c = scenario.connection();
    let org = seed(&c, "auditada", "{}", 40);
    let folder = scenario.org_folder(&org);
    write_blob_file(&folder, &sha(1), 40 * DAY_MS);

    let report =
        run_org(&scenario.warehouse(), org_id(&org), &options_for(30, false)).expect("varredura");
    let database = database_of(&report);

    let audit = audit_entries(&c, &org);
    assert_eq!(audit.len(), 1, "uma linha por org varrida");
    let detail = &audit[0];
    assert_eq!(detail["simulacao"], false);
    assert_eq!(detail["mensagens"], database.messages);
    assert_eq!(detail["anexos"], database.anexos);
    assert_eq!(detail["turnos"], database.turns);
    assert_eq!(detail["inbox"], database.inbox);
    assert_eq!(detail["outbox"], database.outbox);
    assert_eq!(detail["conversas"], database.conversas);
    assert_eq!(detail["arquivos_apagados"], report.deleted_files);
    assert_eq!(detail["arquivos_sem_dono"], report.orphan_files.len());
}

const FLOW_TABLES: &[&str] = &[
    "fluxos",
    "fluxo_revisoes",
    "agente_fluxos",
    "fluxo_execucoes",
    "fluxo_execucao_etapas",
    "ficha_eventos",
    "fluxo_efeitos",
];

#[test]
fn case_execution_step_event_and_effect_survive_the_sweep() {
    let scenario = Scenario::new();
    let c = scenario.connection();
    let org_text = org(&c, "modulo-fluxos", "{}");
    let org = org_id(&org_text);
    let agent: AgentId = insert_agent(&c, &org_text).parse().expect("id do agente");

    let conversation = scenario
        .store
        .conversations()
        .create(
            org,
            NewConversation {
                agent_id: agent,
                integration_id: None,
                canal: "cli".to_owned(),
                peer_externo: Some("cliente-ficticio".to_owned()),
                target_department: None,
            },
        )
        .expect("conversa com Ficha");
    c.execute(
        "UPDATE conversations SET created_at = ?1, updated_at = ?1 WHERE org_id = ?2 AND id = ?3",
        params![NOW_MS - 400 * DAY_MS, org_text, conversation.id.to_string()],
    )
    .expect("envelhece a conversa");

    let flow = scenario
        .store
        .flows()
        .create(
            org,
            NewFlow {
                slug: "agendar_visita".to_owned(),
                name: "Agendar visita".to_owned(),
                description: String::new(),
                rascunho_fonte: "fluxo()".to_owned(),
            },
        )
        .expect("fluxo");
    let revision = scenario
        .store
        .revisions()
        .publish_revision(
            org,
            flow.id,
            RevisionPublication {
                expected: flow.version,
                fonte: "fluxo()".to_owned(),
                definicao_json: "{}".to_owned(),
                schema_versao: Some(1),
                resolucoes_json: "{}".to_owned(),
                ator_tipo: "cli".to_owned(),
                actor_id: None,
            },
        )
        .expect("revisão publicada");
    scenario
        .store
        .flows()
        .release(org, agent, flow.id)
        .expect("libera o fluxo ao agente");
    let case = scenario
        .store
        .cases()
        .by_conversation(org, conversation.id)
        .expect("consulta")
        .expect("a conversa tem Ficha");
    scenario
        .store
        .executions()
        .start_execution(
            org,
            ExecutionStart {
                case: case.id,
                flow: flow.id,
                revision: revision.id,
                actor: EventActor::Agent,
                ator_ref: Some("agente-ficticio"),
                jev_decision: None,
                initial_state: ExecutionState {
                    status: ExecutionStatus::Active,
                    no_atual: "coleta".to_owned(),
                    dados_json: "{}".to_owned(),
                    acting_actors: Vec::new(),
                    result: None,
                    reason: None,
                },
                inbound_effects: vec![TransitionEffect {
                    key: "entrada/0".to_owned(),
                    tipo: "mensagem_ao_cliente".to_owned(),
                    payload_json: "{\"texto\":\"olá\"}".to_owned(),
                }],
                chave_idempotencia: None,
            },
        )
        .expect("abre a execução");

    let before = snapshot(&c, &org_text, FLOW_TABLES);
    assert_eq!(before["fluxo_execucoes"].len(), 1);
    assert_eq!(before["fluxo_execucao_etapas"].len(), 1);
    assert_eq!(before["fluxo_efeitos"].len(), 1);
    assert!(
        before["ficha_eventos"].len() >= 2,
        "aberta + fluxo iniciado"
    );

    let simulation =
        run_org(&scenario.warehouse(), org, &options_for(30, true)).expect("simulação");
    assert_eq!(simulation.situacao, Situacao::Swept);
    assert_eq!(
        database_of(&simulation).conversas,
        1,
        "a conversa vazia sairia"
    );
    assert_eq!(
        snapshot(&c, &org_text, FLOW_TABLES),
        before,
        "a simulação não apaga nada do módulo"
    );
    assert_eq!(
        count_rows(&c, "conversations", &org_text),
        1,
        "a simulação não apaga a conversa"
    );

    let real = run_org(&scenario.warehouse(), org, &options_for(30, false)).expect("varredura");
    assert_eq!(database_of(&real).conversas, 1);
    assert_eq!(count_rows(&c, "conversations", &org_text), 0);
    assert_eq!(
        snapshot(&c, &org_text, FLOW_TABLES),
        before,
        "Ficha, execução, etapa, evento e efeito não podem perder linha"
    );

    assert_eq!(count_rows(&c, "fichas", &org_text), 1);
    let (situacao, conversation_of_case): (String, Option<String>) = c
        .query_row(
            "SELECT status, conversation_id FROM fichas WHERE org_id = ?1",
            params![org_text],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("Ficha depois da varredura");
    assert_eq!(situacao, "open", "a Ficha do atendimento não fecha sozinha");
    assert_eq!(
        conversation_of_case, None,
        "conversa apagada deixa a Ficha com o vínculo nulo"
    );
    assert_eq!(count_rows(&c, "fluxo_execucoes", &org_text), 1);
    assert_eq!(count_rows(&c, "fluxo_execucao_etapas", &org_text), 1);
    assert_eq!(count_rows(&c, "fluxo_efeitos", &org_text), 1);
    assert_eq!(count_rows(&c, "fluxo_revisoes", &org_text), 1);
    assert_eq!(count_rows(&c, "fluxos", &org_text), 1);
}
