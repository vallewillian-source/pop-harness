use std::io::{Read, Write};
use std::time::Instant;

use poph_files::isolation::limit_memory;
use poph_files::readers::read;
use serde_json::json;

const MAX_INPUT_SIZE: u64 = 16 * 1024 * 1024 + 1;

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let response = match run(&arguments) {
        Ok(value) => value,
        Err(motivo) => json!({ "ok": false, "erro": motivo }),
    };
    let mut saida = std::io::stdout().lock();
    let _ = writeln!(saida, "{response}");
    let _ = saida.flush();
}

fn run(arguments: &[String]) -> Result<serde_json::Value, String> {
    let [formato, character_limit, memory_cap] = arguments else {
        return Err(
            "uso: poph-extrator <formato> <limite_caracteres> <teto_memoria_bytes>".to_owned(),
        );
    };
    let character_limit: usize = character_limit
        .parse()
        .map_err(|_| "limite de caracteres inválido".to_owned())?;
    let memory_cap: u64 = memory_cap
        .parse()
        .map_err(|_| "teto de memória inválido".to_owned())?;

    limit_memory(memory_cap)?;

    let mut bytes = Vec::new();
    std::io::stdin()
        .take(MAX_INPUT_SIZE)
        .read_to_end(&mut bytes)
        .map_err(|_| "não consegui ler o arquivo na entrada".to_owned())?;
    if bytes.len() as u64 >= MAX_INPUT_SIZE {
        return Err("o arquivo passa do limite de recebimento".to_owned());
    }

    let start = Instant::now();
    let loaded = read(formato, &bytes, character_limit)?;
    let duration_ms = start.elapsed().as_millis() as u64;

    Ok(json!({
        "ok": true,
        "texto": loaded.texto,
        "caracteres": loaded.characters,
        "pico_memoria_kb": peak_memory_kb(),
        "duracao_ms": duration_ms,
    }))
}

fn peak_memory_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|numero| numero.parse().ok())
}
