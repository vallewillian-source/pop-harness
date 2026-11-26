use std::path::PathBuf;
use std::sync::Arc;

use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::metrics::MetricsMemory;
use poph_agent::products::{NAME_TOOL, ProductParameters};
use poph_agent::turno::{ToolContext, ToolRegistry, UseTools, offered};
use poph_agent::{ProductCatalog, ProductSearcher, SearchProduct};
use poph_rag::{Embedder, FakeEmbedder, Index, IndexConfig};
use poph_store::{
    NewAgent, NewCategory, NewOrg, NewProduct, Product, ProductChange, Status, Store,
};
use poph_types::{AgentId, ConversationId, OrgId, ProductCategoryId, TurnId};

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    category: ProductCategoryId,
    catalog: Arc<ProductCatalog>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-produtos-{label}-{}-{}",
            std::process::id(),
            AgentId::generate()
        ));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("cria a pasta");
        let store = Arc::new(
            Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
                .expect("abre o banco"),
        );
        let org = store
            .orgs()
            .create(NewOrg {
                name: format!("Org {label}"),
                slug: format!("org-{label}-{}", AgentId::generate()),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: format!("Atendente {label}"),
                    slug: format!("atendente-{label}"),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        let category = store
            .categories()
            .create(
                org,
                NewCategory {
                    name: "Pragas urbanas".to_owned(),
                    description: String::new(),
                },
            )
            .expect("cria a categoria")
            .id;
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new());
        let (index, _) =
            Index::open(store.rag_path(), embedder, IndexConfig::default()).expect("abre o índice");
        let catalog = ProductCatalog::new(Arc::clone(&store), Arc::new(index));
        Self {
            folder,
            store,
            org,
            agente,
            category,
            catalog,
        }
    }

    fn nova_categoria(&self, nome: &str) -> ProductCategoryId {
        self.store
            .categories()
            .create(
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
        category: ProductCategoryId,
        nome: &str,
        aliases: &[&str],
        price_text: &str,
        cents: i64,
    ) -> Product {
        self.store
            .products()
            .create(
                self.org,
                NewProduct {
                    category_id: Some(category),
                    name: nome.to_owned(),
                    aliases: aliases.iter().map(|alias| (*alias).to_owned()).collect(),
                    description: format!("Serviço de {nome} com garantia."),
                    price_text: price_text.to_owned(),
                    price_cents: cents,
                    unit: "por visita".to_owned(),
                    conditions: "Pagamento em até 3x".to_owned(),
                    ..NewProduct::default()
                },
            )
            .expect("cria o produto")
    }

    async fn synchronise(&self) {
        self.catalog
            .synchronise_agent(self.org, self.agente)
            .await
            .expect("sincroniza o índice");
    }

    async fn search(&self, term: &str) -> Vec<poph_agent::FoundProduct> {
        let catalog: Arc<dyn ProductSearcher> =
            Arc::clone(&self.catalog) as Arc<dyn ProductSearcher>;
        catalog
            .search(
                self.org,
                self.agente,
                term.to_owned(),
                None,
                ProductParameters {
                    top_k: 3,
                    cutoff: 0.0,
                },
            )
            .await
            .expect("busca produtos")
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

fn context(scenario: &Scenario, usage: Arc<UseTools>) -> ToolContext {
    ToolContext {
        org: scenario.org,
        conversa: ConversationId::generate(),
        agente: scenario.agente,
        turno: TurnId::generate(),
        usage,
    }
}

#[tokio::test]
async fn ferramenta_respeita_d04_e_devolve_preco_do_registro() {
    let scenario = Scenario::new("d04");
    let product = scenario.criar_produto(
        scenario.category,
        "Dedetização de baratas",
        &["baratas", "barata"],
        "R$ 320,00",
        32_000,
    );
    scenario.synchronise().await;

    let config = Arc::new(ConfigHot::new(
        Arc::clone(&scenario.store),
        HotConfigOptions::default(),
        MetricsMemory::own(),
    ));
    let catalog: Arc<dyn ProductSearcher> =
        Arc::clone(&scenario.catalog) as Arc<dyn ProductSearcher>;
    let registro = SearchProduct::registro(Arc::clone(&config), catalog);
    assert_eq!(registro.names(), vec![NAME_TOOL.to_owned()]);

    assert!(
        offered(&registro, &[]).is_empty(),
        "D-04: sem habilitação, a ferramenta não é oferecida"
    );
    let enabled = offered(&registro, &[NAME_TOOL.to_owned()]);
    assert_eq!(enabled.len(), 1);
    assert_eq!(enabled[0].nome, NAME_TOOL);

    let tool = registro.tool(NAME_TOOL).expect("existe");
    let usage = UseTools::new();
    let response = tool
        .run(
            context(&scenario, Arc::clone(&usage)),
            serde_json::json!({"termo": "baratas"}),
        )
        .await
        .expect("a ferramenta executa");
    assert!(response.contains(&product.id.to_string()), "{response}");
    assert!(response.contains("R$ 320,00"), "{response}");
    assert!(response.contains("32000 centavos"), "{response}");
    assert!(response.contains("<produtos_encontrados>"), "{response}");

    let used = usage.products();
    assert_eq!(used.len(), 1);
    assert_eq!(used[0].product, product.id);
    assert_eq!(used[0].price_cents, 32_000);

    let sem_argumento = tool
        .run(context(&scenario, usage), serde_json::json!({}))
        .await;
    assert!(sem_argumento.is_err(), "termo vazio devolve erro ao modelo");
}

#[tokio::test]
async fn produto_inativo_ou_arquivado_sai_da_busca_e_da_ferramenta() {
    let scenario = Scenario::new("inativo");
    let product = scenario.criar_produto(
        scenario.category,
        "Dedetização de cupins",
        &["cupins"],
        "R$ 450,00",
        45_000,
    );
    scenario.synchronise().await;
    assert_eq!(scenario.search("cupins").await.len(), 1);

    let current = scenario
        .store
        .products()
        .get(scenario.org, product.id)
        .expect("lê o produto");
    scenario
        .store
        .products()
        .update(
            scenario.org,
            product.id,
            current.version,
            ProductChange {
                status: Some(Status::Inactive),
                ..ProductChange::default()
            },
        )
        .expect("inativa o produto");
    scenario.synchronise().await;
    assert!(
        scenario.search("cupins").await.is_empty(),
        "produto inativo não volta da busca"
    );

    let current = scenario
        .store
        .products()
        .get(scenario.org, product.id)
        .expect("lê o produto");
    scenario
        .store
        .products()
        .update(
            scenario.org,
            product.id,
            current.version,
            ProductChange {
                status: Some(Status::Archived),
                ..ProductChange::default()
            },
        )
        .expect("arquiva o produto");
    scenario.synchronise().await;
    assert!(scenario.search("cupins").await.is_empty());
}

#[tokio::test]
async fn mudar_preco_aparece_na_resposta_seguinte_sem_reindexar() {
    let scenario = Scenario::new("preco");
    let product = scenario.criar_produto(
        scenario.category,
        "Desratização",
        &["ratos", "ratazanas"],
        "R$ 250,00",
        25_000,
    );
    scenario.synchronise().await;

    let antes = scenario
        .catalog
        .index()
        .indexed_products(scenario.org, scenario.agente)
        .expect("hashes");
    assert_eq!(scenario.search("ratos").await[0].price_cents, 25_000);

    let current = scenario
        .store
        .products()
        .get(scenario.org, product.id)
        .expect("lê o produto");
    scenario
        .store
        .products()
        .update(
            scenario.org,
            product.id,
            current.version,
            ProductChange {
                price_text: Some("R$ 279,90".to_owned()),
                price_cents: Some(27_990),
                ..ProductChange::default()
            },
        )
        .expect("muda o preço");

    let findings = scenario.search("ratos").await;
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].price_cents, 27_990);
    assert_eq!(findings[0].price_text, "R$ 279,90");
    assert_eq!(
        antes,
        scenario
            .catalog
            .index()
            .indexed_products(scenario.org, scenario.agente)
            .expect("hashes"),
        "mudar preço não mexe no índice"
    );
}

#[tokio::test]
async fn preco_zero_e_centavos_saem_exatos() {
    let scenario = Scenario::new("precos");
    scenario.criar_produto(
        scenario.category,
        "Visita de avaliação",
        &["avaliação", "visita"],
        "",
        0,
    );
    scenario.criar_produto(
        scenario.category,
        "Desentupimento de pia",
        &["ralo entupido"],
        "",
        12_345,
    );
    scenario.synchronise().await;

    let gratis = scenario.search("visita de avaliação").await;
    assert_eq!(gratis[0].price_cents, 0);
    assert_eq!(gratis[0].price_text, "R$ 0,00");

    let com_centavos = scenario.search("ralo entupido").await;
    assert_eq!(com_centavos[0].price_cents, 12_345);
    assert_eq!(com_centavos[0].price_text, "R$ 123,45");
}

#[tokio::test]
async fn busca_e_leitura_nao_cruzam_org() {
    let scenario = Scenario::new("orgs");
    let product = scenario.criar_produto(
        scenario.category,
        "Dedetização de formigas",
        &["formigas"],
        "R$ 190,00",
        19_000,
    );
    scenario.synchronise().await;

    let other = scenario
        .store
        .orgs()
        .create(NewOrg {
            name: "Outra org".to_owned(),
            slug: format!("outra-{}", AgentId::generate()),
            config_json: "{}".to_owned(),
        })
        .expect("cria a outra org")
        .id;
    let outro_agente = scenario
        .store
        .agents()
        .create(
            other,
            NewAgent {
                name: "Atendente da outra".to_owned(),
                slug: format!("outro-{}", AgentId::generate()),
                ..NewAgent::default()
            },
        )
        .expect("cria o outro agente")
        .id;
    scenario
        .catalog
        .synchronise_agent(other, outro_agente)
        .await
        .expect("sincroniza a outra org");

    let catalog: Arc<dyn ProductSearcher> =
        Arc::clone(&scenario.catalog) as Arc<dyn ProductSearcher>;
    let findings = catalog
        .search(
            other,
            outro_agente,
            "formigas".to_owned(),
            None,
            ProductParameters {
                top_k: 5,
                cutoff: 0.0,
            },
        )
        .await
        .expect("busca na outra org");
    assert!(findings.is_empty(), "produto de outra org não aparece");

    assert!(
        scenario.store.products().get(other, product.id).is_err(),
        "obter com a org errada não devolve o produto"
    );
    assert!(
        scenario
            .store
            .products()
            .active_by_ids(other, &[product.id])
            .expect("lê por ids")
            .is_empty(),
        "leitura em lote com a org errada não devolve nada"
    );
}

#[tokio::test]
async fn filtro_por_categoria_vinculada() {
    let scenario = Scenario::new("categorias");
    let pragas = scenario.category;
    let hidraulica = scenario.nova_categoria("Hidráulica");
    let baratas = scenario.criar_produto(
        pragas,
        "Dedetização de baratas",
        &["baratas"],
        "R$ 320,00",
        32_000,
    );
    let pia = scenario.criar_produto(
        hidraulica,
        "Desentupimento de pia",
        &["pia entupida", "ralo"],
        "R$ 180,00",
        18_000,
    );
    scenario.synchronise().await;

    assert_eq!(scenario.search("baratas").await[0].product, baratas.id);
    assert_eq!(scenario.search("ralo").await[0].product, pia.id);

    let agente = scenario
        .store
        .agents()
        .get(scenario.org, scenario.agente)
        .expect("lê o agente");
    scenario
        .store
        .agents()
        .define_categories(scenario.org, scenario.agente, agente.version, &[pragas])
        .expect("vincula a categoria");
    scenario.synchronise().await;

    let findings = scenario.search("baratas").await;
    assert_eq!(findings.len(), 1, "só o produto da categoria vinculada");
    assert_eq!(findings[0].product, baratas.id);
    assert!(
        scenario
            .search("ralo")
            .await
            .iter()
            .all(|finding| finding.product != pia.id),
        "produto de categoria não vinculada sai da busca"
    );

    let catalog: Arc<dyn ProductSearcher> =
        Arc::clone(&scenario.catalog) as Arc<dyn ProductSearcher>;
    let filtrado = catalog
        .search(
            scenario.org,
            scenario.agente,
            "baratas".to_owned(),
            Some("pragas urbanas".to_owned()),
            ProductParameters {
                top_k: 3,
                cutoff: 0.0,
            },
        )
        .await
        .expect("busca com filtro");
    assert_eq!(filtrado.len(), 1);
    assert_eq!(filtrado[0].product, baratas.id);

    let negado = catalog
        .search(
            scenario.org,
            scenario.agente,
            "pia".to_owned(),
            Some("hidráulica".to_owned()),
            ProductParameters {
                top_k: 3,
                cutoff: 0.0,
            },
        )
        .await;
    assert!(negado.is_err(), "categoria fora do vínculo é recusada");

    assert!(scenario.store.products().get(scenario.org, pia.id).is_ok());
}
