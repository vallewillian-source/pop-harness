//! Limites do recebimento de arquivos. Os padrões vêm do PRD §11 e do spike HR-02 (§5 e §9).
//! A leitura do arquivo de configuração e do ambiente fica no crate `poph` (raiz de composição).

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::erro::ErroArquivo;

/// Limite de tamanho por arquivo: 16 MB (PRD §11, igual ao limite do WhatsApp).
pub const LIMITE_ARQUIVO_PADRAO: u64 = 16 * 1024 * 1024;
/// Teto de texto extraído por arquivo, em caracteres (PRD §11, padrão proposto).
pub const LIMITE_TEXTO_PADRAO: usize = 20_000;
/// Tempo máximo da leitura de um arquivo (spike HR-02 §5.2: ~5 s).
pub const TEMPO_EXTRACAO_PADRAO: Duration = Duration::from_secs(5);
/// Teto de memória do processo que lê o arquivo. O spike mediu 2,5 GB com um PDF de 149 KB.
pub const MEMORIA_EXTRACAO_PADRAO: u64 = 512 * 1024 * 1024;

/// Valor mínimo aceito para o teto de memória do leitor: abaixo disso nem o processo sobe.
const MEMORIA_MINIMA: u64 = 128 * 1024 * 1024;
/// Teto de sanidade para o limite de arquivo (256 MB). Acima disso o recebimento não faz sentido.
const LIMITE_ARQUIVO_MAXIMO: u64 = 256 * 1024 * 1024;

/// Configuração do recebimento. Todos os campos têm padrão via [`ConfigArquivos::padrao`].
#[derive(Debug, Clone)]
pub struct ConfigArquivos {
    /// Pasta dos blobs (`data/blobs`). Cada org tem a sua subpasta.
    pub pasta_blobs: PathBuf,
    /// Tamanho máximo do arquivo recebido, em bytes.
    pub limite_bytes: u64,
    /// Máximo de caracteres de texto extraído que vai ao LLM.
    pub limite_caracteres: usize,
    /// Tempo máximo da leitura de um arquivo.
    pub tempo_extracao: Duration,
    /// Teto de memória do processo leitor, em bytes.
    pub memoria_extracao_bytes: u64,
    /// Caminho do programa `poph-extrator`. `None` = o ao lado do executável atual.
    pub caminho_extrator: Option<PathBuf>,
}

impl ConfigArquivos {
    /// Configuração com os padrões do PRD, guardando os blobs em `pasta_blobs`.
    pub fn padrao(pasta_blobs: impl AsRef<Path>) -> Self {
        Self {
            pasta_blobs: pasta_blobs.as_ref().to_path_buf(),
            limite_bytes: LIMITE_ARQUIVO_PADRAO,
            limite_caracteres: LIMITE_TEXTO_PADRAO,
            tempo_extracao: TEMPO_EXTRACAO_PADRAO,
            memoria_extracao_bytes: MEMORIA_EXTRACAO_PADRAO,
            caminho_extrator: None,
        }
    }

    /// Confere os valores antes de usar. Chamado no começo de cada recebimento.
    pub fn validar(&self) -> Result<(), ErroArquivo> {
        if self.limite_bytes == 0 || self.limite_bytes > LIMITE_ARQUIVO_MAXIMO {
            return Err(ErroArquivo::Configuracao(format!(
                "o limite de arquivo precisa estar entre 1 byte e {} MB",
                LIMITE_ARQUIVO_MAXIMO / (1024 * 1024)
            )));
        }
        if self.limite_caracteres == 0 {
            return Err(ErroArquivo::Configuracao(
                "o limite de caracteres do texto extraído precisa ser maior que zero".to_owned(),
            ));
        }
        if self.tempo_extracao.is_zero() {
            return Err(ErroArquivo::Configuracao(
                "o tempo máximo de leitura precisa ser maior que zero".to_owned(),
            ));
        }
        if self.memoria_extracao_bytes < MEMORIA_MINIMA {
            return Err(ErroArquivo::Configuracao(format!(
                "o teto de memória do leitor precisa ser de pelo menos {} MB",
                MEMORIA_MINIMA / (1024 * 1024)
            )));
        }
        Ok(())
    }

    /// Caminho do programa leitor: o configurado, ou `poph-extrator` ao lado do executável atual.
    pub fn caminho_extrator_efetivo(&self) -> Result<PathBuf, ErroArquivo> {
        if let Some(caminho) = &self.caminho_extrator {
            return Ok(caminho.clone());
        }
        let executavel = std::env::current_exe().map_err(|erro| {
            ErroArquivo::Extrator(format!("não sei onde estou ({})", erro.kind()))
        })?;
        let pasta = executavel.parent().ok_or_else(|| {
            ErroArquivo::Extrator("o executável não tem pasta conhecida".to_owned())
        })?;
        Ok(pasta.join("poph-extrator"))
    }
}
