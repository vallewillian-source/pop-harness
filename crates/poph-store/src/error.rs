use poph_types::DomainError;
use rusqlite::ffi;
use thiserror::Error;

pub type Resultado<T> = Result<T, StoreError>;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StoreError {
    #[error(transparent)]
    Domain(#[from] DomainError),

    #[error("banco de dados: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("escritor do banco encerrado: nenhuma gravação é aceita")]
    WriterClosed,

    #[error("não consegui abrir o banco {path}: {reason}")]
    OpenFailed { path: String, reason: String },

    #[error("erro interno do armazenamento: {0}")]
    Internal(String),
}

impl StoreError {
    pub fn is_version_conflict(&self) -> bool {
        matches!(
            self,
            StoreError::Domain(DomainError::VersionConflict { .. })
        )
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self, StoreError::Domain(DomainError::NotFound { .. }))
    }

    pub(crate) fn from_write_with(
        field: &'static str,
        columns: &[(&str, &'static str)],
        error: rusqlite::Error,
    ) -> Self {
        let rusqlite::Error::SqliteFailure(failure, mensagem) = &error else {
            return StoreError::Sqlite(error);
        };
        let mensagem = mensagem.as_deref().unwrap_or("");
        let (field, reason) = match failure.extended_code {
            ffi::SQLITE_CONSTRAINT_UNIQUE | ffi::SQLITE_CONSTRAINT_PRIMARYKEY => {
                (field, "já existe um registro com esse valor nesta org")
            }
            ffi::SQLITE_CONSTRAINT_CHECK => (
                column_field(mensagem, columns).unwrap_or(field),
                "valor fora da faixa ou da lista permitida para este campo",
            ),
            ffi::SQLITE_CONSTRAINT_NOTNULL => (
                column_field(mensagem, columns).unwrap_or(field),
                "campo obrigatório, não pode ficar vazio",
            ),
            ffi::SQLITE_CONSTRAINT_FOREIGNKEY => {
                (field, "aponta para um registro que não existe nesta org")
            }
            _ => return StoreError::Sqlite(error),
        };
        StoreError::Domain(DomainError::Validation {
            field,
            reason: reason.to_owned(),
        })
    }

    pub(crate) fn from_write(field: &'static str, error: rusqlite::Error) -> Self {
        Self::from_write_with(field, &[], error)
    }
}

fn column_field(mensagem: &str, columns: &[(&str, &'static str)]) -> Option<&'static str> {
    let words: Vec<&str> = mensagem
        .split(|character: char| !(character.is_alphanumeric() || character == '_'))
        .collect();
    columns
        .iter()
        .find(|(column, _)| words.contains(column))
        .map(|(_, field)| *field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn database() -> Connection {
        let conn = Connection::open_in_memory().expect("abre o banco em memória");
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE pais (id TEXT PRIMARY KEY NOT NULL);
             CREATE TABLE itens (
                 id       TEXT PRIMARY KEY NOT NULL,
                 slug     TEXT NOT NULL,
                 nome     TEXT NOT NULL,
                 faixa    INTEGER NOT NULL CHECK (faixa BETWEEN 1 AND 50),
                 pai_id   TEXT REFERENCES pais (id)
             );
             CREATE UNIQUE INDEX ux_itens_slug ON itens (slug);",
        )
        .expect("cria as tabelas de teste");
        conn
    }

    fn database_error(conn: &Connection, sql: &str) -> rusqlite::Error {
        conn.execute(sql, [])
            .expect_err("o banco devia recusar a escrita")
    }

    fn validation(error: StoreError) -> (&'static str, String) {
        match error {
            StoreError::Domain(DomainError::Validation { field, reason }) => (field, reason),
            other => panic!("esperava validação do campo, veio {other:?}"),
        }
    }

    #[test]
    fn repeated_unique_stays_duplicate_in_caller_field() {
        let conn = database();
        conn.execute("INSERT INTO itens VALUES ('a', 'x', 'A', 1, NULL)", [])
            .unwrap();
        let error = database_error(&conn, "INSERT INTO itens VALUES ('b', 'x', 'B', 1, NULL)");
        let (field, reason) = validation(StoreError::from_write_with("slug", &[], error));
        assert_eq!(field, "slug");
        assert_eq!(reason, "já existe um registro com esse valor nesta org");
    }

    #[test]
    fn check_violates_range_and_names_column_field_even_with_other_default_field() {
        let conn = database();
        let error = database_error(&conn, "INSERT INTO itens VALUES ('a', 'x', 'A', 99, NULL)");
        let columns = [("faixa", "faixa_de_resposta")];
        let (field, reason) = validation(StoreError::from_write_with("slug", &columns, error));
        assert_eq!(field, "faixa_de_resposta");
        assert_eq!(
            reason,
            "valor fora da faixa ou da lista permitida para este campo"
        );
    }

    #[test]
    fn check_without_known_column_falls_back_to_caller_field() {
        let conn = database();
        let error = database_error(&conn, "INSERT INTO itens VALUES ('a', 'x', 'A', 99, NULL)");
        let (field, _) = validation(StoreError::from_write("slug", error));
        assert_eq!(field, "slug");
    }

    #[test]
    fn not_null_names_column_and_says_it_is_required() {
        let conn = database();
        let error = database_error(
            &conn,
            "INSERT INTO itens (id, slug, faixa) VALUES ('a', 'x', 1)",
        );
        let columns = [("nome", "nome")];
        let (field, reason) = validation(StoreError::from_write_with("slug", &columns, error));
        assert_eq!(field, "nome");
        assert_eq!(reason, "campo obrigatório, não pode ficar vazio");
    }

    #[test]
    fn foreign_key_says_record_does_not_exist_in_this_org() {
        let conn = database();
        let error = database_error(
            &conn,
            "INSERT INTO itens VALUES ('a', 'x', 'A', 1, 'pai-que-nao-existe')",
        );
        let (field, reason) = validation(StoreError::from_write("pai", error));
        assert_eq!(field, "pai");
        assert_eq!(reason, "aponta para um registro que não existe nesta org");
    }

    #[test]
    fn other_constraint_keeps_raw_database_error() {
        let conn = database();
        conn.execute_batch(
            "CREATE TRIGGER recusa BEFORE INSERT ON itens
             BEGIN SELECT RAISE(ABORT, 'recusado pelo gatilho'); END;",
        )
        .unwrap();
        let error = database_error(&conn, "INSERT INTO itens VALUES ('a', 'x', 'A', 1, NULL)");
        assert_eq!(
            error.sqlite_extended_error_code(),
            Some(ffi::SQLITE_CONSTRAINT_TRIGGER)
        );
        assert!(matches!(
            StoreError::from_write("slug", error),
            StoreError::Sqlite(_)
        ));
    }

    #[test]
    fn extended_codes_used_in_mapping_are_the_sqlite_ones() {
        assert_eq!(ffi::SQLITE_CONSTRAINT_CHECK, 275);
        assert_eq!(ffi::SQLITE_CONSTRAINT_NOTNULL, 1299);
        assert_eq!(ffi::SQLITE_CONSTRAINT_FOREIGNKEY, 787);
        assert_eq!(ffi::SQLITE_CONSTRAINT_UNIQUE, 2067);
    }
}
