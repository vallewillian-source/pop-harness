//! Índice de trechos (HR-16): indexação incremental, reconstrução do `rag.db`, cache com teto de
//! memória, isolamento por org, troca de modelo e busca por palavras. Usa o embedder falso, então
//! roda sem o modelo instalado e sem rede.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_rag::{
    BatchFuture, Embedder, FakeEmbedder, Index, IndexConfig, ItemRag, ModelIdentity, RagError,
};
use poph_types::{AgentId, KnowledgeItemId, OrgId};

/// Embedder falso que conta quantos textos recebeu e deixa trocar a identidade do modelo.
struct Contador {
    fake: FakeEmbedder,
    identidade: ModelIdentity,
    textos: AtomicUsize,
}

impl Contador {
    fn novo() -> Arc<Self> {
        let fake = FakeEmbedder::new();
        let identidade = fake.identity().clone();
        Arc::new(Self {
            fake,
            identidade,
            textos: AtomicUsize::new(0),
        })
    }

    fn com_outro_modelo() -> Arc<Self> {
        let mut contador = Arc::try_unwrap(Self::novo()).ok().expect("único dono");
        contador.identidade.model_sha256 = "outro-modelo".to_owned();
        Arc::new(contador)
    }

    fn textos(&self) -> usize {
        self.textos.load(Ordering::SeqCst)
    }
}

impl Embedder for Contador {
    fn identity(&self) -> &ModelIdentity {
        &self.identidade
    }

    fn embed_batch<'a>(&'a self, textos: &'a [String]) -> BatchFuture<'a> {
        self.textos.fetch_add(textos.len(), Ordering::SeqCst);
        self.fake.embed_batch(textos)
    }
}

/// Pasta temporária única, apagada ao sair.
struct Pasta(PathBuf);

impl Pasta {
    fn nova(nome: &str) -> Self {
        let agora = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let caminho = std::env::temp_dir().join(format!(
            "poph-rag-indice-{nome}-{}-{agora}",
            std::process::id()
        ));
        fs::create_dir_all(&caminho).expect("cria a pasta temporária");
        Self(caminho)
    }

    fn rag_db(&self) -> PathBuf {
        self.0.join("rag.db")
    }
}

impl Drop for Pasta {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn apagar_rag_db(caminho: &Path) {
    for sufixo in ["", "-wal", "-shm"] {
        let arquivo = PathBuf::from(format!("{}{sufixo}", caminho.display()));
        let _ = fs::remove_file(arquivo);
    }
}

fn abrir(caminho: &Path, embedder: Arc<dyn Embedder>, orcamento: usize) -> Index {
    let (indice, _) = Index::open(
        caminho,
        embedder,
        IndexConfig {
            memory_budget_bytes: orcamento,
        },
    )
    .expect("abre o rag.db");
    indice
}

fn item(titulo: &str, texto: &str) -> ItemRag {
    ItemRag {
        id: KnowledgeItemId::generate(),
        title: titulo.to_owned(),
        texto: texto.to_owned(),
        versao: 1,
    }
}

/// Texto longo, com várias frases, que vira mais de um trecho.
fn texto_longo(tema: &str) -> String {
    (1..=25)
        .map(|n| format!("Parte {n} sobre {tema}, com detalhes e uma condição específica."))
        .collect::<Vec<_>>()
        .join(" ")
}

#[tokio::test]
async fn editar_um_item_reindexa_so_ele() {
    let pasta = Pasta::nova("editar");
    let contador = Contador::novo();
    let indice = abrir(&pasta.rag_db(), contador.clone(), 64 * 1024 * 1024);
    let (org, agente) = (OrgId::generate(), AgentId::generate());

    let mut itens = vec![
        item("Entrega", "Entregamos em três dias úteis."),
        item("Garantia", "A garantia é de doze meses."),
        item("Pagamento", "Aceitamos pix e cartão."),
    ];
    let primeira = indice
        .index_agent(org, agente, &itens)
        .await
        .expect("indexa");
    assert_eq!(primeira.new_items, 3);
    assert_eq!(primeira.generated_embeddings, 3);
    assert_eq!(contador.textos(), 3);

    // Edita só o segundo item.
    itens[1].texto = "A garantia agora é de vinte e quatro meses.".to_owned();
    itens[1].versao = 2;
    let antes = contador.textos();
    let editada = indice
        .index_agent(org, agente, &itens)
        .await
        .expect("reindexa");
    assert_eq!(editada.changed_items, 1);
    assert_eq!(editada.unchanged_items, 2);
    assert_eq!(editada.new_items, 0);
    assert_eq!(
        editada.generated_embeddings, 1,
        "só o item editado vai ao modelo"
    );
    assert_eq!(contador.textos() - antes, 1);

    // Sem mudança nenhuma: nada vai ao modelo e nada é regravado.
    let antes = contador.textos();
    let sem_mudanca = indice
        .index_agent(org, agente, &itens)
        .await
        .expect("reindexa");
    assert_eq!(sem_mudanca.unchanged_items, 3);
    assert_eq!(sem_mudanca.generated_embeddings, 0);
    assert_eq!(sem_mudanca.stored_chunks, 0);
    assert_eq!(contador.textos(), antes);

    // O texto novo é o que a busca acha.
    let achados = indice
        .search_words(org, agente, "vinte e quatro", 3)
        .expect("busca");
    assert_eq!(achados.first().map(|a| a.chunk.item), Some(itens[1].id));
}

#[tokio::test]
async fn item_que_sai_da_lista_some_do_indice() {
    let pasta = Pasta::nova("removido");
    let indice = abrir(&pasta.rag_db(), Contador::novo(), 64 * 1024 * 1024);
    let (org, agente) = (OrgId::generate(), AgentId::generate());

    let mut itens = vec![
        item("Horário", "Atendemos de segunda a sexta."),
        item("Frete", "Frete grátis acima de cem reais."),
    ];
    indice
        .index_agent(org, agente, &itens)
        .await
        .expect("indexa");
    let removido = itens.remove(1);

    let relatorio = indice
        .index_agent(org, agente, &itens)
        .await
        .expect("reindexa");
    assert_eq!(relatorio.removed_items, 1);
    let trechos = indice.list_chunks(org, agente).expect("lista");
    assert!(trechos.iter().all(|t| t.item != removido.id));
    assert!(
        indice
            .search_words(org, agente, "frete", 3)
            .expect("busca")
            .is_empty(),
        "item removido não pode aparecer no texto"
    );
}

#[tokio::test]
async fn apagar_o_rag_db_e_reindexar_reproduz_trechos_e_resultados() {
    let pasta = Pasta::nova("reconstrucao");
    let (org, agente) = (OrgId::generate(), AgentId::generate());
    let itens = vec![
        item("Catálogo", &texto_longo("cadeiras de madeira")),
        item("Prazo", "Prazo de entrega de cinco dias para capital."),
        item("Política", &texto_longo("troca e devolução")),
    ];

    let indice = abrir(&pasta.rag_db(), Contador::novo(), 64 * 1024 * 1024);
    indice
        .index_agent(org, agente, &itens)
        .await
        .expect("indexa");
    let trechos_antes = indice.list_chunks(org, agente).expect("lista");
    let vetor_antes = indice
        .search_vector(org, agente, "devolução de produto", 3)
        .await
        .expect("busca vetor");
    let palavras_antes = indice
        .search_words(org, agente, "prazo entrega", 5)
        .expect("busca palavras");
    assert!(
        trechos_antes.len() > 3,
        "os itens longos têm vários trechos"
    );
    drop(indice);

    apagar_rag_db(&pasta.rag_db());
    let refeito = abrir(&pasta.rag_db(), Contador::novo(), 64 * 1024 * 1024);
    let relatorio = refeito
        .index_agent(org, agente, &itens)
        .await
        .expect("reindexa");
    assert_eq!(relatorio.new_items, 3);

    assert_eq!(
        refeito.list_chunks(org, agente).expect("lista"),
        trechos_antes
    );
    assert_eq!(
        refeito
            .search_vector(org, agente, "devolução de produto", 3)
            .await
            .expect("busca vetor"),
        vetor_antes
    );
    assert_eq!(
        refeito
            .search_words(org, agente, "prazo entrega", 5)
            .expect("busca palavras"),
        palavras_antes
    );
}

#[tokio::test]
async fn agente_ocioso_sai_com_o_teto_e_volta_sem_resultado_errado() {
    let pasta = Pasta::nova("lru");
    let contador = Contador::novo();
    // Cada agente aqui tem 2 trechos de ~404 bytes; o teto cabe dois agentes, não três.
    let indice = abrir(&pasta.rag_db(), contador, 1700);
    let org = OrgId::generate();
    let agentes = [
        AgentId::generate(),
        AgentId::generate(),
        AgentId::generate(),
    ];
    for (n, agente) in agentes.iter().enumerate() {
        let itens = vec![
            item("A", &format!("Assunto {n} primeiro item.")),
            item("B", &format!("Assunto {n} segundo item.")),
        ];
        indice
            .index_agent(org, *agente, &itens)
            .await
            .expect("indexa");
    }

    let mut primeiros = Vec::new();
    for agente in &agentes {
        primeiros.push(
            indice
                .search_vector(org, *agente, "primeiro item", 2)
                .await
                .expect("busca"),
        );
        assert!(indice.bytes_in_memory() <= indice.memory_budget_bytes());
    }
    assert_eq!(
        indice.agents_in_memory(),
        2,
        "o terceiro expulsou o mais antigo"
    );

    // O agente expulso volta do rag.db e responde igual.
    let de_novo = indice
        .search_vector(org, agentes[0], "primeiro item", 2)
        .await
        .expect("busca");
    assert_eq!(de_novo, primeiros[0]);
    assert_eq!(indice.agents_in_memory(), 2);
    assert!(indice.bytes_in_memory() <= indice.memory_budget_bytes());
}

#[tokio::test]
async fn org_a_nao_alcanca_trecho_de_org_b_mesmo_com_id_forjado() {
    let pasta = Pasta::nova("isolamento");
    let indice = abrir(&pasta.rag_db(), Contador::novo(), 64 * 1024 * 1024);
    let (org_a, org_b) = (OrgId::generate(), OrgId::generate());
    let agente_a = AgentId::generate();
    let agente_b = AgentId::generate();

    indice
        .index_agent(
            org_a,
            agente_a,
            &[item("Segredo", "Código interno da org A.")],
        )
        .await
        .expect("indexa A");
    indice
        .index_agent(org_b, agente_b, &[item("Público", "Horário da org B.")])
        .await
        .expect("indexa B");

    // Org B com o agente da org A: nada.
    assert!(
        indice
            .search_vector(org_b, agente_a, "código interno", 3)
            .await
            .expect("busca")
            .is_empty()
    );
    assert!(
        indice
            .search_words(org_b, agente_a, "código interno", 3)
            .expect("busca")
            .is_empty()
    );
    assert!(
        indice
            .list_chunks(org_b, agente_a)
            .expect("lista")
            .is_empty()
    );

    // Org A com o agente dela: acha o próprio conteúdo.
    let achados = indice
        .search_vector(org_a, agente_a, "código interno", 3)
        .await
        .expect("busca");
    assert_eq!(achados.len(), 1);
    assert!(achados[0].chunk.texto.contains("org A"));
}

#[tokio::test]
async fn mesmo_id_de_item_em_orgs_diferentes_nao_se_mistura() {
    let pasta = Pasta::nova("mesmo-id");
    let indice = abrir(&pasta.rag_db(), Contador::novo(), 64 * 1024 * 1024);
    let (org_a, org_b) = (OrgId::generate(), OrgId::generate());
    let agente_a = AgentId::generate();
    let agente_b = AgentId::generate();
    let compartilhado = KnowledgeItemId::generate();
    let de = |texto: &str| ItemRag {
        id: compartilhado,
        title: "Mesmo".to_owned(),
        texto: texto.to_owned(),
        versao: 1,
    };

    indice
        .index_agent(org_a, agente_a, &[de("texto da org A")])
        .await
        .expect("A");
    let relatorio_b = indice
        .index_agent(org_b, agente_b, &[de("texto da org B")])
        .await
        .expect("B");
    assert_eq!(
        relatorio_b.new_items, 1,
        "a org B não reaproveita o item da A"
    );

    // Editar na org A não toca a org B.
    indice
        .index_agent(org_a, agente_a, &[de("texto da org A revisado")])
        .await
        .expect("A revisado");
    let da_b = indice.list_chunks(org_b, agente_b).expect("lista B");
    assert_eq!(da_b.len(), 1);
    assert_eq!(da_b[0].texto, "texto da org B");
}

#[tokio::test]
async fn troca_de_modelo_limpa_o_rag_db_e_so_limpa_uma_vez() {
    let pasta = Pasta::nova("modelo");
    let (org, agente) = (OrgId::generate(), AgentId::generate());
    {
        let indice = abrir(&pasta.rag_db(), Contador::novo(), 64 * 1024 * 1024);
        indice
            .index_agent(org, agente, &[item("X", "Algum texto.")])
            .await
            .expect("indexa");
    }

    let (trocado, relatorio) = Index::open(
        &pasta.rag_db(),
        Contador::com_outro_modelo(),
        IndexConfig::default(),
    )
    .expect("abre com outro modelo");
    assert!(relatorio.cleaned_by_divergence);
    assert!(trocado.list_chunks(org, agente).expect("lista").is_empty());

    let (_, de_novo) = Index::open(
        &pasta.rag_db(),
        Contador::com_outro_modelo(),
        IndexConfig::default(),
    )
    .expect("abre de novo");
    assert!(
        !de_novo.cleaned_by_divergence,
        "a identidade já foi gravada"
    );
}

#[tokio::test]
async fn busca_de_palavras_ignora_acento_e_maiusculas() {
    let pasta = Pasta::nova("palavras");
    let indice = abrir(&pasta.rag_db(), Contador::novo(), 64 * 1024 * 1024);
    let (org, agente) = (OrgId::generate(), AgentId::generate());
    indice
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

    let achados = indice.search_words(org, agente, "preco", 3).expect("busca");
    assert_eq!(achados.len(), 1);
    assert!(achados[0].score > 0.0, "relevância positiva para quem acha");
    // Aspas e símbolos na consulta não quebram a busca.
    assert!(
        indice
            .search_words(org, agente, "\"cadeira\" OR (\"", 3)
            .expect("busca com símbolos")
            .len()
            == 1
    );
    assert!(
        indice
            .search_words(org, agente, "   !!!  ", 3)
            .expect("só símbolos")
            .is_empty()
    );
}

#[tokio::test]
async fn entradas_invalidas_sao_recusadas_com_erro_claro() {
    let pasta = Pasta::nova("invalidas");
    let indice = abrir(&pasta.rag_db(), Contador::novo(), 64 * 1024 * 1024);
    let (org, agente) = (OrgId::generate(), AgentId::generate());

    let repetido = KnowledgeItemId::generate();
    let dois = vec![
        ItemRag {
            id: repetido,
            title: "a".to_owned(),
            texto: "x".to_owned(),
            versao: 1,
        },
        ItemRag {
            id: repetido,
            title: "b".to_owned(),
            texto: "y".to_owned(),
            versao: 1,
        },
    ];
    match indice.index_agent(org, agente, &dois).await {
        Err(RagError::Item(mensagem)) => assert!(mensagem.contains("repetido")),
        outro => panic!("esperava erro de item repetido, veio {outro:?}"),
    }

    assert!(matches!(
        indice.search_vector(org, agente, "algo", 0).await,
        Err(RagError::Configuration(_))
    ));
    assert!(matches!(
        indice.search_words(org, agente, "algo", 0),
        Err(RagError::Configuration(_))
    ));
    assert!(
        indice
            .search_vector(org, agente, "   ", 3)
            .await
            .expect("consulta vazia")
            .is_empty()
    );
}
