//! Migrações versionadas de `harness.db` e `rag.db`.
//!
//! Cada migração é um arquivo SQL embutido no binário (`include_str!`), aplicado uma vez e
//! registrado em `schema_migrations`. O runner é idempotente: rodar de novo num banco já migrado
//! não aplica nada e devolve o mesmo relatório.
//!
//! Decisão de implementação: em vez de `rusqlite_migration` (citado no PRD §5.3), o runner é
//! próprio, com ~60 linhas. Motivo: o PRD §7.2 já exige a tabela `schema_migrations` com versão,
//! nome e data para o `poph doctor` ler, e assim não entra mais uma dependência no binário.
//!
//! Cada migração roda dentro de um `SAVEPOINT`, o que funciona tanto chamada sozinha quanto
//! dentro da transação em lote do escritor: uma migração que falha não leva as outras nem a
//! escrita concorrente junto.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::erro::{ErroStore, Resultado};

/// Uma migração embutida.
#[derive(Debug, Clone, Copy)]
pub struct Migracao {
    /// Número sequencial, começando em 1.
    pub versao: u32,
    /// Nome do arquivo, sem extensão (vai para `schema_migrations.name`).
    pub nome: &'static str,
    /// SQL completo.
    pub sql: &'static str,
}

/// Migrações de `harness.db`, na ordem.
pub const MIGRACOES_HARNESS: &[Migracao] = &[
    Migracao {
        versao: 1,
        nome: "0001_esquema_inicial",
        sql: include_str!("../migrations/harness/0001_esquema_inicial.sql"),
    },
    Migracao {
        versao: 2,
        nome: "0002_entrada_proxima_tentativa",
        sql: include_str!("../migrations/harness/0002_entrada_proxima_tentativa.sql"),
    },
    Migracao {
        versao: 3,
        nome: "0003_origem_autor_mensagens",
        sql: include_str!("../migrations/harness/0003_origem_autor_mensagens.sql"),
    },
    Migracao {
        versao: 4,
        nome: "0004_audit_log_matrix_user",
        sql: include_str!("../migrations/harness/0004_audit_log_matrix_user.sql"),
    },
    Migracao {
        versao: 5,
        nome: "0005_tabelas_matrix",
        sql: include_str!("../migrations/harness/0005_tabelas_matrix.sql"),
    },
    Migracao {
        versao: 6,
        nome: "0006_indice_conversations_atualizada",
        sql: include_str!("../migrations/harness/0006_indice_conversations_atualizada.sql"),
    },
    Migracao {
        versao: 7,
        nome: "0007_matrix_cursor_mensagem_id",
        sql: include_str!("../migrations/harness/0007_matrix_cursor_mensagem_id.sql"),
    },
    Migracao {
        versao: 8,
        nome: "0008_conversa_nome_cliente",
        sql: include_str!("../migrations/harness/0008_conversa_nome_cliente.sql"),
    },
    Migracao {
        versao: 9,
        nome: "0009_matrix_avisos_falha",
        sql: include_str!("../migrations/harness/0009_matrix_avisos_falha.sql"),
    },
    Migracao {
        versao: 10,
        nome: "0010_matrix_rooms_nome_cliente_espelhado",
        sql: include_str!("../migrations/harness/0010_matrix_rooms_nome_cliente_espelhado.sql"),
    },
    Migracao {
        versao: 11,
        nome: "0011_outbox_id_externo",
        sql: include_str!("../migrations/harness/0011_outbox_id_externo.sql"),
    },
    Migracao {
        versao: 12,
        nome: "0012_fichas_e_eventos",
        sql: include_str!("../migrations/harness/0012_fichas_e_eventos.sql"),
    },
    Migracao {
        versao: 13,
        nome: "0013_fluxos_revisoes_execucoes_e_efeitos",
        sql: include_str!("../migrations/harness/0013_fluxos_revisoes_execucoes_e_efeitos.sql"),
    },
    Migracao {
        versao: 15,
        nome: "0015_conversas_por_cliente",
        sql: include_str!("../migrations/harness/0015_conversas_por_cliente.sql"),
    },
];

/// Migrações de `rag.db`. Vazio até a HR-16 (trechos, vetores int8 e FTS5).
pub const MIGRACOES_RAG: &[Migracao] = &[];

/// Versão de esquema que este código espera em `harness.db`.
///
/// É a fonte única de quem escreve ou confere a versão do esquema: a recusa do `poph serve` e os
/// testes que citam a versão montam o texto a partir daqui, então uma migração nova não deixa uma
/// literal antiga para trás.
pub fn versao_alvo_harness() -> u32 {
    MIGRACOES_HARNESS
        .last()
        .map(|migracao| migracao.versao)
        .unwrap_or(0)
}

/// Uma migração que rodou agora.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigracaoAplicada {
    /// Versão aplicada.
    pub versao: u32,
    /// Nome aplicado.
    pub nome: String,
}

/// O que aconteceu ao migrar um banco.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatorioMigracao {
    /// Qual banco (`harness.db` ou `rag.db`).
    pub banco: &'static str,
    /// Caminho do arquivo.
    pub caminho: PathBuf,
    /// Versão antes.
    pub versao_antes: u32,
    /// Versão depois.
    pub versao_depois: u32,
    /// Migrações aplicadas nesta chamada (vazio quando já estava em dia).
    pub aplicadas: Vec<MigracaoAplicada>,
}

impl RelatorioMigracao {
    /// `true` quando nada precisou ser aplicado.
    pub fn ja_estava_em_dia(&self) -> bool {
        self.aplicadas.is_empty()
    }

    /// `true` quando o banco não tem nenhuma migração aplicada.
    pub fn banco_vazio(&self) -> bool {
        self.versao_antes == 0 && !self.aplicadas.is_empty()
    }

    /// Linha única para o resumo em texto.
    pub fn resumo(&self) -> String {
        if self.ja_estava_em_dia() {
            format!(
                "{}: já estava na versão {} ({})",
                self.banco,
                self.versao_depois,
                self.caminho.display()
            )
        } else {
            let nomes = self
                .aplicadas
                .iter()
                .map(|m| format!("{} ({})", m.versao, m.nome))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{}: versão {} → {}; aplicadas: {} ({})",
                self.banco,
                self.versao_antes,
                self.versao_depois,
                nomes,
                self.caminho.display()
            )
        }
    }
}

/// SQL que cria a tabela de controle. Fora das migrações numeradas de propósito: ela precisa
/// existir antes da primeira.
const BOOTSTRAP: &str = "CREATE TABLE IF NOT EXISTS schema_migrations (
    version    INTEGER PRIMARY KEY NOT NULL,
    name       TEXT    NOT NULL,
    applied_at INTEGER NOT NULL
) STRICT;";

/// Aplica as migrações que faltam e devolve o que rodou.
///
/// `agora` é o carimbo de tempo em ms; o armazenamento não lê o relógio por conta própria.
pub fn migrar(
    conn: &Connection,
    banco: &'static str,
    caminho: &Path,
    migracoes: &[Migracao],
    agora: i64,
) -> Resultado<RelatorioMigracao> {
    conn.execute_batch(BOOTSTRAP)?;
    let versao_antes = versao_atual(conn)?;
    let mut aplicadas = Vec::new();

    for migracao in migracoes.iter().filter(|m| m.versao > versao_antes) {
        conn.execute_batch("SAVEPOINT poph_migracao")?;
        let resultado = conn
            .execute_batch(migracao.sql)
            .and_then(|()| {
                conn.execute(
                    "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
                    params![migracao.versao, migracao.nome, agora],
                )
                .map(|_| ())
            })
            .and_then(|()| conn.execute_batch("RELEASE poph_migracao"));
        match resultado {
            Ok(()) => aplicadas.push(MigracaoAplicada {
                versao: migracao.versao,
                nome: migracao.nome.to_owned(),
            }),
            Err(erro) => {
                let _ = conn.execute_batch("ROLLBACK TO poph_migracao; RELEASE poph_migracao");
                return Err(ErroStore::Sqlite(erro));
            }
        }
    }

    Ok(RelatorioMigracao {
        banco,
        caminho: caminho.to_path_buf(),
        versao_antes,
        versao_depois: versao_atual(conn)?,
        aplicadas,
    })
}

/// Maior versão aplicada (0 quando nenhuma).
pub fn versao_atual(conn: &Connection) -> Resultado<u32> {
    let existe: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |linha| linha.get(0),
        )
        .optional()?;
    if existe.is_none() {
        return Ok(0);
    }
    let versao: Option<u32> =
        conn.query_row("SELECT max(version) FROM schema_migrations", [], |linha| {
            linha.get(0)
        })?;
    Ok(versao.unwrap_or(0))
}

/// Lê a versão do esquema de um banco **sem alterá-lo** (usado pelo `poph doctor`).
///
/// `Ok(None)` quando o arquivo existe mas nunca foi migrado; `Err` quando não é SQLite.
pub fn versao_do_arquivo(caminho: &Path) -> Resultado<Option<u32>> {
    use rusqlite::OpenFlags;

    let conn = Connection::open_with_flags(
        caminho,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|erro| ErroStore::Abertura {
        caminho: caminho.display().to_string(),
        motivo: erro.to_string(),
    })?;
    let existe: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |linha| linha.get(0),
        )
        .optional()?;
    if existe.is_none() {
        return Ok(None);
    }
    let versao: Option<u32> =
        conn.query_row("SELECT max(version) FROM schema_migrations", [], |linha| {
            linha.get(0)
        })?;
    // A tabela existe: `0` é "criado e ainda sem migração" (caso do `rag.db` até a HR-16).
    Ok(Some(versao.unwrap_or(0)))
}
