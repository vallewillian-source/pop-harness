//! Programa auxiliar `poph-extrator`: lê um documento num processo separado e devolve o texto.
//!
//! Uso (feito pelo crate `poph-files`, não à mão):
//!
//! ```text
//! poph-extrator <formato> <limite_caracteres> <teto_memoria_bytes> < arquivo
//! ```
//!
//! Formato: `pdf`, `docx`, `pptx`, `xlsx` ou `xls`. O arquivo chega pela entrada padrão e a
//! resposta sai numa linha JSON pela saída padrão. Nada é gravado no disco e nada é lido de
//! caminho nenhum: o processo pai já decidiu o tipo e os limites.

use std::io::{Read, Write};
use std::time::Instant;

use poph_files::isolamento::limitar_memoria;
use poph_files::leitores::ler;
use serde_json::json;

/// Teto de entrada: o limite de recebimento (16 MB) com uma folga de 1 byte para detectar excesso.
const TAMANHO_MAXIMO_ENTRADA: u64 = 16 * 1024 * 1024 + 1;

fn main() {
    let argumentos: Vec<String> = std::env::args().skip(1).collect();
    let resposta = match executar(&argumentos) {
        Ok(valor) => valor,
        Err(motivo) => json!({ "ok": false, "erro": motivo }),
    };
    let mut saida = std::io::stdout().lock();
    let _ = writeln!(saida, "{resposta}");
    let _ = saida.flush();
}

fn executar(argumentos: &[String]) -> Result<serde_json::Value, String> {
    let [formato, limite_caracteres, teto_memoria] = argumentos else {
        return Err(
            "uso: poph-extrator <formato> <limite_caracteres> <teto_memoria_bytes>".to_owned(),
        );
    };
    let limite_caracteres: usize = limite_caracteres
        .parse()
        .map_err(|_| "limite de caracteres inválido".to_owned())?;
    let teto_memoria: u64 = teto_memoria
        .parse()
        .map_err(|_| "teto de memória inválido".to_owned())?;

    // Primeiro o teto de memória: a partir daqui, nenhum byte é lido sem limite.
    limitar_memoria(teto_memoria)?;

    let mut bytes = Vec::new();
    std::io::stdin()
        .take(TAMANHO_MAXIMO_ENTRADA)
        .read_to_end(&mut bytes)
        .map_err(|_| "não consegui ler o arquivo na entrada".to_owned())?;
    if bytes.len() as u64 >= TAMANHO_MAXIMO_ENTRADA {
        return Err("o arquivo passa do limite de recebimento".to_owned());
    }

    let inicio = Instant::now();
    let lido = ler(formato, &bytes, limite_caracteres)?;
    let duracao_ms = inicio.elapsed().as_millis() as u64;

    Ok(json!({
        "ok": true,
        "texto": lido.texto,
        "caracteres": lido.caracteres,
        "pico_memoria_kb": pico_memoria_kb(),
        "duracao_ms": duracao_ms,
    }))
}

/// Pico de memória residente deste processo (VmHWM), em KB. `None` fora do Linux.
fn pico_memoria_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|linha| linha.strip_prefix("VmHWM:"))
        .and_then(|valor| valor.split_whitespace().next())
        .and_then(|numero| numero.parse().ok())
}
