use std::path::PathBuf;

use poph_types::OrgId;

use crate::blob;
use crate::config::FileConfig;
use crate::envelope::{UNTRUSTED_NOTICE, envelopar};
use crate::error::FileError;
use crate::extraction;
use crate::file_type::{Tipo, classify};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMetadata {
    pub original_name: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub mime_real: String,
    pub tipo: Tipo,
    pub blob_path: PathBuf,
    pub blob_already_existed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Image {
        mime: String,
        bytes: Vec<u8>,
    },
    Texto {
        envelopado: String,
        original_characters: usize,
        truncated: bool,
    },
    NoText,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilePart {
    Texto(String),
    Image { mime: String, bytes: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedFile {
    pub metadados: FileMetadata,
    pub content: Content,
}

impl ReceivedFile {
    pub fn for_llm(self) -> Vec<FilePart> {
        match self.content {
            Content::Image { mime, bytes } => vec![FilePart::Image { mime, bytes }],
            Content::Texto {
                envelopado,
                original_characters,
                truncated,
            } => {
                let mut parts = vec![FilePart::Texto(UNTRUSTED_NOTICE.to_owned())];
                if truncated {
                    parts.push(FilePart::Texto(format!(
                        "Aviso: o texto do arquivo tinha {original_characters} caracteres e foi cortado no limite; parte dele não está visível."
                    )));
                }
                parts.push(FilePart::Texto(envelopado));
                parts
            }
            Content::NoText => vec![FilePart::Texto(
                "Não consegui ler o texto do arquivo enviado. Ele pode ser uma imagem digitalizada, sem texto selecionável. Diga ao cliente que não foi possível ler o arquivo e peça outro formato ou o texto digitado. Não invente o conteúdo.".to_owned(),
            )],
        }
    }
}

pub fn receive(
    config: &FileConfig,
    org: OrgId,
    nome: &str,
    bytes: &[u8],
) -> Result<ReceivedFile, FileError> {
    config.validar()?;

    if bytes.is_empty() {
        return Err(FileError::Vazio);
    }
    let size = bytes.len() as u64;
    if size > config.byte_limit {
        return Err(FileError::Large {
            size_mb: mb(size),
            limit_mb: mb(config.byte_limit),
        });
    }

    let tipo = classify(bytes, nome)?;
    let content = if tipo.is_image() {
        Content::Image {
            mime: tipo.mime().to_owned(),
            bytes: bytes.to_vec(),
        }
    } else {
        let formato = tipo
            .reader_format()
            .ok_or_else(|| FileError::Reading("formato sem leitor".to_owned()))?;
        let extracted = extraction::extrair(config, formato, bytes)?;
        if extracted.characters == 0 {
            Content::NoText
        } else {
            Content::Texto {
                envelopado: envelopar(&extracted.texto),
                original_characters: extracted.characters,
                truncated: extracted.characters > config.character_limit,
            }
        }
    };

    let stored = blob::write(&config.blob_folder, org, bytes)?;
    Ok(ReceivedFile {
        metadados: FileMetadata {
            original_name: display_name(nome),
            size_bytes: size,
            sha256: stored.sha256,
            mime_real: tipo.mime().to_owned(),
            tipo,
            blob_path: stored.path,
            blob_already_existed: stored.already_existed,
        },
        content,
    })
}

pub fn display_name(nome: &str) -> String {
    let name_only = nome.rsplit(['/', '\\']).next().unwrap_or(nome);
    let clean: String = name_only
        .chars()
        .filter(|c| !c.is_control())
        .take(255)
        .collect();
    let clean = clean.trim().to_owned();
    if clean.is_empty() {
        "arquivo".to_owned()
    } else {
        clean
    }
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_drops_folder_and_controls() {
        assert_eq!(display_name("../../etc/passwd"), "passwd");
        assert_eq!(display_name("C:\\docs\\contrato.pdf"), "contrato.pdf");
        assert_eq!(display_name("a\u{0007}b.pdf"), "ab.pdf");
        assert_eq!(display_name("   "), "arquivo");
    }
}
