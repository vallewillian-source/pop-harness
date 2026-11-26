//! Efeito da quantização int8 dos vetores (HR-16) com o modelo real. Compara a busca em ponto
//! flutuante com a busca em int8, sobre o corpus de 300 trechos de ~350 caracteres do spike HR-02
//! e as 20 consultas com gabarito do mesmo spike.
//!
//! Sem o modelo instalado, o teste PULA. Para rodar: `poph model install` e depois
//! `cargo test -p poph-rag --test quantizacao_modelo -- --nocapture`.

mod apoio;

use poph_rag::vector::{cosine, dot_product, norm, quantize};
use poph_rag::{Embedder, ModelConfig, OrtEmbedder};
use serde_json::Value;

use apoio::{cosseno as cosseno_f32, modelo_disponivel};

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../spike/hr02/fixtures/corpus350.json"
);
const CONSULTAS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../spike/hr02/fixtures/queries350.json"
);

/// Quantização antiga, com escala fixa (componente 1,0 vira 127). Só para comparar.
fn quantizar_escala_fixa(v: &[f32]) -> Vec<i8> {
    v.iter()
        .map(|x| (x * 127.0).round().clamp(-127.0, 127.0) as i8)
        .collect()
}

fn ordenar(scores: &[f32], k: usize) -> Vec<usize> {
    let mut ordem: Vec<usize> = (0..scores.len()).collect();
    ordem.sort_by(|a, b| {
        scores[*b]
            .partial_cmp(&scores[*a])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(b))
    });
    ordem.truncate(k);
    ordem
}

/// Ranking de consultas contra o corpus usando vetores int8 (já quantizados).
fn ranking_int8(consulta: &[i8], corpus: &[Vec<i8>], normas: &[f32], k: usize) -> Vec<usize> {
    let nq = norm(consulta);
    let scores: Vec<f32> = corpus
        .iter()
        .zip(normas)
        .map(|(c, nc)| cosine(dot_product(consulta, c), nq, *nc))
        .collect();
    ordenar(&scores, k)
}

#[tokio::test]
async fn quantizacao_int8_mantem_a_busca_do_corpus_do_spike() {
    let Some(pasta) = modelo_disponivel("quantizacao_int8_mantem_a_busca_do_corpus_do_spike")
    else {
        return;
    };
    let Ok(corpus_bruto) = std::fs::read_to_string(CORPUS) else {
        eprintln!("PULADO: corpus do spike não encontrado em {CORPUS}");
        return;
    };
    let Ok(consultas_bruto) = std::fs::read_to_string(CONSULTAS) else {
        eprintln!("PULADO: consultas do spike não encontradas em {CONSULTAS}");
        return;
    };
    // Cada trecho é um objeto com `text` (e `source`, de onde veio).
    let corpus: Vec<String> = serde_json::from_str::<Value>(&corpus_bruto)
        .expect("corpus é JSON")
        .as_array()
        .expect("lista de trechos")
        .iter()
        .map(|t| t["text"].as_str().expect("texto do trecho").to_owned())
        .collect();
    let consultas: Vec<(String, usize)> = serde_json::from_str::<Value>(&consultas_bruto)
        .expect("consultas são JSON")
        .as_array()
        .expect("lista de consultas")
        .iter()
        .map(|c| {
            (
                c["q"].as_str().expect("texto da consulta").to_owned(),
                c["gold"].as_u64().expect("gabarito") as usize,
            )
        })
        .collect();

    let modelo = OrtEmbedder::open_model(
        &ModelConfig {
            diretorio: pasta,
            threads: 4,
        },
        None,
    )
    .expect("abre o modelo instalado");
    let vetores_corpus = modelo
        .embed_batch(&corpus)
        .await
        .expect("vetores do corpus");
    let textos_consulta: Vec<String> = consultas.iter().map(|(q, _)| q.clone()).collect();
    let vetores_consulta = modelo
        .embed_batch(&textos_consulta)
        .await
        .expect("vetores das consultas");

    // Quantização de cada esquema.
    let fixa: Vec<Vec<i8>> = vetores_corpus
        .iter()
        .map(|v| quantizar_escala_fixa(v))
        .collect();
    let fixa_normas: Vec<f32> = fixa.iter().map(|q| norm(q)).collect();
    let maior: Vec<Vec<i8>> = vetores_corpus.iter().map(|v| quantize(v)).collect();
    let maior_normas: Vec<f32> = maior.iter().map(|q| norm(q)).collect();

    // Erro de cada vetor: cosseno entre o float e o int8 (o cosseno não depende da escala).
    let cos_medio = |quantizados: &[Vec<i8>]| -> f32 {
        vetores_corpus
            .iter()
            .zip(quantizados)
            .map(|(v, q)| {
                let como_float: Vec<f32> = q.iter().map(|x| f32::from(*x)).collect();
                cosseno_f32(v, &como_float)
            })
            .sum::<f32>()
            / vetores_corpus.len() as f32
    };

    let (mut acerto_f32, mut acerto_fixa, mut acerto_maior) = (0usize, 0usize, 0usize);
    let (mut top3_fixa, mut top3_maior) = (0usize, 0usize);
    let mut mesma_ordem_top3 = 0usize;
    for (i, (_, gold)) in consultas.iter().enumerate() {
        let consulta = &vetores_consulta[i];
        let scores_f32: Vec<f32> = vetores_corpus
            .iter()
            .map(|c| cosseno_f32(consulta, c))
            .collect();
        let r_f32 = ordenar(&scores_f32, 3);

        let r_fixa = ranking_int8(&quantizar_escala_fixa(consulta), &fixa, &fixa_normas, 3);
        let r_maior = ranking_int8(&quantize(consulta), &maior, &maior_normas, 3);

        acerto_f32 += usize::from(r_f32[0] == *gold);
        acerto_fixa += usize::from(r_fixa[0] == *gold);
        acerto_maior += usize::from(r_maior[0] == *gold);
        top3_fixa += usize::from(r_fixa.contains(gold));
        top3_maior += usize::from(r_maior.contains(gold));
        mesma_ordem_top3 += usize::from(r_maior == r_f32);
    }
    let concordancia = mesma_ordem_top3 as f32 / consultas.len() as f32;

    println!(
        "trechos do corpus: {}, consultas com gabarito: {}",
        corpus.len(),
        consultas.len()
    );
    println!(
        "cosseno médio float × int8, escala fixa (antiga): {:.5}",
        cos_medio(&fixa)
    );
    println!(
        "cosseno médio float × int8, pelo maior componente (atual): {:.5}",
        cos_medio(&maior)
    );
    println!(
        "acerto no 1º lugar, float: {acerto_f32}/{}",
        consultas.len()
    );
    println!(
        "acerto no 1º lugar, int8 escala fixa: {acerto_fixa}/{}",
        consultas.len()
    );
    println!(
        "acerto no 1º lugar, int8 pelo maior componente: {acerto_maior}/{}",
        consultas.len()
    );
    println!(
        "gabarito dentro do top-3, int8 escala fixa: {top3_fixa}/{}",
        consultas.len()
    );
    println!(
        "gabarito dentro do top-3, int8 pelo maior componente: {top3_maior}/{}",
        consultas.len()
    );
    println!("top-3 com a mesma ordem do float (int8 atual): {concordancia:.3}");

    assert!(
        cos_medio(&maior) > 0.9999,
        "a quantização atual distorce os vetores além do esperado"
    );
}
