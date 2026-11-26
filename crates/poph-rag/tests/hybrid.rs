use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use poph_rag::{
    Embedder, FakeEmbedder, Index, IndexConfig, ItemRag, ModelConfig, OrtEmbedder, SearchConfig,
};
use poph_types::{AgentId, KnowledgeItemId, OrgId};
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
            std::env::temp_dir().join(format!("poph-hibrida-{nome}-{}-{now}", std::process::id()));
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
    title: String,
    texto: String,
}

fn fixtura() -> (Vec<Labeled>, Vec<(String, Vec<String>)>) {
    let raw: Value = serde_json::from_str(include_str!("fixtures/perguntas_rotuladas.json"))
        .expect("fixtura é JSON válido");
    let items = raw["itens"]
        .as_array()
        .expect("itens")
        .iter()
        .map(|item| Labeled {
            id: item["id"].as_str().expect("id").to_owned(),
            title: item["titulo"].as_str().expect("titulo").to_owned(),
            texto: item["texto"].as_str().expect("texto").to_owned(),
        })
        .collect();
    let questions = raw["perguntas"]
        .as_array()
        .expect("perguntas")
        .iter()
        .map(|question| {
            (
                question["pergunta"].as_str().expect("pergunta").to_owned(),
                question["esperado"]
                    .as_array()
                    .expect("esperado")
                    .iter()
                    .map(|id| id.as_str().expect("id esperado").to_owned())
                    .collect(),
            )
        })
        .collect();
    (items, questions)
}

fn item_id(_text: &str) -> KnowledgeItemId {
    KnowledgeItemId::generate()
}

#[tokio::test]
async fn hybrid_matches_or_beats_vector_only_in_top_three() {
    let Some(model_folder) = available_model("hibrida_acerta_top3") else {
        return;
    };
    let (items, questions) = fixtura();
    assert_eq!(items.len(), 24);
    assert_eq!(questions.len(), 30);

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

    let mut item_ids = Vec::new();
    let mut for_index = Vec::new();
    for item in &items {
        let id = item_id(&item.id);
        item_ids.push((item.id.clone(), id));
        for_index.push(ItemRag {
            id,
            title: item.title.clone(),
            texto: item.texto.clone(),
            versao: 1,
        });
    }
    index
        .index_agent(org, agente, &for_index)
        .await
        .expect("indexa a fixtura");

    let mut vector_hits = 0;
    let mut hybrid_hits = 0;
    let mut vector_misses = Vec::new();
    let mut hybrid_misses = Vec::new();
    for (question, expected_labels) in &questions {
        let expected: Vec<KnowledgeItemId> = item_ids
            .iter()
            .filter(|(label, _)| expected_labels.contains(label))
            .map(|(_, id)| *id)
            .collect();

        let vector = index
            .search_vector(org, agente, question, 3)
            .await
            .expect("busca vetorial");
        let vector_matched = vector.iter().any(|hit| expected.contains(&hit.chunk.item));
        if vector_matched {
            vector_hits += 1;
        } else {
            vector_misses.push(question.clone());
        }

        let hybrid = index
            .search_hybrid(
                org,
                agente,
                question,
                &SearchConfig {
                    top_k: 3,
                    cutoff: 0.0,
                    depth: 20,
                },
            )
            .await
            .expect("busca híbrida");
        let hybrid_matched = hybrid.iter().any(|hit| expected.contains(&hit.chunk.item));
        if hybrid_matched {
            hybrid_hits += 1;
        } else {
            hybrid_misses.push(question.clone());
        }
    }

    let total = questions.len();
    eprintln!(
        "acerto top-3: só vetor {vector_hits}/{total} ({:.1}%); híbrida {hybrid_hits}/{total} ({:.1}%)",
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
        "a busca híbrida precisa acertar pelo menos tanto quanto a vetorial \
         (híbrida {hybrid_hits}, vetor {vector_hits})"
    );
}

#[tokio::test]
async fn relevance_cutoff_excludes_unrelated_chunk() {
    let folder = TempFolder::new("corte");
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new());
    let (index, _) =
        Index::open(&folder.rag_db(), embedder, IndexConfig::default()).expect("abre o índice");
    let org = OrgId::generate();
    let agente = AgentId::generate();
    let item = item_id("prazo");
    index
        .index_agent(
            org,
            agente,
            &[ItemRag {
                id: item,
                title: "Prazo de agendamento".to_owned(),
                texto: "O prazo para agendar uma dedetização é de até 48 horas úteis.".to_owned(),
                versao: 1,
            }],
        )
        .await
        .expect("indexa");

    let without_cutoff = index
        .search_hybrid(
            org,
            agente,
            "qual o preço do sapato de couro",
            &SearchConfig {
                top_k: 3,
                cutoff: 0.0,
                depth: 20,
            },
        )
        .await
        .expect("busca sem corte");
    assert!(!without_cutoff.is_empty(), "sem corte, o top-3 volta");

    let with_cutoff = index
        .search_hybrid(
            org,
            agente,
            "qual o preço do sapato de couro",
            &SearchConfig {
                top_k: 3,
                cutoff: 0.6,
                depth: 20,
            },
        )
        .await
        .expect("busca com corte");
    assert!(
        with_cutoff.is_empty(),
        "com corte 0,6 o trecho sem relação não pode voltar: {with_cutoff:?}"
    );
}

#[tokio::test]
async fn hybrid_keeps_orgs_isolated() {
    let folder = TempFolder::new("orgs");
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new());
    let (index, _) =
        Index::open(&folder.rag_db(), embedder, IndexConfig::default()).expect("abre o índice");
    let org_a = OrgId::generate();
    let org_b = OrgId::generate();
    let agente = AgentId::generate();
    let item = item_id("segredo");
    index
        .index_agent(
            org_a,
            agente,
            &[ItemRag {
                id: item,
                title: "Código interno".to_owned(),
                texto: "O código interno do cliente é azul-turquesa.".to_owned(),
                versao: 1,
            }],
        )
        .await
        .expect("indexa");
    let in_other_org = index
        .search_hybrid(
            org_b,
            agente,
            "código interno azul turquesa",
            &SearchConfig::default(),
        )
        .await
        .expect("busca na outra org");
    assert!(
        in_other_org.is_empty(),
        "trecho de outra org não pode voltar"
    );
}
