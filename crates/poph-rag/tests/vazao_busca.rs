//! Latência da busca exata em int8 com 100 mil trechos de um agente (HR-16). Ignorado no CI: é
//! carga de verdade. Para rodar:
//! `cargo test --release -p poph-rag --test vazao_busca -- --ignored --nocapture`
//!
//! Usa o embedder falso: o tempo da varredura não depende do conteúdo do vetor, só do tamanho. O
//! tempo do modelo real (cerca de 13 ms por consulta, RELATORIO_HR02 §3.1) entra à parte.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use poph_rag::memory::rss_bytes;
use poph_rag::{ConfigIndice, FakeEmbedder, Indice, ItemRag};
use poph_types::{AgentId, KnowledgeItemId, OrgId};

const TRECHOS: usize = 100_000;
const CONSULTAS: usize = 300;

fn percentil(amostras: &mut [f64], p: f64) -> f64 {
    amostras.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let posicao = ((amostras.len() as f64 - 1.0) * p).round() as usize;
    amostras[posicao]
}

#[tokio::test]
#[ignore = "carga de 100 mil trechos: rodar com --release --ignored"]
async fn busca_exata_em_100_mil_trechos_int8() {
    let pasta: PathBuf =
        std::env::temp_dir().join(format!("poph-rag-vazao-{}", std::process::id()));
    fs::create_dir_all(&pasta).expect("pasta temporária");
    let caminho = pasta.join("rag.db");

    let rss_antes = rss_bytes();
    let (indice, _) = Indice::abrir(
        &caminho,
        Arc::new(FakeEmbedder::new()),
        ConfigIndice {
            orcamento_memoria_bytes: 1 << 30,
        },
    )
    .expect("abre o rag.db");
    let (org, agente) = (OrgId::generate(), AgentId::generate());

    let itens: Vec<ItemRag> = (0..TRECHOS)
        .map(|n| ItemRag {
            id: KnowledgeItemId::generate(),
            titulo: format!("Item {n}"),
            texto: format!(
                "Texto do item número {n}, com a palavra{} e o termo grupo{}.",
                n % 997,
                n % 53
            ),
            versao: 1,
        })
        .collect();
    let inicio = Instant::now();
    let relatorio = indice
        .indexar_agente(org, agente, &itens)
        .await
        .expect("indexa 100 mil itens");
    let indexacao_s = inicio.elapsed().as_secs_f64();
    assert_eq!(relatorio.trechos_gravados, TRECHOS);

    // Primeira consulta: carrega o agente do rag.db para a memória.
    let inicio = Instant::now();
    indice
        .buscar_vetor(org, agente, "palavra5 grupo3", 3)
        .await
        .expect("primeira busca");
    let carga_ms = inicio.elapsed().as_secs_f64() * 1000.0;
    let rss_depois_carga = rss_bytes();

    // Consultas em sequência, com o agente já na memória.
    let mut tempos_ms = Vec::with_capacity(CONSULTAS);
    let mut acertos = 0usize;
    for n in 0..CONSULTAS {
        let alvo = (n * 331) % TRECHOS;
        // Mesmo texto que foi embutido (título na frente): o vetor é igual, então o item sai primeiro.
        let consulta = format!(
            "Item {alvo}\nTexto do item número {alvo}, com a palavra{} e o termo grupo{}.",
            alvo % 997,
            alvo % 53
        );
        let inicio = Instant::now();
        let achados = indice
            .buscar_vetor(org, agente, &consulta, 3)
            .await
            .expect("busca");
        tempos_ms.push(inicio.elapsed().as_secs_f64() * 1000.0);
        // Texto idêntico ao do item: o vetor é igual, então o item tem que sair em primeiro.
        if achados.first().map(|a| a.trecho.titulo.clone()) == Some(format!("Item {alvo}")) {
            acertos += 1;
        }
    }
    let p50 = percentil(&mut tempos_ms.clone(), 0.50);
    let p95 = percentil(&mut tempos_ms, 0.95);

    println!("trechos no agente: {TRECHOS}");
    println!("indexação (embedder falso, 100 mil itens): {indexacao_s:.2} s");
    println!("primeira consulta (carrega o agente do rag.db): {carga_ms:.1} ms");
    println!("consultas medidas: {CONSULTAS}, p50 {p50:.2} ms, p95 {p95:.2} ms");
    println!("acerto do item exato no 1º lugar: {acertos}/{CONSULTAS}");
    println!(
        "memória do índice (estimativa pelos vetores): {:.1} MB ({} bytes por trecho)",
        indice.bytes_em_memoria() as f64 / 1e6,
        indice.bytes_em_memoria() / TRECHOS
    );
    if let (Some(antes), Some(depois)) = (rss_antes, rss_depois_carga) {
        println!(
            "RSS do processo depois da carga: +{:.1} MB em relação ao início do teste",
            (depois.saturating_sub(antes)) as f64 / 1e6
        );
    }

    assert_eq!(
        acertos, CONSULTAS,
        "o texto idêntico precisa sair em primeiro"
    );

    // Mesma escala que a HR-01 mediu (200 trechos por agente), para comparar com 13,5 µs p50.
    let (org_pequeno, agente_pequeno) = (OrgId::generate(), AgentId::generate());
    let pequenos: Vec<ItemRag> = (0..200)
        .map(|n| ItemRag {
            id: KnowledgeItemId::generate(),
            titulo: format!("Item {n}"),
            texto: format!("Texto do item número {n}, com a palavra{}.", n % 97),
            versao: 1,
        })
        .collect();
    indice
        .indexar_agente(org_pequeno, agente_pequeno, &pequenos)
        .await
        .expect("indexa 200 itens");
    indice
        .buscar_vetor(org_pequeno, agente_pequeno, "palavra3", 3)
        .await
        .expect("carrega o agente pequeno");
    let mut pequenos_ms = Vec::with_capacity(CONSULTAS);
    for n in 0..CONSULTAS {
        let alvo = (n * 7) % 200;
        let consulta = format!(
            "Item {alvo}\nTexto do item número {alvo}, com a palavra{}.",
            alvo % 97
        );
        let inicio = Instant::now();
        indice
            .buscar_vetor(org_pequeno, agente_pequeno, &consulta, 3)
            .await
            .expect("busca pequena");
        pequenos_ms.push(inicio.elapsed().as_secs_f64() * 1000.0);
    }
    let p50_pequeno = percentil(&mut pequenos_ms.clone(), 0.50);
    let p95_pequeno = percentil(&mut pequenos_ms, 0.95);
    println!(
        "agente de 200 trechos: p50 {:.4} ms, p95 {:.4} ms (HR-01 mediu 0,0135 ms p50 com o mesmo tamanho)",
        p50_pequeno, p95_pequeno
    );
    let _ = fs::remove_dir_all(&pasta);
}
