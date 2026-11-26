//! Turno que usa o catálogo de produtos (HR-18).
//!
//! Prova o caminho completo que a tarefa pede: o modelo pede `buscar_produto`, a ferramenta devolve
//! preço e id do registro, e o turno grava em `turns.retrieved_json` os produtos usados com o preço
//! aplicado — no mesmo formato discriminado que a HR-17 usa para os trechos.
//!
//! Provedor falso e pausa que não dorme: nenhum teste toca a rede nem espera de verdade.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasMemoria, MetricasTurno};
use poph_agent::turno::{FuturoPausa, Motor, ObservadorNulo, OpcoesTurno, Pausa};
use poph_agent::{BuscadorProdutos, BuscarProduto, CatalogoProdutos};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, LlmProvider, Script, ToolCall};
use poph_rag::{Embedder, FakeEmbedder, Index, IndexConfig};
use poph_store::{
    NewAgent, NewCategory, NewConversation, NewMessage, NewOrg, NewProduct, Page, Store,
};
use poph_types::{AgentId, ConversationId, OrgId, StructuredInstructions, ToolConfig};

/// Pausa que não dorme: a janela de agrupamento é irrelevante aqui.
#[derive(Default, Clone)]
struct PausaFalsa;

impl Pausa for PausaFalsa {
    fn esperar(&self, _duracao: Duration) -> FuturoPausa {
        Box::pin(async {})
    }
}

struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    conversa: ConversationId,
    catalogo: Arc<CatalogoProdutos>,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-turno-produto-{rotulo}-{}-{}",
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
                    name: format!("Bia {rotulo}"),
                    slug: format!("bia-{rotulo}"),
                    grouping_window_ms: 0,
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        let estruturado = StructuredInstructions {
            tools: vec![ToolConfig {
                name: "buscar_produto".to_owned(),
                enabled: true,
                when_to_use: String::new(),
            }],
            ..StructuredInstructions::default()
        };
        store
            .instructions()
            .save(
                org,
                agente,
                0,
                &estruturado.to_json(),
                "# Bia\n\nVocê é a atendente da FixProLar. Nunca invente preço.",
                true,
            )
            .expect("grava as instruções");
        let categoria = store
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
        store
            .products()
            .create(
                org,
                NewProduct {
                    category_id: Some(categoria),
                    name: "Dedetização de baratas".to_owned(),
                    aliases: vec!["baratas".to_owned(), "barata".to_owned()],
                    description: "Tratamento com gel e pulverização para eliminar baratas."
                        .to_owned(),
                    price_text: "R$ 320,00".to_owned(),
                    price_cents: 32_000,
                    unit: "por visita".to_owned(),
                    conditions: "Pagamento em até 3x".to_owned(),
                    ..NewProduct::default()
                },
            )
            .expect("cria o produto");
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new());
        let (indice, _) =
            Index::open(store.rag_path(), embedder, IndexConfig::default()).expect("abre o índice");
        let catalogo = CatalogoProdutos::novo(Arc::clone(&store), Arc::new(indice));
        let conversa = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("abre a conversa")
            .id;
        Self {
            pasta,
            store,
            org,
            agente,
            conversa,
            catalogo,
        }
    }

    fn motor(&self, roteiros: Vec<Script>) -> (Motor, Arc<FakeProvider>) {
        let fake = Arc::new(FakeProvider::with_scripts(roteiros));
        let provedor: Arc<dyn LlmProvider> = fake.clone();
        let config = ConfigQuente::nova(
            Arc::clone(&self.store),
            OpcoesConfigQuente::default(),
            MetricasMemoria::proprio(),
        );
        let limitador = Arc::new(Limitador::novo(
            OpcoesLimites::default(),
            MetricasMemoria::proprio(),
        ));
        let catalogo: Arc<dyn BuscadorProdutos> =
            Arc::clone(&self.catalogo) as Arc<dyn BuscadorProdutos>;
        let motor = Motor::novo(
            Arc::clone(&self.store),
            config,
            limitador,
            provedor,
            OpcoesTurno::default(),
            MetricasTurno::proprio(),
        )
        .com_pausa(Arc::new(PausaFalsa));
        let registro = BuscarProduto::registro(Arc::clone(motor.config_quente()), catalogo);
        let motor = motor.com_ferramentas(Arc::new(registro));
        (motor, fake)
    }

    fn falar(&self, texto: &str) -> poph_types::MessageId {
        self.store
            .messages()
            .write(
                self.org,
                NewMessage {
                    conversation_id: self.conversa,
                    direction: poph_store::Direction::Inbound,
                    role: poph_store::Role::User,
                    texto: texto.to_owned(),
                    attachments_json: "[]".to_owned(),
                    external_id: None,
                    turn_id: None,
                    origin: poph_store::Origin::Agente,
                    author_name: None,
                },
            )
            .expect("grava a mensagem")
            .id
    }

    fn turnos(&self) -> Vec<poph_store::Turno> {
        self.store
            .turns()
            .list(self.org, self.conversa, Page::default())
            .expect("lista os turnos")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

fn resposta(texto: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(texto.to_owned()),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

#[tokio::test]
async fn turno_grava_o_produto_usado_com_o_preco_do_registro() {
    let cenario = Cenario::novo("registro");
    cenario
        .catalogo
        .sincronizar_agente(cenario.org, cenario.agente)
        .await
        .expect("sincroniza o índice");

    let chamada = Script::Events(vec![
        LlmEvent::Tool(ToolCall {
            id: "c1".to_owned(),
            nome: "buscar_produto".to_owned(),
            arguments: serde_json::json!({"termo": "baratas"}),
        }),
        LlmEvent::Fim(FinishReason::ToolCalls),
    ]);
    let (motor, fake) = cenario.motor(vec![chamada, resposta("A dedetização custa R$ 320,00.")]);

    let mensagem = cenario.falar("quanto custa a dedetização de baratas?");
    let resumo = motor
        .executar(
            poph_agent::turno::Entrada::nova(cenario.org, cenario.conversa, mensagem),
            &mut ObservadorNulo,
        )
        .await
        .expect("o turno roda");
    assert_eq!(resumo.estado.as_text(), "done");
    assert_eq!(resumo.texto, "A dedetização custa R$ 320,00.");

    // O modelo pediu a ferramenta na primeira rodada e recebeu o preço do registro.
    let pedidos = fake.received();
    assert_eq!(
        pedidos.len(),
        2,
        "uma rodada de ferramenta e o fecho em texto"
    );
    let nomes: Vec<String> = pedidos[0]
        .tools
        .iter()
        .map(|ferramenta| ferramenta.nome.clone())
        .collect();
    assert_eq!(nomes, vec!["buscar_produto".to_owned()]);

    let turnos = cenario.turnos();
    assert_eq!(turnos.len(), 1);
    let registrado: serde_json::Value =
        serde_json::from_str(&turnos[0].passages_json).expect("JSON do registro");
    let itens = registrado.as_array().expect("lista");
    let produto = itens
        .iter()
        .find(|item| item["tipo"] == "produto")
        .expect("o produto usado está no registro");
    assert_eq!(produto["nome"], "Dedetização de baratas");
    assert_eq!(produto["preco_centavos"], 32_000);
    assert_eq!(produto["preco_texto"], "R$ 320,00");
    assert_eq!(produto["unidade"], "por visita");
}

#[tokio::test]
async fn turno_sem_produto_mantem_o_registro_vazio() {
    let cenario = Cenario::novo("vazio");
    let (motor, _) = cenario.motor(vec![resposta("Olá! Como posso ajudar?")]);
    let mensagem = cenario.falar("oi");
    motor
        .executar(
            poph_agent::turno::Entrada::nova(cenario.org, cenario.conversa, mensagem),
            &mut ObservadorNulo,
        )
        .await
        .expect("o turno roda");
    let turnos = cenario.turnos();
    assert_eq!(turnos[0].passages_json, "[]");
}
