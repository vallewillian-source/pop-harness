//! Teto de memória do processo leitor.
//!
//! O processo `poph-extrator` chama [`limitar_memoria`] logo no começo, antes de ler qualquer
//! byte do arquivo. Se um leitor tentar usar mais memória que o teto, o próprio sistema
//! interrompe o processo leitor; o processo principal (o motor do turno) não é afetado.
//!
//! O código `unsafe` fica dentro da biblioteca `rlimit` (chamada de `setrlimit` do sistema);
//! este crate continua sem `unsafe` próprio.

/// Limita a memória de endereçamento deste processo (RLIMIT_AS), em bytes, e desliga o
/// *core dump*: se o leitor for abortado, o sistema não grava a memória (com o conteúdo do
/// documento) num arquivo de núcleo no disco.
pub fn limitar_memoria(bytes: u64) -> Result<(), String> {
    rlimit::Resource::CORE.set(0, 0).map_err(|erro| {
        format!(
            "não consegui desligar o núcleo de memória do leitor ({})",
            erro.kind()
        )
    })?;
    rlimit::Resource::AS
        .set(bytes, bytes)
        .map_err(|erro| format!("não consegui limitar a memória do leitor ({})", erro.kind()))
}
