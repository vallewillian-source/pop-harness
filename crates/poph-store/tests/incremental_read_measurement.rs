use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use poph_store::{Direction, NewAgent, NewMessage, NewOrg, Origin, Role, Store};
use poph_types::{ConversationId, MessageId, OrgId};
use rusqlite::{Connection, params};

const QUERY_MESSAGES: &str = "SELECT rowid, id, org_id, conversation_id, direction, role,
    origin, author_name, text, attachments_json, external_id, created_at
    FROM messages WHERE rowid > ?1 AND role IN ('user', 'assistant') ORDER BY rowid LIMIT ?2";

const QUERY_CONVERSATIONS: &str = "SELECT id, org_id, agent_id, integration_id, channel,
    external_peer, status, target_department_id, summary, version, created_at, updated_at,
    last_message_at FROM conversations WHERE (updated_at, id) > (?1, ?2)
    ORDER BY updated_at, id LIMIT ?3";

const INDEX_CONVERSATIONS: &str =
    "CREATE INDEX ix_conversations_atualizada ON conversations (updated_at, id)";

fn env_usize(nome: &str, default: usize) -> usize {
    std::env::var(nome)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn median_and_p95(mut times: Vec<Duration>) -> (Duration, Duration) {
    times.sort();
    let middle = times[times.len() / 2];
    let p95 = times[(times.len() * 95 / 100).min(times.len() - 1)];
    (middle, p95)
}

fn micros(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

fn prepare(
    folder: &PathBuf,
    conversations_n: usize,
    messages_n: usize,
) -> (Store, OrgId, Vec<ConversationId>) {
    let _ = fs::remove_dir_all(folder);
    fs::create_dir_all(folder).expect("pasta");
    let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db")).expect("abre");
    let org = store
        .orgs()
        .create(NewOrg {
            name: "Medição".to_owned(),
            slug: "medicao-mx05".to_owned(),
            config_json: String::new(),
        })
        .expect("org")
        .id;
    let agent = store
        .agents()
        .create(
            org,
            NewAgent {
                name: "Agente".to_owned(),
                slug: "agente-medicao".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("agente")
        .id;

    let path = store.harness_path().to_path_buf();
    let mut con = Connection::open(&path).expect("conexão de carga");
    let now: i64 = 1_800_000_000_000;
    let org_txt = org.to_string();
    let agent_txt = agent.to_string();
    let tx = con.transaction().expect("transação");
    let mut conversations = Vec::with_capacity(conversations_n);
    {
        let mut insere = tx
            .prepare(
                "INSERT INTO conversations (id, org_id, agent_id, channel, external_peer, status,
                    version, created_at, updated_at, last_message_at)
                 VALUES (?1, ?2, ?3, 'whatsapp', ?4, 'open', 1, ?5, ?5, ?5)",
            )
            .expect("prepara conversas");
        for i in 0..conversations_n {
            let id = ConversationId::generate();
            insere
                .execute(params![
                    id.to_string(),
                    org_txt,
                    agent_txt,
                    format!("peer-{i}"),
                    now + i as i64
                ])
                .expect("conversa");
            conversations.push(id);
        }
    }
    {
        let mut insere = tx
            .prepare(
                "INSERT INTO messages (id, org_id, conversation_id, direction, role, text,
                    attachments_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, '[]', ?7)",
            )
            .expect("prepara mensagens");
        for i in 0..messages_n {
            let conversation = conversations[i % conversations_n].to_string();
            let (direction, role) = match i % 10 {
                0..=3 => ("in", "user"),
                4..=7 => ("out", "assistant"),
                8 => ("out", "tool"),
                _ => ("out", "system"),
            };
            insere
                .execute(params![
                    MessageId::generate().to_string(),
                    org_txt,
                    conversation,
                    direction,
                    role,
                    "texto sintético da medição da leitura incremental",
                    now + i as i64
                ])
                .expect("mensagem");
        }
    }
    tx.commit().expect("commit da carga");
    (store, org, conversations)
}

fn prints_plan(con: &Connection, label: &str, sql: &str, parameters: &[&dyn rusqlite::ToSql]) {
    let mut statement = con
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("explain");
    let rows = statement
        .query_map(parameters, |row| row.get::<_, String>(3))
        .expect("plano");
    println!("plano · {label}:");
    for row in rows {
        println!("    {}", row.expect("linha do plano"));
    }
}

#[test]
#[ignore = "medição de minutos; rodar com --ignored --nocapture (de preferência com --release)"]
fn incremental_read_measurement_and_index_cost() {
    let messages_n = env_usize("POPH_MX05_MENSAGENS", 1_000_000);
    let conversations_n = env_usize("POPH_MX05_CONVERSAS", 5_000);
    let folder = std::env::temp_dir().join(format!("poph-mx05-medicao-{}", std::process::id()));

    let beginning = Instant::now();
    let (store, org, conversations) = prepare(&folder, conversations_n, messages_n);
    println!(
        "carga: {messages_n} mensagens em {conversations_n} conversas, preparadas em {:.1} s",
        beginning.elapsed().as_secs_f64()
    );

    let path = store.harness_path().to_path_buf();
    let con = Connection::open(&path).expect("conexão de leitura");
    let middle = (messages_n as i64) / 2;
    prints_plan(
        &con,
        "mensagens desde(rowid)",
        QUERY_MESSAGES,
        &[&middle, &200_i64],
    );
    prints_plan(
        &con,
        "conversas alteradas_desde",
        QUERY_CONVERSATIONS,
        &[&0_i64, &"", &200_i64],
    );

    let repetitions = 300;
    for (label, after) in [("início", 0_u64), ("meio", middle as u64)] {
        let mut times = Vec::with_capacity(repetitions);
        for _ in 0..repetitions {
            let t = Instant::now();
            let page = store.messages().since(after, 200).expect("desde");
            times.push(t.elapsed());
            assert!(page.len() <= 200);
        }
        let (median, p95) = median_and_p95(times);
        println!(
            "página de 200 mensagens desde o {label}: mediana {:.0} µs, p95 {:.0} µs",
            micros(median),
            micros(p95)
        );
    }
    let mut times = Vec::with_capacity(repetitions);
    for _ in 0..repetitions {
        let t = Instant::now();
        let page = store
            .conversations()
            .changed_since(Some((1_800_000_000_000, conversations[0])), 200)
            .expect("alteradas");
        times.push(t.elapsed());
        assert!(page.len() <= 200);
    }
    let (median, p95) = median_and_p95(times);
    println!(
        "página de 200 conversas alteradas_desde: mediana {:.0} µs, p95 {:.0} µs",
        micros(median),
        micros(p95)
    );

    let turns = 20_000_usize;
    let round = |label: &str| {
        let mut times = Vec::with_capacity(turns);
        let beginning = Instant::now();
        for i in 0..turns {
            let conversation = conversations[(i * 7919) % conversations_n];
            let t = Instant::now();
            store
                .messages()
                .write(
                    org,
                    NewMessage {
                        conversation_id: conversation,
                        direction: Direction::Inbound,
                        role: Role::User,
                        origin: Origin::Agente,
                        author_name: None,
                        texto: "turno de medição".to_owned(),
                        attachments_json: "[]".to_owned(),
                        external_id: None,
                        turn_id: None,
                    },
                )
                .expect("grava");
            store
                .conversations()
                .touch(org, conversation, 1_900_000_000_000 + i as i64)
                .expect("toca");
            times.push(t.elapsed());
        }
        let total = beginning.elapsed().as_secs_f64();
        let (median, p95) = median_and_p95(times);
        println!(
            "escrita do turno · {label}: {:.0} turnos/s; mediana {:.0} µs, p95 {:.0} µs",
            turns as f64 / total,
            micros(median),
            micros(p95)
        );
    };

    let ddl = Connection::open(&path).expect("conexão de DDL");
    for cycle in 1..=3 {
        ddl.execute_batch("DROP INDEX IF EXISTS ix_conversations_atualizada;")
            .expect("sem índice");
        round(&format!("sem o índice ({cycle}ª)"));
        ddl.execute_batch(INDEX_CONVERSATIONS).expect("com índice");
        round(&format!("com o índice ({cycle}ª)"));
    }

    drop(con);
    drop(ddl);
    drop(store);
    let _ = fs::remove_dir_all(&folder);
}
