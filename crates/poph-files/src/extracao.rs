//! Extração isolada: a leitura do arquivo roda num processo filho (`poph-extrator`), com teto de
//! memória (dentro do filho, antes de ler) e prazo de tempo (controlado aqui, pelo pai).
//!
//! Por que processo e não thread: o spike HR-02 (§5.1) mediu um PDF de 149 KB que consome 2,5 GB
//! de memória nos dois leitores. Uma thread não tem teto próprio de memória e não pode ser
//! morta; um processo filho pode ser morto e o sistema corta o excesso de memória sem derrubar
//! o processo principal.
//!
//! O filho recebe o arquivo pela entrada padrão e devolve um JSON na saída padrão. Nada de
//! caminho de disco entra no filho, e o ambiente dele é vazio.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::config::ConfigArquivos;
use crate::erro::ErroArquivo;

/// Teto da saída do filho. O texto já vem cortado no limite de caracteres; isto é só segurança.
const LIMITE_SAIDA_BYTES: u64 = 4 * 1024 * 1024;
/// Intervalo entre conferências do filho.
const INTERVALO_CONFERENCIA: Duration = Duration::from_millis(5);

/// Resultado da leitura feita pelo filho.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extraido {
    /// Texto cortado no limite de caracteres. Vazio = o arquivo não tem texto (ex.: PDF escaneado).
    pub texto: String,
    /// Caracteres antes do corte.
    pub caracteres: usize,
    /// Pico de memória do filho, em KB (quando o sistema informa).
    pub pico_memoria_kb: Option<u64>,
    /// Tempo gasto pelo filho, em milissegundos (medido dentro do filho).
    pub duracao_ms: Option<u64>,
}

/// Lê o texto de `bytes` no formato `formato` (`pdf`, `docx`, `pptx`, `xlsx`, `xls`) num processo
/// isolado, com o tempo e a memória da configuração.
pub fn extrair(
    config: &ConfigArquivos,
    formato: &str,
    bytes: &[u8],
) -> Result<Extraido, ErroArquivo> {
    let programa = config.caminho_extrator_efetivo()?;
    if !programa.is_file() {
        return Err(ErroArquivo::Extrator(
            "o programa poph-extrator não está ao lado do poph; rode o build do crate poph-files"
                .to_owned(),
        ));
    }

    let mut filho = Command::new(&programa)
        .arg(formato)
        .arg(config.limite_caracteres.to_string())
        .arg(config.memoria_extracao_bytes.to_string())
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|erro| {
            ErroArquivo::Extrator(format!("não consegui iniciar o leitor ({})", erro.kind()))
        })?;

    // Entrada e saída em threads próprias: o filho pode terminar antes de ler tudo, e o pai não
    // pode ficar preso num pipe cheio.
    let mut entrada = filho.stdin.take();
    let copia_entrada = bytes.to_vec();
    let escrita = thread::spawn(move || {
        if let Some(mut pipe) = entrada.take() {
            let _ = pipe.write_all(&copia_entrada);
        }
    });
    let mut saida_pipe = filho.stdout.take();
    let leitura = thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(pipe) = saida_pipe.as_mut() {
            let _ = pipe.take(LIMITE_SAIDA_BYTES + 1).read_to_end(&mut buffer);
        }
        buffer
    });

    let inicio = Instant::now();
    let prazo = config.tempo_extracao;
    let estado = loop {
        match filho.try_wait() {
            Ok(Some(estado)) => break Some(estado),
            Ok(None) => {
                if inicio.elapsed() >= prazo {
                    let _ = filho.kill();
                    let _ = filho.wait();
                    break None;
                }
                thread::sleep(INTERVALO_CONFERENCIA);
            }
            Err(_) => {
                let _ = filho.kill();
                let _ = filho.wait();
                break None;
            }
        }
    };

    let _ = escrita.join();
    let saida = leitura.join().unwrap_or_default();

    let Some(estado) = estado else {
        return Err(ErroArquivo::Tempo {
            limite_segundos: prazo.as_secs_f64(),
        });
    };
    if !estado.success() {
        return Err(ErroArquivo::Leitura(
            "o leitor parou antes de terminar (arquivo muito pesado, corrompido ou protegido)"
                .to_owned(),
        ));
    }
    if saida.len() as u64 > LIMITE_SAIDA_BYTES {
        return Err(ErroArquivo::Leitura(
            "o resultado da leitura passou do limite de saída".to_owned(),
        ));
    }
    interpretar_saida(&saida)
}

/// Lê a resposta JSON do filho (`ok`, `texto`, `caracteres`, `pico_memoria_kb`, `duracao_ms`).
fn interpretar_saida(saida: &[u8]) -> Result<Extraido, ErroArquivo> {
    let valor: Value = serde_json::from_slice(saida)
        .map_err(|_| ErroArquivo::Leitura("o leitor devolveu uma resposta inválida".to_owned()))?;
    if valor.get("ok").and_then(Value::as_bool) != Some(true) {
        let motivo = valor
            .get("erro")
            .and_then(Value::as_str)
            .unwrap_or("erro não informado pelo leitor");
        return Err(ErroArquivo::Leitura(motivo.to_owned()));
    }
    let texto = valor
        .get("texto")
        .and_then(Value::as_str)
        .ok_or_else(|| ErroArquivo::Leitura("o leitor não devolveu texto".to_owned()))?
        .to_owned();
    let caracteres = valor.get("caracteres").and_then(Value::as_u64).unwrap_or(0) as usize;
    Ok(Extraido {
        texto,
        caracteres,
        pico_memoria_kb: valor.get("pico_memoria_kb").and_then(Value::as_u64),
        duracao_ms: valor.get("duracao_ms").and_then(Value::as_u64),
    })
}
