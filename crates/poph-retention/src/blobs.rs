use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::error::RetentionError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orphan {
    pub nome: String,
    pub path: PathBuf,
}

pub fn find_orphans(
    folder: &Path,
    referenced: &HashSet<String>,
    cutoff_age_ms: i64,
) -> Result<Vec<Orphan>, RetentionError> {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(file_error(folder, error)),
    };
    let mut orphans = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| file_error(folder, error))?;
        let path = entry.path();
        let Ok(metadados) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadados.file_type().is_file() {
            continue;
        }
        let Some(nome) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let is_temporary = nome.ends_with(".tmp");
        let is_blob = is_sha256(&nome);
        if !is_temporary && !is_blob {
            continue;
        }
        if is_blob && referenced.contains(&nome) {
            continue;
        }
        let Some(modified) = modified_at_ms(&metadados) else {
            continue;
        };
        if modified >= cutoff_age_ms {
            continue;
        }
        orphans.push(Orphan { nome, path });
    }
    orphans.sort_by(|a, b| a.nome.cmp(&b.nome));
    Ok(orphans)
}

pub fn remove(orphan: &Orphan) -> Result<(), RetentionError> {
    fs::remove_file(&orphan.path).map_err(|error| file_error(&orphan.path, error))
}

pub fn is_sha256(nome: &str) -> bool {
    nome.len() == 64
        && nome
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn modified_at_ms(metadados: &fs::Metadata) -> Option<i64> {
    let instant = metadados.modified().ok()?;
    let since_epoch = instant.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(since_epoch.as_millis()).ok()
}

fn file_error(path: &Path, error: std::io::Error) -> RetentionError {
    RetentionError::Files {
        path: path.display().to_string(),
        motivo: error.to_string(),
    }
}

#[cfg(test)]
pub(crate) fn system_time_of(ms: i64) -> std::time::SystemTime {
    UNIX_EPOCH + std::time::Duration::from_millis(u64::try_from(ms).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_A: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
    const SHA_B: &str = "0f0e0d0c0b0a09080706050403020100ffeeddccbbaa99887766554433221100";
    const NOW: i64 = 1_795_131_511_000;
    const OLD: i64 = NOW - 40 * 86_400_000;
    const RECENT: i64 = NOW - 60_000;

    fn write_blob(folder: &Path, nome: &str, modified_ms: i64) -> PathBuf {
        let path = folder.join(nome);
        fs::write(&path, b"conteudo").unwrap();
        let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_modified(system_time_of(modified_ms)).unwrap();
        path
    }

    fn names_of(orphans: &[Orphan]) -> Vec<&str> {
        orphans.iter().map(|o| o.nome.as_str()).collect()
    }

    #[test]
    fn only_deletes_unowned_blob_outside_grace() {
        let folder = tempfile::tempdir().unwrap();
        write_blob(folder.path(), SHA_A, OLD);
        write_blob(folder.path(), SHA_B, OLD);
        let grace = NOW - 3_600_000;
        let refs: HashSet<String> = [SHA_B.to_owned()].into();
        let orphans = find_orphans(folder.path(), &refs, grace).unwrap();
        assert_eq!(names_of(&orphans), [SHA_A]);
    }

    #[test]
    fn recent_blob_stays_even_without_owner() {
        let folder = tempfile::tempdir().unwrap();
        write_blob(folder.path(), SHA_A, RECENT);
        let orphans = find_orphans(folder.path(), &HashSet::new(), NOW - 3_600_000).unwrap();
        assert!(
            orphans.is_empty(),
            "blob recente não pode sair: {orphans:?}"
        );
    }

    #[test]
    fn abandoned_temp_goes_and_recent_temp_stays() {
        let folder = tempfile::tempdir().unwrap();
        write_blob(folder.path(), ".abc.123.tmp", OLD);
        write_blob(folder.path(), ".def.456.tmp", RECENT);
        let orphans = find_orphans(folder.path(), &HashSet::new(), NOW - 3_600_000).unwrap();
        assert_eq!(names_of(&orphans), [".abc.123.tmp"]);
    }

    #[test]
    fn non_blob_name_never_enters() {
        let folder = tempfile::tempdir().unwrap();
        write_blob(folder.path(), "anotacoes.txt", OLD);
        write_blob(folder.path(), "SHA_MAIUSCULO_NAO_E_BLOB", OLD);
        fs::create_dir(folder.path().join(SHA_A)).unwrap();
        let orphans = find_orphans(folder.path(), &HashSet::new(), NOW).unwrap();
        assert!(orphans.is_empty(), "{orphans:?}");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_never_deleted() {
        let outside = tempfile::tempdir().unwrap();
        let target = write_blob(outside.path(), "alvo.bin", OLD);
        let folder = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(&target, folder.path().join(SHA_A)).unwrap();
        let orphans = find_orphans(folder.path(), &HashSet::new(), NOW).unwrap();
        assert!(orphans.is_empty(), "link não é blob: {orphans:?}");
        assert!(target.exists());
    }

    #[test]
    fn missing_folder_has_no_orphans() {
        let folder = tempfile::tempdir().unwrap();
        let absent = folder.path().join("nao-existe");
        assert!(
            find_orphans(&absent, &HashSet::new(), NOW)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn remove_deletes_only_the_given_file() {
        let folder = tempfile::tempdir().unwrap();
        let path = write_blob(folder.path(), SHA_A, OLD);
        let other = write_blob(folder.path(), SHA_B, OLD);
        let orphan = Orphan {
            nome: SHA_A.to_owned(),
            path: path.clone(),
        };
        remove(&orphan).unwrap();
        assert!(!path.exists());
        assert!(other.exists());
    }
}
