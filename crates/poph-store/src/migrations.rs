use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{Resultado, StoreError};

#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

pub const HARNESS_MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "0001_esquema_inicial",
        sql: include_str!("../migrations/harness/0001_esquema_inicial.sql"),
    },
    Migration {
        version: 2,
        name: "0002_entrada_proxima_tentativa",
        sql: include_str!("../migrations/harness/0002_entrada_proxima_tentativa.sql"),
    },
    Migration {
        version: 3,
        name: "0003_origem_autor_mensagens",
        sql: include_str!("../migrations/harness/0003_origem_autor_mensagens.sql"),
    },
    Migration {
        version: 4,
        name: "0004_audit_log_matrix_user",
        sql: include_str!("../migrations/harness/0004_audit_log_matrix_user.sql"),
    },
    Migration {
        version: 5,
        name: "0005_tabelas_matrix",
        sql: include_str!("../migrations/harness/0005_tabelas_matrix.sql"),
    },
    Migration {
        version: 6,
        name: "0006_indice_conversations_atualizada",
        sql: include_str!("../migrations/harness/0006_indice_conversations_atualizada.sql"),
    },
    Migration {
        version: 7,
        name: "0007_matrix_cursor_mensagem_id",
        sql: include_str!("../migrations/harness/0007_matrix_cursor_mensagem_id.sql"),
    },
    Migration {
        version: 8,
        name: "0008_conversa_nome_cliente",
        sql: include_str!("../migrations/harness/0008_conversa_nome_cliente.sql"),
    },
    Migration {
        version: 9,
        name: "0009_matrix_avisos_falha",
        sql: include_str!("../migrations/harness/0009_matrix_avisos_falha.sql"),
    },
    Migration {
        version: 10,
        name: "0010_matrix_rooms_nome_cliente_espelhado",
        sql: include_str!("../migrations/harness/0010_matrix_rooms_nome_cliente_espelhado.sql"),
    },
    Migration {
        version: 11,
        name: "0011_outbox_id_externo",
        sql: include_str!("../migrations/harness/0011_outbox_id_externo.sql"),
    },
    Migration {
        version: 12,
        name: "0012_fichas_e_eventos",
        sql: include_str!("../migrations/harness/0012_fichas_e_eventos.sql"),
    },
    Migration {
        version: 13,
        name: "0013_fluxos_revisoes_execucoes_e_efeitos",
        sql: include_str!("../migrations/harness/0013_fluxos_revisoes_execucoes_e_efeitos.sql"),
    },
    Migration {
        version: 15,
        name: "0015_conversas_por_cliente",
        sql: include_str!("../migrations/harness/0015_conversas_por_cliente.sql"),
    },
    Migration {
        version: 16,
        name: "0016_matrix_midia_expiracao",
        sql: include_str!("../migrations/harness/0016_matrix_midia_expiracao.sql"),
    },
    Migration {
        version: 17,
        name: "0017_analytics_operational_summary",
        sql: include_str!("../migrations/harness/0017_analytics_operational_summary.sql"),
    },
    Migration {
        version: 18,
        name: "0018_usage_tokens",
        sql: include_str!("../migrations/harness/0018_usage_tokens.sql"),
    },
    Migration {
        version: 19,
        name: "0019_site_conversations",
        sql: include_str!("../migrations/harness/0019_site_conversations.sql"),
    },
    Migration {
        version: 20,
        name: "0020_flow_http_effects",
        sql: include_str!("../migrations/harness/0020_flow_http_effects.sql"),
    },
    Migration {
        version: 21,
        name: "0021_adapter_conversations",
        sql: include_str!("../migrations/harness/0021_adapter_conversations.sql"),
    },
    Migration {
        version: 22,
        name: "0022_adapter_turns",
        sql: include_str!("../migrations/harness/0022_adapter_turns.sql"),
    },
    Migration {
        version: 23,
        name: "0023_adapter_events",
        sql: include_str!("../migrations/harness/0023_adapter_events.sql"),
    },
];

pub const RAG_MIGRATIONS: &[Migration] = &[];

pub fn harness_target_version() -> u32 {
    HARNESS_MIGRATIONS
        .last()
        .map(|migration| migration.version)
        .unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedMigration {
    pub version: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    pub database: &'static str,
    pub path: PathBuf,
    pub version_before: u32,
    pub version_after: u32,
    pub applied: Vec<AppliedMigration>,
}

impl MigrationReport {
    pub fn already_up_to_date(&self) -> bool {
        self.applied.is_empty()
    }

    pub fn empty_database(&self) -> bool {
        self.version_before == 0 && !self.applied.is_empty()
    }

    pub fn summary(&self) -> String {
        if self.already_up_to_date() {
            format!(
                "{}: já estava na versão {} ({})",
                self.database,
                self.version_after,
                self.path.display()
            )
        } else {
            let names = self
                .applied
                .iter()
                .map(|m| format!("{} ({})", m.version, m.name))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{}: versão {} → {}; aplicadas: {} ({})",
                self.database,
                self.version_before,
                self.version_after,
                names,
                self.path.display()
            )
        }
    }
}

const BOOTSTRAP: &str = "CREATE TABLE IF NOT EXISTS schema_migrations (
    version    INTEGER PRIMARY KEY NOT NULL,
    name       TEXT    NOT NULL,
    applied_at INTEGER NOT NULL
) STRICT;";

pub fn migrate(
    conn: &Connection,
    database: &'static str,
    path: &Path,
    migrations: &[Migration],
    now: i64,
) -> Resultado<MigrationReport> {
    conn.execute_batch(BOOTSTRAP)?;
    let version_before = current_version(conn)?;
    let mut applied = Vec::new();

    for migration in migrations.iter().filter(|m| m.version > version_before) {
        conn.execute_batch("SAVEPOINT poph_migration")?;
        let result = conn
            .execute_batch(migration.sql)
            .and_then(|()| {
                conn.execute(
                    "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
                    params![migration.version, migration.name, now],
                )
                .map(|_| ())
            })
            .and_then(|()| conn.execute_batch("RELEASE poph_migration"));
        match result {
            Ok(()) => applied.push(AppliedMigration {
                version: migration.version,
                name: migration.name.to_owned(),
            }),
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK TO poph_migration; RELEASE poph_migration");
                return Err(StoreError::Sqlite(error));
            }
        }
    }

    Ok(MigrationReport {
        database,
        path: path.to_path_buf(),
        version_before,
        version_after: current_version(conn)?,
        applied,
    })
}

pub fn current_version(conn: &Connection) -> Resultado<u32> {
    let exists: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Ok(0);
    }
    let version: Option<u32> =
        conn.query_row("SELECT max(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })?;
    Ok(version.unwrap_or(0))
}

pub fn file_version(path: &Path) -> Resultado<Option<u32>> {
    use rusqlite::OpenFlags;

    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| StoreError::OpenFailed {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;
    let exists: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Ok(None);
    }
    let version: Option<u32> =
        conn.query_row("SELECT max(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })?;
    Ok(Some(version.unwrap_or(0)))
}
