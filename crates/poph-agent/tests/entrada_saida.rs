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

    fn reabrir(&mut self) {
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

struct ProcessadorDeTeste {
    registro: Arc<Registro>,
    fluxo: Arc<OnceLock<Fluxo>>,
}

#[derive(Default)]
struct Registro {
    recebidas: Mutex<Vec<Task>>,
    wait: Duration,
}

impl Registro {
    fn quantity(&self) -> usize {
        self.recebidas.lock().expect("sem veneno").len()
    }
}

impl Processor for ProcessadorDeTeste {
    fn process(&self, task: Task) -> BoxFuture {
        let registro = Arc::clone(&self.registro);
        let fluxo = Arc::clone(&self.fluxo);
        Box::pin(async move {
            if !registro.wait.is_zero() {
                tokio::time::sleep(registro.wait).await;
            }
            registro.recebidas.lock().expect("sem veneno").push(task);
            if let (Some(entry), Some(fluxo)) = (task.entry, fluxo.get()) {
                fluxo.concluir_entry(task.org, entry, None)?;
            }
            Ok(())
        })
    }
}

struct SemProcessar;

impl Processor for SemProcessar {
    fn process(&self, _tarefa: Task) -> BoxFuture {
        Box::pin(async { Ok(()) })
    }
}

fn montar_fluxo(
    scenario: &Scenario,
    options: FlowOptions,
    opcoes_atores: ActorOptions,
    wait: Duration,
) -> (Fluxo, Actors, Arc<Registro>) {
    let registro = Arc::new(Registro {
        recebidas: Mutex::new(Vec::new()),
        wait,
    });
    let fluxo_guardado: Arc<OnceLock<Fluxo>> = Arc::new(OnceLock::new());
    let actors = Actors::new(
        Arc::new(ProcessadorDeTeste {
            registro: Arc::clone(&registro),
            fluxo: Arc::clone(&fluxo_guardado),
        }),
        opcoes_atores,
        MetricsMemory::own(),
    );
    let fluxo = Fluxo::new(Arc::clone(&scenario.store), actors.clone(), options);
    let _ = fluxo_guardado.set(fluxo.clone());
    (fluxo, actors, registro)
}

fn opcoes_rapidas() -> FlowOptions {
    FlowOptions {
        backoff: Backoff {
            base: Duration::ZERO,
            factor: 2,
            cap: Duration::ZERO,
        },
        ..FlowOptions::default()
    }
}

fn opcoes_atores(teto_fila: usize) -> ActorOptions {
    ActorOptions {
        ociosidade: Duration::from_secs(60),
        queue_by_conversation: teto_fila,
        max_actors: 32,
    }
}

struct TransporteDeTeste {
    envios: AtomicUsize,
    received: Mutex<Vec<OutboxItem>>,
    failure: Mutex<Option<SendFailure>>,
    wait: Duration,
}

impl TransporteDeTeste {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            envios: AtomicUsize::new(0),
            received: Mutex::new(Vec::new()),
            failure: Mutex::new(None),
            wait: Duration::ZERO,
        })
    }

    fn com_espera(wait: Duration) -> Arc<Self> {
        let mut transport = Self::new();
        Arc::get_mut(&mut transport).expect("recém-criado").wait = wait;
        transport
    }

    fn com_falha(failure: SendFailure) -> Arc<Self> {
        let transport = Self::new();
        *transport.failure.lock().expect("sem veneno") = Some(failure);
        transport
    }

    fn envios(&self) -> usize {
        self.envios.load(Ordering::SeqCst)
    }
}

impl OutputTransport for TransporteDeTeste {
    fn send(&self, item: OutboxItem) -> SendFuture {
        self.envios.fetch_add(1, Ordering::SeqCst);
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

fn enfileirar_texto(fluxo: &Fluxo, scenario: &Scenario, turno: TurnId, seq: u32) -> OutboxItem {
    let (_novo, item) = fluxo
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

fn gravar_mensagem(scenario: &Scenario, turno: Option<TurnId>) -> MessageId {
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
async fn mesma_mensagem_entregue_duas_vezes_e_processada_uma_vez() {
    let scenario = Scenario::new("duplicada");
    let (fluxo, actors, registro) = montar_fluxo(
        &scenario,
        opcoes_rapidas(),
        opcoes_atores(8),
        Duration::ZERO,
    );
    let mensagem = gravar_mensagem(&scenario, None);

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

    let segunda = fluxo
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
        segunda.situation,
        poph_store::Received::Duplicate,
        "a segunda entrega é descartada como duplicada"
    );
    assert_eq!(
        segunda.item.id, first.item.id,
        "as duas entregas apontam para o mesmo item da inbox"
    );
    assert_eq!(
        segunda.mensagem.id, first.mensagem.id,
        "e para a mesma mensagem"
    );

    let cru = fluxo
        .receive(
            scenario.org,
            NewInboxItem {
                integration_id: scenario.integracao,
                external_id: "wamid-1".to_owned(),
                event_json: scenario.evento(mensagem),
            },
        )
        .expect("entrega crua");
    assert_eq!(cru, poph_store::Received::Duplicate);

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
fn filho_grava_entrada_e_trava() {
    let Ok(label) = std::env::var("POPH_HR12_FILHO") else {
        return;
    };
    if label != "entrada" {
        return;
    }
    let folder = PathBuf::from(std::env::var("POPH_HR12_PASTA").expect("pasta no ambiente"));
    let marcador = PathBuf::from(std::env::var("POPH_HR12_MARCADOR").expect("marcador"));
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
    let actors = Actors::new(
        Arc::new(SemProcessar),
        opcoes_atores(8),
        MetricsMemory::own(),
    );
    let fluxo = Fluxo::new(store, actors, opcoes_rapidas());
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

    std::fs::write(&marcador, "gravado").expect("escreve o marcador");
    std::thread::sleep(Duration::from_secs(30));
    panic!("o pai devia ter matado o filho antes disso");
}

#[test]
fn sigkill_entre_gravar_e_processar_nao_perde_a_mensagem() {
    let mut scenario = Scenario::new("sigkill-entrada");
    let org = scenario.org;
    let conversa = scenario.conversa;
    let integracao = scenario.integracao;
    let folder = scenario.folder.clone();
    let marcador = folder.join("marcador-entrada");
    scenario.store = Arc::new(
        Store::open_at(&folder.join("harness.db"), &folder.join("rag.db")).expect("reabre"),
    );

    let filho = std::process::Command::new(std::env::current_exe().expect("binário de teste"))
        .args([
            "--exact",
            "filho_grava_entrada_e_trava",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("POPH_HR12_FILHO", "entrada")
        .env("POPH_HR12_PASTA", &folder)
        .env("POPH_HR12_MARCADOR", &marcador)
        .env("POPH_HR12_ORG", org.to_string())
        .env("POPH_HR12_CONVERSA", conversa.to_string())
        .env("POPH_HR12_INTEGRACAO", integracao.to_string())
        .spawn()
        .expect("inicia o filho");

    esperar_marcador(&marcador);
    matar(filho);

    scenario.reabrir();
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
    let (fluxo, actors, registro) = montar_fluxo(
        &scenario,
        opcoes_rapidas(),
        opcoes_atores(8),
        Duration::ZERO,
    );
    let transport = TransporteDeTeste::new();
    let retomada = runtime
        .block_on(async {
            let retomada = fluxo.retomar(transport.as_ref()).await?;
            assert!(actors.wait_queues_empty(Duration::from_secs(10)).await);
            Ok::<_, poph_agent::AgentError>(retomada)
        })
        .expect("retomada");

    assert_eq!(
        retomada.entry.dispatch, 1,
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
fn filho_fica_em_sending_e_trava() {
    let Ok(label) = std::env::var("POPH_HR12_FILHO") else {
        return;
    };
    if label != "saida" {
        return;
    }
    let folder = PathBuf::from(std::env::var("POPH_HR12_PASTA").expect("pasta"));
    let marcador = PathBuf::from(std::env::var("POPH_HR12_MARCADOR").expect("marcador"));
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
    let (_novo, item) = store
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
    std::fs::write(&marcador, "sending").expect("escreve o marcador");
    std::thread::sleep(Duration::from_secs(30));
    panic!("o pai devia ter matado o filho antes disso");
}

#[test]
fn sigkill_entre_sending_e_sent_reenvia_uma_vez() {
    let mut scenario = Scenario::new("sigkill-saida");
    let org = scenario.org;
    let conversa = scenario.conversa;
    let turno = TurnId::generate();
    let folder = scenario.folder.clone();
    let marcador = folder.join("marcador-saida");
    scenario.store = Arc::new(
        Store::open_at(&folder.join("harness.db"), &folder.join("rag.db")).expect("reabre"),
    );

    let filho = std::process::Command::new(std::env::current_exe().expect("binário de teste"))
        .args([
            "--exact",
            "filho_fica_em_sending_e_trava",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("POPH_HR12_FILHO", "saida")
        .env("POPH_HR12_PASTA", &folder)
        .env("POPH_HR12_MARCADOR", &marcador)
        .env("POPH_HR12_ORG", org.to_string())
        .env("POPH_HR12_CONVERSA", conversa.to_string())
        .env("POPH_HR12_TURNO", turno.to_string())
        .spawn()
        .expect("inicia o filho");

    esperar_marcador(&marcador);
    matar(filho);

    scenario.reabrir();
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
    let (fluxo, actors, _registro) = montar_fluxo(
        &scenario,
        opcoes_rapidas(),
        opcoes_atores(8),
        Duration::ZERO,
    );
    let transport = TransporteDeTeste::new();
    let retomada = runtime
        .block_on(fluxo.retomar(transport.as_ref()))
        .expect("retomada");

    assert_eq!(retomada.resends, 1, "um item estava em sending");
    assert_eq!(transport.envios(), 1, "reenviou exatamente uma vez");
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

    let segunda = runtime
        .block_on(fluxo.retomar(transport.as_ref()))
        .expect("segunda retomada");
    assert_eq!(segunda.resends, 0);
    assert_eq!(transport.envios(), 1, "nada duplicado na segunda subida");
    drop(actors);
}

#[tokio::test]
async fn falha_repetida_vai_para_failed_com_motivo_e_contador() {
    let scenario = Scenario::new("falha-repetida");
    let (fluxo, _atores, _registro) = montar_fluxo(
        &scenario,
        FlowOptions {
            max_attempts: 3,
            ..opcoes_rapidas()
        },
        opcoes_atores(8),
        Duration::ZERO,
    );
    let transport = TransporteDeTeste::com_falha(SendFailure::transient("net::ERR_CONNECTION"));
    let item = enfileirar_texto(&fluxo, &scenario, TurnId::generate(), 0);

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
    assert_eq!(transport.envios(), 3, "não tentou depois de esgotar");
    assert_eq!(fluxo.metrics().outputs_transient.value(), 3);
    assert_eq!(fluxo.metrics().outputs_exhausted.value(), 1);
}

#[tokio::test]
async fn falha_definitiva_nao_retenta() {
    let scenario = Scenario::new("falha-definitiva");
    let (fluxo, _atores, _registro) = montar_fluxo(
        &scenario,
        opcoes_rapidas(),
        opcoes_atores(8),
        Duration::ZERO,
    );
    let transport = TransporteDeTeste::com_falha(SendFailure::definitive("destinatário inválido"));
    let item = enfileirar_texto(&fluxo, &scenario, TurnId::generate(), 0);

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
    assert_eq!(transport.envios(), 1);
    assert_eq!(fluxo.metrics().outputs_definitive.value(), 1);
}

#[tokio::test]
async fn saida_idempotente_por_turno_e_sequencia() {
    let scenario = Scenario::new("saida-idempotente");
    let (fluxo, _atores, _registro) = montar_fluxo(
        &scenario,
        opcoes_rapidas(),
        opcoes_atores(8),
        Duration::ZERO,
    );
    let turno = TurnId::generate();
    let first = enfileirar_texto(&fluxo, &scenario, turno, 0);
    let (_novo, segunda) = fluxo
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
    assert_eq!(first.id, segunda.id, "a mesma chave não duplica a linha");
    assert_eq!(
        segunda.content_json, "{\"texto\":\"resposta 0\"}",
        "o conteúdo original é preservado"
    );
}

#[test]
fn backoff_cresce_e_para_no_teto() {
    let backoff = Backoff {
        base: Duration::from_millis(100),
        factor: 2,
        cap: Duration::from_secs(1),
    };
    let esperas: Vec<u128> = (1..=8)
        .map(|attempt| backoff.wait(attempt).as_millis())
        .collect();
    assert_eq!(esperas, vec![100, 200, 400, 800, 1000, 1000, 1000, 1000]);
    assert!(
        esperas.windows(2).all(|par| par[0] <= par[1]),
        "nunca diminui"
    );
    assert_eq!(
        backoff.wait(0),
        Duration::from_millis(100),
        "tentativa 0 = base"
    );
}

#[tokio::test]
async fn item_em_backoff_nao_e_reenviado_antes_da_hora() {
    let scenario = Scenario::new("backoff-virtual");
    let clock = Arc::new(Mutex::new(Instant::now()));
    let relogio_para_o_fluxo = Arc::clone(&clock);
    let (fluxo, _atores, _registro) = montar_fluxo(
        &scenario,
        FlowOptions {
            backoff: Backoff {
                base: Duration::from_secs(2),
                factor: 2,
                cap: Duration::from_secs(30),
            },
            ..FlowOptions::default()
        },
        opcoes_atores(8),
        Duration::ZERO,
    );
    fluxo.with_clock(Arc::new(move || {
        *relogio_para_o_fluxo.lock().expect("sem veneno")
    }));

    let transport = TransporteDeTeste::com_falha(SendFailure::transient("429"));
    enfileirar_texto(&fluxo, &scenario, TurnId::generate(), 0);

    let first = fluxo
        .send_pending(scenario.org, transport.as_ref())
        .await
        .expect("envia");
    assert_eq!(first.failed, 1);
    assert_eq!(transport.envios(), 1);

    let adiado = fluxo
        .send_pending(scenario.org, transport.as_ref())
        .await
        .expect("envia");
    assert_eq!(adiado.deferred, 1, "ainda dentro do backoff");
    assert_eq!(transport.envios(), 1, "não chamou o canal de novo");

    *clock.lock().expect("sem veneno") += Duration::from_secs(2);
    let second = fluxo
        .send_pending(scenario.org, transport.as_ref())
        .await
        .expect("envia");
    assert_eq!(second.failed, 1);
    assert_eq!(transport.envios(), 2, "repetiu depois da espera");

    *clock.lock().expect("sem veneno") += Duration::from_secs(3);
    let ainda = fluxo
        .send_pending(scenario.org, transport.as_ref())
        .await
        .expect("envia");
    assert_eq!(ainda.deferred, 1, "a espera dobrou");
    assert_eq!(transport.envios(), 2);
}

#[tokio::test]
async fn caixas_nao_cruzam_org() {
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
    let (fluxo, _atores, _registro) = montar_fluxo(
        &scenario,
        opcoes_rapidas(),
        opcoes_atores(8),
        Duration::ZERO,
    );

    let recebido = fluxo
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
    let saida = enfileirar_texto(&fluxo, &scenario, TurnId::generate(), 0);

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
        scenario.store.inbox().get(other, recebido.item.id).is_err(),
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
async fn fila_cheia_adia_e_nao_perde_o_evento() {
    let scenario = Scenario::new("fila-cheia");
    let (fluxo, actors, registro) = montar_fluxo(
        &scenario,
        opcoes_rapidas(),
        opcoes_atores(1),
        Duration::from_millis(120),
    );

    let mut items = HashMap::new();
    for index in 0..3 {
        let recebido = fluxo
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
        items.insert(index, recebido.item.id);
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
async fn lista_lida_antes_da_conclusao_nao_repete_item_processado() {
    let scenario = Scenario::new("lista-antiga");
    let (fluxo, actors, registro) = montar_fluxo(
        &scenario,
        opcoes_rapidas(),
        opcoes_atores(4),
        Duration::ZERO,
    );
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

    let antiga = scenario
        .store
        .inbox()
        .list_pending(scenario.org, 10)
        .expect("lista");
    assert_eq!(antiga.len(), 1, "a lista antiga traz o item");

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
        .dispatch_items(scenario.org, antiga)
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
async fn dois_despachos_concorrentes_nao_duplicam_o_evento() {
    let scenario = Scenario::new("concorrencia-entrada");
    let (fluxo, actors, registro) = montar_fluxo(
        &scenario,
        opcoes_rapidas(),
        opcoes_atores(64),
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
    let resultados: Vec<poph_agent::entry_output::DispatchReport> = std::thread::scope(|scope| {
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

    let dispatch: u32 = resultados.iter().map(|report| report.dispatch).sum();
    assert_eq!(dispatch, 6, "cada item foi reservado uma única vez");
    assert!(actors.wait_queues_empty(Duration::from_secs(10)).await);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(registro.quantity(), 6, "seis processamentos, nem um a mais");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dois_envios_concorrentes_nao_duplicam_a_saida() {
    let scenario = Scenario::new("concorrencia-saida");
    let (fluxo, _atores, _registro) = montar_fluxo(
        &scenario,
        opcoes_rapidas(),
        opcoes_atores(8),
        Duration::ZERO,
    );
    for _ in 0..4 {
        enfileirar_texto(&fluxo, &scenario, TurnId::generate(), 0);
    }
    let transport = TransporteDeTeste::com_espera(Duration::from_millis(40));

    let (first, second) = tokio::join!(
        fluxo.send_pending(scenario.org, transport.as_ref()),
        fluxo.send_pending(scenario.org, transport.as_ref()),
    );
    let first = first.expect("envia");
    let second = second.expect("envia");

    assert_eq!(transport.envios(), 4, "cada item foi ao canal uma vez só");
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

fn esperar_marcador(marcador: &Path) {
    let beginning = Instant::now();
    while beginning.elapsed() < Duration::from_secs(30) {
        if marcador.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("o filho não escreveu o marcador em {} s", 30);
}

fn matar(mut filho: std::process::Child) {
    filho.kill().expect("SIGKILL no filho");
    let saida = filho.wait().expect("espera o filho");
    assert!(
        !saida.success(),
        "o filho devia ter morrido por sinal, não saído com sucesso"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "medição, não checagem"]
async fn medir_ingestao() {
    const THREADS: usize = 8;
    const POR_THREAD: usize = 1500;

    let scenario = Scenario::new("medir-ingestao");
    let (fluxo, _atores, _registro) = montar_fluxo(
        &scenario,
        opcoes_rapidas(),
        opcoes_atores(8),
        Duration::ZERO,
    );

    let beginning = Instant::now();
    let mut latencias: Vec<u64> = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for thread in 0..THREADS {
            let fluxo = fluxo.clone();
            let org = scenario.org;
            let integracao = scenario.integracao;
            let conversa = scenario.conversa;
            handles.push(scope.spawn(move || {
                let mut micros = Vec::with_capacity(POR_THREAD);
                for index in 0..POR_THREAD {
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
        let mut todas = Vec::with_capacity(THREADS * POR_THREAD);
        for handle in handles {
            todas.extend(handle.join().expect("thread de medição"));
        }
        todas
    });
    let total = beginning.elapsed();
    latencias.sort_unstable();
    let p = |fracao: f64| {
        latencias[((latencias.len() as f64 * fracao) as usize).min(latencias.len() - 1)]
    };
    let quantity = latencias.len();
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
        (THREADS * POR_THREAD) as u64,
        "todas as mensagens ficaram na fila, nenhuma duplicada nem perdida"
    );
}
