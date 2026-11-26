//! Efeito do G3 no turno (HR-31): `nada` dispensa base e catálogo.
//!
//! O que se prova aqui:
//! 1. com o gate ligado e a decisão `nada`, os trechos da busca em andamento são descartados, a
//!    busca é abandonada sem ser aguardada, as ferramentas `consultar_base` e `buscar_produto` não
//!    vão no pedido ao modelo e não são executadas nem se o modelo inventar a chamada;
//! 2. com a decisão `base`, tudo continua como antes (trechos no prompt, ferramentas oferecidas);
//! 3. gate desligado, abstenção, confiança baixa e Jev fora do ar caem no comportamento de hoje —
//!    o gate nunca bloqueia a resposta.
//!
//! Provedor falso, busca falsa e pausa que não dorme: nenhum teste toca a rede.
//!
//! A medição de tokens e latência fica no fim do arquivo, num teste `#[ignore]`:
//! `cargo test -p poph-agent --test g3_base medicao -- --ignored --nocapture`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

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
    BatchResponse, G3_BASE, G3_NADA, IdGate, JevError, JevFalso, JevResponse as RespostaJev,
    MotorJev,
};
use poph_store::{Direction, NewAgent, NewConversation, NewMessage, NewOrg, Role, Store};
use poph_types::{
    AgentId, ConversationId, JevGates, MessageId, OrgId, StructuredInstructions, ToolConfig,
};

/// Texto do trecho que a busca falsa devolve. Se ele aparecer no prompt, a base foi usada.
const TEXTO_DO_TRECHO: &str = "Prazo de entrega: até 8 dias úteis após a confirmação.";

struct SemEspera;

impl Pausa for SemEspera {
    fn esperar(&self, _duracao: Duration) -> FuturoPausa {
        Box::pin(async {})
    }
}

/// Busca falsa: conta as buscas, demora o que for pedido e devolve um trecho fixo.
struct BuscaContada {
    buscas: Arc<AtomicU64>,
    demora: Duration,
    trechos: Vec<TrechoRecuperado>,
}

impl BuscaContada {
    fn nova() -> Arc<Self> {
        Self::com_demora(Duration::ZERO)
    }

    fn com_demora(demora: Duration) -> Arc<Self> {
        Arc::new(Self {
            buscas: Arc::new(AtomicU64::new(0)),
            demora,
            trechos: vec![TrechoRecuperado {
                item: poph_types::KnowledgeItemId::generate(),
                titulo: "Prazo de entrega".to_owned(),
                grupo: "Políticas da loja".to_owned(),
                texto: TEXTO_DO_TRECHO.to_owned(),
                similaridade: 0.82,
                relevancia: 0.5,
            }],
        })
    }

    fn contador(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.buscas)
    }
}

impl BuscadorBase for BuscaContada {
    fn buscar(
        &self,
        _org: OrgId,
        _agente: AgentId,
        _consulta: String,
        _parametros: ParametrosBusca,
    ) -> FuturoBusca<'_> {
        self.buscas.fetch_add(1, Ordering::SeqCst);
        let trechos = self.trechos.clone();
        let demora = self.demora;
        Box::pin(async move {
            if !demora.is_zero() {
                tokio::time::sleep(demora).await;
            }
            Ok(trechos)
        })
    }
}

/// Catálogo falso: existe para a ferramenta `buscar_produto` entrar na lista de oferecidas.
struct CatalogoVazio;

impl BuscadorProdutos for CatalogoVazio {
    fn buscar(
        &self,
        _org: OrgId,
        _agente: AgentId,
        _termo: String,
        _categoria: Option<String>,
        _parametros: ParametrosProdutos,
    ) -> FuturoProdutos<'_> {
        let vazio: Vec<ProdutoEncontrado> = Vec::new();
        Box::pin(async move { Ok(vazio) })
    }
}

/// Banco, org, agente (com base e catálogo habilitados) e conversa de verdade.
struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    conversa: ConversationId,
    busca: Arc<dyn BuscadorBase>,
    buscas: Arc<AtomicU64>,
    metricas: MetricasTurno,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let busca = BuscaContada::nova();
        let contador = busca.contador();
        let base: Arc<dyn BuscadorBase> = busca;
        Self::com_busca(rotulo, base, contador)
    }

    fn com_busca(rotulo: &str, busca: Arc<dyn BuscadorBase>, buscas: Arc<AtomicU64>) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-hr31-{rotulo}-{}-{}",
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
        Self {
            pasta,
            store,
            org,
            agente,
            conversa,
            busca,
            buscas,
            metricas: MetricasTurno::proprio(),
        }
    }

    fn buscas(&self) -> u64 {
        self.buscas.load(Ordering::SeqCst)
    }

    /// Nova conversa no mesmo agente (a medição usa uma por mensagem, sem histórico acumulado).
    fn nova_conversa(&self) -> ConversationId {
        self.store
            .conversations()
            .create(
                self.org,
                NewConversation {
                    agent_id: self.agente,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("abre a conversa")
            .id
    }

    fn falar(&self, texto: &str) -> MessageId {
        self.falar_em(self.conversa, texto)
    }

    fn falar_em(&self, conversa: ConversationId, texto: &str) -> MessageId {
        self.store
            .messages()
            .write(
                self.org,
                NewMessage {
                    conversation_id: conversa,
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
        jev: Option<Arc<MotorJev>>,
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
        let catalogo: Arc<dyn BuscadorProdutos> = Arc::new(CatalogoVazio);
        let mut motor = Motor::novo(
            Arc::clone(&self.store),
            config,
            limitador,
            fake.clone(),
            OpcoesTurno::default(),
            self.metricas.clone(),
        )
        .com_pausa(Arc::new(SemEspera))
        .com_busca(Arc::clone(&self.busca));
        let registro =
            ConsultarBase::registro(Arc::clone(motor.config_quente()), Arc::clone(&self.busca))
                .com(BuscarProduto::nova(
                    Arc::clone(motor.config_quente()),
                    catalogo,
                ));
        motor = motor.com_ferramentas(Arc::new(registro));
        if let Some(jev) = jev {
            motor = motor.com_jev(jev);
        }
        (motor, fake)
    }

    async fn turno(&self, motor: &Motor, gatilho: MessageId) -> ResumoTurno {
        self.turno_em(motor, self.conversa, gatilho).await
    }

    async fn turno_em(
        &self,
        motor: &Motor,
        conversa: ConversationId,
        gatilho: MessageId,
    ) -> ResumoTurno {
        motor
            .executar(
                Entrada::nova(self.org, conversa, gatilho),
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

fn so_g3() -> JevGates {
    JevGates {
        g3: true,
        ..JevGates::default()
    }
}

fn motor_jev(jev: Arc<JevFalso>) -> Arc<MotorJev> {
    Arc::new(MotorJev::novo(jev, so_g3(), "jev-latest"))
}

fn escolhas(itens: &[(&str, &str)]) -> BTreeMap<String, String> {
    itens
        .iter()
        .map(|(id, escolha)| ((*id).to_owned(), (*escolha).to_owned()))
        .collect()
}

/// Nomes das ferramentas enviadas no pedido ao modelo.
fn nomes_das_ferramentas(pedido: &poph_core::llm::Request) -> Vec<String> {
    pedido
        .tools
        .iter()
        .map(|ferramenta| ferramenta.nome.clone())
        .collect()
}

/// Todo o texto do prompt, para procurar o trecho da base.
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

/// Com o G3 em `nada`: nada de trechos no prompt e nada de base/catálogo entre as ferramentas.
#[tokio::test]
async fn g3_nada_descarta_a_busca_e_nao_oferece_base_nem_catalogo() {
    let cenario = Cenario::novo("nada");
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[("g3_precisa", G3_NADA)])));
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Oi! Tudo bem? Como posso ajudar?")],
        Some(motor_jev(Arc::clone(&jev))),
    );

    let gatilho = cenario.falar("oi, bom dia");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    assert_eq!(resumo.texto, "Oi! Tudo bem? Como posso ajudar?");
    assert_eq!(
        resumo.jev.as_ref().and_then(|jev| jev.escolha(IdGate::G3)),
        Some(G3_NADA)
    );

    // O resultado da busca não entra no prompt nem no registro.
    let pedidos = provedor.received();
    assert_eq!(pedidos.len(), 1, "uma chamada ao modelo");
    let prompt = texto_do_prompt(&pedidos[0]);
    assert!(
        !prompt.contains(TEXTO_DO_TRECHO),
        "o trecho da base não pode entrar no prompt quando o G3 diz nada"
    );
    let ferramentas = nomes_das_ferramentas(&pedidos[0]);
    assert!(
        !ferramentas.contains(&"consultar_base".to_owned()),
        "consultar_base não deve ser oferecida: {ferramentas:?}"
    );
    assert!(
        !ferramentas.contains(&"buscar_produto".to_owned()),
        "buscar_produto não deve ser oferecida: {ferramentas:?}"
    );
    let turnos = cenario
        .store
        .turns()
        .list(
            cenario.org,
            cenario.conversa,
            poph_store::Page::with_limit(5),
        )
        .expect("lista os turnos");
    assert_eq!(
        turnos[0].passages_json, "[]",
        "o registro não guarda trecho quando o G3 diz nada"
    );
    assert_eq!(
        cenario.metricas.buscas_puladas_g3.value(),
        1,
        "a métrica de buscas dispensadas pelo G3 conta o turno"
    );
}

/// Mesmo que o modelo invente a chamada, `consultar_base` não é executada quando o G3 diz `nada`.
#[tokio::test]
async fn g3_nada_nao_executa_ferramenta_inventada_pelo_modelo() {
    let cenario = Cenario::novo("inventada");
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[("g3_precisa", G3_NADA)])));
    let (motor, _) = cenario.motor(
        vec![
            Script::Events(vec![
                LlmEvent::Tool(ToolCall {
                    id: "c1".to_owned(),
                    nome: "consultar_base".to_owned(),
                    arguments: serde_json::json!({"consulta": "prazo"}),
                }),
                LlmEvent::Fim(FinishReason::ToolCalls),
            ]),
            roteiro_texto("Tudo certo por aqui!"),
        ],
        Some(motor_jev(Arc::clone(&jev))),
    );

    let gatilho = cenario.falar("ok, obrigado");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    assert_eq!(
        cenario.buscas(),
        1,
        "só a busca prévia do turno rodou; a ferramenta inventada não pode consultar a base"
    );
}

/// Com o G3 em `base` (ou `ambos`), o turno continua consultando a base e oferecendo as ferramentas.
#[tokio::test]
async fn g3_base_mantem_a_busca_e_as_ferramentas() {
    let cenario = Cenario::novo("base");
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[("g3_precisa", G3_BASE)])));
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("O prazo é de até 8 dias úteis.")],
        Some(motor_jev(Arc::clone(&jev))),
    );

    let gatilho = cenario.falar("qual é o prazo de entrega para o interior?");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    let pedidos = provedor.received();
    assert_eq!(pedidos.len(), 1);
    assert!(
        texto_do_prompt(&pedidos[0]).contains(TEXTO_DO_TRECHO),
        "com o G3 em base, o trecho recuperado entra no prompt"
    );
    let ferramentas = nomes_das_ferramentas(&pedidos[0]);
    assert!(
        ferramentas.contains(&"consultar_base".to_owned()),
        "{ferramentas:?}"
    );
    assert!(
        ferramentas.contains(&"buscar_produto".to_owned()),
        "{ferramentas:?}"
    );
}

/// Gate desligado é o comportamento anterior à HR-31: sem Jev, com trechos e ferramentas.
#[tokio::test]
async fn g3_desligado_nao_muda_o_comportamento() {
    let cenario = Cenario::novo("desligado");
    let (motor, provedor) = cenario.motor(vec![roteiro_texto("Olá! Como posso ajudar?")], None);

    let gatilho = cenario.falar("oi");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    assert!(
        resumo.jev.is_none(),
        "sem gate ligado o turno não consulta o Jev"
    );
    let pedidos = provedor.received();
    assert!(texto_do_prompt(&pedidos[0]).contains(TEXTO_DO_TRECHO));
    let ferramentas = nomes_das_ferramentas(&pedidos[0]);
    assert!(
        ferramentas.contains(&"consultar_base".to_owned()),
        "{ferramentas:?}"
    );
    assert!(
        ferramentas.contains(&"buscar_produto".to_owned()),
        "{ferramentas:?}"
    );
}

/// Abstenção (o Jev não respondeu a pergunta) cai no comportamento de hoje: oferece tudo.
#[tokio::test]
async fn g3_sem_resposta_cai_no_comportamento_de_hoje() {
    let cenario = Cenario::novo("abstencao");
    // Mapa vazio: o Jev falso responde o lote sem `g3_precisa`, e o gate cai no fallback `tudo`.
    let jev = Arc::new(JevFalso::com_escolhas(BTreeMap::new()));
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Olá! Como posso ajudar?")],
        Some(motor_jev(Arc::clone(&jev))),
    );

    let gatilho = cenario.falar("oi");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    let pedidos = provedor.received();
    assert!(texto_do_prompt(&pedidos[0]).contains(TEXTO_DO_TRECHO));
    let ferramentas = nomes_das_ferramentas(&pedidos[0]);
    assert!(
        ferramentas.contains(&"consultar_base".to_owned()),
        "{ferramentas:?}"
    );
}

/// Confiança abaixo do limiar também é abstenção: nada é descartado.
#[tokio::test]
async fn g3_confianca_baixa_cai_no_comportamento_de_hoje() {
    let cenario = Cenario::novo("confianca");
    let jev = Arc::new(JevFalso::novo(|pedido: &poph_jev::Pedido| {
        let mut respostas = BTreeMap::new();
        for id in pedido.perguntas.keys() {
            let mut resposta = RespostaJev {
                choice: Some(G3_NADA.to_owned()),
                confidence: 0.4,
                ..RespostaJev::default()
            };
            resposta.probabilities.insert(G3_NADA.to_owned(), 0.4);
            resposta.probabilities.insert(G3_BASE.to_owned(), 0.4);
            respostas.insert(id.clone(), resposta);
        }
        Ok(BatchResponse {
            answers: respostas,
            tokens: 7,
            ..BatchResponse::default()
        })
    }));
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Olá! Como posso ajudar?")],
        Some(motor_jev(Arc::clone(&jev))),
    );

    let gatilho = cenario.falar("oi");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    let pedidos = provedor.received();
    assert!(
        texto_do_prompt(&pedidos[0]).contains(TEXTO_DO_TRECHO),
        "confiança baixa não pode descartar a base"
    );
}

/// Jev fora do ar não bloqueia nada: o turno responde com base e ferramentas, como sem o Jev.
#[tokio::test]
async fn g3_jev_fora_do_ar_cai_no_comportamento_de_hoje() {
    let cenario = Cenario::novo("fora");
    let jev = Arc::new(JevFalso::falhando(JevError::Status {
        code: 503,
        motivo: "Service Unavailable".to_owned(),
    }));
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Olá! Como posso ajudar?")],
        Some(motor_jev(Arc::clone(&jev))),
    );

    let gatilho = cenario.falar("oi");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    let pedidos = provedor.received();
    assert!(texto_do_prompt(&pedidos[0]).contains(TEXTO_DO_TRECHO));
    let ferramentas = nomes_das_ferramentas(&pedidos[0]);
    assert!(
        ferramentas.contains(&"consultar_base".to_owned()),
        "{ferramentas:?}"
    );
}

/// A busca em andamento é **abandonada** quando o G3 diz `nada`: o turno não paga o tempo dela.
///
/// Busca falsa de 300 ms e Jev instantâneo: com `nada` o turno termina antes dos 300 ms; com `base`
/// ele espera a busca, como hoje.
#[tokio::test]
async fn g3_nada_abandona_a_busca_em_andamento() {
    let cenario = Cenario::com_busca(
        "abandona",
        BuscaContada::com_demora(Duration::from_millis(300)) as Arc<dyn BuscadorBase>,
        Arc::new(AtomicU64::new(0)),
    );

    let jev_nada = Arc::new(JevFalso::com_escolhas(escolhas(&[("g3_precisa", G3_NADA)])));
    let (motor, _) = cenario.motor(
        vec![roteiro_texto("Oi! Como posso ajudar?")],
        Some(motor_jev(Arc::clone(&jev_nada))),
    );
    let gatilho = cenario.falar("oi, bom dia");
    let inicio = Instant::now();
    let resumo = cenario.turno(&motor, gatilho).await;
    let com_nada = inicio.elapsed();
    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    assert!(
        com_nada < Duration::from_millis(250),
        "com o G3 em nada o turno não pode esperar a busca: levou {com_nada:?}"
    );

    let cenario_base = Cenario::com_busca(
        "espera",
        BuscaContada::com_demora(Duration::from_millis(300)) as Arc<dyn BuscadorBase>,
        Arc::new(AtomicU64::new(0)),
    );
    let jev_base = Arc::new(JevFalso::com_escolhas(escolhas(&[("g3_precisa", G3_BASE)])));
    let (motor, _) = cenario_base.motor(
        vec![roteiro_texto("O prazo é de até 8 dias úteis.")],
        Some(motor_jev(Arc::clone(&jev_base))),
    );
    let gatilho = cenario_base.falar("qual é o prazo de entrega?");
    let inicio = Instant::now();
    let resumo = cenario_base.turno(&motor, gatilho).await;
    let com_base = inicio.elapsed();
    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    assert!(
        com_base >= Duration::from_millis(300),
        "com o G3 em base a busca é aguardada: levou {com_base:?}"
    );
}

/// G1 e G3 ligados ao mesmo tempo: se o G1 transfere e o G3 diz `nada`, a busca é abandonada e a
/// transferência segue igual — o caminho da transferência não depende dos trechos.
#[tokio::test]
async fn g3_nada_com_transferencia_nao_quebra_o_turno() {
    let cenario = Cenario::com_busca(
        "transferencia",
        BuscaContada::com_demora(Duration::from_millis(300)) as Arc<dyn BuscadorBase>,
        Arc::new(AtomicU64::new(0)),
    );
    // Handoff ligado: sem isso o G1 nem se aplica.
    let atual = cenario
        .store
        .agents()
        .get(cenario.org, cenario.agente)
        .expect("lê o agente");
    cenario
        .store
        .agents()
        .update(
            cenario.org,
            cenario.agente,
            atual.version,
            poph_store::AgentChange {
                handoff_active: Some(true),
                ..poph_store::AgentChange::default()
            },
        )
        .expect("liga o handoff");

    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[
        ("g1_transferir", poph_jev::G1_TRANSFERIR),
        ("g3_precisa", G3_NADA),
    ])));
    let gates = JevGates {
        g1: true,
        g3: true,
        ..JevGates::default()
    };
    let motor_jev = Arc::new(MotorJev::novo(jev, gates, "jev-latest"));
    let (motor, _) = cenario.motor(
        vec![roteiro_texto("Vou te passar para um atendente.")],
        Some(motor_jev),
    );

    let gatilho = cenario.falar("quero falar com uma pessoa de verdade");
    let inicio = Instant::now();
    let resumo = cenario.turno(&motor, gatilho).await;
    let decorrido = inicio.elapsed();

    // A transferência não espera os 300 ms da busca.
    assert!(
        decorrido < Duration::from_millis(250),
        "a busca não devia ser aguardada: levou {decorrido:?}"
    );
    assert_eq!(
        resumo.jev.as_ref().and_then(|jev| jev.escolha(IdGate::G1)),
        Some(poph_jev::G1_TRANSFERIR)
    );
    assert_eq!(
        cenario.metricas.buscas_puladas_g3.value(),
        1,
        "o G3 contou a busca dispensada"
    );
}

// ───────────────────────────── Medição da redução (HR-31) ─────────────────────────────
//
// Roda as mensagens que o conjunto `tests/jev/g3.jsonl` rotula como `nada` em duas condições, com
// a mesma base (três trechos de ~1000 caracteres, como o trecho do RAG no PRD):
//   * **sem o G3** — comportamento de hoje: trechos no prompt e base/catálogo oferecidos;
//   * **com o G3 em `nada`** — sem trechos e sem as duas ferramentas.
// Imprime o tamanho do prompt, a diferença e a duração do turno em cada condição.
//
// `cargo test -p poph-agent --test g3_base medicao -- --ignored --nocapture`

/// Busca com o material que o RAG injeta de verdade: três trechos de ~1000 caracteres.
struct BuscaRealista {
    trechos: Vec<TrechoRecuperado>,
}

impl BuscadorBase for BuscaRealista {
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

fn trechos_realistas() -> Vec<TrechoRecuperado> {
    let texto =
        "Política da loja: o prazo de entrega para o interior é de até 8 dias úteis após a \
confirmação do pagamento, e o frete é grátis acima de R$ 199,00. "
            .repeat(11);
    (0..3)
        .map(|indice| TrechoRecuperado {
            item: poph_types::KnowledgeItemId::generate(),
            titulo: format!("Item {} de política", indice + 1),
            grupo: "Políticas da loja".to_owned(),
            texto: texto.clone(),
            similaridade: 0.8 - (indice as f32) * 0.05,
            relevancia: 0.5,
        })
        .collect()
}

#[tokio::test]
#[ignore = "medição da HR-31: rodar com --ignored --nocapture"]
async fn medicao_de_tokens_e_latencia_do_g3() {
    let arquivo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/jev/g3.jsonl");
    let texto = std::fs::read_to_string(&arquivo).expect("lê o conjunto g3.jsonl");
    let mut mensagens: Vec<String> = Vec::new();
    for linha in texto.lines() {
        let linha = linha.trim();
        if linha.is_empty() || linha.starts_with('#') {
            continue;
        }
        let caso: serde_json::Value = serde_json::from_str(linha).expect("caso é JSON");
        if caso["esperado"] == "nada" {
            let ultima = caso["estado"]["mensagens"]
                .as_array()
                .and_then(|itens| itens.last())
                .and_then(|item| item["texto"].as_str())
                .unwrap_or_default()
                .to_owned();
            mensagens.push(ultima);
        }
    }

    let trechos = trechos_realistas();
    let tamanho_da_base: usize = trechos
        .iter()
        .map(|trecho| trecho.texto.chars().count())
        .sum();

    let mut total_sem = 0usize;
    let mut total_com = 0usize;
    let mut duracao_sem = 0u128;
    let mut duracao_com = 0u128;
    let mut ferramentas_sem = 0usize;
    let mut ferramentas_com = 0usize;

    for mensagem in &mensagens {
        // Condição A: sem o G3 (comportamento de hoje).
        let cenario = Cenario::com_busca(
            "medicao-sem",
            Arc::new(BuscaRealista {
                trechos: trechos.clone(),
            }),
            Arc::new(AtomicU64::new(0)),
        );
        let (motor, provedor) = cenario.motor(vec![roteiro_texto("Oi! Como posso ajudar?")], None);
        let conversa = cenario.nova_conversa();
        let gatilho = cenario.falar_em(conversa, mensagem);
        let inicio = Instant::now();
        let resumo = cenario.turno_em(&motor, conversa, gatilho).await;
        duracao_sem += inicio.elapsed().as_micros();
        assert_eq!(resumo.estado, poph_store::TurnState::Completed);
        let pedido = provedor.received().remove(0);
        total_sem += texto_do_prompt(&pedido).chars().count();
        ferramentas_sem += pedido.tools.len();

        // Condição B: com o G3 ligado e a decisão `nada` (a que o Jev real deu para estas mensagens).
        let cenario = Cenario::com_busca(
            "medicao-com",
            Arc::new(BuscaRealista {
                trechos: trechos.clone(),
            }),
            Arc::new(AtomicU64::new(0)),
        );
        let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[("g3_precisa", G3_NADA)])));
        let (motor, provedor) = cenario.motor(
            vec![roteiro_texto("Oi! Como posso ajudar?")],
            Some(motor_jev(Arc::clone(&jev))),
        );
        let conversa = cenario.nova_conversa();
        let gatilho = cenario.falar_em(conversa, mensagem);
        let inicio = Instant::now();
        let resumo = cenario.turno_em(&motor, conversa, gatilho).await;
        duracao_com += inicio.elapsed().as_micros();
        assert_eq!(resumo.estado, poph_store::TurnState::Completed);
        let pedido = provedor.received().remove(0);
        total_com += texto_do_prompt(&pedido).chars().count();
        ferramentas_com += pedido.tools.len();
    }

    let casos = mensagens.len();
    let media_sem = total_sem / casos;
    let media_com = total_com / casos;
    let economia = media_sem.saturating_sub(media_com);
    println!("\n=== Medição G3 (HR-31) — {casos} mensagens rotuladas `nada` ===");
    println!(
        "base de exemplo: {} trechos, {tamanho_da_base} caracteres no total",
        trechos.len()
    );
    println!("prompt médio SEM o G3: {media_sem} caracteres");
    println!("prompt médio COM o G3 em `nada`: {media_com} caracteres");
    println!(
        "economia média: {economia} caracteres ({:.1}% do prompt) ≈ {} tokens por mensagem \
         (estimativa caracteres/4; o tokenizador do provedor não está no repositório)",
        (economia as f64) * 100.0 / (media_sem as f64),
        economia / 4
    );
    println!(
        "ferramentas oferecidas por turno: {:.1} sem o G3, {:.1} com o G3",
        ferramentas_sem as f64 / casos as f64,
        ferramentas_com as f64 / casos as f64
    );
    println!(
        "duração média do turno (parte local, provedor falso): {:.2} ms sem o G3, {:.2} ms com o G3",
        duracao_sem as f64 / casos as f64 / 1000.0,
        duracao_com as f64 / casos as f64 / 1000.0
    );
    assert!(economia > 0, "com o G3 em nada o prompt tem de encolher");
}

/// Padrão de instalação (decisão do mantenedor, 20/11/2026): agente sem configuração de gates e
/// Jev com a configuração padrão. O G3 pergunta e age: `nada` dispensa base e catálogo.
#[tokio::test]
async fn padrao_de_instalacao_liga_o_g3_sem_configurar_o_agente() {
    let cenario = Cenario::novo("padrao-g3");
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[("g3_precisa", G3_NADA)])));
    let gates_padrao = poph_types::JevConfig::default().gates;
    let avaliador = Arc::new(MotorJev::novo(jev, gates_padrao, "jev-latest"));
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Oi! Como posso ajudar?")],
        Some(avaliador),
    );

    let gatilho = cenario.falar("oi, bom dia");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    assert_eq!(
        resumo.jev.as_ref().and_then(|jev| jev.escolha(IdGate::G3)),
        Some(G3_NADA),
        "sem configuração no agente, o G3 do padrão de instalação precisa perguntar"
    );
    let pedidos = provedor.received();
    assert!(
        !texto_do_prompt(&pedidos[0]).contains(TEXTO_DO_TRECHO),
        "com `nada`, a base não entra no prompt"
    );
    assert!(
        !nomes_das_ferramentas(&pedidos[0]).contains(&"consultar_base".to_owned()),
        "com `nada`, consultar_base não é oferecida"
    );
}

/// Agente com `{"g3": false}` desliga o G3 mesmo com o padrão ligado: volta ao comportamento de
/// hoje (base no prompt, ferramenta oferecida) e o Jev não é perguntado sobre o G3.
#[tokio::test]
async fn agente_que_desliga_o_g3_volta_ao_comportamento_de_hoje() {
    let cenario = Cenario::novo("agente-g3-off");
    let atual = cenario
        .store
        .agents()
        .get(cenario.org, cenario.agente)
        .expect("lê o agente");
    cenario
        .store
        .agents()
        .update(
            cenario.org,
            cenario.agente,
            atual.version,
            poph_store::AgentChange {
                jev_json: Some(r#"{"g3": false}"#.to_owned()),
                ..poph_store::AgentChange::default()
            },
        )
        .expect("desliga o G3 no agente");

    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[("g3_precisa", G3_NADA)])));
    let gates_padrao = poph_types::JevConfig::default().gates;
    let avaliador = Arc::new(MotorJev::novo(jev, gates_padrao, "jev-latest"));
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Olá! Como posso ajudar?")],
        Some(avaliador),
    );

    let gatilho = cenario.falar("oi");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    assert!(
        resumo
            .jev
            .as_ref()
            .and_then(|jev| jev.escolha(IdGate::G3))
            .is_none(),
        "com o G3 desligado no agente, o Jev não decide o G3"
    );
    let pedidos = provedor.received();
    assert!(texto_do_prompt(&pedidos[0]).contains(TEXTO_DO_TRECHO));
    assert!(nomes_das_ferramentas(&pedidos[0]).contains(&"consultar_base".to_owned()));
}

/// `{"ativo": false}` no agente desliga todos os gates dele, inclusive o G3 do padrão.
#[tokio::test]
async fn agente_com_ativo_false_nao_consulta_o_jev_nem_o_g3() {
    let cenario = Cenario::novo("agente-inativo-g3");
    let atual = cenario
        .store
        .agents()
        .get(cenario.org, cenario.agente)
        .expect("lê o agente");
    cenario
        .store
        .agents()
        .update(
            cenario.org,
            cenario.agente,
            atual.version,
            poph_store::AgentChange {
                jev_json: Some(r#"{"ativo": false}"#.to_owned()),
                ..poph_store::AgentChange::default()
            },
        )
        .expect("desliga o Jev do agente");

    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[("g3_precisa", G3_NADA)])));
    let gates_padrao = poph_types::JevConfig::default().gates;
    let avaliador = Arc::new(MotorJev::novo(jev, gates_padrao, "jev-latest"));
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Olá! Como posso ajudar?")],
        Some(avaliador),
    );

    let gatilho = cenario.falar("oi");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert!(
        resumo
            .jev
            .as_ref()
            .is_none_or(|jev| jev.decisoes.is_empty()),
        "agente com ativo=false não recebe decisão nenhuma do Jev"
    );
    let pedidos = provedor.received();
    assert!(texto_do_prompt(&pedidos[0]).contains(TEXTO_DO_TRECHO));
    assert!(nomes_das_ferramentas(&pedidos[0]).contains(&"consultar_base".to_owned()));
}
