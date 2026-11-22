//! Auxiliares dos testes do embedding: pasta do modelo, referência do spike, cosseno e captura de log.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// Tolerância: cosseno mínimo entre o vetor daqui e o vetor do spike (mesmo modelo e tokenizador).
pub const COSSENO_MINIMO: f32 = 0.9999;
/// RSS do spike para o int8 depois da carga (RELATORIO_HR02 §3.1): 438 MB.
pub const RSS_SPIKE_BYTES: u64 = 438 * 1_000_000;

pub fn pasta_modelos() -> PathBuf {
    match std::env::var_os("POPH_MODELOS_TESTE") {
        Some(caminho) => PathBuf::from(caminho),
        None => PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/modelos")),
    }
}

/// Devolve `true` se o modelo está na pasta; senão avisa e o teste pula.
pub fn modelo_disponivel(nome_do_teste: &str) -> Option<PathBuf> {
    let pasta = pasta_modelos();
    if pasta.join("model_int8.onnx").is_file() && pasta.join("tokenizer.json").is_file() {
        return Some(pasta);
    }
    eprintln!("PULADO ({nome_do_teste}): modelo não instalado; rode `poph model install` antes");
    None
}

pub fn referencia() -> (Vec<String>, Vec<Vec<f32>>) {
    let bruto: Value =
        serde_json::from_str(include_str!("../fixtures/vetores_referencia_int8.json"))
            .expect("fixture de referência é JSON válido");
    let textos = bruto["textos"]
        .as_array()
        .expect("textos")
        .iter()
        .map(|t| t.as_str().expect("texto").to_owned())
        .collect();
    let vetores = bruto["vetores"]
        .as_array()
        .expect("vetores")
        .iter()
        .map(|v| {
            v.as_array()
                .expect("vetor")
                .iter()
                .map(|x| x.as_f64().expect("número") as f32)
                .collect()
        })
        .collect();
    (textos, vetores)
}

pub fn cosseno(a: &[f32], b: &[f32]) -> f32 {
    let produto: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum();
    let na: f64 = a.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    (produto / (na * nb)) as f32
}

/// Escritor de log que guarda tudo em memória, para checar o que aparece em `info`.
#[derive(Clone, Default)]
pub struct Captura(pub Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captura {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("trava").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
