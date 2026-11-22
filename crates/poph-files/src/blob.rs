//! Blobs: o conteúdo do arquivo guardado em `<pasta_blobs>/<org>/<sha256>`, deduplicado por hash.
//!
//! O nome do arquivo no disco é só o hash (64 caracteres hexadecimais): o nome que o cliente
//! mandou nunca entra num caminho, então não há como escapar da pasta da org.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use poph_types::OrgId;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::erro::ErroArquivo;

/// Resultado da gravação de um blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobGravado {
    /// SHA-256 do conteúdo, em hexadecimal minúsculo.
    pub sha256: String,
    /// Caminho do blob (dentro da pasta da org).
    pub caminho: PathBuf,
    /// `true` se o blob já estava no disco e não foi regravado.
    pub ja_existia: bool,
}

/// SHA-256 do conteúdo em hexadecimal minúsculo.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Grava o conteúdo como blob da org, ou reaproveita o que já existe com o mesmo hash.
///
/// A gravação é atômica: escreve num arquivo temporário da mesma pasta e só então renomeia,
/// então uma queda no meio não deixa um blob pela metade com o nome final.
pub fn gravar(pasta_blobs: &Path, org: OrgId, bytes: &[u8]) -> Result<BlobGravado, ErroArquivo> {
    let sha256 = sha256_hex(bytes);
    let pasta_org = pasta_blobs.join(org.to_string());
    let destino = pasta_org.join(&sha256);

    // Já existe com o tamanho certo: é o mesmo conteúdo (o hash confere), não regrava.
    if let Ok(metadados) = fs::metadata(&destino)
        && metadados.is_file()
        && metadados.len() == bytes.len() as u64
    {
        return Ok(BlobGravado {
            sha256,
            caminho: destino,
            ja_existia: true,
        });
    }

    fs::create_dir_all(&pasta_org).map_err(|erro| {
        ErroArquivo::Gravacao(format!("não consegui criar a pasta ({})", erro.kind()))
    })?;

    let temporario = pasta_org.join(format!(".{sha256}.{}.tmp", Uuid::now_v7().simple()));
    let resultado = escrever_e_renomear(&temporario, &destino, bytes);
    if let Err(erro) = resultado {
        let _ = fs::remove_file(&temporario);
        return Err(ErroArquivo::Gravacao(format!("{}", erro.kind())));
    }

    Ok(BlobGravado {
        sha256,
        caminho: destino,
        ja_existia: false,
    })
}

/// Lê o conteúdo de um blob da org (HR-23b).
///
/// `sha256` é o nome do arquivo no disco: só 64 caracteres hexadecimais minúsculos são aceitos, e
/// o valor é recusado **antes** de tocar a pasta. Assim um hash vindo do banco não tem como virar
/// caminho para fora da pasta da org. `limite_bytes` evita carregar na memória um blob maior do que
/// o turno aceita.
///
/// A mensagem de erro não traz o caminho nem o conteúdo do arquivo.
pub fn ler(
    pasta_blobs: &Path,
    org: OrgId,
    sha256: &str,
    limite_bytes: u64,
) -> Result<Vec<u8>, ErroArquivo> {
    if sha256.len() != 64
        || !sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ErroArquivo::Leitura(
            "o hash do arquivo gravado é inválido".to_owned(),
        ));
    }
    let caminho = pasta_blobs.join(org.to_string()).join(sha256);
    let metadados = fs::metadata(&caminho)
        .map_err(|_| ErroArquivo::Leitura("o arquivo gravado não está no disco".to_owned()))?;
    if !metadados.is_file() {
        return Err(ErroArquivo::Leitura(
            "o arquivo gravado não está no disco".to_owned(),
        ));
    }
    if metadados.len() > limite_bytes {
        return Err(ErroArquivo::Leitura(
            "o arquivo gravado passa do limite de leitura do turno".to_owned(),
        ));
    }
    fs::read(&caminho).map_err(|erro| {
        ErroArquivo::Leitura(format!("não consegui ler o arquivo ({})", erro.kind()))
    })
}

fn escrever_e_renomear(temporario: &Path, destino: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut arquivo = fs::File::create_new(temporario)?;
    arquivo.write_all(bytes)?;
    arquivo.sync_all()?;
    drop(arquivo);
    fs::rename(temporario, destino)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn sha256_conhecido() {
        // SHA-256 de "abc" (vetor público do FIPS 180-2).
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn ler_devolve_o_conteudo_gravado() {
        let pasta = std::env::temp_dir().join(format!("poph-blob-{}", Uuid::now_v7().simple()));
        let org = OrgId::generate();
        let gravado = gravar(&pasta, org, b"conteudo-da-imagem").expect("grava o blob");
        let lido = ler(&pasta, org, &gravado.sha256, 1024).expect("lê o blob");
        assert_eq!(lido, b"conteudo-da-imagem");
        let _ = fs::remove_dir_all(&pasta);
    }

    #[test]
    fn ler_recusa_hash_que_nao_e_sha256() {
        let pasta = std::env::temp_dir();
        let org = OrgId::generate();
        for hash in ["../../etc/passwd", "SHA-EM-MAIUSCULA", "abc", ""] {
            let erro = ler(&pasta, org, hash, 1024).expect_err("hash inválido é recusado");
            assert!(matches!(erro, ErroArquivo::Leitura(_)), "{hash}: {erro:?}");
        }
    }

    #[test]
    fn ler_recusa_blob_ausente_e_acima_do_limite() {
        let pasta = std::env::temp_dir().join(format!("poph-blob-{}", Uuid::now_v7().simple()));
        let org = OrgId::generate();
        let gravado = gravar(&pasta, org, b"0123456789").expect("grava o blob");
        let ausente = ler(&pasta, OrgId::generate(), &gravado.sha256, 1024);
        assert!(matches!(ausente, Err(ErroArquivo::Leitura(_))));
        let grande = ler(&pasta, org, &gravado.sha256, 4);
        assert!(matches!(grande, Err(ErroArquivo::Leitura(_))));
        let _ = fs::remove_dir_all(&pasta);
    }
}
