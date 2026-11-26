use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;

use poph_rag::RagError;
use poph_rag::installation::{InstallStatus, ModelFile, install_files};
use sha2::{Digest, Sha256};

struct TempFolder(PathBuf);

impl TempFolder {
    fn new(nome: &str) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "poph-instalacao-{nome}-{}-{now}",
            std::process::id()
        ));
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn sha_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn serve(responses: Vec<(u16, Vec<u8>)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("porta local");
    let origin = format!("http://{}", listener.local_addr().expect("endereço"));
    thread::spawn(move || {
        for (status, body) in responses {
            let Ok((mut connection, _)) = listener.accept() else {
                return;
            };
            let mut request = [0u8; 4096];
            let _ = connection.read(&mut request);
            let header = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = connection.write_all(header.as_bytes());
            let _ = connection.write_all(&body);
        }
    });
    origin
}

fn file(nome: &str, origin: &str, sha: &str) -> ModelFile {
    ModelFile {
        name: nome.to_owned(),
        url: format!("{origin}/{nome}"),
        sha256: sha.to_owned(),
    }
}

#[test]
fn downloads_verifies_publishes_then_reuses_file() {
    let folder = TempFolder::new("ok");
    let body = b"modelo de teste, conteudo fixo".to_vec();
    let origin = serve(vec![(200, body.clone())]);
    let items = [file("modelo.bin", &origin, &sha_hex(&body))];

    let first = install_files(folder.path(), &items).expect("instala");
    assert_eq!(first.files.len(), 1);
    assert_eq!(first.files[0].status, InstallStatus::Downloaded);
    assert_eq!(first.files[0].bytes, body.len() as u64);
    assert_eq!(
        fs::read(folder.path().join("modelo.bin")).expect("lê"),
        body
    );
    assert!(
        !folder.path().join(".modelo.bin.parcial").exists(),
        "temporário deve sumir"
    );

    let second = install_files(folder.path(), &items).expect("segunda vez");
    assert_eq!(second.files[0].status, InstallStatus::AlreadyInstalled);
}

#[test]
fn wrong_sha_prevents_publishing_file() {
    let folder = TempFolder::new("sha-errado");
    let body = b"conteudo que nao confere".to_vec();
    let origin = serve(vec![(200, body.clone())]);
    let expected = sha_hex(b"outro conteudo");
    let items = [file("modelo.bin", &origin, &expected)];

    let error = install_files(folder.path(), &items).expect_err("hash errado não pode ser aceito");

    match &error {
        RagError::ChecksumMismatch {
            file,
            expected: e,
            actual,
        } => {
            assert_eq!(file, "modelo.bin");
            assert_eq!(e, &expected);
            assert_eq!(actual, &sha_hex(&body));
        }
        other => panic!("erro inesperado: {other:?}"),
    }
    assert!(error.to_string().contains("não confere"), "{error}");
    assert!(
        !folder.path().join("modelo.bin").exists(),
        "arquivo com hash errado não pode ser publicado"
    );
    assert!(
        !folder.path().join(".modelo.bin.parcial").exists(),
        "temporário deve ser removido"
    );
}

#[test]
fn existing_file_with_wrong_content_is_replaced() {
    let folder = TempFolder::new("substitui");
    fs::create_dir_all(folder.path()).expect("pasta");
    fs::write(folder.path().join("modelo.bin"), b"lixo antigo").expect("escreve");
    let body = b"versao correta".to_vec();
    let origin = serve(vec![(200, body.clone())]);
    let items = [file("modelo.bin", &origin, &sha_hex(&body))];

    let report = install_files(folder.path(), &items).expect("substitui");

    assert_eq!(report.files[0].status, InstallStatus::Downloaded);
    assert_eq!(
        fs::read(folder.path().join("modelo.bin")).expect("lê"),
        body
    );
}

#[test]
fn server_error_response_reports_clear_message() {
    let folder = TempFolder::new("404");
    let origin = serve(vec![(404, b"nao achei".to_vec())]);
    let items = [file("modelo.bin", &origin, &sha_hex(b"qualquer"))];

    let error = install_files(folder.path(), &items).expect_err("404 é erro");

    match &error {
        RagError::Download {
            file,
            reason: motivo,
        } => {
            assert_eq!(file, "modelo.bin");
            assert!(motivo.contains("404"), "{motivo}");
        }
        other => panic!("erro inesperado: {other:?}"),
    }
    assert!(!folder.path().join(".modelo.bin.parcial").exists());
}
