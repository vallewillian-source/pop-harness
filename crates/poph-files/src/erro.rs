//! Erros do recebimento de arquivos. Mensagens em português, prontas para o cliente do Console
//! e para a resposta do agente. Nenhuma mensagem traz caminho de disco nem conteúdo do arquivo.

use thiserror::Error;

/// Erros de recebimento, validação, gravação e extração de arquivos.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum ErroArquivo {
    /// Arquivo sem nenhum byte.
    #[error("o arquivo está vazio")]
    Vazio,

    /// Passou do limite de tamanho. Não é gravado.
    #[error("o arquivo tem {tamanho_mb} MB e o limite é de {limite_mb} MB; não foi gravado")]
    Grande { tamanho_mb: f64, limite_mb: f64 },

    /// Tipo que não é aceito (programa, script, áudio, formato antigo do Office etc.). Não é gravado.
    #[error("arquivo recusado: {0}")]
    TipoNaoAceito(String),

    /// O nome diz um tipo e o conteúdo é outro. Não é gravado.
    #[error("arquivo recusado: o nome termina em .{declarada}, mas o conteúdo é {real}")]
    ExtensaoEnganosa { declarada: String, real: String },

    /// O leitor do arquivo recusou ou não conseguiu ler o conteúdo (arquivo corrompido, bomba de
    /// descompactação, protegido por senha etc.).
    #[error("não consegui ler o arquivo: {0}")]
    Leitura(String),

    /// A leitura passou do limite de tempo e foi interrompida.
    #[error("a leitura do arquivo passou de {limite_segundos} s e foi interrompida")]
    Tempo { limite_segundos: f64 },

    /// O programa auxiliar de leitura não foi encontrado ou não pôde ser iniciado.
    #[error("programa de leitura de arquivos indisponível: {0}")]
    Extrator(String),

    /// Falha ao gravar o arquivo (blob) no disco.
    #[error("falha ao gravar o arquivo no disco: {0}")]
    Gravacao(String),

    /// Configuração inválida (limite zero, tempo zero etc.).
    #[error("configuração de arquivos inválida: {0}")]
    Configuracao(String),
}
