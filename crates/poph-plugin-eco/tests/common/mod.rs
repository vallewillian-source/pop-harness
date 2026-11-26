#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use poph_agent::atores::{Atores, Futuro, OpcoesAtores, Processador, Tarefa};
use poph_agent::entrada_saida::{Fluxo, OpcoesFluxo};
use poph_agent::fila_integracao::{ConversaDoCanal, EnvioPeloCanal, InboxDoCanal};
use poph_agent::metricas::MetricasMemoria;
use poph_plugins::{Backoff, PluginOptions, Supervisor, SupervisorLimits};
use poph_store::{
    Conversa, NewAgent, NewConversation, NewOrg, NewOutboxItem, OutboxItem, OutboxState, Page,
    Store,
};
use poph_types::{ConversationId, EventSink, Integration, IntegrationId, OrgId};

pub fn fast_limits() -> SupervisorLimits {
    SupervisorLimits {
        max_message_bytes: 1024 * 1024,
        timeout_handshake: Duration::from_secs(5),
        command_timeout: Duration::from_millis(500),
        health_timeout: Duration::from_secs(2),
        backoff: Backoff {
            base: Duration::from_millis(30),
            factor: 2,
            cap: Duration::from_millis(200),
        },
        max_failures: 5,
    }
}

pub struct NoTurn;

impl Processador for NoTurn {
    fn processar(&self, _task: Tarefa) -> Futuro {
        Box::pin(async { Ok(()) })
    }
}

pub fn temporary_dir(label: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "poph-eco-{label}-{}-{sequence}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("criar a pasta do cenário");
    directory
}

pub struct Scenario {
    pub directory: PathBuf,
    pub store: Arc<Store>,
    pub org: OrgId,
    pub agent: poph_types::AgentId,
    pub conversation: Conversa,
    pub flow: Fluxo,
    pub inbox: Arc<InboxDoCanal>,
    pub supervisor: Supervisor,
    pub channel: Arc<dyn Integration>,
    pub integration: IntegrationId,
}

impl Scenario {
    pub async fn new(label: &str, program: &PathBuf, peer: Option<&str>) -> Self {
        let directory = temporary_dir(label);
        let store = Arc::new(
            Store::open_at(&directory.join("harness.db"), &directory.join("rag.db"))
                .expect("abre o banco"),
        );
        let org = store
            .orgs()
            .create(NewOrg {
                name: format!("Org {label}"),
                slug: format!("org-{label}"),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agent = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: format!("Agente {label}"),
                    slug: format!("agente-{label}"),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        let conversation = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agent,
                    integration_id: None,
                    canal: "eco".to_owned(),
                    peer_externo: peer.map(str::to_owned),
                    target_department: None,
                },
            )
            .expect("abre a conversa");

        let actors = Atores::novo(
            Arc::new(NoTurn),
            OpcoesAtores::default(),
            MetricasMemoria::proprio(),
        );
        let flow = Fluxo::nova(Arc::clone(&store), actors, OpcoesFluxo::default());
        let resolver = Arc::new(ConversaDoCanal::novo(
            Arc::clone(&store),
            agent,
            "eco",
            None,
        ));
        let inbox = InboxDoCanal::iniciar(flow.clone(), org, resolver);

        let integration = IntegrationId::generate();
        let options = PluginOptions::new(program, integration, org)
            .with_kind("eco")
            .with_limits(fast_limits());
        let supervisor = Supervisor::start(options, Arc::clone(&inbox) as Arc<dyn EventSink>)
            .await
            .expect("sobe o plugin eco");
        let channel: Arc<dyn Integration> = Arc::new(supervisor.clone());
        Self {
            directory,
            store,
            org,
            agent,
            conversation,
            flow,
            inbox,
            supervisor,
            channel,
            integration,
        }
    }

    pub fn transport(&self) -> EnvioPeloCanal {
        EnvioPeloCanal::novo(Arc::clone(&self.store), Arc::clone(&self.channel))
    }

    pub fn enqueue_text(&self, turn: poph_types::TurnId, text: &str) -> OutboxItem {
        self.store
            .outbox()
            .enqueue(
                self.org,
                NewOutboxItem {
                    conversation_id: self.conversation.id,
                    turn_id: turn,
                    seq: 0,
                    tipo: "text".to_owned(),
                    content_json: serde_json::json!({"texto": text}).to_string(),
                },
            )
            .expect("enfileira o texto")
            .1
    }

    pub fn item_state(&self, id: poph_types::OutboxId) -> OutboxState {
        self.store
            .outbox()
            .get(self.org, id)
            .expect("lê o item de saída")
            .state
    }

    pub fn messages(&self) -> Vec<poph_store::Mensagem> {
        self.store
            .messages()
            .list(self.org, self.conversation.id, Page::with_limit(50))
            .expect("lista as mensagens")
    }

    pub async fn wait_until(&self, label: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("a condição `{label}` não ficou verdadeira em 5 s");
    }

    pub async fn shutdown(self) {
        self.supervisor.shutdown().await;
    }

    pub fn conversation_id(&self) -> ConversationId {
        self.conversation.id
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
