//! Ponto de entrada do recebimento: tamanho, tipo real, leitura isolada, gravação do blob e as
//! partes que vão ao LLM.
//!
//! Ordem: tamanho → tipo → leitura (só para documento) → blob. Um arquivo recusado em qualquer
//! etapa não deixa nada gravado. A gravação na tabela `attachments` é da HR-24.

use std::path::PathBuf;

use poph_types::OrgId;

use crate::blob;
use crate::config::ConfigArquivos;
use crate::envelope::{AVISO_NAO_CONFIAVEL, envelopar};
use crate::erro::ErroArquivo;
use crate::extracao;
use crate::tipo::{Tipo, classificar};

/// Metadados devolvidos para quem grava a tabela `attachments` (HR-24).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadadosArquivo {
    /// Nome que o cliente mandou, só o nome (sem pasta), para mostrar. Nunca vai para caminho de disco.
    pub nome_original: String,
    /// Tamanho em bytes.
    pub tamanho_bytes: u64,
    /// SHA-256 em hexadecimal minúsculo.
    pub sha256: String,
    /// Tipo MIME real, pelo conteúdo.
    pub mime_real: String,
    /// Tipo reconhecido.
    pub tipo: Tipo,
    /// Onde o conteúdo está no disco (`<pasta_blobs>/<org>/<sha256>`).
    pub caminho_blob: PathBuf,
    /// `true` se o mesmo conteúdo já estava guardado para esta org.
    pub blob_ja_existia: bool,
}

/// O que o arquivo vira para o LLM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conteudo {
    /// Imagem, como parte multimodal.
    Imagem { mime: String, bytes: Vec<u8> },
    /// Texto extraído, já dentro do bloco não confiável.
    Texto {
        /// Texto envelopado (ver `envelope`).
        envelopado: String,
        /// Caracteres do texto antes do corte.
        caracteres_originais: usize,
        /// `true` se o texto foi cortado no limite.
        truncado: bool,
    },
    /// Documento sem texto (ex.: PDF escaneado). O LLM deve dizer que não conseguiu ler.
    SemTexto,
}

/// Parte entregue ao LLM. Mesma forma do que o provedor aceita: texto ou imagem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParteArquivo {
    /// Texto de aviso (nosso, confiável) ou bloco não confiável, conforme o caso.
    Texto(String),
    /// Imagem em bytes. `mime` é um dos tipos de imagem aceitos.
    Imagem { mime: String, bytes: Vec<u8> },
}

/// Arquivo recebido e validado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArquivoRecebido {
    pub metadados: MetadadosArquivo,
    pub conteudo: Conteudo,
}

impl ArquivoRecebido {
    /// Partes para o LLM, na ordem: aviso confiável (se houver) e depois o conteúdo.
    ///
    /// - Imagem: uma parte de imagem.
    /// - Texto: aviso sobre o bloco não confiável, o aviso de corte (se cortou) e o bloco.
    /// - Sem texto: uma parte de texto dizendo que não foi possível ler o arquivo.
    pub fn para_llm(self) -> Vec<ParteArquivo> {
        // O nome do arquivo veio do cliente: não entra nos avisos, que o LLM lê como confiáveis.
        match self.conteudo {
            Conteudo::Imagem { mime, bytes } => vec![ParteArquivo::Imagem { mime, bytes }],
            Conteudo::Texto {
                envelopado,
                caracteres_originais,
                truncado,
            } => {
                let mut partes = vec![ParteArquivo::Texto(AVISO_NAO_CONFIAVEL.to_owned())];
                if truncado {
                    partes.push(ParteArquivo::Texto(format!(
                        "Aviso: o texto do arquivo tinha {caracteres_originais} caracteres e foi cortado no limite; parte dele não está visível."
                    )));
                }
                partes.push(ParteArquivo::Texto(envelopado));
                partes
            }
            Conteudo::SemTexto => vec![ParteArquivo::Texto(
                "Não consegui ler o texto do arquivo enviado. Ele pode ser uma imagem digitalizada, sem texto selecionável. Diga ao cliente que não foi possível ler o arquivo e peça outro formato ou o texto digitado. Não invente o conteúdo.".to_owned(),
            )],
        }
    }
}

/// Recebe um arquivo do cliente: valida, lê (se for documento) e grava o blob da org.
///
/// `nome` é o nome que veio do cliente; é usado para mostrar e para checar a extensão, nunca
/// para montar caminho de disco.
pub fn receber(
    config: &ConfigArquivos,
    org: OrgId,
    nome: &str,
    bytes: &[u8],
) -> Result<ArquivoRecebido, ErroArquivo> {
    config.validar()?;

    if bytes.is_empty() {
        return Err(ErroArquivo::Vazio);
    }
    let tamanho = bytes.len() as u64;
    if tamanho > config.limite_bytes {
        return Err(ErroArquivo::Grande {
            tamanho_mb: mb(tamanho),
            limite_mb: mb(config.limite_bytes),
        });
    }

    let tipo = classificar(bytes, nome)?;
    let conteudo = if tipo.eh_imagem() {
        Conteudo::Imagem {
            mime: tipo.mime().to_owned(),
            bytes: bytes.to_vec(),
        }
    } else {
        let formato = tipo
            .formato_leitor()
            .ok_or_else(|| ErroArquivo::Leitura("formato sem leitor".to_owned()))?;
        let extraido = extracao::extrair(config, formato, bytes)?;
        if extraido.caracteres == 0 {
            Conteudo::SemTexto
        } else {
            Conteudo::Texto {
                envelopado: envelopar(&extraido.texto),
                caracteres_originais: extraido.caracteres,
                truncado: extraido.caracteres > config.limite_caracteres,
            }
        }
    };

    let gravado = blob::gravar(&config.pasta_blobs, org, bytes)?;
    Ok(ArquivoRecebido {
        metadados: MetadadosArquivo {
            nome_original: nome_para_exibir(nome),
            tamanho_bytes: tamanho,
            sha256: gravado.sha256,
            mime_real: tipo.mime().to_owned(),
            tipo,
            caminho_blob: gravado.caminho,
            blob_ja_existia: gravado.ja_existia,
        },
        conteudo,
    })
}

/// Só o nome do arquivo (sem pasta), sem caracteres de controle, com teto de 255 caracteres.
pub fn nome_para_exibir(nome: &str) -> String {
    let apenas_nome = nome.rsplit(['/', '\\']).next().unwrap_or(nome);
    let limpo: String = apenas_nome
        .chars()
        .filter(|c| !c.is_control())
        .take(255)
        .collect();
    let limpo = limpo.trim().to_owned();
    if limpo.is_empty() {
        "arquivo".to_owned()
    } else {
        limpo
    }
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn nome_perde_a_pasta_e_os_controles() {
        assert_eq!(nome_para_exibir("../../etc/passwd"), "passwd");
        assert_eq!(nome_para_exibir("C:\\docs\\contrato.pdf"), "contrato.pdf");
        assert_eq!(nome_para_exibir("a\u{0007}b.pdf"), "ab.pdf");
        assert_eq!(nome_para_exibir("   "), "arquivo");
    }
}
