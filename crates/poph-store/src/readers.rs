use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;

use crate::connection::open_for_read;
use crate::error::{Resultado, StoreError};

const MAX_READERS: usize = 8;

pub(crate) struct ReaderPool {
    path: PathBuf,
    free: Mutex<Vec<Connection>>,
}

impl ReaderPool {
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            free: Mutex::new(Vec::new()),
        }
    }

    pub fn run<T>(&self, query: impl FnOnce(&Connection) -> Resultado<T>) -> Resultado<T> {
        let mut connection = {
            let mut free = self.free.lock().unwrap_or_else(|e| e.into_inner());
            free.pop()
        };
        let connection = match connection.take() {
            Some(connection) => connection,
            None => open_for_read(&self.path)?,
        };
        let result = query(&connection);
        let mut free = self.free.lock().unwrap_or_else(|e| e.into_inner());
        if free.len() < MAX_READERS {
            free.push(connection);
        }
        result
    }
}

impl std::fmt::Debug for ReaderPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let quantity = self.free.lock().map(|free| free.len()).unwrap_or(0);
        f.debug_struct("ReaderPool")
            .field("caminho", &self.path)
            .field("livres", &quantity)
            .finish()
    }
}

pub(crate) fn require<T>(value: Option<T>, entity: &'static str, id: &str) -> Resultado<T> {
    value.ok_or_else(|| {
        StoreError::Domain(poph_types::DomainError::NotFound {
            entity,
            id: id.to_owned(),
        })
    })
}
