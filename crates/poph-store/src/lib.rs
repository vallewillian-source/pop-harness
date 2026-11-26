pub mod backup;
pub mod backup_document;
mod connection;
pub mod error;
pub mod metrics;
pub mod migrations;
pub mod model;
mod readers;
mod repos;
mod writer;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use poph_types::{ExecutionStatus, HarnessConfig};
use rusqlite::Connection;

pub use error::StoreError;
pub use migrations::{
    AppliedMigration, HARNESS_MIGRATIONS, MigrationReport, RAG_MIGRATIONS, file_version,
    harness_target_version,
};
pub use model::*;
pub use repos::*;

pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

pub type Resultado<T> = Result<T, StoreError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReports {
    pub harness: MigrationReport,
    pub rag: MigrationReport,
}

impl MigrationReports {
    pub fn already_up_to_date(&self) -> bool {
        self.harness.already_up_to_date() && self.rag.already_up_to_date()
    }
}

pub fn migrate_databases(harness: &Path, rag: &Path) -> Resultado<MigrationReports> {
    create_folder(harness)?;
    create_folder(rag)?;
    let now = system_now();
    Ok(MigrationReports {
        harness: prepare(harness, "harness.db", HARNESS_MIGRATIONS, now)?,
        rag: prepare(rag, "rag.db", RAG_MIGRATIONS, now)?,
    })
}

pub fn system_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

pub struct Store {
    harness: PathBuf,
    rag: PathBuf,
    clock: Clock,
    writer: writer::Writer,
    readers: readers::ReaderPool,
    reads: AtomicU64,
    execution_metrics: RwLock<Option<metrics::ExecutionMetrics>>,
}

impl Store {
    pub fn open(config: &HarnessConfig) -> Resultado<Self> {
        let diretorio = &config.data.directory;
        std::fs::create_dir_all(diretorio).map_err(|error| StoreError::OpenFailed {
            path: diretorio.display().to_string(),
            reason: error.to_string(),
        })?;
        Self::open_at(&config.harness_db(), &config.rag_db())
    }

    pub fn open_at(harness: &Path, rag: &Path) -> Resultado<Self> {
        create_folder(harness)?;
        create_folder(rag)?;
        let clock: Clock = Arc::new(system_now);
        prepare(harness, "harness.db", HARNESS_MIGRATIONS, clock())?;
        prepare(rag, "rag.db", RAG_MIGRATIONS, clock())?;
        let writer = writer::Writer::start(harness.to_path_buf())?;
        Ok(Self {
            harness: harness.to_path_buf(),
            rag: rag.to_path_buf(),
            clock,
            readers: readers::ReaderPool::new(harness),
            writer,
            reads: AtomicU64::new(0),
            execution_metrics: RwLock::new(None),
        })
    }

    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    pub fn now(&self) -> i64 {
        (self.clock)()
    }

    pub fn publish_execution_metrics(&self, metrics: metrics::ExecutionMetrics) {
        let mut guard = self
            .execution_metrics
            .write()
            .unwrap_or_else(|poison| poison.into_inner());
        *guard = Some(metrics);
    }

    pub(crate) fn execution_metrics(&self) -> Option<metrics::ExecutionMetrics> {
        self.execution_metrics
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    pub(crate) fn count_opened_execution(&self) {
        if let Some(metrics) = self.execution_metrics() {
            metrics.opened(1);
        }
    }

    pub(crate) fn count_execution_outcome(&self, status: ExecutionStatus) {
        if let Some(metrics) = self.execution_metrics() {
            metrics.desfecho(status, 1);
        }
    }

    pub(crate) fn count_rule_error(&self) {
        if let Some(metrics) = self.execution_metrics() {
            metrics.rule_failed(1);
        }
    }

    pub fn harness_path(&self) -> &Path {
        &self.harness
    }

    pub fn rag_path(&self) -> &Path {
        &self.rag
    }

    pub fn migrate(&self) -> Resultado<MigrationReports> {
        let now = self.now();
        let harness_path = self.harness.clone();
        let harness = self.write(move |conn| {
            migrations::migrate(conn, "harness.db", &harness_path, HARNESS_MIGRATIONS, now)
        })?;
        let rag = prepare(&self.rag, "rag.db", RAG_MIGRATIONS, now)?;
        Ok(MigrationReports { harness, rag })
    }

    pub fn broken_foreign_keys(&self) -> Resultado<Vec<String>> {
        self.read(|conn| {
            let mut query = conn.prepare("PRAGMA foreign_key_check")?;
            let rows = query.query_map([], |row| {
                let table: String = row.get(0)?;
                let row_id: i64 = row.get(1).unwrap_or(0);
                let parent: String = row.get(2)?;
                Ok(format!("{table}#{row_id} → {parent}"))
            })?;
            let mut quebradas = Vec::new();
            for row in rows {
                quebradas.push(row?);
            }
            Ok(quebradas)
        })
    }

    pub fn tables(&self) -> Resultado<Vec<String>> {
        self.read(|conn| {
            let mut query = conn.prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )?;
            let rows = query.query_map([], |row| row.get::<_, String>(0))?;
            let mut names = Vec::new();
            for row in rows {
                names.push(row?);
            }
            Ok(names)
        })
    }

    pub fn columns(&self, table: &str) -> Resultado<Vec<String>> {
        self.read(|conn| {
            let mut query = conn.prepare(&format!("PRAGMA table_info({table})"))?;
            let rows = query.query_map([], |row| row.get::<_, String>(1))?;
            let mut names = Vec::new();
            for row in rows {
                names.push(row?);
            }
            Ok(names)
        })
    }

    pub(crate) fn write<T: Send + 'static>(
        &self,
        task: impl FnOnce(&Connection) -> Resultado<T> + Send + 'static,
    ) -> Resultado<T> {
        self.writer.run(task)
    }

    pub(crate) fn read<T>(&self, query: impl FnOnce(&Connection) -> Resultado<T>) -> Resultado<T> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.readers.com(query)
    }

    pub fn reads_done(&self) -> u64 {
        self.reads.load(Ordering::Relaxed)
    }

    pub fn orgs(&self) -> Orgaos<'_> {
        Orgaos(self)
    }

    pub fn tokens(&self) -> Tokens<'_> {
        Tokens(self)
    }

    pub fn knowledge(&self) -> Conhecimento<'_> {
        Conhecimento(self)
    }

    pub fn categories(&self) -> Categorias<'_> {
        Categorias(self)
    }

    pub fn products(&self) -> Produtos<'_> {
        Produtos(self)
    }

    pub fn departments(&self) -> Departamentos<'_> {
        Departamentos(self)
    }

    pub fn agents(&self) -> Agentes<'_> {
        Agentes(self)
    }

    pub fn instructions(&self) -> InstrucoesRepo<'_> {
        InstrucoesRepo(self)
    }

    pub fn conversas(&self) -> Conversas<'_> {
        Conversas(self)
    }

    pub fn fichas(&self) -> Fichas<'_> {
        Fichas(self)
    }

    pub fn sla_analytics(&self) -> SlaAnalytics<'_> {
        SlaAnalytics(self)
    }

    pub fn conversation_analytics(&self) -> ConversationAnalytics<'_> {
        ConversationAnalytics(self)
    }

    pub fn usage(&self) -> Usage<'_> {
        Usage(self)
    }

    pub fn analytics(&self) -> Analytics<'_> {
        Analytics(self)
    }

    pub fn fluxos(&self) -> Fluxos<'_> {
        Fluxos(self)
    }

    pub fn revisions(&self) -> Revisoes<'_> {
        Revisoes(self)
    }

    pub fn executions(&self) -> Execucoes<'_> {
        Execucoes(self)
    }

    pub fn effects(&self) -> Efeitos<'_> {
        Efeitos(self)
    }

    pub fn messages(&self) -> Mensagens<'_> {
        Mensagens(self)
    }

    pub fn inbox(&self) -> CaixaEntrada<'_> {
        CaixaEntrada(self)
    }

    pub fn outbox(&self) -> CaixaSaida<'_> {
        CaixaSaida(self)
    }

    pub fn turns(&self) -> Turnos<'_> {
        Turnos(self)
    }

    pub fn audit(&self) -> Auditoria<'_> {
        Auditoria(self)
    }

    pub fn retention(&self) -> Retencao<'_> {
        Retencao(self)
    }

    pub fn carts(&self) -> Carrinhos<'_> {
        Carrinhos(self)
    }

    pub fn matrix(&self) -> Matrix<'_> {
        Matrix(self)
    }

    pub fn matrix_midia(&self) -> MatrixMidia<'_> {
        MatrixMidia(self)
    }

    pub fn library(&self) -> Biblioteca<'_> {
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

fn create_folder(file: &Path) -> Resultado<()> {
    let Some(folder) = file.parent() else {
        return Ok(());
    };
    if folder.as_os_str().is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(folder).map_err(|error| StoreError::OpenFailed {
        path: folder.display().to_string(),
        reason: error.to_string(),
    })
}

fn prepare(
    path: &Path,
    database: &'static str,
    migrations: &[migrations::Migration],
    now: i64,
) -> Resultado<MigrationReport> {
    let conn = connection::open_for_write(path)?;
    migrations::migrate(&conn, database, path, migrations, now)
}

mod admin_integrations;
pub use admin_integrations::{AdminIntegration, IntegrationChange};
