use std::fs;
use std::path::{Path, PathBuf};

use poph_rag::installation::{file_sha256, verify_file};
use poph_rag::{ModelConfig, OrtEmbedder, RagError};

struct TempFolder(PathBuf);

impl TempFolder {
    fn new(nome: &str) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "poph-rag-teste-{nome}-{}-{now}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("cria a pasta temporária");
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

#[test]
fn wrong_checksum_blocks_model_startup_with_clear_error() {
    let folder = TempFolder::new("checksum");
    fs::write(folder.path().join("model_int8.onnx"), b"nao sou o modelo").expect("escreve");
    fs::write(folder.path().join("tokenizer.json"), b"nem o tokenizador").expect("escreve");

    let error = OrtEmbedder::open_model(
        &ModelConfig {
            diretorio: folder.path().to_path_buf(),
            threads: 4,
        },
        None,
    )
    .err()
    .expect("não pode subir com checksum errado");

    match &error {
        RagError::ChecksumMismatch {
            file,
            expected,
            actual,
        } => {
            assert_eq!(file, "model_int8.onnx");
            assert_eq!(
                expected,
                "d6ea442ff6a891daefed7c83b2f596fc5dc66bf697e4d006236f64f34bbcf4c8"
            );
            assert_ne!(actual, expected);
        }
        other => panic!("erro inesperado: {other:?}"),
    }
    let texto = error.to_string();
    assert!(
        texto.contains("poph model install"),
        "a mensagem deve dizer o que fazer: {texto}"
    );
    assert!(
        !texto.contains(&folder.path().display().to_string()),
        "a mensagem não pode trazer o caminho do modelo"
    );
}

#[test]
fn missing_model_suggests_running_install() {
    let folder = TempFolder::new("ausente");

    let error = OrtEmbedder::open_model(
        &ModelConfig {
            diretorio: folder.path().to_path_buf(),
            threads: 4,
        },
        None,
    )
    .err()
    .expect("sem modelo não sobe");

    assert_eq!(
        error,
        RagError::ModelMissing {
            file: "model_int8.onnx".to_owned()
        }
    );
    assert!(error.to_string().contains("poph model install"));
}

#[test]
fn file_check_accepts_correct_sha_and_rejects_incorrect_sha() {
    let folder = TempFolder::new("conferir");
    let file = folder.path().join("abc.txt");
    fs::write(&file, b"abc").expect("escreve");
    let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    assert_eq!(file_sha256(&file).expect("lê"), expected);
    assert_eq!(
        verify_file(folder.path(), "abc.txt", expected).expect("confere"),
        expected
    );

    let error =
        verify_file(folder.path(), "abc.txt", &"0".repeat(64)).expect_err("sha errado é recusado");
    assert!(matches!(error, RagError::ChecksumMismatch { ref file, .. } if file == "abc.txt"));
}
