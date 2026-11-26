use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::error::RagError;

pub const MODEL_FILE: &str = "model_int8.onnx";
pub const TOKENIZER_FILE: &str = "tokenizer.json";
pub const MODEL_SHA256: &str = "d6ea442ff6a891daefed7c83b2f596fc5dc66bf697e4d006236f64f34bbcf4c8";
pub const TOKENIZER_SHA256: &str =
    "2c3387be76557bd40970cec13153b3bbf80407865484b209e655e5e4729076b8";
pub const MODEL_URL: &str = "https://huggingface.co/Xenova/paraphrase-multilingual-MiniLM-L12-v2/resolve/2c4055b12046f11709e9df2c122e59ffbdc2f900/onnx/model_int8.onnx";
pub const TOKENIZER_URL: &str = "https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2/resolve/e8f8c211226b894fcb81acc59f3b34ba3efd5f42/tokenizer.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFile {
    pub name: String,
    pub url: String,
    pub sha256: String,
}

pub fn default_files() -> Vec<ModelFile> {
    vec![
        ModelFile {
            name: MODEL_FILE.to_owned(),
            url: MODEL_URL.to_owned(),
            sha256: MODEL_SHA256.to_owned(),
        },
        ModelFile {
            name: TOKENIZER_FILE.to_owned(),
            url: TOKENIZER_URL.to_owned(),
            sha256: TOKENIZER_SHA256.to_owned(),
        },
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallStatus {
    Downloaded,
    AlreadyInstalled,
}

impl InstallStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            InstallStatus::Downloaded => "baixado",
            InstallStatus::AlreadyInstalled => "ja_estava_instalado",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledFile {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
    pub status: InstallStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport {
    pub directory: PathBuf,
    pub files: Vec<InstalledFile>,
}

pub fn install(directory: &Path) -> Result<InstallReport, RagError> {
    install_files(directory, &default_files())
}

pub fn install_files(directory: &Path, files: &[ModelFile]) -> Result<InstallReport, RagError> {
    fs::create_dir_all(directory).map_err(|error| {
        RagError::File(format!("não consegui criar a pasta de modelos: {error}"))
    })?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(1800))
        .user_agent("poph-model-install")
        .build()
        .map_err(|error| RagError::Download {
            file: "cliente HTTP".to_owned(),
            reason: error.to_string(),
        })?;
    let mut entries = Vec::with_capacity(files.len());
    for file in files {
        entries.push(ensure_file(directory, file, &client)?);
    }
    Ok(InstallReport {
        directory: directory.to_path_buf(),
        files: entries,
    })
}

fn ensure_file(
    directory: &Path,
    file: &ModelFile,
    client: &reqwest::blocking::Client,
) -> Result<InstalledFile, RagError> {
    let destination = directory.join(&file.name);
    if destination.is_file() {
        let actual = file_sha256(&destination)
            .map_err(|error| RagError::File(format!("não consegui ler {}: {error}", file.name)))?;
        if actual == file.sha256 {
            let bytes = fs::metadata(&destination).map(|m| m.len()).unwrap_or(0);
            return Ok(InstalledFile {
                name: file.name.clone(),
                bytes,
                sha256: actual,
                status: InstallStatus::AlreadyInstalled,
            });
        }
    }

    let partial = directory.join(format!(".{}.parcial", file.name));
    let (actual, bytes) = match download(client, &file.url, &partial, &file.name) {
        Ok(result) => result,
        Err(error) => {
            let _ = fs::remove_file(&partial);
            return Err(error);
        }
    };
    if actual != file.sha256 {
        let _ = fs::remove_file(&partial);
        return Err(RagError::ChecksumMismatch {
            file: file.name.clone(),
            expected: file.sha256.clone(),
            actual,
        });
    }
    fs::rename(&partial, &destination).map_err(|error| {
        let _ = fs::remove_file(&partial);
        RagError::File(format!("não consegui publicar {}: {error}", file.name))
    })?;
    Ok(InstalledFile {
        name: file.name.clone(),
        bytes,
        sha256: actual,
        status: InstallStatus::Downloaded,
    })
}

fn download(
    client: &reqwest::blocking::Client,
    url: &str,
    destination: &Path,
    name: &str,
) -> Result<(String, u64), RagError> {
    let failure = |reason: String| RagError::Download {
        file: name.to_owned(),
        reason,
    };
    let mut response = client
        .get(url)
        .send()
        .map_err(|error| failure(error.to_string()))?;
    if !response.status().is_success() {
        return Err(failure(format!(
            "resposta HTTP {}",
            response.status().as_u16()
        )));
    }
    let mut file = File::create(destination)
        .map_err(|error| RagError::File(format!("não consegui gravar {name}: {error}")))?;
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut block = vec![0u8; 64 * 1024];
    loop {
        let bytes_read = response
            .read(&mut block)
            .map_err(|error| failure(error.to_string()))?;
        if bytes_read == 0 {
            break;
        }
        hash.update(&block[..bytes_read]);
        file.write_all(&block[..bytes_read])
            .map_err(|error| RagError::File(format!("não consegui gravar {name}: {error}")))?;
        bytes += bytes_read as u64;
    }
    file.sync_all()
        .map_err(|error| RagError::File(format!("não consegui gravar {name}: {error}")))?;
    Ok((hex(&hash.finalize()), bytes))
}

pub fn verify_file(directory: &Path, name: &str, expected: &str) -> Result<String, RagError> {
    let path = directory.join(name);
    if !path.is_file() {
        return Err(RagError::ModelMissing {
            file: name.to_owned(),
        });
    }
    let actual = file_sha256(&path)
        .map_err(|error| RagError::File(format!("não consegui ler {name}: {error}")))?;
    if actual != expected {
        return Err(RagError::ChecksumMismatch {
            file: name.to_owned(),
            expected: expected.to_owned(),
            actual,
        });
    }
    Ok(actual)
}

pub fn file_sha256(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut block = vec![0u8; 1024 * 1024];
    loop {
        let bytes_read = file.read(&mut block)?;
        if bytes_read == 0 {
            break;
        }
        hash.update(&block[..bytes_read]);
    }
    Ok(hex(&hash.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}
