mod support;

use poph_rag::{Embedder, ModelConfig, OrtEmbedder};

use support::{MIN_COSINE, available_model, cosine, reference_vectors};

#[tokio::test]
async fn vectors_match_spike_reference() {
    let Some(folder) = available_model("vetores_batem_com_a_referencia_do_spike") else {
        return;
    };
    let modelo = OrtEmbedder::open_model(
        &ModelConfig {
            diretorio: folder,
            threads: 4,
        },
        None,
    )
    .expect("abre o modelo instalado");
    let (texts, reference_vectors) = reference_vectors();

    let actual_vectors = modelo.embed_batch(&texts).await.expect("gera os vetores");

    assert_eq!(actual_vectors.len(), reference_vectors.len());
    let mut worst = 1.0f32;
    for (expected, actual) in reference_vectors.iter().zip(&actual_vectors) {
        assert_eq!(actual.len(), 384, "dimensão dos vetores");
        worst = worst.min(cosine(expected, actual));
    }
    println!("vetores: pior cosseno contra a referência = {worst:.7} (mínimo {MIN_COSINE})");
    assert!(
        worst >= MIN_COSINE,
        "vetor diferente da referência do spike: cosseno {worst}"
    );
}

#[tokio::test]
async fn batch_and_single_text_return_same_vector() {
    let Some(folder) = available_model("lote_e_texto_unico_dao_o_mesmo_vetor") else {
        return;
    };
    let modelo = OrtEmbedder::open_model(
        &ModelConfig {
            diretorio: folder,
            threads: 4,
        },
        None,
    )
    .expect("abre o modelo");
    let (texts, _) = reference_vectors();

    let batched = modelo.embed_batch(&texts).await.expect("lote");
    let mut worst = 1.0f32;
    for (index, texto) in texts.iter().enumerate() {
        let single = modelo
            .embed_batch(std::slice::from_ref(texto))
            .await
            .expect("texto único");
        worst = worst.min(cosine(&batched[index], &single[0]));
    }
    println!("lote x texto único: pior cosseno = {worst:.7}");
    assert!(
        worst >= MIN_COSINE,
        "o preenchimento do lote muda o vetor: cosseno {worst}"
    );
}

#[tokio::test]
async fn repeated_text_returns_same_vector() {
    let Some(folder) = available_model("mesmo_texto_mesmo_vetor_nas_duas_vezes") else {
        return;
    };
    let modelo = OrtEmbedder::open_model(
        &ModelConfig {
            diretorio: folder,
            threads: 4,
        },
        None,
    )
    .expect("abre o modelo");
    let (texts, _) = reference_vectors();

    let first = modelo.embed_batch(&texts).await.expect("primeira");
    let second = modelo.embed_batch(&texts).await.expect("segunda");
    assert_eq!(
        first, second,
        "a mesma entrada deve dar o mesmo vetor, bit a bit"
    );
}
