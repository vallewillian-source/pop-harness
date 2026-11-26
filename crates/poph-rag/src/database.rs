use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, Transaction, params};

use crate::embedder::ModelIdentity;
use crate::error::RagError;

pub const SCHEMA_VERSION: &str = "1";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS rag_meta (
    chave TEXT PRIMARY KEY,
    valor TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS rag_itens (
    org_id TEXT NOT NULL,
    agente_id TEXT NOT NULL,
    item_id TEXT NOT NULL,
    titulo TEXT NOT NULL,
    versao INTEGER NOT NULL,
    hash_origem TEXT NOT NULL,
    trechos INTEGER NOT NULL,
    PRIMARY KEY (org_id, agente_id, item_id)
) STRICT;

CREATE TABLE IF NOT EXISTS rag_trechos (
    id INTEGER PRIMARY KEY,
    org_id TEXT NOT NULL,
    agente_id TEXT NOT NULL,
    item_id TEXT NOT NULL,
    ordem INTEGER NOT NULL,
    texto TEXT NOT NULL,
    vetor BLOB NOT NULL,
    norma REAL NOT NULL,
    FOREIGN KEY (org_id, agente_id, item_id)
        REFERENCES rag_itens (org_id, agente_id, item_id)
) STRICT;

CREATE INDEX IF NOT EXISTS rag_trechos_agente
    ON rag_trechos (org_id, agente_id, id);

CREATE VIRTUAL TABLE IF NOT EXISTS rag_trechos_fts USING fts5(
    texto_busca,
    org_id UNINDEXED,
    agente_id UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Produtos (HR-18): mesmas estruturas da base de conhecimento, em tabelas próprias para as duas
-- buscas não se misturarem. Só o que casa por texto entra aqui: preço e condições vêm do registro
-- no `harness.db` na hora da resposta, então mudar preço não pede reindexação.
CREATE TABLE IF NOT EXISTS rag_produtos (
    org_id TEXT NOT NULL,
    agente_id TEXT NOT NULL,
    produto_id TEXT NOT NULL,
    nome TEXT NOT NULL,
    versao INTEGER NOT NULL,
    hash_origem TEXT NOT NULL,
    trechos INTEGER NOT NULL,
    PRIMARY KEY (org_id, agente_id, produto_id)
) STRICT;

CREATE TABLE IF NOT EXISTS rag_produtos_trechos (
    id INTEGER PRIMARY KEY,
    org_id TEXT NOT NULL,
    agente_id TEXT NOT NULL,
    produto_id TEXT NOT NULL,
    ordem INTEGER NOT NULL,
    texto TEXT NOT NULL,
    vetor BLOB NOT NULL,
    norma REAL NOT NULL,
    FOREIGN KEY (org_id, agente_id, produto_id)
        REFERENCES rag_produtos (org_id, agente_id, produto_id)
) STRICT;

CREATE INDEX IF NOT EXISTS rag_produtos_trechos_agente
    ON rag_produtos_trechos (org_id, agente_id, id);

CREATE VIRTUAL TABLE IF NOT EXISTS rag_produtos_fts USING fts5(
    texto_busca,
    org_id UNINDEXED,
    agente_id UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
);
";

pub fn database_error(error: rusqlite::Error) -> RagError {
    RagError::Database(error.to_string())
}

pub fn open_database(path: &Path) -> Result<Connection, RagError> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory).map_err(|error| {
            RagError::File(format!("não consegui criar a pasta do rag.db: {error}"))
        })?;
    }
    let conn = Connection::open(path).map_err(database_error)?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(database_error)?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(database_error)?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(database_error)?;
    conn.busy_timeout(Duration::from_secs(2))
        .map_err(database_error)?;
    conn.execute_batch(SCHEMA).map_err(database_error)?;
    Ok(conn)
}

pub fn identity_metadata(identity: &ModelIdentity) -> BTreeMap<String, String> {
    [
        ("esquema", SCHEMA_VERSION.to_owned()),
        ("nome_modelo", identity.name.to_owned()),
        ("quantizacao", identity.quantization.to_owned()),
        ("dimensao", identity.dimension.to_string()),
        ("janela_tokens", identity.token_window.to_string()),
        ("sha256_modelo", identity.model_sha256.clone()),
        ("sha256_tokenizador", identity.tokenizer_sha256.clone()),
        ("versao_chunker", identity.chunker_version.to_owned()),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value))
    .collect()
}

pub fn check_identity(tx: &Transaction<'_>, identity: &ModelIdentity) -> Result<bool, RagError> {
    let stored = read_metadata(tx)?;
    let expected = identity_metadata(identity);
    if stored == expected {
        return Ok(false);
    }
    let had_data = !stored.is_empty();
    tx.execute_batch(
        "DELETE FROM rag_trechos_fts;
         DELETE FROM rag_trechos;
         DELETE FROM rag_itens;
         DELETE FROM rag_produtos_fts;
         DELETE FROM rag_produtos_trechos;
         DELETE FROM rag_produtos;
         DELETE FROM rag_meta;",
    )
    .map_err(database_error)?;
    for (key, value) in &expected {
        tx.execute(
            "INSERT INTO rag_meta (chave, valor) VALUES (?1, ?2)",
            params![key, value],
        )
        .map_err(database_error)?;
    }
    Ok(had_data)
}

pub fn read_metadata(conn: &Connection) -> Result<BTreeMap<String, String>, RagError> {
    let mut stmt = conn
        .prepare("SELECT chave, valor FROM rag_meta")
        .map_err(database_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(database_error)?;
    let mut meta = BTreeMap::new();
    for row in rows {
        let (key, value) = row.map_err(database_error)?;
        meta.insert(key, value);
    }
    Ok(meta)
}
