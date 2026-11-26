//! Efeito do G5 no turno (HR-33): o Jev escolhe o produto e o turno dispensa a ferramenta.
//!
//! O que se prova aqui:
//! 1. com o gate ligado e um id **de candidato**, o produto entra no bloco volátil (com o preço do
//!    registro), `buscar_produto` não é oferecida e o turno termina sem rodada de ferramenta;
//! 2. id inventado, `nenhum`, confiança baixa, Jev fora do ar e busca vazia caem no comportamento
//!    de hoje — candidatos vão ao LLM e a ferramenta é oferecida —, e o turno nunca cai;
//! 3. mesmo que o modelo invente a chamada, `buscar_produto` não é executada no turno em que o G5
//!    escolheu o produto (a busca não é repetida);
//! 4. gate desligado não faz busca de produtos nenhuma (o turno é o de antes da HR-33);
//! 5. G3 em `nada` dispensa o G5: a mensagem social não paga a busca de produtos.
//!
//! Provedor falso, busca falsa e pausa que não dorme: nenhum teste toca a rede.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use poph_agent::base_conhecimento::{
    BuscadorBase, ConsultarBase, FuturoBusca, ParametrosBusca, TrechoRecuperado,
};
use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasMemoria, MetricasTurno};
use poph_agent::produtos::{
    BuscadorProdutos, BuscarProduto, FuturoProdutos, ParametrosProdutos, ProdutoEncontrado,
};
use poph_agent::turno::{
    Entrada, FuturoPausa, Motor, ObservadorNulo, OpcoesTurno, Pausa, ResumoTurno,
};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, Script, ToolCall, Usage};
use poph_jev::{
    AvaliadorGates, BatchResponse, Decisao, DecisionOrigin, G3_NOTHING, G5_NONE, GateOverrides,
    IdGate, JevError, JevFalso, JevResponse as RespostaJev, JevState, MotorJev, OpcoesJev,
    ResultadoJev,
};
use poph_store::{Direction, NewAgent, NewConversation, NewMessage, NewOrg, Role, Store};
use poph_types::{
    AgentId, ConversationId, JevGates, MessageId, OrgId, ProductId, StructuredInstructions,
    ToolConfig,
};

/// Produto que a busca falsa devolve. Nome e preço servem de prova no prompt.
const NOME_BARATAS: &str = "Dedetização de baratas";
const PRECO_BARATAS: &str = "R$ 180,00";
const NOME_CUPINS: &str = "Descupinização de cupins";
const PRECO_CUPINS: &str = "R$ 350,00";

struct SemEspera;

impl Pausa for SemEspera {
    fn esperar(&self, _duracao: Duration) -> FuturoPausa {
        Box::pin(async {})
    }
}

/// Busca na base falsa: devolve um trecho e não atrapalha o teste.
struct BuscaSimples {
    trechos: Vec<TrechoRecuperado>,
}

impl BuscadorBase for BuscaSimples {
    fn buscar(
        &self,
        _org: OrgId,
        _agente: AgentId,
        _consulta: String,
        _parametros: ParametrosBusca,
    ) -> FuturoBusca<'_> {
        let trechos = self.trechos.clone();
        Box::pin(async move { Ok(trechos) })
    }
}

/// Catálogo falso: devolve dois produtos e conta as buscas (a prova de que a pré-consulta rodou).
struct CatalogoContado {
    buscas: Arc<AtomicU64>,
    produtos: Vec<ProdutoEncontrado>,
}

impl CatalogoContado {
    fn novo(baratas: ProductId, cupins: ProductId, buscas: Arc<AtomicU64>) -> Arc<Self> {
        Arc::new(Self {
            buscas,
            produtos: vec![
                produto(baratas, NOME_BARATAS, PRECO_BARATAS, 18_000),
                produto(cupins, NOME_CUPINS, PRECO_CUPINS, 35_000),
            ],
        })
    }

    fn vazio(buscas: Arc<AtomicU64>) -> Arc<Self> {
        Arc::new(Self {
            buscas,
            produtos: Vec::new(),
        })
    }
}

fn produto(id: ProductId, nome: &str, preco: &str, centavos: i64) -> ProdutoEncontrado {
    ProdutoEncontrado {
        produto: id,
        nome: nome.to_owned(),
        categoria: "Pragas".to_owned(),
        preco_texto: preco.to_owned(),
        preco_centavos: centavos,
        unidade: "por visita".to_owned(),
        inclui: String::new(),
        exclui: String::new(),
        condicoes: "Garantia de 90 dias.".to_owned(),
        similaridade: 0.81,
        relevancia: 0.5,
    }
}

impl BuscadorProdutos for CatalogoContado {
    fn buscar(
        &self,
        _org: OrgId,
        _agente: AgentId,
        _termo: String,
        _categoria: Option<String>,
        _parametros: ParametrosProdutos,
    ) -> FuturoProdutos<'_> {
        self.buscas.fetch_add(1, Ordering::SeqCst);
        let produtos = self.produtos.clone();
        Box::pin(async move { Ok(produtos) })
    }
}

/// Banco, org, agente (base e catálogo habilitados) e conversa de verdade.
struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    conversa: ConversationId,
    busca: Arc<dyn BuscadorBase>,
    catalogo: Arc<dyn BuscadorProdutos>,
    buscas_produto: Arc<AtomicU64>,
    baratas: ProductId,
    metricas: MetricasTurno,
}

impl Cenario {
    fn novo(rotulo: &str, catalogo_vazio: bool) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-hr33-{rotulo}-{}-{}",
            std::process::id(),
            AgentId::generate()
        ));
        let _ = std::fs::remove_dir_all(&pasta);
        std::fs::create_dir_all(&pasta).expect("pasta do teste");
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
            tools: vec![
                ToolConfig {
                    name: "consultar_base".to_owned(),
                    enabled: true,
                    when_to_use: String::new(),
                },
                ToolConfig {
                    name: "buscar_produto".to_owned(),
                    enabled: true,
                    when_to_use: String::new(),
                },
            ],
            ..StructuredInstructions::default()
        };
        store
            .instructions()
            .save(
                org,
                agente,
                0,
                &estruturado.to_json(),
                "# Bia\n\nVocê é a atendente da FixProLar.",
                true,
            )
            .expect("grava as instruções");
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
        let baratas = ProductId::generate();
        let cupins = ProductId::generate();
        let buscas_produto = Arc::new(AtomicU64::new(0));
        let catalogo: Arc<dyn BuscadorProdutos> = if catalogo_vazio {
            CatalogoContado::vazio(Arc::clone(&buscas_produto))
        } else {
            CatalogoContado::novo(baratas, cupins, Arc::clone(&buscas_produto))
        };
        let busca: Arc<dyn BuscadorBase> = Arc::new(BuscaSimples {
            trechos: vec![TrechoRecuperado {
                item: poph_types::KnowledgeItemId::generate(),
                titulo: "Prazo de entrega".to_owned(),
                grupo: "Políticas da loja".to_owned(),
                texto: "Prazo de entrega: até 8 dias úteis.".to_owned(),
                similaridade: 0.8,
                relevancia: 0.5,
            }],
        });
        Self {
            pasta,
            store,
            org,
            conversa,
            busca,
            catalogo,
            buscas_produto,
            baratas,
            metricas: MetricasTurno::proprio(),
        }
    }

    fn catalogo_buscas(&self) -> u64 {
        self.buscas_produto.load(Ordering::SeqCst)
    }

    fn falar(&self, texto: &str) -> MessageId {
        self.store
            .messages()
            .write(
                self.org,
                NewMessage {
                    conversation_id: self.conversa,
                    direction: Direction::Inbound,
                    role: Role::User,
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

    fn motor(
        &self,
        roteiros: Vec<Script>,
        jev: Option<Arc<dyn AvaliadorGates>>,
    ) -> (Motor, Arc<FakeProvider>) {
        let fake = Arc::new(FakeProvider::with_scripts(roteiros));
        let config = ConfigQuente::nova(
            Arc::clone(&self.store),
            OpcoesConfigQuente::default(),
            MetricasMemoria::proprio(),
        );
        let limitador = Arc::new(Limitador::novo(
            OpcoesLimites::default(),
            MetricasMemoria::proprio(),
        ));
        let mut motor = Motor::novo(
            Arc::clone(&self.store),
            config,
            limitador,
            fake.clone(),
            OpcoesTurno::default(),
            self.metricas.clone(),
        )
        .com_pausa(Arc::new(SemEspera))
        .com_busca(Arc::clone(&self.busca))
        .com_produtos(Arc::clone(&self.catalogo));
        let registro =
            ConsultarBase::registro(Arc::clone(motor.config_quente()), Arc::clone(&self.busca))
                .com(BuscarProduto::nova(
                    Arc::clone(motor.config_quente()),
                    Arc::clone(&self.catalogo),
                ));
        motor = motor.com_ferramentas(Arc::new(registro));
        if let Some(jev) = jev {
            motor = motor.com_jev(jev);
        }
        (motor, fake)
    }

    async fn turno(&self, motor: &Motor, gatilho: MessageId) -> ResumoTurno {
        motor
            .executar(
                Entrada::nova(self.org, self.conversa, gatilho),
                &mut ObservadorNulo,
            )
            .await
            .expect("o turno roda")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

fn roteiro_texto(texto: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(texto.to_owned()),
        LlmEvent::Usage(Usage {
            input: 100,
            saida: 10,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

fn so_g5() -> JevGates {
    JevGates {
        g5: true,
        ..JevGates::default()
    }
}

fn motor_jev(jev: Arc<JevFalso>, gates: JevGates) -> Arc<dyn AvaliadorGates> {
    Arc::new(MotorJev::novo(jev, gates, "jev-latest"))
}

fn escolhas(itens: &[(&str, &str)]) -> BTreeMap<String, String> {
    itens
        .iter()
        .map(|(id, escolha)| ((*id).to_owned(), (*escolha).to_owned()))
        .collect()
}

fn nomes_das_ferramentas(pedido: &poph_core::llm::Request) -> Vec<String> {
    pedido
        .tools
        .iter()
        .map(|ferramenta| ferramenta.nome.clone())
        .collect()
}

fn texto_do_prompt(pedido: &poph_core::llm::Request) -> String {
    pedido
        .messages
        .iter()
        .map(|mensagem| match mensagem {
            poph_core::llm::Message::System(texto) => texto.clone(),
            poph_core::llm::Message::User(partes) => partes
                .iter()
                .map(|parte| match parte {
                    poph_core::llm::Part::Texto(texto) => texto.clone(),
                    _ => String::new(),
                })
                .collect::<Vec<_>>()
                .join("\n"),
            poph_core::llm::Message::Assistant { texto, .. } => texto.clone(),
            poph_core::llm::Message::Resultado {
                content: conteudo, ..
            } => conteudo.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn turnos_json(cenario: &Cenario) -> (String, String) {
    let turnos = cenario
        .store
        .turns()
        .list(
            cenario.org,
            cenario.conversa,
            poph_store::Page::with_limit(5),
        )
        .expect("lista os turnos");
    (turnos[0].passages_json.clone(), turnos[0].jev_json.clone())
}

// ───────────────────────────── efeito ─────────────────────────────

/// G5 escolheu: o produto entra no prompt, `buscar_produto` sai, e o turno não tem rodada de
/// ferramenta.
#[tokio::test]
async fn g5_escolhe_o_produto_e_dispensa_a_ferramenta() {
    let cenario = Cenario::novo("escolhe", false);
    let baratas = cenario.baratas.to_string();
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g5_produto",
        baratas.as_str(),
    )])));
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto(
            "A dedetização de baratas custa R$ 180,00 por visita.",
        )],
        Some(motor_jev(Arc::clone(&jev), so_g5())),
    );

    let gatilho = cenario.falar("quanto custa para matar as baratas daqui de casa?");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    assert_eq!(resumo.tempos.rodadas_ferramenta, 0);
    assert_eq!(
        provedor.received().len(),
        1,
        "uma chamada ao modelo, sem ferramenta"
    );
    let pedido = &provedor.received()[0];
    let prompt = texto_do_prompt(pedido);
    assert!(
        prompt.contains(NOME_BARATAS),
        "o produto escolhido entra no prompt"
    );
    assert!(prompt.contains(PRECO_BARATAS), "o preço vem do registro");
    assert!(
        !prompt.contains(NOME_CUPINS),
        "só o produto escolhido entra no bloco, não a lista inteira"
    );
    let ferramentas = nomes_das_ferramentas(pedido);
    assert!(
        !ferramentas.contains(&"buscar_produto".to_owned()),
        "o catálogo já foi consultado: {ferramentas:?}"
    );
    assert!(
        ferramentas.contains(&"consultar_base".to_owned()),
        "{ferramentas:?}"
    );

    // Registro do turno: o produto escolhido conta como usado, e a decisão do G5 fica no `jev_json`.
    let (trechos_json, jev_json) = turnos_json(&cenario);
    assert!(trechos_json.contains(&baratas), "{trechos_json}");
    assert!(
        trechos_json.contains("\"tipo\":\"produto\""),
        "{trechos_json}"
    );
    assert!(jev_json.contains("\"gate\":\"g5\""), "{jev_json}");
    assert!(jev_json.contains(&baratas), "{jev_json}");
    assert_eq!(cenario.metricas.produtos_escolhidos_g5.value(), 1);
    assert_eq!(
        cenario.catalogo_buscas(),
        1,
        "a busca de produtos roda uma vez, antes do modelo"
    );
}

/// Avaliador com defeito (ou Jev que devolveu id inventado sem passar pelo gate): a decisão do G5
/// chega com um id **fora dos candidatos** e o turno não injeta nada — a validação é do código.
struct JevInventor {
    produto: String,
}

impl AvaliadorGates for JevInventor {
    fn gates_efetivos(&self, _ajuste: Option<GateOverrides>) -> JevGates {
        JevGates {
            g5: true,
            ..JevGates::default()
        }
    }

    fn avaliar<'a>(
        &'a self,
        _estado: JevState,
        _ajuste: Option<GateOverrides>,
    ) -> futures::future::BoxFuture<'a, ResultadoJev> {
        let produto = self.produto.clone();
        Box::pin(async move {
            ResultadoJev {
                decisoes: vec![Decisao {
                    gate: IdGate::G5,
                    choice: Some(produto),
                    origin: DecisionOrigin::Jev,
                    confidence: 0.99,
                    margin: 0.9,
                    motivo: None,
                    ms: 1,
                    tokens: 1,
                }],
                ..ResultadoJev::default()
            }
        })
    }
}

/// A decisão do avaliador não é confiável por si: id que não é candidato do turno é descartado.
#[tokio::test]
async fn g5_id_fora_dos_candidatos_nunca_e_injetado_pelo_codigo() {
    let cenario = Cenario::novo("fora-candidatos", false);
    let jev: Arc<dyn AvaliadorGates> = Arc::new(JevInventor {
        produto: "p-de-outra-org".to_owned(),
    });
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Vou confirmar o valor com a equipe.")],
        Some(jev),
    );

    let gatilho = cenario.falar("quanto custa a dedetização?");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    let pedido = &provedor.received()[0];
    let prompt = texto_do_prompt(pedido);
    assert!(
        !prompt.contains(PRECO_BARATAS),
        "nada de produto fora dos candidatos"
    );
    assert!(!prompt.contains(NOME_CUPINS));
    assert!(nomes_das_ferramentas(pedido).contains(&"buscar_produto".to_owned()));
    assert_eq!(cenario.metricas.produtos_escolhidos_g5.value(), 0);
}

/// Id inventado (fora dos candidatos) não injeta nada: o turno é o de hoje, com a ferramenta.
#[tokio::test]
async fn g5_id_inventado_mantem_o_comportamento_de_hoje() {
    let cenario = Cenario::novo("inventado", false);
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g5_produto",
        "p-produto-que-nao-existe",
    )])));
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Vou confirmar o valor com a equipe.")],
        Some(motor_jev(Arc::clone(&jev), so_g5())),
    );

    let gatilho = cenario.falar("quanto custa a dedetização?");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    let pedido = &provedor.received()[0];
    let prompt = texto_do_prompt(pedido);
    assert!(
        !prompt.contains(NOME_BARATAS),
        "produto inventado não entra no prompt"
    );
    assert!(!prompt.contains(PRECO_BARATAS));
    let ferramentas = nomes_das_ferramentas(pedido);
    assert!(
        ferramentas.contains(&"buscar_produto".to_owned()),
        "sem escolha válida, o LLM continua com a ferramenta: {ferramentas:?}"
    );
    assert_eq!(cenario.metricas.produtos_escolhidos_g5.value(), 0);
}

/// `nenhum` é abstenção: comportamento de hoje.
#[tokio::test]
async fn g5_nenhum_mantem_o_comportamento_de_hoje() {
    let cenario = Cenario::novo("nenhum", false);
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[("g5_produto", G5_NONE)])));
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Posso ajudar com o prazo e o orçamento.")],
        Some(motor_jev(Arc::clone(&jev), so_g5())),
    );

    let gatilho = cenario.falar("qual é o prazo de entrega?");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    let pedido = &provedor.received()[0];
    assert!(!texto_do_prompt(pedido).contains(PRECO_CUPINS));
    let ferramentas = nomes_das_ferramentas(pedido);
    assert!(
        ferramentas.contains(&"buscar_produto".to_owned()),
        "{ferramentas:?}"
    );
}

/// Confiança abaixo do limiar não escolhe produto.
#[tokio::test]
async fn g5_confianca_baixa_mantem_o_comportamento_de_hoje() {
    let cenario = Cenario::novo("confianca", false);
    let baratas = cenario.baratas.to_string();
    let escolha = baratas.clone();
    let jev = Arc::new(JevFalso::novo(move |pedido: &poph_jev::Pedido| {
        let mut respostas = BTreeMap::new();
        for id in pedido.perguntas.keys() {
            let mut resposta = RespostaJev {
                choice: Some(escolha.clone()),
                confidence: 0.4,
                ..RespostaJev::default()
            };
            resposta.probabilities.insert(escolha.clone(), 0.4);
            resposta.probabilities.insert(G5_NONE.to_owned(), 0.4);
            respostas.insert(id.clone(), resposta);
        }
        Ok(BatchResponse {
            answers: respostas,
            tokens: 7,
            ..BatchResponse::default()
        })
    }));
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Vou verificar o valor.")],
        Some(motor_jev(Arc::clone(&jev), so_g5())),
    );

    let gatilho = cenario.falar("quanto custa a dedetização?");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    let pedido = &provedor.received()[0];
    assert!(!texto_do_prompt(pedido).contains(PRECO_BARATAS));
    assert!(nomes_das_ferramentas(pedido).contains(&"buscar_produto".to_owned()));
}

/// Jev fora do ar não bloqueia nada: o turno segue como sem o G5.
#[tokio::test]
async fn g5_jev_fora_do_ar_mantem_o_comportamento_de_hoje() {
    let cenario = Cenario::novo("fora", false);
    let jev = Arc::new(JevFalso::falhando(JevError::Status {
        code: 503,
        motivo: "Service Unavailable".to_owned(),
    }));
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Olá! Como posso ajudar?")],
        Some(motor_jev(Arc::clone(&jev), so_g5())),
    );

    let gatilho = cenario.falar("bom dia, quanto custa a dedetização?");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    let pedido = &provedor.received()[0];
    assert!(!texto_do_prompt(pedido).contains(PRECO_BARATAS));
    assert!(nomes_das_ferramentas(pedido).contains(&"buscar_produto".to_owned()));
}

/// Prazo estourado na pergunta do G5: o turno segue como sem o gate (não espera os 12 s do lote).
#[tokio::test]
async fn g5_jev_que_nao_responde_em_prazo_mantem_o_comportamento_de_hoje() {
    let cenario = Cenario::novo("prazo", false);
    // Atraso maior que o prazo do lote: `avaliar_agora` desiste e cai no fallback de cada gate.
    let jev =
        Arc::new(JevFalso::com_escolhas(escolhas(&[])).com_atraso(Duration::from_millis(200)));
    let avaliador: Arc<dyn AvaliadorGates> = Arc::new(
        MotorJev::novo(jev, so_g5(), "jev-latest").com_opcoes(OpcoesJev {
            tempo_maximo: Duration::from_millis(30),
            ..OpcoesJev::default()
        }),
    );
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Vou confirmar o valor com a equipe.")],
        Some(avaliador),
    );

    let gatilho = cenario.falar("quanto custa a dedetização?");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    let pedido = &provedor.received()[0];
    assert!(!texto_do_prompt(pedido).contains(PRECO_BARATAS));
    assert!(nomes_das_ferramentas(pedido).contains(&"buscar_produto".to_owned()));
    let jev = resumo
        .jev
        .expect("o resultado do G5 fica registrado mesmo no fallback");
    assert!(
        jev.falhou(),
        "o prazo estourado aparece no relato da chamada"
    );
    assert_eq!(jev.escolha_confiavel(IdGate::G5), None);
}

/// Busca sem candidatos não pergunta o G5 (nem gasta chamada).
#[tokio::test]
async fn g5_sem_candidatos_nao_pergunta() {
    let cenario = Cenario::novo("vazio", true);
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[])));
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Não encontrei esse produto no catálogo.")],
        Some(motor_jev(Arc::clone(&jev), so_g5())),
    );

    let gatilho = cenario.falar("vocês fazem dedetização?");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    assert!(
        jev.pedidos().is_empty(),
        "sem candidatos o G5 nem é perguntado: {:?}",
        jev.pedidos()
    );
    let pedido = &provedor.received()[0];
    assert!(nomes_das_ferramentas(pedido).contains(&"buscar_produto".to_owned()));
}

/// Gate desligado: nenhuma busca de produtos antes do modelo (o turno é o de antes da HR-33).
#[tokio::test]
async fn g5_desligado_nao_faz_busca_de_produto_nem_pergunta() {
    let cenario = Cenario::novo("desligado", false);
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[])));
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Olá! Como posso ajudar?")],
        Some(motor_jev(Arc::clone(&jev), JevGates::default())),
    );

    let gatilho = cenario.falar("quanto custa a dedetização?");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    assert!(
        resumo.jev.is_none()
            || resumo
                .jev
                .as_ref()
                .is_some_and(|jev| jev.decisoes.is_empty())
    );
    assert_eq!(
        cenario.catalogo_buscas(),
        0,
        "o gate desligado não faz pré-consulta"
    );
    let pedido = &provedor.received()[0];
    assert!(nomes_das_ferramentas(pedido).contains(&"buscar_produto".to_owned()));
}

/// Mesmo que o modelo invente a chamada, `buscar_produto` não é executada no turno do G5.
#[tokio::test]
async fn g5_ferramenta_inventada_pelo_modelo_nao_e_executada() {
    let cenario = Cenario::novo("inventada", false);
    let baratas = cenario.baratas.to_string();
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g5_produto",
        baratas.as_str(),
    )])));
    let (motor, provedor) = cenario.motor(
        vec![
            Script::Events(vec![
                LlmEvent::Tool(ToolCall {
                    id: "c1".to_owned(),
                    nome: "buscar_produto".to_owned(),
                    arguments: serde_json::json!({"termo": "baratas"}),
                }),
                LlmEvent::Fim(FinishReason::ToolCalls),
            ]),
            roteiro_texto("A dedetização de baratas custa R$ 180,00."),
        ],
        Some(motor_jev(Arc::clone(&jev), so_g5())),
    );

    let gatilho = cenario.falar("quanto custa para matar baratas?");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    assert_eq!(
        cenario.catalogo_buscas(),
        1,
        "só a pré-consulta do G5; a chamada inventada não busca de novo"
    );
    let pedidos = provedor.received();
    assert_eq!(pedidos.len(), 2);
    let resultado = texto_do_prompt(&pedidos[1]);
    assert!(
        resultado.contains("já foi consultado"),
        "o modelo recebe o erro explicando que o catálogo já foi consultado: {resultado}"
    );
}

/// G3 em `nada` dispensa o G5: mensagem social não paga a busca de produtos.
#[tokio::test]
async fn g3_nada_dispensa_o_g5() {
    let cenario = Cenario::novo("g3nada", false);
    let baratas = cenario.baratas.to_string();
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[
        ("g3_precisa", G3_NOTHING),
        ("g5_produto", baratas.as_str()),
    ])));
    let gates = JevGates {
        g3: true,
        g5: true,
        ..JevGates::default()
    };
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Oi! Tudo bem? Como posso ajudar?")],
        Some(motor_jev(Arc::clone(&jev), gates)),
    );

    let gatilho = cenario.falar("oi, bom dia");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    assert_eq!(cenario.catalogo_buscas(), 0);
    let ferramentas = nomes_das_ferramentas(&provedor.received()[0]);
    assert!(
        !ferramentas.contains(&"buscar_produto".to_owned()),
        "o G3 em nada já tira o catálogo do turno: {ferramentas:?}"
    );
}

/// O ajuste `apenas` liga só o gate pedido (usado na chamada fora do lote).
#[test]
fn ajuste_apenas_liga_so_o_gate_pedido() {
    let ajuste = GateOverrides::only(IdGate::G5);
    let efetivos = ajuste.with_base(JevGates {
        g1: true,
        g3: true,
        ..JevGates::default()
    });
    assert!(efetivos.g5);
    assert!(!efetivos.g1 && !efetivos.g2 && !efetivos.g3 && !efetivos.g4);
}

// ───────────────────────────── medição (rodadas de ferramenta) ─────────────────────────────
//
// O PRD pede "menos rodadas de ferramenta por turno comparado ao sem o gate". A rodada de
// ferramenta é uma chamada inteira ao modelo; o teste abaixo mede as duas condições com o mesmo
// texto de cliente e o mesmo catálogo falso:
//   * **sem o G5** — o modelo chama `buscar_produto` (comportamento de hoje): 2 chamadas ao modelo,
//     1 rodada de ferramenta;
//   * **com o G5 escolhendo** — o produto já está no prompt: 1 chamada ao modelo, 0 rodada.

#[tokio::test]
async fn medicao_rodadas_de_ferramenta_por_turno() {
    // 1. Sem o G5: o modelo precisa da ferramenta para falar de preço.
    let sem = Cenario::novo("medicao-sem", false);
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[])));
    let (motor_sem, _) = sem.motor(
        vec![
            Script::Events(vec![
                LlmEvent::Tool(ToolCall {
                    id: "c1".to_owned(),
                    nome: "buscar_produto".to_owned(),
                    arguments: serde_json::json!({"termo": "baratas"}),
                }),
                LlmEvent::Fim(FinishReason::ToolCalls),
            ]),
            roteiro_texto("A dedetização de baratas custa R$ 180,00."),
        ],
        Some(motor_jev(Arc::clone(&jev), JevGates::default())),
    );
    let gatilho = sem.falar("quanto custa para matar baratas?");
    let sem_g5 = sem.turno(&motor_sem, gatilho).await;
    assert_eq!(sem_g5.estado, poph_store::TurnState::Completed);
    assert_eq!(
        sem_g5.tempos.rodadas_ferramenta, 1,
        "sem o G5 há uma rodada"
    );

    // 2. Com o G5: o produto escolhido já está no prompt, sem rodada.
    let com = Cenario::novo("medicao-com", false);
    let baratas = com.baratas.to_string();
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g5_produto",
        baratas.as_str(),
    )])));
    let (motor_com, _) = com.motor(
        vec![roteiro_texto("A dedetização de baratas custa R$ 180,00.")],
        Some(motor_jev(Arc::clone(&jev), so_g5())),
    );
    let gatilho = com.falar("quanto custa para matar baratas?");
    let com_g5 = com.turno(&motor_com, gatilho).await;
    assert_eq!(com_g5.estado, poph_store::TurnState::Completed);
    assert_eq!(
        com_g5.tempos.rodadas_ferramenta, 0,
        "com o G5 não há rodada"
    );

    println!(
        "rodadas de ferramenta por turno: sem o G5 = {} · com o G5 = {}",
        sem_g5.tempos.rodadas_ferramenta, com_g5.tempos.rodadas_ferramenta
    );
}

/// Padrão de instalação (decisão do mantenedor, 20/11/2026): agente sem configuração de gates e Jev
/// com a configuração padrão. O G5 escolhe o produto e a ferramenta sai do turno.
#[tokio::test]
async fn padrao_de_instalacao_liga_o_g5_sem_configurar_o_agente() {
    let cenario = Cenario::novo("padrao-g5", false);
    let baratas = cenario.baratas.to_string();
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[
        ("g3_precisa", poph_jev::G3_PRODUCT),
        ("g5_produto", baratas.as_str()),
    ])));
    let gates_padrao = poph_types::JevConfig::default().gates;
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto(
            "A dedetização de baratas custa R$ 180,00 por visita.",
        )],
        Some(motor_jev(Arc::clone(&jev), gates_padrao)),
    );

    let gatilho = cenario.falar("quanto custa para matar as baratas daqui de casa?");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    assert_eq!(
        resumo.tempos.rodadas_ferramenta, 0,
        "sem rodada de ferramenta"
    );
    let pedido = &provedor.received()[0];
    assert!(
        texto_do_prompt(pedido).contains(NOME_BARATAS),
        "o produto escolhido entra no prompt"
    );
    assert!(
        !nomes_das_ferramentas(pedido).contains(&"buscar_produto".to_owned()),
        "com o G5 escolhido, buscar_produto sai do turno"
    );
}
