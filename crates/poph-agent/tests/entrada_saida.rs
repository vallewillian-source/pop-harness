use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use poph_agent::actors::{ActorOptions, Actors, BoxFuture, Processor, Task};
use poph_agent::entry_output::{
    Backoff, FlowOptions, Fluxo, InputEvent, OutputTransport, ReceivedMessage, SendFailure,
    SendFuture,
};
use poph_agent::metrics::MetricsMemory;
use poph_store::{
    Direction, NewAgent, NewConversation, NewInboxItem, NewMessage, NewOrg, NewOutboxItem,
    OutboxItem, OutboxState, Role, Store,
};
use poph_types::{ConversationId, IntegrationId, MessageId, OrgId, TurnId};

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    integracao: IntegrationId,
    conversa: ConversationId,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let folder = std::env::temp_dir().join(format!("poph-hr12-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("cria a pasta do teste");
        let store = Arc::new(
            Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
                .expect("abre o banco"),
        );
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Org HR-12".to_owned(),
                slug: format!("org-hr12-{label}"),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: format!("Agente {label}"),
                    slug: format!("agente-{label}"),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente");
        let conversa = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente.id,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("abre a conversa");
        Self {
            folder,
            store,
            org,
            integracao: IntegrationId::generate(),
            conversa: conversa.id,
        }
    }

    fn reopen(&mut self) {
        self.store = Arc::new(
            Store::open_at(&self.folder.join("harness.db"), &self.folder.join("rag.db"))
                .expect("reabre o banco"),
        );
    }

    fn evento(&self, mensagem: MessageId) -> String {
        InputEvent {
            conversa: self.conversa,
            mensagem,
        }
        .for_json()
        .expect("serializa o evento")
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

struct ProcessorTest {
    registro: Arc<Registro>,
    fluxo: Arc<OnceLock<Fluxo>>,
}

#[derive(Default)]
struct Registro {
    arrived: Mutex<Vec<Task>>,
    wait: Duration,
}

impl Registro {
    fn quantity(&self) -> usize {
        self.arrived.lock().expect("sem veneno").len()
    }
}

impl Processor for ProcessorTest {
    fn process(&self, task: Task) -> BoxFuture {
        let registro = Arc::clone(&self.registro);
        let fluxo = Arc::clone(&self.fluxo);
        Box::pin(async move {
            if !registro.wait.is_zero() {
                tokio::time::sleep(registro.wait).await;
            }
            registro.arrived.lock().expect("sem veneno").push(task);
            if let (Some(entry), Some(fluxo)) = (task.entry, fluxo.get()) {
                fluxo.concluir_entry(task.org, entry, None)?;
            }
            Ok(())
        })
    }
}

struct NoProcess;

impl Processor for NoProcess {
    fn process(&self, _task: Task) -> BoxFuture {
        Box::pin(async { Ok(()) })
    }
}

fn build_flow(
    scenario: &Scenario,
    options: FlowOptions,
    options_actors: ActorOptions,
    wait: Duration,
) -> (Fluxo, Actors, Arc<Registro>) {
    let registro = Arc::new(Registro {
        arrived: Mutex::new(Vec::new()),
        wait,
    });
    let flow_stored: Arc<OnceLock<Fluxo>> = Arc::new(OnceLock::new());
    let actors = Actors::new(
        Arc::new(ProcessorTest {
            registro: Arc::clone(&registro),
            fluxo: Arc::clone(&flow_stored),
        }),
        options_actors,
        MetricsMemory::own(),
    );
    let fluxo = Fluxo::new(Arc::clone(&scenario.store), actors.clone(), options);
    let _ = flow_stored.set(fluxo.clone());
    (fluxo, actors, registro)
}

fn options_fast() -> FlowOptions {
    FlowOptions {
        backoff: Backoff {
            base: Duration::ZERO,
            factor: 2,
            cap: Duration::ZERO,
        },
        ..FlowOptions::default()
    }
}

fn options_actors(cap_queue: usize) -> ActorOptions {
    ActorOptions {
        ociosidade: Duration::from_secs(60),
        queue_by_conversation: cap_queue,
        max_actors: 32,
    }
}

struct TransportTest {
    sends: AtomicUsize,
    received: Mutex<Vec<OutboxItem>>,
    failure: Mutex<Option<SendFailure>>,
    wait: Duration,
}

impl TransportTest {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            sends: AtomicUsize::new(0),
            received: Mutex::new(Vec::new()),
            failure: Mutex::new(None),
            wait: Duration::ZERO,
        })
    }

    fn with_wait(wait: Duration) -> Arc<Self> {
        let mut transport = Self::new();
        Arc::get_mut(&mut transport).expect("recém-criado").wait = wait;
        transport
    }

    fn with_failure(failure: SendFailure) -> Arc<Self> {
        let transport = Self::new();
        *transport.failure.lock().expect("sem veneno") = Some(failure);
        transport
    }

    fn sends(&self) -> usize {
        self.sends.load(Ordering::SeqCst)
    }
}

impl OutputTransport for TransportTest {
    fn send(&self, item: OutboxItem) -> SendFuture {
        self.sends.fetch_add(1, Ordering::SeqCst);
        self.received.lock().expect("sem veneno").push(item);
        let failure = self.failure.lock().expect("sem veneno").clone();
        let wait = self.wait;
        Box::pin(async move {
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
            match failure {
                Some(failure) => Err(failure),
                None => Ok(()),
            }
        })
    }
}

fn enqueue_text(fluxo: &Fluxo, scenario: &Scenario, turno: TurnId, seq: u32) -> OutboxItem {
    let (_new, item) = fluxo
        .enqueue_output(
            scenario.org,
            NewOutboxItem {
                conversation_id: scenario.conversa,
                turn_id: turno,
                seq,
                tipo: "text".to_owned(),
                content_json: format!("{{\"texto\":\"resposta {seq}\"}}"),
            },
        )
        .expect("enfileira a saída");
    item
}

fn write_message(scenario: &Scenario, turno: Option<TurnId>) -> MessageId {
    scenario
        .store
        .messages()
        .write(
            scenario.org,
            NewMessage {
                conversation_id: scenario.conversa,
                direction: Direction::Inbound,
                role: Role::User,
                texto: "oi".to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: turno,
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
        )
        .expect("grava a mensagem")
        .id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_message_delivered_two_times_processed_once() {
    let scenario = Scenario::new("duplicada");
    let (fluxo, actors, registro) =
        build_flow(&scenario, options_fast(), options_actors(8), Duration::ZERO);
    let mensagem = write_message(&scenario, None);

    let first = fluxo
        .receive_message(
            scenario.org,
            ReceivedMessage {
                integration_id: scenario.integracao,
                id_externo: "wamid-1".to_owned(),
                conversa: scenario.conversa,
                texto: "oi".to_owned(),
                attachments_json: "[]".to_owned(),
            },
        )
        .expect("primeira entrega");
    assert_eq!(
        first.situation,
        poph_store::Received::New,
        "a primeira entrega é nova"
    );

    let second_one = fluxo
        .receive_message(
            scenario.org,
            ReceivedMessage {
                integration_id: scenario.integracao,
                id_externo: "wamid-1".to_owned(),
                conversa: scenario.conversa,
                texto: "oi".to_owned(),
                attachments_json: "[]".to_owned(),
            },
        )
        .expect("reentrega não é erro");
    assert_eq!(
        second_one.situation,
        poph_store::Received::Duplicate,
        "a segunda entrega é descartada como duplicada"
    );
    assert_eq!(
        second_one.item.id, first.item.id,
        "as duas entregas apontam para o mesmo item da inbox"
    );
    assert_eq!(
        second_one.mensagem.id, first.mensagem.id,
        "e para a mesma mensagem"
    );

    let raw = fluxo
        .receive(
            scenario.org,
            NewInboxItem {
                integration_id: scenario.integracao,
                external_id: "wamid-1".to_owned(),
                event_json: scenario.evento(mensagem),
            },
        )
        .expect("entrega crua");
    assert_eq!(raw, poph_store::Received::Duplicate);

    for _ in 0..3 {
        fluxo.dispatch(scenario.org).expect("despacha");
        assert!(actors.wait_queues_empty(Duration::from_secs(5)).await);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert_eq!(registro.quantity(), 1, "um único processamento");
    assert_eq!(
        fluxo
            .statistics(scenario.org)
            .expect("estatísticas")
            .entry_pending,
        0,
        "o item saiu da fila depois de concluído"
    );
    assert_eq!(
        fluxo.metrics().entries_duplicated.value(),
        2,
        "as duas reentregas contam como duplicadas"
    );
}

#[test]
fn child_records_entry_locks() {
    let Ok(label) = std::env::var("POPH_HR12_FILHO") else {
        return;
    };
    if label != "entrada" {
        return;
    }
    let folder = PathBuf::from(std::env::var("POPH_HR12_PASTA").expect("pasta no ambiente"));
    let marker = PathBuf::from(std::env::var("POPH_HR12_MARCADOR").expect("marcador"));
    let org: OrgId = std::env::var("POPH_HR12_ORG")
        .expect("org")
        .parse()
        .expect("org válida");
    let conversa: ConversationId = std::env::var("POPH_HR12_CONVERSA")
        .expect("conversa")
        .parse()
        .expect("conversa válida");
    let integracao: IntegrationId = std::env::var("POPH_HR12_INTEGRACAO")
        .expect("integração")
        .parse()
        .expect("integração válida");

    let store = Arc::new(
        Store::open_at(&folder.join("harness.db"), &folder.join("rag.db")).expect("abre o banco"),
    );
    let actors = Actors::new(Arc::new(NoProcess), options_actors(8), MetricsMemory::own());
    let fluxo = Fluxo::new(store, actors, options_fast());
    let stored = fluxo
        .receive_message(
            org,
            ReceivedMessage {
                integration_id: integracao,
                id_externo: "wamid-sigkill".to_owned(),
                conversa,
                texto: "mensagem que não pode se perder".to_owned(),
                attachments_json: "[]".to_owned(),
            },
        )
        .expect("grava a entrada");
    assert_eq!(stored.situation, poph_store::Received::New);

    std::fs::write(&marker, "gravado").expect("escreve o marcador");
    std::thread::sleep(Duration::from_secs(30));
    panic!("o pai devia ter matado o filho antes disso");
}

#[test]
fn sigkill_between_write_process_not_loses_message() {
    let mut scenario = Scenario::new("sigkill-entrada");
    let org = scenario.org;
    let conversa = scenario.conversa;
    let integracao = scenario.integracao;
    let folder = scenario.folder.clone();
    let marker = folder.join("marcador-entrada");
    scenario.store = Arc::new(
        Store::open_at(&folder.join("harness.db"), &folder.join("rag.db")).expect("reabre"),
    );

    let child = std::process::Command::new(std::env::current_exe().expect("binário de teste"))
        .args([
            "--exact",
            "child_records_entry_locks",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("POPH_HR12_FILHO", "entrada")
        .env("POPH_HR12_PASTA", &folder)
        .env("POPH_HR12_MARCADOR", &marker)
        .env("POPH_HR12_ORG", org.to_string())
        .env("POPH_HR12_CONVERSA", conversa.to_string())
        .env("POPH_HR12_INTEGRACAO", integracao.to_string())
        .spawn()
        .expect("inicia o filho");

    wait_marker(&marker);
    kill_child(child);

    scenario.reopen();
    assert_eq!(
        scenario.store.inbox().count_pending(org).expect("conta"),
        1,
        "a entrada pendente sobreviveu à queda"
    );
    let pending = scenario.store.inbox().list_pending(org, 10).expect("lista");
    assert_eq!(pending[0].external_id, "wamid-sigkill");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (fluxo, actors, registro) =
        build_flow(&scenario, options_fast(), options_actors(8), Duration::ZERO);
    let transport = TransportTest::new();
    let resumed = runtime
        .block_on(async {
            let resumed = fluxo.retomar(transport.as_ref()).await?;
            assert!(actors.wait_queues_empty(Duration::from_secs(10)).await);
            Ok::<_, poph_agent::AgentError>(resumed)
        })
        .expect("retomada");

    assert_eq!(
        resumed.entry.dispatch, 1,
        "a retomada despachou o item pendente"
    );
    assert_eq!(registro.quantity(), 1, "processado uma vez na retomada");
    assert_eq!(
        scenario.store.inbox().count_pending(org).expect("conta"),
        0,
        "depois de processado, a fila fica vazia"
    );
    let messages = scenario
        .store
        .messages()
        .list(org, conversa, poph_store::Page::with_limit(10))
        .expect("lista mensagens");
    assert!(
        messages
            .iter()
            .any(|mensagem| mensagem.texto == "mensagem que não pode se perder"),
        "a mensagem gravada na mesma transação continua lá"
    );
}

#[test]
fn child_stays_sending_locks() {
    let Ok(label) = std::env::var("POPH_HR12_FILHO") else {
        return;
    };
    if label != "saida" {
        return;
    }
    let folder = PathBuf::from(std::env::var("POPH_HR12_PASTA").expect("pasta"));
    let marker = PathBuf::from(std::env::var("POPH_HR12_MARCADOR").expect("marcador"));
    let org: OrgId = std::env::var("POPH_HR12_ORG")
        .expect("org")
        .parse()
        .expect("org");
    let conversa: ConversationId = std::env::var("POPH_HR12_CONVERSA")
        .expect("conversa")
        .parse()
        .expect("conversa");
    let turno: TurnId = std::env::var("POPH_HR12_TURNO")
        .expect("turno")
        .parse()
        .expect("turno");

    let store =
        Arc::new(Store::open_at(&folder.join("harness.db"), &folder.join("rag.db")).expect("abre"));
    let (_new, item) = store
        .outbox()
        .enqueue(
            org,
            NewOutboxItem {
                conversation_id: conversa,
                turn_id: turno,
                seq: 0,
                tipo: "text".to_owned(),
                content_json: "{\"texto\":\"resposta\"}".to_owned(),
            },
        )
        .expect("enfileira");
    store
        .outbox()
        .start_send(org, item.id)
        .expect("entra em sending");
    std::fs::write(&marker, "sending").expect("escreve o marcador");
    std::thread::sleep(Duration::from_secs(30));
    panic!("o pai devia ter matado o filho antes disso");
}

#[test]
fn sigkill_between_sending_sent_resends_once() {
    let mut scenario = Scenario::new("sigkill-saida");
    let org = scenario.org;
    let conversa = scenario.conversa;
    let turno = TurnId::generate();
    let folder = scenario.folder.clone();
    let marker = folder.join("marcador-saida");
    scenario.store = Arc::new(
        Store::open_at(&folder.join("harness.db"), &folder.join("rag.db")).expect("reabre"),
    );

    let child = std::process::Command::new(std::env::current_exe().expect("binário de teste"))
        .args([
            "--exact",
            "child_stays_sending_locks",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("POPH_HR12_FILHO", "saida")
        .env("POPH_HR12_PASTA", &folder)
        .env("POPH_HR12_MARCADOR", &marker)
        .env("POPH_HR12_ORG", org.to_string())
        .env("POPH_HR12_CONVERSA", conversa.to_string())
        .env("POPH_HR12_TURNO", turno.to_string())
        .spawn()
        .expect("inicia o filho");

    wait_marker(&marker);
    kill_child(child);

    scenario.reopen();
    assert_eq!(
        scenario
            .store
            .outbox()
            .count_by_state(org, OutboxState::Sending)
            .expect("conta"),
        1,
        "o item ficou em sending depois da queda"
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (fluxo, actors, _record) =
        build_flow(&scenario, options_fast(), options_actors(8), Duration::ZERO);
    let transport = TransportTest::new();
    let resumed = runtime
        .block_on(fluxo.retomar(transport.as_ref()))
        .expect("retomada");

    assert_eq!(resumed.resends, 1, "um item estava em sending");
    assert_eq!(transport.sends(), 1, "reenviou exatamente uma vez");
    assert_eq!(
        fluxo.metrics().resends_resumption.value(),
        1,
        "a métrica de reenvio conta a retomada"
    );
    let pending = scenario
        .store
        .outbox()
        .list_pending(org, 10)
        .expect("lista");
    assert!(pending.is_empty(), "nada sobrou na fila de saída");
    let items = scenario
        .store
        .outbox()
        .count_by_state(org, OutboxState::Sent)
        .expect("conta");
    assert_eq!(items, 1, "e o item terminou como enviado");

    let second_one = runtime
        .block_on(fluxo.retomar(transport.as_ref()))
        .expect("segunda retomada");
    assert_eq!(second_one.resends, 0);
    assert_eq!(transport.sends(), 1, "nada duplicado na segunda subida");
    drop(actors);
}

#[tokio::test]
async fn failure_repeated_goes_for_failed_with_reason_counter() {
    let scenario = Scenario::new("falha-repetida");
    let (fluxo, _actors, _record) = build_flow(
        &scenario,
        FlowOptions {
            max_attempts: 3,
            ..options_fast()
        },
        options_actors(8),
        Duration::ZERO,
    );
    let transport = TransportTest::with_failure(SendFailure::transient("net::ERR_CONNECTION"));
    let item = enqueue_text(&fluxo, &scenario, TurnId::generate(), 0);

    for _ in 0..4 {
        fluxo
            .send_pending(scenario.org, transport.as_ref())
            .await
            .expect("envia");
    }

    let stored = scenario
        .store
        .outbox()
        .get(scenario.org, item.id)
        .expect("busca");
    assert_eq!(stored.state, OutboxState::Failed);
    assert_eq!(
        stored.attempts, 3,
        "três tentativas reais antes de desistir"
    );
    assert_eq!(
        stored.ultimo_erro.as_deref(),
        Some("tentativas de envio esgotadas"),
        "o motivo fica legível no banco"
    );
    assert_eq!(transport.sends(), 3, "não tentou depois de esgotar");
    assert_eq!(fluxo.metrics().outputs_transient.value(), 3);
    assert_eq!(fluxo.metrics().outputs_exhausted.value(), 1);
}

#[tokio::test]
async fn failure_definitive_not_retries() {
    let scenario = Scenario::new("falha-definitiva");
    let (fluxo, _actors, _record) =
        build_flow(&scenario, options_fast(), options_actors(8), Duration::ZERO);
    let transport = TransportTest::with_failure(SendFailure::definitive("destinatário inválido"));
    let item = enqueue_text(&fluxo, &scenario, TurnId::generate(), 0);

    fluxo
        .send_pending(scenario.org, transport.as_ref())
        .await
        .expect("envia");
    fluxo
        .send_pending(scenario.org, transport.as_ref())
        .await
        .expect("envia de novo");

    let stored = scenario
        .store
        .outbox()
        .get(scenario.org, item.id)
        .expect("busca");
    assert_eq!(stored.state, OutboxState::Failed);
    assert_eq!(stored.ultimo_erro.as_deref(), Some("destinatário inválido"));
    assert_eq!(stored.attempts, 1, "erro definitivo não repete");
    assert_eq!(transport.sends(), 1);
    assert_eq!(fluxo.metrics().outputs_definitive.value(), 1);
}

#[tokio::test]
async fn output_idempotent_by_turn_sequence() {
    let scenario = Scenario::new("saida-idempotente");
    let (fluxo, _actors, _record) =
        build_flow(&scenario, options_fast(), options_actors(8), Duration::ZERO);
    let turno = TurnId::generate();
    let first = enqueue_text(&fluxo, &scenario, turno, 0);
    let (_new, second_one) = fluxo
        .enqueue_output(
            scenario.org,
            NewOutboxItem {
                conversation_id: scenario.conversa,
                turn_id: turno,
                seq: 0,
                tipo: "text".to_owned(),
                content_json: "{\"texto\":\"outro\"}".to_owned(),
            },
        )
        .expect("enfileira de novo");
    assert_eq!(first.id, second_one.id, "a mesma chave não duplica a linha");
    assert_eq!(
        second_one.content_json, "{\"texto\":\"resposta 0\"}",
        "o conteúdo original é preservado"
    );
}

#[test]
fn backoff_grows_for_cap() {
    let backoff = Backoff {
        base: Duration::from_millis(100),
        factor: 2,
        cap: Duration::from_secs(1),
    };
    let waits: Vec<u128> = (1..=8)
        .map(|attempt| backoff.wait(attempt).as_millis())
        .collect();
    assert_eq!(waits, vec![100, 200, 400, 800, 1000, 1000, 1000, 1000]);
    assert!(
        waits.windows(2).all(|pair| pair[0] <= pair[1]),
        "nunca diminui"
    );
    assert_eq!(
        backoff.wait(0),
        Duration::from_millis(100),
        "tentativa 0 = base"
    );
}

#[tokio::test]
async fn item_backoff_not_resent_before_hour() {
    let scenario = Scenario::new("backoff-virtual");
    let clock = Arc::new(Mutex::new(Instant::now()));
    let clock_for_flow = Arc::clone(&clock);
    let (fluxo, _actors, _record) = build_flow(
        &scenario,
        FlowOptions {
            backoff: Backoff {
                base: Duration::from_secs(2),
                factor: 2,
                cap: Duration::from_secs(30),
            },
            ..FlowOptions::default()
        },
        options_actors(8),
        Duration::ZERO,
    );
    fluxo.with_clock(Arc::new(move || {
        *clock_for_flow.lock().expect("sem veneno")
    }));

    let transport = TransportTest::with_failure(SendFailure::transient("429"));
    enqueue_text(&fluxo, &scenario, TurnId::generate(), 0);

    let first = fluxo
        .send_pending(scenario.org, transport.as_ref())
        .await
        .expect("envia");
    assert_eq!(first.failed, 1);
    assert_eq!(transport.sends(), 1);

    let deferred = fluxo
        .send_pending(scenario.org, transport.as_ref())
        .await
        .expect("envia");
    assert_eq!(deferred.deferred, 1, "ainda dentro do backoff");
    assert_eq!(transport.sends(), 1, "não chamou o canal de novo");

    *clock.lock().expect("sem veneno") += Duration::from_secs(2);
    let second = fluxo
        .send_pending(scenario.org, transport.as_ref())
        .await
        .expect("envia");
    assert_eq!(second.failed, 1);
    assert_eq!(transport.sends(), 2, "repetiu depois da espera");

    *clock.lock().expect("sem veneno") += Duration::from_secs(3);
    let ainda = fluxo
        .send_pending(scenario.org, transport.as_ref())
        .await
        .expect("envia");
    assert_eq!(ainda.deferred, 1, "a espera dobrou");
    assert_eq!(transport.sends(), 2);
}

#[tokio::test]
async fn boxes_not_cross_org() {
    let scenario = Scenario::new("isolamento");
    let other = scenario
        .store
        .orgs()
        .create(NewOrg {
            name: "Outra org".to_owned(),
            slug: "outra-org-hr12".to_owned(),
            config_json: String::new(),
        })
        .expect("cria a outra org")
        .id;
    let (fluxo, _actors, _record) =
        build_flow(&scenario, options_fast(), options_actors(8), Duration::ZERO);

    let incoming = fluxo
        .receive_message(
            scenario.org,
            ReceivedMessage {
                integration_id: scenario.integracao,
                id_externo: "wamid-org-a".to_owned(),
                conversa: scenario.conversa,
                texto: "da org A".to_owned(),
                attachments_json: "[]".to_owned(),
            },
        )
        .expect("grava na org A");
    let saida = enqueue_text(&fluxo, &scenario, TurnId::generate(), 0);

    assert_eq!(
        scenario.store.inbox().count_pending(other).expect("conta"),
        0,
        "a org B não vê a entrada da org A"
    );
    assert!(
        scenario
            .store
            .inbox()
            .list_pending(other, 10)
            .expect("lista")
            .is_empty()
    );
    assert!(
        scenario.store.inbox().get(other, incoming.item.id).is_err(),
        "buscar o item da org A com a org B falha"
    );
    assert!(
        scenario
            .store
            .outbox()
            .list_pending(other, 10)
            .expect("lista")
            .is_empty()
    );
    assert!(
        scenario.store.outbox().get(other, saida.id).is_err(),
        "buscar a saída da org A com a org B falha"
    );
    assert!(
        scenario
            .store
            .outbox()
            .count_by_state(other, OutboxState::Pending)
            .expect("conta")
            == 0
    );
    assert!(
        scenario.store.outbox().start_send(other, saida.id).is_err(),
        "não dá para mexer no estado da saída de outra org"
    );
    assert_eq!(
        scenario
            .store
            .outbox()
            .get(scenario.org, saida.id)
            .expect("na org certa, existe")
            .state,
        OutboxState::Pending,
        "a tentativa da org errada não mudou nada"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_full_postpones_not_loses_event() {
    let scenario = Scenario::new("fila-cheia");
    let (fluxo, actors, registro) = build_flow(
        &scenario,
        options_fast(),
        options_actors(1),
        Duration::from_millis(120),
    );

    let mut items = HashMap::new();
    for index in 0..3 {
        let incoming = fluxo
            .receive_message(
                scenario.org,
                ReceivedMessage {
                    integration_id: scenario.integracao,
                    id_externo: format!("wamid-fila-{index}"),
                    conversa: scenario.conversa,
                    texto: format!("mensagem {index}"),
                    attachments_json: "[]".to_owned(),
                },
            )
            .expect("grava");
        items.insert(index, incoming.item.id);
    }

    let first = fluxo.dispatch(scenario.org).expect("despacha");
    assert_eq!(
        first.dispatch + first.deferred,
        3,
        "todos os eventos foram vistos no despacho"
    );
    assert!(first.dispatch >= 1, "ao menos um foi despachado");
    assert_eq!(
        scenario
            .store
            .inbox()
            .count_pending(scenario.org)
            .expect("conta"),
        3,
        "nada foi marcado como processado no despacho"
    );

    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(60)).await;
        fluxo.dispatch(scenario.org).expect("despacha");
        if registro.quantity() == 3 {
            break;
        }
    }
    assert!(actors.wait_queues_empty(Duration::from_secs(10)).await);
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(
        registro.quantity(),
        3,
        "as três mensagens foram processadas"
    );
    assert_eq!(
        scenario
            .store
            .inbox()
            .count_pending(scenario.org)
            .expect("conta"),
        0
    );
    if first.deferred > 0 {
        assert!(
            fluxo.metrics().entries_deferred.value() >= 1,
            "evento adiado conta na métrica"
        );
    }
    let _ = items;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_read_before_completion_not_repeats_item_processed() {
    let scenario = Scenario::new("lista-antiga");
    let (fluxo, actors, registro) =
        build_flow(&scenario, options_fast(), options_actors(4), Duration::ZERO);
    fluxo
        .receive_message(
            scenario.org,
            ReceivedMessage {
                integration_id: scenario.integracao,
                id_externo: "wamid-lista-antiga".to_owned(),
                conversa: scenario.conversa,
                texto: "mensagem única".to_owned(),
                attachments_json: "[]".to_owned(),
            },
        )
        .expect("grava");

    let previous = scenario
        .store
        .inbox()
        .list_pending(scenario.org, 10)
        .expect("lista");
    assert_eq!(previous.len(), 1, "a lista antiga traz o item");

    let first = fluxo.dispatch(scenario.org).expect("despacha");
    assert_eq!(first.dispatch, 1);
    assert!(actors.wait_queues_empty(Duration::from_secs(10)).await);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(registro.quantity(), 1, "o turno rodou uma vez");
    assert_eq!(
        scenario
            .store
            .inbox()
            .count_pending(scenario.org)
            .expect("conta"),
        0,
        "o item já está processado no banco"
    );

    let report = fluxo
        .dispatch_items(scenario.org, previous)
        .expect("despacha a lista antiga");
    assert_eq!(
        report.dispatch, 0,
        "item já processado não pode ser despachado de novo"
    );
    assert!(actors.wait_queues_empty(Duration::from_secs(10)).await);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        registro.quantity(),
        1,
        "o turno não pode rodar duas vezes para o mesmo item"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_dispatches_concurrent_not_duplicate_event() {
    let scenario = Scenario::new("concorrencia-entrada");
    let (fluxo, actors, registro) = build_flow(
        &scenario,
        options_fast(),
        options_actors(64),
        Duration::ZERO,
    );
    for index in 0..6 {
        fluxo
            .receive_message(
                scenario.org,
                ReceivedMessage {
                    integration_id: scenario.integracao,
                    id_externo: format!("wamid-concorrente-{index}"),
                    conversa: scenario.conversa,
                    texto: format!("concorrente {index}"),
                    attachments_json: "[]".to_owned(),
                },
            )
            .expect("grava");
    }

    let handle = tokio::runtime::Handle::current();
    let results: Vec<poph_agent::entry_output::DispatchReport> = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..2 {
            let fluxo = fluxo.clone();
            let org = scenario.org;
            let handle = handle.clone();
            handles.push(scope.spawn(move || {
                let _context = handle.enter();
                fluxo.dispatch(org).expect("despacha")
            }));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().expect("thread do despacho"))
            .collect()
    });

    let dispatch: u32 = results.iter().map(|report| report.dispatch).sum();
    assert_eq!(dispatch, 6, "cada item foi reservado uma única vez");
    assert!(actors.wait_queues_empty(Duration::from_secs(10)).await);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(registro.quantity(), 6, "seis processamentos, nem um a mais");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_sends_concurrent_not_duplicate_output() {
    let scenario = Scenario::new("concorrencia-saida");
    let (fluxo, _actors, _record) =
        build_flow(&scenario, options_fast(), options_actors(8), Duration::ZERO);
    for _ in 0..4 {
        enqueue_text(&fluxo, &scenario, TurnId::generate(), 0);
    }
    let transport = TransportTest::with_wait(Duration::from_millis(40));

    let (first, second) = tokio::join!(
        fluxo.send_pending(scenario.org, transport.as_ref()),
        fluxo.send_pending(scenario.org, transport.as_ref()),
    );
    let first = first.expect("envia");
    let second = second.expect("envia");

    assert_eq!(transport.sends(), 4, "cada item foi ao canal uma vez só");
    assert_eq!(
        first.sent + second.sent,
        4,
        "as duas passagens somam os quatro envios"
    );
    assert_eq!(
        fluxo
            .statistics(scenario.org)
            .expect("estatísticas")
            .output_pending,
        0
    );
}

fn wait_marker(marker: &Path) {
    let beginning = Instant::now();
    while beginning.elapsed() < Duration::from_secs(30) {
        if marker.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("o filho não escreveu o marcador em {} s", 30);
}

fn kill_child(mut child: std::process::Child) {
    child.kill().expect("SIGKILL no filho");
    let saida = child.wait().expect("espera o filho");
    assert!(
        !saida.success(),
        "o filho devia ter morrido por sinal, não saído com sucesso"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "medição, não checagem"]
async fn measure_ingestion() {
    const THREADS: usize = 8;
    const BY_THREAD: usize = 1500;

    let scenario = Scenario::new("medir-ingestao");
    let (fluxo, _actors, _record) =
        build_flow(&scenario, options_fast(), options_actors(8), Duration::ZERO);

    let beginning = Instant::now();
    let mut latencies: Vec<u64> = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for thread in 0..THREADS {
            let fluxo = fluxo.clone();
            let org = scenario.org;
            let integracao = scenario.integracao;
            let conversa = scenario.conversa;
            handles.push(scope.spawn(move || {
                let mut micros = Vec::with_capacity(BY_THREAD);
                for index in 0..BY_THREAD {
                    let id = format!("wamid-medir-{thread}-{index}");
                    let comeco = Instant::now();
                    fluxo
                        .receive_message(
                            org,
                            ReceivedMessage {
                                integration_id: integracao,
                                id_externo: id,
                                conversa,
                                texto: "mensagem de medição".to_owned(),
                                attachments_json: "[]".to_owned(),
                            },
                        )
                        .expect("ingere");
                    micros.push(u64::try_from(comeco.elapsed().as_micros()).unwrap_or(u64::MAX));
                }
                micros
            }));
        }
        let mut all = Vec::with_capacity(THREADS * BY_THREAD);
        for handle in handles {
            all.extend(handle.join().expect("thread de medição"));
        }
        all
    });
    let total = beginning.elapsed();
    latencies.sort_unstable();
    let p = |fracao: f64| {
        latencies[((latencies.len() as f64 * fracao) as usize).min(latencies.len() - 1)]
    };
    let quantity = latencies.len();
    println!(
        "ingestão atômica: {quantity} mensagens em {:.2} s = {:.0} msg/s; ACK p50 {} µs, p95 {} µs, p99 {} µs",
        total.as_secs_f64(),
        quantity as f64 / total.as_secs_f64(),
        p(0.50),
        p(0.95),
        p(0.99)
    );
    assert_eq!(
        scenario
            .store
            .inbox()
            .count_pending(scenario.org)
            .expect("conta"),
        (THREADS * BY_THREAD) as u64,
        "todas as mensagens ficaram na fila, nenhuma duplicada nem perdida"
    );
}
