use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use poph_rag::memory::rss_bytes;
use poph_rag::{FakeEmbedder, Index, IndexConfig, ItemRag};
use poph_types::{AgentId, KnowledgeItemId, OrgId};

const CHUNKS: usize = 100_000;
const QUERIES: usize = 300;

fn percentile(samples: &mut [f64], p: f64) -> f64 {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let position = ((samples.len() as f64 - 1.0) * p).round() as usize;
    samples[position]
}

#[tokio::test]
#[ignore = "carga de 100 mil trechos: rodar com --release --ignored"]
async fn exact_search_over_hundred_thousand_int8_chunks() {
    let folder: PathBuf =
        std::env::temp_dir().join(format!("poph-rag-vazao-{}", std::process::id()));
    fs::create_dir_all(&folder).expect("pasta temporária");
    let path = folder.join("rag.db");

    let rss_before = rss_bytes();
    let (index, _) = Index::open(
        &path,
        Arc::new(FakeEmbedder::new()),
        IndexConfig {
            memory_budget_bytes: 1 << 30,
        },
    )
    .expect("abre o rag.db");
    let (org, agente) = (OrgId::generate(), AgentId::generate());

    let items: Vec<ItemRag> = (0..CHUNKS)
        .map(|n| ItemRag {
            id: KnowledgeItemId::generate(),
            title: format!("Item {n}"),
            texto: format!(
                "Texto do item número {n}, com a palavra{} e o termo grupo{}.",
                n % 997,
                n % 53
            ),
            versao: 1,
        })
        .collect();
    let start = Instant::now();
    let report = index
        .index_agent(org, agente, &items)
        .await
        .expect("indexa 100 mil itens");
    let indexing_seconds = start.elapsed().as_secs_f64();
    assert_eq!(report.stored_chunks, CHUNKS);

    let start = Instant::now();
    index
        .search_vector(org, agente, "palavra5 grupo3", 3)
        .await
        .expect("primeira busca");
    let load_ms = start.elapsed().as_secs_f64() * 1000.0;
    let rss_after_load = rss_bytes();

    let mut timings_ms = Vec::with_capacity(QUERIES);
    let mut matches = 0usize;
    for n in 0..QUERIES {
        let target = (n * 331) % CHUNKS;
        let query = format!(
            "Item {target}\nTexto do item número {target}, com a palavra{} e o termo grupo{}.",
            target % 997,
            target % 53
        );
        let start = Instant::now();
        let hits = index
            .search_vector(org, agente, &query, 3)
            .await
            .expect("busca");
        timings_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        if hits.first().map(|a| a.chunk.title.clone()) == Some(format!("Item {target}")) {
            matches += 1;
        }
    }
    let p50 = percentile(&mut timings_ms.clone(), 0.50);
    let p95 = percentile(&mut timings_ms, 0.95);

    println!("trechos no agente: {CHUNKS}");
    println!("indexação (embedder falso, 100 mil itens): {indexing_seconds:.2} s");
    println!("primeira consulta (carrega o agente do rag.db): {load_ms:.1} ms");
    println!("consultas medidas: {QUERIES}, p50 {p50:.2} ms, p95 {p95:.2} ms");
    println!("acerto do item exato no 1º lugar: {matches}/{QUERIES}");
    println!(
        "memória do índice (estimativa pelos vetores): {:.1} MB ({} bytes por trecho)",
        index.bytes_in_memory() as f64 / 1e6,
        index.bytes_in_memory() / CHUNKS
    );
    if let (Some(before), Some(after)) = (rss_before, rss_after_load) {
        println!(
            "RSS do processo depois da carga: +{:.1} MB em relação ao início do teste",
            (after.saturating_sub(before)) as f64 / 1e6
        );
    }

    assert_eq!(
        matches, QUERIES,
        "o texto idêntico precisa sair em primeiro"
    );

    let (small_org, small_agent) = (OrgId::generate(), AgentId::generate());
    let small_items: Vec<ItemRag> = (0..200)
        .map(|n| ItemRag {
            id: KnowledgeItemId::generate(),
            title: format!("Item {n}"),
            texto: format!("Texto do item número {n}, com a palavra{}.", n % 97),
            versao: 1,
        })
        .collect();
    index
        .index_agent(small_org, small_agent, &small_items)
        .await
        .expect("indexa 200 itens");
    index
        .search_vector(small_org, small_agent, "palavra3", 3)
        .await
        .expect("carrega o agente pequeno");
    let mut small_timings_ms = Vec::with_capacity(QUERIES);
    for n in 0..QUERIES {
        let target = (n * 7) % 200;
        let query = format!(
            "Item {target}\nTexto do item número {target}, com a palavra{}.",
            target % 97
        );
        let start = Instant::now();
        index
            .search_vector(small_org, small_agent, &query, 3)
            .await
            .expect("busca pequena");
        small_timings_ms.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let p50_small = percentile(&mut small_timings_ms.clone(), 0.50);
    let p95_small = percentile(&mut small_timings_ms, 0.95);
    println!(
        "agente de 200 trechos: p50 {:.4} ms, p95 {:.4} ms (HR-01 mediu 0,0135 ms p50 com o mesmo tamanho)",
        p50_small, p95_small
    );
    let _ = fs::remove_dir_all(&folder);
}
