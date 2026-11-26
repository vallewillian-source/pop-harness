//! Instalação do modelo de embedding (`poph model install`, D-09).
//!
//! É o único ponto que baixa da rede. O turno nunca chama isto: se o modelo falta, o erro manda
//! rodar o comando. Cada arquivo é baixado para um temporário, o sha256 é conferido e só então
//! ele recebe o nome final. Arquivo com hash errado nunca fica na pasta de modelos.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::erro::RagError;

/// Nome do arquivo do modelo int8 na pasta de modelos.
pub const ARQUIVO_MODELO: &str = "model_int8.onnx";
/// Nome do arquivo do tokenizador na pasta de modelos.
pub const ARQUIVO_TOKENIZADOR: &str = "tokenizer.json";
/// sha256 esperado do `model_int8.onnx` (FETCH_MODELS.md do spike HR-02).
pub const SHA256_MODELO: &str = "d6ea442ff6a891daefed7c83b2f596fc5dc66bf697e4d006236f64f34bbcf4c8";
/// sha256 esperado do `tokenizer.json` do repositório original do sentence-transformers.
pub const SHA256_TOKENIZADOR: &str =
    "2c3387be76557bd40970cec13153b3bbf80407865484b209e655e5e4729076b8";
/// Origem do modelo int8, fixada no commit `2c4055b1…` do repositório Xenova.
pub const URL_MODELO: &str = "https://huggingface.co/Xenova/paraphrase-multilingual-MiniLM-L12-v2/resolve/2c4055b12046f11709e9df2c122e59ffbdc2f900/onnx/model_int8.onnx";
/// Origem do tokenizador, fixada no commit `e8f8c211…` do repositório do sentence-transformers.
pub const URL_TOKENIZADOR: &str = "https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2/resolve/e8f8c211226b894fcb81acc59f3b34ba3efd5f42/tokenizer.json";

/// Um arquivo do modelo: nome local, origem e sha256 esperado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArquivoModelo {
    /// Nome do arquivo na pasta de modelos.
    pub nome: String,
    /// Endereço de onde baixar.
    pub url: String,
    /// sha256 em hexadecimal minúsculo.
    pub sha256: String,
}

/// Os dois arquivos que a V1 precisa: modelo int8 e tokenizador.
pub fn arquivos_padrao() -> Vec<ArquivoModelo> {
    vec![
        ArquivoModelo {
            nome: ARQUIVO_MODELO.to_owned(),
            url: URL_MODELO.to_owned(),
            sha256: SHA256_MODELO.to_owned(),
        },
        ArquivoModelo {
            nome: ARQUIVO_TOKENIZADOR.to_owned(),
            url: URL_TOKENIZADOR.to_owned(),
            sha256: SHA256_TOKENIZADOR.to_owned(),
        },
    ]
}

/// Como o arquivo ficou na instalação.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Situacao {
    /// Baixado agora e conferido.
    Baixado,
    /// Já estava na pasta com o sha256 certo; nada foi baixado.
    JaEstava,
}

impl Situacao {
    /// Texto estável para o JSON da CLI.
    pub fn como_texto(self) -> &'static str {
        match self {
            Situacao::Baixado => "baixado",
            Situacao::JaEstava => "ja_estava_instalado",
        }
    }
}

/// Resultado de um arquivo da instalação.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArquivoInstalado {
    /// Nome do arquivo.
    pub nome: String,
    /// Tamanho em bytes.
    pub bytes: u64,
    /// sha256 conferido.
    pub sha256: String,
    /// Se foi baixado agora ou já estava.
    pub situacao: Situacao,
}

/// Resultado da instalação inteira.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatorioInstalacao {
    /// Pasta de modelos usada.
    pub diretorio: PathBuf,
    /// Um item por arquivo do modelo.
    pub arquivos: Vec<ArquivoInstalado>,
}

/// Instala os arquivos padrão do modelo em `diretorio` (cria a pasta se preciso).
pub fn instalar(diretorio: &Path) -> Result<RelatorioInstalacao, RagError> {
    instalar_arquivos(diretorio, &arquivos_padrao())
}

/// Instala a lista dada de arquivos. Usado pela CLI (com os padrões) e pelos testes (com servidor local).
pub fn instalar_arquivos(
    diretorio: &Path,
    arquivos: &[ArquivoModelo],
) -> Result<RelatorioInstalacao, RagError> {
    fs::create_dir_all(diretorio)
        .map_err(|erro| RagError::File(format!("não consegui criar a pasta de modelos: {erro}")))?;
    let cliente = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(1800))
        .user_agent("poph-model-install")
        .build()
        .map_err(|erro| RagError::Download {
            file: "cliente HTTP".to_owned(),
            reason: erro.to_string(),
        })?;
    let mut itens = Vec::with_capacity(arquivos.len());
    for arquivo in arquivos {
        itens.push(garantir(diretorio, arquivo, &cliente)?);
    }
    Ok(RelatorioInstalacao {
        diretorio: diretorio.to_path_buf(),
        arquivos: itens,
    })
}

fn garantir(
    diretorio: &Path,
    arquivo: &ArquivoModelo,
    cliente: &reqwest::blocking::Client,
) -> Result<ArquivoInstalado, RagError> {
    let destino = diretorio.join(&arquivo.nome);
    if destino.is_file() {
        let obtido = sha256_de_arquivo(&destino)
            .map_err(|erro| RagError::File(format!("não consegui ler {}: {erro}", arquivo.nome)))?;
        if obtido == arquivo.sha256 {
            let bytes = fs::metadata(&destino).map(|m| m.len()).unwrap_or(0);
            return Ok(ArquivoInstalado {
                nome: arquivo.nome.clone(),
                bytes,
                sha256: obtido,
                situacao: Situacao::JaEstava,
            });
        }
        // Arquivo presente mas diferente do esperado: é substituído pelo download abaixo.
    }

    let parcial = diretorio.join(format!(".{}.parcial", arquivo.nome));
    let (obtido, bytes) = match baixar(cliente, &arquivo.url, &parcial, &arquivo.nome) {
        Ok(resultado) => resultado,
        Err(erro) => {
            let _ = fs::remove_file(&parcial);
            return Err(erro);
        }
    };
    if obtido != arquivo.sha256 {
        let _ = fs::remove_file(&parcial);
        return Err(RagError::ChecksumMismatch {
            file: arquivo.nome.clone(),
            expected: arquivo.sha256.clone(),
            actual: obtido,
        });
    }
    fs::rename(&parcial, &destino).map_err(|erro| {
        let _ = fs::remove_file(&parcial);
        RagError::File(format!("não consegui publicar {}: {erro}", arquivo.nome))
    })?;
    Ok(ArquivoInstalado {
        nome: arquivo.nome.clone(),
        bytes,
        sha256: obtido,
        situacao: Situacao::Baixado,
    })
}

/// Baixa `url` para `destino` e devolve (sha256, bytes). Não confere o hash: quem chama confere.
fn baixar(
    cliente: &reqwest::blocking::Client,
    url: &str,
    destino: &Path,
    nome: &str,
) -> Result<(String, u64), RagError> {
    let falha = |motivo: String| RagError::Download {
        file: nome.to_owned(),
        reason: motivo,
    };
    let mut resposta = cliente
        .get(url)
        .send()
        .map_err(|erro| falha(erro.to_string()))?;
    if !resposta.status().is_success() {
        return Err(falha(format!(
            "resposta HTTP {}",
            resposta.status().as_u16()
        )));
    }
    let mut arquivo = File::create(destino)
        .map_err(|erro| RagError::File(format!("não consegui gravar {nome}: {erro}")))?;
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut bloco = vec![0u8; 64 * 1024];
    loop {
        let lidos = resposta
            .read(&mut bloco)
            .map_err(|erro| falha(erro.to_string()))?;
        if lidos == 0 {
            break;
        }
        hash.update(&bloco[..lidos]);
        arquivo
            .write_all(&bloco[..lidos])
            .map_err(|erro| RagError::File(format!("não consegui gravar {nome}: {erro}")))?;
        bytes += lidos as u64;
    }
    arquivo
        .sync_all()
        .map_err(|erro| RagError::File(format!("não consegui gravar {nome}: {erro}")))?;
    Ok((hex(&hash.finalize()), bytes))
}

/// Confere o arquivo `nome` em `diretorio` contra o sha256 esperado.
///
/// Devolve o sha256 obtido quando confere. Erros: [`ErroRag::ModeloAusente`] se o arquivo não
/// existe, [`ErroRag::ChecksumNaoConfere`] se o conteúdo é outro.
pub fn conferir_arquivo(diretorio: &Path, nome: &str, esperado: &str) -> Result<String, RagError> {
    let caminho = diretorio.join(nome);
    if !caminho.is_file() {
        return Err(RagError::ModelMissing {
            file: nome.to_owned(),
        });
    }
    let obtido = sha256_de_arquivo(&caminho)
        .map_err(|erro| RagError::File(format!("não consegui ler {nome}: {erro}")))?;
    if obtido != esperado {
        return Err(RagError::ChecksumMismatch {
            file: nome.to_owned(),
            expected: esperado.to_owned(),
            actual: obtido,
        });
    }
    Ok(obtido)
}

/// sha256 de um arquivo, lido em blocos (não carrega o arquivo inteiro na memória).
pub fn sha256_de_arquivo(caminho: &Path) -> io::Result<String> {
    let mut arquivo = File::open(caminho)?;
    let mut hash = Sha256::new();
    let mut bloco = vec![0u8; 1024 * 1024];
    loop {
        let lidos = arquivo.read(&mut bloco)?;
        if lidos == 0 {
            break;
        }
        hash.update(&bloco[..lidos]);
    }
    Ok(hex(&hash.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    let mut texto = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(texto, "{byte:02x}");
    }
    texto
}
