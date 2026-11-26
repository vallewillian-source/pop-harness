mod support;

use poph_rag::vector::{cosine, dot_product, norm, quantize};
use poph_rag::{Embedder, ModelConfig, OrtEmbedder};
use serde_json::Value;

use support::{available_model, cosine as cosine_f32};

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../spike/hr02/fixtures/corpus350.json"
);
const QUERIES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../spike/hr02/fixtures/queries350.json"
);

fn quantize_fixed_scale(v: &[f32]) -> Vec<i8> {
    v.iter()
        .map(|x| (x * 127.0).round().clamp(-127.0, 127.0) as i8)
        .collect()
}

fn rank_scores(scores: &[f32], k: usize) -> Vec<usize> {
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

fn ranking_int8(query: &[i8], corpus: &[Vec<i8>], norms: &[f32], k: usize) -> Vec<usize> {
    let nq = norm(query);
    let scores: Vec<f32> = corpus
        .iter()
        .zip(norms)
        .map(|(c, nc)| cosine(dot_product(query, c), nq, *nc))
        .collect();
    rank_scores(&scores, k)
}

#[tokio::test]
async fn int8_quantization_preserves_search_on_spike_corpus() {
    let Some(folder) = available_model("quantizacao_int8_mantem_a_busca_do_corpus_do_spike") else {
        return;
    };
    let Ok(raw_corpus) = std::fs::read_to_string(CORPUS) else {
        eprintln!("PULADO: corpus do spike não encontrado em {CORPUS}");
        return;
    };
    let Ok(raw_queries) = std::fs::read_to_string(QUERIES) else {
        eprintln!("PULADO: consultas do spike não encontradas em {QUERIES}");
        return;
    };
    let corpus: Vec<String> = serde_json::from_str::<Value>(&raw_corpus)
        .expect("corpus é JSON")
        .as_array()
        .expect("lista de trechos")
        .iter()
        .map(|t| t["text"].as_str().expect("texto do trecho").to_owned())
        .collect();
    let queries: Vec<(String, usize)> = serde_json::from_str::<Value>(&raw_queries)
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
            diretorio: folder,
            threads: 4,
        },
        None,
    )
    .expect("abre o modelo instalado");
    let corpus_vectors = modelo
        .embed_batch(&corpus)
        .await
        .expect("vetores do corpus");
    let query_texts: Vec<String> = queries.iter().map(|(q, _)| q.clone()).collect();
    let query_vectors = modelo
        .embed_batch(&query_texts)
        .await
        .expect("vetores das consultas");

    let fixed: Vec<Vec<i8>> = corpus_vectors
        .iter()
        .map(|v| quantize_fixed_scale(v))
        .collect();
    let fixed_norms: Vec<f32> = fixed.iter().map(|q| norm(q)).collect();
    let maior: Vec<Vec<i8>> = corpus_vectors.iter().map(|v| quantize(v)).collect();
    let scaled_norms: Vec<f32> = maior.iter().map(|q| norm(q)).collect();

    let mean_cosine = |quantized: &[Vec<i8>]| -> f32 {
        corpus_vectors
            .iter()
            .zip(quantized)
            .map(|(v, q)| {
                let as_float: Vec<f32> = q.iter().map(|x| f32::from(*x)).collect();
                cosine_f32(v, &as_float)
            })
            .sum::<f32>()
            / corpus_vectors.len() as f32
    };

    let (mut float_hits, mut fixed_hits, mut scaled_hits) = (0usize, 0usize, 0usize);
    let (mut fixed_top_three, mut scaled_top_three) = (0usize, 0usize);
    let mut same_top_three_order = 0usize;
    for (i, (_, gold)) in queries.iter().enumerate() {
        let query = &query_vectors[i];
        let scores_f32: Vec<f32> = corpus_vectors
            .iter()
            .map(|c| cosine_f32(query, c))
            .collect();
        let float_rank = rank_scores(&scores_f32, 3);

        let fixed_rank = ranking_int8(&quantize_fixed_scale(query), &fixed, &fixed_norms, 3);
        let scaled_rank = ranking_int8(&quantize(query), &maior, &scaled_norms, 3);

        float_hits += usize::from(float_rank[0] == *gold);
        fixed_hits += usize::from(fixed_rank[0] == *gold);
        scaled_hits += usize::from(scaled_rank[0] == *gold);
        fixed_top_three += usize::from(fixed_rank.contains(gold));
        scaled_top_three += usize::from(scaled_rank.contains(gold));
        same_top_three_order += usize::from(scaled_rank == float_rank);
    }
    let agreement = same_top_three_order as f32 / queries.len() as f32;

    println!(
        "trechos do corpus: {}, consultas com gabarito: {}",
        corpus.len(),
        queries.len()
    );
    println!(
        "cosseno médio float × int8, escala fixa (antiga): {:.5}",
        mean_cosine(&fixed)
    );
    println!(
        "cosseno médio float × int8, pelo maior componente (atual): {:.5}",
        mean_cosine(&maior)
    );
    println!("acerto no 1º lugar, float: {float_hits}/{}", queries.len());
    println!(
        "acerto no 1º lugar, int8 escala fixa: {fixed_hits}/{}",
        queries.len()
    );
    println!(
        "acerto no 1º lugar, int8 pelo maior componente: {scaled_hits}/{}",
        queries.len()
    );
    println!(
        "gabarito dentro do top-3, int8 escala fixa: {fixed_top_three}/{}",
        queries.len()
    );
    println!(
        "gabarito dentro do top-3, int8 pelo maior componente: {scaled_top_three}/{}",
        queries.len()
    );
    println!("top-3 com a mesma ordem do float (int8 atual): {agreement:.3}");

    assert!(
        mean_cosine(&maior) > 0.9999,
        "a quantização atual distorce os vetores além do esperado"
    );
}
