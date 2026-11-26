use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poph_agent::actors::{ActorOptions, Actors, BoxFuture, Processor, Task};
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::context::{build, estimate_tokens};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::MetricsMemory;
use poph_store::{Direction, Mensagem, NewAgent, NewConversation, NewMessage, NewOrg, Role, Store};
use poph_types::{AgentId, ConversationId, MessageId, OrgId};

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    conversa: ConversationId,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let folder = std::env::temp_dir().join(format!("poph-hr11-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("cria a pasta do teste");
        let store = Arc::new(
            Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
                .expect("abre o banco"),
        );
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Org HR-11".to_owned(),
                slug: format!("org-hr11-{label}"),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        Self {
            folder,
            store,
            org,
            agente: AgentId::generate(),
            conversa: ConversationId::generate(),
        }
    }

    fn criar_agente(&mut self, slug: &str) -> AgentId {
        let agente = self
            .store
            .agents()
            .create(
                self.org,
                NewAgent {
                    name: format!("Agente {slug}"),
                    slug: slug.to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente");
        self.agente = agente.id;
        self.abrir_conversa();
        agente.id
    }

    fn abrir_conversa(&mut self) {
        let conversa = self
            .store
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
            .expect("abre a conversa");
        self.conversa = conversa.id;
    }

    fn textos(&self) -> Vec<String> {
        self.store
            .messages()
            .list(self.org, self.conversa, poph_store::Page::with_limit(200))
            .expect("lista as mensagens")
            .into_iter()
            .map(|mensagem| mensagem.texto)
            .collect()
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

#[derive(Default)]
struct DadosSerializacao {
    executando: AtomicUsize,
    maximum: AtomicUsize,
    ordem: Mutex<Vec<MessageId>>,
    atraso: Duration,
}

struct Serializador {
    dados: Arc<DadosSerializacao>,
}

impl Processor for Serializador {
    fn process(&self, task: Task) -> BoxFuture {
        let dados = Arc::clone(&self.dados);
        Box::pin(async move {
            let current = dados.executando.fetch_add(1, Ordering::SeqCst) + 1;
            dados.maximum.fetch_max(current, Ordering::SeqCst);
            dados.ordem.lock().expect("sem veneno").push(task.mensagem);
            tokio::time::sleep(dados.atraso).await;
            dados.executando.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dois_eventos_da_mesma_conversa_nunca_rodam_em_paralelo() {
    let scenario = Scenario::new("serializacao");
    let dados = Arc::new(DadosSerializacao {
        atraso: Duration::from_millis(15),
        ..DadosSerializacao::default()
    });
    let actors = Actors::new(
        Arc::new(Serializador {
            dados: Arc::clone(&dados),
        }),
        ActorOptions {
            ociosidade: Duration::from_secs(30),
            queue_by_conversation: 32,
            max_actors: 16,
        },
        MetricsMemory::own(),
    );
    let sent: Vec<MessageId> = (0..8).map(|_| MessageId::generate()).collect();
    for mensagem in &sent {
        actors
            .send(Task {
                org: scenario.org,
                conversa: scenario.conversa,
                mensagem: *mensagem,
                entry: None,
            })
            .expect("enfileira");
    }
    assert!(
        actors.wait_queues_empty(Duration::from_secs(10)).await,
        "as filas deviam esvaziar"
    );
    assert_eq!(
        dados.maximum.load(Ordering::SeqCst),
        1,
        "nunca pode haver dois turnos na mesma conversa"
    );
    assert_eq!(*dados.ordem.lock().expect("sem veneno"), sent);
    assert_eq!(actors.actors_live(), 1);
}

struct ContadorDeEco {
    store: Arc<Store>,
    org: OrgId,
    conversa: ConversationId,
    acordadas: Arc<AtomicUsize>,
    recebidas: Arc<AtomicUsize>,
}

impl Processor for ContadorDeEco {
    fn process(&self, _tarefa: Task) -> BoxFuture {
        let store = Arc::clone(&self.store);
        let org = self.org;
        let conversa = self.conversa;
        let recebidas = Arc::clone(&self.recebidas);
        Box::pin(async move {
            let numero = recebidas.fetch_add(1, Ordering::SeqCst) + 1;
            store
                .messages()
                .write(
                    org,
                    NewMessage {
                        conversation_id: conversa,
                        direction: Direction::Outbound,
                        role: Role::Assistant,
                        texto: format!("eco {numero}"),
                        attachments_json: String::new(),
                        external_id: None,
                        turn_id: None,
                        origin: poph_store::Origin::Agente,
                        author_name: None,
                    },
                )
                .expect("grava a resposta");
            Ok(())
        })
    }

    fn to_acordar(&self, _org: OrgId, _conversa: ConversationId) -> BoxFuture {
        let acordadas = Arc::clone(&self.acordadas);
        let store = Arc::clone(&self.store);
        let org = self.org;
        let conversa = self.conversa;
        Box::pin(async move {
            store
                .messages()
                .last_set(org, conversa, 10)
                .expect("lê o histórico ao acordar");
            acordadas.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn conversa_ociosa_e_despejada_e_recarregada_sem_perder_mensagem() {
    let mut scenario = Scenario::new("despejo");
    scenario.criar_agente("despejo");
    let acordadas = Arc::new(AtomicUsize::new(0));
    let recebidas = Arc::new(AtomicUsize::new(0));
    let actors = Actors::new(
        Arc::new(ContadorDeEco {
            store: Arc::clone(&scenario.store),
            org: scenario.org,
            conversa: scenario.conversa,
            acordadas: Arc::clone(&acordadas),
            recebidas: Arc::clone(&recebidas),
        }),
        ActorOptions {
            ociosidade: Duration::from_millis(80),
            queue_by_conversation: 32,
            max_actors: 16,
        },
        MetricsMemory::own(),
    );

    for _ in 0..5 {
        actors
            .send(Task {
                org: scenario.org,
                conversa: scenario.conversa,
                mensagem: MessageId::generate(),
                entry: None,
            })
            .expect("enfileira");
    }
    assert!(actors.wait_queues_empty(Duration::from_secs(5)).await);
    assert_eq!(actors.actors_live(), 1);

    assert!(
        actors.wait_despejo(Duration::from_secs(5)).await,
        "o ator ocioso devia ser despejado"
    );
    assert_eq!(acordadas.load(Ordering::SeqCst), 1, "acordou uma vez");

    for _ in 0..3 {
        actors
            .send(Task {
                org: scenario.org,
                conversa: scenario.conversa,
                mensagem: MessageId::generate(),
                entry: None,
            })
            .expect("enfileira depois do despejo");
    }
    assert!(actors.wait_queues_empty(Duration::from_secs(5)).await);
    assert_eq!(acordadas.load(Ordering::SeqCst), 2, "recarregou do banco");

    let textos = scenario.textos();
    let expected: Vec<String> = (1..=8).map(|numero| format!("eco {numero}")).collect();
    assert_eq!(
        textos, expected,
        "nenhuma mensagem pode faltar nem trocar de ordem"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mudanca_de_versao_invalida_o_cache() {
    let mut scenario = Scenario::new("cache");
    let agente = scenario.criar_agente("cache");
    scenario
        .store
        .instructions()
        .save(
            scenario.org,
            agente,
            0,
            r#"{"identity":{"name":"Bia"}}"#,
            "v1 do prompt",
            true,
        )
        .expect("grava as instruções");

    let metrics = MetricsMemory::own();
    let cache = ConfigHot::new(
        Arc::clone(&scenario.store),
        HotConfigOptions::default(),
        metrics.clone(),
    );

    let first = cache.load_verify(scenario.org, agente).expect("carrega");
    assert_eq!(first.markdown, "v1 do prompt");
    assert_eq!(first.versao(), 1);
    assert_eq!(metrics.config_errors.value(), 1);

    let segunda = cache
        .load_verify(scenario.org, agente)
        .expect("carrega do cache");
    assert_eq!(segunda.markdown, "v1 do prompt");
    assert_eq!(metrics.config_hits.value(), 1);

    let instructions = scenario
        .store
        .instructions()
        .get(scenario.org, agente)
        .expect("lê as instruções");
    scenario
        .store
        .instructions()
        .save(
            scenario.org,
            agente,
            instructions.version,
            r#"{"identity":{"name":"Bia v2"}}"#,
            "v2 do prompt",
            true,
        )
        .expect("regrava as instruções");

    let terceira = cache.load_verify(scenario.org, agente).expect("recarrega");
    assert_eq!(terceira.markdown, "v2 do prompt");
    assert_eq!(metrics.config_invalidadas.value(), 1);

    let current = scenario
        .store
        .agents()
        .get(scenario.org, agente)
        .expect("lê o agente");
    scenario
        .store
        .agents()
        .update(
            scenario.org,
            agente,
            current.version,
            poph_store::AgentChange {
                name: Some("Bia Editada".to_owned()),
                ..poph_store::AgentChange::default()
            },
        )
        .expect("edita o agente");
    let quarta = cache
        .load_verify(scenario.org, agente)
        .expect("recarrega de novo");
    assert_eq!(quarta.agente.name, "Bia Editada");
    assert_eq!(quarta.markdown, "v2 do prompt");

    let outra_org = scenario
        .store
        .orgs()
        .create(NewOrg {
            name: "Outra".to_owned(),
            slug: "outra-org".to_owned(),
            config_json: String::new(),
        })
        .expect("cria a outra org")
        .id;
    assert!(
        cache.load_verify(outra_org, agente).is_err(),
        "agente não existe na outra org"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn org_saturada_nao_atrasa_outra_org() {
    let limiter = Limiter::new(
        LimitOptions {
            turns_global: 16,
            turns_by_org: 1,
            turns_by_agent: 0,
            calls_by_provider: 4,
            cap_by_provider: Default::default(),
            taxa_por_org_por_minuto: 0,
        },
        MetricsMemory::own(),
    );
    let org_a = OrgId::generate();
    let org_b = OrgId::generate();
    let agente = AgentId::generate();

    let vaga_a = limiter
        .acquire_turn(org_a, agente, Duration::from_millis(200))
        .await
        .expect("org A pega a vaga");
    let segura = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        drop(vaga_a);
    });

    tokio::time::sleep(Duration::from_millis(30)).await;
    let beginning = Instant::now();
    let vaga_b = limiter
        .acquire_turn(org_b, agente, Duration::from_secs(2))
        .await
        .expect("org B pega a vaga");
    let esperou = beginning.elapsed();
    drop(vaga_b);
    segura.await.expect("tarefa de apoio");
    assert!(
        esperou < Duration::from_millis(150),
        "org B esperou {esperou:?}: a saturação da org A vazou para ela"
    );

    let first = limiter
        .acquire_turn(org_a, agente, Duration::from_millis(50))
        .await
        .expect("org A pega a primeira vaga");
    let segunda = limiter
        .acquire_turn(org_a, agente, Duration::from_millis(50))
        .await;
    assert!(
        segunda.is_err(),
        "org A não pode ter duas vagas com limite 1"
    );
    drop(first);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cota_por_minuto_da_org_e_consumida_e_recusada() {
    let limiter = Limiter::new(
        LimitOptions {
            taxa_por_org_por_minuto: 2,
            ..LimitOptions::default()
        },
        MetricsMemory::own(),
    );
    let org = OrgId::generate();
    limiter.consumir_cota(org).expect("1ª mensagem");
    limiter.consumir_cota(org).expect("2ª mensagem");
    let terceira = limiter.consumir_cota(org);
    assert!(terceira.is_err(), "a cota da org devia ter acabado");
    assert!(terceira.expect_err("erro de cota").eh_transient());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cache_respeita_o_teto_de_memoria_com_dez_mil_agentes() {
    let mut scenario = Scenario::new("teto");
    scenario.criar_agente("teto-0");
    let cap = 1024 * 1024;
    let cache = ConfigHot::new(
        Arc::clone(&scenario.store),
        HotConfigOptions {
            validity: Duration::from_secs(600),
            cap_bytes: cap,
        },
        MetricsMemory::own(),
    );

    let mut criados = 0u64;
    let mut last = AgentId::generate();
    for index in 0..10_000u32 {
        let agente = scenario
            .store
            .agents()
            .create(
                scenario.org,
                NewAgent {
                    name: format!("Agente de carga {index}"),
                    slug: format!("carga-{index}"),
                    description: "config sintética para medir o teto do cache".repeat(4),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente da carga");
        cache.load(scenario.org, agente.id).expect("carrega");
        last = agente.id;
        criados += 1;
    }
    cache.synchronise();
    assert_eq!(criados, 10_000);
    assert!(
        cache.bytes() <= cap,
        "o cache passou do teto: {} bytes > {cap}",
        cache.bytes()
    );
    assert!(
        cache.size() < 10_000,
        "o cache guardou tudo ({} entradas): o teto não foi aplicado",
        cache.size()
    );
    assert!(cache.load(scenario.org, last).is_ok());
}

fn mensagem_de(texto: &str) -> Mensagem {
    Mensagem {
        id: MessageId::generate(),
        org_id: OrgId::generate(),
        conversation_id: ConversationId::generate(),
        direction: Direction::Inbound,
        role: Role::User,
        origin: poph_store::Origin::Agente,
        author_name: None,
        texto: texto.to_owned(),
        attachments_json: String::new(),
        external_id: None,
        turn_id: None,
        created_at: 0,
    }
}

#[test]
fn janela_respeita_o_orcamento_e_guarda_as_recentes() {
    let messages: Vec<Mensagem> = (1..=10)
        .map(|index| mensagem_de(&format!("{index:02}-{}", "x".repeat(397))))
        .collect();
    let ultimo_texto = messages.last().expect("tem mensagens").texto.clone();

    let window = build(messages, 300, 0);
    assert_eq!(window.messages.len(), 2);
    assert_eq!(window.discarded, 8);
    assert_eq!(window.messages.last().expect("última").texto, ultimo_texto);
    assert!(window.tokens_estimate <= 300, "estourou o orçamento");
    assert!(!window.last_truncate);
}

#[test]
fn janela_respeita_o_teto_de_quantidade_de_mensagens() {
    let messages: Vec<Mensagem> = (0..5).map(|_| mensagem_de("curta")).collect();
    let window = build(messages, 10_000, 2);
    assert_eq!(window.messages.len(), 2);
    assert_eq!(window.discarded, 3);
}

#[test]
fn mensagem_unica_maior_que_o_orcamento_e_cortada() {
    let messages = vec![mensagem_de(&"a".repeat(1000))];
    let window = build(messages, 50, 0);
    assert_eq!(window.messages.len(), 1);
    assert!(window.last_truncate);
    assert!(window.tokens_estimate <= 50);
    assert!(window.messages[0].texto.ends_with('…'));
    assert!(estimate_tokens(&window.messages[0].texto) <= 50);
}

#[test]
fn janela_vazia_nao_quebra() {
    let window = build(Vec::new(), 100, 10);
    assert!(window.messages.is_empty());
    assert_eq!(window.discarded, 0);
    assert_eq!(window.tokens_estimate, 0);
}

struct SemCusto;

impl Processor for SemCusto {
    fn process(&self, _tarefa: Task) -> BoxFuture {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "mede o custo; rodar com --ignored --nocapture"]
async fn custo_por_evento_e_por_leitura_de_cache() {
    const EVENTOS: usize = 8_000;
    let scenario = Scenario::new("custo");
    let actors = Actors::new(
        Arc::new(SemCusto),
        ActorOptions {
            ociosidade: Duration::from_secs(60),
            queue_by_conversation: EVENTOS,
            max_actors: 8,
        },
        MetricsMemory::own(),
    );
    let beginning = Instant::now();
    for _ in 0..EVENTOS {
        actors
            .send(Task {
                org: scenario.org,
                conversa: scenario.conversa,
                mensagem: MessageId::generate(),
                entry: None,
            })
            .expect("enfileira");
    }
    let enqueue = beginning.elapsed();
    assert!(actors.wait_queues_empty(Duration::from_secs(60)).await);
    let total = beginning.elapsed();
    println!(
        "ator: {EVENTOS} eventos; enfileirar {:.3} µs/evento; laço completo {:.3} µs/evento",
        enqueue.as_secs_f64() * 1e6 / EVENTOS as f64,
        total.as_secs_f64() * 1e6 / EVENTOS as f64
    );

    let mut cenario_cache = Scenario::new("custo-cache");
    let agente = cenario_cache.criar_agente("cache");
    let cache = ConfigHot::new(
        Arc::clone(&cenario_cache.store),
        HotConfigOptions::default(),
        MetricsMemory::own(),
    );
    cache.load(cenario_cache.org, agente).expect("aquecer");
    let leituras = 50_000;
    let beginning = Instant::now();
    for _ in 0..leituras {
        cache
            .load(cenario_cache.org, agente)
            .expect("leitura quente");
    }
    let por_leitura = beginning.elapsed().as_secs_f64() * 1e6 / leituras as f64;
    println!("cache de config quente: {por_leitura:.3} µs por leitura (acerto)");
}
