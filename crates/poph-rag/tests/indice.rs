//! Índice de trechos (HR-16): indexação incremental, reconstrução do `rag.db`, cache com teto de
//! memória, isolamento por org, troca de modelo e busca por palavras. Usa o embedder falso, então
//! roda sem o modelo instalado e sem rede.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_rag::{
    ConfigIndice, Embedder, EmbedderFake, ErroRag, FuturoLote, IdentidadeModelo, Indice, ItemRag,
};
use poph_types::{AgentId, KnowledgeItemId, OrgId};

/// Embedder falso que conta quantos textos recebeu e deixa trocar a identidade do modelo.
struct Contador {
    fake: EmbedderFake,
    identidade: IdentidadeModelo,
    textos: AtomicUsize,
}

impl Contador {
    fn novo() -> Arc<Self> {
        let fake = EmbedderFake::new();
        let identidade = fake.identidade().clone();
        Arc::new(Self {
            fake,
            identidade,
            textos: AtomicUsize::new(0),
        })
    }

    fn com_outro_modelo() -> Arc<Self> {
        let mut contador = Arc::try_unwrap(Self::novo()).ok().expect("único dono");
        contador.identidade.sha256_modelo = "outro-modelo".to_owned();
        Arc::new(contador)
    }

    fn textos(&self) -> usize {
        self.textos.load(Ordering::SeqCst)
    }
}

impl Embedder for Contador {
    fn identidade(&self) -> &IdentidadeModelo {
        &self.identidade
    }

    fn embedir_lote<'a>(&'a self, textos: &'a [String]) -> FuturoLote<'a> {
        self.textos.fetch_add(textos.len(), Ordering::SeqCst);
        self.fake.embedir_lote(textos)
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

fn abrir(caminho: &Path, embedder: Arc<dyn Embedder>, orcamento: usize) -> Indice {
    let (indice, _) = Indice::abrir(
        caminho,
        embedder,
        ConfigIndice {
            orcamento_memoria_bytes: orcamento,
        },
    )
    .expect("abre o rag.db");
    indice
}

fn item(titulo: &str, texto: &str) -> ItemRag {
    ItemRag {
        id: KnowledgeItemId::generate(),
        titulo: titulo.to_owned(),
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
        .indexar_agente(org, agente, &itens)
        .await
        .expect("indexa");
    assert_eq!(primeira.itens_novos, 3);
    assert_eq!(primeira.embeddings_gerados, 3);
    assert_eq!(contador.textos(), 3);

    // Edita só o segundo item.
    itens[1].texto = "A garantia agora é de vinte e quatro meses.".to_owned();
    itens[1].versao = 2;
    let antes = contador.textos();
    let editada = indice
        .indexar_agente(org, agente, &itens)
        .await
        .expect("reindexa");
    assert_eq!(editada.itens_alterados, 1);
    assert_eq!(editada.itens_inalterados, 2);
    assert_eq!(editada.itens_novos, 0);
    assert_eq!(
        editada.embeddings_gerados, 1,
        "só o item editado vai ao modelo"
    );
    assert_eq!(contador.textos() - antes, 1);

    // Sem mudança nenhuma: nada vai ao modelo e nada é regravado.
    let antes = contador.textos();
    let sem_mudanca = indice
        .indexar_agente(org, agente, &itens)
        .await
        .expect("reindexa");
    assert_eq!(sem_mudanca.itens_inalterados, 3);
    assert_eq!(sem_mudanca.embeddings_gerados, 0);
    assert_eq!(sem_mudanca.trechos_gravados, 0);
    assert_eq!(contador.textos(), antes);

    // O texto novo é o que a busca acha.
    let achados = indice
        .buscar_palavras(org, agente, "vinte e quatro", 3)
        .expect("busca");
    assert_eq!(achados.first().map(|a| a.trecho.item), Some(itens[1].id));
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
        .indexar_agente(org, agente, &itens)
        .await
        .expect("indexa");
    let removido = itens.remove(1);

    let relatorio = indice
        .indexar_agente(org, agente, &itens)
        .await
        .expect("reindexa");
    assert_eq!(relatorio.itens_removidos, 1);
    let trechos = indice.listar_trechos(org, agente).expect("lista");
    assert!(trechos.iter().all(|t| t.item != removido.id));
    assert!(
        indice
            .buscar_palavras(org, agente, "frete", 3)
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
        .indexar_agente(org, agente, &itens)
        .await
        .expect("indexa");
    let trechos_antes = indice.listar_trechos(org, agente).expect("lista");
    let vetor_antes = indice
        .buscar_vetor(org, agente, "devolução de produto", 3)
        .await
        .expect("busca vetor");
    let palavras_antes = indice
        .buscar_palavras(org, agente, "prazo entrega", 5)
        .expect("busca palavras");
    assert!(
        trechos_antes.len() > 3,
        "os itens longos têm vários trechos"
    );
    drop(indice);

    apagar_rag_db(&pasta.rag_db());
    let refeito = abrir(&pasta.rag_db(), Contador::novo(), 64 * 1024 * 1024);
    let relatorio = refeito
        .indexar_agente(org, agente, &itens)
        .await
        .expect("reindexa");
    assert_eq!(relatorio.itens_novos, 3);

    assert_eq!(
        refeito.listar_trechos(org, agente).expect("lista"),
        trechos_antes
    );
    assert_eq!(
        refeito
            .buscar_vetor(org, agente, "devolução de produto", 3)
            .await
            .expect("busca vetor"),
        vetor_antes
    );
    assert_eq!(
        refeito
            .buscar_palavras(org, agente, "prazo entrega", 5)
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
            .indexar_agente(org, *agente, &itens)
            .await
            .expect("indexa");
    }

    let mut primeiros = Vec::new();
    for agente in &agentes {
        primeiros.push(
            indice
                .buscar_vetor(org, *agente, "primeiro item", 2)
                .await
                .expect("busca"),
        );
        assert!(indice.bytes_em_memoria() <= indice.orcamento_memoria_bytes());
    }
    assert_eq!(
        indice.agentes_em_memoria(),
        2,
        "o terceiro expulsou o mais antigo"
    );

    // O agente expulso volta do rag.db e responde igual.
    let de_novo = indice
        .buscar_vetor(org, agentes[0], "primeiro item", 2)
        .await
        .expect("busca");
    assert_eq!(de_novo, primeiros[0]);
    assert_eq!(indice.agentes_em_memoria(), 2);
    assert!(indice.bytes_em_memoria() <= indice.orcamento_memoria_bytes());
}

#[tokio::test]
async fn org_a_nao_alcanca_trecho_de_org_b_mesmo_com_id_forjado() {
    let pasta = Pasta::nova("isolamento");
    let indice = abrir(&pasta.rag_db(), Contador::novo(), 64 * 1024 * 1024);
    let (org_a, org_b) = (OrgId::generate(), OrgId::generate());
    let agente_a = AgentId::generate();
    let agente_b = AgentId::generate();

    indice
        .indexar_agente(
            org_a,
            agente_a,
            &[item("Segredo", "Código interno da org A.")],
        )
        .await
        .expect("indexa A");
    indice
        .indexar_agente(org_b, agente_b, &[item("Público", "Horário da org B.")])
        .await
        .expect("indexa B");

    // Org B com o agente da org A: nada.
    assert!(
        indice
            .buscar_vetor(org_b, agente_a, "código interno", 3)
            .await
            .expect("busca")
            .is_empty()
    );
    assert!(
        indice
            .buscar_palavras(org_b, agente_a, "código interno", 3)
            .expect("busca")
            .is_empty()
    );
    assert!(
        indice
            .listar_trechos(org_b, agente_a)
            .expect("lista")
            .is_empty()
    );

    // Org A com o agente dela: acha o próprio conteúdo.
    let achados = indice
        .buscar_vetor(org_a, agente_a, "código interno", 3)
        .await
        .expect("busca");
    assert_eq!(achados.len(), 1);
    assert!(achados[0].trecho.texto.contains("org A"));
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
        titulo: "Mesmo".to_owned(),
        texto: texto.to_owned(),
        versao: 1,
    };

    indice
        .indexar_agente(org_a, agente_a, &[de("texto da org A")])
        .await
        .expect("A");
    let relatorio_b = indice
        .indexar_agente(org_b, agente_b, &[de("texto da org B")])
        .await
        .expect("B");
    assert_eq!(
        relatorio_b.itens_novos, 1,
        "a org B não reaproveita o item da A"
    );

    // Editar na org A não toca a org B.
    indice
        .indexar_agente(org_a, agente_a, &[de("texto da org A revisado")])
        .await
        .expect("A revisado");
    let da_b = indice.listar_trechos(org_b, agente_b).expect("lista B");
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
            .indexar_agente(org, agente, &[item("X", "Algum texto.")])
            .await
            .expect("indexa");
    }

    let (trocado, relatorio) = Indice::abrir(
        &pasta.rag_db(),
        Contador::com_outro_modelo(),
        ConfigIndice::default(),
    )
    .expect("abre com outro modelo");
    assert!(relatorio.limpo_por_divergencia);
    assert!(
        trocado
            .listar_trechos(org, agente)
            .expect("lista")
            .is_empty()
    );

    let (_, de_novo) = Indice::abrir(
        &pasta.rag_db(),
        Contador::com_outro_modelo(),
        ConfigIndice::default(),
    )
    .expect("abre de novo");
    assert!(
        !de_novo.limpo_por_divergencia,
        "a identidade já foi gravada"
    );
}

#[tokio::test]
async fn busca_de_palavras_ignora_acento_e_maiusculas() {
    let pasta = Pasta::nova("palavras");
    let indice = abrir(&pasta.rag_db(), Contador::novo(), 64 * 1024 * 1024);
    let (org, agente) = (OrgId::generate(), AgentId::generate());
    indice
        .indexar_agente(
            org,
            agente,
            &[
                item("Preço", "O PREÇO da cadeira é fixo."),
                item("Outro", "Sem relação nenhuma."),
            ],
        )
        .await
        .expect("indexa");

    let achados = indice
        .buscar_palavras(org, agente, "preco", 3)
        .expect("busca");
    assert_eq!(achados.len(), 1);
    assert!(
        achados[0].pontuacao > 0.0,
        "relevância positiva para quem acha"
    );
    // Aspas e símbolos na consulta não quebram a busca.
    assert!(
        indice
            .buscar_palavras(org, agente, "\"cadeira\" OR (\"", 3)
            .expect("busca com símbolos")
            .len()
            == 1
    );
    assert!(
        indice
            .buscar_palavras(org, agente, "   !!!  ", 3)
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
            titulo: "a".to_owned(),
            texto: "x".to_owned(),
            versao: 1,
        },
        ItemRag {
            id: repetido,
            titulo: "b".to_owned(),
            texto: "y".to_owned(),
            versao: 1,
        },
    ];
    match indice.indexar_agente(org, agente, &dois).await {
        Err(ErroRag::Item(mensagem)) => assert!(mensagem.contains("repetido")),
        outro => panic!("esperava erro de item repetido, veio {outro:?}"),
    }

    assert!(matches!(
        indice.buscar_vetor(org, agente, "algo", 0).await,
        Err(ErroRag::Configuracao(_))
    ));
    assert!(matches!(
        indice.buscar_palavras(org, agente, "algo", 0),
        Err(ErroRag::Configuracao(_))
    ));
    assert!(
        indice
            .buscar_vetor(org, agente, "   ", 3)
            .await
            .expect("consulta vazia")
            .is_empty()
    );
}
