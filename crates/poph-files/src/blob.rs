use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use poph_types::OrgId;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::FileError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobWritten {
    pub sha256: String,
    pub path: PathBuf,
    pub already_existed: bool,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn write(blob_folder: &Path, org: OrgId, bytes: &[u8]) -> Result<BlobWritten, FileError> {
    let sha256 = sha256_hex(bytes);
    let org_folder = blob_folder.join(org.to_string());
    let destination = org_folder.join(&sha256);

    if let Ok(metadados) = fs::metadata(&destination)
        && metadados.is_file()
        && metadados.len() == bytes.len() as u64
    {
        return Ok(BlobWritten {
            sha256,
            path: destination,
            already_existed: true,
        });
    }

    fs::create_dir_all(&org_folder).map_err(|error| {
        FileError::WriteOutcome(format!("não consegui criar a pasta ({})", error.kind()))
    })?;

    let temporary = org_folder.join(format!(".{sha256}.{}.tmp", Uuid::now_v7().simple()));
    let resultado = write_and_rename(&temporary, &destination, bytes);
    if let Err(error) = resultado {
        let _ = fs::remove_file(&temporary);
        return Err(FileError::WriteOutcome(format!("{}", error.kind())));
    }

    Ok(BlobWritten {
        sha256,
        path: destination,
        already_existed: false,
    })
}

pub fn read(
    blob_folder: &Path,
    org: OrgId,
    sha256: &str,
    byte_limit: u64,
) -> Result<Vec<u8>, FileError> {
    if sha256.len() != 64
        || !sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(FileError::Reading(
            "o hash do arquivo gravado é inválido".to_owned(),
        ));
    }
    let path = blob_folder.join(org.to_string()).join(sha256);
    let metadados = fs::metadata(&path)
        .map_err(|_| FileError::Reading("o arquivo gravado não está no disco".to_owned()))?;
    if !metadados.is_file() {
        return Err(FileError::Reading(
            "o arquivo gravado não está no disco".to_owned(),
        ));
    }
    if metadados.len() > byte_limit {
        return Err(FileError::Reading(
            "o arquivo gravado passa do limite de leitura do turno".to_owned(),
        ));
    }
    fs::read(&path).map_err(|error| {
        FileError::Reading(format!("não consegui ler o arquivo ({})", error.kind()))
    })
}

fn write_and_rename(temporary: &Path, destination: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = fs::File::create_new(temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_conhecido() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn read_returns_stored_content() {
        let folder = std::env::temp_dir().join(format!("poph-blob-{}", Uuid::now_v7().simple()));
        let org = OrgId::generate();
        let stored = write(&folder, org, b"conteudo-da-imagem").expect("grava o blob");
        let loaded = read(&folder, org, &stored.sha256, 1024).expect("lê o blob");
        assert_eq!(loaded, b"conteudo-da-imagem");
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn read_refuses_non_sha256_hash() {
        let folder = std::env::temp_dir();
        let org = OrgId::generate();
        for hash in ["../../etc/passwd", "SHA-EM-MAIUSCULA", "abc", ""] {
            let error = read(&folder, org, hash, 1024).expect_err("hash inválido é recusado");
            assert!(matches!(error, FileError::Reading(_)), "{hash}: {error:?}");
        }
    }

    #[test]
    fn read_refuses_missing_or_oversized_blob() {
        let folder = std::env::temp_dir().join(format!("poph-blob-{}", Uuid::now_v7().simple()));
        let org = OrgId::generate();
        let stored = write(&folder, org, b"0123456789").expect("grava o blob");
        let missing = read(&folder, OrgId::generate(), &stored.sha256, 1024);
        assert!(matches!(missing, Err(FileError::Reading(_))));
        let large = read(&folder, org, &stored.sha256, 4);
        assert!(matches!(large, Err(FileError::Reading(_))));
        let _ = fs::remove_dir_all(&folder);
    }
}
