//! Subida do modelo sem precisar do modelo real: checksum errado, arquivo ausente e a conferência.

use std::fs;
use std::path::{Path, PathBuf};

use poph_rag::installation::{file_sha256, verify_file};
use poph_rag::{ModelConfig, OrtEmbedder, RagError};

/// Pasta temporária única para o teste; apagada ao sair.
struct Pasta(PathBuf);

impl Pasta {
    fn nova(nome: &str) -> Self {
        let agora = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let caminho = std::env::temp_dir().join(format!(
            "poph-rag-teste-{nome}-{}-{agora}",
            std::process::id()
        ));
        fs::create_dir_all(&caminho).expect("cria a pasta temporária");
        Self(caminho)
    }

    fn caminho(&self) -> &Path {
        &self.0
    }
}

impl Drop for Pasta {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn checksum_errado_impede_a_subida_com_erro_claro() {
    let pasta = Pasta::nova("checksum");
    fs::write(pasta.caminho().join("model_int8.onnx"), b"nao sou o modelo").expect("escreve");
    fs::write(pasta.caminho().join("tokenizer.json"), b"nem o tokenizador").expect("escreve");

    let erro = OrtEmbedder::open_model(
        &ModelConfig {
            diretorio: pasta.caminho().to_path_buf(),
            threads: 4,
        },
        None,
    )
    .err()
    .expect("não pode subir com checksum errado");

    match &erro {
        RagError::ChecksumMismatch {
            file: arquivo,
            expected: esperado,
            actual: obtido,
        } => {
            assert_eq!(arquivo, "model_int8.onnx");
            assert_eq!(
                esperado,
                "d6ea442ff6a891daefed7c83b2f596fc5dc66bf697e4d006236f64f34bbcf4c8"
            );
            assert_ne!(obtido, esperado);
        }
        outro => panic!("erro inesperado: {outro:?}"),
    }
    let texto = erro.to_string();
    assert!(
        texto.contains("poph model install"),
        "a mensagem deve dizer o que fazer: {texto}"
    );
    assert!(
        !texto.contains(&pasta.caminho().display().to_string()),
        "a mensagem não pode trazer o caminho do modelo"
    );
}

#[test]
fn modelo_ausente_manda_rodar_o_install() {
    let pasta = Pasta::nova("ausente");

    let erro = OrtEmbedder::open_model(
        &ModelConfig {
            diretorio: pasta.caminho().to_path_buf(),
            threads: 4,
        },
        None,
    )
    .err()
    .expect("sem modelo não sobe");

    assert_eq!(
        erro,
        RagError::ModelMissing {
            file: "model_int8.onnx".to_owned()
        }
    );
    assert!(erro.to_string().contains("poph model install"));
}

#[test]
fn conferir_arquivo_aceita_o_sha_certo_e_recusa_o_errado() {
    let pasta = Pasta::nova("conferir");
    let arquivo = pasta.caminho().join("abc.txt");
    fs::write(&arquivo, b"abc").expect("escreve");
    // sha256("abc"), vetor de teste publicado (FIPS 180-2).
    let esperado = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    assert_eq!(file_sha256(&arquivo).expect("lê"), esperado);
    assert_eq!(
        verify_file(pasta.caminho(), "abc.txt", esperado).expect("confere"),
        esperado
    );

    let erro = verify_file(pasta.caminho(), "abc.txt", &"0".repeat(64))
        .expect_err("sha errado é recusado");
    assert!(matches!(erro, RagError::ChecksumMismatch { ref file, .. } if file == "abc.txt"));
}
