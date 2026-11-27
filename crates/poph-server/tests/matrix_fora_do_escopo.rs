use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use poph_matrix::error::CallError;
use poph_matrix::format::Outbound;
use poph_matrix::rooms::{ConversationToRoom, RoomError};
use poph_matrix::users::Ghost;
use poph_server::matrix_mirror::{
    AssetSend, MatrixMirror, MatrixTarget, PublishedAsset, RoomNotice,
};
use poph_store::{Direction, NewAgent, NewConversation, NewMessage, NewOrg, Origin, Role, Store};
use poph_types::{AgentId, ConversationId, MatrixConfig, OrgId, UnsupportedContent};

struct Scenario {
    folder: PathBuf,
    store: Option<Arc<Store>>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder = std::env::temp_dir().join(format!(
            "poph-fora-do-escopo-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("cria a pasta do teste");
        let clock = Arc::new(AtomicI64::new(1_000));
        let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
            .expect("abre o banco")
            .with_clock(Arc::new(move || clock.fetch_add(1, Ordering::SeqCst)));
        Self {
            folder,
            store: Some(Arc::new(store)),
        }
    }

    fn store(&self) -> Arc<Store> {
        Arc::clone(self.store.as_ref().expect("store viva"))
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.folder);
    }
}

fn config() -> MatrixConfig {
    MatrixConfig {
        enabled: true,
        homeserver_url: "http://127.0.0.1:1".to_owned(),
        server_name: "matrix-dev.local".to_owned(),
        prefix: "ph".to_owned(),
        concurrency: 4,
        interval_ms: 500,
        ..MatrixConfig::default()
    }
}

fn create_org(store: &Store, slug: &str) -> OrgId {
    store
        .orgs()
        .create(NewOrg {
            name: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id
}

fn create_agent(store: &Store, org: OrgId) -> AgentId {
    store
        .agents()
        .create(
            org,
            NewAgent {
                name: "Bia".to_owned(),
                slug: "bia".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente")
        .id
}

fn create_conversation(store: &Store, org: OrgId, agent: AgentId, peer: &str) -> ConversationId {
    store
        .conversations()
        .create(
            org,
            NewConversation {
                agent_id: agent,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: Some(peer.to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa")
        .id
}

fn client_message(store: &Store, org: OrgId, conversation: ConversationId, text: &str) {
    store
        .messages()
        .write_with_attachments(
            org,
            NewMessage {
                conversation_id: conversation,
                direction: Direction::Inbound,
                role: Role::User,
                origin: Origin::Agente,
                author_name: None,
                texto: text.to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
            },
            &[],
        )
        .expect("grava a mensagem");
}

fn agent_message(store: &Store, org: OrgId, conversation: ConversationId, text: &str) {
    store
        .messages()
        .write_with_attachments(
            org,
            NewMessage {
                conversation_id: conversation,
                direction: Direction::Outbound,
                role: Role::Assistant,
                origin: Origin::Agente,
                author_name: None,
                texto: text.to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
            },
            &[],
        )
        .expect("grava a mensagem do agente");
}

fn cursor(store: &Store, org: OrgId) -> (u64, String) {
    let cursors = store.matrix().read_cursors(org).expect("lê os cursores");
    (cursors.messages, cursors.message_ids)
}

fn last_rowid_and_id(store: &Store) -> (u64, String) {
    let (rowid, id) = store
        .messages()
        .last_rowid()
        .expect("lê a última rowid")
        .expect("tem mensagem");
    (rowid, id.to_string())
}

#[derive(Debug, Clone)]
struct DeliveredText {
    txn: String,
    sender: String,
    msgtype: String,
    body: String,
}

#[derive(Default)]
struct Fake {
    texts: Mutex<Vec<DeliveredText>>,
    tentativas: Mutex<Vec<String>>,
    transient_failures: Mutex<usize>,
}

impl Fake {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn texts(&self) -> Vec<DeliveredText> {
        self.texts.lock().unwrap().clone()
    }

    fn attempts_for(&self, txn: &str) -> usize {
        self.tentativas
            .lock()
            .unwrap()
            .iter()
            .filter(|t| t.as_str() == txn)
            .count()
    }
}

fn transient_error() -> CallError {
    CallError::Response {
        operation: "send",
        status: 503,
        errcode: Some("M_UNKNOWN".to_owned()),
        message: "indisponível".to_owned(),
        retry_after_ms: None,
    }
}

struct Signal(Arc<Fake>);

impl MatrixTarget for Signal {
    fn ensure_room(
        &self,
        dados: &ConversationToRoom,
    ) -> impl std::future::Future<Output = Result<String, RoomError>> + Send {
        let room = format!("!{}:matrix-dev.local", dados.conversation);
        async move { Ok(room) }
    }

    async fn ensure_device(
        &self,
        _data: &ConversationToRoom,
        _room: &str,
        _device: &Ghost,
    ) -> Result<(), RoomError> {
        Ok(())
    }

    fn send(
        &self,
        _room: &str,
        send: &Outbound,
    ) -> impl std::future::Future<Output = Result<(), CallError>> + Send {
        let fake = &self.0;
        let txn = send.txn_id.clone();
        fake.tentativas.lock().unwrap().push(txn.clone());
        let resultado = {
            let mut failures = fake.transient_failures.lock().unwrap();
            if *failures > 0 {
                *failures -= 1;
                Err(transient_error())
            } else {
                Ok(())
            }
        };
        if resultado.is_ok() {
            let mut texts = fake.texts.lock().unwrap();
            if !texts.iter().any(|t| t.txn == txn) {
                texts.push(DeliveredText {
                    txn,
                    sender: send.sender.user_id().to_owned(),
                    msgtype: send.msgtype.to_owned(),
                    body: send.body.clone(),
                });
            }
        }
        async move { resultado }
    }

    async fn send_asset(
        &self,
        _room: &str,
        _asset: &AssetSend,
    ) -> Result<PublishedAsset, CallError> {
        Err(CallError::Configuration(
            "sem mídia neste destino".to_owned(),
        ))
    }

    async fn send_notice(&self, _room: &str, _notice: &RoomNotice) -> Result<(), CallError> {
        Ok(())
    }
}

fn mirror(scenario: &Scenario, fake: Arc<Fake>) -> MatrixMirror<Signal> {
    MatrixMirror::new(scenario.store(), Signal(fake), &config()).expect("monta o espelho")
}

#[tokio::test]
async fn six_types_become_notice_in_room_and_next_caption_passes() {
    let scenario = Scenario::new("seis-tipos");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "5511977770001");
    store.matrix().enable(org).expect("liga o módulo");

    for tipo in UnsupportedContent::ALL_VALUES {
        client_message(&store, org, conversation, tipo.history_text());
    }
    client_message(
        &store,
        org,
        conversation,
        "tudo bem, pode me responder por texto?",
    );

    let fake = Fake::new();
    let report = mirror(&scenario, Arc::clone(&fake))
        .cycle()
        .await
        .expect("ciclo");

    let texts = fake.texts();
    assert_eq!(
        texts.len(),
        7,
        "seis avisos e a legenda de texto: {texts:?}"
    );
    for (delivered, tipo) in texts.iter().zip(UnsupportedContent::ALL_VALUES) {
        assert_eq!(delivered.msgtype, "m.notice", "{tipo:?}");
        assert_eq!(delivered.body, tipo.room_notice(), "{tipo:?}");
        assert_eq!(
            delivered.sender,
            format!("@ph_ag_{agent}:matrix-dev.local"),
            "o aviso é do fantasma do agente, dono da sala"
        );
    }
    let last = &texts[6];
    assert_eq!(
        last.msgtype, "m.text",
        "a mensagem seguinte entra como texto"
    );
    assert_eq!(last.body, "tudo bem, pode me responder por texto?");
    assert_eq!(
        last.sender, "@ph_wa_5511977770001:matrix-dev.local",
        "a legenda de texto sai pelo fantasma do cliente, como antes"
    );
    assert_eq!(report.sent, 7);
    assert_eq!(cursor(&store, org), last_rowid_and_id(&store));
}

#[tokio::test]
async fn reprocessing_notice_does_not_duplicate_in_room() {
    let scenario = Scenario::new("reprocessa");
    let store = scenario.store();
    let org = create_org(&store, "b");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "5511977770002");
    store.matrix().enable(org).expect("liga o módulo");
    client_message(
        &store,
        org,
        conversation,
        UnsupportedContent::Audio.history_text(),
    );

    let fake = Fake::new();
    *fake.transient_failures.lock().unwrap() = 1;
    let service = mirror(&scenario, Arc::clone(&fake));
    service.cycle().await.expect("primeiro ciclo");
    assert!(fake.texts().is_empty(), "a primeira tentativa falhou");
    service.cycle().await.expect("segundo ciclo");

    let texts = fake.texts();
    assert_eq!(texts.len(), 1, "um aviso só na sala: {texts:?}");
    assert_eq!(texts[0].msgtype, "m.notice");
    let txn = texts[0].txn.clone();
    assert_eq!(fake.attempts_for(&txn), 2, "a mesma entrega foi repetida");
    assert_eq!(cursor(&store, org), last_rowid_and_id(&store));
}

#[tokio::test]
async fn history_written_by_attendant_stays_as_text() {
    let scenario = Scenario::new("atendente");
    let store = scenario.store();
    let org = create_org(&store, "c");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "5511977770003");
    store.matrix().enable(org).expect("liga o módulo");
    agent_message(
        &store,
        org,
        conversation,
        UnsupportedContent::Audio.history_text(),
    );

    let fake = Fake::new();
    mirror(&scenario, Arc::clone(&fake))
        .cycle()
        .await
        .expect("ciclo");

    let texts = fake.texts();
    assert_eq!(texts.len(), 1, "{texts:?}");
    assert_eq!(texts[0].msgtype, "m.text", "só o cliente vira aviso");
    assert_eq!(texts[0].sender, format!("@ph_ag_{agent}:matrix-dev.local"));
}
