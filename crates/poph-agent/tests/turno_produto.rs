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
use poph_core::llm::{ChamadaFerramenta, EventoLlm, LlmProvider, MotivoFim, ProvedorFake, Roteiro};
use poph_rag::{ConfigIndice, Embedder, EmbedderFake, Indice};
use poph_store::{
    NovaCategoria, NovaConversa, NovaMensagem, NovaOrg, NovoAgente, NovoProduto, Pagina, Store,
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
            Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db"))
                .expect("abre o banco"),
        );
        let org = store
            .orgaos()
            .criar(NovaOrg {
                nome: format!("Org {rotulo}"),
                slug: format!("org-{rotulo}-{}", AgentId::generate()),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agentes()
            .criar(
                org,
                NovoAgente {
                    nome: format!("Bia {rotulo}"),
                    slug: format!("bia-{rotulo}"),
                    janela_agrupamento_ms: 0,
                    ..NovoAgente::default()
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
            .instrucoes()
            .salvar(
                org,
                agente,
                0,
                &estruturado.to_json(),
                "# Bia\n\nVocê é a atendente da FixProLar. Nunca invente preço.",
                true,
            )
            .expect("grava as instruções");
        let categoria = store
            .categorias()
            .criar(
                org,
                NovaCategoria {
                    nome: "Pragas urbanas".to_owned(),
                    descricao: String::new(),
                },
            )
            .expect("cria a categoria")
            .id;
        store
            .produtos()
            .criar(
                org,
                NovoProduto {
                    categoria_id: Some(categoria),
                    nome: "Dedetização de baratas".to_owned(),
                    aliases: vec!["baratas".to_owned(), "barata".to_owned()],
                    descricao: "Tratamento com gel e pulverização para eliminar baratas."
                        .to_owned(),
                    preco_texto: "R$ 320,00".to_owned(),
                    preco_centavos: 32_000,
                    unidade: "por visita".to_owned(),
                    condicoes: "Pagamento em até 3x".to_owned(),
                    ..NovoProduto::default()
                },
            )
            .expect("cria o produto");
        let embedder: Arc<dyn Embedder> = Arc::new(EmbedderFake::new());
        let (indice, _) = Indice::abrir(store.caminho_rag(), embedder, ConfigIndice::default())
            .expect("abre o índice");
        let catalogo = CatalogoProdutos::novo(Arc::clone(&store), Arc::new(indice));
        let conversa = store
            .conversas()
            .criar(
                org,
                NovaConversa {
                    agent_id: agente,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    departamento_alvo: None,
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

    fn motor(&self, roteiros: Vec<Roteiro>) -> (Motor, Arc<ProvedorFake>) {
        let fake = Arc::new(ProvedorFake::com_roteiros(roteiros));
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
            .mensagens()
            .gravar(
                self.org,
                NovaMensagem {
                    conversation_id: self.conversa,
                    direcao: poph_store::Direcao::Entrada,
                    papel: poph_store::Papel::Usuario,
                    texto: texto.to_owned(),
                    anexos_json: "[]".to_owned(),
                    id_externo: None,
                    turn_id: None,
                    origem: poph_store::Origem::Agente,
                    autor_nome: None,
                },
            )
            .expect("grava a mensagem")
            .id
    }

    fn turnos(&self) -> Vec<poph_store::Turno> {
        self.store
            .turnos()
            .listar(self.org, self.conversa, Pagina::default())
            .expect("lista os turnos")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

fn resposta(texto: &str) -> Roteiro {
    Roteiro::Eventos(vec![
        EventoLlm::Texto(texto.to_owned()),
        EventoLlm::Fim(MotivoFim::Concluido),
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

    let chamada = Roteiro::Eventos(vec![
        EventoLlm::Ferramenta(ChamadaFerramenta {
            id: "c1".to_owned(),
            nome: "buscar_produto".to_owned(),
            argumentos: serde_json::json!({"termo": "baratas"}),
        }),
        EventoLlm::Fim(MotivoFim::ChamadasFerramenta),
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
    assert_eq!(resumo.estado.como_texto(), "done");
    assert_eq!(resumo.texto, "A dedetização custa R$ 320,00.");

    // O modelo pediu a ferramenta na primeira rodada e recebeu o preço do registro.
    let pedidos = fake.recebidos();
    assert_eq!(
        pedidos.len(),
        2,
        "uma rodada de ferramenta e o fecho em texto"
    );
    let nomes: Vec<String> = pedidos[0]
        .ferramentas
        .iter()
        .map(|ferramenta| ferramenta.nome.clone())
        .collect();
    assert_eq!(nomes, vec!["buscar_produto".to_owned()]);

    let turnos = cenario.turnos();
    assert_eq!(turnos.len(), 1);
    let registrado: serde_json::Value =
        serde_json::from_str(&turnos[0].trechos_json).expect("JSON do registro");
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
    assert_eq!(turnos[0].trechos_json, "[]");
}
