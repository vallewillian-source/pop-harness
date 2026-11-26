//! Testes do motor do turno (HR-13).
//!
//! Cobrem os sete grupos que a tarefa exige:
//! 1. cinco mensagens: stream, mensagens e turno gravados, tempos por etapa preenchidos;
//! 2. prefixo do prompt idêntico byte a byte em três turnos consecutivos;
//! 3. duas orgs rodando juntas não se misturam (prompt, contexto, resposta, registro);
//! 4. janela de agrupamento: duas seguidas viram um turno, depois do teto vira outro;
//! 5. ferramenta não habilitada não é oferecida e o laço para no limite de rodadas;
//! 6. cancelamento no meio do stream encerra o turno e conclui a entrada;
//! 7. erro do provedor no meio conclui a entrada, grava o turno e não deixa item em `sending`.
//!
//! Mais o caminho completo da integração: `inbox` → ator → turno → `outbox` → envio.
//!
//! O provedor é o [`ProvedorFake`] e a espera é uma [`PausaFalsa`]: nenhum teste dorme de verdade.

use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poph_agent::atores::{Atores, OpcoesAtores};
use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::entrada_saida::{
    Fluxo, FuturoEnvio, MensagemRecebida, OpcoesFluxo, TransporteSaida,
};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasFluxo, MetricasMemoria, MetricasTurno};
use poph_agent::turno::{
    FuturoPausa, NoopObserver, Pausa, RegistroFerramentas, TurnEngine, TurnEvent, TurnInput,
    TurnObserver, TurnOptions, TurnProcessor,
};
use poph_core::llm::{
    Cancellation, FakeProvider, FinishReason, LlmError, LlmEvent, LlmProvider, Script, ToolCall,
    Usage,
};
use poph_store::{
    Direction, NewAgent, NewConversation, NewMessage, NewOrg, OutboxItem, OutboxState, Page, Role,
    Store, TurnState,
};
use poph_types::{AgentId, ConversationId, IntegrationId, MessageId, OrgId, ToolConfig};

// ───────────────────────────── Infraestrutura ─────────────────────────────

/// O que fazer enquanto a janela "passa": é como o teste simula a mensagem que chega durante ela.
type AoEsperar = Arc<dyn Fn() + Send + Sync>;

/// Espera que não dorme: registra o que foi pedido, roda o gatilho e volta na hora.
#[derive(Default, Clone)]
struct PausaFalsa {
    pedidos: Arc<Mutex<Vec<Duration>>>,
    ao_esperar: Option<AoEsperar>,
}

impl PausaFalsa {
    /// Pausa que, ao ser chamada a primeira vez, roda a ação (ex.: grava a segunda mensagem).
    fn com_acao(acao: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            pedidos: Arc::default(),
            ao_esperar: Some(Arc::new(acao)),
        }
    }

    fn pedidos(&self) -> Vec<Duration> {
        self.pedidos
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .clone()
    }
}

impl Pausa for PausaFalsa {
    fn esperar(&self, duracao: Duration) -> FuturoPausa {
        self.pedidos
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .push(duracao);
        if let Some(acao) = &self.ao_esperar
            && !duracao.is_zero()
        {
            acao();
        }
        Box::pin(async {})
    }
}

/// Observador que guarda os eventos do turno.
#[derive(Default)]
struct Coletor {
    eventos: Vec<TurnEvent>,
}

impl Coletor {
    fn texto(&self) -> String {
        let mut texto = String::new();
        for evento in &self.eventos {
            if let TurnEvent::Texto(pedaco) = evento {
                texto.push_str(pedaco);
            }
        }
        texto
    }
}

impl TurnObserver for Coletor {
    fn evento(&mut self, evento: TurnEvent) {
        self.eventos.push(evento);
    }
}

/// Um banco de verdade em pasta temporária, com org, agente, instruções e conversa.
struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    conversa: ConversationId,
    agente: AgentId,
    relogio: Arc<AtomicI64>,
    integracao: IntegrationId,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-hr13-{rotulo}-{}-{}",
            std::process::id(),
            AgentId::generate()
        ));
        let _ = std::fs::remove_dir_all(&pasta);
        std::fs::create_dir_all(&pasta).expect("cria a pasta do teste");
        let relogio = Arc::new(AtomicI64::new(1_795_130_160_000));
        let store = Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db"))
            .expect("abre o banco")
            .with_clock({
                let relogio = Arc::clone(&relogio);
                Arc::new(move || relogio.load(Ordering::SeqCst))
            });
        let store = Arc::new(store);
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
                    modelo: "gemini-3.8-flash-tiered".to_owned(),
                    grouping_window_ms: 2_000,
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente");
        store
            .instructions()
            .save(
                org,
                agente.id,
                0,
                "{}",
                "# Bia\n\nVocê é a atendente da FixProLar.",
                true,
            )
            .expect("grava as instruções");
        let conversa = Self::abrir_conversa(&store, org, agente.id, "cli");
        Self {
            pasta,
            store,
            org,
            conversa,
            agente: agente.id,
            relogio,
            integracao: IntegrationId::generate(),
        }
    }

    fn abrir_conversa(store: &Store, org: OrgId, agente: AgentId, canal: &str) -> ConversationId {
        store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente,
                    integration_id: None,
                    canal: canal.to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("abre a conversa")
            .id
    }

    fn avancar(&self, ms: i64) {
        self.relogio.fetch_add(ms, Ordering::SeqCst);
    }

    /// Grava a mensagem do cliente e devolve o id (é o que o `chat` faz antes do turno).
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
            .expect("grava a mensagem do cliente")
            .id
    }

    fn pausa(&self) -> Arc<PausaFalsa> {
        Arc::new(PausaFalsa::default())
    }

    fn motor(
        &self,
        roteiros: Vec<Script>,
        pausa: Arc<PausaFalsa>,
    ) -> (TurnEngine, Arc<FakeProvider>) {
        self.motor_com(roteiros, pausa, TurnOptions::default())
    }

    fn motor_com(
        &self,
        roteiros: Vec<Script>,
        pausa: Arc<PausaFalsa>,
        opcoes: TurnOptions,
    ) -> (TurnEngine, Arc<FakeProvider>) {
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
        let motor = TurnEngine::new(
            Arc::clone(&self.store),
            config,
            limitador,
            provedor,
            opcoes,
            MetricasTurno::proprio(),
        )
        .with_pause(pausa);
        (motor, fake)
    }

    /// Fluxo de entrada e saída ligado a um motor, com o processador de turno de verdade.
    fn fluxo(&self, motor: Arc<TurnEngine>) -> Fluxo {
        self.fluxo_com(motor, OpcoesFluxo::default())
    }

    fn fluxo_com(&self, motor: Arc<TurnEngine>, opcoes: OpcoesFluxo) -> Fluxo {
        let processador = Arc::new(TurnProcessor::new(motor));
        let atores = Atores::novo(
            processador.clone(),
            OpcoesAtores::default(),
            MetricasMemoria::proprio(),
        );
        let fluxo = Fluxo::com_metricas(
            Arc::clone(&self.store),
            atores,
            opcoes,
            MetricasFluxo::proprio(),
        );
        processador.attach_io(fluxo.clone());
        fluxo
    }

    fn mensagens(&self) -> Vec<poph_store::Mensagem> {
        self.store
            .messages()
            .list(self.org, self.conversa, Page::with_limit(200))
            .expect("lista as mensagens")
    }

    fn turnos(&self) -> Vec<poph_store::Turno> {
        self.store
            .turns()
            .list(self.org, self.conversa, Page::with_limit(200))
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
        LlmEvent::Usage(Usage {
            input: 120,
            saida: 30,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

// ───────────────────────────── 1. Cinco mensagens ─────────────────────────────

#[tokio::test]
async fn cinco_mensagens_gravam_stream_mensagens_e_turno() {
    let cenario = Cenario::novo("cinco");
    let roteiros: Vec<Script> = (1..=5)
        .map(|n| resposta(&format!("resposta {n}")))
        .collect();
    let pausa = cenario.pausa();
    let (motor, fake) = cenario.motor(roteiros, Arc::clone(&pausa));

    let mut textos = Vec::new();
    for n in 1..=5 {
        let gatilho = cenario.falar(&format!("pergunta {n}"));
        let mut coletor = Coletor::default();
        let resumo = motor
            .run(
                TurnInput::new(cenario.org, cenario.conversa, gatilho),
                &mut coletor,
            )
            .await
            .expect("turno roda");
        assert_eq!(resumo.estado, TurnState::Completed, "{:?}", resumo.error);
        assert_eq!(resumo.texto, format!("resposta {n}"));
        assert_eq!(coletor.texto(), resumo.texto, "o stream é a resposta final");
        assert!(
            coletor
                .eventos
                .iter()
                .any(|evento| matches!(evento, TurnEvent::Started { .. })),
            "o turno avisa o início"
        );
        textos.push(resumo);
    }

    assert_eq!(fake.received().len(), 5, "uma chamada ao modelo por turno");

    // Mensagens: 5 do cliente e 5 do agente, na ordem.
    let mensagens = cenario.mensagens();
    assert_eq!(mensagens.len(), 10);
    for (indice, mensagem) in mensagens.iter().enumerate() {
        if indice % 2 == 0 {
            assert_eq!(mensagem.role, Role::User);
            assert_eq!(mensagem.direction, Direction::Inbound);
        } else {
            assert_eq!(mensagem.role, Role::Assistant);
            assert_eq!(mensagem.direction, Direction::Outbound);
        }
    }

    // Turno: estado, tokens, tempos por etapa e modelo.
    let turnos = cenario.turnos();
    assert_eq!(turnos.len(), 5);
    for turno in &turnos {
        assert_eq!(turno.state, TurnState::Completed);
        assert_eq!(turno.input_tokens, 120);
        assert_eq!(turno.output_tokens, 30);
        let tempos: serde_json::Value =
            serde_json::from_str(&turno.times_json).expect("tempos em JSON");
        for chave in [
            "fila_ms",
            "montagem_ms",
            "primeiro_texto_ms",
            "total_ms",
            "rodadas_ferramenta",
        ] {
            assert!(tempos.get(chave).is_some(), "falta `{chave}`: {tempos}");
        }
        assert_eq!(tempos["modelo"], "gemini-3.8-flash-tiered");
        assert_eq!(tempos["motivo_fim"], "concluido");
    }

    // A saída fica na outbox, pronta para o transporte.
    let saida = cenario
        .store
        .outbox()
        .list_pending(cenario.org, 10)
        .expect("lista a saída");
    assert_eq!(saida.len(), 5);
    assert_eq!(
        saida[0].content_json,
        serde_json::json!({"texto": "resposta 1"}).to_string()
    );
}

// ───────────────────────────── 2. Prefixo estável ─────────────────────────────

#[tokio::test]
async fn prefixo_identico_em_tres_turnos() {
    let cenario = Cenario::novo("prefixo");
    let pausa = cenario.pausa();
    let (motor, fake) = cenario.motor(
        vec![resposta("um"), resposta("dois"), resposta("três")],
        pausa,
    );

    for n in 1..=3 {
        let gatilho = cenario.falar(&format!("mensagem {n}"));
        // O relógio avança entre os turnos: o bloco volátil muda, o prefixo não.
        cenario.avancar(60_000);
        motor
            .run(
                TurnInput::new(cenario.org, cenario.conversa, gatilho),
                &mut NoopObserver,
            )
            .await
            .expect("turno roda");
    }

    let pedidos = fake.received();
    assert_eq!(pedidos.len(), 3);
    let prefixo = |pedido: &poph_core::llm::Request| match &pedido.messages[0] {
        poph_core::llm::Message::System(texto) => texto.clone(),
        outro => panic!("a primeira mensagem devia ser o prefixo: {outro:?}"),
    };
    let primeiro = prefixo(&pedidos[0]);
    assert!(primeiro.contains("FixProLar"), "{primeiro}");
    for pedido in &pedidos[1..] {
        assert_eq!(prefixo(pedido), primeiro, "prefixo mudou entre turnos");
    }

    // O bloco volátil (segunda mensagem de sistema) traz a hora e muda entre turnos.
    let volatil = |pedido: &poph_core::llm::Request| match &pedido.messages[1] {
        poph_core::llm::Message::System(texto) => texto.clone(),
        outro => panic!("a segunda mensagem devia ser o bloco volátil: {outro:?}"),
    };
    assert!(volatil(&pedidos[0]).contains("hora:"));
    assert_ne!(volatil(&pedidos[0]), volatil(&pedidos[1]));
}

// ───────────────────────────── 3. Duas orgs ─────────────────────────────

#[tokio::test]
async fn duas_orgs_nao_se_misturam() {
    let alfa = Cenario::novo("org-alfa");
    let beta = Cenario::novo("org-beta");
    let (motor_alfa, fake_alfa) = alfa.motor(vec![resposta("sou a alfa")], alfa.pausa());
    let (motor_beta, fake_beta) = beta.motor(vec![resposta("sou a beta")], beta.pausa());

    let gatilho_alfa = alfa.falar("quem é você?");
    let gatilho_beta = beta.falar("e você?");
    let (mut obs_alfa, mut obs_beta) = (NoopObserver, NoopObserver);
    let (resumo_alfa, resumo_beta) = tokio::join!(
        motor_alfa.run(
            TurnInput::new(alfa.org, alfa.conversa, gatilho_alfa),
            &mut obs_alfa
        ),
        motor_beta.run(
            TurnInput::new(beta.org, beta.conversa, gatilho_beta),
            &mut obs_beta
        ),
    );
    let resumo_alfa = resumo_alfa.expect("turno da alfa");
    let resumo_beta = resumo_beta.expect("turno da beta");

    assert_eq!(resumo_alfa.texto, "sou a alfa");
    assert_eq!(resumo_beta.texto, "sou a beta");

    // Cada provedor falso só viu pedidos da sua org.
    for pedido in fake_alfa.received() {
        assert_eq!(pedido.org, alfa.org);
    }
    for pedido in fake_beta.received() {
        assert_eq!(pedido.org, beta.org);
    }

    // O registro não vaza: cada org enxerga a sua conversa e a sua mensagem do agente.
    assert_eq!(alfa.turnos().len(), 1);
    assert_eq!(beta.turnos().len(), 1);
    let mensagens_beta = beta.mensagens();
    assert_eq!(mensagens_beta.len(), 2);
    assert_eq!(mensagens_beta[1].texto, "sou a beta");
    assert_eq!(mensagens_beta[1].conversation_id, beta.conversa);
}

// ───────────────────────────── 4. Janela de agrupamento ─────────────────────────────

#[tokio::test]
async fn janela_de_agrupamento_junta_duas_e_separa_depois_do_teto() {
    let cenario = Cenario::novo("janela");
    // A pausa falsa grava a segunda mensagem no meio da janela — é o que o cliente faz de verdade:
    // manda uma mensagem e completa o pensamento dois segundos depois.
    let segunda: Arc<Mutex<Option<String>>> = Arc::default();
    let segunda_texto = "queria um orçamento";
    let escrever = {
        let segunda = Arc::clone(&segunda);
        let cenario_pasta = cenario.pasta.clone();
        let store = Arc::clone(&cenario.store);
        let org = cenario.org;
        let conversa = cenario.conversa;
        move || {
            let mut guarda = segunda.lock().unwrap_or_else(|v| v.into_inner());
            if guarda.is_none() {
                let mensagem = store
                    .messages()
                    .write(
                        org,
                        NewMessage {
                            conversation_id: conversa,
                            direction: Direction::Inbound,
                            role: Role::User,
                            texto: segunda_texto.to_owned(),
                            attachments_json: "[]".to_owned(),
                            external_id: None,
                            turn_id: None,
                            origin: poph_store::Origin::Agente,
                            author_name: None,
                        },
                    )
                    .expect("grava a mensagem que chega na janela");
                *guarda = Some(mensagem.id.to_string());
                let _ = cenario_pasta;
            }
        }
    };
    let pausa = Arc::new(PausaFalsa::com_acao(escrever));
    let (motor, fake) = cenario.motor(
        vec![resposta("juntas"), resposta("sozinha")],
        Arc::clone(&pausa),
    );

    let primeira = cenario.falar("oi");
    let resumo = motor
        .run(
            TurnInput::new(cenario.org, cenario.conversa, primeira),
            &mut NoopObserver,
        )
        .await
        .expect("turno roda");
    assert_eq!(resumo.estado, TurnState::Completed);
    let segunda_id: MessageId = segunda
        .lock()
        .unwrap_or_else(|v| v.into_inner())
        .as_ref()
        .expect("a segunda mensagem chegou")
        .parse()
        .expect("id da segunda mensagem");
    assert_eq!(
        resumo.messages,
        vec![primeira, segunda_id],
        "as duas mensagens da janela entraram no mesmo turno"
    );
    assert_eq!(fake.received().len(), 1, "um turno, uma chamada ao modelo");

    // A janela foi esperada uma vez, com o valor do agente (2 s).
    let pedidos = pausa.pedidos();
    assert_eq!(pedidos, vec![Duration::from_millis(2_000)]);

    // A terceira chega depois do teto da janela: turno novo, com uma mensagem só.
    cenario.avancar(9_000);
    let terceira = cenario.falar("e o prazo?");
    let resumo = motor
        .run(
            TurnInput::new(cenario.org, cenario.conversa, terceira),
            &mut NoopObserver,
        )
        .await
        .expect("segundo turno roda");
    assert_eq!(resumo.estado, TurnState::Completed);
    assert_eq!(resumo.messages, vec![terceira], "turno novo, uma mensagem");
    assert_eq!(fake.received().len(), 2);

    // Nada se perdeu: 3 mensagens do cliente, 2 respostas, 2 turnos.
    let mensagens = cenario.mensagens();
    assert_eq!(mensagens.len(), 5);
    assert_eq!(cenario.turnos().len(), 2);
}

// ───────────────────────────── 5. Ferramentas ─────────────────────────────

/// Ferramenta de teste que responde sempre a mesma coisa.
struct FerramentaDeTeste {
    nome: &'static str,
}

impl poph_agent::turno::FerramentaInterna for FerramentaDeTeste {
    fn nome(&self) -> &'static str {
        self.nome
    }

    fn descricao(&self) -> &'static str {
        "ferramenta de teste"
    }

    fn parametros(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }

    fn executar(
        &self,
        _contexto: poph_agent::turno::ContextoFerramenta,
        _argumentos: serde_json::Value,
    ) -> poph_agent::turno::FuturoFerramenta {
        Box::pin(async { Ok("resultado da ferramenta".to_owned()) })
    }
}

struct DoisRegistros;

impl RegistroFerramentas for DoisRegistros {
    fn ferramenta(&self, nome: &str) -> Option<Arc<dyn poph_agent::turno::FerramentaInterna>> {
        match nome {
            "consultar_base" => Some(Arc::new(FerramentaDeTeste {
                nome: "consultar_base",
            })),
            "buscar_produto" => Some(Arc::new(FerramentaDeTeste {
                nome: "buscar_produto",
            })),
            _ => None,
        }
    }
}

#[tokio::test]
async fn ferramenta_nao_habilitada_nao_e_oferecida_e_laco_para_no_limite() {
    let cenario = Cenario::novo("ferramentas");
    // Agente com uma ferramenta habilitada e outra desabilitada, e limite de 1 rodada.
    cenario
        .store
        .agents()
        .update(
            cenario.org,
            cenario.agente,
            1,
            poph_store::AgentChange {
                max_tool_rounds: Some(1),
                max_tools_per_message: Some(1),
                ..poph_store::AgentChange::default()
            },
        )
        .expect("ajusta o agente");
    let instrucoes = cenario
        .store
        .instructions()
        .get(cenario.org, cenario.agente)
        .expect("lê as instruções");
    let estruturado = poph_types::StructuredInstructions {
        tools: vec![
            ToolConfig {
                name: "consultar_base".to_owned(),
                enabled: true,
                when_to_use: String::new(),
            },
            ToolConfig {
                name: "buscar_produto".to_owned(),
                enabled: false,
                when_to_use: String::new(),
            },
        ],
        ..poph_types::StructuredInstructions::default()
    };
    cenario
        .store
        .instructions()
        .save(
            cenario.org,
            cenario.agente,
            instrucoes.version,
            &estruturado.to_json(),
            &instrucoes.markdown,
            false,
        )
        .expect("grava as ferramentas");

    // Primeira resposta do modelo: pede a ferramenta desabilitada e a habilitada na mesma mensagem.
    let duas_chamadas = Script::Events(vec![
        LlmEvent::Tool(ToolCall {
            id: "c1".to_owned(),
            nome: "buscar_produto".to_owned(),
            arguments: serde_json::json!({}),
        }),
        LlmEvent::Tool(ToolCall {
            id: "c2".to_owned(),
            nome: "consultar_base".to_owned(),
            arguments: serde_json::json!({}),
        }),
        LlmEvent::Fim(FinishReason::ToolCalls),
    ]);
    let (motor, fake) = cenario.motor(vec![duas_chamadas, resposta("pronto")], cenario.pausa());
    let motor = motor.with_tools(Arc::new(DoisRegistros));

    let gatilho = cenario.falar("quanto custa a dedetização?");
    let resumo = motor
        .run(
            TurnInput::new(cenario.org, cenario.conversa, gatilho),
            &mut NoopObserver,
        )
        .await
        .expect("turno roda");
    assert_eq!(resumo.estado, TurnState::Completed);
    assert_eq!(resumo.texto, "pronto");

    let pedidos = fake.received();
    assert_eq!(
        pedidos.len(),
        2,
        "uma rodada de ferramenta (o limite) e o fecho em texto"
    );

    // Rodada 0: só a ferramenta habilitada é oferecida.
    let nomes: Vec<String> = pedidos[0]
        .tools
        .iter()
        .map(|ferramenta| ferramenta.nome.clone())
        .collect();
    assert_eq!(nomes, vec!["consultar_base".to_owned()]);

    // Chamada de ferramenta desabilitada vira resultado de erro, não execução.
    let resultados: Vec<String> = pedidos[1]
        .messages
        .iter()
        .filter_map(|mensagem| match mensagem {
            poph_core::llm::Message::Resultado {
                content: conteudo, ..
            } => Some(conteudo.clone()),
            _ => None,
        })
        .collect();
    assert!(
        resultados[0].contains("não está habilitada"),
        "{:?}",
        resultados
    );

    // A habilitada foi pedida junto e o limite de chamadas por mensagem a recusou: uma rodada só.
    assert!(
        resultados[1].contains("limite de 1 chamadas"),
        "{:?}",
        resultados
    );

    // Rodada final (limite atingido): o modelo recebe o pedido sem ferramenta nenhuma.
    assert!(pedidos[1].tools.is_empty());
}

// ───────────────────────────── 6. Cancelamento ─────────────────────────────

#[tokio::test]
async fn cancelamento_encerra_o_turno_e_conclui_a_entrada() {
    let cenario = Cenario::novo("cancelamento");
    let (motor, _fake) = cenario.motor(
        vec![Script::Pending {
            events: vec![LlmEvent::Texto("parcial ".to_owned())],
            after: Duration::from_secs(30),
            error: LlmError::TimedOut { seconds: 30 },
        }],
        cenario.pausa(),
    );
    // Cancelamento do processo, como o Ctrl-C do `chat`.
    let compartilhado = Cancellation::new();
    let fluxo = cenario.fluxo(Arc::new(motor.with_cancellation(compartilhado.clone())));

    fluxo
        .receber_mensagem(
            cenario.org,
            MensagemRecebida {
                integration_id: cenario.integracao,
                id_externo: "ext-cancel-1".to_owned(),
                conversa: cenario.conversa,
                texto: "me conta tudo, com detalhes".to_owned(),
                anexos_json: "[]".to_owned(),
            },
        )
        .expect("grava a entrada");
    fluxo.despachar(cenario.org).expect("despacha");

    // Dá tempo de o turno começar e ficar pendurado no provedor, e então cancela.
    tokio::time::sleep(Duration::from_millis(150)).await;
    compartilhado.cancel();
    esperar_fila_vazia(&fluxo, &cenario).await;

    let turnos = cenario.turnos();
    assert_eq!(turnos.len(), 1);
    assert_eq!(turnos[0].state, TurnState::Failed);
    assert!(
        turnos[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("cancelada"),
        "{:?}",
        turnos[0].error
    );
    assert!(turnos[0].finished_at.is_some(), "o turno foi fechado");

    // A entrada foi concluída (com erro) — não fica pendente nem volta em laço.
    let pendentes = cenario
        .store
        .inbox()
        .count_pending(cenario.org)
        .expect("conta a entrada");
    assert_eq!(
        pendentes, 0,
        "a entrada foi concluída mesmo no cancelamento"
    );

    // Resposta parcial não vai para o cliente e não vira mensagem do agente.
    let saida = cenario
        .store
        .outbox()
        .list_pending(cenario.org, 10)
        .expect("lista a saída");
    assert!(saida.is_empty(), "nada de resposta parcial na outbox");
    let mensagens = cenario.mensagens();
    assert_eq!(mensagens.len(), 1, "só a mensagem do cliente");
    assert_eq!(mensagens[0].role, Role::User);
}

// ───────────────────────────── 7. Erro do provedor ─────────────────────────────

#[tokio::test]
async fn erro_do_provedor_grava_o_turno_e_nao_deixa_item_preso() {
    let cenario = Cenario::novo("erro-provedor");
    let (motor, _fake) = cenario.motor(
        vec![Script::Failure(LlmError::Unavailable {
            tentativas: 3,
            last_error: "503".to_owned(),
        })],
        cenario.pausa(),
    );
    let fluxo = cenario.fluxo(Arc::new(motor));

    let recebido = fluxo
        .receber_mensagem(
            cenario.org,
            MensagemRecebida {
                integration_id: IntegrationId::generate(),
                id_externo: "ext-erro-1".to_owned(),
                conversa: cenario.conversa,
                texto: "oi".to_owned(),
                anexos_json: "[]".to_owned(),
            },
        )
        .expect("grava a entrada");
    assert_eq!(recebido.situation, poph_store::Received::New);
    fluxo.despachar(cenario.org).expect("despacha");
    esperar_tentativas(&cenario, recebido.item.id, 1).await;

    let transporte = TransporteDeTeste::default();
    fluxo
        .enviar_pendentes(cenario.org, &transporte)
        .await
        .expect("envia");

    // Turno gravado como falho, com o motivo do provedor.
    let turnos = cenario.turnos();
    assert_eq!(turnos.len(), 1);
    assert_eq!(turnos[0].state, TurnState::Failed);
    assert!(
        turnos[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("503"),
        "{:?}",
        turnos[0].error
    );
    // HR-26i: a falha do modelo adia a entrada (uma tentativa contada), não a conclui.
    let pendentes = cenario
        .store
        .inbox()
        .count_pending(cenario.org)
        .expect("conta a entrada");
    assert_eq!(pendentes, 1, "a entrada fica para nova tentativa");
    let tentativas = cenario
        .store
        .inbox()
        .attempts(cenario.org, &[recebido.item.id])
        .expect("tentativas");
    assert_eq!(tentativas, vec![1]);
    // Nenhum item preso em `sending` e nada enviado.
    let enviando = cenario
        .store
        .outbox()
        .count_by_state(cenario.org, OutboxState::Sending)
        .expect("conta a saída");
    assert_eq!(enviando, 0);
    assert!(
        transporte.enviados().is_empty(),
        "não há resposta para enviar"
    );
    // Nenhuma resposta gravada no histórico.
    let mensagens = cenario.mensagens();
    assert_eq!(mensagens.len(), 1);
    assert_eq!(mensagens[0].role, Role::User);
}

#[tokio::test]
async fn sem_vaga_deixa_a_entrada_pendente_para_nova_tentativa() {
    let cenario = Cenario::novo("sem-vaga");
    let (motor, fake) = cenario.motor(vec![resposta("não deveria ser chamada")], cenario.pausa());
    let _ = &motor; // o motor do cenário só serve para criar o provedor falso
    let provedor: Arc<dyn LlmProvider> = fake.clone();
    // Um único turno simultâneo na instância e a vaga já ocupada por outro turno.
    let limitador = Arc::new(Limitador::novo(
        OpcoesLimites {
            turnos_global: 1,
            turnos_por_org: 1,
            turnos_por_agente: 0,
            chamadas_por_provedor: 8,
            tetos_por_provedor: Default::default(),
            taxa_por_org_por_minuto: 0,
        },
        MetricasMemoria::proprio(),
    ));
    let vaga = limitador
        .adquirir_turno(cenario.org, cenario.agente, Duration::from_millis(10))
        .await
        .expect("pega a única vaga");
    let config = ConfigQuente::nova(
        Arc::clone(&cenario.store),
        OpcoesConfigQuente::default(),
        MetricasMemoria::proprio(),
    );
    let motor = TurnEngine::new(
        Arc::clone(&cenario.store),
        config,
        Arc::clone(&limitador),
        provedor,
        TurnOptions {
            slot_wait: Duration::from_millis(20),
            ..TurnOptions::default()
        },
        MetricasTurno::proprio(),
    )
    .with_pause(cenario.pausa());
    let fluxo = cenario.fluxo(Arc::new(motor));

    fluxo
        .receber_mensagem(
            cenario.org,
            MensagemRecebida {
                integration_id: cenario.integracao,
                id_externo: "ext-vaga-1".to_owned(),
                conversa: cenario.conversa,
                texto: "oi".to_owned(),
                anexos_json: "[]".to_owned(),
            },
        )
        .expect("grava a entrada");
    fluxo.despachar(cenario.org).expect("despacha");
    esperar_turno_fechado(&cenario).await;

    // O turno foi gravado como falho, mas a mensagem continua na fila para nova tentativa.
    let turnos = cenario.turnos();
    assert_eq!(turnos.len(), 1);
    assert_eq!(turnos[0].state, TurnState::Failed);
    assert!(
        turnos[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("sem vaga"),
        "{:?}",
        turnos[0].error
    );
    let pendentes = cenario
        .store
        .inbox()
        .count_pending(cenario.org)
        .expect("conta a entrada");
    assert_eq!(
        pendentes, 1,
        "a entrada fica adiada e pendente quando a falta de vaga é transitória"
    );
    drop(vaga);
}

// ───────────────────────────── 7b. Falta de vaga e recusa do provedor (HR-26k) ─────────────────────────────

/// Limitador com uma única vaga de turno e uma de chamada ao provedor. O teste segura as vagas para
/// simular a instância ocupada, ou o provedor sem vaga livre.
fn limitador_de_uma_vaga() -> Arc<Limitador> {
    Arc::new(Limitador::novo(
        OpcoesLimites {
            turnos_global: 1,
            turnos_por_org: 1,
            turnos_por_agente: 0,
            chamadas_por_provedor: 1,
            tetos_por_provedor: Default::default(),
            taxa_por_org_por_minuto: 0,
        },
        MetricasMemoria::proprio(),
    ))
}

/// Motor com espera curta por vaga (20 ms reais: a espera do turno é relógio de parede, a do adiamento
/// é o relógio falso do teste).
fn motor_com_limitador(
    cenario: &Cenario,
    roteiros: Vec<Script>,
    limitador: Arc<Limitador>,
) -> (TurnEngine, Arc<FakeProvider>) {
    let fake = Arc::new(FakeProvider::with_scripts(roteiros));
    let provedor: Arc<dyn LlmProvider> = fake.clone();
    let config = ConfigQuente::nova(
        Arc::clone(&cenario.store),
        OpcoesConfigQuente::default(),
        MetricasMemoria::proprio(),
    );
    let motor = TurnEngine::new(
        Arc::clone(&cenario.store),
        config,
        limitador,
        provedor,
        TurnOptions {
            slot_wait: Duration::from_millis(20),
            ..TurnOptions::default()
        },
        MetricasTurno::proprio(),
    )
    .with_pause(cenario.pausa());
    (motor, fake)
}

/// Provedor do agente de teste (o padrão da configuração).
const PROVEDOR_DO_AGENTE: &str = "antigravity2api";

#[tokio::test]
async fn falta_de_vaga_do_turno_entra_na_escada_e_esgota_em_tres_tentativas() {
    let cenario = Cenario::novo("vaga-turno-esgota");
    let limitador = limitador_de_uma_vaga();
    // A vaga da instância fica ocupada o tempo todo: nenhum turno chega ao modelo.
    let _ocupada = limitador
        .adquirir_turno(cenario.org, cenario.agente, Duration::from_millis(10))
        .await
        .expect("ocupa a vaga da instância");
    let (motor, fake) = motor_com_limitador(
        &cenario,
        vec![resposta("não deveria ser chamada")],
        Arc::clone(&limitador),
    );
    let motor = Arc::new(motor);
    let fluxo = cenario.fluxo_com(
        Arc::clone(&motor),
        OpcoesFluxo::default().com_falha_modelo(3, Duration::from_millis(1_000)),
    );
    let item = entregar(&fluxo, &cenario, "ext-vaga-turno-1", "oi");

    fluxo.despachar(cenario.org).expect("despacha");
    esperar_tentativas(&cenario, item, 1).await;
    // Entre as tentativas nada vai para a saída, e a entrada segue pendente (adiada).
    assert!(
        cenario
            .store
            .outbox()
            .list_pending(cenario.org, 10)
            .expect("saída")
            .is_empty()
    );
    cenario.avancar(1_000);
    fluxo.despachar(cenario.org).expect("despacha 2");
    esperar_tentativas(&cenario, item, 2).await;
    cenario.avancar(4_000);
    fluxo.despachar(cenario.org).expect("despacha 3");
    esperar_fila_vazia(&fluxo, &cenario).await;

    // Três turnos falhos e nenhuma chamada ao modelo: a vaga nunca liberou.
    assert_eq!(cenario.turnos().len(), 3);
    assert!(
        cenario
            .turnos()
            .iter()
            .all(|t| t.state == TurnState::Failed)
    );
    assert_eq!(fake.received().len(), 0);
    let concluido = cenario.store.inbox().get(cenario.org, item).expect("item");
    assert!(
        concluido.processed_at.is_some(),
        "concluída depois do limite"
    );
    assert!(
        concluido
            .ultimo_erro
            .as_deref()
            .unwrap_or_default()
            .contains("sem vaga"),
        "{:?}",
        concluido.ultimo_erro
    );
    assert_eq!(cenario.mensagens().len(), 1, "só a mensagem do cliente");

    // Passado o instante, a entrada não volta: nem turno novo, nem tentativa nova.
    cenario.avancar(3_600_000);
    fluxo.despachar(cenario.org).expect("despacha de novo");
    deixar_passar(60).await;
    assert_eq!(
        cenario.turnos().len(),
        3,
        "nenhuma repetição depois do limite"
    );
    assert_eq!(fake.received().len(), 0);

    let metricas = motor.metrics();
    assert_eq!(metricas.falhas_modelo_adiadas.value(), 2);
    assert_eq!(metricas.falhas_modelo_definitivas.value(), 1);
    assert_eq!(metricas.falhas_modelo_recusadas.value(), 0);
}

#[tokio::test]
async fn falta_de_vaga_do_turno_na_primeira_tentativa_e_resposta_na_segunda() {
    let cenario = Cenario::novo("vaga-turno-repete");
    let limitador = limitador_de_uma_vaga();
    let ocupada = limitador
        .adquirir_turno(cenario.org, cenario.agente, Duration::from_millis(10))
        .await
        .expect("ocupa a vaga da instância");
    let (motor, fake) = motor_com_limitador(
        &cenario,
        vec![resposta("bom dia, em que posso ajudar")],
        Arc::clone(&limitador),
    );
    let motor = Arc::new(motor);
    let fluxo = cenario.fluxo_com(
        Arc::clone(&motor),
        OpcoesFluxo::default().com_falha_modelo(3, Duration::from_millis(1_000)),
    );
    let item = entregar(&fluxo, &cenario, "ext-vaga-turno-2", "oi");

    fluxo.despachar(cenario.org).expect("despacha");
    esperar_tentativas(&cenario, item, 1).await;
    // A vaga volta a ficar livre antes da segunda tentativa.
    drop(ocupada);
    cenario.avancar(1_000);
    fluxo.despachar(cenario.org).expect("despacha 2");
    esperar_fila_vazia(&fluxo, &cenario).await;

    // Uma chamada ao modelo, uma resposta, uma saída: a falta de vaga não duplica nada.
    assert_eq!(fake.received().len(), 1);
    let respostas = cenario
        .mensagens()
        .into_iter()
        .filter(|m| m.role == Role::Assistant)
        .count();
    assert_eq!(respostas, 1);
    assert_eq!(
        cenario
            .store
            .outbox()
            .list_pending(cenario.org, 10)
            .expect("saída")
            .len(),
        1
    );
    let metricas = motor.metrics();
    assert_eq!(metricas.falhas_modelo_adiadas.value(), 1);
    assert_eq!(metricas.falhas_modelo_definitivas.value(), 0);
}

#[tokio::test]
async fn falta_de_vaga_do_provedor_entra_na_escada_e_nao_chama_o_modelo() {
    let cenario = Cenario::novo("vaga-provedor");
    let limitador = limitador_de_uma_vaga();
    // A única chamada ao provedor está em uso por outro turno: este não consegue a vez.
    let mut ocupada = Some(
        limitador
            .adquirir_provedor(PROVEDOR_DO_AGENTE, Duration::from_millis(10))
            .await
            .expect("ocupa a vaga do provedor"),
    );
    let (motor, fake) = motor_com_limitador(
        &cenario,
        vec![resposta("bom dia, em que posso ajudar")],
        Arc::clone(&limitador),
    );
    let motor = Arc::new(motor);
    let fluxo = cenario.fluxo_com(
        Arc::clone(&motor),
        OpcoesFluxo::default().com_falha_modelo(3, Duration::from_millis(1_000)),
    );
    let item = entregar(&fluxo, &cenario, "ext-vaga-prov-1", "oi");

    fluxo.despachar(cenario.org).expect("despacha");
    esperar_tentativas(&cenario, item, 1).await;
    // Primeira tentativa sem vaga: nenhuma chamada saiu e a saída está vazia.
    assert_eq!(fake.received().len(), 0);
    assert!(
        cenario
            .store
            .outbox()
            .list_pending(cenario.org, 10)
            .expect("saída")
            .is_empty()
    );
    // Segunda tentativa: a vaga já foi devolvida, o modelo responde.
    drop(ocupada.take());
    cenario.avancar(1_000);
    fluxo.despachar(cenario.org).expect("despacha 2");
    esperar_fila_vazia(&fluxo, &cenario).await;

    assert_eq!(fake.received().len(), 1, "uma única chamada ao provedor");
    assert_eq!(cenario.turnos().len(), 2);
    let metricas = motor.metrics();
    assert_eq!(metricas.falhas_modelo_adiadas.value(), 1);
    assert_eq!(metricas.falhas_modelo_definitivas.value(), 0);
}

#[tokio::test]
async fn provedor_que_recusa_a_chave_nao_gasta_a_escada() {
    let cenario = Cenario::novo("recusa-401");
    let (motor, fake) = cenario.motor(
        vec![Script::Failure(LlmError::Rejected {
            status: 401,
            mensagem: "chave recusada".to_owned(),
        })],
        cenario.pausa(),
    );
    let motor = Arc::new(motor);
    let fluxo = cenario.fluxo_com(
        Arc::clone(&motor),
        OpcoesFluxo::default().com_falha_modelo(3, Duration::from_millis(1_000)),
    );
    let item = entregar(&fluxo, &cenario, "ext-401-1", "oi");

    fluxo.despachar(cenario.org).expect("despacha");
    esperar_fila_vazia(&fluxo, &cenario).await;

    // Uma única chamada: a recusa não se resolve repetindo.
    assert_eq!(fake.received().len(), 1);
    let turnos = cenario.turnos();
    assert_eq!(turnos.len(), 1);
    assert_eq!(turnos[0].state, TurnState::Failed);
    let concluido = cenario.store.inbox().get(cenario.org, item).expect("item");
    assert!(
        concluido.processed_at.is_some(),
        "concluída na primeira tentativa"
    );
    assert!(
        concluido
            .ultimo_erro
            .as_deref()
            .unwrap_or_default()
            .contains("401"),
        "{:?}",
        concluido.ultimo_erro
    );
    assert!(
        cenario
            .store
            .outbox()
            .list_pending(cenario.org, 10)
            .expect("saída")
            .is_empty()
    );

    // Passado o tempo da escada, nada volta a ser chamado.
    cenario.avancar(3_600_000);
    fluxo.despachar(cenario.org).expect("despacha de novo");
    deixar_passar(60).await;
    assert_eq!(fake.received().len(), 1);

    let metricas = motor.metrics();
    assert_eq!(metricas.falhas_modelo_recusadas.value(), 1);
    assert_eq!(metricas.falhas_modelo_definitivas.value(), 1);
    assert_eq!(metricas.falhas_modelo_adiadas.value(), 0);
}

// ───────────────────────────── 8. Caminho completo ─────────────────────────────

/// Transporte de teste que guarda o que "enviou".
#[derive(Default)]
struct TransporteDeTeste {
    enviados: Mutex<Vec<OutboxItem>>,
}

impl TransporteDeTeste {
    fn enviados(&self) -> Vec<OutboxItem> {
        self.enviados
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .clone()
    }
}

impl TransporteSaida for TransporteDeTeste {
    fn enviar(&self, item: OutboxItem) -> FuturoEnvio {
        self.enviados
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .push(item);
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn fluxo_completo_da_inbox_ate_a_outbox_entrega_a_resposta() {
    let cenario = Cenario::novo("fluxo-completo");
    let (motor, fake) = cenario.motor(vec![resposta("claro, posso ajudar")], cenario.pausa());
    let fluxo = cenario.fluxo(Arc::new(motor));

    // Duas entregas da mesma mensagem: a segunda é duplicata e não gera segundo turno.
    let entrega = || MensagemRecebida {
        integration_id: cenario.integracao,
        id_externo: "ext-1".to_owned(),
        conversa: cenario.conversa,
        texto: "bom dia".to_owned(),
        anexos_json: "[]".to_owned(),
    };
    assert_eq!(
        fluxo
            .receber_mensagem(cenario.org, entrega())
            .expect("grava")
            .situation,
        poph_store::Received::New
    );
    let segunda = fluxo
        .receber_mensagem(cenario.org, entrega())
        .expect("reentrega da mesma mensagem");
    assert_eq!(
        segunda.situation,
        poph_store::Received::Duplicate,
        "a segunda entrega é duplicata, não erro"
    );

    fluxo.despachar(cenario.org).expect("despacha");
    esperar_fila_vazia(&fluxo, &cenario).await;

    let transporte = TransporteDeTeste::default();
    let relato = fluxo
        .enviar_pendentes(cenario.org, &transporte)
        .await
        .expect("envia");
    assert_eq!(relato.enviados, 1);
    let enviados = transporte.enviados();
    assert_eq!(enviados.len(), 1);
    assert_eq!(
        enviados[0].content_json,
        serde_json::json!({"texto": "claro, posso ajudar"}).to_string()
    );
    assert_eq!(
        enviados[0].conversation_id, cenario.conversa,
        "o item de saída é da conversa do turno"
    );

    // A reentrega não gerou segundo turno: uma chamada ao modelo, um turno, uma resposta.
    assert_eq!(fake.received().len(), 1, "a duplicata não abre turno");
    let turnos = cenario.turnos();
    assert_eq!(turnos.len(), 1);
    assert_eq!(turnos[0].state, TurnState::Completed);
    let mensagens = cenario.mensagens();
    assert_eq!(mensagens.len(), 2, "uma fala do cliente e uma do agente");
}

/// Espera o turno sair de `running` (a entrada pode continuar pendente, de propósito).
async fn esperar_turno_fechado(cenario: &Cenario) {
    for _ in 0..300 {
        if cenario
            .turnos()
            .iter()
            .any(|turno| turno.state != TurnState::Running)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("o turno não fechou a tempo");
}

/// Espera a `inbox` esvaziar (o turno concluiu a entrada).
async fn esperar_fila_vazia(fluxo: &Fluxo, cenario: &Cenario) {
    for _ in 0..300 {
        let pendentes = fluxo
            .estatisticas(cenario.org)
            .expect("lê as estatísticas")
            .entrada_pendente;
        let turno_rodando = cenario
            .turnos()
            .iter()
            .any(|turno| turno.state == TurnState::Running);
        if pendentes == 0 && !turno_rodando {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("a fila não esvaziou a tempo");
}

// ───────────────────────────── 8. Falha do modelo (HR-26i) ─────────────────────────────

fn falha_do_provedor() -> Script {
    Script::Failure(LlmError::Unavailable {
        tentativas: 3,
        last_error: "503 indisponível".to_owned(),
    })
}

/// Entrega uma mensagem de cliente pela `inbox` (o caminho da integração) e devolve o id do item.
fn entregar(
    fluxo: &Fluxo,
    cenario: &Cenario,
    id_externo: &str,
    texto: &str,
) -> poph_types::InboxId {
    fluxo
        .receber_mensagem(
            cenario.org,
            MensagemRecebida {
                integration_id: cenario.integracao,
                id_externo: id_externo.to_owned(),
                conversa: cenario.conversa,
                texto: texto.to_owned(),
                anexos_json: "[]".to_owned(),
            },
        )
        .expect("grava a entrada")
        .item
        .id
}

/// Espera a tentativa do item chegar ao `minimo` (o adiamento é a última coisa que o turno grava).
async fn esperar_tentativas(cenario: &Cenario, id: poph_types::InboxId, minimo: u32) {
    for _ in 0..500 {
        let atual = cenario
            .store
            .inbox()
            .attempts(cenario.org, &[id])
            .expect("tentativas")[0];
        if atual >= minimo {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("a tentativa {minimo} do item não foi registrada a tempo");
}

/// Espera a passagem do despacho terminar sem nenhum turno novo (usado para provar que nada rodou).
async fn deixar_passar(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tokio::test]
async fn falha_do_modelo_repete_com_espera_crescente_e_responde_na_terceira() {
    let cenario = Cenario::novo("falha-repete");
    let (motor, fake) = cenario.motor(
        vec![
            falha_do_provedor(),
            falha_do_provedor(),
            resposta("bom dia, em que posso ajudar"),
        ],
        cenario.pausa(),
    );
    let motor = Arc::new(motor);
    let fluxo = cenario.fluxo_com(
        Arc::clone(&motor),
        OpcoesFluxo::default().com_falha_modelo(3, Duration::from_millis(1_000)),
    );
    let item = entregar(&fluxo, &cenario, "ext-repete-1", "oi");

    // Primeira tentativa: falha, e o item fica adiado (não concluído, não vai para a saída).
    fluxo.despachar(cenario.org).expect("despacha");
    esperar_tentativas(&cenario, item, 1).await;
    assert_eq!(fake.received().len(), 1);
    assert_eq!(
        cenario
            .store
            .inbox()
            .count_pending(cenario.org)
            .expect("pendentes"),
        1,
        "adiado continua pendente"
    );
    assert!(
        cenario
            .store
            .outbox()
            .list_pending(cenario.org, 10)
            .expect("saída")
            .is_empty(),
        "nada foi para a saída por causa da falha"
    );

    // Antes da hora, o despacho não pega o item: nenhum turno novo e nenhuma chamada ao modelo.
    fluxo
        .despachar(cenario.org)
        .expect("despacha antes da hora");
    deixar_passar(60).await;
    assert_eq!(cenario.turnos().len(), 1, "nada roda antes do instante");
    assert_eq!(fake.received().len(), 1);

    // Vencida a primeira espera (1 s): segunda tentativa, que também falha.
    cenario.avancar(1_000);
    fluxo.despachar(cenario.org).expect("despacha vencido");
    esperar_tentativas(&cenario, item, 2).await;
    assert_eq!(fake.received().len(), 2);

    // A segunda espera é quatro vezes a primeira (4 s): antes disso, nada; depois, a resposta.
    cenario.avancar(3_000);
    fluxo
        .despachar(cenario.org)
        .expect("despacha antes da segunda espera");
    deixar_passar(60).await;
    assert_eq!(
        fake.received().len(),
        2,
        "espera crescente: ainda não venceu"
    );
    cenario.avancar(1_000);
    fluxo.despachar(cenario.org).expect("despacha vencido 2");
    esperar_fila_vazia(&fluxo, &cenario).await;

    // Chamadas ao modelo: exatamente uma por tentativa, sem repetição extra.
    assert_eq!(fake.received().len(), 3);
    let estados: Vec<TurnState> = cenario.turnos().iter().map(|t| t.state).collect();
    assert_eq!(
        estados.iter().filter(|e| **e == TurnState::Failed).count(),
        2
    );
    assert_eq!(
        estados
            .iter()
            .filter(|e| **e == TurnState::Completed)
            .count(),
        1
    );

    // Uma resposta só, uma só na saída, e a entrada concluída.
    let mensagens = cenario.mensagens();
    let respostas: Vec<_> = mensagens
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .collect();
    assert_eq!(respostas.len(), 1, "uma resposta do agente");
    let saida = cenario
        .store
        .outbox()
        .list_pending(cenario.org, 10)
        .expect("saída");
    assert_eq!(saida.len(), 1, "uma mensagem na outbox");
    assert_eq!(
        cenario
            .store
            .inbox()
            .count_pending(cenario.org)
            .expect("pendentes"),
        0
    );

    let metricas = motor.metrics();
    assert_eq!(metricas.falhas_modelo_adiadas.value(), 2);
    assert_eq!(metricas.falhas_modelo_definitivas.value(), 0);
}

#[tokio::test]
async fn falha_do_modelo_sempre_esgota_em_tres_tentativas_e_conclui_sem_resposta() {
    let cenario = Cenario::novo("falha-esgota");
    let (motor, fake) = cenario.motor(
        vec![
            falha_do_provedor(),
            falha_do_provedor(),
            falha_do_provedor(),
        ],
        cenario.pausa(),
    );
    let motor = Arc::new(motor);
    let fluxo = cenario.fluxo_com(
        Arc::clone(&motor),
        OpcoesFluxo::default().com_falha_modelo(3, Duration::from_millis(1_000)),
    );
    let item = entregar(&fluxo, &cenario, "ext-esgota-1", "oi");

    fluxo.despachar(cenario.org).expect("despacha");
    esperar_tentativas(&cenario, item, 1).await;
    cenario.avancar(1_000);
    fluxo.despachar(cenario.org).expect("despacha 2");
    esperar_tentativas(&cenario, item, 2).await;
    cenario.avancar(4_000);
    fluxo.despachar(cenario.org).expect("despacha 3");
    esperar_fila_vazia(&fluxo, &cenario).await;

    // Exatamente três chamadas e três turnos, todos falhos.
    assert_eq!(fake.received().len(), 3);
    let turnos = cenario.turnos();
    assert_eq!(turnos.len(), 3);
    assert!(turnos.iter().all(|t| t.state == TurnState::Failed));

    // Concluída (não volta para a fila), com o motivo guardado, sem resposta nem saída.
    let concluido = cenario.store.inbox().get(cenario.org, item).expect("item");
    assert!(concluido.processed_at.is_some(), "a entrada foi concluída");
    assert!(
        concluido
            .ultimo_erro
            .as_deref()
            .unwrap_or_default()
            .contains("503"),
        "{:?}",
        concluido.ultimo_erro
    );
    assert!(
        cenario
            .store
            .outbox()
            .list_pending(cenario.org, 10)
            .expect("saída")
            .is_empty()
    );
    assert_eq!(cenario.mensagens().len(), 1, "só a mensagem do cliente");

    // Passado o instante, o item não volta: nenhum turno novo, nenhuma chamada nova.
    cenario.avancar(3_600_000);
    fluxo.despachar(cenario.org).expect("despacha de novo");
    deixar_passar(60).await;
    assert_eq!(cenario.turnos().len(), 3);
    assert_eq!(fake.received().len(), 3);

    let metricas = motor.metrics();
    assert_eq!(metricas.falhas_modelo_adiadas.value(), 2);
    assert_eq!(metricas.falhas_modelo_definitivas.value(), 1);
}

#[tokio::test]
async fn com_limite_de_uma_tentativa_a_primeira_falha_ja_e_definitiva() {
    let cenario = Cenario::novo("falha-uma");
    let (motor, fake) = cenario.motor(vec![falha_do_provedor()], cenario.pausa());
    let motor = Arc::new(motor);
    let fluxo = cenario.fluxo_com(
        Arc::clone(&motor),
        OpcoesFluxo::default().com_falha_modelo(1, Duration::from_millis(1_000)),
    );
    entregar(&fluxo, &cenario, "ext-uma-1", "oi");
    fluxo.despachar(cenario.org).expect("despacha");
    esperar_fila_vazia(&fluxo, &cenario).await;
    assert_eq!(fake.received().len(), 1);
    assert_eq!(motor.metrics().falhas_modelo_definitivas.value(), 1);
    assert_eq!(motor.metrics().falhas_modelo_adiadas.value(), 0);
}

#[tokio::test]
async fn tempo_maximo_do_turno_conta_como_falha_do_modelo_e_entra_na_escada() {
    let cenario = Cenario::novo("tempo-maximo");
    let pendurado = Script::Pending {
        events: Vec::new(),
        after: Duration::from_secs(10),
        error: LlmError::TimedOut { seconds: 10 },
    };
    let opcoes_turno = TurnOptions {
        turn_timeout: Duration::from_millis(50),
        ..TurnOptions::default()
    };
    let (motor, fake) = cenario.motor_com(
        vec![pendurado, resposta("consegui responder")],
        cenario.pausa(),
        opcoes_turno,
    );
    let motor = Arc::new(motor);
    let fluxo = cenario.fluxo_com(
        Arc::clone(&motor),
        OpcoesFluxo::default().com_falha_modelo(3, Duration::from_millis(1_000)),
    );
    let item = entregar(&fluxo, &cenario, "ext-tempo-1", "oi");

    fluxo.despachar(cenario.org).expect("despacha");
    esperar_tentativas(&cenario, item, 1).await;
    let turno = cenario.turnos().into_iter().next().expect("um turno");
    assert_eq!(turno.state, TurnState::Failed);
    assert!(
        turno
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("tempo máximo"),
        "{:?}",
        turno.error
    );
    assert_eq!(motor.metrics().falhas_modelo_adiadas.value(), 1);

    cenario.avancar(1_000);
    fluxo.despachar(cenario.org).expect("despacha 2");
    esperar_fila_vazia(&fluxo, &cenario).await;
    assert_eq!(fake.received().len(), 2);
    assert_eq!(
        cenario
            .mensagens()
            .iter()
            .filter(|m| m.role == Role::Assistant)
            .count(),
        1
    );
}

#[tokio::test]
async fn repetir_depois_de_resposta_ja_gravada_nao_chama_o_modelo() {
    let cenario = Cenario::novo("ja-respondida");
    let (motor, fake) = cenario.motor(
        vec![falha_do_provedor(), resposta("não deveria ser usada")],
        cenario.pausa(),
    );
    let motor = Arc::new(motor);
    let fluxo = cenario.fluxo_com(
        Arc::clone(&motor),
        OpcoesFluxo::default().com_falha_modelo(3, Duration::from_millis(1_000)),
    );
    let item = entregar(&fluxo, &cenario, "ext-ja-1", "oi");
    fluxo.despachar(cenario.org).expect("despacha");
    esperar_tentativas(&cenario, item, 1).await;
    assert_eq!(fake.received().len(), 1);

    // Uma resposta chegou ao cliente antes da repetição (a entrada ficou aberta por queda, por
    // exemplo). A repetição tem de reconhecer isso e não chamar o modelo de novo.
    cenario
        .store
        .messages()
        .write(
            cenario.org,
            NewMessage {
                conversation_id: cenario.conversa,
                direction: Direction::Outbound,
                role: Role::Assistant,
                texto: "resposta que já saiu".to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
        )
        .expect("grava a resposta anterior");

    cenario.avancar(1_000);
    fluxo.despachar(cenario.org).expect("despacha de novo");
    esperar_fila_vazia(&fluxo, &cenario).await;

    assert_eq!(fake.received().len(), 1, "o modelo não foi chamado de novo");
    assert_eq!(
        cenario
            .store
            .inbox()
            .count_pending(cenario.org)
            .expect("pendentes"),
        0
    );
    assert!(
        cenario
            .store
            .outbox()
            .list_pending(cenario.org, 10)
            .expect("saída")
            .is_empty(),
        "nenhuma resposta nova"
    );
}

#[tokio::test]
async fn item_adiado_so_volta_junto_com_a_mensagem_nova_e_nao_antes_do_instante() {
    // Adiado, o item não é despachado sozinho antes do instante. Quando o cliente manda outra
    // mensagem, o turno dela atende as duas (o grupo), e a entrada adiada sai da fila com ela.
    let cenario = Cenario::novo("adiado-com-nova");
    let (motor, fake) = cenario.motor(
        vec![falha_do_provedor(), resposta("respondi as duas")],
        cenario.pausa(),
    );
    let fluxo = cenario.fluxo_com(
        Arc::new(motor),
        OpcoesFluxo::default().com_falha_modelo(3, Duration::from_millis(5_000)),
    );
    let primeira = entregar(&fluxo, &cenario, "ext-antecipa-1", "primeira");
    fluxo.despachar(cenario.org).expect("despacha");
    esperar_tentativas(&cenario, primeira, 1).await;
    assert_eq!(cenario.turnos().len(), 1);

    // Sem o instante vencido, o despacho não relança a primeira sozinha.
    fluxo.despachar(cenario.org).expect("despacha sem vencer");
    deixar_passar(60).await;
    assert_eq!(cenario.turnos().len(), 1, "o adiado não roda antes da hora");
    assert_eq!(fake.received().len(), 1);

    // A mensagem nova entra e é despachada; o turno dela leva a primeira junto, no grupo.
    entregar(&fluxo, &cenario, "ext-antecipa-2", "segunda");
    fluxo.despachar(cenario.org).expect("despacha a nova");
    esperar_fila_vazia(&fluxo, &cenario).await;

    assert_eq!(fake.received().len(), 2);
    let respostas = cenario
        .mensagens()
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .count();
    assert_eq!(respostas, 1, "uma resposta para as duas mensagens");
    assert_eq!(
        cenario
            .store
            .inbox()
            .count_pending(cenario.org)
            .expect("pendentes"),
        0
    );
}
