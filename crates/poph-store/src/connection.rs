use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::error::{Resultado, StoreError};

pub const MAX_WAIT_MS: u64 = 2_000;

pub(crate) fn open_for_write(path: &Path) -> Resultado<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| StoreError::OpenFailed {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;
    apply_pragmas(&conn)?;
    let mode: String = conn
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .map_err(|error| StoreError::OpenFailed {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StoreError::OpenFailed {
            path: path.display().to_string(),
            reason: format!("não consegui ativar WAL (modo atual: {mode})"),
        });
    }
    Ok(conn)
}

pub(crate) fn open_for_read(path: &Path) -> Resultado<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| StoreError::OpenFailed {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;
    apply_pragmas(&conn)?;
    Ok(conn)
}

fn apply_pragmas(conn: &Connection) -> Resultado<()> {
    conn.busy_timeout(Duration::from_millis(MAX_WAIT_MS))?;
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
    Ok(())
}
