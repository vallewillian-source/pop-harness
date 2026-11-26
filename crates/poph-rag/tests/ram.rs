mod support;

use std::time::Instant;

use poph_rag::{Embedder, ModelConfig, OrtEmbedder};

use support::{LogCapture, RSS_SPIKE_BYTES, available_model, reference_vectors};

#[tokio::test]
#[ignore = "medição de RAM: roda no passo próprio do CI, com -p poph-rag (meta provisória, oficial só na HR-55)"]
async fn ram_latency_and_log_without_model_path() {
    let Some(folder) = available_model("ram_latencia_e_log_sem_caminho_do_modelo") else {
        return;
    };
    let capture = LogCapture::default();
    let writer = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let path_text = folder.display().to_string();
    let open_start = Instant::now();
    let modelo = OrtEmbedder::open_model(
        &ModelConfig {
            diretorio: folder,
            threads: 4,
        },
        None,
    )
    .expect("abre o modelo");
    let open_ms = open_start.elapsed().as_secs_f64() * 1000.0;
    let load = modelo.warm_up().await.expect("carrega a sessão");
    let (texts, _) = reference_vectors();

    let mut single_text = Vec::new();
    for _ in 0..20 {
        let start = Instant::now();
        modelo
            .embed_batch(std::slice::from_ref(&texts[0]))
            .await
            .expect("texto");
        single_text.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let mut batch = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        modelo.embed_batch(&texts).await.expect("lote");
        batch.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let base: Vec<char> = texts.join(" ").chars().collect();
    let long_texts: Vec<String> = (0..32)
        .map(|i| base.iter().cycle().skip(i * 7).take(1000).collect())
        .collect();
    let mut long_single = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();
        modelo
            .embed_batch(std::slice::from_ref(&long_texts[0]))
            .await
            .expect("texto longo");
        long_single.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let mut long_batch = Vec::new();
    for _ in 0..3 {
        let start = Instant::now();
        modelo.embed_batch(&long_texts).await.expect("lote longo");
        long_batch.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    single_text.sort_by(|a, b| a.partial_cmp(b).expect("números"));
    batch.sort_by(|a, b| a.partial_cmp(b).expect("números"));
    long_single.sort_by(|a, b| a.partial_cmp(b).expect("números"));
    long_batch.sort_by(|a, b| a.partial_cmp(b).expect("números"));
    let p50_single = single_text[single_text.len() / 2];
    let p50_batch = batch[batch.len() / 2];
    let p50_long_single = long_single[long_single.len() / 2];
    let p50_long_batch = long_batch[long_batch.len() / 2];

    let delta = load.delta_rss_bytes();
    println!("abertura (confere sha256 dos dois arquivos): {open_ms:.0} ms");
    println!(
        "carga: {:.2} s | RSS antes {:?} | depois {:?} | delta {:?} bytes",
        load.seconds, load.rss_before_bytes, load.rss_after_bytes, delta
    );
    println!(
        "latência (CPU, 4 threads, janela 128): 1 texto p50 {p50_single:.1} ms | lote de {} p50 {p50_batch:.1} ms ({:.1} ms por trecho)",
        texts.len(),
        p50_batch / texts.len() as f64
    );
    println!(
        "trecho de ~1000 caracteres (128 tokens, como no spike): 1 texto p50 {p50_long_single:.1} ms | lote de 32 p50 {p50_long_batch:.1} ms ({:.1} ms por trecho, {:.0} trechos/s)",
        p50_long_batch / 32.0,
        1000.0 / (p50_long_batch / 32.0)
    );
    if let Some(rss) = load.rss_after_bytes {
        println!(
            "RSS depois da carga: {:.0} MB (spike HR-02: 438 MB)",
            rss as f64 / 1_000_000.0
        );
        assert!(
            (rss as f64) <= RSS_SPIKE_BYTES as f64 * 1.10,
            "RSS do modelo acima da referência do spike: {rss} bytes"
        );
    }

    let log = String::from_utf8(capture.0.lock().expect("trava").clone()).expect("UTF-8");
    assert!(
        log.contains("modelo de embedding carregado"),
        "log de carga ausente: {log}"
    );
    assert!(
        !log.contains(&path_text),
        "caminho do modelo apareceu no log de info"
    );
}
