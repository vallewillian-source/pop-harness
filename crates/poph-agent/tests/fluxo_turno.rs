//! O fluxo do atendimento no turno (FX-23).
//!
//! O que se prova aqui, com Jev e modelo falsos e banco de verdade:
//!
//! 1. o fluxo é iniciado **antes** da chamada do modelo, e o prompt já nasce com o bloco da etapa de
//!    entrada;
//! 2. o prefixo estável é idêntico byte a byte com e sem fluxo em execução (o bloco é volátil);
//! 3. `nenhum` não inicia e **não** põe o catálogo no prompt; confiança baixa também não;
//! 4. Jev fora do ar entrega a reserva (catálogo) e o turno registra isso no `turns.jev_json`;
//! 5. fluxo de outra org, inativo ou não liberado **nunca** inicia, mesmo que o Jev escolha;
//! 6. execução ativa não volta a perguntar ao Jev, e o bloco dela entra no prompt;
//! 7. `[fluxos] ativo = false` desliga tudo e a Ficha continua existindo;
//! 8. o painel "por quê" mostra a decisão do G9 e o que o código fez com ela.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasG9, MetricasMemoria, MetricasTurno};
use poph_agent::turno::{
    Entrada, FuturoPausa, Motor, ObservadorNulo, OpcoesTurno, Pausa, ResumoTurno,
};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, Script, Usage};
use poph_fluxos::{Limites, compilar};
use poph_jev::{ErroJev, G9_NENHUM, IdGate, JevFalso, MotorJev};
use poph_store::{
    AgentChange, Direction, NewAgent, NewConversation, NewFlow, NewMessage, NewOrg,
    RevisionPublication, Role, Store,
};
use poph_types::{AgentId, ConversationId, FlowsConfig, JevGates, MessageId, OrgId};

struct SemEspera;

impl Pausa for SemEspera {
    fn esperar(&self, _duracao: Duration) -> FuturoPausa {
        Box::pin(async {})
    }
}

/// Script de um fluxo de duas etapas, com gatilho e um efeito de entrada.
const SCRIPT: &str = r#"fluxo(
    id = "agendar_visita",
    nome = "Agendar visita",
    inicio = "coleta",
    gatilho = gatilho(
        quando = "O cliente quer marcar uma visita técnica.",
        nao_quando = "O cliente só está perguntando o preço.",
        exemplos = ["quero marcar uma visita", "tem horário para o técnico?"],
    ),
)

etapa(
    id = "coleta",
    nome = "Coleta",
    instrucao = "Peça o nome e o endereço.",
    campos = [campo("nome", "Nome", obrigatorio = True)],
    acoes = [acao("seguir", "Seguir", exige = ["nome"], vai_para = fim("pronto", resultado = "Agendado"))],
)
"#;

/// Um fluxo publicado, liberado ao agente, pronto para ser escolhido pelo G9.
struct FluxoDoTeste {
    id: poph_types::FlowId,
    slug: String,
}

/// Banco, org, agente e conversa de verdade numa pasta temporária.
struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-fx23-{rotulo}-{}-{}",
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
        store
            .instructions()
            .save(
                org,
                agente,
                0,
                "{}",
                "# Bia\n\nVocê é a atendente da FixProLar.",
                true,
            )
            .expect("grava as instruções");
        Self {
            pasta,
            store,
            org,
            agente,
        }
    }

    /// Cria o fluxo, publica a primeira revisão (IR compilada do script) e o libera ao agente.
    fn fluxo(&self, agente: AgentId) -> FluxoDoTeste {
        let fluxo = self
            .store
            .flows()
            .create(
                self.org,
                NewFlow {
                    slug: "agendar_visita".to_owned(),
                    name: "Agendar visita".to_owned(),
                    description: "Visita técnica".to_owned(),
                    rascunho_fonte: SCRIPT.to_owned(),
                },
            )
            .expect("cria o fluxo");
        self.publicar(fluxo.id);
        self.store
            .flows()
            .release(self.org, agente, fluxo.id)
            .expect("libera o fluxo");
        FluxoDoTeste {
            id: fluxo.id,
            slug: "agendar_visita".to_owned(),
        }
    }

    /// Publica a revisão do fluxo com a IR compilada do script do teste.
    fn publicar(&self, fluxo: poph_types::FlowId) {
        let compilado = compilar(SCRIPT, &Limites::default());
        let ir = compilado.ir().expect("o script do teste compila");
        let atual = self.store.flows().get(self.org, fluxo).expect("lê o fluxo");
        self.store
            .revisions()
            .publish_revision(
                self.org,
                fluxo,
                RevisionPublication {
                    expected: atual.version,
                    fonte: SCRIPT.to_owned(),
                    definicao_json: serde_json::to_string(ir).expect("IR em JSON"),
                    schema_versao: Some(ir.schema_versao),
                    resolucoes_json: "{}".to_owned(),
                    ator_tipo: "system".to_owned(),
                    actor_id: None,
                },
            )
            .expect("publica a revisão");
    }

    fn conversa(&self, agente: AgentId) -> ConversationId {
        self.store
            .conversations()
            .create(
                self.org,
                NewConversation {
                    agent_id: agente,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: Some("cliente-ficticio".to_owned()),
                    target_department: None,
                },
            )
            .expect("abre a conversa")
            .id
    }

    fn falar(&self, conversa: ConversationId, texto: &str) -> MessageId {
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

    fn ajustar_jev_do_agente(&self, agente: AgentId, jev_json: &str) {
        let atual = self
            .store
            .agents()
            .get(self.org, agente)
            .expect("lê o agente");
        self.store
            .agents()
            .update(
                self.org,
                agente,
                atual.version,
                AgentChange {
                    jev_json: Some(jev_json.to_owned()),
                    ..AgentChange::default()
                },
            )
            .expect("ajusta o jev_json do agente");
    }

    fn motor(
        &self,
        agente: AgentId,
        jev: Option<Arc<MotorJev>>,
        fluxos: Option<FlowsConfig>,
        g9: MetricasG9,
    ) -> (Motor, Arc<FakeProvider>) {
        let _ = agente;
        let config = ConfigQuente::nova(
            Arc::clone(&self.store),
            OpcoesConfigQuente::default(),
            MetricasMemoria::proprio(),
        );
        let limitador = Arc::new(Limitador::novo(
            OpcoesLimites::default(),
            MetricasMemoria::proprio(),
        ));
        let provedor = Arc::new(FakeProvider::with_scripts(vec![
            roteiro("Certo! Vou anotar."),
            roteiro("Certo! Vou anotar."),
            roteiro("Certo! Vou anotar."),
        ]));
        let mut motor = Motor::novo(
            Arc::clone(&self.store),
            config,
            limitador,
            provedor.clone(),
            OpcoesTurno::default(),
            MetricasTurno::proprio(),
        )
        .com_pausa(Arc::new(SemEspera))
        .com_resumo(poph_agent::resumo::OpcoesResumo {
            ativo: false,
            ..poph_agent::resumo::OpcoesResumo::default()
        })
        .com_metricas_g9(g9);
        if let Some(ajustes) = fluxos {
            motor = motor.com_fluxos(&ajustes);
        }
        if let Some(jev) = jev {
            motor = motor.com_jev(jev);
        }
        (motor, provedor)
    }

    async fn turno(
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

    fn ficha(&self, conversa: ConversationId) -> poph_store::Ficha {
        self.store
            .cases()
            .by_conversation(self.org, conversa)
            .expect("lê a Ficha")
            .expect("a conversa tem Ficha")
    }

    fn ativa(&self, conversa: ConversationId) -> Option<poph_store::Execution> {
        self.store
            .executions()
            .active_of_case(self.org, self.ficha(conversa).id)
            .expect("lê a execução ativa")
    }

    fn eventos(&self, conversa: ConversationId) -> Vec<poph_store::RecordEvent> {
        self.store
            .cases()
            .events(self.org, self.ficha(conversa).id, None, 50)
            .expect("lê os eventos")
    }

    fn ultimo_turno(&self, conversa: ConversationId) -> poph_store::Turno {
        self.store
            .turns()
            .list(self.org, conversa, poph_store::Page::with_limit(5))
            .expect("lista os turnos")
            .into_iter()
            .next()
            .expect("há um turno gravado")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

fn roteiro(texto: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(texto.to_owned()),
        LlmEvent::Usage(Usage {
            input: 10,
            saida: 5,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

/// Gates com o G9 ligado, como no padrão de instalação (os outros ficam fora para o teste isolar).
fn so_g9() -> JevGates {
    JevGates {
        g9: true,
        ..JevGates::default()
    }
}

fn motor_jev(jev: Arc<JevFalso>) -> Arc<MotorJev> {
    Arc::new(MotorJev::novo(jev, so_g9(), "jev-latest"))
}

/// Sistema de mensagens do pedido ao modelo, na ordem em que foram enviadas.
fn sistemas(pedido: &poph_core::llm::Request) -> Vec<String> {
    pedido
        .messages
        .iter()
        .filter_map(|mensagem| match mensagem {
            poph_core::llm::Message::System(texto) => Some(texto.clone()),
            _ => None,
        })
        .collect()
}

fn pedido(pedido: &poph_core::llm::Request, trecho: &str) -> bool {
    pedido.messages.iter().any(|mensagem| match mensagem {
        poph_core::llm::Message::System(texto) => texto.contains(trecho),
        _ => false,
    })
}

/// 1. A execução é aberta **antes** do modelo, e o primeiro pedido já leva o bloco da etapa.
#[tokio::test]
async fn inicio_acontece_antes_da_chamada_do_modelo() {
    let cenario = Cenario::novo("antes");
    let fluxo = cenario.fluxo(cenario.agente);
    let conversa = cenario.conversa(cenario.agente);
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g9_iniciar_fluxo",
        &fluxo.slug,
    )])));
    let (motor, provedor) = cenario.motor(
        cenario.agente,
        Some(motor_jev(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        MetricasG9::proprias(),
    );

    let gatilho = cenario.falar(conversa, "queria marcar uma visita técnica");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;
    assert_eq!(resumo.estado, poph_store::TurnState::Completed);

    let ativa = cenario.ativa(conversa).expect("a execução foi aberta");
    assert_eq!(ativa.fluxo_id, fluxo.id);
    assert_eq!(ativa.no_atual, "coleta");

    let pedidos = provedor.received();
    assert_eq!(pedidos.len(), 1, "uma chamada ao modelo");
    assert!(
        pedido(&pedidos[0], "Fluxo em andamento: Agendar visita."),
        "o prompt do modelo já nasce com o bloco da etapa de entrada"
    );
    assert!(pedido(&pedidos[0], "Peça o nome e o endereço."));

    // O evento da abertura leva a decisão do Jev.
    let eventos = cenario.eventos(conversa);
    let iniciado = eventos
        .iter()
        .find(|evento| evento.tipo == poph_types::RecordEventKind::FlowStarted)
        .expect("há o evento fluxo_iniciado");
    let detalhe: serde_json::Value =
        serde_json::from_str(&iniciado.payload_json).expect("payload é JSON");
    assert_eq!(detalhe["fluxo"], "agendar_visita");
    assert_eq!(detalhe["ator"], "jev");
    assert_eq!(detalhe["jev"]["escolha"], "agendar_visita");
}

/// 2. O prefixo estável (primeira mensagem de sistema) é o mesmo com e sem fluxo em execução.
#[tokio::test]
async fn prefixo_estavel_identico_com_e_sem_fluxo() {
    let cenario = Cenario::novo("prefixo");
    let fluxo = cenario.fluxo(cenario.agente);

    // Conversa A: o Jev escolhe o fluxo.
    let com_fluxo = cenario.conversa(cenario.agente);
    let jev_com = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g9_iniciar_fluxo",
        &fluxo.slug,
    )])));
    let (motor_com, provedor_com) = cenario.motor(
        cenario.agente,
        Some(motor_jev(Arc::clone(&jev_com))),
        Some(FlowsConfig::default()),
        MetricasG9::proprias(),
    );
    let gatilho = cenario.falar(com_fluxo, "quero marcar visita");
    cenario.turno(&motor_com, com_fluxo, gatilho).await;

    // Conversa B: o Jev responde "nenhum" — turno sem fluxo, mesmo agente.
    let sem_fluxo = cenario.conversa(cenario.agente);
    let jev_sem = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g9_iniciar_fluxo",
        G9_NENHUM,
    )])));
    let (motor_sem, provedor_sem) = cenario.motor(
        cenario.agente,
        Some(motor_jev(Arc::clone(&jev_sem))),
        Some(FlowsConfig::default()),
        MetricasG9::proprias(),
    );
    let gatilho = cenario.falar(sem_fluxo, "só queria saber o horário");
    cenario.turno(&motor_sem, sem_fluxo, gatilho).await;

    let prefixo_com = sistemas(&provedor_com.received()[0])[0].clone();
    let prefixo_sem = sistemas(&provedor_sem.received()[0])[0].clone();
    assert_eq!(
        prefixo_com, prefixo_sem,
        "o prefixo estável não pode mudar por causa do fluxo"
    );
    // E o bloco do fluxo está fora dele.
    assert!(!prefixo_com.contains("Fluxo em andamento"));
    assert!(
        pedido(
            &provedor_com.received()[0],
            "Fluxo em andamento: Agendar visita."
        ),
        "o bloco do fluxo entra no bloco volátil, que é o segundo sistema"
    );
    assert_eq!(sistemas(&provedor_com.received()[0]).len(), 2);
}

/// 3. `nenhum` não inicia e o catálogo **não** vai ao prompt; confiança baixa também não.
#[tokio::test]
async fn nenhum_e_limiar_baixo_nao_iniciam_e_nao_oferecem_catalogo() {
    let cenario = Cenario::novo("nenhum");
    let fluxo = cenario.fluxo(cenario.agente);

    // (a) `nenhum`.
    let conversa = cenario.conversa(cenario.agente);
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g9_iniciar_fluxo",
        G9_NENHUM,
    )])));
    let g9 = MetricasG9::proprias();
    let (motor, provedor) = cenario.motor(
        cenario.agente,
        Some(motor_jev(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        g9.clone(),
    );
    let gatilho = cenario.falar(conversa, "bom dia, tudo bem?");
    cenario.turno(&motor, conversa, gatilho).await;
    assert!(
        cenario.ativa(conversa).is_none(),
        "nada inicia com `nenhum`"
    );
    assert!(
        !pedido(&provedor.received()[0], "<fluxos_de_atendimento>"),
        "o catálogo não vai ao prompt com `nenhum`"
    );
    assert_eq!(g9.nenhum.value(), 1);
    let turno = cenario.ultimo_turno(conversa);
    let gravado: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    assert_eq!(gravado["fluxo"]["resultado"], "nenhum");

    // (b) o mesmo fluxo, com o Jev escolhendo com confiança baixa.
    let conversa = cenario.conversa(cenario.agente);
    let baixa = Arc::new(JevFalso::novo(|_pedido| {
        let mut respostas = std::collections::BTreeMap::new();
        respostas.insert(
            "g9_iniciar_fluxo".to_owned(),
            poph_jev::Resposta {
                escolha: Some("agendar_visita".to_owned()),
                confianca: 0.3,
                ..poph_jev::Resposta::default()
            },
        );
        Ok(poph_jev::LoteResposta {
            respostas,
            tokens: 1,
            ..poph_jev::LoteResposta::default()
        })
    }));
    let g9 = MetricasG9::proprias();
    let (motor, provedor) = cenario.motor(
        cenario.agente,
        Some(motor_jev(Arc::clone(&baixa))),
        Some(FlowsConfig::default()),
        g9.clone(),
    );
    let gatilho = cenario.falar(conversa, "acho que queria algo de visita");
    cenario.turno(&motor, conversa, gatilho).await;
    assert!(
        cenario.ativa(conversa).is_none(),
        "confiança baixa não inicia nada, mesmo escolhendo o fluxo certo"
    );
    assert!(!pedido(&provedor.received()[0], "<fluxos_de_atendimento>"));
    assert_eq!(g9.abaixo_do_limiar.value(), 1);
    let _ = fluxo;
}

/// 4. Jev fora do ar entrega a reserva: o catálogo vai ao prompt e o turno registra o fallback.
#[tokio::test]
async fn jev_fora_entrega_a_reserva() {
    let cenario = Cenario::novo("reserva");
    let fluxo = cenario.fluxo(cenario.agente);
    let conversa = cenario.conversa(cenario.agente);
    let jev = Arc::new(JevFalso::falhando(ErroJev::Rede(
        "sem rede no teste".to_owned(),
    )));
    let g9 = MetricasG9::proprias();
    let (motor, provedor) = cenario.motor(
        cenario.agente,
        Some(motor_jev(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        g9.clone(),
    );

    let gatilho = cenario.falar(conversa, "quero marcar uma visita");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;
    assert_eq!(resumo.estado, poph_store::TurnState::Completed);
    assert!(
        cenario.ativa(conversa).is_none(),
        "Jev fora do ar não inicia nada: quem inicia é o código"
    );
    let pedidos = provedor.received();
    assert!(
        pedido(&pedidos[0], "<fluxos_de_atendimento>"),
        "a reserva põe o catálogo no prompt"
    );
    assert!(
        pedido(&pedidos[0], &fluxo.slug),
        "o catálogo traz o id do fluxo"
    );
    assert!(pedido(
        &pedidos[0],
        "O cliente quer marcar uma visita técnica."
    ));
    // Sem a ferramenta (FX-24), o catálogo vai como informação, não como instrução impossível.
    assert!(!pedido(&pedidos[0], "chame fluxo_iniciar"));

    let turno = cenario.ultimo_turno(conversa);
    let gravado: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    assert_eq!(gravado["fluxo"]["resultado"], "fallback");
    assert_eq!(g9.fallback.value(), 1);
}

/// 5a. Fluxo inativo não entra nas opções do G9 e nunca inicia, mesmo escolhido pelo Jev.
#[tokio::test]
async fn fluxo_inativo_nao_entra_nas_opcoes_nem_inicia() {
    let cenario = Cenario::novo("inativo");
    let fluxo = cenario.fluxo(cenario.agente);
    let atual = cenario
        .store
        .flows()
        .get(cenario.org, fluxo.id)
        .expect("lê o fluxo");
    cenario
        .store
        .flows()
        .update(
            cenario.org,
            fluxo.id,
            atual.version,
            poph_store::FlowChange {
                status: Some(poph_store::Status::Inactive),
                ..poph_store::FlowChange::default()
            },
        )
        .expect("desativa o fluxo");

    let conversa = cenario.conversa(cenario.agente);
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g9_iniciar_fluxo",
        &fluxo.slug,
    )])));
    let (motor, provedor) = cenario.motor(
        cenario.agente,
        Some(motor_jev(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        MetricasG9::proprias(),
    );
    let gatilho = cenario.falar(conversa, "quero marcar visita");
    cenario.turno(&motor, conversa, gatilho).await;
    assert!(
        cenario.ativa(conversa).is_none(),
        "fluxo inativo não inicia execução nova (PRD §6.2)"
    );
    // Ele nem entra nas opções do G9 (é o que `fluxo::preparar` filtra) — e por isso não aparece
    // em prompt nenhum. A recusa da validação para um fluxo que ficou inativo no meio do turno é
    // provada nos testes de unidade de `fluxo::iniciar`.
    let pedidos = provedor.received();
    assert!(!pedido(&pedidos[0], "agendar_visita"));
    assert!(!pedido(&pedidos[0], "<fluxos_de_atendimento>"));
}

/// 5b. Fluxo não liberado ao agente não entra nas opções nem inicia, mesmo escolhido pelo Jev.
#[tokio::test]
async fn fluxo_nao_liberado_nao_entra_nas_opcoes_nem_inicia() {
    let cenario = Cenario::novo("nao-liberado");
    // Outro agente é quem recebe a permissão: o agente da conversa não pode iniciar.
    let outro = cenario
        .store
        .agents()
        .create(
            cenario.org,
            NewAgent {
                name: "Outro".to_owned(),
                slug: "outro".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o outro agente")
        .id;
    let fluxo = cenario.fluxo(outro);

    let conversa = cenario.conversa(cenario.agente);
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g9_iniciar_fluxo",
        &fluxo.slug,
    )])));
    let (motor, provedor) = cenario.motor(
        cenario.agente,
        Some(motor_jev(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        MetricasG9::proprias(),
    );
    let gatilho = cenario.falar(conversa, "quero marcar visita");
    cenario.turno(&motor, conversa, gatilho).await;
    assert!(
        cenario.ativa(conversa).is_none(),
        "sem a permissão, o fluxo não é oferecido nem iniciado"
    );
    let pedidos = provedor.received();
    assert!(!pedido(&pedidos[0], "agendar_visita"));
}

/// 5c. Fluxo de outra organização nunca inicia, mesmo com o Jev escolhendo o slug dele.
#[tokio::test]
async fn fluxo_de_outra_org_nunca_inicia() {
    let cenario = Cenario::novo("outra-org");
    // A org vizinha tem um fluxo com o **mesmo slug**; ele não pode ser iniciado aqui.
    let outra_org = cenario
        .store
        .orgs()
        .create(NewOrg {
            name: "Org vizinha".to_owned(),
            slug: format!("vizinha-{}", AgentId::generate()),
            config_json: "{}".to_owned(),
        })
        .expect("cria a org vizinha")
        .id;
    let agente_vizinho = cenario
        .store
        .agents()
        .create(
            outra_org,
            NewAgent {
                name: "Vizinho".to_owned(),
                slug: "vizinho".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente vizinho")
        .id;
    let fluxo_vizinho = cenario
        .store
        .flows()
        .create(
            outra_org,
            NewFlow {
                slug: "agendar_visita".to_owned(),
                name: "Agendar visita (vizinha)".to_owned(),
                description: String::new(),
                rascunho_fonte: SCRIPT.to_owned(),
            },
        )
        .expect("cria o fluxo da vizinha");
    let atual = cenario
        .store
        .flows()
        .get(outra_org, fluxo_vizinho.id)
        .expect("lê o fluxo da vizinha");
    let compilado = compilar(SCRIPT, &Limites::default());
    let ir = compilado.ir().expect("compila");
    cenario
        .store
        .revisions()
        .publish_revision(
            outra_org,
            fluxo_vizinho.id,
            RevisionPublication {
                expected: atual.version,
                fonte: SCRIPT.to_owned(),
                definicao_json: serde_json::to_string(ir).expect("IR em JSON"),
                schema_versao: Some(ir.schema_versao),
                resolucoes_json: "{}".to_owned(),
                ator_tipo: "system".to_owned(),
                actor_id: None,
            },
        )
        .expect("publica na vizinha");
    cenario
        .store
        .flows()
        .release(outra_org, agente_vizinho, fluxo_vizinho.id)
        .expect("libera para o vizinho");

    let conversa = cenario.conversa(cenario.agente);
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g9_iniciar_fluxo",
        "agendar_visita",
    )])));
    let (motor, provedor) = cenario.motor(
        cenario.agente,
        Some(motor_jev(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        MetricasG9::proprias(),
    );
    let gatilho = cenario.falar(conversa, "quero marcar visita");
    cenario.turno(&motor, conversa, gatilho).await;
    assert!(
        cenario.ativa(conversa).is_none(),
        "fluxo de outra org não existe para esta organização"
    );
    // O slug existe só na org vizinha: não está nas opções, nem no prompt, nem em execução.
    let pedidos = provedor.received();
    assert!(!pedido(&pedidos[0], "agendar_visita"));
    // E a Ficha daqui continua sem execução nenhuma.
    assert!(
        cenario
            .store
            .executions()
            .from_case(cenario.org, cenario.ficha(conversa).id, 10)
            .expect("lê as execuções")
            .is_empty()
    );
}

/// 6. Com execução ativa o G9 não é perguntado (e o bloco dela entra no prompt).
#[tokio::test]
async fn execucao_ativa_nao_pergunta_o_jev() {
    let cenario = Cenario::novo("ativa");
    let fluxo = cenario.fluxo(cenario.agente);
    let conversa = cenario.conversa(cenario.agente);

    // O atendente abriu o fluxo pela API (FX-30) antes da mensagem.
    let ficha = cenario.ficha(conversa);
    let revisao = cenario
        .store
        .revisions()
        .current(cenario.org, fluxo.id)
        .expect("lê a revisão")
        .expect("há revisão publicada");
    let compilado = compilar(SCRIPT, &Limites::default());
    let ir = compilado.ir().expect("compila");
    let estado = poph_fluxos::motor::iniciar(ir).expect("estado inicial");
    let avaliador = poph_fluxos::Avaliador::sem_funcoes();
    let contexto = poph_fluxos::motor::Contexto::novo(&avaliador);
    let efeitos =
        poph_fluxos::motor::efeitos_de_entrada(ir, estado.no_atual.id(), &estado.dados, &contexto)
            .expect("efeitos de entrada")
            .iter()
            .map(poph_agent::fluxos_transicao::efeito_da_transicao)
            .collect();
    cenario
        .store
        .executions()
        .start_execution(
            cenario.org,
            poph_store::ExecutionStart {
                case: ficha.id,
                flow: fluxo.id,
                revision: revisao.id,
                actor: poph_types::EventActor::Attendant,
                ator_ref: Some("token-de-teste"),
                jev_decision: None,
                initial_state: poph_agent::fluxos_transicao::estado_para_banco(&estado),
                inbound_effects: efeitos,
                chave_idempotencia: Some("abertura-do-atendente"),
            },
        )
        .expect("o atendente abre a execução");

    let jev = Arc::new(JevFalso::com_escolhas(std::collections::BTreeMap::new()));
    let (motor, provedor) = cenario.motor(
        cenario.agente,
        Some(motor_jev(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        MetricasG9::proprias(),
    );
    let gatilho = cenario.falar(conversa, "meu nome é Ana Teste");
    cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(
        jev.chamadas(),
        0,
        "com execução ativa não há pergunta ao Jev"
    );
    let pedidos = provedor.received();
    assert!(pedido(&pedidos[0], "Fluxo em andamento: Agendar visita."));
    let turno = cenario.ultimo_turno(conversa);
    let gravado: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    assert_eq!(gravado["fluxo"]["resultado"], "execucao_ativa");
}

/// 7. `[fluxos] ativo = false` desliga tudo isso e a Ficha continua existindo.
#[tokio::test]
async fn fluxos_desligados_ignoram_tudo_e_a_ficha_segue() {
    let cenario = Cenario::novo("desligado");
    let fluxo = cenario.fluxo(cenario.agente);
    let conversa = cenario.conversa(cenario.agente);
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g9_iniciar_fluxo",
        &fluxo.slug,
    )])));
    let (motor, provedor) = cenario.motor(
        cenario.agente,
        Some(motor_jev(Arc::clone(&jev))),
        Some(FlowsConfig {
            enabled: false,
            ..FlowsConfig::default()
        }),
        MetricasG9::proprias(),
    );
    let gatilho = cenario.falar(conversa, "quero marcar visita");
    cenario.turno(&motor, conversa, gatilho).await;

    assert!(
        cenario.ativa(conversa).is_none(),
        "motor desligado não inicia"
    );
    assert_eq!(jev.chamadas(), 0, "motor desligado não pergunta ao Jev");
    assert!(!pedido(&provedor.received()[0], "<fluxos_de_atendimento>"));
    assert!(
        cenario
            .store
            .cases()
            .by_conversation(cenario.org, conversa)
            .expect("lê")
            .is_some(),
        "a Ficha continua sendo criada com o motor desligado"
    );
}

/// 8. O painel "por quê" (`turns.jev_json`) mostra a decisão do G9 e o que o código fez.
#[tokio::test]
async fn painel_por_que_mostra_a_decisao() {
    let cenario = Cenario::novo("painel");
    let fluxo = cenario.fluxo(cenario.agente);
    let conversa = cenario.conversa(cenario.agente);
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g9_iniciar_fluxo",
        &fluxo.slug,
    )])));
    let g9 = MetricasG9::proprias();
    let (motor, _provedor) = cenario.motor(
        cenario.agente,
        Some(motor_jev(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        g9.clone(),
    );
    let gatilho = cenario.falar(conversa, "quero marcar uma visita");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    let registrado = resumo.jev.expect("o turno devolve o Jev");
    assert_eq!(registrado.escolha(IdGate::G9), Some(fluxo.slug.as_str()));
    let turno = cenario.ultimo_turno(conversa);
    let gravado: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    assert_eq!(gravado["decisoes"][0]["gate"], "g9");
    assert_eq!(gravado["decisoes"][0]["escolha"], "agendar_visita");
    assert_eq!(gravado["fluxo"]["resultado"], "iniciou");
    assert_eq!(gravado["fluxo"]["fluxo"], "agendar_visita");
    assert_eq!(gravado["fluxo"]["etapa"], "coleta");
    assert!(gravado["fluxo"]["execucao"].as_str().is_some());
    assert_eq!(g9.iniciou.value(), 1);
}

/// 4b. Gate desligado no agente: o Jev não é perguntado e o catálogo vai ao prompt (reserva).
#[tokio::test]
async fn gate_desligado_no_agente_entrega_a_reserva_sem_perguntar() {
    let cenario = Cenario::novo("gate-off");
    let fluxo = cenario.fluxo(cenario.agente);
    cenario.ajustar_jev_do_agente(cenario.agente, r#"{"g9": false}"#);
    let conversa = cenario.conversa(cenario.agente);
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g9_iniciar_fluxo",
        &fluxo.slug,
    )])));
    let (motor, provedor) = cenario.motor(
        cenario.agente,
        Some(motor_jev(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        MetricasG9::proprias(),
    );
    let gatilho = cenario.falar(conversa, "quero marcar uma visita");
    cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(jev.chamadas(), 0, "gate desligado não é perguntado");
    assert!(cenario.ativa(conversa).is_none(), "e nada inicia sozinho");
    let pedidos = provedor.received();
    assert!(
        pedido(&pedidos[0], "<fluxos_de_atendimento>"),
        "a reserva entrega o catálogo ao modelo"
    );
    let turno = cenario.ultimo_turno(conversa);
    let gravado: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    assert_eq!(gravado["fluxo"]["resultado"], "reserva");
}

/// O estado enviado ao Jev leva os fluxos liberados (opções do G9), com o gatilho do script.
#[tokio::test]
async fn estado_do_g9_leva_os_fluxos_liberados() {
    let cenario = Cenario::novo("estado");
    let fluxo = cenario.fluxo(cenario.agente);
    let conversa = cenario.conversa(cenario.agente);
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g9_iniciar_fluxo",
        &fluxo.slug,
    )])));
    let (motor, _provedor) = cenario.motor(
        cenario.agente,
        Some(motor_jev(Arc::clone(&jev))),
        Some(FlowsConfig::default()),
        MetricasG9::proprias(),
    );
    let gatilho = cenario.falar(conversa, "quero marcar uma visita");
    cenario.turno(&motor, conversa, gatilho).await;

    let pedido = &jev.pedidos()[0];
    assert!(pedido.perguntas.contains_key("g9_iniciar_fluxo"));
    let liberados = &pedido.estado["fluxos"]["liberados"];
    assert_eq!(liberados[0]["id"], "agendar_visita");
    assert_eq!(liberados[0]["nome"], "Agendar visita");
    assert_eq!(
        liberados[0]["quando"],
        "O cliente quer marcar uma visita técnica."
    );
    assert_eq!(liberados[0]["exemplos"][0], "quero marcar uma visita");
}

fn escolhas(itens: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
    itens
        .iter()
        .map(|(id, escolha)| ((*id).to_owned(), (*escolha).to_owned()))
        .collect()
}
