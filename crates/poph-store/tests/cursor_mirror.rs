use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::Store;
use poph_store::migrations::{HARNESS_MIGRATIONS, migrate};
use poph_types::{MessageId, OrgId};

fn folder(label: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let folder = std::env::temp_dir().join(format!("poph-mx14-{label}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).expect("cria a pasta");
    folder
}

#[test]
fn migration_0007_keeps_existing_cursor_and_puts_empty_id() {
    let folder = folder("migracao");
    let path = folder.join("harness.db");
    let conn = rusqlite::Connection::open(&path).expect("abre o banco");

    migrate(&conn, "harness.db", &path, &HARNESS_MIGRATIONS[..6], 1).expect("migra até a 6");
    conn.execute_batch("PRAGMA foreign_keys = OFF;")
        .expect("desliga");
    conn.execute(
        "INSERT INTO matrix_estado (org_id, messages_cursor, updated_at)
         VALUES ('0190a000-0000-7000-8000-00000000a001', 42, 10)",
        [],
    )
    .expect("grava cursor no formato anterior");

    let report = migrate(&conn, "harness.db", &path, HARNESS_MIGRATIONS, 2).expect("até a 7");
    assert_eq!(report.version_before, 6);
    assert_eq!(report.version_after, poph_store::harness_target_version());

    let (cursor, id): (i64, String) = conn
        .query_row(
            "SELECT messages_cursor, messages_cursor_id FROM matrix_estado",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("lê o cursor");
    assert_eq!(cursor, 42, "o cursor anterior não muda");
    assert_eq!(id, "", "sem id guardado ainda");
    let integridade: String = conn
        .query_row("PRAGMA integrity_check", [], |l| l.get(0))
        .expect("integridade");
    assert_eq!(integridade, "ok");
    drop(conn);
    let _ = fs::remove_dir_all(&folder);
}

#[test]
fn cursor_with_id_only_moves_forward_and_rewinds_only_at_safe_point() {
    let folder = folder("cursor");
    let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db")).expect("abre");
    let org = store
        .orgs()
        .create(poph_store::NewOrg {
            name: "Org".to_owned(),
            slug: "org-cursor".to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id;
    let id_a = MessageId::generate().to_string();
    let id_b = MessageId::generate().to_string();

    let first_value = store
        .matrix()
        .advance_messages_with_id(org, 10, &id_a)
        .expect("avança");
    assert_eq!(first_value.messages, 10);
    assert_eq!(first_value.message_ids, id_a);

    let stopped = store
        .matrix()
        .advance_messages_with_id(org, 5, &id_b)
        .expect("tenta recuar");
    assert_eq!(stopped.messages, 10);
    assert_eq!(stopped.message_ids, id_a);

    let recuado = store
        .matrix()
        .rewind_cursor_messages(org, 5, &id_b)
        .expect("recua");
    assert_eq!(recuado.messages, 5);
    assert_eq!(recuado.message_ids, id_b);

    let read_one = store.matrix().read_cursors(org).expect("lê");
    assert_eq!(read_one.messages, 5);
    assert_eq!(read_one.message_ids, id_b);
    drop(store);
    let _ = fs::remove_dir_all(&folder);
}

#[test]
fn last_rowid_and_had_messages_before_in_empty_database() {
    let folder = folder("vazio");
    let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db")).expect("abre");
    assert!(store.messages().last_rowid().expect("ultima").is_none());
    let org = OrgId::generate();
    let conversation = poph_types::ConversationId::generate();
    assert!(
        !store
            .messages()
            .had_messages_before(org, conversation, i64::MAX)
            .expect("teve")
    );
    drop(store);
    let _ = fs::remove_dir_all(&folder);
}
