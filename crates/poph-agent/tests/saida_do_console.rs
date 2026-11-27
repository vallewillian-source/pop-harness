use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poph_agent::actors::{ActorOptions, Actors, BoxFuture, Processor, Task};
use poph_agent::entry_output::{FlowOptions, Fluxo, OutputTransport, SendFuture};
use poph_agent::metrics::MetricsMemory;
use poph_store::{
    NewAgent, NewConversation, NewOrg, NewOutboxItem, OutboxItem, OutboxState, Store,
};
use poph_types::{ConversationId, OrgId, TurnId};

struct NoProcess;

impl Processor for NoProcess {
    fn process(&self, _task: Task) -> BoxFuture {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Default)]
struct TransportTest {
    sends: AtomicUsize,
    received: Mutex<Vec<ConversationId>>,
}

impl TransportTest {
    fn sends(&self) -> usize {
        self.sends.load(Ordering::SeqCst)
    }

    fn conversations_sent(&self) -> Vec<ConversationId> {
        self.received.lock().expect("sem veneno").clone()
    }
}

impl OutputTransport for TransportTest {
    fn send(&self, item: OutboxItem) -> SendFuture {
        self.sends.fetch_add(1, Ordering::SeqCst);
        self.received
            .lock()
            .expect("sem veneno")
            .push(item.conversation_id);
        Box::pin(async { Ok(()) })
    }
}

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    console: ConversationId,
    whatsapp: ConversationId,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let folder =
            std::env::temp_dir().join(format!("poph-hr26f-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("cria a pasta do teste");
        let store = Arc::new(
            Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
                .expect("abre o banco"),
        );
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Org HR-26f".to_owned(),
                slug: format!("org-hr26f-{label}"),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Agente HR-26f".to_owned(),
                    slug: format!("agente-hr26f-{label}"),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        let integracao = store
            .create_integration(org, agente, "whatsapp".to_owned(), "active".to_owned())
            .expect("cria a integração")
            .id
            .parse()
            .expect("id da integração");
        let console = store
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
            .expect("abre a conversa do console")
            .id;
        let whatsapp = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente,
                    integration_id: Some(integracao),
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some("cliente-de-teste".to_owned()),
                    target_department: None,
                },
            )
            .expect("abre a conversa do WhatsApp")
            .id;
        Self {
            folder,
            store,
            org,
            console,
            whatsapp,
        }
    }

    fn fluxo(&self) -> Fluxo {
        let actors = Actors::new(
            Arc::new(NoProcess),
            ActorOptions {
                ociosidade: Duration::from_secs(60),
                queue_by_conversation: 8,
                max_actors: 8,
            },
            MetricsMemory::own(),
        );
        Fluxo::new(Arc::clone(&self.store), actors, FlowOptions::default())
    }

    fn enqueue(&self, conversa: ConversationId, seq: u32) -> OutboxItem {
        let (_new, item) = self
            .store
            .outbox()
            .enqueue(
                self.org,
                NewOutboxItem {
                    conversation_id: conversa,
                    turn_id: TurnId::generate(),
                    seq,
                    tipo: "text".to_owned(),
                    content_json: format!("{{\"texto\":\"resposta {seq}\"}}"),
                },
            )
            .expect("enfileira a saída");
        item
    }

    fn estado(&self, item: &OutboxItem) -> OutboxState {
        self.store
            .outbox()
            .get(self.org, item.id)
            .expect("lê o item")
            .state
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

#[tokio::test]
async fn chat_confirms_console_does_not_touch_whatsapp() {
    let scenario = Scenario::new("chat");
    let do_console = scenario.enqueue(scenario.console, 0);
    let whatsapp_item = scenario.enqueue(scenario.whatsapp, 0);
    let transport = TransportTest::default();

    let report = scenario
        .fluxo()
        .send_pending_console(scenario.org, &transport)
        .await
        .expect("envia o que é do console");

    assert_eq!(report.sent, 1, "só o do console sai");
    assert_eq!(transport.sends(), 1, "o transporte foi chamado uma vez");
    assert_eq!(
        transport.conversations_sent(),
        vec![scenario.console],
        "a única chamada é da conversa do console"
    );
    assert_eq!(scenario.estado(&do_console), OutboxState::Sent);
    assert_eq!(
        scenario.estado(&whatsapp_item),
        OutboxState::Pending,
        "o item de WhatsApp continua pendente"
    );
    let stored = scenario
        .store
        .outbox()
        .get(scenario.org, whatsapp_item.id)
        .expect("lê o item de WhatsApp");
    assert_eq!(stored.attempts, 0, "nenhuma tentativa foi gasta nele");
    assert!(stored.ultimo_erro.is_none(), "nenhum erro foi gravado nele");
}

#[tokio::test]
async fn general_send_would_take_whatsapp_but_this_chat_does_not_use_it() {
    let scenario = Scenario::new("geral");
    let whatsapp_item = scenario.enqueue(scenario.whatsapp, 0);
    let transport = TransportTest::default();

    scenario
        .fluxo()
        .send_pending(scenario.org, &transport)
        .await
        .expect("envio geral");

    assert_eq!(transport.sends(), 1, "o geral tocou no item de WhatsApp");
    assert_eq!(scenario.estado(&whatsapp_item), OutboxState::Sent);
}

#[tokio::test]
async fn whatsapp_pending_does_not_call_transport() {
    let scenario = Scenario::new("so-whatsapp");
    scenario.enqueue(scenario.whatsapp, 0);
    scenario.enqueue(scenario.whatsapp, 1);
    let transport = TransportTest::default();

    let report = scenario
        .fluxo()
        .send_pending_console(scenario.org, &transport)
        .await
        .expect("envio do console");

    assert_eq!(transport.sends(), 0, "nenhuma chamada ao canal do chat");
    assert_eq!(report.sent, 0);
}
