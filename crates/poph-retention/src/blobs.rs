//! Blobs órfãos: arquivos em `<pasta de blobs>/<org>/` que nenhum anexo nem arquivo da biblioteca
//! usa mais, e temporários `.tmp` abandonados por queda no meio da gravação.
//!
//! Regras de segurança:
//! - só arquivo comum: pasta e link simbólico nunca são apagados;
//! - só nome de blob (64 caracteres hexadecimais minúsculos, o sha256) ou `.tmp`; o resto fica;
//! - arquivo com idade menor que a carência não entra, para não apagar um blob que outro
//!   pedido acabou de gravar e ainda não registrou no banco.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::erro::ErroRetencao;

/// Arquivo da pasta de blobs que pode ser apagado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orfao {
    /// Nome do arquivo (o sha256, ou o nome do temporário).
    pub nome: String,
    /// Caminho completo.
    pub caminho: PathBuf,
}

/// Lista os órfãos de uma pasta de org. Pasta inexistente = nenhum órfão.
///
/// `referenciados` são os sha256 que ainda têm dono (anexos vivos e biblioteca de arquivos).
/// `corte_idade_ms` é o instante limite: só entra arquivo modificado antes dele.
pub fn encontrar_orfaos(
    pasta: &Path,
    referenciados: &HashSet<String>,
    corte_idade_ms: i64,
) -> Result<Vec<Orfao>, ErroRetencao> {
    let entradas = match fs::read_dir(pasta) {
        Ok(entradas) => entradas,
        Err(erro) if erro.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(erro) => return Err(arquivos(pasta, erro)),
    };
    let mut orfaos = Vec::new();
    for entrada in entradas {
        let entrada = entrada.map_err(|erro| arquivos(pasta, erro))?;
        let caminho = entrada.path();
        // `symlink_metadata` não segue link: um link para fora da pasta nunca é tocado.
        let Ok(metadados) = fs::symlink_metadata(&caminho) else {
            continue;
        };
        if !metadados.file_type().is_file() {
            continue;
        }
        let Some(nome) = entrada.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let e_temporario = nome.ends_with(".tmp");
        let e_blob = e_sha256(&nome);
        if !e_temporario && !e_blob {
            continue;
        }
        if e_blob && referenciados.contains(&nome) {
            continue;
        }
        let Some(modificado) = modificado_em_ms(&metadados) else {
            continue;
        };
        if modificado >= corte_idade_ms {
            continue;
        }
        orfaos.push(Orfao { nome, caminho });
    }
    orfaos.sort_by(|a, b| a.nome.cmp(&b.nome));
    Ok(orfaos)
}

/// Apaga um órfão. Devolve erro de arquivo se o sistema recusar.
pub fn remover(orfao: &Orfao) -> Result<(), ErroRetencao> {
    fs::remove_file(&orfao.caminho).map_err(|erro| arquivos(&orfao.caminho, erro))
}

/// `true` quando o nome tem a forma de um sha256 (64 hexadecimais minúsculos).
pub fn e_sha256(nome: &str) -> bool {
    nome.len() == 64
        && nome
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn modificado_em_ms(metadados: &fs::Metadata) -> Option<i64> {
    let instante = metadados.modified().ok()?;
    let desde_epoca = instante.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(desde_epoca.as_millis()).ok()
}

fn arquivos(caminho: &Path, erro: std::io::Error) -> ErroRetencao {
    ErroRetencao::Arquivos {
        caminho: caminho.display().to_string(),
        motivo: erro.to_string(),
    }
}

/// Usado pelos testes para fixar a idade de um arquivo.
#[cfg(test)]
pub(crate) fn instante_de_sistema(ms: i64) -> std::time::SystemTime {
    UNIX_EPOCH + std::time::Duration::from_millis(u64::try_from(ms).unwrap_or(0))
}

#[cfg(test)]
mod testes {
    use super::*;

    const SHA_A: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
    const SHA_B: &str = "0f0e0d0c0b0a09080706050403020100ffeeddccbbaa99887766554433221100";
    const AGORA: i64 = 1_795_131_511_000;
    const ANTIGO: i64 = AGORA - 40 * 86_400_000;
    const RECENTE: i64 = AGORA - 60_000;

    fn gravar(pasta: &Path, nome: &str, modificado_ms: i64) -> PathBuf {
        let caminho = pasta.join(nome);
        fs::write(&caminho, b"conteudo").unwrap();
        let arquivo = fs::OpenOptions::new().write(true).open(&caminho).unwrap();
        arquivo
            .set_modified(instante_de_sistema(modificado_ms))
            .unwrap();
        caminho
    }

    fn nomes(orfaos: &[Orfao]) -> Vec<&str> {
        orfaos.iter().map(|o| o.nome.as_str()).collect()
    }

    #[test]
    fn so_apaga_blob_sem_dono_e_fora_da_carencia() {
        let pasta = tempfile::tempdir().unwrap();
        gravar(pasta.path(), SHA_A, ANTIGO); // órfão antigo: sai
        gravar(pasta.path(), SHA_B, ANTIGO); // antigo, mas com dono: fica
        let carencia = AGORA - 3_600_000; // 1 hora
        let refs: HashSet<String> = [SHA_B.to_owned()].into();
        let orfaos = encontrar_orfaos(pasta.path(), &refs, carencia).unwrap();
        assert_eq!(nomes(&orfaos), [SHA_A]);
    }

    #[test]
    fn blob_recente_fica_mesmo_sem_dono() {
        let pasta = tempfile::tempdir().unwrap();
        gravar(pasta.path(), SHA_A, RECENTE);
        let orfaos = encontrar_orfaos(pasta.path(), &HashSet::new(), AGORA - 3_600_000).unwrap();
        assert!(orfaos.is_empty(), "blob recente não pode sair: {orfaos:?}");
    }

    #[test]
    fn temporario_abandonado_sai_e_temporario_recente_fica() {
        let pasta = tempfile::tempdir().unwrap();
        gravar(pasta.path(), ".abc.123.tmp", ANTIGO);
        gravar(pasta.path(), ".def.456.tmp", RECENTE);
        let orfaos = encontrar_orfaos(pasta.path(), &HashSet::new(), AGORA - 3_600_000).unwrap();
        assert_eq!(nomes(&orfaos), [".abc.123.tmp"]);
    }

    #[test]
    fn nome_que_nao_e_blob_nunca_entra() {
        let pasta = tempfile::tempdir().unwrap();
        gravar(pasta.path(), "anotacoes.txt", ANTIGO);
        gravar(pasta.path(), "SHA_MAIUSCULO_NAO_E_BLOB", ANTIGO);
        fs::create_dir(pasta.path().join(SHA_A)).unwrap(); // pasta com nome de blob
        let orfaos = encontrar_orfaos(pasta.path(), &HashSet::new(), AGORA).unwrap();
        assert!(orfaos.is_empty(), "{orfaos:?}");
    }

    #[cfg(unix)]
    #[test]
    fn link_simbolico_nunca_e_apagado() {
        let fora = tempfile::tempdir().unwrap();
        let alvo = gravar(fora.path(), "alvo.bin", ANTIGO);
        let pasta = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(&alvo, pasta.path().join(SHA_A)).unwrap();
        let orfaos = encontrar_orfaos(pasta.path(), &HashSet::new(), AGORA).unwrap();
        assert!(orfaos.is_empty(), "link não é blob: {orfaos:?}");
        assert!(alvo.exists());
    }

    #[test]
    fn pasta_inexistente_nao_tem_orfaos() {
        let pasta = tempfile::tempdir().unwrap();
        let ausente = pasta.path().join("nao-existe");
        assert!(
            encontrar_orfaos(&ausente, &HashSet::new(), AGORA)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn remover_apaga_so_o_arquivo_indicado() {
        let pasta = tempfile::tempdir().unwrap();
        let caminho = gravar(pasta.path(), SHA_A, ANTIGO);
        let outro = gravar(pasta.path(), SHA_B, ANTIGO);
        let orfao = Orfao {
            nome: SHA_A.to_owned(),
            caminho: caminho.clone(),
        };
        remover(&orfao).unwrap();
        assert!(!caminho.exists());
        assert!(outro.exists());
    }
}
