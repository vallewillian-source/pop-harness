#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::Value;

pub const MIN_COSINE: f32 = 0.9999;
pub const RSS_SPIKE_BYTES: u64 = 438 * 1_000_000;

pub fn model_folder() -> PathBuf {
    match std::env::var_os("POPH_MODELOS_TESTE") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/modelos")),
    }
}

pub fn available_model(test_name: &str) -> Option<PathBuf> {
    let folder = model_folder();
    if folder.join("model_int8.onnx").is_file() && folder.join("tokenizer.json").is_file() {
        return Some(folder);
    }
    eprintln!("PULADO ({test_name}): modelo não instalado; rode `poph model install` antes");
    None
}

pub fn reference_vectors() -> (Vec<String>, Vec<Vec<f32>>) {
    let raw: Value = serde_json::from_str(include_str!("../fixtures/vetores_referencia_int8.json"))
        .expect("fixture de referência é JSON válido");
    let texts = raw["textos"]
        .as_array()
        .expect("textos")
        .iter()
        .map(|t| t.as_str().expect("texto").to_owned())
        .collect();
    let vectors = raw["vetores"]
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
    (texts, vectors)
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let product: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum();
    let norm_a: f64 = a.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    (product / (norm_a * norm_b)) as f32
}

#[derive(Clone, Default)]
pub struct LogCapture(pub Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogCapture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("trava").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
