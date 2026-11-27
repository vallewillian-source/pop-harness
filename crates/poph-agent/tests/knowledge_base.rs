use std::path::PathBuf;
use std::sync::Arc;

use poph_agent::base_knowledge::{NAME_TOOL, SearchParameters};
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::metrics::MetricsMemory;
use poph_agent::turno::{ToolContext, ToolRegistry, UseTools, offered};
use poph_agent::{KnowledgeBase, KnowledgeSearcher, QueryKnowledge};
use poph_rag::{Embedder, FakeEmbedder, Index, IndexConfig};
use poph_store::{
    KnowledgeGroupChange, KnowledgeItemChange, NewAgent, NewKnowledgeGroup, NewKnowledgeItem,
    NewOrg, Status, Store,
};
use poph_types::{AgentId, ConversationId, KnowledgeGroupId, KnowledgeItemId, OrgId, TurnId};

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    group: KnowledgeGroupId,
    base: Arc<KnowledgeBase>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-base-{label}-{}-{}",
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
        let group = store
            .knowledge()
            .create_group(
                org,
                NewKnowledgeGroup {
                    name: "Políticas".to_owned(),
                    description: String::new(),
                },
            )
            .expect("cria o grupo")
            .id;
        store
            .knowledge()
            .link_agent(org, agente, group)
            .expect("liga o grupo ao agente");
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new());
        let (index, _) =
            Index::open(store.rag_path(), embedder, IndexConfig::default()).expect("abre o índice");
        let base = KnowledgeBase::new(Arc::clone(&store), Arc::new(index));
        Self {
            folder,
            store,
            org,
            agente,
            group,
            base,
        }
    }

    fn criar_item(&self, title: &str, content: &str) -> KnowledgeItemId {
        self.store
            .knowledge()
            .create_item(
                self.org,
                NewKnowledgeItem {
                    group_id: self.group,
                    title: title.to_owned(),
                    content: content.to_owned(),
                    tags: Vec::new(),
                    position: 0,
                },
            )
            .expect("cria o item")
            .id
    }

    async fn synchronise(&self) {
        self.base
            .synchronise_agent(self.org, self.agente)
            .await
            .expect("sincroniza o índice");
    }

    async fn search(&self, query: &str, cutoff: f32) -> Vec<poph_agent::RecoveredPassage> {
        self.base
            .search(
                self.org,
                self.agente,
                query.to_owned(),
                SearchParameters { top_k: 3, cutoff },
            )
            .await
            .expect("busca")
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

#[tokio::test]
async fn index_tracks_create_edit_delete() {
    let scenario = Scenario::new("ciclo");
    let item = scenario.criar_item(
        "Prazo de agendamento",
        "O prazo para agendar a dedetização é de 48 horas úteis.",
    );
    scenario.synchronise().await;

    let findings = scenario.search("prazo agendamento dedetização", 0.0).await;
    assert_eq!(findings.len(), 1, "o item novo entra no índice");
    assert_eq!(findings[0].title, "Prazo de agendamento");
    assert_eq!(findings[0].group, "Políticas");
    assert_eq!(findings[0].item, item);

    let versao = scenario
        .store
        .knowledge()
        .get_item(scenario.org, item)
        .expect("lê o item")
        .version;
    scenario
        .store
        .knowledge()
        .update_item(
            scenario.org,
            item,
            versao,
            KnowledgeItemChange {
                content: Some("O agendamento agora é imediato pelo aplicativo.".to_owned()),
                ..KnowledgeItemChange::default()
            },
        )
        .expect("edita o item");
    scenario.synchronise().await;
    let new = scenario
        .search("agendamento aplicativo imediato", 0.0)
        .await;
    assert_eq!(new.len(), 1);
    assert!(new[0].texto.contains("aplicativo"));
    let old = scenario.search("Xilofone quartzo vulcanico", 0.0).await;
    assert!(old.is_empty() || !old[0].texto.contains("48 horas"));

    scenario
        .store
        .knowledge()
        .delete_item(scenario.org, item)
        .expect("apaga o item");
    scenario.synchronise().await;
    assert!(
        scenario
            .search("agendamento aplicativo", 0.0)
            .await
            .is_empty(),
        "item apagado não volta da busca"
    );
}

#[tokio::test]
async fn query_base_respects_list_enabled_returns_source() {
    let scenario = Scenario::new("ferramenta");
    scenario.criar_item(
        "Região atendida",
        "Atendemos a região metropolitana de São Paulo e o ABC paulista.",
    );
    scenario.synchronise().await;

    let config = Arc::new(ConfigHot::new(
        Arc::clone(&scenario.store),
        HotConfigOptions::default(),
        MetricsMemory::own(),
    ));
    let lookup: Arc<dyn KnowledgeSearcher> =
        Arc::clone(&scenario.base) as Arc<dyn KnowledgeSearcher>;
    let registro = QueryKnowledge::registro(Arc::clone(&config), lookup);
    assert_eq!(registro.names(), vec![NAME_TOOL.to_owned()]);

    let disabled = offered(&registro, &[]);
    assert!(disabled.is_empty(), "sem habilitação, não é oferecida");
    let enabled = offered(&registro, &[NAME_TOOL.to_owned()]);
    assert_eq!(enabled.len(), 1);

    let tool = registro.tool(NAME_TOOL).expect("existe");
    let context = ToolContext {
        org: scenario.org,
        conversa: ConversationId::generate(),
        agente: scenario.agente,
        turno: TurnId::generate(),
        usage: UseTools::new(),
    };
    let response = tool
        .run(
            context.clone(),
            serde_json::json!({"consulta": "vocês atendem o ABC?"}),
        )
        .await
        .expect("a ferramenta executa");
    assert!(response.contains("fonte: Região atendida"), "{response}");
    assert!(response.contains("grupo: Políticas"), "{response}");
    assert!(response.contains("ABC paulista"), "{response}");

    let without_argument = tool.run(context, serde_json::json!({})).await;
    assert!(
        without_argument.is_err(),
        "consulta vazia devolve erro ao modelo"
    );
}

#[tokio::test]
async fn cutoff_blocks_chunk_without_relation() {
    let scenario = Scenario::new("corte");
    scenario.criar_item(
        "Prazo de agendamento",
        "O prazo para agendar a dedetização é de 48 horas úteis.",
    );
    scenario.synchronise().await;
    let without_cutoff = scenario.search("Xilofone quartzo vulcanico", 0.0).await;
    assert!(
        !without_cutoff.is_empty(),
        "sem corte, o top-k sempre volta"
    );
    let with_cutoff = scenario.search("Xilofone quartzo vulcanico", 0.4).await;
    assert!(
        with_cutoff.is_empty(),
        "com corte, o trecho sem relação não é injetado: {with_cutoff:?}"
    );
}

#[tokio::test]
async fn search_does_not_cross_org() {
    let scenario = Scenario::new("orgs");
    scenario.criar_item(
        "Código interno",
        "O código interno do cliente é azul turquesa.",
    );
    scenario.synchronise().await;
    let other_org = scenario
        .store
        .orgs()
        .create(NewOrg {
            name: "Outra".to_owned(),
            slug: format!("outra-{}", AgentId::generate()),
            config_json: "{}".to_owned(),
        })
        .expect("cria a outra org")
        .id;
    let findings = scenario
        .base
        .search(
            other_org,
            scenario.agente,
            "código interno azul turquesa".to_owned(),
            SearchParameters::default(),
        )
        .await
        .expect("busca");
    assert!(findings.is_empty(), "trecho de outra org não aparece");
}

#[tokio::test]
async fn limit_thousand_characters_validated_with_error_clear() {
    let scenario = Scenario::new("limite");
    let mil = "a".repeat(1000);
    let item = scenario.criar_item("Item no limite", &mil);
    assert_eq!(
        scenario
            .store
            .knowledge()
            .get_item(scenario.org, item)
            .expect("lê")
            .content
            .chars()
            .count(),
        1000
    );

    let error = scenario
        .store
        .knowledge()
        .create_item(
            scenario.org,
            NewKnowledgeItem {
                group_id: scenario.group,
                title: "Item acima do limite".to_owned(),
                content: "a".repeat(1001),
                tags: Vec::new(),
                position: 0,
            },
        )
        .expect_err("1001 caracteres são recusados");
    let texto = error.to_string();
    assert!(
        texto.contains("1000") && texto.contains("1001"),
        "mensagem precisa dizer o limite e o tamanho: {texto}"
    );
}

#[tokio::test]
async fn item_group_inactive_leave_index() {
    let scenario = Scenario::new("inativo");
    let item = scenario.criar_item(
        "Prazo de agendamento",
        "O prazo para agendar a dedetização é de 48 horas úteis.",
    );
    scenario.synchronise().await;
    assert!(!scenario.search("prazo agendamento", 0.0).await.is_empty());

    let versao = scenario
        .store
        .knowledge()
        .get_item(scenario.org, item)
        .expect("lê")
        .version;
    scenario
        .store
        .knowledge()
        .update_item(
            scenario.org,
            item,
            versao,
            KnowledgeItemChange {
                status: Some(Status::Inactive),
                ..KnowledgeItemChange::default()
            },
        )
        .expect("inativa o item");
    scenario.synchronise().await;
    assert!(
        scenario.search("prazo agendamento", 0.0).await.is_empty(),
        "item inativo sai do índice"
    );

    let versao = scenario
        .store
        .knowledge()
        .get_item(scenario.org, item)
        .expect("lê")
        .version;
    scenario
        .store
        .knowledge()
        .update_item(
            scenario.org,
            item,
            versao,
            KnowledgeItemChange {
                status: Some(Status::Active),
                ..KnowledgeItemChange::default()
            },
        )
        .expect("reativa o item");
    let versao = scenario
        .store
        .knowledge()
        .get_group(scenario.org, scenario.group)
        .expect("lê o grupo")
        .version;
    scenario
        .store
        .knowledge()
        .update_group(
            scenario.org,
            scenario.group,
            versao,
            KnowledgeGroupChange {
                status: Some(Status::Inactive),
                ..KnowledgeGroupChange::default()
            },
        )
        .expect("inativa o grupo");
    scenario.synchronise().await;
    assert!(
        scenario.search("prazo agendamento", 0.0).await.is_empty(),
        "grupo inativo tira os itens do índice"
    );

    let items = scenario
        .store
        .knowledge()
        .agent_items(scenario.org, scenario.agente)
        .expect("lista");
    assert!(items.is_empty());
}

#[tokio::test]
async fn search_filters_eligibility_same_with_index_outdated() {
    struct Failure(poph_rag::FakeEmbedder);
    impl Embedder for Failure {
        fn identity(&self) -> &poph_rag::ModelIdentity {
            self.0.identity()
        }
        fn embed_batch<'a>(&'a self, _: &'a [String]) -> poph_rag::BatchFuture<'a> {
            Box::pin(async { Err(poph_rag::RagError::Inference("falha simulada".into())) })
        }
    }
    let c = Scenario::new("elegibilidade");
    for texto in [
        "prazo entrega um",
        "prazo entrega dois",
        "prazo entrega tres",
    ] {
        c.criar_item(texto, texto);
    }
    c.synchronise().await;
    let before = c.search("prazo entrega", 0.0).await;
    assert_eq!(before.len(), 3);
    assert_eq!(
        before,
        c.search("prazo entrega", 0.0).await,
        "top-3 permanece igual"
    );
    let group = c.store.knowledge().get_group(c.org, c.group).unwrap();
    c.store
        .knowledge()
        .update_group(
            c.org,
            c.group,
            group.version,
            KnowledgeGroupChange {
                status: Some(Status::Inactive),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        c.search("prazo entrega", 0.0).await.is_empty(),
        "grupo inativo"
    );
    let group = c.store.knowledge().get_group(c.org, c.group).unwrap();
    c.store
        .knowledge()
        .update_group(
            c.org,
            c.group,
            group.version,
            KnowledgeGroupChange {
                status: Some(Status::Active),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(before, c.search("prazo entrega", 0.0).await);
    let agente = c.store.agents().get(c.org, c.agente).unwrap();
    let agente = c
        .store
        .agents()
        .update(
            c.org,
            c.agente,
            agente.version,
            poph_store::AgentChange {
                status: Some(Status::Inactive),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        c.search("prazo entrega", 0.0).await.is_empty(),
        "agente inativo"
    );
    c.store
        .agents()
        .update(
            c.org,
            c.agente,
            agente.version,
            poph_store::AgentChange {
                status: Some(Status::Active),
                ..Default::default()
            },
        )
        .unwrap();
    c.store
        .knowledge()
        .unlink_agent(c.org, c.agente, c.group)
        .unwrap();
    assert!(
        c.search("prazo entrega", 0.0).await.is_empty(),
        "grupo desvinculado"
    );
    let new = c
        .store
        .knowledge()
        .create_group(
            c.org,
            NewKnowledgeGroup {
                name: "Novo".into(),
                description: String::new(),
            },
        )
        .unwrap();
    c.store
        .knowledge()
        .link_agent(c.org, c.agente, new.id)
        .unwrap();
    c.store
        .knowledge()
        .create_item(
            c.org,
            NewKnowledgeItem {
                group_id: new.id,
                title: "prazo novo".into(),
                content: "prazo entrega novo".into(),
                tags: vec![],
                position: 0,
            },
        )
        .unwrap();
    let (index, _) = Index::open(
        c.store.rag_path(),
        Arc::new(Failure(FakeEmbedder::new())),
        IndexConfig::default(),
    )
    .unwrap();
    let failure = KnowledgeBase::new(Arc::clone(&c.store), Arc::new(index));
    assert!(failure.synchronise_agent(c.org, c.agente).await.is_err());
    assert!(
        c.search("prazo entrega", 0.0).await.is_empty(),
        "falha de reindexação não revela base desvinculada"
    );
}
