use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_agent::actors::{ActorOptions, Actors};
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::entry_output::{FlowOptions, Fluxo};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::turno::{TurnEngine, TurnOptions, TurnProcessor};
use poph_core::llm::{FakeProvider, FinishReason, LlmError, LlmEvent, LlmProvider, Script};
use poph_server::ServerState;
use poph_server::turn_input::INTERRUPTED_REASON;
use poph_store::{
    Direction, NewAgent, NewConversation, NewMessage, NewOrg, Page, Role, Store, TurnState,
};
use poph_types::config::GlobalTokenConfig;
use poph_types::{ConversationId, HarnessConfig, OrgId};
use rand_core::OsRng;
use reqwest::{Client, StatusCode};
use serde_json::Value;
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-hr57";
const QUESTION: &str = "qual o prazo de entrega?";
const RESPONSE: &str = "o prazo é de cinco dias úteis";

fn hash_argon2(value: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

fn multipart(texto: &str) -> (String, Vec<u8>) {
    let limit = "hr57-boundary";
    let body = format!(
        "--{limit}\r\nContent-Disposition: form-data; name=\"text\"\r\n\r\n{texto}\r\n--{limit}--\r\n"
    )
    .into_bytes();
    (format!("multipart/form-data; boundary={limit}"), body)
}

fn test_folder(label: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let folder = std::env::temp_dir().join(format!(
        "poph-resiliencia-{label}-{}-{timestamp}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(folder.join("dados")).expect("pasta de dados");
    folder
}

fn configuracao(folder: &Path) -> HarnessConfig {
    let mut config = HarnessConfig::default();
    config.data.directory = folder.join("dados");
    config.server.port = 7484;
    config.admin.tokens_globais = vec![GlobalTokenConfig {
        label: "teste".to_owned(),
        hash: hash_argon2(TOKEN_GLOBAL),
    }];
    config.validar().expect("configuração válida");
    config
}

fn open_store(folder: &Path) -> (Arc<Store>, HarnessConfig) {
    let config = configuracao(folder);
    let store = Arc::new(Store::open(&config).expect("abre o banco"));
    (store, config)
}

fn seed_org_and_agent(store: &Store) -> (OrgId, poph_types::AgentId) {
    let org = store
        .orgs()
        .create(NewOrg {
            name: "Org HR-57".to_owned(),
            slug: "org-hr57".to_owned(),
            config_json: "{}".to_owned(),
        })
        .expect("org")
        .id;
    let agente = store
        .agents()
        .create(
            org,
            NewAgent {
                name: "Bia".to_owned(),
                slug: "bia".to_owned(),
                grouping_window_ms: 1,
                ..Default::default()
            },
        )
        .expect("agente")
        .id;
    (org, agente)
}

fn open_conversation(
    store: &Store,
    org: OrgId,
    agente: poph_types::AgentId,
    canal: &str,
) -> ConversationId {
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
        .expect("conversa")
        .id
}

fn record_question(
    store: &Store,
    org: OrgId,
    conversa: ConversationId,
    texto: &str,
) -> poph_types::MessageId {
    store
        .messages()
        .write(
            org,
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
        .expect("mensagem")
        .id
}

struct Servidor {
    base: String,
    client: Client,
    store: Arc<Store>,
    org: OrgId,
    agente: poph_types::AgentId,
    desligar: Option<oneshot::Sender<()>>,
    folder: PathBuf,
}

impl Drop for Servidor {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        let _ = fs::remove_dir_all(&self.folder);
    }
}

impl Servidor {
    async fn boot(
        label: &str,
        prepare: impl FnOnce(&Store) -> (OrgId, poph_types::AgentId),
        scripts: Vec<Script>,
        reservation_expiry: Option<Duration>,
    ) -> Self {
        let folder = test_folder(label);
        let (store, config) = open_store(&folder);
        let (org, agente) = prepare(&store);

        let fake = Arc::new(FakeProvider::with_scripts(scripts));
        let provedor: Arc<dyn LlmProvider> = fake.clone();
        let engine = TurnEngine::new(
            Arc::clone(&store),
            ConfigHot::new(
                Arc::clone(&store),
                HotConfigOptions::default(),
                MetricsMemory::own(),
            ),
            Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own())),
            provedor,
            TurnOptions::default(),
            MetricsTurn::own(),
        );
        let engine = Arc::new(engine);
        let processor = Arc::new(TurnProcessor::new(Arc::clone(&engine)));
        let actors = Actors::new(
            processor.clone(),
            ActorOptions::default(),
            MetricsMemory::own(),
        );
        let fluxo = Fluxo::new(Arc::clone(&store), actors, FlowOptions::default());
        processor.attach_io(fluxo.clone());

        let mut estado = ServerState::new(Arc::clone(&store), config)
            .expect("estado do servidor")
            .with_engine(engine)
            .with_turn(fluxo, Duration::from_millis(20));
        if let Some(expiry) = reservation_expiry {
            estado = estado.with_reservation_ttl(expiry);
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, signal) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = poph_server::serve_at(listener, estado, async {
                let _ = signal.await;
            })
            .await;
        });
        let servidor = Self {
            base: format!("http://{endereco}"),
            client: Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("cliente HTTP"),
            store,
            org,
            agente,
            desligar: Some(desligar),
            folder,
        };
        servidor.wait_boot().await;
        servidor
    }

    async fn wait_boot(&self) {
        let start = Instant::now();
        loop {
            match self
                .client
                .get(format!("{}/healthz", self.base))
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => return,
                _ if start.elapsed() < Duration::from_secs(10) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                other => panic!("servidor não subiu: {other:?}"),
            }
        }
    }

    fn root(&self) -> String {
        format!(
            "/admin/v1/orgs/{}/agents/{}/test-chat",
            self.org, self.agente
        )
    }

    async fn create_conversation(&self) -> String {
        let response = self
            .client
            .post(format!("{}{}", self.base, self.root()))
            .bearer_auth(TOKEN_GLOBAL)
            .send()
            .await
            .expect("criar conversa");
        assert_eq!(response.status(), StatusCode::CREATED);
        let body: Value = response.json().await.expect("json da conversa");
        body["id"].as_str().expect("id da conversa").to_owned()
    }

    async fn send(&self, conversa: &str, texto: &str) -> reqwest::Response {
        let (tipo, body) = multipart(texto);
        self.client
            .post(format!("{}{}/{conversa}/messages", self.base, self.root()))
            .bearer_auth(TOKEN_GLOBAL)
            .header("content-type", tipo)
            .body(body)
            .send()
            .await
            .expect("envia a mensagem")
    }

    async fn wait_turn_finish(&self, conversa: ConversationId) -> (String, Option<String>) {
        let start = Instant::now();
        loop {
            let turns = self
                .store
                .turns()
                .list(self.org, conversa, Page::with_limit(50))
                .expect("turnos");
            if let Some(turno) = turns.last()
                && turno.state != TurnState::Running
            {
                return (turno.state.as_text().to_owned(), turno.error.clone());
            }
            assert!(
                start.elapsed() < Duration::from_secs(15),
                "o turno não terminou a tempo: {turns:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn outbound_messages(&self, conversa: ConversationId) -> usize {
        self.store
            .messages()
            .list(self.org, conversa, Page::with_limit(200))
            .expect("mensagens")
            .iter()
            .filter(|mensagem| mensagem.direction == Direction::Outbound)
            .count()
    }

    fn inbound_messages(&self, conversa: ConversationId) -> usize {
        self.store
            .messages()
            .list(self.org, conversa, Page::with_limit(200))
            .expect("mensagens")
            .iter()
            .filter(|mensagem| mensagem.direction == Direction::Inbound)
            .count()
    }
}

fn scripted_ok() -> Script {
    Script::Events(vec![
        LlmEvent::Texto(RESPONSE.into()),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

#[tokio::test]
async fn turn_interrupted_becomes_failed_on_boot_and_the_conversation_accepts_the_next_message() {
    let (previous_conversation, previous_turn) =
        (std::cell::Cell::new(None), std::cell::Cell::new(None));
    let servidor = Servidor::boot(
        "orfao",
        |store| {
            let (org, agente) = seed_org_and_agent(store);
            let conversa = open_conversation(store, org, agente, "test");
            record_question(store, org, conversa, QUESTION);
            let turno = store
                .turns()
                .start(org, conversa, Some(agente))
                .expect("turno do processo anterior");
            previous_conversation.set(Some(conversa));
            previous_turn.set(Some(turno.id));
            (org, agente)
        },
        vec![scripted_ok(), scripted_ok()],
        None,
    )
    .await;

    let conversa = previous_conversation.get().expect("conversa semeada");
    let turno = previous_turn.get().expect("turno semeado");

    let recorded = servidor
        .store
        .turns()
        .get(servidor.org, turno)
        .expect("turno gravado");
    assert_eq!(recorded.state, TurnState::Failed);
    assert_eq!(recorded.error.as_deref(), Some(INTERRUPTED_REASON));
    assert!(
        recorded.finished_at.is_some(),
        "o turno órfão ganha hora de fim"
    );

    assert_eq!(servidor.inbound_messages(conversa), 1);
    assert_eq!(servidor.outbound_messages(conversa), 0);

    let response = servidor.send(&conversa.to_string(), "e o frete?").await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "sem 409 turno_em_andamento"
    );
    let body = response.text().await.expect("corpo do SSE");
    assert!(body.contains("event: fim"), "o turno novo fecha: {body}");
    assert!(
        body.contains("\"state\":\"done\""),
        "o turno novo conclui: {body}"
    );

    let (estado, error) = servidor.wait_turn_finish(conversa).await;
    assert_eq!(estado, "done", "último turno concluiu: {error:?}");
    assert_eq!(
        servidor.outbound_messages(conversa),
        1,
        "uma única resposta do agente na conversa"
    );
}

#[tokio::test]
async fn model_unavailable_leaves_turn_failed_without_losing_the_message() {
    let servidor = Servidor::boot(
        "indisponivel",
        seed_org_and_agent,
        vec![
            Script::Failure(LlmError::Unavailable {
                tentativas: 2,
                last_error: "HTTP 503".to_owned(),
            }),
            scripted_ok(),
        ],
        None,
    )
    .await;
    let conversa = servidor.create_conversation().await;

    let response = servidor.send(&conversa, QUESTION).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("corpo do SSE");
    assert!(
        body.contains("\"state\":\"failed\""),
        "o fim do turno diz que falhou: {body}"
    );
    assert!(
        !body.contains("\"state\":\"done\""),
        "nunca conclui sem resposta: {body}"
    );

    let id: ConversationId = conversa.parse().expect("conversa");
    let (estado, error) = servidor.wait_turn_finish(id).await;
    assert_eq!(estado, "failed");
    assert!(
        error.as_deref().is_some_and(|error| !error.is_empty()),
        "o motivo da falha fica gravado: {error:?}"
    );
    assert_eq!(servidor.inbound_messages(id), 1, "a mensagem não se perde");
    assert_eq!(servidor.outbound_messages(id), 0, "não inventa resposta");

    let response = servidor.send(&conversa, "pode tentar de novo?").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("corpo do SSE");
    assert!(
        body.contains("\"state\":\"done\""),
        "segundo turno conclui: {body}"
    );
    assert_eq!(
        servidor.outbound_messages(id),
        1,
        "uma resposta, sem duplicar"
    );
}

#[tokio::test]
async fn cut_stream_leaves_turn_failed_without_partial_reply() {
    let servidor = Servidor::boot(
        "stream-cortado",
        seed_org_and_agent,
        vec![
            Script::Pending {
                events: vec![LlmEvent::Texto("o prazo é de ".into())],
                after: Duration::from_millis(50),
                error: LlmError::ConnectionInterrupted("conexão caiu".to_owned()),
            },
            scripted_ok(),
        ],
        None,
    )
    .await;
    let conversa = servidor.create_conversation().await;

    let response = servidor.send(&conversa, QUESTION).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("corpo do SSE");
    assert!(
        body.contains("\"state\":\"failed\""),
        "o turno falha: {body}"
    );

    let id: ConversationId = conversa.parse().expect("conversa");
    let (estado, error) = servidor.wait_turn_finish(id).await;
    assert_eq!(estado, "failed");
    assert!(
        error.as_deref().is_some_and(|error| error.contains("caiu")),
        "o motivo do corte fica gravado: {error:?}"
    );
    assert_eq!(
        servidor.outbound_messages(id),
        0,
        "texto parcial não vira resposta na conversa"
    );

    let response = servidor.send(&conversa, "e aí?").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("corpo do SSE");
    assert!(body.contains("\"state\":\"done\""));
}

#[tokio::test]
async fn console_outbox_stuck_in_sending_is_confirmed_on_boot() {
    let servidor = Servidor::boot(
        "console-preso",
        |store| {
            let (org, agente) = seed_org_and_agent(store);
            let conversa = open_conversation(store, org, agente, "test");
            let turno = store
                .turns()
                .start(org, conversa, Some(agente))
                .expect("turno do processo anterior");
            let (_, item) = store
                .outbox()
                .enqueue(
                    org,
                    poph_store::NewOutboxItem {
                        conversation_id: conversa,
                        turn_id: turno.id,
                        seq: 0,
                        tipo: "text".to_owned(),
                        content_json: "{\"texto\":\"resposta já mostrada\"}".to_owned(),
                    },
                )
                .expect("enfileira a resposta");
            store
                .outbox()
                .start_send(org, item.id)
                .expect("marca como em envio");
            (org, agente)
        },
        Vec::new(),
        None,
    )
    .await;

    let start = Instant::now();
    loop {
        let sent = servidor
            .store
            .outbox()
            .count_by_state(servidor.org, poph_store::OutboxState::Sent)
            .expect("conta a fila");
        if sent == 1 {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "o item preso em `sending` não foi confirmado na subida"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        servidor
            .store
            .outbox()
            .count_by_state(servidor.org, poph_store::OutboxState::Sending)
            .expect("conta a fila"),
        0,
        "nada fica em sending"
    );
}

#[tokio::test]
async fn test_turn_reservation_expires() {
    let folder = test_folder("reserva");
    let (store, config) = open_store(&folder);
    let (org, agente) = seed_org_and_agent(&store);
    let conversa = open_conversation(&store, org, agente, "test");
    let estado = ServerState::new(store, config)
        .expect("estado")
        .with_reservation_ttl(Duration::from_millis(60));

    assert!(estado.reserve_test_turn(conversa));
    assert!(
        !estado.reserve_test_turn(conversa),
        "enquanto vale, a segunda aba recebe 409"
    );
    tokio::time::sleep(Duration::from_millis(90)).await;
    assert!(
        estado.reserve_test_turn(conversa),
        "vencida a reserva, a conversa volta a aceitar mensagem"
    );
    estado.release_test_turn(conversa);
    assert!(estado.reserve_test_turn(conversa));
    let _ = fs::remove_dir_all(&folder);
}
