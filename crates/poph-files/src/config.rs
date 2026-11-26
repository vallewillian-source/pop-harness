use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::FileError;

pub const DEFAULT_FILE_LIMIT: u64 = 16 * 1024 * 1024;
pub const DEFAULT_TEXT_LIMIT: usize = 20_000;
pub const DEFAULT_EXTRACTION_TIME: Duration = Duration::from_secs(5);
pub const DEFAULT_EXTRACTION_MEMORY: u64 = 512 * 1024 * 1024;

const MIN_MEMORY: u64 = 128 * 1024 * 1024;
const MAX_FILE_LIMIT: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct FileConfig {
    pub blob_folder: PathBuf,
    pub byte_limit: u64,
    pub character_limit: usize,
    pub extraction_time: Duration,
    pub extraction_memory_bytes: u64,
    pub extractor_path: Option<PathBuf>,
}

impl FileConfig {
    pub fn default(blob_folder: impl AsRef<Path>) -> Self {
        Self {
            blob_folder: blob_folder.as_ref().to_path_buf(),
            byte_limit: DEFAULT_FILE_LIMIT,
            character_limit: DEFAULT_TEXT_LIMIT,
            extraction_time: DEFAULT_EXTRACTION_TIME,
            extraction_memory_bytes: DEFAULT_EXTRACTION_MEMORY,
            extractor_path: None,
        }
    }

    pub fn validar(&self) -> Result<(), FileError> {
        if self.byte_limit == 0 || self.byte_limit > MAX_FILE_LIMIT {
            return Err(FileError::Configuracao(format!(
                "o limite de arquivo precisa estar entre 1 byte e {} MB",
                MAX_FILE_LIMIT / (1024 * 1024)
            )));
        }
        if self.character_limit == 0 {
            return Err(FileError::Configuracao(
                "o limite de caracteres do texto extraído precisa ser maior que zero".to_owned(),
            ));
        }
        if self.extraction_time.is_zero() {
            return Err(FileError::Configuracao(
                "o tempo máximo de leitura precisa ser maior que zero".to_owned(),
            ));
        }
        if self.extraction_memory_bytes < MIN_MEMORY {
            return Err(FileError::Configuracao(format!(
                "o teto de memória do leitor precisa ser de pelo menos {} MB",
                MIN_MEMORY / (1024 * 1024)
            )));
        }
        Ok(())
    }

    pub fn effective_extractor_path(&self) -> Result<PathBuf, FileError> {
        if let Some(path) = &self.extractor_path {
            return Ok(path.clone());
        }
        let executavel = std::env::current_exe().map_err(|error| {
            FileError::Extrator(format!("não sei onde estou ({})", error.kind()))
        })?;
        let folder = executavel.parent().ok_or_else(|| {
            FileError::Extrator("o executável não tem pasta conhecida".to_owned())
        })?;
        Ok(folder.join("poph-extrator"))
    }
}
