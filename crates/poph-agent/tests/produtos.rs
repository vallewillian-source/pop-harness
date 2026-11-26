//! Catálogo de produtos e ferramenta `buscar_produto` (HR-18).
//!
//! Usa o embedder falso (bag of words determinístico): roda sem o modelo instalado e sem rede.
//! O que se prova aqui:
//! 1. `buscar_produto` respeita a lista habilitada (D-04) e devolve id e preço;
//! 2. produto inativo ou arquivado sai da busca e da ferramenta (D-05 aplicado a produto);
//! 3. mudar preço aparece na resposta seguinte sem reindexar (o preço vem do registro);
//! 4. preço zero e preço com centavos saem exatos;
//! 5. busca e leitura não cruzam org;
//! 6. filtro por categoria vinculada (com e sem vínculo);
//! 7. o produto usado fica registrado no coletor do turno (`UsoFerramentas`).

use std::path::PathBuf;
use std::sync::Arc;

use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::metricas::MetricasMemoria;
use poph_agent::produtos::{NOME_FERRAMENTA, ParametrosProdutos};
use poph_agent::turno::{ContextoFerramenta, RegistroFerramentas, UsoFerramentas, oferecidas};
use poph_agent::{BuscadorProdutos, BuscarProduto, CatalogoProdutos};
use poph_rag::{ConfigIndice, Embedder, EmbedderFake, Indice};
use poph_store::{
    NewAgent, NewCategory, NewOrg, NewProduct, Product, ProductChange, Status, Store,
};
use poph_types::{AgentId, ConversationId, OrgId, ProductCategoryId, TurnId};

struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    categoria: ProductCategoryId,
    catalogo: Arc<CatalogoProdutos>,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-produtos-{rotulo}-{}-{}",
            std::process::id(),
            AgentId::generate()
        ));
        let _ = std::fs::remove_dir_all(&pasta);
        std::fs::create_dir_all(&pasta).expect("cria a pasta");
        let store = Arc::new(
            Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre o banco"),
        );
        let org = store
            .orgs()
            .criar(NewOrg {
                name: format!("Org {rotulo}"),
                slug: format!("org-{rotulo}-{}", AgentId::generate()),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .criar(
                org,
                NewAgent {
                    name: format!("Atendente {rotulo}"),
                    slug: format!("atendente-{rotulo}"),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        let categoria = store
            .categories()
            .criar(
                org,
                NewCategory {
                    name: "Pragas urbanas".to_owned(),
                    description: String::new(),
                },
            )
            .expect("cria a categoria")
            .id;
        let embedder: Arc<dyn Embedder> = Arc::new(EmbedderFake::new());
        let (indice, _) = Indice::abrir(store.rag_path(), embedder, ConfigIndice::default())
            .expect("abre o índice");
        let catalogo = CatalogoProdutos::novo(Arc::clone(&store), Arc::new(indice));
        Self {
            pasta,
            store,
            org,
            agente,
            categoria,
            catalogo,
        }
    }

    fn nova_categoria(&self, nome: &str) -> ProductCategoryId {
        self.store
            .categories()
            .criar(
                self.org,
                NewCategory {
                    name: nome.to_owned(),
                    description: String::new(),
                },
            )
            .expect("cria a categoria")
            .id
    }

    fn criar_produto(
        &self,
        categoria: ProductCategoryId,
        nome: &str,
        aliases: &[&str],
        preco_texto: &str,
        centavos: i64,
    ) -> Product {
        self.store
            .products()
            .criar(
                self.org,
                NewProduct {
                    category_id: Some(categoria),
                    name: nome.to_owned(),
                    aliases: aliases.iter().map(|alias| (*alias).to_owned()).collect(),
                    description: format!("Serviço de {nome} com garantia."),
                    price_text: preco_texto.to_owned(),
                    price_cents: centavos,
                    unit: "por visita".to_owned(),
                    conditions: "Pagamento em até 3x".to_owned(),
                    ..NewProduct::default()
                },
            )
            .expect("cria o produto")
    }

    async fn sincronizar(&self) {
        self.catalogo
            .sincronizar_agente(self.org, self.agente)
            .await
            .expect("sincroniza o índice");
    }

    async fn buscar(&self, termo: &str) -> Vec<poph_agent::ProdutoEncontrado> {
        let catalogo: Arc<dyn BuscadorProdutos> =
            Arc::clone(&self.catalogo) as Arc<dyn BuscadorProdutos>;
        catalogo
            .buscar(
                self.org,
                self.agente,
                termo.to_owned(),
                None,
                ParametrosProdutos {
                    top_k: 3,
                    corte: 0.0,
                },
            )
            .await
            .expect("busca produtos")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

fn contexto(cenario: &Cenario, uso: Arc<UsoFerramentas>) -> ContextoFerramenta {
    ContextoFerramenta {
        org: cenario.org,
        conversa: ConversationId::generate(),
        agente: cenario.agente,
        turno: TurnId::generate(),
        uso,
    }
}

#[tokio::test]
async fn ferramenta_respeita_d04_e_devolve_preco_do_registro() {
    let cenario = Cenario::novo("d04");
    let produto = cenario.criar_produto(
        cenario.categoria,
        "Dedetização de baratas",
        &["baratas", "barata"],
        "R$ 320,00",
        32_000,
    );
    cenario.sincronizar().await;

    let config = Arc::new(ConfigQuente::nova(
        Arc::clone(&cenario.store),
        OpcoesConfigQuente::default(),
        MetricasMemoria::proprio(),
    ));
    let catalogo: Arc<dyn BuscadorProdutos> =
        Arc::clone(&cenario.catalogo) as Arc<dyn BuscadorProdutos>;
    let registro = BuscarProduto::registro(Arc::clone(&config), catalogo);
    assert_eq!(registro.nomes(), vec![NOME_FERRAMENTA.to_owned()]);

    assert!(
        oferecidas(&registro, &[]).is_empty(),
        "D-04: sem habilitação, a ferramenta não é oferecida"
    );
    let habilitada = oferecidas(&registro, &[NOME_FERRAMENTA.to_owned()]);
    assert_eq!(habilitada.len(), 1);
    assert_eq!(habilitada[0].nome, NOME_FERRAMENTA);

    let ferramenta = registro.ferramenta(NOME_FERRAMENTA).expect("existe");
    let uso = UsoFerramentas::nova();
    let resposta = ferramenta
        .executar(
            contexto(&cenario, Arc::clone(&uso)),
            serde_json::json!({"termo": "baratas"}),
        )
        .await
        .expect("a ferramenta executa");
    assert!(resposta.contains(&produto.id.to_string()), "{resposta}");
    assert!(resposta.contains("R$ 320,00"), "{resposta}");
    assert!(resposta.contains("32000 centavos"), "{resposta}");
    assert!(resposta.contains("<produtos_encontrados>"), "{resposta}");

    let usados = uso.produtos();
    assert_eq!(usados.len(), 1);
    assert_eq!(usados[0].produto, produto.id);
    assert_eq!(usados[0].preco_centavos, 32_000);

    let sem_argumento = ferramenta
        .executar(contexto(&cenario, uso), serde_json::json!({}))
        .await;
    assert!(sem_argumento.is_err(), "termo vazio devolve erro ao modelo");
}

#[tokio::test]
async fn produto_inativo_ou_arquivado_sai_da_busca_e_da_ferramenta() {
    let cenario = Cenario::novo("inativo");
    let produto = cenario.criar_produto(
        cenario.categoria,
        "Dedetização de cupins",
        &["cupins"],
        "R$ 450,00",
        45_000,
    );
    cenario.sincronizar().await;
    assert_eq!(cenario.buscar("cupins").await.len(), 1);

    // Inativo: sai do índice na sincronização.
    let atual = cenario
        .store
        .products()
        .obter(cenario.org, produto.id)
        .expect("lê o produto");
    cenario
        .store
        .products()
        .atualizar(
            cenario.org,
            produto.id,
            atual.version,
            ProductChange {
                status: Some(Status::Inactive),
                ..ProductChange::default()
            },
        )
        .expect("inativa o produto");
    cenario.sincronizar().await;
    assert!(
        cenario.buscar("cupins").await.is_empty(),
        "produto inativo não volta da busca"
    );

    // Arquivado: mesma coisa.
    let atual = cenario
        .store
        .products()
        .obter(cenario.org, produto.id)
        .expect("lê o produto");
    cenario
        .store
        .products()
        .atualizar(
            cenario.org,
            produto.id,
            atual.version,
            ProductChange {
                status: Some(Status::Archived),
                ..ProductChange::default()
            },
        )
        .expect("arquiva o produto");
    cenario.sincronizar().await;
    assert!(cenario.buscar("cupins").await.is_empty());
}

#[tokio::test]
async fn mudar_preco_aparece_na_resposta_seguinte_sem_reindexar() {
    let cenario = Cenario::novo("preco");
    let produto = cenario.criar_produto(
        cenario.categoria,
        "Desratização",
        &["ratos", "ratazanas"],
        "R$ 250,00",
        25_000,
    );
    cenario.sincronizar().await;

    let antes = cenario
        .catalogo
        .indice()
        .produtos_indexados(cenario.org, cenario.agente)
        .expect("hashes");
    assert_eq!(cenario.buscar("ratos").await[0].preco_centavos, 25_000);

    let atual = cenario
        .store
        .products()
        .obter(cenario.org, produto.id)
        .expect("lê o produto");
    cenario
        .store
        .products()
        .atualizar(
            cenario.org,
            produto.id,
            atual.version,
            ProductChange {
                price_text: Some("R$ 279,90".to_owned()),
                price_cents: Some(27_990),
                ..ProductChange::default()
            },
        )
        .expect("muda o preço");

    // Nenhuma reindexação: o índice nem foi tocado, e a resposta já traz o preço novo.
    let achados = cenario.buscar("ratos").await;
    assert_eq!(achados.len(), 1);
    assert_eq!(achados[0].preco_centavos, 27_990);
    assert_eq!(achados[0].preco_texto, "R$ 279,90");
    assert_eq!(
        antes,
        cenario
            .catalogo
            .indice()
            .produtos_indexados(cenario.org, cenario.agente)
            .expect("hashes"),
        "mudar preço não mexe no índice"
    );
}

#[tokio::test]
async fn preco_zero_e_centavos_saem_exatos() {
    let cenario = Cenario::novo("precos");
    cenario.criar_produto(
        cenario.categoria,
        "Visita de avaliação",
        &["avaliação", "visita"],
        "",
        0,
    );
    cenario.criar_produto(
        cenario.categoria,
        "Desentupimento de pia",
        &["ralo entupido"],
        "",
        12_345,
    );
    cenario.sincronizar().await;

    let gratis = cenario.buscar("visita de avaliação").await;
    assert_eq!(gratis[0].preco_centavos, 0);
    assert_eq!(gratis[0].preco_texto, "R$ 0,00");

    let com_centavos = cenario.buscar("ralo entupido").await;
    assert_eq!(com_centavos[0].preco_centavos, 12_345);
    assert_eq!(com_centavos[0].preco_texto, "R$ 123,45");
}

#[tokio::test]
async fn busca_e_leitura_nao_cruzam_org() {
    let cenario = Cenario::novo("orgs");
    let produto = cenario.criar_produto(
        cenario.categoria,
        "Dedetização de formigas",
        &["formigas"],
        "R$ 190,00",
        19_000,
    );
    cenario.sincronizar().await;

    let outra = cenario
        .store
        .orgs()
        .criar(NewOrg {
            name: "Outra org".to_owned(),
            slug: format!("outra-{}", AgentId::generate()),
            config_json: "{}".to_owned(),
        })
        .expect("cria a outra org")
        .id;
    let outro_agente = cenario
        .store
        .agents()
        .criar(
            outra,
            NewAgent {
                name: "Atendente da outra".to_owned(),
                slug: format!("outro-{}", AgentId::generate()),
                ..NewAgent::default()
            },
        )
        .expect("cria o outro agente")
        .id;
    cenario
        .catalogo
        .sincronizar_agente(outra, outro_agente)
        .await
        .expect("sincroniza a outra org");

    let catalogo: Arc<dyn BuscadorProdutos> =
        Arc::clone(&cenario.catalogo) as Arc<dyn BuscadorProdutos>;
    let achados = catalogo
        .buscar(
            outra,
            outro_agente,
            "formigas".to_owned(),
            None,
            ParametrosProdutos {
                top_k: 5,
                corte: 0.0,
            },
        )
        .await
        .expect("busca na outra org");
    assert!(achados.is_empty(), "produto de outra org não aparece");

    // Leitura direta por id também não cruza.
    assert!(
        cenario.store.products().obter(outra, produto.id).is_err(),
        "obter com a org errada não devolve o produto"
    );
    assert!(
        cenario
            .store
            .products()
            .ativos_por_ids(outra, &[produto.id])
            .expect("lê por ids")
            .is_empty(),
        "leitura em lote com a org errada não devolve nada"
    );
}

#[tokio::test]
async fn filtro_por_categoria_vinculada() {
    let cenario = Cenario::novo("categorias");
    let pragas = cenario.categoria;
    let hidraulica = cenario.nova_categoria("Hidráulica");
    let baratas = cenario.criar_produto(
        pragas,
        "Dedetização de baratas",
        &["baratas"],
        "R$ 320,00",
        32_000,
    );
    let pia = cenario.criar_produto(
        hidraulica,
        "Desentupimento de pia",
        &["pia entupida", "ralo"],
        "R$ 180,00",
        18_000,
    );
    cenario.sincronizar().await;

    // Sem vínculo: o agente vê o catálogo inteiro (com corte 0, o top-k volta inteiro).
    assert_eq!(cenario.buscar("baratas").await[0].produto, baratas.id);
    assert_eq!(cenario.buscar("ralo").await[0].produto, pia.id);

    // Com vínculo só em pragas: hidráulica sai do índice do agente.
    let agente = cenario
        .store
        .agents()
        .get(cenario.org, cenario.agente)
        .expect("lê o agente");
    cenario
        .store
        .agents()
        .definir_categorias(cenario.org, cenario.agente, agente.version, &[pragas])
        .expect("vincula a categoria");
    cenario.sincronizar().await;

    let achados = cenario.buscar("baratas").await;
    assert_eq!(achados.len(), 1, "só o produto da categoria vinculada");
    assert_eq!(achados[0].produto, baratas.id);
    assert!(
        cenario
            .buscar("ralo")
            .await
            .iter()
            .all(|achado| achado.produto != pia.id),
        "produto de categoria não vinculada sai da busca"
    );

    // Filtro pedido pelo modelo dentro da categoria visível funciona.
    let catalogo: Arc<dyn BuscadorProdutos> =
        Arc::clone(&cenario.catalogo) as Arc<dyn BuscadorProdutos>;
    let filtrado = catalogo
        .buscar(
            cenario.org,
            cenario.agente,
            "baratas".to_owned(),
            Some("pragas urbanas".to_owned()),
            ParametrosProdutos {
                top_k: 3,
                corte: 0.0,
            },
        )
        .await
        .expect("busca com filtro");
    assert_eq!(filtrado.len(), 1);
    assert_eq!(filtrado[0].produto, baratas.id);

    // Categoria não vinculada é recusada (paridade com o POP Atende).
    let negado = catalogo
        .buscar(
            cenario.org,
            cenario.agente,
            "pia".to_owned(),
            Some("hidráulica".to_owned()),
            ParametrosProdutos {
                top_k: 3,
                corte: 0.0,
            },
        )
        .await;
    assert!(negado.is_err(), "categoria fora do vínculo é recusada");

    // O produto da outra categoria continua no banco (só saiu do índice do agente).
    assert!(cenario.store.products().obter(cenario.org, pia.id).is_ok());
}
