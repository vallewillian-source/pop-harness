use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use poph_rag::{
    Embedder, FakeEmbedder, Index, IndexConfig, ModelConfig, OrtEmbedder, ProductRag, SearchConfig,
};
use poph_types::{AgentId, OrgId, ProductId};
use serde_json::Value;

fn model_folder() -> PathBuf {
    match std::env::var_os("POPH_MODELOS_TESTE") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/modelos")),
    }
}

fn available_model(nome: &str) -> Option<PathBuf> {
    let folder = model_folder();
    if folder.join("model_int8.onnx").is_file() && folder.join("tokenizer.json").is_file() {
        return Some(folder);
    }
    eprintln!("PULADO ({nome}): modelo não instalado; rode `poph model install` antes");
    None
}

struct TempFolder(PathBuf);

impl TempFolder {
    fn new(nome: &str) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path =
            std::env::temp_dir().join(format!("poph-produtos-{nome}-{}-{now}", std::process::id()));
        fs::create_dir_all(&path).expect("cria a pasta temporária");
        Self(path)
    }

    fn rag_db(&self) -> PathBuf {
        self.0.join("rag.db")
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Labeled {
    id: String,
    nome: String,
    aliases: Vec<String>,
    descricao: String,
    tags: Vec<String>,
}

fn fixtura() -> (Vec<Labeled>, Vec<(String, Vec<String>)>) {
    let raw: Value = serde_json::from_str(include_str!("fixtures/produtos_rotulados.json"))
        .expect("fixtura é JSON válido");
    let products = raw["produtos"]
        .as_array()
        .expect("produtos")
        .iter()
        .map(|product| Labeled {
            id: product["id"].as_str().expect("id").to_owned(),
            nome: product["nome"].as_str().expect("nome").to_owned(),
            aliases: string_list(&product["aliases"]),
            descricao: product["descricao"].as_str().expect("descricao").to_owned(),
            tags: string_list(&product["tags"]),
        })
        .collect();
    let queries = raw["consultas"]
        .as_array()
        .expect("consultas")
        .iter()
        .map(|query| {
            (
                query["consulta"].as_str().expect("consulta").to_owned(),
                string_list(&query["esperado"]),
            )
        })
        .collect();
    (products, queries)
}

fn string_list(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("lista")
        .iter()
        .map(|item| item.as_str().expect("texto").to_owned())
        .collect()
}

fn product_text(product: &Labeled) -> String {
    let mut texto = product.nome.clone();
    if !product.aliases.is_empty() {
        texto.push_str(&format!(
            "\nTambém chamado de: {}",
            product.aliases.join(", ")
        ));
    }
    if !product.descricao.trim().is_empty() {
        texto.push_str(&format!("\nDescrição: {}", product.descricao.trim()));
    }
    if !product.tags.is_empty() {
        texto.push_str(&format!("\nTags: {}", product.tags.join(", ")));
    }
    texto
}

#[tokio::test]
async fn product_hybrid_matches_or_beats_vector_only_in_top_three() {
    let Some(model_folder) = available_model("produtos_hibrida") else {
        return;
    };
    let (products, queries) = fixtura();
    assert_eq!(products.len(), 12, "a fixtura tem 12 produtos");
    assert_eq!(queries.len(), 20, "a fixtura tem 20 consultas");

    let folder = TempFolder::new("top3");
    let embedder: Arc<dyn Embedder> = Arc::new(
        OrtEmbedder::open_model(
            &ModelConfig {
                diretorio: model_folder,
                ..ModelConfig::default()
            },
            None,
        )
        .expect("abre o modelo"),
    );
    let (index, _) =
        Index::open(&folder.rag_db(), embedder, IndexConfig::default()).expect("abre o índice");
    let org = OrgId::generate();
    let agente = AgentId::generate();

    let mut item_ids: Vec<(String, ProductId)> = Vec::new();
    let mut for_index = Vec::new();
    for product in &products {
        let id = ProductId::generate();
        item_ids.push((product.id.clone(), id));
        for_index.push(ProductRag {
            id,
            nome: product.nome.clone(),
            texto: product_text(product),
            versao: 1,
        });
    }
    index
        .index_products(org, agente, &for_index)
        .await
        .expect("indexa a fixtura");

    let config = SearchConfig {
        top_k: 3,
        cutoff: 0.0,
        depth: 20,
    };
    let mut vector_hits = 0;
    let mut hybrid_hits = 0;
    let mut vector_misses = Vec::new();
    let mut hybrid_misses = Vec::new();
    for (query, expected_labels) in &queries {
        let expected: Vec<ProductId> = item_ids
            .iter()
            .filter(|(label, _)| expected_labels.contains(label))
            .map(|(_, id)| *id)
            .collect();

        let vector = index
            .search_products_vector(org, agente, query, 3)
            .await
            .expect("busca vetorial");
        if vector.iter().any(|hit| expected.contains(&hit.product)) {
            vector_hits += 1;
        } else {
            vector_misses.push(query.clone());
        }

        let hybrid = index
            .search_products(org, agente, query, &config)
            .await
            .expect("busca híbrida");
        if hybrid.iter().any(|hit| expected.contains(&hit.product)) {
            hybrid_hits += 1;
        } else {
            hybrid_misses.push(query.clone());
        }
    }

    let total = queries.len();
    eprintln!(
        "acerto top-3 (produtos): só vetor {vector_hits}/{total} ({:.1}%); híbrida \
         {hybrid_hits}/{total} ({:.1}%)",
        vector_hits as f64 * 100.0 / total as f64,
        hybrid_hits as f64 * 100.0 / total as f64,
    );
    if !vector_misses.is_empty() {
        eprintln!("falhas do vetor: {vector_misses:?}");
    }
    if !hybrid_misses.is_empty() {
        eprintln!("falhas da híbrida: {hybrid_misses:?}");
    }
    assert!(
        hybrid_hits >= vector_hits,
        "a busca híbrida de produtos precisa acertar pelo menos tanto quanto a vetorial \
         (híbrida {hybrid_hits}, vetor {vector_hits})"
    );
}

#[tokio::test]
async fn changing_price_keeps_index_hash() {
    let folder = TempFolder::new("preco");
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new());
    let (index, _) =
        Index::open(&folder.rag_db(), embedder, IndexConfig::default()).expect("abre o índice");
    let org = OrgId::generate();
    let agente = AgentId::generate();
    let product = ProductRag {
        id: ProductId::generate(),
        nome: "Dedetização de baratas".to_owned(),
        texto: "Dedetização de baratas\nTambém chamado de: baratas, barata".to_owned(),
        versao: 1,
    };
    index
        .index_products(org, agente, std::slice::from_ref(&product))
        .await
        .expect("indexa");
    let before = index.indexed_products(org, agente).expect("lê os hashes");

    let after = index
        .index_products(org, agente, std::slice::from_ref(&product))
        .await
        .expect("reindexa");
    assert_eq!(after.changed_items, 0);
    assert_eq!(after.generated_embeddings, 0, "nada foi reembedado");
    assert_eq!(before, index.indexed_products(org, agente).expect("hashes"));
}

#[tokio::test]
async fn product_removed_from_list_disappears_from_index() {
    let folder = TempFolder::new("sai");
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new());
    let (index, _) =
        Index::open(&folder.rag_db(), embedder, IndexConfig::default()).expect("abre o índice");
    let org = OrgId::generate();
    let agente = AgentId::generate();
    let roaches = ProductRag {
        id: ProductId::generate(),
        nome: "Dedetização de baratas".to_owned(),
        texto: "Dedetização de baratas\nTambém chamado de: baratas".to_owned(),
        versao: 1,
    };
    let termites = ProductRag {
        id: ProductId::generate(),
        nome: "Dedetização de cupins".to_owned(),
        texto: "Dedetização de cupins\nTambém chamado de: cupins".to_owned(),
        versao: 1,
    };
    index
        .index_products(org, agente, &[roaches.clone(), termites.clone()])
        .await
        .expect("indexa");
    let config = SearchConfig {
        top_k: 5,
        cutoff: 0.0,
        depth: 20,
    };
    let hits = index
        .search_products(org, agente, "cupins", &config)
        .await
        .expect("busca");
    assert!(hits.iter().any(|hit| hit.product == termites.id));

    let report = index
        .index_products(org, agente, std::slice::from_ref(&roaches))
        .await
        .expect("reindexa");
    assert_eq!(report.removed_items, 1);
    let hits = index
        .search_products(org, agente, "cupins", &config)
        .await
        .expect("busca");
    assert!(
        hits.iter().all(|hit| hit.product != termites.id),
        "produto fora da lista não volta da busca"
    );
}
