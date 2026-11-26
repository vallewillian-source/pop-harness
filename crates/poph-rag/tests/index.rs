use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_rag::{
    BatchFuture, Embedder, FakeEmbedder, Index, IndexConfig, ItemRag, ModelIdentity, RagError,
};
use poph_types::{AgentId, KnowledgeItemId, OrgId};

struct CountingEmbedder {
    fake: FakeEmbedder,
    model_identity: ModelIdentity,
    texts: AtomicUsize,
}

impl CountingEmbedder {
    fn new() -> Arc<Self> {
        let fake = FakeEmbedder::new();
        let model_identity = fake.identity().clone();
        Arc::new(Self {
            fake,
            model_identity,
            texts: AtomicUsize::new(0),
        })
    }

    fn with_other_model() -> Arc<Self> {
        let mut counted = Arc::try_unwrap(Self::new()).ok().expect("único dono");
        counted.model_identity.model_sha256 = "outro-modelo".to_owned();
        Arc::new(counted)
    }

    fn texts(&self) -> usize {
        self.texts.load(Ordering::SeqCst)
    }
}

impl Embedder for CountingEmbedder {
    fn identity(&self) -> &ModelIdentity {
        &self.model_identity
    }

    fn embed_batch<'a>(&'a self, texts: &'a [String]) -> BatchFuture<'a> {
        self.texts.fetch_add(texts.len(), Ordering::SeqCst);
        self.fake.embed_batch(texts)
    }
}

struct TempFolder(PathBuf);

impl TempFolder {
    fn new(nome: &str) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "poph-rag-indice-{nome}-{}-{now}",
            std::process::id()
        ));
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

fn delete_rag_db(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let file = PathBuf::from(format!("{}{suffix}", path.display()));
        let _ = fs::remove_file(file);
    }
}

fn open_index(path: &Path, embedder: Arc<dyn Embedder>, budget: usize) -> Index {
    let (index, _) = Index::open(
        path,
        embedder,
        IndexConfig {
            memory_budget_bytes: budget,
        },
    )
    .expect("abre o rag.db");
    index
}

fn item(title: &str, texto: &str) -> ItemRag {
    ItemRag {
        id: KnowledgeItemId::generate(),
        title: title.to_owned(),
        texto: texto.to_owned(),
        versao: 1,
    }
}

fn long_text(topic: &str) -> String {
    (1..=25)
        .map(|n| format!("Parte {n} sobre {topic}, com detalhes e uma condição específica."))
        .collect::<Vec<_>>()
        .join(" ")
}

#[tokio::test]
async fn editing_one_item_reindexes_only_that_item() {
    let folder = TempFolder::new("editar");
    let counted = CountingEmbedder::new();
    let index = open_index(&folder.rag_db(), counted.clone(), 64 * 1024 * 1024);
    let (org, agente) = (OrgId::generate(), AgentId::generate());

    let mut items = vec![
        item("Entrega", "Entregamos em três dias úteis."),
        item("Garantia", "A garantia é de doze meses."),
        item("Pagamento", "Aceitamos pix e cartão."),
    ];
    let first = index
        .index_agent(org, agente, &items)
        .await
        .expect("indexa");
    assert_eq!(first.new_items, 3);
    assert_eq!(first.generated_embeddings, 3);
    assert_eq!(counted.texts(), 3);

    items[1].texto = "A garantia agora é de vinte e quatro meses.".to_owned();
    items[1].versao = 2;
    let before = counted.texts();
    let edited = index
        .index_agent(org, agente, &items)
        .await
        .expect("reindexa");
    assert_eq!(edited.changed_items, 1);
    assert_eq!(edited.unchanged_items, 2);
    assert_eq!(edited.new_items, 0);
    assert_eq!(
        edited.generated_embeddings, 1,
        "só o item editado vai ao modelo"
    );
    assert_eq!(counted.texts() - before, 1);

    let before = counted.texts();
    let unchanged = index
        .index_agent(org, agente, &items)
        .await
        .expect("reindexa");
    assert_eq!(unchanged.unchanged_items, 3);
    assert_eq!(unchanged.generated_embeddings, 0);
    assert_eq!(unchanged.stored_chunks, 0);
    assert_eq!(counted.texts(), before);

    let hits = index
        .search_words(org, agente, "vinte e quatro", 3)
        .expect("busca");
    assert_eq!(hits.first().map(|a| a.chunk.item), Some(items[1].id));
}

#[tokio::test]
async fn item_removed_from_list_disappears_from_index() {
    let folder = TempFolder::new("removido");
    let index = open_index(&folder.rag_db(), CountingEmbedder::new(), 64 * 1024 * 1024);
    let (org, agente) = (OrgId::generate(), AgentId::generate());

    let mut items = vec![
        item("Horário", "Atendemos de segunda a sexta."),
        item("Frete", "Frete grátis acima de cem reais."),
    ];
    index
        .index_agent(org, agente, &items)
        .await
        .expect("indexa");
    let removed = items.remove(1);

    let report = index
        .index_agent(org, agente, &items)
        .await
        .expect("reindexa");
    assert_eq!(report.removed_items, 1);
    let chunks = index.list_chunks(org, agente).expect("lista");
    assert!(chunks.iter().all(|t| t.item != removed.id));
    assert!(
        index
            .search_words(org, agente, "frete", 3)
            .expect("busca")
            .is_empty(),
        "item removido não pode aparecer no texto"
    );
}

#[tokio::test]
async fn rebuilding_rag_db_reproduces_chunks_and_search_results() {
    let folder = TempFolder::new("reconstrucao");
    let (org, agente) = (OrgId::generate(), AgentId::generate());
    let items = vec![
        item("Catálogo", &long_text("cadeiras de madeira")),
        item("Prazo", "Prazo de entrega de cinco dias para capital."),
        item("Política", &long_text("troca e devolução")),
    ];

    let index = open_index(&folder.rag_db(), CountingEmbedder::new(), 64 * 1024 * 1024);
    index
        .index_agent(org, agente, &items)
        .await
        .expect("indexa");
    let chunks_before = index.list_chunks(org, agente).expect("lista");
    let vector_before = index
        .search_vector(org, agente, "devolução de produto", 3)
        .await
        .expect("busca vetor");
    let words_before = index
        .search_words(org, agente, "prazo entrega", 5)
        .expect("busca palavras");
    assert!(
        chunks_before.len() > 3,
        "os itens longos têm vários trechos"
    );
    drop(index);

    delete_rag_db(&folder.rag_db());
    let rebuilt = open_index(&folder.rag_db(), CountingEmbedder::new(), 64 * 1024 * 1024);
    let report = rebuilt
        .index_agent(org, agente, &items)
        .await
        .expect("reindexa");
    assert_eq!(report.new_items, 3);

    assert_eq!(
        rebuilt.list_chunks(org, agente).expect("lista"),
        chunks_before
    );
    assert_eq!(
        rebuilt
            .search_vector(org, agente, "devolução de produto", 3)
            .await
            .expect("busca vetor"),
        vector_before
    );
    assert_eq!(
        rebuilt
            .search_words(org, agente, "prazo entrega", 5)
            .expect("busca palavras"),
        words_before
    );
}

#[tokio::test]
async fn idle_agent_is_evicted_at_budget_and_returns_correct_results() {
    let folder = TempFolder::new("lru");
    let counted = CountingEmbedder::new();
    let index = open_index(&folder.rag_db(), counted, 1700);
    let org = OrgId::generate();
    let agents = [
        AgentId::generate(),
        AgentId::generate(),
        AgentId::generate(),
    ];
    for (n, agente) in agents.iter().enumerate() {
        let items = vec![
            item("A", &format!("Assunto {n} primeiro item.")),
            item("B", &format!("Assunto {n} segundo item.")),
        ];
        index
            .index_agent(org, *agente, &items)
            .await
            .expect("indexa");
    }

    let mut first_hits = Vec::new();
    for agente in &agents {
        first_hits.push(
            index
                .search_vector(org, *agente, "primeiro item", 2)
                .await
                .expect("busca"),
        );
        assert!(index.bytes_in_memory() <= index.memory_budget_bytes());
    }
    assert_eq!(
        index.agents_in_memory(),
        2,
        "o terceiro expulsou o mais antigo"
    );

    let again = index
        .search_vector(org, agents[0], "primeiro item", 2)
        .await
        .expect("busca");
    assert_eq!(again, first_hits[0]);
    assert_eq!(index.agents_in_memory(), 2);
    assert!(index.bytes_in_memory() <= index.memory_budget_bytes());
}

#[tokio::test]
async fn org_a_cannot_reach_org_b_chunks_even_with_forged_id() {
    let folder = TempFolder::new("isolamento");
    let index = open_index(&folder.rag_db(), CountingEmbedder::new(), 64 * 1024 * 1024);
    let (org_a, org_b) = (OrgId::generate(), OrgId::generate());
    let agent_a = AgentId::generate();
    let agent_b = AgentId::generate();

    index
        .index_agent(
            org_a,
            agent_a,
            &[item("Segredo", "Código interno da org A.")],
        )
        .await
        .expect("indexa A");
    index
        .index_agent(org_b, agent_b, &[item("Público", "Horário da org B.")])
        .await
        .expect("indexa B");

    assert!(
        index
            .search_vector(org_b, agent_a, "código interno", 3)
            .await
            .expect("busca")
            .is_empty()
    );
    assert!(
        index
            .search_words(org_b, agent_a, "código interno", 3)
            .expect("busca")
            .is_empty()
    );
    assert!(index.list_chunks(org_b, agent_a).expect("lista").is_empty());

    let hits = index
        .search_vector(org_a, agent_a, "código interno", 3)
        .await
        .expect("busca");
    assert_eq!(hits.len(), 1);
    assert!(hits[0].chunk.texto.contains("org A"));
}

#[tokio::test]
async fn same_item_id_in_distinct_orgs_stays_isolated() {
    let folder = TempFolder::new("mesmo-id");
    let index = open_index(&folder.rag_db(), CountingEmbedder::new(), 64 * 1024 * 1024);
    let (org_a, org_b) = (OrgId::generate(), OrgId::generate());
    let agent_a = AgentId::generate();
    let agent_b = AgentId::generate();
    let shared = KnowledgeItemId::generate();
    let de = |texto: &str| ItemRag {
        id: shared,
        title: "Mesmo".to_owned(),
        texto: texto.to_owned(),
        versao: 1,
    };

    index
        .index_agent(org_a, agent_a, &[de("texto da org A")])
        .await
        .expect("A");
    let report_b = index
        .index_agent(org_b, agent_b, &[de("texto da org B")])
        .await
        .expect("B");
    assert_eq!(report_b.new_items, 1, "a org B não reaproveita o item da A");

    index
        .index_agent(org_a, agent_a, &[de("texto da org A revisado")])
        .await
        .expect("A revisado");
    let from_b = index.list_chunks(org_b, agent_b).expect("lista B");
    assert_eq!(from_b.len(), 1);
    assert_eq!(from_b[0].texto, "texto da org B");
}

#[tokio::test]
async fn changing_model_clears_rag_db_only_once() {
    let folder = TempFolder::new("modelo");
    let (org, agente) = (OrgId::generate(), AgentId::generate());
    {
        let index = open_index(&folder.rag_db(), CountingEmbedder::new(), 64 * 1024 * 1024);
        index
            .index_agent(org, agente, &[item("X", "Algum texto.")])
            .await
            .expect("indexa");
    }

    let (switched, report) = Index::open(
        &folder.rag_db(),
        CountingEmbedder::with_other_model(),
        IndexConfig::default(),
    )
    .expect("abre com outro modelo");
    assert!(report.cleaned_by_divergence);
    assert!(switched.list_chunks(org, agente).expect("lista").is_empty());

    let (_, again) = Index::open(
        &folder.rag_db(),
        CountingEmbedder::with_other_model(),
        IndexConfig::default(),
    )
    .expect("abre de novo");
    assert!(!again.cleaned_by_divergence, "a identidade já foi gravada");
}

#[tokio::test]
async fn word_search_ignores_accents_and_case() {
    let folder = TempFolder::new("palavras");
    let index = open_index(&folder.rag_db(), CountingEmbedder::new(), 64 * 1024 * 1024);
    let (org, agente) = (OrgId::generate(), AgentId::generate());
    index
        .index_agent(
            org,
            agente,
            &[
                item("Preço", "O PREÇO da cadeira é fixo."),
                item("Outro", "Sem relação nenhuma."),
            ],
        )
        .await
        .expect("indexa");

    let hits = index.search_words(org, agente, "preco", 3).expect("busca");
    assert_eq!(hits.len(), 1);
    assert!(hits[0].score > 0.0, "relevância positiva para quem acha");
    assert!(
        index
            .search_words(org, agente, "\"cadeira\" OR (\"", 3)
            .expect("busca com símbolos")
            .len()
            == 1
    );
    assert!(
        index
            .search_words(org, agente, "   !!!  ", 3)
            .expect("só símbolos")
            .is_empty()
    );
}

#[tokio::test]
async fn invalid_inputs_return_clear_errors() {
    let folder = TempFolder::new("invalidas");
    let index = open_index(&folder.rag_db(), CountingEmbedder::new(), 64 * 1024 * 1024);
    let (org, agente) = (OrgId::generate(), AgentId::generate());

    let duplicate = KnowledgeItemId::generate();
    let two = vec![
        ItemRag {
            id: duplicate,
            title: "a".to_owned(),
            texto: "x".to_owned(),
            versao: 1,
        },
        ItemRag {
            id: duplicate,
            title: "b".to_owned(),
            texto: "y".to_owned(),
            versao: 1,
        },
    ];
    match index.index_agent(org, agente, &two).await {
        Err(RagError::Item(mensagem)) => assert!(mensagem.contains("repetido")),
        other => panic!("esperava erro de item repetido, veio {other:?}"),
    }

    assert!(matches!(
        index.search_vector(org, agente, "algo", 0).await,
        Err(RagError::Configuration(_))
    ));
    assert!(matches!(
        index.search_words(org, agente, "algo", 0),
        Err(RagError::Configuration(_))
    ));
    assert!(
        index
            .search_vector(org, agente, "   ", 3)
            .await
            .expect("consulta vazia")
            .is_empty()
    );
}
