//! Erro do armazenamento.
//!
//! Os erros de domínio (id inválido, registro ausente, conflito de versão, campo recusado) vêm de
//! `poph-types` e são reaproveitados: quem chama o repositório trata conflito de versão igual em
//! qualquer entidade. As variantes daqui cobrem só o que é do banco (SQLite, escritor, abertura).

use poph_types::DomainError;
use rusqlite::ffi;
use thiserror::Error;

/// Resultado padrão dos repositórios.
pub type Resultado<T> = Result<T, ErroStore>;

/// Tudo que pode dar errado ao falar com o banco.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ErroStore {
    /// Regra de domínio: id inválido, registro ausente, conflito de versão, valor recusado.
    #[error(transparent)]
    Dominio(#[from] DomainError),

    /// Erro cru do SQLite (sintaxe, restrição, banco ocupado).
    #[error("banco de dados: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// O escritor dedicado terminou (o processo está encerrando ou ele falhou).
    #[error("escritor do banco encerrado: nenhuma gravação é aceita")]
    EscritorEncerrado,

    /// Não consegui abrir o arquivo do banco (pasta sem escrita, caminho é pasta, etc.).
    #[error("não consegui abrir o banco {caminho}: {motivo}")]
    Abertura { caminho: String, motivo: String },

    /// Falha que não deveria acontecer (tipo de resposta inesperado, transação quebrada).
    #[error("erro interno do armazenamento: {0}")]
    Interno(String),
}

impl ErroStore {
    /// `true` quando a escrita perdeu a corrida com outra escrita (o `If-Match` está velho).
    pub fn eh_conflito_de_versao(&self) -> bool {
        matches!(
            self,
            ErroStore::Dominio(DomainError::VersionConflict { .. })
        )
    }

    /// `true` quando o registro não existe (ou não existe na org pedida).
    pub fn eh_nao_encontrado(&self) -> bool {
        matches!(self, ErroStore::Dominio(DomainError::NotFound { .. }))
    }

    /// Traduz a falha de uma escrita no erro de domínio que a API e a CLI mostram.
    ///
    /// Vale o código estendido do SQLite, não só a classe da restrição:
    /// - índice único ou chave primária repetida → duplicidade (texto de sempre);
    /// - `CHECK` (faixa ou lista de valores) e `NOT NULL` → o campo e o motivo;
    /// - chave estrangeira → o registro apontado não existe nesta org;
    /// - qualquer outra falha → erro cru do banco, sem mensagem inventada.
    ///
    /// `campo` é o nome do campo que o chamador expõe. Quando a escrita toca mais de uma coluna,
    /// `colunas` diz qual campo corresponde a cada coluna, porque o SQLite nomeia a coluna na
    /// mensagem de `CHECK` e de `NOT NULL` (ex.: `max_context_tokens`). Sem coluna reconhecida,
    /// vale `campo`.
    pub(crate) fn da_escrita_em(
        campo: &'static str,
        colunas: &[(&str, &'static str)],
        erro: rusqlite::Error,
    ) -> Self {
        let rusqlite::Error::SqliteFailure(falha, mensagem) = &erro else {
            return ErroStore::Sqlite(erro);
        };
        let mensagem = mensagem.as_deref().unwrap_or("");
        let (campo, motivo) = match falha.extended_code {
            ffi::SQLITE_CONSTRAINT_UNIQUE | ffi::SQLITE_CONSTRAINT_PRIMARYKEY => {
                (campo, "já existe um registro com esse valor nesta org")
            }
            ffi::SQLITE_CONSTRAINT_CHECK => (
                campo_da_coluna(mensagem, colunas).unwrap_or(campo),
                "valor fora da faixa ou da lista permitida para este campo",
            ),
            ffi::SQLITE_CONSTRAINT_NOTNULL => (
                campo_da_coluna(mensagem, colunas).unwrap_or(campo),
                "campo obrigatório, não pode ficar vazio",
            ),
            ffi::SQLITE_CONSTRAINT_FOREIGNKEY => {
                (campo, "aponta para um registro que não existe nesta org")
            }
            _ => return ErroStore::Sqlite(erro),
        };
        ErroStore::Dominio(DomainError::Validation {
            field: campo,
            reason: motivo.to_owned(),
        })
    }

    /// Atalho de [`ErroStore::da_escrita_em`] para escritas que tocam um único campo relevante.
    pub(crate) fn da_escrita(campo: &'static str, erro: rusqlite::Error) -> Self {
        Self::da_escrita_em(campo, &[], erro)
    }
}

/// Procura, na mensagem do SQLite, a primeira coluna conhecida e devolve o campo dela.
///
/// A mensagem traz a coluna como palavra: `CHECK constraint failed: max_context_tokens >= 1000`
/// ou `NOT NULL constraint failed: agents.name`.
fn campo_da_coluna(mensagem: &str, colunas: &[(&str, &'static str)]) -> Option<&'static str> {
    let palavras: Vec<&str> = mensagem
        .split(|caractere: char| !(caractere.is_alphanumeric() || caractere == '_'))
        .collect();
    colunas
        .iter()
        .find(|(coluna, _)| palavras.contains(coluna))
        .map(|(_, campo)| *campo)
}

#[cfg(test)]
mod testes {
    use super::*;
    use rusqlite::Connection;

    /// Banco em memória com uma restrição de cada categoria, como as tabelas do harness.
    fn banco() -> Connection {
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

    /// Erro cru que o SQLite devolve para a instrução dada.
    fn erro_do_banco(conn: &Connection, sql: &str) -> rusqlite::Error {
        conn.execute(sql, [])
            .expect_err("o banco devia recusar a escrita")
    }

    fn validacao(erro: ErroStore) -> (&'static str, String) {
        match erro {
            ErroStore::Dominio(DomainError::Validation { field, reason }) => (field, reason),
            outro => panic!("esperava validação do campo, veio {outro:?}"),
        }
    }

    #[test]
    fn unico_repetido_continua_duplicidade_no_campo_do_chamador() {
        let conn = banco();
        conn.execute("INSERT INTO itens VALUES ('a', 'x', 'A', 1, NULL)", [])
            .unwrap();
        let erro = erro_do_banco(&conn, "INSERT INTO itens VALUES ('b', 'x', 'B', 1, NULL)");
        let (campo, motivo) = validacao(ErroStore::da_escrita_em("slug", &[], erro));
        assert_eq!(campo, "slug");
        assert_eq!(motivo, "já existe um registro com esse valor nesta org");
    }

    #[test]
    fn check_viola_faixa_e_nomeia_o_campo_da_coluna_mesmo_com_outro_campo_padrao() {
        let conn = banco();
        let erro = erro_do_banco(&conn, "INSERT INTO itens VALUES ('a', 'x', 'A', 99, NULL)");
        let colunas = [("faixa", "faixa_de_resposta")];
        let (campo, motivo) = validacao(ErroStore::da_escrita_em("slug", &colunas, erro));
        assert_eq!(campo, "faixa_de_resposta");
        assert_eq!(
            motivo,
            "valor fora da faixa ou da lista permitida para este campo"
        );
    }

    #[test]
    fn check_sem_coluna_reconhecida_cai_no_campo_do_chamador() {
        let conn = banco();
        let erro = erro_do_banco(&conn, "INSERT INTO itens VALUES ('a', 'x', 'A', 99, NULL)");
        let (campo, _) = validacao(ErroStore::da_escrita("slug", erro));
        assert_eq!(campo, "slug");
    }

    #[test]
    fn nao_nulo_nomeia_a_coluna_e_diz_que_e_obrigatorio() {
        let conn = banco();
        let erro = erro_do_banco(
            &conn,
            "INSERT INTO itens (id, slug, faixa) VALUES ('a', 'x', 1)",
        );
        let colunas = [("nome", "nome")];
        let (campo, motivo) = validacao(ErroStore::da_escrita_em("slug", &colunas, erro));
        assert_eq!(campo, "nome");
        assert_eq!(motivo, "campo obrigatório, não pode ficar vazio");
    }

    #[test]
    fn chave_estrangeira_diz_que_o_registro_nao_existe_nesta_org() {
        let conn = banco();
        let erro = erro_do_banco(
            &conn,
            "INSERT INTO itens VALUES ('a', 'x', 'A', 1, 'pai-que-nao-existe')",
        );
        let (campo, motivo) = validacao(ErroStore::da_escrita("pai", erro));
        assert_eq!(campo, "pai");
        assert_eq!(motivo, "aponta para um registro que não existe nesta org");
    }

    #[test]
    fn outra_restricao_continua_erro_cru_do_banco() {
        // `RAISE(ABORT)` de gatilho sai com o código estendido de gatilho, que não é tratado aqui.
        let conn = banco();
        conn.execute_batch(
            "CREATE TRIGGER recusa BEFORE INSERT ON itens
             BEGIN SELECT RAISE(ABORT, 'recusado pelo gatilho'); END;",
        )
        .unwrap();
        let erro = erro_do_banco(&conn, "INSERT INTO itens VALUES ('a', 'x', 'A', 1, NULL)");
        assert_eq!(
            erro.sqlite_extended_error_code(),
            Some(ffi::SQLITE_CONSTRAINT_TRIGGER)
        );
        assert!(matches!(
            ErroStore::da_escrita("slug", erro),
            ErroStore::Sqlite(_)
        ));
    }

    #[test]
    fn codigos_estendidos_usados_no_mapeamento_sao_os_do_sqlite() {
        // Trava: se a versão do SQLite mudar um código, o teste acusa.
        assert_eq!(ffi::SQLITE_CONSTRAINT_CHECK, 275);
        assert_eq!(ffi::SQLITE_CONSTRAINT_NOTNULL, 1299);
        assert_eq!(ffi::SQLITE_CONSTRAINT_FOREIGNKEY, 787);
        assert_eq!(ffi::SQLITE_CONSTRAINT_UNIQUE, 2067);
    }
}
