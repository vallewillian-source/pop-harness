use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::config::FileConfig;
use crate::error::FileError;

const MAX_OUTPUT_BYTES: u64 = 4 * 1024 * 1024;
const CHECK_INTERVAL: Duration = Duration::from_millis(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    pub texto: String,
    pub characters: usize,
    pub peak_memory_kb: Option<u64>,
    pub duration_ms: Option<u64>,
}

pub fn extrair(config: &FileConfig, formato: &str, bytes: &[u8]) -> Result<Extracted, FileError> {
    let program = config.effective_extractor_path()?;
    if !program.is_file() {
        return Err(FileError::Extrator(
            "o programa poph-extrator não está ao lado do poph; rode o build do crate poph-files"
                .to_owned(),
        ));
    }

    let mut child = Command::new(&program)
        .arg(formato)
        .arg(config.character_limit.to_string())
        .arg(config.extraction_memory_bytes.to_string())
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            FileError::Extrator(format!("não consegui iniciar o leitor ({})", error.kind()))
        })?;

    let mut input = child.stdin.take();
    let input_copy = bytes.to_vec();
    let writing = thread::spawn(move || {
        if let Some(mut pipe) = input.take() {
            let _ = pipe.write_all(&input_copy);
        }
    });
    let mut pipe_output = child.stdout.take();
    let reading = thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(pipe) = pipe_output.as_mut() {
            let _ = pipe.take(MAX_OUTPUT_BYTES + 1).read_to_end(&mut buffer);
        }
        buffer
    });

    let start = Instant::now();
    let deadline = config.extraction_time;
    let estado = loop {
        match child.try_wait() {
            Ok(Some(estado)) => break Some(estado),
            Ok(None) => {
                if start.elapsed() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                thread::sleep(CHECK_INTERVAL);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };

    let _ = writing.join();
    let saida = reading.join().unwrap_or_default();

    let Some(estado) = estado else {
        return Err(FileError::Time {
            limit_seconds: deadline.as_secs_f64(),
        });
    };
    if !estado.success() {
        return Err(FileError::Reading(
            "o leitor parou antes de terminar (arquivo muito pesado, corrompido ou protegido)"
                .to_owned(),
        ));
    }
    if saida.len() as u64 > MAX_OUTPUT_BYTES {
        return Err(FileError::Reading(
            "o resultado da leitura passou do limite de saída".to_owned(),
        ));
    }
    parse_output(&saida)
}

fn parse_output(saida: &[u8]) -> Result<Extracted, FileError> {
    let value: Value = serde_json::from_slice(saida)
        .map_err(|_| FileError::Reading("o leitor devolveu uma resposta inválida".to_owned()))?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        let motivo = value
            .get("erro")
            .and_then(Value::as_str)
            .unwrap_or("erro não informado pelo leitor");
        return Err(FileError::Reading(motivo.to_owned()));
    }
    let texto = value
        .get("texto")
        .and_then(Value::as_str)
        .ok_or_else(|| FileError::Reading("o leitor não devolveu texto".to_owned()))?
        .to_owned();
    let characters = value.get("caracteres").and_then(Value::as_u64).unwrap_or(0) as usize;
    Ok(Extracted {
        texto,
        characters,
        peak_memory_kb: value.get("pico_memoria_kb").and_then(Value::as_u64),
        duration_ms: value.get("duracao_ms").and_then(Value::as_u64),
    })
}
