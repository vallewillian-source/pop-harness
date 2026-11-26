use std::collections::BTreeSet;

use poph_types::OrgId;
use rusqlite::{Connection, params};

use crate::Store;
use crate::error::{Resultado, StoreError};

const SAVEPOINT: &str = "poph_retencao";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanRetention {
    pub messages: u64,
    pub attachments: u64,
    pub turns: u64,
    pub inbox: u64,
    pub outbox: u64,
    pub conversations: u64,
    pub conversation_ids: Vec<String>,
    pub hashes_referenciados: Vec<String>,
}

pub struct Retencao<'a>(pub(crate) &'a Store);

impl Retencao<'_> {
    pub fn scan_table(
        &self,
        org: OrgId,
        cutoff_ms: i64,
        simulate: bool,
    ) -> Resultado<ScanRetention> {
        let texto = org.to_string();
        self.0.write(move |conn| {
            conn.execute_batch(&format!("SAVEPOINT {SAVEPOINT}"))?;
            let resultado = scan_in_transaction(conn, &texto, cutoff_ms);
            let closing = if resultado.is_ok() && !simulate {
                format!("RELEASE {SAVEPOINT}")
            } else {
                format!("ROLLBACK TO {SAVEPOINT}; RELEASE {SAVEPOINT}")
            };
            conn.execute_batch(&closing).map_err(StoreError::from)?;
            resultado
        })
    }
}

fn scan_in_transaction(conn: &Connection, org: &str, cutoff: i64) -> Resultado<ScanRetention> {
    let mut scan = ScanRetention::default();
    let mut conversations: BTreeSet<String> = BTreeSet::new();

    {
        let mut statement = conn.prepare(
            "SELECT DISTINCT conversation_id FROM messages
             WHERE org_id = ?1 AND created_at < ?2",
        )?;
        let rows = statement.query_map(params![org, cutoff], |row| row.get::<_, String>(0))?;
        for row in rows {
            conversations.insert(row?);
        }
    }

    scan.attachments = conn.execute(
        "DELETE FROM attachments
          WHERE org_id = ?1
            AND message_id IN (SELECT id FROM messages WHERE org_id = ?1 AND created_at < ?2)",
        params![org, cutoff],
    )? as u64;
    scan.messages = conn.execute(
        "DELETE FROM messages WHERE org_id = ?1 AND created_at < ?2",
        params![org, cutoff],
    )? as u64;

    scan.outbox = conn.execute(
        "DELETE FROM outbox
          WHERE org_id = ?1 AND state = 'sent' AND COALESCE(sent_at, created_at) < ?2",
        params![org, cutoff],
    )? as u64;

    scan.inbox = conn.execute(
        "DELETE FROM inbox
          WHERE org_id = ?1 AND processed_at IS NOT NULL AND processed_at < ?2",
        params![org, cutoff],
    )? as u64;

    scan.turns = conn.execute(
        "DELETE FROM turns
          WHERE org_id = ?1
            AND state <> 'running'
            AND COALESCE(finished_at, started_at) < ?2
            AND NOT EXISTS (SELECT 1 FROM outbox o WHERE o.turn_id = turns.id)
            AND NOT EXISTS (SELECT 1 FROM messages m WHERE m.turn_id = turns.id)",
        params![org, cutoff],
    )? as u64;

    const EMPTY: &str = "NOT EXISTS (SELECT 1 FROM messages m
                           WHERE m.org_id = conversations.org_id AND m.conversation_id = conversations.id)
        AND NOT EXISTS (SELECT 1 FROM turns t
                           WHERE t.org_id = conversations.org_id AND t.conversation_id = conversations.id)
        AND NOT EXISTS (SELECT 1 FROM outbox o
                           WHERE o.org_id = conversations.org_id AND o.conversation_id = conversations.id)
        AND NOT EXISTS (SELECT 1 FROM carts k
                           WHERE k.org_id = conversations.org_id AND k.conversation_id = conversations.id)";
    {
        let mut statement = conn.prepare(&format!(
            "SELECT id FROM conversations
              WHERE org_id = ?1 AND created_at < ?2 AND {EMPTY}"
        ))?;
        let rows = statement.query_map(params![org, cutoff], |row| row.get::<_, String>(0))?;
        for row in rows {
            conversations.insert(row?);
        }
    }
    scan.conversations = conn.execute(
        &format!(
            "DELETE FROM conversations
              WHERE org_id = ?1 AND created_at < ?2 AND {EMPTY}"
        ),
        params![org, cutoff],
    )? as u64;

    {
        let mut statement = conn.prepare(
            "SELECT sha256 FROM attachments WHERE org_id = ?1
             UNION
             SELECT sha256 FROM file_library WHERE org_id = ?1
             ORDER BY 1",
        )?;
        let rows = statement.query_map(params![org], |row| row.get::<_, String>(0))?;
        for row in rows {
            scan.hashes_referenciados.push(row?);
        }
    }

    scan.conversation_ids = conversations.into_iter().collect();
    Ok(scan)
}
