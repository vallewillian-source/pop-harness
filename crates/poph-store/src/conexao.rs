//! Abertura de conexões e pragmas do SQLite.
//!
//! Um escritor dedicado (`journal_mode=WAL`, `synchronous=NORMAL`) e leitores em paralelo.
//! `foreign_keys=ON` em toda conexão e `busy_timeout` curto (2 s): com um escritor só, espera
//! longa significa defeito, não carga.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::erro::{ErroStore, Resultado};

/// Espera máxima por um bloqueio, em milissegundos.
pub const ESPERA_MAXIMA_MS: u64 = 2_000;

/// Abre uma conexão de escrita (cria o arquivo se não existir) e aplica os pragmas.
pub(crate) fn abrir_escrita(caminho: &Path) -> Resultado<Connection> {
    let conn = Connection::open_with_flags(
        caminho,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|erro| ErroStore::Abertura {
        caminho: caminho.display().to_string(),
        motivo: erro.to_string(),
    })?;
    aplica_pragmas(&conn)?;
    // WAL é persistente no arquivo; devolve o modo efetivo, que precisa ser `wal`.
    let modo: String = conn
        .query_row("PRAGMA journal_mode = WAL", [], |linha| linha.get(0))
        .map_err(|erro| ErroStore::Abertura {
            caminho: caminho.display().to_string(),
            motivo: erro.to_string(),
        })?;
    if !modo.eq_ignore_ascii_case("wal") {
        return Err(ErroStore::Abertura {
            caminho: caminho.display().to_string(),
            motivo: format!("não consegui ativar WAL (modo atual: {modo})"),
        });
    }
    Ok(conn)
}

/// Abre uma conexão só de leitura (o arquivo precisa existir).
pub(crate) fn abrir_leitura(caminho: &Path) -> Resultado<Connection> {
    let conn = Connection::open_with_flags(
        caminho,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|erro| ErroStore::Abertura {
        caminho: caminho.display().to_string(),
        motivo: erro.to_string(),
    })?;
    aplica_pragmas(&conn)?;
    Ok(conn)
}

fn aplica_pragmas(conn: &Connection) -> Resultado<()> {
    conn.busy_timeout(Duration::from_millis(ESPERA_MAXIMA_MS))?;
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
    Ok(())
}
