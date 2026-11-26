use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use futures::stream;
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::context::{COST_BY_MESSAGE, estimate_tokens};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::summary::{self, Summary, SummaryOptions};
use poph_agent::turno::{
    FuturePause, NoopObserver, Pause, TurnEngine, TurnInput, TurnOptions, TurnSummary,
};
use poph_core::llm::{
    FakeProvider, FinishReason, LlmError, LlmEvent, LlmProvider, LlmStream, Message, Part, Request,
    Script, Usage,
};
use poph_store::{
    Direction, NewAgent, NewConversation, NewMessage, NewOrg, Page, Role, Store, TurnState,
};
use poph_types::{AgentId, ConversationId, MessageId, OrgId};

struct SemEspera;

impl Pause for SemEspera {
    fn wait(&self, _duracao: Duration) -> FuturePause {
        Box::pin(async {})
    }
}

const ENCHIMENTO: &str = "palavra palavra palavra palavra palavra palavra palavra palavra palavra \
palavra palavra palavra palavra palavra palavra palavra palavra palavra palavra palavra palavra \
palavra palavra palavra palavra palavra palavra";

fn texto(i: usize) -> String {
    format!("[#{i:02}] {ENCHIMENTO}")
}

fn e_pedido_de_resumo(request: &Request) -> bool {
    matches!(request.messages.first(), Some(Message::System(texto)) if texto.starts_with("Você mantém o resumo"))
}

fn history(request: &Request) -> Vec<String> {
    request
        .messages
        .iter()
        .filter_map(|mensagem| match mensagem {
            Message::User(parts) => parts.iter().find_map(|parte| match parte {
                Part::Texto(texto) => Some(texto.clone()),
                Part::Image { .. } => None,
            }),
            Message::Assistant { texto, .. } => Some(texto.clone()),
            _ => None,
        })
        .collect()
}

fn block_volatile(request: &Request) -> Option<String> {
    request.messages.iter().find_map(|mensagem| match mensagem {
        Message::System(texto) if texto.starts_with("<contexto_do_turno>") => Some(texto.clone()),
        _ => None,
    })
}

fn tokens(textos: &[String]) -> u64 {
    textos
        .iter()
        .map(|texto| estimate_tokens(texto) + COST_BY_MESSAGE)
        .sum()
}

struct ProvedorLento {
    latencia: Duration,
    calls: AtomicU64,
}

impl LlmProvider for ProvedorLento {
    fn stream<'a>(&'a self, request: Request) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            tokio::time::sleep(self.latencia).await;
            self.calls.fetch_add(1, Ordering::SeqCst);
            let texto = if e_pedido_de_resumo(&request) {
                "Resumo: cliente pediu orçamento e prazo.".to_owned()
            } else {
                format!("[#zz] {ENCHIMENTO}")
            };
            let events = vec![
                LlmEvent::Texto(texto),
                LlmEvent::Usage(Usage {
                    input: 100,
                    saida: 20,
                }),
                LlmEvent::Fim(FinishReason::Completed),
            ];
            Ok(Box::pin(stream::iter(events.into_iter().map(Ok))) as LlmStream)
        })
    }
}

fn response(texto: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(texto.to_owned()),
        LlmEvent::Usage(Usage { input: 1, saida: 1 }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

fn failure() -> Script {
    Script::Failure(LlmError::Configuracao(
        "modelo indisponível neste teste".to_owned(),
    ))
}

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    clock: Arc<AtomicI64>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-hr14-{label}-{}-{}",
            std::process::id(),
            AgentId::generate()
        ));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("cria a pasta do teste");
        let clock = Arc::new(AtomicI64::new(1_795_130_160_000));
        let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
            .expect("abre o banco")
            .with_clock({
                let clock = Arc::clone(&clock);
                Arc::new(move || clock.load(Ordering::SeqCst))
            });
        let store = Arc::new(store);
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
                    name: format!("Bia {label}"),
                    slug: format!("bia-{label}"),
                    modelo: "gemini-3.8-flash-tiered".to_owned(),
                    grouping_window_ms: 2_000,
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
            folder,
            store,
            org,
            agente,
            clock,
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

    fn write(&self, conversa: ConversationId, role: Role, texto: String) -> MessageId {
        let direction = match role {
            Role::User => Direction::Inbound,
            _ => Direction::Outbound,
        };
        self.store
            .messages()
            .write(
                self.org,
                NewMessage {
                    conversation_id: conversa,
                    direction,
                    role,
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

    fn falar(&self, conversa: ConversationId, texto: String) -> MessageId {
        self.write(conversa, Role::User, texto)
    }

    fn historico_inicial(&self, conversa: ConversationId, beginning: usize, n: usize) {
        for i in beginning..beginning + n {
            let role = if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            self.write(conversa, role, texto(i));
        }
        self.avancar(1_000);
    }

    fn avancar(&self, ms: i64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }

    fn resumo_da_conversa(&self, conversa: ConversationId) -> Summary {
        let registro = self
            .store
            .conversations()
            .get(self.org, conversa)
            .expect("lê a conversa");
        Summary::decode(registro.summary.as_deref())
    }

    fn engine(&self, provedor: Arc<dyn LlmProvider>, summary: SummaryOptions) -> TurnEngine {
        let config = ConfigHot::new(
            Arc::clone(&self.store),
            HotConfigOptions::default(),
            MetricsMemory::own(),
        );
        let limiter = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
        TurnEngine::new(
            Arc::clone(&self.store),
            config,
            limiter,
            provedor,
            TurnOptions::default(),
            MetricsTurn::own(),
        )
        .with_pause(Arc::new(SemEspera))
        .with_summary(summary)
    }

    async fn turno(
        &self,
        engine: &TurnEngine,
        conversa: ConversationId,
        gatilho: MessageId,
    ) -> TurnSummary {
        engine
            .run(
                TurnInput::new(self.org, conversa, gatilho),
                &mut NoopObserver,
            )
            .await
            .expect("o turno roda")
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

#[tokio::test]
async fn janela_cabe_no_orcamento_e_o_resumo_e_usado_no_turno_seguinte() {
    let scenario = Scenario::new("janela");
    let conversa = scenario.conversa();
    scenario.historico_inicial(conversa, 0, 59);
    let gatilho = scenario.falar(conversa, texto(59));

    let fake = Arc::new(FakeProvider::with_scripts(vec![
        response(&texto(60)),
        response("Resumo: cliente pediu orçamento de produto."),
        response(&texto(62)),
        response("Resumo atualizado: cliente pediu orçamento e prazo."),
    ]));
    let engine = scenario.engine(fake.clone(), SummaryOptions::default());

    let first = scenario.turno(&engine, conversa, gatilho).await;
    assert_eq!(first.estado, TurnState::Completed);
    assert!(
        engine.wait_for_summaries(Duration::from_secs(10)).await,
        "o resumo não terminou a tempo"
    );

    let pedidos = fake.received();
    assert_eq!(pedidos.len(), 2, "um pedido do turno e um do resumo");
    let hist1 = history(&pedidos[0]);
    assert!(hist1.len() < 60, "a janela não pode ser a conversa inteira");
    assert_eq!(hist1.last().map(String::as_str), Some(texto(59).as_str()));
    assert!(tokens(&hist1) <= 1_000, "janela estourou o orçamento");
    assert!(block_volatile(&pedidos[0]).is_none_or(|block| !block.contains("resumo da conversa")));

    let n = hist1.len();
    let cutoff = 60 - n;
    let summary = scenario.resumo_da_conversa(conversa);
    assert_eq!(summary.texto, "Resumo: cliente pediu orçamento de produto.");
    let until = summary.until.expect("o resumo tem ponto de corte");
    let ate_texto = scenario
        .store
        .messages()
        .get(scenario.org, conversa, until)
        .expect("lê a última mensagem coberta")
        .texto;
    assert!(
        ate_texto.starts_with(&format!("[#{cutoff:02}]")),
        "o resumo deve parar na última mensagem que saiu da janela, veio {}",
        &ate_texto[..8]
    );
    let pedido_resumo = &pedidos[1];
    assert!(e_pedido_de_resumo(pedido_resumo));
    let body = match pedido_resumo.messages.last() {
        Some(Message::User(parts)) => match parts.first() {
            Some(Part::Texto(texto)) => texto.clone(),
            _ => String::new(),
        },
        _ => String::new(),
    };
    assert!(body.contains(&format!("[#{cutoff:02}]")));
    assert!(
        !body.contains(&format!("[#{:02}]", cutoff + 1)),
        "não resumir o que ainda está na janela"
    );

    let gatilho2 = scenario.falar(conversa, texto(61));
    let second = scenario.turno(&engine, conversa, gatilho2).await;
    assert_eq!(second.estado, TurnState::Completed);
    assert!(engine.wait_for_summaries(Duration::from_secs(10)).await);

    let pedidos = fake.received();
    let pedido_turno2 = &pedidos[2];
    let hist2 = history(pedido_turno2);
    let block = block_volatile(pedido_turno2).expect("o turno leva o bloco volátil");
    assert!(
        block.contains("Resumo: cliente pediu orçamento de produto."),
        "resumo não chegou ao turno"
    );
    let cost_summary = estimate_tokens("Resumo: cliente pediu orçamento de produto.") + 30;
    assert!(
        tokens(&hist2) + cost_summary <= 1_000,
        "janela do segundo turno estourou o orçamento"
    );
    assert_eq!(hist2.last().map(String::as_str), Some(texto(61).as_str()));
    assert!(
        hist2[0].starts_with(&format!("[#{:02}]", cutoff + 1)),
        "a mensagem logo após o resumo deveria estar no prompt, veio {}",
        &hist2[0][..hist2[0].len().min(8)]
    );
}

#[tokio::test]
async fn falha_do_resumo_nao_derruba_o_turno() {
    let scenario = Scenario::new("falha");
    let conversa = scenario.conversa();
    scenario.historico_inicial(conversa, 0, 40);
    let gatilho = scenario.falar(conversa, texto(40));

    let fake = Arc::new(FakeProvider::with_scripts(vec![
        response(&texto(41)),
        failure(),
        response(&texto(43)),
        failure(),
    ]));
    let engine = scenario.engine(fake.clone(), SummaryOptions::default());

    let first = scenario.turno(&engine, conversa, gatilho).await;
    assert_eq!(first.estado, TurnState::Completed);
    assert!(
        !first.texto.is_empty(),
        "a resposta saiu mesmo com o resumo falhando"
    );
    assert!(engine.wait_for_summaries(Duration::from_secs(10)).await);
    assert!(
        scenario.resumo_da_conversa(conversa).vazio(),
        "falha não grava resumo"
    );

    let gatilho2 = scenario.falar(conversa, texto(42));
    let second = scenario.turno(&engine, conversa, gatilho2).await;
    assert_eq!(second.estado, TurnState::Completed);
    assert_eq!(second.texto, texto(43));
    assert!(engine.wait_for_summaries(Duration::from_secs(10)).await);

    let registros = scenario
        .store
        .audit()
        .list(scenario.org, Page::with_limit(50))
        .expect("lê a auditoria");
    let failures = registros
        .iter()
        .filter(|registro| registro.acao == "resumo_falhou")
        .count();
    assert!(
        failures >= 1,
        "a falha do resumo fica registrada na auditoria"
    );
}

#[tokio::test]
async fn conversa_nova_nao_herda_o_resumo() {
    let scenario = Scenario::new("nova");
    let antiga = scenario.conversa();
    scenario.historico_inicial(antiga, 0, 40);
    let gatilho = scenario.falar(antiga, texto(40));
    let fake = Arc::new(FakeProvider::with_scripts(vec![
        response(&texto(41)),
        response("Resumo: cliente da conversa antiga pediu orçamento."),
    ]));
    let engine = scenario.engine(fake.clone(), SummaryOptions::default());
    scenario.turno(&engine, antiga, gatilho).await;
    assert!(engine.wait_for_summaries(Duration::from_secs(10)).await);
    assert!(
        !scenario.resumo_da_conversa(antiga).vazio(),
        "a conversa antiga tem resumo"
    );

    let new = scenario.conversa();
    let gatilho_nova = scenario.falar(new, "oi, sou um cliente novo".to_owned());
    let fake2 = Arc::new(FakeProvider::with_scripts(vec![response(
        "Olá! Como posso ajudar?",
    )]));
    let motor2 = scenario.engine(fake2.clone(), SummaryOptions::default());
    let resultado = scenario.turno(&motor2, new, gatilho_nova).await;
    assert_eq!(resultado.estado, TurnState::Completed);

    let request = &fake2.received()[0];
    let block = block_volatile(request).expect("o turno leva o bloco volátil");
    assert!(
        !block.contains("resumo da conversa"),
        "conversa nova não tem resumo"
    );
    assert!(!block.contains("cliente da conversa antiga"));
    assert!(scenario.resumo_da_conversa(new).vazio());
}

#[tokio::test]
async fn resumo_nao_aumenta_o_tempo_do_turno() {
    const LATENCIA: Duration = Duration::from_millis(100);
    const TURNOS: usize = 5;

    let scenario = Scenario::new("medicao");
    let provedor: Arc<dyn LlmProvider> = Arc::new(ProvedorLento {
        latencia: LATENCIA,
        calls: AtomicU64::new(0),
    });

    let mut sem_resumo = Vec::new();
    let mut com_resumo = Vec::new();
    for round in 0..2 {
        for ligado in [false, true] {
            let conversa = scenario.conversa();
            scenario.historico_inicial(conversa, 0, 40);
            let options = SummaryOptions {
                ativo: ligado,
                ..SummaryOptions::default()
            };
            let engine = scenario.engine(Arc::clone(&provedor), options);
            let mut tempos = Vec::new();
            for turno in 0..TURNOS {
                let gatilho = scenario.falar(conversa, texto(40 + turno + round * 10));
                let resultado = scenario.turno(&engine, conversa, gatilho).await;
                assert_eq!(resultado.estado, TurnState::Completed);
                tempos.push(resultado.timings.total_ms as f64);
                scenario.avancar(5_000);
            }
            engine.wait_for_summaries(Duration::from_secs(10)).await;
            if ligado {
                assert!(
                    !scenario.resumo_da_conversa(conversa).vazio(),
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

#[tokio::test]
async fn conversa_longa_sem_resumo_alcanca_a_janela() {
    let scenario = Scenario::new("longa");
    let conversa = scenario.conversa();
    scenario.historico_inicial(conversa, 0, 300);
    let gatilho = scenario.falar(conversa, texto(300));
    let provedor: Arc<dyn LlmProvider> = Arc::new(ProvedorLento {
        latencia: Duration::ZERO,
        calls: AtomicU64::new(0),
    });
    let engine = scenario.engine(Arc::clone(&provedor), SummaryOptions::default());

    let resultado = scenario.turno(&engine, conversa, gatilho).await;
    assert_eq!(resultado.estado, TurnState::Completed);
    assert!(engine.wait_for_summaries(Duration::from_secs(20)).await);

    let todas = scenario
        .store
        .messages()
        .last_set(scenario.org, conversa, 200)
        .expect("lê as últimas mensagens");
    let summary = scenario.resumo_da_conversa(conversa);
    let until = summary.until.expect("o resumo tem ponto de corte");
    let pos_ate = todas
        .iter()
        .position(|mensagem| mensagem.id == until)
        .expect("a mensagem coberta está entre as últimas");
    let assembly = summary::build(&todas, &summary, 1_000, 30);
    let first = assembly.messages[0].id;
    let pos_primeira = todas
        .iter()
        .position(|mensagem| mensagem.id == first)
        .expect("a primeira mensagem do prompt está entre as últimas");
    assert!(
        pos_primeira <= pos_ate + 1,
        "buraco entre o resumo e a janela: o prompt começa em {pos_primeira}, o resumo cobre até {pos_ate}"
    );
    let cutoff = todas.len()
        - summary::build(&todas, &Summary::default(), 1_000, 30)
            .messages
            .len();
    assert!(
        pos_ate + 5 >= cutoff && pos_ate < cutoff + 5,
        "o resumo deve chegar perto do corte da janela (corte {cutoff}, coberto até {pos_ate})"
    );
    assert!(!scenario.resumo_da_conversa(conversa).vazio());
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
