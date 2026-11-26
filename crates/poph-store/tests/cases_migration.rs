use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::migrations::migrate;
use poph_store::{HARNESS_MIGRATIONS, Store, file_version, harness_target_version};
use poph_types::{ClosedBy, EventActor, RecordEventKind, RecordId, RecordStatus};
use rusqlite::Connection;

const MIGRATION: &str = "0012_fichas_e_eventos";

fn folder_new(label: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let folder =
        std::env::temp_dir().join(format!("poph-fichas-{label}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).expect("cria a pasta do teste");
    folder
}

fn index_of_migration() -> usize {
    HARNESS_MIGRATIONS
        .iter()
        .position(|migration| migration.name == MIGRATION)
        .expect("a migração das Fichas está registrada em MIGRACOES_HARNESS")
}

fn connection(path: &std::path::Path) -> Connection {
    let conn = Connection::open(path).expect("abre o banco para conferir");
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 2000;")
        .expect("pragmas");
    conn
}

fn seed_org_and_agent(conn: &Connection, label: &str) -> (String, String) {
    let org = uuid::Uuid::now_v7().to_string();
    let agent = uuid::Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO orgs (id, name, slug, created_at, updated_at) VALUES (?1, ?2, ?3, 1, 1)",
        rusqlite::params![org, format!("Org {label}"), format!("org-{label}")],
    )
    .expect("org");
    conn.execute(
        "INSERT INTO agents (id, org_id, name, slug, provider, model, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 'antigravity2api', 'gemini', 1, 1)",
        rusqlite::params![
            agent,
            org,
            format!("Agente {label}"),
            format!("agente-{label}")
        ],
    )
    .expect("agente");
    (org, agent)
}

fn seed_conversation(
    conn: &Connection,
    org: &str,
    agent: &str,
    status: &str,
    peer: Option<&str>,
    created_at: i64,
    updated_at: i64,
) -> String {
    let conversation = uuid::Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO conversations
            (id, org_id, agent_id, channel, external_peer, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'whatsapp', ?4, ?5, ?6, ?7)",
        rusqlite::params![
            conversation,
            org,
            agent,
            peer,
            status,
            created_at,
            updated_at
        ],
    )
    .expect("conversa");
    conversation
}

fn seed_message(conn: &Connection, org: &str, conversation: &str, created_at: i64) {
    conn.execute(
        "INSERT INTO messages (id, org_id, conversation_id, direction, role, text, created_at)
         VALUES (?1, ?2, ?3, 'in', 'user', 'oi', ?4)",
        rusqlite::params![
            uuid::Uuid::now_v7().to_string(),
            org,
            conversation,
            created_at
        ],
    )
    .expect("mensagem");
}

struct DatabaseOld {
    folder: PathBuf,
    conversations: Vec<(String, String, String)>,
}

impl DatabaseOld {
    fn build(label: &str) -> Self {
        let folder = folder_new(label);
        let harness = folder.join("harness.db");
        let before = index_of_migration();
        let mut conversations = Vec::new();
        {
            let conn = Connection::open(&harness).expect("abre o banco antigo");
            migrate(
                &conn,
                "harness.db",
                &harness,
                &HARNESS_MIGRATIONS[..before],
                1,
            )
            .expect("migra até a versão anterior");
            conn.execute_batch("PRAGMA foreign_keys = ON;")
                .expect("pragma");

            let (org_a, agent_a) = seed_org_and_agent(&conn, "a");
            let (org_b, agent_b) = seed_org_and_agent(&conn, "b");

            let opened = seed_conversation(
                &conn,
                &org_a,
                &agent_a,
                "open",
                Some("5521900000001"),
                1_000,
                1_500,
            );
            seed_message(&conn, &org_a, &opened, 1_100);
            seed_message(&conn, &org_a, &opened, 1_500);

            let transferred = seed_conversation(
                &conn,
                &org_a,
                &agent_a,
                "transferred",
                Some("5521900000002"),
                2_000,
                2_200,
            );

            let closed = seed_conversation(
                &conn,
                &org_a,
                &agent_a,
                "closed",
                Some("5521900000003"),
                3_000,
                3_900,
            );
            seed_message(&conn, &org_a, &closed, 3_100);

            let closed_empty =
                seed_conversation(&conn, &org_a, &agent_a, "closed", None, 4_000, 4_050);

            let of_other_org = seed_conversation(
                &conn,
                &org_b,
                &agent_b,
                "open",
                Some("5521900000004"),
                5_000,
                5_000,
            );

            for (scenario, id, org) in [
                ("aberta", &opened, &org_a),
                ("transferida", &transferred, &org_a),
                ("fechada", &closed, &org_a),
                ("fechada-sem-mensagem", &closed_empty, &org_a),
                ("outra-org", &of_other_org, &org_b),
            ] {
                conversations.push((scenario.to_owned(), id.clone(), org.clone()));
            }
        }
        Self {
            folder,
            conversations,
        }
    }

    fn harness(&self) -> PathBuf {
        self.folder.join("harness.db")
    }

    fn id(&self, scenario: &str) -> &str {
        &self
            .conversations
            .iter()
            .find(|(nome, _, _)| nome == scenario)
            .expect("cenário semeado")
            .1
    }
}

impl Drop for DatabaseOld {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.folder);
    }
}

struct CaseStored {
    id: String,
    status: String,
    aberta_em: i64,
    fechada_em: Option<i64>,
    fechada_por: Option<String>,
    fechada_motivo: Option<String>,
    agent_id: String,
    canal: String,
    peer_externo: Option<String>,
    version: i64,
}

fn case_of_conversation(conn: &Connection, conversation: &str) -> CaseStored {
    conn.query_row(
        "SELECT id, status, aberta_em, fechada_em, fechada_por, fechada_motivo, agent_id, canal,
                peer_externo, version
           FROM fichas WHERE conversation_id = ?1",
        [conversation],
        |row| {
            Ok(CaseStored {
                id: row.get(0)?,
                status: row.get(1)?,
                aberta_em: row.get(2)?,
                fechada_em: row.get(3)?,
                fechada_por: row.get(4)?,
                fechada_motivo: row.get(5)?,
                agent_id: row.get(6)?,
                canal: row.get(7)?,
                peer_externo: row.get(8)?,
                version: row.get(9)?,
            })
        },
    )
    .expect("a conversa tem Ficha")
}

fn count_rows(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).expect("conta")
}

#[test]
fn migration_creates_two_tables_with_combined_indexes() {
    let folder = folder_new("esquema");
    let harness = folder.join("harness.db");
    let store = Store::open_at(&harness, &folder.join("rag.db")).expect("abre e migra");

    for table in ["fichas", "ficha_eventos"] {
        assert!(
            store.tables().expect("tabelas").iter().any(|t| t == table),
            "tabela {table} não foi criada"
        );
    }
    assert_eq!(
        file_version(&harness).expect("versão"),
        Some(harness_target_version())
    );

    let columns = store.columns("fichas").expect("colunas de fichas");
    for expected in [
        "id",
        "org_id",
        "conversation_id",
        "agent_id",
        "canal",
        "peer_externo",
        "status",
        "aberta_em",
        "fechada_em",
        "fechada_por",
        "fechada_motivo",
        "version",
    ] {
        assert!(
            columns.iter().any(|column| column == expected),
            "fichas sem a coluna {expected}; tem {columns:?}"
        );
    }
    let columns = store.columns("ficha_eventos").expect("colunas de eventos");
    for expected in [
        "id",
        "org_id",
        "ficha_id",
        "execucao_id",
        "seq",
        "tipo",
        "ator_tipo",
        "ator_ref",
        "payload_json",
        "chave_idempotencia",
        "created_at",
    ] {
        assert!(
            columns.iter().any(|column| column == expected),
            "ficha_eventos sem a coluna {expected}; tem {columns:?}"
        );
    }

    let conn = connection(&harness);
    for index in [
        "ux_fichas_conversa",
        "ix_fichas_org_status",
        "ux_fichas_org_id",
        "ux_ficha_eventos_seq",
        "ux_ficha_eventos_idem",
    ] {
        let exists = count_rows(
            &conn,
            &format!(
                "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = '{index}'"
            ),
        );
        assert_eq!(exists, 1, "índice {index} não foi criado");
    }
}

#[test]
fn backfill_creates_exactly_one_case_per_conversation() {
    let database = DatabaseOld::build("backfill");
    let harness = database.harness();
    let store = Store::open_at(&harness, &database.folder.join("rag.db")).expect("abre e migra");
    assert_eq!(
        file_version(&harness).expect("versão"),
        Some(harness_target_version())
    );
    let conn = connection(&harness);

    assert_eq!(
        count_rows(&conn, "SELECT count(*) FROM fichas"),
        count_rows(&conn, "SELECT count(*) FROM conversations"),
        "o backfill tem que criar uma Ficha para cada conversa"
    );
    assert_eq!(
        count_rows(
            &conn,
            "SELECT count(*) FROM conversations c
              WHERE NOT EXISTS (SELECT 1 FROM fichas f WHERE f.conversation_id = c.id)"
        ),
        0,
        "sobrou conversa sem Ficha"
    );
    assert_eq!(
        count_rows(
            &conn,
            "SELECT count(*) FROM (SELECT conversation_id FROM fichas
                WHERE conversation_id IS NOT NULL
                GROUP BY conversation_id HAVING count(*) > 1)"
        ),
        0,
        "alguma conversa ficou com duas Fichas"
    );

    for (scenario, created_at) in [
        ("aberta", 1_000),
        ("transferida", 2_000),
        ("outra-org", 5_000),
    ] {
        let case = case_of_conversation(&conn, database.id(scenario));
        assert_eq!(
            case.status.parse::<RecordStatus>().expect("status válido"),
            RecordStatus::Open,
            "{scenario} deveria ficar aberta"
        );
        assert_eq!(
            case.aberta_em, created_at,
            "{scenario}: aberta_em é a criação da conversa"
        );
        assert_eq!(case.fechada_em, None, "{scenario} não pode ter fechamento");
        assert_eq!(case.fechada_por, None);
        assert_eq!(case.canal, "whatsapp");
        assert_eq!(case.version, 1);
        assert!(
            !case.agent_id.is_empty(),
            "{scenario} sem o agente do atendimento"
        );
    }

    for scenario in ["fechada", "fechada-sem-mensagem"] {
        let case = case_of_conversation(&conn, database.id(scenario));
        assert_eq!(
            case.status.parse::<RecordStatus>().expect("status válido"),
            RecordStatus::Closed,
            "{scenario} deveria ficar fechada"
        );
        assert_eq!(
            case.fechada_por.as_deref(),
            Some(ClosedBy::Migration.as_text())
        );
        assert_eq!(
            case.fechada_em,
            Some(if scenario == "fechada" { 3_900 } else { 4_050 }),
            "fechada_em é o último instante conhecido da conversa"
        );
        assert!(
            case.fechada_motivo
                .as_deref()
                .is_some_and(|motivo| !motivo.is_empty()),
            "{scenario} fechada sem motivo"
        );
    }

    assert_eq!(
        case_of_conversation(&conn, database.id("aberta"))
            .peer_externo
            .as_deref(),
        Some("5521900000001")
    );
    assert_eq!(
        case_of_conversation(&conn, database.id("fechada-sem-mensagem")).peer_externo,
        None
    );

    for (_, conversation, _) in &database.conversations {
        let case = case_of_conversation(&conn, conversation);
        case.id
            .parse::<RecordId>()
            .unwrap_or_else(|error| panic!("id de Ficha inválido no backfill: {error}"));
    }

    assert_eq!(
        count_rows(&conn, "SELECT count(*) FROM messages"),
        3,
        "o backfill não pode criar nem apagar mensagem"
    );

    drop(store);
}

#[test]
fn running_migration_again_does_nothing() {
    let database = DatabaseOld::build("idempotente");
    let store = Store::open_at(&database.harness(), &database.folder.join("rag.db")).expect("abre");
    let before = {
        let conn = connection(&database.harness());
        count_rows(&conn, "SELECT count(*) FROM fichas")
    };

    let reports = store.migrate().expect("migra de novo");
    assert!(reports.already_up_to_date(), "{reports:?}");

    let conn = connection(&database.harness());
    assert_eq!(count_rows(&conn, "SELECT count(*) FROM fichas"), before);

    let sql = HARNESS_MIGRATIONS[index_of_migration()].sql;
    let beginning = sql
        .find("INSERT INTO fichas")
        .expect("o backfill está na migração");
    conn.execute_batch(&sql[beginning..])
        .expect("repetir o backfill não pode falhar");
    assert_eq!(count_rows(&conn, "SELECT count(*) FROM fichas"), before);
}

#[test]
fn database_refuses_two_cases_for_same_conversation() {
    let database = DatabaseOld::build("unica");
    let _store =
        Store::open_at(&database.harness(), &database.folder.join("rag.db")).expect("abre");
    let conn = connection(&database.harness());
    let conversation = database.id("aberta");
    let org: String = conn
        .query_row(
            "SELECT org_id FROM conversations WHERE id = ?1",
            [conversation],
            |row| row.get(0),
        )
        .expect("org da conversa");

    let error = conn
        .execute(
            "INSERT INTO fichas (id, org_id, conversation_id, agent_id, canal, status, aberta_em)
             VALUES (?1, ?2, ?3, 'ag', 'whatsapp', 'open', 1)",
            rusqlite::params![uuid::Uuid::now_v7().to_string(), org, conversation],
        )
        .expect_err("a segunda Ficha da mesma conversa tem que ser recusada");
    assert!(
        error.to_string().contains("UNIQUE"),
        "erro inesperado: {error}"
    );

    conn.execute("DELETE FROM conversations WHERE id = ?1", [conversation])
        .expect("apaga a conversa");
    let without_owner: i64 = conn
        .query_row(
            "SELECT count(*) FROM fichas WHERE conversation_id IS NULL",
            [],
            |row| row.get(0),
        )
        .expect("conta");
    assert_eq!(without_owner, 1, "a Ficha sobrevive à conversa apagada");
    conn.execute(
        "INSERT INTO fichas (id, org_id, conversation_id, agent_id, canal, status, aberta_em)
         VALUES (?1, ?2, NULL, 'ag', 'whatsapp', 'open', 2)",
        rusqlite::params![uuid::Uuid::now_v7().to_string(), org],
    )
    .expect("duas Fichas sem conversa não colidem");
}

#[test]
fn database_refuses_case_with_incoherent_status_and_closing() {
    let database = DatabaseOld::build("coerencia");
    let _store =
        Store::open_at(&database.harness(), &database.folder.join("rag.db")).expect("abre");
    let conn = connection(&database.harness());
    let org: String = conn
        .query_row("SELECT org_id FROM conversations LIMIT 1", [], |row| {
            row.get(0)
        })
        .expect("org da conversa semeada");

    let inserir = |status: &str, fechada_em: Option<i64>, fechada_por: Option<&str>| {
        conn.execute(
            "INSERT INTO fichas (id, org_id, conversation_id, agent_id, canal, status, aberta_em,
                                 fechada_em, fechada_por)
             VALUES (?1, ?2, NULL, 'ag', 'whatsapp', ?3, 1, ?4, ?5)",
            rusqlite::params![
                uuid::Uuid::now_v7().to_string(),
                org,
                status,
                fechada_em,
                fechada_por
            ],
        )
    };

    assert!(
        inserir("closed", None, Some("agente")).is_err(),
        "fechada sem instante de fechamento"
    );
    assert!(
        inserir("closed", Some(10), None).is_err(),
        "fechada sem autor do fechamento"
    );
    assert!(
        inserir("open", Some(10), Some("agente")).is_err(),
        "aberta com fechamento"
    );
    assert!(
        inserir("aberto", None, None).is_err(),
        "status fora da lista"
    );
    assert!(
        inserir("closed", Some(10), Some("cliente_qualquer")).is_err(),
        "autor fora da lista"
    );
    assert!(
        inserir("closed", Some(10), Some("migracao")).is_ok(),
        "o par coerente é aceito"
    );
}

#[test]
fn database_refuses_event_with_text_outside_list_and_across_orgs() {
    let database = DatabaseOld::build("contexto");
    let _store =
        Store::open_at(&database.harness(), &database.folder.join("rag.db")).expect("abre");
    let conn = connection(&database.harness());
    let org: String = conn
        .query_row("SELECT org_id FROM conversations LIMIT 1", [], |row| {
            row.get(0)
        })
        .expect("org da conversa semeada");

    let evento = |case: &str, seq: i64, tipo: &str, actor: &str, chave: Option<&str>| {
        conn.execute(
            "INSERT INTO ficha_eventos
                (id, org_id, ficha_id, seq, tipo, ator_tipo, chave_idempotencia, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1)",
            rusqlite::params![
                uuid::Uuid::now_v7().to_string(),
                org,
                case,
                seq,
                tipo,
                actor,
                chave
            ],
        )
    };

    let case = database.id("aberta").to_owned();
    let case = case_of_conversation(&conn, &case).id;

    let mut seq = 1;
    for tipo in RecordEventKind::ALL {
        for actor in EventActor::ALL {
            evento(&case, seq, tipo.as_text(), actor.as_text(), None)
                .unwrap_or_else(|error| panic!("{tipo}/{actor} recusado pelo banco: {error}"));
            seq += 1;
        }
    }
    assert!(
        evento(&case, seq, "tipo_inventado", "sistema", None).is_err(),
        "tipo fora da lista tem que ser recusado"
    );
    assert!(
        evento(&case, seq + 1, "ficha_aberta", "operador", None).is_err(),
        "ator fora da lista tem que ser recusado"
    );
    assert!(
        evento(&case, seq + 2, "ficha_aberta", "cliente", None).is_err(),
        "o cliente não assina evento da Ficha"
    );

    let repeated_value = seq - 1;
    assert!(
        evento(&case, repeated_value, "ficha_aberta", "sistema", None).is_err(),
        "seq repetida na mesma Ficha tem que ser recusada"
    );

    assert!(evento(&case, seq + 10, "ficha_aberta", "sistema", Some("k1")).is_ok());
    assert!(
        evento(&case, seq + 11, "ficha_aberta", "sistema", Some("k1")).is_err(),
        "chave de idempotência repetida tem que ser recusada"
    );
    assert!(evento(&case, seq + 12, "ficha_aberta", "sistema", None).is_ok());
    assert!(evento(&case, seq + 13, "ficha_aberta", "sistema", None).is_ok());

    let other_org: String = conn
        .query_row(
            "SELECT id FROM orgs WHERE id <> ?1 LIMIT 1",
            [&org],
            |row| row.get(0),
        )
        .expect("segunda org");
    let error = conn
        .execute(
            "INSERT INTO ficha_eventos (id, org_id, ficha_id, seq, tipo, ator_tipo, created_at)
             VALUES (?1, ?2, ?3, 9000, 'ficha_aberta', 'sistema', 1)",
            rusqlite::params![uuid::Uuid::now_v7().to_string(), other_org, case],
        )
        .expect_err("evento cruzando orgs tem que ser recusado pelo banco");
    assert!(
        error.to_string().contains("FOREIGN KEY"),
        "erro inesperado: {error}"
    );
}

#[test]
fn events_survive_case_and_disappear_with_it() {
    let database = DatabaseOld::build("cascata");
    let _store =
        Store::open_at(&database.harness(), &database.folder.join("rag.db")).expect("abre");
    let conn = connection(&database.harness());
    let case = case_of_conversation(&conn, database.id("aberta"));
    conn.execute(
        "INSERT INTO ficha_eventos (id, org_id, ficha_id, seq, tipo, ator_tipo, created_at)
         SELECT ?1, org_id, id, 1, 'ficha_aberta', 'sistema', 1 FROM fichas WHERE id = ?2",
        rusqlite::params![uuid::Uuid::now_v7().to_string(), case.id],
    )
    .expect("evento");
    conn.execute("DELETE FROM fichas WHERE id = ?1", [&case.id])
        .expect("apaga a Ficha");
    assert_eq!(
        count_rows(&conn, "SELECT count(*) FROM ficha_eventos"),
        0,
        "evento órfão ficou no banco"
    );
}
