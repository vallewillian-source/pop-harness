//! Testes do resumo rolante da conversa (HR-14).
//!
//! Cobrem os quatro pontos do aceite:
//! 1. conversa de 60 mensagens: a janela enviada ao modelo fica dentro do orçamento, o resumo
//!    existe e é usado no turno seguinte, e não sobra buraco entre o resumo e a janela;
//! 2. falha na geração do resumo não derruba o turno (a resposta sai e a conversa segue);
//! 3. conversa nova não herda o resumo de outra;
//! 4. o tempo de resposta do turno não aumenta quando o resumo é gerado (medido com provedor
//!    de latência simulada; não é medição com o provedor real).
//!
//! O provedor é o [`ProvedorFake`] (roteiros em ordem) e a espera da janela é uma pausa falsa.
//! Nenhum teste dorme além do que a medição de latência pede.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use futures::stream;
use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::contexto::{CUSTO_POR_MENSAGEM, estimar_tokens};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasMemoria, MetricasTurno};
use poph_agent::resumo::{self, OpcoesResumo, Resumo};
use poph_agent::turno::{
    Entrada, FuturoPausa, Motor, ObservadorNulo, OpcoesTurno, Pausa, ResumoTurno,
};
use poph_core::llm::{
    FakeProvider, FinishReason, LlmError, LlmEvent, LlmProvider, LlmStream, Message, Part, Request,
    Script, Usage,
};
use poph_store::{
    Direction, NewAgent, NewConversation, NewMessage, NewOrg, Page, Role, Store, TurnState,
};
use poph_types::{AgentId, ConversationId, MessageId, OrgId};

// ───────────────────────────── Infraestrutura ─────────────────────────────

/// Espera que não dorme.
struct SemEspera;

impl Pausa for SemEspera {
    fn esperar(&self, _duracao: Duration) -> FuturoPausa {
        Box::pin(async {})
    }
}

/// Tamanho fixo do texto de cada mensagem, para o orçamento ser previsível.
const ENCHIMENTO: &str = "palavra palavra palavra palavra palavra palavra palavra palavra palavra \
palavra palavra palavra palavra palavra palavra palavra palavra palavra palavra palavra palavra \
palavra palavra palavra palavra palavra palavra";

/// Texto da mensagem de número `i`, com marca que os testes procuram.
fn texto(i: usize) -> String {
    format!("[#{i:02}] {ENCHIMENTO}")
}

/// Texto de um pedido de resumo: é o que começa com as instruções do resumidor.
fn e_pedido_de_resumo(pedido: &Request) -> bool {
    matches!(pedido.messages.first(), Some(Message::System(texto)) if texto.starts_with("Você mantém o resumo"))
}

/// Mensagens de histórico (cliente e agente) de um pedido, em ordem.
fn historico(pedido: &Request) -> Vec<String> {
    pedido
        .messages
        .iter()
        .filter_map(|mensagem| match mensagem {
            Message::User(partes) => partes.iter().find_map(|parte| match parte {
                Part::Texto(texto) => Some(texto.clone()),
                Part::Image { .. } => None,
            }),
            Message::Assistant { texto, .. } => Some(texto.clone()),
            _ => None,
        })
        .collect()
}

/// Bloco volátil do turno (o que começa com a marca de contexto), se houver.
fn bloco_volatil(pedido: &Request) -> Option<String> {
    pedido.messages.iter().find_map(|mensagem| match mensagem {
        Message::System(texto) if texto.starts_with("<contexto_do_turno>") => Some(texto.clone()),
        _ => None,
    })
}

/// Tokens estimados de uma lista de textos de histórico, como a janela conta.
fn tokens(textos: &[String]) -> u64 {
    textos
        .iter()
        .map(|texto| estimar_tokens(texto) + CUSTO_POR_MENSAGEM)
        .sum()
}

/// Provedor de latência simulada: espera antes de responder e distingue o pedido de resumo.
struct ProvedorLento {
    latencia: Duration,
    chamadas: AtomicU64,
}

impl LlmProvider for ProvedorLento {
    fn stream<'a>(&'a self, pedido: Request) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            tokio::time::sleep(self.latencia).await;
            self.chamadas.fetch_add(1, Ordering::SeqCst);
            let texto = if e_pedido_de_resumo(&pedido) {
                "Resumo: cliente pediu orçamento e prazo.".to_owned()
            } else {
                // Mesmo tamanho das mensagens da conversa, sem marca numérica.
                format!("[#zz] {ENCHIMENTO}")
            };
            let eventos = vec![
                LlmEvent::Texto(texto),
                LlmEvent::Usage(Usage {
                    input: 100,
                    saida: 20,
                }),
                LlmEvent::Fim(FinishReason::Completed),
            ];
            Ok(Box::pin(stream::iter(eventos.into_iter().map(Ok))) as LlmStream)
        })
    }
}

fn resposta(texto: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(texto.to_owned()),
        LlmEvent::Usage(Usage { input: 1, saida: 1 }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

fn falha() -> Script {
    Script::Failure(LlmError::Configuracao(
        "modelo indisponível neste teste".to_owned(),
    ))
}

/// Um banco de verdade em pasta temporária, com org, agente e instruções.
struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    relogio: Arc<AtomicI64>,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-hr14-{rotulo}-{}-{}",
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
                    // Orçamento mínimo aceito pelo banco: é ele que corta a janela nos testes.
                    max_context_tokens: 1_000,
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
            relogio,
        }
    }

    fn conversa(&self) -> ConversationId {
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

    fn gravar(&self, conversa: ConversationId, papel: Role, texto: String) -> MessageId {
        let direcao = match papel {
            Role::User => Direction::Inbound,
            _ => Direction::Outbound,
        };
        self.store
            .messages()
            .write(
                self.org,
                NewMessage {
                    conversation_id: conversa,
                    direction: direcao,
                    role: papel,
                    texto,
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

    /// Grava a fala do cliente, como o `chat` faz antes do turno.
    fn falar(&self, conversa: ConversationId, texto: String) -> MessageId {
        self.gravar(conversa, Role::User, texto)
    }

    /// Grava `n` mensagens alternando cliente e agente, começando em `inicio`.
    fn historico_inicial(&self, conversa: ConversationId, inicio: usize, n: usize) {
        for i in inicio..inicio + n {
            let papel = if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            self.gravar(conversa, papel, texto(i));
        }
        self.avancar(1_000);
    }

    fn avancar(&self, ms: i64) {
        self.relogio.fetch_add(ms, Ordering::SeqCst);
    }

    fn resumo_da_conversa(&self, conversa: ConversationId) -> Resumo {
        let registro = self
            .store
            .conversations()
            .get(self.org, conversa)
            .expect("lê a conversa");
        Resumo::decodificar(registro.summary.as_deref())
    }

    fn motor(&self, provedor: Arc<dyn LlmProvider>, resumo: OpcoesResumo) -> Motor {
        let config = ConfigQuente::nova(
            Arc::clone(&self.store),
            OpcoesConfigQuente::default(),
            MetricasMemoria::proprio(),
        );
        let limitador = Arc::new(Limitador::novo(
            OpcoesLimites::default(),
            MetricasMemoria::proprio(),
        ));
        Motor::novo(
            Arc::clone(&self.store),
            config,
            limitador,
            provedor,
            OpcoesTurno::default(),
            MetricasTurno::proprio(),
        )
        .com_pausa(Arc::new(SemEspera))
        .com_resumo(resumo)
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
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

// ───────────────────────────── Testes ─────────────────────────────

/// Aceite 1: 60 mensagens, janela dentro do orçamento, resumo gravado e usado no turno seguinte.
#[tokio::test]
async fn janela_cabe_no_orcamento_e_o_resumo_e_usado_no_turno_seguinte() {
    let cenario = Cenario::novo("janela");
    let conversa = cenario.conversa();
    // 59 mensagens antes do gatilho: 60 no total quando o cliente fala.
    cenario.historico_inicial(conversa, 0, 59);
    let gatilho = cenario.falar(conversa, texto(59));

    let fake = Arc::new(FakeProvider::with_scripts(vec![
        resposta(&texto(60)),
        resposta("Resumo: cliente pediu orçamento de produto."),
        resposta(&texto(62)),
        resposta("Resumo atualizado: cliente pediu orçamento e prazo."),
    ]));
    let motor = cenario.motor(fake.clone(), OpcoesResumo::default());

    let primeiro = cenario.turno(&motor, conversa, gatilho).await;
    assert_eq!(primeiro.estado, TurnState::Completed);
    assert!(
        motor.aguardar_resumos(Duration::from_secs(10)).await,
        "o resumo não terminou a tempo"
    );

    let pedidos = fake.received();
    assert_eq!(pedidos.len(), 2, "um pedido do turno e um do resumo");
    let hist1 = historico(&pedidos[0]);
    assert!(hist1.len() < 60, "a janela não pode ser a conversa inteira");
    assert_eq!(hist1.last().map(String::as_str), Some(texto(59).as_str()));
    assert!(tokens(&hist1) <= 1_000, "janela estourou o orçamento");
    assert!(bloco_volatil(&pedidos[0]).is_none_or(|bloco| !bloco.contains("resumo da conversa")));

    // O resumo cobre tudo o que saiu da janela e nada que ainda está nela.
    let n = hist1.len();
    let corte = 60 - n;
    let resumo = cenario.resumo_da_conversa(conversa);
    assert_eq!(resumo.texto, "Resumo: cliente pediu orçamento de produto.");
    let ate = resumo.ate.expect("o resumo tem ponto de corte");
    let ate_texto = cenario
        .store
        .messages()
        .get(cenario.org, conversa, ate)
        .expect("lê a última mensagem coberta")
        .texto;
    assert!(
        ate_texto.starts_with(&format!("[#{corte:02}]")),
        "o resumo deve parar na última mensagem que saiu da janela, veio {}",
        &ate_texto[..8]
    );
    let pedido_resumo = &pedidos[1];
    assert!(e_pedido_de_resumo(pedido_resumo));
    let corpo = match pedido_resumo.messages.last() {
        Some(Message::User(partes)) => match partes.first() {
            Some(Part::Texto(texto)) => texto.clone(),
            _ => String::new(),
        },
        _ => String::new(),
    };
    assert!(corpo.contains(&format!("[#{corte:02}]")));
    assert!(
        !corpo.contains(&format!("[#{:02}]", corte + 1)),
        "não resumir o que ainda está na janela"
    );

    // Turno seguinte: o resumo entra no bloco volátil e a janela continua dentro do orçamento.
    let gatilho2 = cenario.falar(conversa, texto(61));
    let segundo = cenario.turno(&motor, conversa, gatilho2).await;
    assert_eq!(segundo.estado, TurnState::Completed);
    assert!(motor.aguardar_resumos(Duration::from_secs(10)).await);

    let pedidos = fake.received();
    let pedido_turno2 = &pedidos[2];
    let hist2 = historico(pedido_turno2);
    let bloco = bloco_volatil(pedido_turno2).expect("o turno leva o bloco volátil");
    assert!(
        bloco.contains("Resumo: cliente pediu orçamento de produto."),
        "resumo não chegou ao turno"
    );
    let custo_resumo = estimar_tokens("Resumo: cliente pediu orçamento de produto.") + 30;
    assert!(
        tokens(&hist2) + custo_resumo <= 1_000,
        "janela do segundo turno estourou o orçamento"
    );
    assert_eq!(hist2.last().map(String::as_str), Some(texto(61).as_str()));
    // Sem buraco: a primeira mensagem depois do resumo está no prompt.
    assert!(
        hist2[0].starts_with(&format!("[#{:02}]", corte + 1)),
        "a mensagem logo após o resumo deveria estar no prompt, veio {}",
        &hist2[0][..hist2[0].len().min(8)]
    );
}

/// Aceite 2: falha ao gerar o resumo não derruba o turno nem a conversa.
#[tokio::test]
async fn falha_do_resumo_nao_derruba_o_turno() {
    let cenario = Cenario::novo("falha");
    let conversa = cenario.conversa();
    cenario.historico_inicial(conversa, 0, 40);
    let gatilho = cenario.falar(conversa, texto(40));

    let fake = Arc::new(FakeProvider::with_scripts(vec![
        resposta(&texto(41)),
        falha(),
        resposta(&texto(43)),
        falha(),
    ]));
    let motor = cenario.motor(fake.clone(), OpcoesResumo::default());

    let primeiro = cenario.turno(&motor, conversa, gatilho).await;
    assert_eq!(primeiro.estado, TurnState::Completed);
    assert!(
        !primeiro.texto.is_empty(),
        "a resposta saiu mesmo com o resumo falhando"
    );
    assert!(motor.aguardar_resumos(Duration::from_secs(10)).await);
    assert!(
        cenario.resumo_da_conversa(conversa).vazio(),
        "falha não grava resumo"
    );

    let gatilho2 = cenario.falar(conversa, texto(42));
    let segundo = cenario.turno(&motor, conversa, gatilho2).await;
    assert_eq!(segundo.estado, TurnState::Completed);
    assert_eq!(segundo.texto, texto(43));
    assert!(motor.aguardar_resumos(Duration::from_secs(10)).await);

    let registros = cenario
        .store
        .audit()
        .list(cenario.org, Page::with_limit(50))
        .expect("lê a auditoria");
    let falhas = registros
        .iter()
        .filter(|registro| registro.acao == "resumo_falhou")
        .count();
    assert!(
        falhas >= 1,
        "a falha do resumo fica registrada na auditoria"
    );
}

/// Aceite 3: a conversa nova não recebe o resumo de outra conversa.
#[tokio::test]
async fn conversa_nova_nao_herda_o_resumo() {
    let cenario = Cenario::novo("nova");
    let antiga = cenario.conversa();
    cenario.historico_inicial(antiga, 0, 40);
    let gatilho = cenario.falar(antiga, texto(40));
    let fake = Arc::new(FakeProvider::with_scripts(vec![
        resposta(&texto(41)),
        resposta("Resumo: cliente da conversa antiga pediu orçamento."),
    ]));
    let motor = cenario.motor(fake.clone(), OpcoesResumo::default());
    cenario.turno(&motor, antiga, gatilho).await;
    assert!(motor.aguardar_resumos(Duration::from_secs(10)).await);
    assert!(
        !cenario.resumo_da_conversa(antiga).vazio(),
        "a conversa antiga tem resumo"
    );

    let nova = cenario.conversa();
    let gatilho_nova = cenario.falar(nova, "oi, sou um cliente novo".to_owned());
    let fake2 = Arc::new(FakeProvider::with_scripts(vec![resposta(
        "Olá! Como posso ajudar?",
    )]));
    let motor2 = cenario.motor(fake2.clone(), OpcoesResumo::default());
    let resultado = cenario.turno(&motor2, nova, gatilho_nova).await;
    assert_eq!(resultado.estado, TurnState::Completed);

    let pedido = &fake2.received()[0];
    let bloco = bloco_volatil(pedido).expect("o turno leva o bloco volátil");
    assert!(
        !bloco.contains("resumo da conversa"),
        "conversa nova não tem resumo"
    );
    assert!(!bloco.contains("cliente da conversa antiga"));
    assert!(cenario.resumo_da_conversa(nova).vazio());
}

/// Aceite 4: o tempo de resposta do turno não aumenta com o resumo (latência simulada).
#[tokio::test]
async fn resumo_nao_aumenta_o_tempo_do_turno() {
    const LATENCIA: Duration = Duration::from_millis(100);
    const TURNOS: usize = 5;

    let cenario = Cenario::novo("medicao");
    let provedor: Arc<dyn LlmProvider> = Arc::new(ProvedorLento {
        latencia: LATENCIA,
        chamadas: AtomicU64::new(0),
    });

    let mut sem_resumo = Vec::new();
    let mut com_resumo = Vec::new();
    for rodada in 0..2 {
        for ligado in [false, true] {
            let conversa = cenario.conversa();
            cenario.historico_inicial(conversa, 0, 40);
            let opcoes = OpcoesResumo {
                ativo: ligado,
                ..OpcoesResumo::default()
            };
            let motor = cenario.motor(Arc::clone(&provedor), opcoes);
            let mut tempos = Vec::new();
            for turno in 0..TURNOS {
                let gatilho = cenario.falar(conversa, texto(40 + turno + rodada * 10));
                let resultado = cenario.turno(&motor, conversa, gatilho).await;
                assert_eq!(resultado.estado, TurnState::Completed);
                tempos.push(resultado.tempos.total_ms as f64);
                cenario.avancar(5_000);
            }
            motor.aguardar_resumos(Duration::from_secs(10)).await;
            if ligado {
                assert!(
                    !cenario.resumo_da_conversa(conversa).vazio(),
                    "com o resumo ligado, a conversa tem resumo"
                );
                com_resumo.extend(tempos);
            } else {
                sem_resumo.extend(tempos);
            }
        }
    }

    let mediana_sem = mediana(&mut sem_resumo);
    let mediana_com = mediana(&mut com_resumo);
    eprintln!(
        "medição HR-14 (latência simulada de {} ms por chamada): turno sem resumo mediana {:.1} ms; \
         turno com resumo mediana {:.1} ms; {} turnos em cada lado",
        LATENCIA.as_millis(),
        mediana_sem,
        mediana_com,
        sem_resumo.len()
    );
    assert!(
        mediana_com <= mediana_sem + 50.0,
        "o resumo não pode atrasar o turno: sem {mediana_sem:.1} ms, com {mediana_com:.1} ms"
    );
}

/// Aceite 5 (extra): conversa longa sem resumo alcança a janela numa única execução em segundo
/// plano, com várias rodadas, sem deixar buraco entre o resumo e a janela.
#[tokio::test]
async fn conversa_longa_sem_resumo_alcanca_a_janela() {
    let cenario = Cenario::novo("longa");
    let conversa = cenario.conversa();
    cenario.historico_inicial(conversa, 0, 300);
    let gatilho = cenario.falar(conversa, texto(300));
    let provedor: Arc<dyn LlmProvider> = Arc::new(ProvedorLento {
        latencia: Duration::ZERO,
        chamadas: AtomicU64::new(0),
    });
    let motor = cenario.motor(Arc::clone(&provedor), OpcoesResumo::default());

    let resultado = cenario.turno(&motor, conversa, gatilho).await;
    assert_eq!(resultado.estado, TurnState::Completed);
    assert!(motor.aguardar_resumos(Duration::from_secs(20)).await);

    // A janela do turno seguinte começa logo depois do resumo: sem buraco e sem o resumo ter
    // andado além do corte.
    let todas = cenario
        .store
        .messages()
        .last_set(cenario.org, conversa, 200)
        .expect("lê as últimas mensagens");
    let resumo = cenario.resumo_da_conversa(conversa);
    let ate = resumo.ate.expect("o resumo tem ponto de corte");
    let pos_ate = todas
        .iter()
        .position(|mensagem| mensagem.id == ate)
        .expect("a mensagem coberta está entre as últimas");
    let montagem = resumo::montar(&todas, &resumo, 1_000, 30);
    let primeira = montagem.mensagens[0].id;
    let pos_primeira = todas
        .iter()
        .position(|mensagem| mensagem.id == primeira)
        .expect("a primeira mensagem do prompt está entre as últimas");
    assert!(
        pos_primeira <= pos_ate + 1,
        "buraco entre o resumo e a janela: o prompt começa em {pos_primeira}, o resumo cobre até {pos_ate}"
    );
    let corte = todas.len()
        - resumo::montar(&todas, &Resumo::default(), 1_000, 30)
            .mensagens
            .len();
    assert!(
        pos_ate + 5 >= corte && pos_ate < corte + 5,
        "o resumo deve chegar perto do corte da janela (corte {corte}, coberto até {pos_ate})"
    );
    assert!(!cenario.resumo_da_conversa(conversa).vazio());
}

fn mediana(valores: &mut [f64]) -> f64 {
    valores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let meio = valores.len() / 2;
    if valores.len().is_multiple_of(2) {
        (valores[meio - 1] + valores[meio]) / 2.0
    } else {
        valores[meio]
    }
}
