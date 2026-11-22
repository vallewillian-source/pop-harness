//! Esquema e conexão do `rag.db` (HR-16). Derivado e reconstruível: a base de origem fica no
//! `harness.db`. Ver também `docs/DADOS.md`.
//!
//! Tabelas:
//! - `rag_meta`: identidade do modelo e do divisor que geraram os vetores.
//! - `rag_itens`: um item por (org, agente), com o hash de origem usado na indexação incremental.
//! - `rag_trechos`: trechos com vetor int8, norma e texto.
//! - `rag_trechos_fts`: índice FTS5 do título e do texto de cada trecho (mesmo `rowid`).

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, Transaction, params};

use crate::embedder::IdentidadeModelo;
use crate::erro::ErroRag;

/// Versão do esquema deste arquivo. Mudar o esquema pede migração nova.
pub const VERSAO_ESQUEMA: &str = "1";

const ESQUEMA: &str = "
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

pub fn erro_banco(erro: rusqlite::Error) -> ErroRag {
    ErroRag::Banco(erro.to_string())
}

/// Abre (ou cria) o `rag.db` com os pragmas do projeto.
pub fn abrir(caminho: &Path) -> Result<Connection, ErroRag> {
    if let Some(pasta) = caminho.parent() {
        std::fs::create_dir_all(pasta).map_err(|erro| {
            ErroRag::Arquivo(format!("não consegui criar a pasta do rag.db: {erro}"))
        })?;
    }
    let conn = Connection::open(caminho).map_err(erro_banco)?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(erro_banco)?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(erro_banco)?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(erro_banco)?;
    conn.busy_timeout(Duration::from_secs(2))
        .map_err(erro_banco)?;
    conn.execute_batch(ESQUEMA).map_err(erro_banco)?;
    Ok(conn)
}

/// Identidade gravada em `rag_meta`. Se não bater com o modelo em uso, os vetores são de outro
/// modelo ou de outro divisor e não podem ser misturados.
pub fn meta_da_identidade(identidade: &IdentidadeModelo) -> BTreeMap<String, String> {
    [
        ("esquema", VERSAO_ESQUEMA.to_owned()),
        ("nome_modelo", identidade.nome.to_owned()),
        ("quantizacao", identidade.quantizacao.to_owned()),
        ("dimensao", identidade.dimensao.to_string()),
        ("janela_tokens", identidade.janela_tokens.to_string()),
        ("sha256_modelo", identidade.sha256_modelo.clone()),
        ("sha256_tokenizador", identidade.sha256_tokenizador.clone()),
        ("versao_chunker", identidade.versao_chunker.to_owned()),
    ]
    .into_iter()
    .map(|(chave, valor)| (chave.to_owned(), valor))
    .collect()
}

/// Confere a identidade do `rag.db` com a do modelo em uso. Se divergir, apaga os vetores (são
/// reconstruíveis) e grava a identidade nova. Devolve `true` quando apagou dados por divergência.
pub fn conferir_identidade(
    tx: &Transaction<'_>,
    identidade: &IdentidadeModelo,
) -> Result<bool, ErroRag> {
    let gravado = ler_meta(tx)?;
    let esperado = meta_da_identidade(identidade);
    if gravado == esperado {
        return Ok(false);
    }
    let havia_dados = !gravado.is_empty();
    tx.execute_batch(
        "DELETE FROM rag_trechos_fts;
         DELETE FROM rag_trechos;
         DELETE FROM rag_itens;
         DELETE FROM rag_produtos_fts;
         DELETE FROM rag_produtos_trechos;
         DELETE FROM rag_produtos;
         DELETE FROM rag_meta;",
    )
    .map_err(erro_banco)?;
    for (chave, valor) in &esperado {
        tx.execute(
            "INSERT INTO rag_meta (chave, valor) VALUES (?1, ?2)",
            params![chave, valor],
        )
        .map_err(erro_banco)?;
    }
    Ok(havia_dados)
}

pub fn ler_meta(conn: &Connection) -> Result<BTreeMap<String, String>, ErroRag> {
    let mut stmt = conn
        .prepare("SELECT chave, valor FROM rag_meta")
        .map_err(erro_banco)?;
    let linhas = stmt
        .query_map([], |linha| {
            Ok((linha.get::<_, String>(0)?, linha.get::<_, String>(1)?))
        })
        .map_err(erro_banco)?;
    let mut meta = BTreeMap::new();
    for linha in linhas {
        let (chave, valor) = linha.map_err(erro_banco)?;
        meta.insert(chave, valor);
    }
    Ok(meta)
}
