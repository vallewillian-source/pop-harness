//! RAM e latência do modelo carregado, e o log de carga sem caminho do modelo.
//!
//! Arquivo próprio de propósito: cada arquivo de teste de integração roda num processo, e a RAM
//! medida aqui só significa algo se o processo não carregou outros modelos antes.
//! O RSS só é comparável quando o crate é construído sozinho.

mod apoio;

use std::time::Instant;

use poph_rag::{ConfigModelo, Embedder, EmbedderOrt};

use apoio::{Captura, RSS_SPIKE_BYTES, modelo_disponivel, referencia};

#[tokio::test]
#[ignore = "medição de RAM: roda no passo próprio do CI, com -p poph-rag (meta provisória, oficial só na HR-55)"]
async fn ram_latencia_e_log_sem_caminho_do_modelo() {
    let Some(pasta) = modelo_disponivel("ram_latencia_e_log_sem_caminho_do_modelo") else {
        return;
    };
    let captura = Captura::default();
    let escritor = captura.clone();
    let assinante = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .with_writer(move || escritor.clone())
        .finish();
    let _guarda = tracing::subscriber::set_default(assinante);

    let caminho_texto = pasta.display().to_string();
    let inicio_abrir = Instant::now();
    let modelo = EmbedderOrt::abrir(
        &ConfigModelo {
            diretorio: pasta,
            threads: 4,
        },
        None,
    )
    .expect("abre o modelo");
    let abrir_ms = inicio_abrir.elapsed().as_secs_f64() * 1000.0;
    let carga = modelo.aquecer().await.expect("carrega a sessão");
    let (textos, _) = referencia();

    let mut um_texto = Vec::new();
    for _ in 0..20 {
        let inicio = Instant::now();
        modelo
            .embed_batch(std::slice::from_ref(&textos[0]))
            .await
            .expect("texto");
        um_texto.push(inicio.elapsed().as_secs_f64() * 1000.0);
    }
    let mut lote = Vec::new();
    for _ in 0..5 {
        let inicio = Instant::now();
        modelo.embed_batch(&textos).await.expect("lote");
        lote.push(inicio.elapsed().as_secs_f64() * 1000.0);
    }
    // Carga parecida com a do spike (trechos de ~1000 caracteres, que usam a janela de 128 tokens inteira).
    let base: Vec<char> = textos.join(" ").chars().collect();
    let longos: Vec<String> = (0..32)
        .map(|i| base.iter().cycle().skip(i * 7).take(1000).collect())
        .collect();
    let mut longo_um = Vec::new();
    for _ in 0..10 {
        let inicio = Instant::now();
        modelo
            .embed_batch(std::slice::from_ref(&longos[0]))
            .await
            .expect("texto longo");
        longo_um.push(inicio.elapsed().as_secs_f64() * 1000.0);
    }
    let mut longo_lote = Vec::new();
    for _ in 0..3 {
        let inicio = Instant::now();
        modelo.embed_batch(&longos).await.expect("lote longo");
        longo_lote.push(inicio.elapsed().as_secs_f64() * 1000.0);
    }

    um_texto.sort_by(|a, b| a.partial_cmp(b).expect("números"));
    lote.sort_by(|a, b| a.partial_cmp(b).expect("números"));
    longo_um.sort_by(|a, b| a.partial_cmp(b).expect("números"));
    longo_lote.sort_by(|a, b| a.partial_cmp(b).expect("números"));
    let p50_um = um_texto[um_texto.len() / 2];
    let p50_lote = lote[lote.len() / 2];
    let p50_longo_um = longo_um[longo_um.len() / 2];
    let p50_longo_lote = longo_lote[longo_lote.len() / 2];

    let delta = carga.delta_rss_bytes();
    println!("abertura (confere sha256 dos dois arquivos): {abrir_ms:.0} ms");
    println!(
        "carga: {:.2} s | RSS antes {:?} | depois {:?} | delta {:?} bytes",
        carga.segundos, carga.rss_antes_bytes, carga.rss_depois_bytes, delta
    );
    println!(
        "latência (CPU, 4 threads, janela 128): 1 texto p50 {p50_um:.1} ms | lote de {} p50 {p50_lote:.1} ms ({:.1} ms por trecho)",
        textos.len(),
        p50_lote / textos.len() as f64
    );
    println!(
        "trecho de ~1000 caracteres (128 tokens, como no spike): 1 texto p50 {p50_longo_um:.1} ms | lote de 32 p50 {p50_longo_lote:.1} ms ({:.1} ms por trecho, {:.0} trechos/s)",
        p50_longo_lote / 32.0,
        1000.0 / (p50_longo_lote / 32.0)
    );
    if let Some(rss) = carga.rss_depois_bytes {
        println!(
            "RSS depois da carga: {:.0} MB (spike HR-02: 438 MB)",
            rss as f64 / 1_000_000.0
        );
        // Teto da HR-15: RSS depois da carga não pode passar do valor medido no spike (com 10% de folga para o harness de teste).
        assert!(
            (rss as f64) <= RSS_SPIKE_BYTES as f64 * 1.10,
            "RSS do modelo acima da referência do spike: {rss} bytes"
        );
    }

    let log = String::from_utf8(captura.0.lock().expect("trava").clone()).expect("UTF-8");
    assert!(
        log.contains("modelo de embedding carregado"),
        "log de carga ausente: {log}"
    );
    assert!(
        !log.contains(&caminho_texto),
        "caminho do modelo apareceu no log de info"
    );
}
