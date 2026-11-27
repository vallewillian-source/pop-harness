use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use poph_evolution::webhook::SECRET_HEADER;
use poph_evolution::{ATTACHMENT_CAP, ConfigEvolution, EvolutionClient};
use poph_matrix::{ClientConfig, GhostIds, MatrixClient};
use poph_server::evolution::EvolutionChannel;
use poph_server::{AsReceiver, AttendantResponse, HarnessResponsePort, ResponsePort, ServerState};
use poph_store::error::StoreError;
use poph_store::{
    HumanResponse, NewAgent, NewConversation, NewHumanResponse, NewOrg, Origin, OutboxState, Page,
    Store,
};
use poph_types::{ConversationId, HarnessConfig, IntegrationId, OrgId, Secret};
use reqwest::{Client, StatusCode as StatusHttp};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const PREFIX: &str = "ph";
const SERVER_NAME: &str = "matrix-dev.local";

const HS_TOKEN: &str = "CANARIO-HS-mx18-5f3c-TAO-SECRETO";

const AS_TOKEN: &str = "CANARIO-AS-mx18-91ab-TAO-SECRETO";

const INSTANCIA: &str = "loja";
const EVO_KEY: &str = "chave-evo-mx18-teste";
const WEBHOOK_SECRET: &str = "segredo-webhook-mx18";

const PHONE: &str = "5511999990000";
const JID: &str = "5511999990000@s.whatsapp.net";

const ATTENDANT: &str = "@ana:matrix-dev.local";
const ATTENDANT_NAME: &str = "Ana Atendente";

const ROOM: &str = "!sala-a:matrix-dev.local";
const ROOM_B: &str = "!sala-b:matrix-dev.local";

#[derive(Clone, Debug)]
struct Write {
    room: String,
    tipo: String,
    txn: String,
    disfarcado: Option<String>,
    authorization: Option<String>,
    body: Value,
}

#[derive(Clone, Default)]
struct FakeSynapse {
    profiles: Arc<Mutex<HashMap<String, String>>>,
    writes: Arc<Mutex<Vec<Write>>>,
    profiles_read: Arc<AtomicUsize>,
}

async fn fake_profile(
    State(fake): State<FakeSynapse>,
    Path(user): Path<String>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    fake.profiles_read.fetch_add(1, Ordering::SeqCst);
    let _ = headers;
    let profiles = fake.profiles.lock().unwrap();
    match profiles.get(&user) {
        Some(name) => (
            StatusCode::OK,
            Json(json!({ "displayname": name, "avatar_url": null })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "errcode": "M_NOT_FOUND" })),
        ),
    }
}

async fn send_fake(
    State(fake): State<FakeSynapse>,
    Path((room, tipo, txn)): Path<(String, String, String)>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let mut writes = fake.writes.lock().unwrap();
    let position = writes.len() + 1;
    writes.push(Write {
        room,
        tipo,
        txn,
        disfarcado: query.get("user_id").cloned(),
        authorization,
        body,
    });
    (
        StatusCode::OK,
        Json(json!({ "event_id": format!("$escrito-{position}:matrix-dev.local") })),
    )
}

async fn boot_fake_synapse() -> (String, FakeSynapse) {
    let fake = FakeSynapse::default();
    {
        let mut profiles = fake.profiles.lock().unwrap();
        profiles.insert(ATTENDANT.to_owned(), ATTENDANT_NAME.to_owned());
    }
    let router = Router::new()
        .route("/pronto", get(|| async { StatusCode::OK }))
        .route("/_matrix/client/v3/profile/{user}", get(fake_profile))
        .route(
            "/_matrix/client/v3/rooms/{room}/send/{kind}/{txn}",
            put(send_fake),
        )
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera do Synapse falso");
    let endereco = listener.local_addr().expect("endereço do Synapse falso");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{endereco}"), fake)
}

async fn wait_synapse(url: &str) {
    let client = Client::new();
    let start = Instant::now();
    loop {
        let reply = client.get(format!("{url}/pronto")).send().await;
        if reply.is_ok() {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "Synapse falso não subiu"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[derive(Clone, Default)]
struct FakeEvolution {
    sends: Arc<Mutex<Vec<Value>>>,
}

async fn send_via_whatsapp(
    State(fake): State<FakeEvolution>,
    Path(_instance): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let mut sends = fake.sends.lock().unwrap();
    sends.push(body);
    let position = sends.len();
    (
        StatusCode::OK,
        Json(json!({ "key": { "id": format!("MSG-EVO-{position}") } })),
    )
}

async fn boot_fake_evolution() -> (String, FakeEvolution) {
    let fake = FakeEvolution::default();
    let router = Router::new()
        .route("/message/sendText/{instance}", post(send_via_whatsapp))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera da Evolution falsa");
    let endereco = listener.local_addr().expect("endereço da Evolution falsa");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{endereco}"), fake)
}

struct PortWithFailure {
    real: HarnessResponsePort,
    fail: AtomicBool,
}

impl PortWithFailure {
    fn fresh(store: Arc<Store>) -> Self {
        Self {
            real: HarnessResponsePort::fresh(store),
            fail: AtomicBool::new(true),
        }
    }

    fn fix(&self) {
        self.fail.store(false, Ordering::SeqCst);
    }
}

impl ResponsePort for PortWithFailure {
    fn room_conversation(
        &self,
        room: &str,
    ) -> Result<Option<poph_server::matrix_response::RoomConversation>, StoreError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(StoreError::Internal("banco fora do ar (teste)".to_owned()));
        }
        self.real.room_conversation(room)
    }

    fn conversation_agent(
        &self,
        target: poph_server::matrix_response::RoomConversation,
    ) -> Result<Option<poph_types::AgentId>, StoreError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(StoreError::Internal("banco fora do ar (teste)".to_owned()));
        }
        self.real.conversation_agent(target)
    }

    fn record_response(
        &self,
        target: poph_server::matrix_response::RoomConversation,
        fresh: NewHumanResponse<'_>,
    ) -> Result<HumanResponse, StoreError> {
        self.real.record_response(target, fresh)
    }

    fn record_response_with_file(
        &self,
        target: poph_server::matrix_response::RoomConversation,
        fresh: poph_store::HumanResponseWithFile<'_>,
    ) -> Result<HumanResponse, StoreError> {
        self.real.record_response_with_file(target, fresh)
    }
}

struct PortWithoutAgent {
    real: HarnessResponsePort,
}

impl ResponsePort for PortWithoutAgent {
    fn room_conversation(
        &self,
        room: &str,
    ) -> Result<Option<poph_server::matrix_response::RoomConversation>, StoreError> {
        self.real.room_conversation(room)
    }

    fn conversation_agent(
        &self,
        _target: poph_server::matrix_response::RoomConversation,
    ) -> Result<Option<poph_types::AgentId>, StoreError> {
        Ok(None)
    }

    fn record_response(
        &self,
        target: poph_server::matrix_response::RoomConversation,
        fresh: NewHumanResponse<'_>,
    ) -> Result<HumanResponse, StoreError> {
        self.real.record_response(target, fresh)
    }

    fn record_response_with_file(
        &self,
        target: poph_server::matrix_response::RoomConversation,
        fresh: poph_store::HumanResponseWithFile<'_>,
    ) -> Result<HumanResponse, StoreError> {
        self.real.record_response_with_file(target, fresh)
    }
}

struct ScenarioConversation {
    org: OrgId,
    conversation: ConversationId,
}

struct Scenario {
    dir: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    conversation: ConversationId,
    org_b: OrgId,
    conversation_b: ConversationId,
    synapse: FakeSynapse,
    evo: FakeEvolution,
    url_synapse: String,
    url_evo: String,
}

impl Scenario {
    async fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "poph-matrix-resposta-{label}-{}-{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("pasta do teste");
        let store = Arc::new(
            Store::open_at(&dir.join("harness.db"), &dir.join("rag.db")).expect("abre o banco"),
        );

        let a = Self::org_with_conversation(&store, "loja-a", "Sala A", "bia-a");
        let b = Self::org_with_conversation(&store, "loja-b", "Sala B", "bia-b");

        store
            .matrix()
            .enable(a.org)
            .expect("liga o módulo na org A");
        store
            .matrix()
            .ensure_room(a.org, a.conversation, ROOM, None)
            .expect("grava a sala da org A");
        store
            .matrix()
            .mark_room_ready(a.org, a.conversation)
            .expect("sala pronta");
        store
            .matrix()
            .enable(b.org)
            .expect("liga o módulo na org B");
        store
            .matrix()
            .ensure_room(b.org, b.conversation, ROOM_B, None)
            .expect("grava a sala da org B");
        store
            .matrix()
            .mark_room_ready(b.org, b.conversation)
            .expect("sala B pronta");

        let (url_synapse, synapse) = boot_fake_synapse().await;
        wait_synapse(&url_synapse).await;
        let (url_evo, evo) = boot_fake_evolution().await;
        Self {
            dir,
            store,
            org: a.org,
            conversation: a.conversation,
            org_b: b.org,
            conversation_b: b.conversation,
            synapse,
            evo,
            url_synapse,
            url_evo,
        }
    }

    fn org_with_conversation(
        store: &Store,
        slug: &str,
        agent_name: &str,
        agent_slug: &str,
    ) -> ScenarioConversation {
        let org = store
            .orgs()
            .create(NewOrg {
                name: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agent = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: agent_name.to_owned(),
                    slug: agent_slug.to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        let integracao: IntegrationId = store
            .create_integration(org, agent, "whatsapp".to_owned(), "active".to_owned())
            .expect("cria a integração")
            .id
            .parse()
            .expect("id da integração");
        let conversation = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agent,
                    integration_id: Some(integracao),
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some(PHONE.to_owned()),
                    target_department: None,
                },
            )
            .expect("cria a conversa")
            .id;
        ScenarioConversation { org, conversation }
    }

    fn channel(&self, org: OrgId, conversation: ConversationId, url_evo: &str) -> EvolutionChannel {
        let client = EvolutionClient::new(
            ConfigEvolution::new(
                url_evo.to_owned(),
                INSTANCIA,
                Secret::new(EVO_KEY.to_owned()),
            )
            .expect("config da Evolution falsa"),
        )
        .expect("cliente da Evolution falsa");
        let integracao = self
            .store
            .conversations()
            .get(org, conversation)
            .expect("lê a conversa")
            .integration_id
            .expect("a conversa tem integração");
        EvolutionChannel {
            org,
            instance: INSTANCIA.to_owned(),
            integration: integracao,
            secret: Some(Secret::new(WEBHOOK_SECRET.to_owned())),
            client: Some(client),
            attachment_limit: ATTACHMENT_CAP,
        }
    }

    async fn boot(&self, porta: Arc<dyn ResponsePort>) -> Servidor {
        let channels = vec![
            self.channel(self.org, self.conversation, &self.url_evo),
            self.channel(self.org_b, self.conversation_b, &self.url_evo),
        ];
        let matrix_customer = MatrixClient::new(
            ClientConfig::new(
                self.url_synapse.clone(),
                SERVER_NAME.to_owned(),
                Secret::new(AS_TOKEN.to_owned()),
            )
            .expect("config do cliente do Matrix"),
        )
        .expect("cliente do Matrix");
        let ids = GhostIds::new(PREFIX, SERVER_NAME).expect("ids dos fantasmas");
        let delivery = Arc::new(
            AttendantResponse::new(porta, Some(matrix_customer), ids)
                .with_blob_folder(self.dir.join("blobs")),
        );

        let state = ServerState::new(Arc::clone(&self.store), HarnessConfig::default())
            .expect("estado do servidor")
            .with_evolution_channels(channels)
            .with_matrix_as(Arc::new(AsReceiver::new(
                PREFIX,
                SERVER_NAME,
                Secret::new(HS_TOKEN.to_owned()),
                delivery.clone(),
            )));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, signal) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = poph_server::serve_at(listener, state, async {
                let _ = signal.await;
            })
            .await;
        });
        let servidor = Servidor {
            base: format!("http://{endereco}"),
            client: Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("cliente HTTP"),
            delivery,
            desligar: Some(desligar),
        };
        servidor.wait_boot().await;
        servidor
    }

    fn org_conversation(&self, org: OrgId) -> ConversationId {
        if org == self.org {
            self.conversation
        } else {
            self.conversation_b
        }
    }

    fn status(&self, org: OrgId) -> poph_store::ConversationStatus {
        self.store
            .conversations()
            .get(org, self.org_conversation(org))
            .expect("lê a conversa")
            .status
    }

    fn messages(&self, org: OrgId) -> Vec<poph_store::Mensagem> {
        self.store
            .messages()
            .list(org, self.org_conversation(org), Page::with_limit(50))
            .expect("lista as mensagens")
    }

    fn sent(&self, org: OrgId) -> u64 {
        self.store
            .outbox()
            .count_by_state(org, OutboxState::Sent)
            .expect("conta a fila de saída")
    }

    fn pending(&self, org: OrgId) -> u64 {
        self.store
            .outbox()
            .count_by_state(org, OutboxState::Pending)
            .expect("conta a fila de saída")
    }

    fn whatsapp_sends(&self) -> Vec<Value> {
        self.evo.sends.lock().unwrap().clone()
    }

    fn synapse_writes(&self) -> Vec<Write> {
        self.synapse.writes.lock().unwrap().clone()
    }

    fn agent_ghost(&self) -> String {
        let agent = self
            .store
            .conversations()
            .get(self.org, self.conversation)
            .expect("lê a conversa")
            .agent_id;
        GhostIds::new(PREFIX, SERVER_NAME)
            .expect("ids dos fantasmas")
            .agent(&agent.to_string())
            .expect("fantasma do agente")
            .user_id()
            .to_owned()
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

struct Servidor {
    base: String,
    client: Client,
    delivery: Arc<AttendantResponse>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Servidor {
    async fn wait_boot(&self) {
        let start = Instant::now();
        loop {
            match self
                .client
                .get(format!("{}/healthz", self.base))
                .send()
                .await
            {
                Ok(reply) if reply.status().is_success() => return,
                _ if start.elapsed() < Duration::from_secs(10) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                other => panic!("servidor não subiu: {other:?}"),
            }
        }
    }

    async fn transaction(&self, txn: &str, body: &Value) -> (StatusHttp, Value) {
        let reply = self
            .client
            .put(format!("{}/_matrix/app/v1/transactions/{txn}", self.base))
            .header("authorization", format!("Bearer {HS_TOKEN}"))
            .json(body)
            .send()
            .await
            .expect("entrega a transação");
        let status = reply.status();
        let value = reply.json::<Value>().await.unwrap_or(Value::Null);
        (status, value)
    }

    async fn attendant_text(
        &self,
        txn: &str,
        room: &str,
        event: &str,
        text: &str,
    ) -> (StatusHttp, Value) {
        self.transaction(
            txn,
            &json!({
                "events": [{
                    "type": "m.room.message",
                    "room_id": room,
                    "sender": ATTENDANT,
                    "event_id": event,
                    "origin_server_ts": 1_795_300_000_000i64,
                    "content": { "msgtype": "m.text", "body": text }
                }]
            }),
        )
        .await
    }

    async fn webhook(&self, org: OrgId, body: &Value) -> (StatusHttp, Value) {
        let reply = self
            .client
            .post(format!("{}/webhook/evolution/{org}/{INSTANCIA}", self.base))
            .header(SECRET_HEADER, WEBHOOK_SECRET)
            .json(body)
            .send()
            .await
            .expect("entrega o webhook");
        let status = reply.status();
        let value = reply.json::<Value>().await.unwrap_or(Value::Null);
        (status, value)
    }

    fn counters(&self) -> &poph_server::matrix_response::ResponseCounters {
        self.delivery.counters()
    }
}

impl Drop for Servidor {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
    }
}

async fn wait(label: &str, mut condition: impl FnMut() -> bool) {
    let start = Instant::now();
    loop {
        if condition() {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "não aconteceu a tempo: {label}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn texts_sent(sends: &[Value]) -> Vec<String> {
    sends
        .iter()
        .map(|send| {
            send.get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

#[tokio::test]
async fn attendant_reply_becomes_prefixed_send_and_transfers() {
    let scenario = Scenario::new("a2").await;
    let servidor = scenario
        .boot(Arc::new(HarnessResponsePort::fresh(Arc::clone(
            &scenario.store,
        ))))
        .await;

    let (status, body) = servidor
        .attendant_text(
            "txn-1",
            ROOM,
            "$evento-1:matrix-dev.local",
            "olá, tudo bem?",
        )
        .await;
    assert_eq!(status, StatusHttp::OK);
    assert_eq!(body, json!({}));

    wait("o envio sair pela Evolution", || {
        !scenario.whatsapp_sends().is_empty()
    })
    .await;
    let sends = scenario.whatsapp_sends();
    assert_eq!(sends.len(), 1, "uma mensagem do atendente = um envio");
    assert_eq!(
        texts_sent(&sends),
        vec!["*[Ana Atendente]*:\nolá, tudo bem?".to_owned()]
    );

    assert_eq!(
        scenario.status(scenario.org),
        poph_store::ConversationStatus::Transferred
    );

    let messages = scenario.messages(scenario.org);
    assert_eq!(messages.len(), 1);
    let message = &messages[0];
    assert_eq!(message.origin, Origin::Human);
    assert_eq!(message.texto, "olá, tudo bem?");
    assert_eq!(
        message.external_id.as_deref(),
        Some("$evento-1:matrix-dev.local")
    );
    assert_eq!(message.author_name.as_deref(), Some("Ana Atendente"));

    assert_eq!(scenario.sent(scenario.org), 1);
    assert_eq!(scenario.pending(scenario.org), 0);

    let (status, _) = servidor
        .webhook(
            scenario.org,
            &json!({
                "event": "messages.upsert",
                "instance": INSTANCIA,
                "apikey": EVO_KEY,
                "destination": "http://harness.teste/webhook/evolution/x/loja",
                "data": {
                    "key": { "id": "MSG-CLIENTE-1", "remoteJid": JID, "fromMe": false },
                    "pushName": "Cliente Teste",
                    "messageType": "conversation",
                    "message": { "conversation": "e agora?" },
                    "messageTimestamp": 1_795_300_100i64
                }
            }),
        )
        .await;
    assert_eq!(status, StatusHttp::OK);
    wait("a mensagem do cliente ser gravada", || {
        scenario.messages(scenario.org).len() == 2
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(scenario.whatsapp_sends().len(), 1, "o agente não respondeu");
    assert_eq!(
        scenario.status(scenario.org),
        poph_store::ConversationStatus::Transferred
    );

    assert_eq!(
        scenario.status(scenario.org_b),
        poph_store::ConversationStatus::Open
    );
    assert_eq!(scenario.sent(scenario.org_b), 0);
}

#[tokio::test]
async fn resend_of_same_transaction_or_event_makes_no_second_send() {
    let scenario = Scenario::new("a3").await;
    let porta = Arc::new(HarnessResponsePort::fresh(Arc::clone(&scenario.store)));
    let servidor = scenario.boot(porta).await;

    let (first, _) = servidor
        .attendant_text(
            "txn-repetida",
            ROOM,
            "$evento-repetido:matrix-dev.local",
            "oi",
        )
        .await;
    let (second, _) = servidor
        .attendant_text(
            "txn-repetida",
            ROOM,
            "$evento-repetido:matrix-dev.local",
            "oi",
        )
        .await;
    let (third, _) = servidor
        .attendant_text("txn-outra", ROOM, "$evento-repetido:matrix-dev.local", "oi")
        .await;

    assert_eq!(first, StatusHttp::OK);
    assert_eq!(second, StatusHttp::OK);
    assert_eq!(third, StatusHttp::OK);

    wait("o envio sair pela Evolution", || {
        scenario.sent(scenario.org) == 1
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        scenario.whatsapp_sends().len(),
        1,
        "reenvio não duplica o envio"
    );
    assert_eq!(
        scenario.messages(scenario.org).len(),
        1,
        "uma mensagem no histórico"
    );
    assert_eq!(
        servidor.counters().duplicates.load(Ordering::SeqCst),
        1,
        "o reenvio com transação nova foi reconhecido como duplicado"
    );
}

#[tokio::test]
async fn event_from_one_org_does_not_touch_other_org_conversation() {
    let scenario = Scenario::new("a9").await;
    let porta = Arc::new(HarnessResponsePort::fresh(Arc::clone(&scenario.store)));
    let servidor = scenario.boot(porta).await;

    let (status, _) = servidor
        .attendant_text(
            "txn-a",
            ROOM,
            "$evento-a:matrix-dev.local",
            "resposta da org A",
        )
        .await;
    assert_eq!(status, StatusHttp::OK);
    wait("o envio da org A sair", || scenario.sent(scenario.org) == 1).await;

    assert_eq!(
        scenario.status(scenario.org),
        poph_store::ConversationStatus::Transferred
    );
    assert_eq!(
        scenario.status(scenario.org_b),
        poph_store::ConversationStatus::Open,
        "a conversa da org B continua aberta"
    );
    assert!(
        scenario.messages(scenario.org_b).is_empty(),
        "nada gravado na org B"
    );
    assert_eq!(scenario.sent(scenario.org_b), 0, "nada enviado pela org B");

    let (status, _) = servidor
        .attendant_text(
            "txn-b",
            ROOM_B,
            "$evento-b:matrix-dev.local",
            "resposta da org B",
        )
        .await;
    assert_eq!(status, StatusHttp::OK);
    wait("o envio da org B sair", || {
        scenario.sent(scenario.org_b) == 1
    })
    .await;
    assert_eq!(
        scenario.messages(scenario.org).len(),
        1,
        "a org A ficou como estava"
    );
    assert_eq!(scenario.sent(scenario.org), 1);
}

#[tokio::test]
async fn closed_conversation_does_not_send_and_notifies_room() {
    let scenario = Scenario::new("encerrada").await;
    let conversation = scenario
        .store
        .conversations()
        .get(scenario.org, scenario.conversation)
        .expect("lê a conversa");
    scenario
        .store
        .conversations()
        .close_audited(
            scenario.org,
            scenario.conversation,
            conversation.version,
            "teste".to_owned(),
        )
        .expect("encerra a conversa");
    let porta = Arc::new(HarnessResponsePort::fresh(Arc::clone(&scenario.store)));
    let servidor = scenario.boot(porta).await;

    let (status, _) = servidor
        .attendant_text(
            "txn-fechada",
            ROOM,
            "$evento-fechado:matrix-dev.local",
            "oi",
        )
        .await;
    assert_eq!(
        status,
        StatusHttp::OK,
        "conversa encerrada não é erro de transporte"
    );

    wait("o aviso aparecer na sala", || {
        !scenario.synapse_writes().is_empty()
    })
    .await;
    let writes = scenario.synapse_writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(
        writes[0].body.get("body").and_then(Value::as_str),
        Some(poph_server::matrix_response::CLOSED_CONVERSATION_NOTICE)
    );
    assert_eq!(
        writes[0].body.get("msgtype").and_then(Value::as_str),
        Some("m.notice")
    );
    assert_eq!(scenario.whatsapp_sends().len(), 0, "nada vai ao cliente");
    assert!(
        scenario.messages(scenario.org).is_empty(),
        "nada gravado no histórico"
    );
    assert_eq!(
        scenario.status(scenario.org),
        poph_store::ConversationStatus::Closed
    );
}

#[tokio::test]
async fn unknown_room_is_ignored_with_200() {
    let scenario = Scenario::new("desconhecida").await;
    let porta = Arc::new(HarnessResponsePort::fresh(Arc::clone(&scenario.store)));
    let servidor = scenario.boot(porta).await;

    let (status, body) = servidor
        .attendant_text(
            "txn-estranha",
            "!sala-que-nao-existe:matrix-dev.local",
            "$evento-x:matrix-dev.local",
            "oi",
        )
        .await;
    assert_eq!(status, StatusHttp::OK);
    assert_eq!(body, json!({}));
    assert_eq!(servidor.counters().unknown_rooms.load(Ordering::SeqCst), 1);
    assert!(scenario.messages(scenario.org).is_empty());
    assert!(scenario.messages(scenario.org_b).is_empty());
    assert_eq!(scenario.sent(scenario.org), 0);
}

#[tokio::test]
async fn db_failure_gives_503_and_resend_processes() {
    let scenario = Scenario::new("falha").await;
    let porta = Arc::new(PortWithFailure::fresh(Arc::clone(&scenario.store)));
    let servidor = scenario.boot(porta.clone()).await;

    let (status, _) = servidor
        .attendant_text(
            "txn-instavel",
            ROOM,
            "$evento-instavel:matrix-dev.local",
            "oi",
        )
        .await;
    assert_eq!(
        status,
        StatusHttp::SERVICE_UNAVAILABLE,
        "erro transitório nosso é 5xx (o Synapse reenvia), nunca 4xx"
    );

    porta.fix();
    let (status, _) = servidor
        .attendant_text(
            "txn-instavel",
            ROOM,
            "$evento-instavel:matrix-dev.local",
            "oi",
        )
        .await;
    assert_eq!(status, StatusHttp::OK, "a mesma transação é reprocessada");

    wait("o envio sair depois do reenvio", || {
        scenario.sent(scenario.org) == 1
    })
    .await;
    assert_eq!(scenario.whatsapp_sends().len(), 1);
}

#[tokio::test]
async fn file_synapse_no_longer_delivers_becomes_notice_not_sent_to_customer() {
    let scenario = Scenario::new("midia").await;
    let porta = Arc::new(HarnessResponsePort::fresh(Arc::clone(&scenario.store)));
    let servidor = scenario.boot(porta).await;

    let (status, _) = servidor
        .transaction(
            "txn-midia",
            &json!({
                "events": [{
                    "type": "m.room.message",
                    "room_id": ROOM,
                    "sender": ATTENDANT,
                    "event_id": "$evento-midia:matrix-dev.local",
                    "content": {
                        "msgtype": "m.image",
                        "body": "foto.png",
                        "url": "mxc://matrix-dev.local/nao-existe"
                    }
                }]
            }),
        )
        .await;
    assert_eq!(status, StatusHttp::OK);

    wait("o aviso de mídia aparecer na sala", || {
        !scenario.synapse_writes().is_empty()
    })
    .await;
    let writes = scenario.synapse_writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(
        writes[0].body.get("body").and_then(Value::as_str),
        Some("Não consegui enviar o arquivo ao cliente: não consegui baixar o arquivo da sala")
    );
    assert_eq!(
        writes[0].body.get("msgtype").and_then(Value::as_str),
        Some("m.notice")
    );
    assert_eq!(
        scenario.whatsapp_sends().len(),
        0,
        "sem o arquivo, nada sai para o cliente"
    );
    assert!(scenario.messages(scenario.org).is_empty());
    assert_eq!(
        scenario.status(scenario.org),
        poph_store::ConversationStatus::Open,
        "sem arquivo, a conversa não é transferida"
    );
}

#[tokio::test]
async fn attendant_name_is_read_once_and_cached() {
    let scenario = Scenario::new("cache-nome").await;
    let porta = Arc::new(HarnessResponsePort::fresh(Arc::clone(&scenario.store)));
    let servidor = scenario.boot(porta).await;

    let (status, _) = servidor
        .attendant_text("txn-n1", ROOM, "$evento-n1:matrix-dev.local", "primeira")
        .await;
    assert_eq!(status, StatusHttp::OK);
    let (status, _) = servidor
        .attendant_text("txn-n2", ROOM, "$evento-n2:matrix-dev.local", "segunda")
        .await;
    assert_eq!(status, StatusHttp::OK);

    wait("os dois envios saírem", || {
        scenario.whatsapp_sends().len() == 2
    })
    .await;
    assert_eq!(
        scenario.synapse.profiles_read.load(Ordering::SeqCst),
        1,
        "o perfil do mesmo atendente é lido uma vez só dentro da validade"
    );
    let sends = scenario.whatsapp_sends();
    assert!(
        texts_sent(&sends)
            .iter()
            .all(|text| text.starts_with("*[Ana Atendente]*:\n"))
    );
    assert!(servidor.delivery.name_ttl() > Duration::ZERO);
}

#[tokio::test]
async fn conversation_without_agent_does_not_fetch_media_or_invent_signature() {
    let scenario = Scenario::new("sem-agente").await;
    let porta = Arc::new(PortWithoutAgent {
        real: HarnessResponsePort::fresh(Arc::clone(&scenario.store)),
    });
    let servidor = scenario.boot(porta).await;

    let (status, _) = servidor
        .transaction(
            "txn-sem-agente",
            &json!({
                "events": [{
                    "type": "m.room.message",
                    "room_id": ROOM,
                    "sender": ATTENDANT,
                    "event_id": "$sem-agente:matrix-dev.local",
                    "content": {
                        "msgtype": "m.image",
                        "body": "foto.png",
                        "url": "mxc://matrix-dev.local/sem-agente"
                    }
                }]
            }),
        )
        .await;
    assert_eq!(
        status,
        StatusHttp::OK,
        "sem assinatura é falha relatada, não erro de transporte"
    );

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        scenario.synapse_writes().is_empty(),
        "sem fantasma do agente não há aviso na sala: {:?}",
        scenario.synapse_writes()
    );
    assert!(
        scenario.whatsapp_sends().is_empty(),
        "o arquivo não vai ao cliente"
    );
    assert_eq!(
        servidor
            .delivery
            .counters()
            .unsigned_assets
            .load(Ordering::SeqCst),
        1,
        "o caso é contado para aparecer na operação"
    );
}

#[tokio::test]
async fn synapse_and_harness_tokens_do_not_leak_to_customer_or_room() {
    let scenario = Scenario::new("canario").await;
    let porta = Arc::new(HarnessResponsePort::fresh(Arc::clone(&scenario.store)));
    let servidor = scenario.boot(porta).await;

    let (status, body) = servidor
        .attendant_text(
            "txn-canario",
            ROOM,
            "$evento-canario:matrix-dev.local",
            "oi",
        )
        .await;
    assert_eq!(status, StatusHttp::OK);
    let reply_text = body.to_string();
    assert!(!reply_text.contains(HS_TOKEN));
    assert!(!reply_text.contains(AS_TOKEN));

    let (status, _) = servidor
        .transaction(
            "txn-canario-midia",
            &json!({
                "events": [{
                    "type": "m.room.message",
                    "room_id": ROOM,
                    "sender": ATTENDANT,
                    "event_id": "$evento-canario-midia:matrix-dev.local",
                    "content": { "msgtype": "m.file", "body": "contrato.pdf", "url": "mxc://x/y" }
                }]
            }),
        )
        .await;
    assert_eq!(status, StatusHttp::OK);

    wait("os envios saírem", || {
        scenario.whatsapp_sends().len() == 1 && !scenario.synapse_writes().is_empty()
    })
    .await;

    let for_customer =
        serde_json::to_string(&scenario.whatsapp_sends()).expect("serializa o envio");
    assert!(!for_customer.contains(HS_TOKEN));
    assert!(!for_customer.contains(AS_TOKEN));

    let writes = scenario.synapse_writes();
    let for_synapse = writes
        .iter()
        .map(|write| {
            format!(
                "{} {} {} {} {}",
                write.room,
                write.tipo,
                write.txn,
                write.disfarcado.clone().unwrap_or_default(),
                serde_json::to_string(&write.body).unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!for_synapse.contains(HS_TOKEN));
    assert!(
        !for_synapse.contains(AS_TOKEN),
        "o token vai só no cabeçalho"
    );
    assert!(
        writes[0]
            .authorization
            .as_deref()
            .is_some_and(|value| value == format!("Bearer {AS_TOKEN}")),
        "o token do Harness viaja no cabeçalho Authorization"
    );
    let ghost = scenario.agent_ghost();
    assert_eq!(
        writes[0].disfarcado.as_deref(),
        Some(ghost.as_str()),
        "o aviso é escrito em nome do fantasma do agente, nunca do atendente humano"
    );
    assert!(
        !ghost.starts_with(ATTENDANT),
        "sanidade: o fantasma do agente é do namespace do AS, e o atendente não"
    );
}
