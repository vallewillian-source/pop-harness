//! Vetores do embedding contra a referência do spike HR-02, com o modelo real (int8).
//!
//! Usa o modelo da pasta `data/modelos` do Harness (ou `POPH_MODELOS_TESTE`). Sem o modelo
//! instalado, cada teste PULA com mensagem (não falha o CI e não baixa nada). Para rodar de
//! verdade: `poph model install` e depois `cargo test -p poph-rag -- --nocapture`.

mod apoio;

use poph_rag::{Embedder, ModelConfig, OrtEmbedder};

use apoio::{COSSENO_MINIMO, cosseno, modelo_disponivel, referencia};

#[tokio::test]
async fn vetores_batem_com_a_referencia_do_spike() {
    let Some(pasta) = modelo_disponivel("vetores_batem_com_a_referencia_do_spike") else {
        return;
    };
    let modelo = OrtEmbedder::open_model(
        &ModelConfig {
            diretorio: pasta,
            threads: 4,
        },
        None,
    )
    .expect("abre o modelo instalado");
    let (textos, referencia) = referencia();

    let obtidos = modelo.embed_batch(&textos).await.expect("gera os vetores");

    assert_eq!(obtidos.len(), referencia.len());
    let mut pior = 1.0f32;
    for (esperado, obtido) in referencia.iter().zip(&obtidos) {
        assert_eq!(obtido.len(), 384, "dimensão dos vetores");
        pior = pior.min(cosseno(esperado, obtido));
    }
    println!("vetores: pior cosseno contra a referência = {pior:.7} (mínimo {COSSENO_MINIMO})");
    assert!(
        pior >= COSSENO_MINIMO,
        "vetor diferente da referência do spike: cosseno {pior}"
    );
}

#[tokio::test]
async fn lote_e_texto_unico_dao_o_mesmo_vetor() {
    let Some(pasta) = modelo_disponivel("lote_e_texto_unico_dao_o_mesmo_vetor") else {
        return;
    };
    let modelo = OrtEmbedder::open_model(
        &ModelConfig {
            diretorio: pasta,
            threads: 4,
        },
        None,
    )
    .expect("abre o modelo");
    let (textos, _) = referencia();

    let em_lote = modelo.embed_batch(&textos).await.expect("lote");
    let mut pior = 1.0f32;
    for (indice, texto) in textos.iter().enumerate() {
        let sozinho = modelo
            .embed_batch(std::slice::from_ref(texto))
            .await
            .expect("texto único");
        pior = pior.min(cosseno(&em_lote[indice], &sozinho[0]));
    }
    println!("lote x texto único: pior cosseno = {pior:.7}");
    assert!(
        pior >= COSSENO_MINIMO,
        "o preenchimento do lote muda o vetor: cosseno {pior}"
    );
}

#[tokio::test]
async fn mesmo_texto_mesmo_vetor_nas_duas_vezes() {
    let Some(pasta) = modelo_disponivel("mesmo_texto_mesmo_vetor_nas_duas_vezes") else {
        return;
    };
    let modelo = OrtEmbedder::open_model(
        &ModelConfig {
            diretorio: pasta,
            threads: 4,
        },
        None,
    )
    .expect("abre o modelo");
    let (textos, _) = referencia();

    let primeira = modelo.embed_batch(&textos).await.expect("primeira");
    let segunda = modelo.embed_batch(&textos).await.expect("segunda");
    assert_eq!(
        primeira, segunda,
        "a mesma entrada deve dar o mesmo vetor, bit a bit"
    );
}
