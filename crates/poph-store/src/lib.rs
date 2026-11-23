//! Armazenamento do POP Harness.
//!
//! O que este crate faz (HR-04b + HR-07):
//! - cria e migra `harness.db` (esquema do PRD §7.2) e o arquivo de `rag.db` (as tabelas de
//!   trechos entram na HR-16);
//! - mantém **um escritor dedicado** que grava em lote, com WAL, `synchronous=NORMAL`,
//!   `foreign_keys=ON` e `busy_timeout` curto;
//! - mantém um pool de leitores que não bloqueia a escrita;
//! - expõe repositórios tipados por entidade, **todos com `org_id` obrigatório**;
//! - exige `If-Match`/`versao` nas entidades editáveis, com erro de conflito tipado.
//!
//! O que ele não faz: memória (HR-11), RAG (HR-16), cifra de campos (HR-26), redes externas.
//! Tabelas do esquema que ainda não têm repositório (produtos, base de conhecimento, biblioteca
//! de arquivos, integrações, anexos, carrinho) existem para as tarefas que as usam.

pub mod backup;
pub mod backup_documento;
mod conexao;
pub mod erro;
mod escritor;
mod leitores;
pub mod migracoes;
pub mod modelo;
mod repos;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use poph_types::HarnessConfig;
use rusqlite::Connection;

pub use erro::ErroStore;
pub use migracoes::{
    MIGRACOES_HARNESS, MIGRACOES_RAG, MigracaoAplicada, RelatorioMigracao, versao_alvo_harness,
    versao_do_arquivo,
};
pub use modelo::*;
pub use repos::*;

/// Relógio injetável. O armazenamento não lê o relógio por conta própria em regra de negócio,
/// e os testes fixam a hora para conferir carimbos.
pub type Relogio = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Resultado dos repositórios.
pub type Resultado<T> = Result<T, ErroStore>;

/// Relatório de migração dos dois bancos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatoriosMigracao {
    /// `harness.db`.
    pub harness: RelatorioMigracao,
    /// `rag.db`.
    pub rag: RelatorioMigracao,
}

impl RelatoriosMigracao {
    /// `true` quando nenhum dos dois precisou de migração.
    pub fn ja_estava_em_dia(&self) -> bool {
        self.harness.ja_estava_em_dia() && self.rag.ja_estava_em_dia()
    }
}

/// Aplica as migrações de `harness.db` e `rag.db` sem abrir o servidor.
///
/// É o que o `poph migrate` usa. Só deve rodar com o servidor parado: abre conexão de escrita
/// própria, e a regra do crate é um escritor por vez.
pub fn migrar_bancos(harness: &Path, rag: &Path) -> Resultado<RelatoriosMigracao> {
    criar_pasta(harness)?;
    criar_pasta(rag)?;
    let agora = agora_do_sistema();
    Ok(RelatoriosMigracao {
        harness: prepare(harness, "harness.db", MIGRACOES_HARNESS, agora)?,
        rag: prepare(rag, "rag.db", MIGRACOES_RAG, agora)?,
    })
}

/// Hora do sistema em ms desde a época Unix (relógio padrão).
pub fn agora_do_sistema() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duracao| duracao.as_millis() as i64)
        .unwrap_or(0)
}

/// Banco de dados do Harness: escritor dedicado + pool de leitores.
pub struct Store {
    harness: PathBuf,
    rag: PathBuf,
    relogio: Relogio,
    escritor: escritor::Escritor,
    leitores: leitores::PoolLeitores,
    /// Quantas leituras o store já fez (ver [`Store::leituras_feitas`]).
    leituras: AtomicU64,
}

impl Store {
    /// Abre (e migra) os bancos nos caminhos da configuração, criando a pasta de dados.
    pub fn abrir(config: &HarnessConfig) -> Resultado<Self> {
        let diretorio = &config.dados.diretorio;
        std::fs::create_dir_all(diretorio).map_err(|erro| ErroStore::Abertura {
            caminho: diretorio.display().to_string(),
            motivo: erro.to_string(),
        })?;
        Self::abrir_em(&config.harness_db(), &config.rag_db())
    }

    /// Abre (e migra) os bancos nos caminhos dados. A pasta de cada arquivo é criada.
    pub fn abrir_em(harness: &Path, rag: &Path) -> Resultado<Self> {
        criar_pasta(harness)?;
        criar_pasta(rag)?;
        let relogio: Relogio = Arc::new(agora_do_sistema);
        prepare(harness, "harness.db", MIGRACOES_HARNESS, relogio())?;
        prepare(rag, "rag.db", MIGRACOES_RAG, relogio())?;
        let escritor = escritor::Escritor::iniciar(harness.to_path_buf())?;
        Ok(Self {
            harness: harness.to_path_buf(),
            rag: rag.to_path_buf(),
            relogio,
            leitores: leitores::PoolLeitores::novo(harness),
            escritor,
            leituras: AtomicU64::new(0),
        })
    }

    /// Troca o relógio (testes com hora fixa).
    pub fn com_relogio(mut self, relogio: Relogio) -> Self {
        self.relogio = relogio;
        self
    }

    /// Hora atual do store, em milissegundos desde a época Unix.
    pub fn agora(&self) -> i64 {
        (self.relogio)()
    }

    /// Caminho do `harness.db` em uso.
    pub fn caminho_harness(&self) -> &Path {
        &self.harness
    }

    /// Caminho do `rag.db` em uso.
    pub fn caminho_rag(&self) -> &Path {
        &self.rag
    }

    /// Aplica as migrações que faltarem (idempotente) e devolve o que rodou.
    pub fn migrar(&self) -> Resultado<RelatoriosMigracao> {
        let agora = self.agora();
        let caminho_harness = self.harness.clone();
        let harness = self.escrita(move |conn| {
            migracoes::migrar(
                conn,
                "harness.db",
                &caminho_harness,
                MIGRACOES_HARNESS,
                agora,
            )
        })?;
        let rag = prepare(&self.rag, "rag.db", MIGRACOES_RAG, agora)?;
        Ok(RelatoriosMigracao { harness, rag })
    }

    /// `PRAGMA foreign_key_check`: lista vazia = nenhuma chave quebrada.
    pub fn chaves_quebradas(&self) -> Resultado<Vec<String>> {
        self.leitura(|conn| {
            let mut consulta = conn.prepare("PRAGMA foreign_key_check")?;
            let linhas = consulta.query_map([], |linha| {
                let tabela: String = linha.get(0)?;
                let linha_id: i64 = linha.get(1).unwrap_or(0);
                let pai: String = linha.get(2)?;
                Ok(format!("{tabela}#{linha_id} → {pai}"))
            })?;
            let mut quebradas = Vec::new();
            for linha in linhas {
                quebradas.push(linha?);
            }
            Ok(quebradas)
        })
    }

    /// Tabelas existentes (nome, na ordem alfabética).
    pub fn tabelas(&self) -> Resultado<Vec<String>> {
        self.leitura(|conn| {
            let mut consulta = conn.prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )?;
            let linhas = consulta.query_map([], |linha| linha.get::<_, String>(0))?;
            let mut nomes = Vec::new();
            for linha in linhas {
                nomes.push(linha?);
            }
            Ok(nomes)
        })
    }

    /// Colunas de uma tabela (para o teste de `org_id` e para inspeção).
    pub fn colunas(&self, tabela: &str) -> Resultado<Vec<String>> {
        self.leitura(|conn| {
            let mut consulta = conn.prepare(&format!("PRAGMA table_info({tabela})"))?;
            let linhas = consulta.query_map([], |linha| linha.get::<_, String>(1))?;
            let mut nomes = Vec::new();
            for linha in linhas {
                nomes.push(linha?);
            }
            Ok(nomes)
        })
    }

    /// Roda uma escrita no escritor dedicado, dentro do lote e da transação dele.
    pub(crate) fn escrita<T: Send + 'static>(
        &self,
        tarefa: impl FnOnce(&Connection) -> Resultado<T> + Send + 'static,
    ) -> Resultado<T> {
        self.escritor.executar(tarefa)
    }

    /// Roda uma leitura no pool.
    pub(crate) fn leitura<T>(
        &self,
        consulta: impl FnOnce(&Connection) -> Resultado<T>,
    ) -> Resultado<T> {
        self.leituras.fetch_add(1, Ordering::Relaxed);
        self.leitores.com(consulta)
    }

    /// Quantas leituras o store já fez desde que foi aberto: cada chamada de repositório que lê
    /// faz uma. Serve para provar, em teste, que uma página da API não faz uma consulta por linha
    /// (RP-01): o número de leituras de uma página não cresce com o tamanho dela.
    pub fn leituras_feitas(&self) -> u64 {
        self.leituras.load(Ordering::Relaxed)
    }

    /// Repositório de orgs.
    pub fn orgaos(&self) -> Orgaos<'_> {
        Orgaos(self)
    }

    /// Repositório de tokens de administração.
    pub fn tokens(&self) -> Tokens<'_> {
        Tokens(self)
    }

    /// Base de conhecimento: grupos, itens e vínculo com agentes (HR-17).
    pub fn conhecimento(&self) -> Conhecimento<'_> {
        Conhecimento(self)
    }

    /// Categorias de produto da org.
    pub fn categorias(&self) -> Categorias<'_> {
        Categorias(self)
    }

    /// Produtos do catálogo da org.
    pub fn produtos(&self) -> Produtos<'_> {
        Produtos(self)
    }

    /// Repositório de departamentos.
    pub fn departamentos(&self) -> Departamentos<'_> {
        Departamentos(self)
    }

    /// Repositório de agentes.
    pub fn agentes(&self) -> Agentes<'_> {
        Agentes(self)
    }

    /// Repositório de instruções do agente.
    pub fn instrucoes(&self) -> InstrucoesRepo<'_> {
        InstrucoesRepo(self)
    }

    /// Repositório de conversas.
    pub fn conversas(&self) -> Conversas<'_> {
        Conversas(self)
    }

    /// Repositório de Fichas (o registro do atendimento) e da linha do tempo delas.
    pub fn fichas(&self) -> Fichas<'_> {
        Fichas(self)
    }

    /// Repositório de mensagens.
    /// Fluxos da organização e a permissão de cada agente (FX-08).
    pub fn fluxos(&self) -> Fluxos<'_> {
        Fluxos(self)
    }

    pub fn mensagens(&self) -> Mensagens<'_> {
        Mensagens(self)
    }

    /// Repositório da caixa de entrada.
    pub fn caixa_entrada(&self) -> CaixaEntrada<'_> {
        CaixaEntrada(self)
    }

    /// Repositório da caixa de saída.
    pub fn caixa_saida(&self) -> CaixaSaida<'_> {
        CaixaSaida(self)
    }

    /// Repositório de turnos.
    pub fn turnos(&self) -> Turnos<'_> {
        Turnos(self)
    }

    /// Repositório de auditoria.
    pub fn auditoria(&self) -> Auditoria<'_> {
        Auditoria(self)
    }

    /// Repositório da retenção (varredura do histórico vencido, HR-51).
    pub fn retencao(&self) -> Retencao<'_> {
        Retencao(self)
    }

    /// Repositório do carrinho de orçamento por conversa (HR-19).
    pub fn carrinhos(&self) -> Carrinhos<'_> {
        Carrinhos(self)
    }

    /// Tabelas do módulo Matrix (MX-08): org ligada, salas, espaços, cursores e falhas.
    pub fn matrix(&self) -> Matrix<'_> {
        Matrix(self)
    }

    /// Mídia publicada nas salas do módulo Matrix e a expiração dela (MX-46).
    pub fn matrix_midia(&self) -> MatrixMidia<'_> {
        MatrixMidia(self)
    }

    /// Biblioteca de arquivos do agente (HR-24, D13).
    pub fn biblioteca(&self) -> Biblioteca<'_> {
        Biblioteca(self)
    }
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("harness", &self.harness)
            .field("rag", &self.rag)
            .finish_non_exhaustive()
    }
}

fn criar_pasta(arquivo: &Path) -> Resultado<()> {
    let Some(pasta) = arquivo.parent() else {
        return Ok(());
    };
    if pasta.as_os_str().is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(pasta).map_err(|erro| ErroStore::Abertura {
        caminho: pasta.display().to_string(),
        motivo: erro.to_string(),
    })
}

/// Abre o arquivo, aplica as migrações pendentes e fecha.
fn prepare(
    caminho: &Path,
    banco: &'static str,
    migracoes: &[migracoes::Migracao],
    agora: i64,
) -> Resultado<RelatorioMigracao> {
    let conn = conexao::abrir_escrita(caminho)?;
    migracoes::migrar(&conn, banco, caminho, migracoes, agora)
}

mod integracoes_admin;
pub use integracoes_admin::{AlteracaoIntegracao, IntegracaoAdmin};
