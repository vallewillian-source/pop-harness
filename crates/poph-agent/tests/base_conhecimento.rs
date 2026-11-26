//! Base de conhecimento e ferramenta `consultar_base` (HR-17).
//!
//! Usa o embedder falso (bag of words determinístico): roda sem o modelo instalado e sem rede.
//! O que se prova aqui:
//! 1. o índice acompanha a escrita (criar, editar, apagar) e a busca traz a fonte;
//! 2. `consultar_base` respeita a lista habilitada (D-04) e devolve trecho com fonte;
//! 3. o corte do agente impede injetar trecho sem relação;
//! 4. busca não cruza org;
//! 5. item de 1001 caracteres é recusado e 1000 é aceito;
//! 6. item inativo e grupo inativo saem do índice (D-05).

use std::path::PathBuf;
use std::sync::Arc;

use poph_agent::base_conhecimento::{NOME_FERRAMENTA, ParametrosBusca};
use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::metricas::MetricasMemoria;
use poph_agent::turno::{ContextoFerramenta, RegistroFerramentas, UsoFerramentas, oferecidas};
use poph_agent::{BaseConhecimento, BuscadorBase, ConsultarBase};
use poph_rag::{ConfigIndice, Embedder, EmbedderFake, Indice};
use poph_store::{
    KnowledgeGroupChange, KnowledgeItemChange, NewAgent, NewKnowledgeGroup, NewKnowledgeItem,
    NewOrg, Status, Store,
};
use poph_types::{AgentId, ConversationId, KnowledgeGroupId, KnowledgeItemId, OrgId, TurnId};

struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    grupo: KnowledgeGroupId,
    base: Arc<BaseConhecimento>,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-base-{rotulo}-{}-{}",
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
            .create(NewOrg {
                name: format!("Org {rotulo}"),
                slug: format!("org-{rotulo}-{}", AgentId::generate()),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: format!("Atendente {rotulo}"),
                    slug: format!("atendente-{rotulo}"),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        let grupo = store
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
            .link_agent(org, agente, grupo)
            .expect("liga o grupo ao agente");
        let embedder: Arc<dyn Embedder> = Arc::new(EmbedderFake::new());
        let (indice, _) = Indice::abrir(store.rag_path(), embedder, ConfigIndice::default())
            .expect("abre o índice");
        let base = BaseConhecimento::nova(Arc::clone(&store), Arc::new(indice));
        Self {
            pasta,
            store,
            org,
            agente,
            grupo,
            base,
        }
    }

    fn criar_item(&self, titulo: &str, conteudo: &str) -> KnowledgeItemId {
        self.store
            .knowledge()
            .create_item(
                self.org,
                NewKnowledgeItem {
                    group_id: self.grupo,
                    title: titulo.to_owned(),
                    content: conteudo.to_owned(),
                    tags: Vec::new(),
                    position: 0,
                },
            )
            .expect("cria o item")
            .id
    }

    async fn sincronizar(&self) {
        self.base
            .sincronizar_agente(self.org, self.agente)
            .await
            .expect("sincroniza o índice");
    }

    async fn buscar(&self, consulta: &str, corte: f32) -> Vec<poph_agent::TrechoRecuperado> {
        self.base
            .buscar(
                self.org,
                self.agente,
                consulta.to_owned(),
                ParametrosBusca { top_k: 3, corte },
            )
            .await
            .expect("busca")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

#[tokio::test]
async fn indice_acompanha_criar_editar_e_apagar() {
    let cenario = Cenario::novo("ciclo");
    let item = cenario.criar_item(
        "Prazo de agendamento",
        "O prazo para agendar a dedetização é de 48 horas úteis.",
    );
    cenario.sincronizar().await;

    let achados = cenario.buscar("prazo agendamento dedetização", 0.0).await;
    assert_eq!(achados.len(), 1, "o item novo entra no índice");
    assert_eq!(achados[0].titulo, "Prazo de agendamento");
    assert_eq!(achados[0].grupo, "Políticas");
    assert_eq!(achados[0].item, item);

    // Editar: o termo novo passa a ser achado e o velho sai (o trecho foi regerado).
    let versao = cenario
        .store
        .knowledge()
        .get_item(cenario.org, item)
        .expect("lê o item")
        .version;
    cenario
        .store
        .knowledge()
        .update_item(
            cenario.org,
            item,
            versao,
            KnowledgeItemChange {
                content: Some("O agendamento agora é imediato pelo aplicativo.".to_owned()),
                ..KnowledgeItemChange::default()
            },
        )
        .expect("edita o item");
    cenario.sincronizar().await;
    let novo = cenario.buscar("agendamento aplicativo imediato", 0.0).await;
    assert_eq!(novo.len(), 1);
    assert!(novo[0].texto.contains("aplicativo"));
    let velho = cenario.buscar("Xilofone quartzo vulcanico", 0.0).await;
    assert!(velho.is_empty() || !velho[0].texto.contains("48 horas"));

    // Apagar: o item sai do índice.
    cenario
        .store
        .knowledge()
        .delete_item(cenario.org, item)
        .expect("apaga o item");
    cenario.sincronizar().await;
    assert!(
        cenario
            .buscar("agendamento aplicativo", 0.0)
            .await
            .is_empty(),
        "item apagado não volta da busca"
    );
}

#[tokio::test]
async fn consultar_base_respeita_a_lista_habilitada_e_devolve_fonte() {
    let cenario = Cenario::novo("ferramenta");
    cenario.criar_item(
        "Região atendida",
        "Atendemos a região metropolitana de São Paulo e o ABC paulista.",
    );
    cenario.sincronizar().await;

    let config = Arc::new(ConfigQuente::nova(
        Arc::clone(&cenario.store),
        OpcoesConfigQuente::default(),
        MetricasMemoria::proprio(),
    ));
    let busca: Arc<dyn BuscadorBase> = Arc::clone(&cenario.base) as Arc<dyn BuscadorBase>;
    let registro = ConsultarBase::registro(Arc::clone(&config), busca);
    assert_eq!(registro.nomes(), vec![NOME_FERRAMENTA.to_owned()]);

    // D-04: a ferramenta só é oferecida se o agente a tem habilitada.
    let desabilitada = oferecidas(&registro, &[]);
    assert!(desabilitada.is_empty(), "sem habilitação, não é oferecida");
    let habilitada = oferecidas(&registro, &[NOME_FERRAMENTA.to_owned()]);
    assert_eq!(habilitada.len(), 1);

    let ferramenta = registro.ferramenta(NOME_FERRAMENTA).expect("existe");
    let contexto = ContextoFerramenta {
        org: cenario.org,
        conversa: ConversationId::generate(),
        agente: cenario.agente,
        turno: TurnId::generate(),
        uso: UsoFerramentas::nova(),
    };
    let resposta = ferramenta
        .executar(
            contexto.clone(),
            serde_json::json!({"consulta": "vocês atendem o ABC?"}),
        )
        .await
        .expect("a ferramenta executa");
    assert!(resposta.contains("fonte: Região atendida"), "{resposta}");
    assert!(resposta.contains("grupo: Políticas"), "{resposta}");
    assert!(resposta.contains("ABC paulista"), "{resposta}");

    let sem_argumento = ferramenta.executar(contexto, serde_json::json!({})).await;
    assert!(
        sem_argumento.is_err(),
        "consulta vazia devolve erro ao modelo"
    );
}

#[tokio::test]
async fn corte_barra_trecho_sem_relacao() {
    let cenario = Cenario::novo("corte");
    cenario.criar_item(
        "Prazo de agendamento",
        "O prazo para agendar a dedetização é de 48 horas úteis.",
    );
    cenario.sincronizar().await;
    let sem_corte = cenario.buscar("Xilofone quartzo vulcanico", 0.0).await;
    assert!(!sem_corte.is_empty(), "sem corte, o top-k sempre volta");
    let com_corte = cenario.buscar("Xilofone quartzo vulcanico", 0.4).await;
    assert!(
        com_corte.is_empty(),
        "com corte, o trecho sem relação não é injetado: {com_corte:?}"
    );
}

#[tokio::test]
async fn busca_nao_cruza_org() {
    let cenario = Cenario::novo("orgs");
    cenario.criar_item(
        "Código interno",
        "O código interno do cliente é azul turquesa.",
    );
    cenario.sincronizar().await;
    let outra_org = cenario
        .store
        .orgs()
        .create(NewOrg {
            name: "Outra".to_owned(),
            slug: format!("outra-{}", AgentId::generate()),
            config_json: "{}".to_owned(),
        })
        .expect("cria a outra org")
        .id;
    let achados = cenario
        .base
        .buscar(
            outra_org,
            cenario.agente,
            "código interno azul turquesa".to_owned(),
            ParametrosBusca::default(),
        )
        .await
        .expect("busca");
    assert!(achados.is_empty(), "trecho de outra org não aparece");
}

#[tokio::test]
async fn limite_de_mil_caracteres_e_validado_com_erro_claro() {
    let cenario = Cenario::novo("limite");
    let mil = "a".repeat(1000);
    let item = cenario.criar_item("Item no limite", &mil);
    assert_eq!(
        cenario
            .store
            .knowledge()
            .get_item(cenario.org, item)
            .expect("lê")
            .content
            .chars()
            .count(),
        1000
    );

    let erro = cenario
        .store
        .knowledge()
        .create_item(
            cenario.org,
            NewKnowledgeItem {
                group_id: cenario.grupo,
                title: "Item acima do limite".to_owned(),
                content: "a".repeat(1001),
                tags: Vec::new(),
                position: 0,
            },
        )
        .expect_err("1001 caracteres são recusados");
    let texto = erro.to_string();
    assert!(
        texto.contains("1000") && texto.contains("1001"),
        "mensagem precisa dizer o limite e o tamanho: {texto}"
    );
}

#[tokio::test]
async fn item_e_grupo_inativos_saem_do_indice() {
    let cenario = Cenario::novo("inativo");
    let item = cenario.criar_item(
        "Prazo de agendamento",
        "O prazo para agendar a dedetização é de 48 horas úteis.",
    );
    cenario.sincronizar().await;
    assert!(!cenario.buscar("prazo agendamento", 0.0).await.is_empty());

    // Item inativo (D-05).
    let versao = cenario
        .store
        .knowledge()
        .get_item(cenario.org, item)
        .expect("lê")
        .version;
    cenario
        .store
        .knowledge()
        .update_item(
            cenario.org,
            item,
            versao,
            KnowledgeItemChange {
                status: Some(Status::Inactive),
                ..KnowledgeItemChange::default()
            },
        )
        .expect("inativa o item");
    cenario.sincronizar().await;
    assert!(
        cenario.buscar("prazo agendamento", 0.0).await.is_empty(),
        "item inativo sai do índice"
    );

    // Volta ativo e o grupo inteiro fica inativo.
    let versao = cenario
        .store
        .knowledge()
        .get_item(cenario.org, item)
        .expect("lê")
        .version;
    cenario
        .store
        .knowledge()
        .update_item(
            cenario.org,
            item,
            versao,
            KnowledgeItemChange {
                status: Some(Status::Active),
                ..KnowledgeItemChange::default()
            },
        )
        .expect("reativa o item");
    let versao = cenario
        .store
        .knowledge()
        .get_group(cenario.org, cenario.grupo)
        .expect("lê o grupo")
        .version;
    cenario
        .store
        .knowledge()
        .update_group(
            cenario.org,
            cenario.grupo,
            versao,
            KnowledgeGroupChange {
                status: Some(Status::Inactive),
                ..KnowledgeGroupChange::default()
            },
        )
        .expect("inativa o grupo");
    cenario.sincronizar().await;
    assert!(
        cenario.buscar("prazo agendamento", 0.0).await.is_empty(),
        "grupo inativo tira os itens do índice"
    );

    // E a lista do agente (fonte do índice) não traz nada.
    let itens = cenario
        .store
        .knowledge()
        .agent_items(cenario.org, cenario.agente)
        .expect("lista");
    assert!(itens.is_empty());
}

#[tokio::test]
async fn busca_filtra_elegibilidade_mesmo_com_indice_desatualizado() {
    struct Falha(poph_rag::EmbedderFake);
    impl Embedder for Falha {
        fn identidade(&self) -> &poph_rag::IdentidadeModelo {
            self.0.identidade()
        }
        fn embedir_lote<'a>(&'a self, _: &'a [String]) -> poph_rag::FuturoLote<'a> {
            Box::pin(async { Err(poph_rag::ErroRag::Inferencia("falha simulada".into())) })
        }
    }
    let c = Cenario::novo("elegibilidade");
    for texto in [
        "prazo entrega um",
        "prazo entrega dois",
        "prazo entrega tres",
    ] {
        c.criar_item(texto, texto);
    }
    c.sincronizar().await;
    let antes = c.buscar("prazo entrega", 0.0).await;
    assert_eq!(antes.len(), 3);
    assert_eq!(
        antes,
        c.buscar("prazo entrega", 0.0).await,
        "top-3 permanece igual"
    );
    let grupo = c.store.knowledge().get_group(c.org, c.grupo).unwrap();
    c.store
        .knowledge()
        .update_group(
            c.org,
            c.grupo,
            grupo.version,
            KnowledgeGroupChange {
                status: Some(Status::Inactive),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        c.buscar("prazo entrega", 0.0).await.is_empty(),
        "grupo inativo"
    );
    let grupo = c.store.knowledge().get_group(c.org, c.grupo).unwrap();
    c.store
        .knowledge()
        .update_group(
            c.org,
            c.grupo,
            grupo.version,
            KnowledgeGroupChange {
                status: Some(Status::Active),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(antes, c.buscar("prazo entrega", 0.0).await);
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
        c.buscar("prazo entrega", 0.0).await.is_empty(),
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
        .unlink_agent(c.org, c.agente, c.grupo)
        .unwrap();
    assert!(
        c.buscar("prazo entrega", 0.0).await.is_empty(),
        "grupo desvinculado"
    );
    let novo = c
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
        .link_agent(c.org, c.agente, novo.id)
        .unwrap();
    c.store
        .knowledge()
        .create_item(
            c.org,
            NewKnowledgeItem {
                group_id: novo.id,
                title: "prazo novo".into(),
                content: "prazo entrega novo".into(),
                tags: vec![],
                position: 0,
            },
        )
        .unwrap();
    let (indice, _) = Indice::abrir(
        c.store.rag_path(),
        Arc::new(Falha(EmbedderFake::new())),
        ConfigIndice::default(),
    )
    .unwrap();
    let falha = BaseConhecimento::nova(Arc::clone(&c.store), Arc::new(indice));
    assert!(falha.sincronizar_agente(c.org, c.agente).await.is_err());
    assert!(
        c.buscar("prazo entrega", 0.0).await.is_empty(),
        "falha de reindexação não revela base desvinculada"
    );
}
