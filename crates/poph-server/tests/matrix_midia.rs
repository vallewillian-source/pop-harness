use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use poph_matrix::asset::{
    self, ASSET_BYTES_LIMIT, ATTACHMENT_BLOCK, MSGTYPE_FILE, MSGTYPE_IMAGE, PREFIX_NOTICE,
};
use poph_matrix::error::CallError;
use poph_matrix::format::Outbound;
use poph_matrix::rooms::{ConversationToRoom, RoomError};
use poph_matrix::users::Ghost;
use poph_server::matrix_mirror::{
    AssetSend, MatrixMirror, MatrixTarget, PublishedAsset, RoomNotice,
};
use poph_store::{
    Direction, NewAgent, NewChannelAttachment, NewConversation, NewMessage, NewOrg, Origin, Role,
    Store,
};
use poph_types::{AgentId, ConversationId, MatrixConfig, OrgId};
use serde_json::Value;

struct Scenario {
    dir: PathBuf,
    blobs: PathBuf,
    store: Option<Arc<Store>>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("poph-midia-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("cria a pasta do teste");
        let blobs = dir.join("blobs");
        fs::create_dir_all(&blobs).expect("cria a pasta de blobs");
        let clock = Arc::new(AtomicI64::new(1_000));
        let store = Store::open_at(&dir.join("harness.db"), &dir.join("rag.db"))
            .expect("abre o banco")
            .with_clock(Arc::new(move || clock.fetch_add(1, Ordering::SeqCst)));
        Self {
            dir,
            blobs,
            store: Some(Arc::new(store)),
        }
    }

    fn store(&self) -> Arc<Store> {
        Arc::clone(self.store.as_ref().expect("store viva"))
    }

    fn attachment(&self, org: OrgId, name: &str, mime: &str, bytes: &[u8]) -> NewChannelAttachment {
        let recorded = poph_files::blob::write(&self.blobs, org, bytes).expect("grava o blob");
        NewChannelAttachment {
            name: name.to_owned(),
            mime: mime.to_owned(),
            size_bytes: bytes.len() as u64,
            sha256: recorded.sha256.clone(),
            blob_path: format!("{org}/{}", recorded.sha256),
            extracted_text: None,
            extraction_status: "skipped",
        }
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.dir);
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

fn customer_msg(
    store: &Store,
    org: OrgId,
    conversation: ConversationId,
    text: &str,
    anexos: &[NewChannelAttachment],
) -> poph_types::MessageId {
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
            anexos,
        )
        .expect("grava a mensagem")
        .id
}

#[derive(Debug, Clone)]
struct MediaDelivered {
    room: String,
    txn: String,
    sender: String,
    mxc: String,
    content: Value,
    bytes: usize,
}

impl MediaDelivered {
    fn msgtype(&self) -> &str {
        self.content["msgtype"].as_str().unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
struct NoticeDelivered {
    room: String,
    txn: String,
    sender: String,
    body: String,
}

#[derive(Debug, Clone, Default)]
enum Refusal {
    #[default]
    Nothing,
    DefinitiveMedia(usize),
    TransientMedia(usize),
}

#[derive(Default)]
struct Fake {
    media_items: Mutex<Vec<MediaDelivered>>,
    notices: Mutex<Vec<NoticeDelivered>>,
    texts: Mutex<Vec<(String, String, String, String)>>,
    tentativas: Mutex<HashMap<String, u32>>,
    refusal: Mutex<Refusal>,
    rooms: AtomicUsize,
    ordem: Mutex<Vec<String>>,
}

impl Fake {
    fn new(refusal: Refusal) -> Arc<Self> {
        let fake = Self {
            refusal: Mutex::new(refusal),
            ..Self::default()
        };
        Arc::new(fake)
    }

    fn media_items(&self) -> Vec<MediaDelivered> {
        self.media_items.lock().unwrap().clone()
    }

    fn notices(&self) -> Vec<NoticeDelivered> {
        self.notices.lock().unwrap().clone()
    }

    fn texts(&self) -> Vec<(String, String, String, String)> {
        self.texts.lock().unwrap().clone()
    }

    fn ordem(&self) -> Vec<String> {
        self.ordem.lock().unwrap().clone()
    }

    fn attempts_of(&self, txn: &str) -> u32 {
        *self.tentativas.lock().unwrap().get(txn).unwrap_or(&0)
    }

    fn decide_media(&self) -> Result<(), CallError> {
        let mut refusal = self.refusal.lock().unwrap();
        match &mut *refusal {
            Refusal::Nothing => Ok(()),
            Refusal::DefinitiveMedia(times) if *times > 0 => {
                *times -= 1;
                Err(refused())
            }
            Refusal::TransientMedia(times) if *times > 0 => {
                *times -= 1;
                Err(transient())
            }
            _ => Ok(()),
        }
    }
}

fn transient() -> CallError {
    CallError::Response {
        operation: "send",
        status: 503,
        errcode: Some("M_UNKNOWN".to_owned()),
        message: "indisponível".to_owned(),
        retry_after_ms: None,
    }
}

fn refused() -> CallError {
    CallError::Response {
        operation: "send",
        status: 403,
        errcode: Some("M_FORBIDDEN".to_owned()),
        message: "recusado".to_owned(),
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
        self.0.rooms.fetch_add(1, Ordering::SeqCst);
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
        room: &str,
        send: &Outbound,
    ) -> impl std::future::Future<Output = Result<(), CallError>> + Send {
        let fake = &self.0;
        let room = room.to_owned();
        let txn = send.txn_id.clone();
        let body = send.body.clone();
        let sender = send.sender.user_id().to_owned();
        *fake
            .tentativas
            .lock()
            .unwrap()
            .entry(txn.clone())
            .or_insert(0) += 1;
        let mut texts = fake.texts.lock().unwrap();
        if !texts.iter().any(|(_, t, _, _)| *t == txn) {
            texts.push((room, txn.clone(), body, sender));
            fake.ordem.lock().unwrap().push(format!("texto:{txn}"));
        }
        drop(texts);
        async move { Ok(()) }
    }

    fn send_asset(
        &self,
        room: &str,
        send: &AssetSend,
    ) -> impl std::future::Future<Output = Result<PublishedAsset, CallError>> + Send {
        let fake = &self.0;
        let room = room.to_owned();
        let txn = send.asset.txn_id.clone();
        *fake
            .tentativas
            .lock()
            .unwrap()
            .entry(txn.clone())
            .or_insert(0) += 1;
        let resultado = fake.decide_media();
        let sender = send.sender.user_id().to_owned();
        let mxc = format!("mxc://matrix-dev.local/{}", txn);
        let content = asset::event_content(&mxc, &send.asset);
        let mut media_items = fake.media_items.lock().unwrap();
        let new = !media_items.iter().any(|m| m.room == room && m.txn == txn);
        if resultado.is_ok() && new {
            fake.ordem.lock().unwrap().push(format!("midia:{txn}"));
            media_items.push(MediaDelivered {
                room: room.clone(),
                txn: txn.clone(),
                sender,
                mxc: mxc.clone(),
                content,
                bytes: send.content.len(),
            });
        }
        drop(media_items);
        let published = PublishedAsset {
            event: format!("$midia-{txn}:matrix-dev.local"),
            mxc,
        };
        async move { resultado.map(|()| published) }
    }

    fn send_notice(
        &self,
        room: &str,
        notice: &RoomNotice,
    ) -> impl std::future::Future<Output = Result<(), CallError>> + Send {
        let fake = &self.0;
        let room = room.to_owned();
        let txn = notice.txn_id.clone();
        let sender = notice.sender.user_id().to_owned();
        let body = asset::notice_body(notice.reason);
        let mut notices = fake.notices.lock().unwrap();
        if !notices.iter().any(|a| a.room == room && a.txn == txn) {
            fake.ordem.lock().unwrap().push(format!("aviso:{txn}"));
            notices.push(NoticeDelivered {
                room,
                txn,
                sender,
                body,
            });
        }
        drop(notices);
        async move { Ok(()) }
    }
}

fn mirror(scenario: &Scenario, fake: Arc<Fake>) -> MatrixMirror<Signal> {
    MatrixMirror::new(scenario.store(), Signal(fake), &config())
        .expect("monta o espelho")
        .with_blob_folder(scenario.blobs.clone())
}

fn room_of(conversation: ConversationId) -> String {
    format!("!{conversation}:matrix-dev.local")
}

fn customer_ghost(peer: &str) -> String {
    format!("@ph_wa_{peer}:matrix-dev.local")
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nimagem-de-teste";
const PDF: &[u8] = b"%PDF-1.4\nlaudo de teste\n%%EOF\n";

fn cursor(store: &Store, org: OrgId) -> (u64, String) {
    let cursors = store.matrix().read_cursors(org).expect("lê os cursores");
    (cursors.messages, cursors.message_ids)
}

fn max_rowid_and_id(store: &Store) -> (u64, String) {
    let (rowid, id) = store
        .messages()
        .last_rowid()
        .expect("lê a última rowid")
        .expect("tem mensagem");
    (rowid, id.to_string())
}

fn block_id(asset: &MediaDelivered) -> String {
    asset.content[ATTACHMENT_BLOCK]["id"]
        .as_str()
        .expect("o bloco leva o id")
        .to_owned()
}

#[tokio::test]
async fn customer_image_and_pdf_reach_room_after_caption() {
    let scenario = Scenario::new("imagem-pdf");
    let store = scenario.store();
    let org = create_org(&store, "a");
    let agent = create_agent(&store, org);
    let peer = "5521988887777";
    let conversation = create_conversation(&store, org, agent, peer);
    store.matrix().enable(org).expect("liga");

    let image = scenario.attachment(org, "foto.jpg", "image/jpeg", PNG);
    let pdf = scenario.attachment(org, "laudo.pdf", "application/pdf", PDF);
    let with_image = customer_msg(&store, org, conversation, "olha a foto", &[image]);
    let with_pdf = customer_msg(&store, org, conversation, "segue o laudo", &[pdf]);

    let fake = Fake::new(Refusal::Nothing);
    let report = mirror(&scenario, Arc::clone(&fake))
        .cycle()
        .await
        .expect("ciclo");

    let media_items = fake.media_items();
    assert_eq!(media_items.len(), 2, "uma imagem e um PDF:");
    assert_eq!(report.assets_sent, 2);
    assert_eq!(report.sent, 2, "as duas legendas");

    assert_eq!(media_items[0].msgtype(), MSGTYPE_IMAGE);
    assert_eq!(media_items[0].sender, customer_ghost(peer));
    assert_eq!(media_items[0].room, room_of(conversation));
    assert_eq!(
        media_items[0].content["body"],
        Value::String("foto.jpg".into())
    );
    assert_eq!(
        media_items[0].content["url"],
        Value::String(media_items[0].mxc.clone())
    );
    assert_eq!(
        media_items[0].content["info"]["mimetype"],
        Value::String("image/jpeg".into())
    );
    assert_eq!(media_items[0].bytes, PNG.len());
    let block = &media_items[0].content[ATTACHMENT_BLOCK];
    assert_eq!(block["source"], Value::String("whatsapp".into()));
    assert!(
        block["id"].as_str().is_some_and(|id| !id.is_empty()),
        "o bloco leva o id do anexo: {block}"
    );

    assert_eq!(media_items[1].msgtype(), MSGTYPE_FILE);
    assert_eq!(
        media_items[1].content["info"]["mimetype"],
        Value::String("application/pdf".into())
    );
    assert_eq!(media_items[1].bytes, PDF.len());

    let texts = fake.texts();
    assert_eq!(texts.len(), 2);
    assert_eq!(texts[0].2, "olha a foto");
    assert_eq!(texts[0].1, format!("poph-msg-{with_image}"));
    assert_eq!(texts[1].2, "segue o laudo");
    assert_eq!(texts[1].1, format!("poph-msg-{with_pdf}"));

    let ordem = fake.ordem();
    assert_eq!(
        ordem,
        vec![
            format!("midia:{}", asset::asset_txn(&block_id(&media_items[0]))),
            format!("texto:poph-msg-{with_image}"),
            format!("midia:{}", asset::asset_txn(&block_id(&media_items[1]))),
            format!("texto:poph-msg-{with_pdf}"),
        ]
    );

    assert!(fake.notices().is_empty(), "nada recusado");
    assert_eq!(cursor(&store, org), max_rowid_and_id(&store));

    let registered = store
        .matrix_asset()
        .pending(org, i64::MAX, i64::MAX, 10)
        .expect("lê o registro da mídia");
    assert_eq!(registered.len(), 2, "as duas mídias registradas");
    for (asset, registered) in media_items.iter().zip(&registered) {
        assert_eq!(registered.room, asset.room);
        assert_eq!(
            registered.evento,
            format!("$midia-{}:matrix-dev.local", asset.txn)
        );
        assert_eq!(registered.mxc, asset.mxc);
        assert_eq!(
            registered.nome,
            asset.content["body"].as_str().unwrap_or_default()
        );
        assert_eq!(registered.agent, agent.to_string());
        assert_eq!(
            registered.category,
            if asset.msgtype() == MSGTYPE_IMAGE {
                poph_store::CategoryAsset::Image
            } else {
                poph_store::CategoryAsset::Document
            },
            "categoria da mídia registrada"
        );
        assert_eq!(registered.size_bytes, asset.bytes as u64);
        assert!(
            registered.still_to_draft() && registered.still_to_delete(),
            "recém-publicada: as duas etapas pendentes"
        );
    }
}

#[tokio::test]
async fn reprocessing_same_media_does_not_duplicate_event() {
    let scenario = Scenario::new("idempotente");
    let store = scenario.store();
    let org = create_org(&store, "b");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "5511911112222");
    store.matrix().enable(org).expect("liga");
    let image = scenario.attachment(org, "foto.png", "image/png", PNG);
    customer_msg(&store, org, conversation, "foto", &[image]);

    let fake = Fake::new(Refusal::TransientMedia(1));
    let case_mirror = mirror(&scenario, Arc::clone(&fake));
    let first = case_mirror.cycle().await.expect("ciclo 1");
    assert_eq!(first.assets_sent, 0);
    assert_eq!(first.pending, 1, "a mídia ficou pendente");
    assert!(fake.media_items().is_empty());
    let (pending_rowid, _) = cursor(&store, org);
    assert_eq!(pending_rowid, 0, "o cursor não passou da mídia pendente");

    let second = case_mirror.cycle().await.expect("ciclo 2");
    assert_eq!(second.assets_sent, 1);
    let media_items = fake.media_items();
    assert_eq!(
        media_items.len(),
        1,
        "um evento só, apesar das duas passagens"
    );
    assert_eq!(
        fake.attempts_of(&media_items[0].txn),
        2,
        "as duas passagens usaram o mesmo `txnId` (o Synapse não duplicaria)"
    );

    let third = case_mirror.cycle().await.expect("ciclo 3");
    assert_eq!(third.assets_sent, 0);
    assert_eq!(fake.media_items().len(), 1);
    assert_eq!(cursor(&store, org), max_rowid_and_id(&store));
}

#[tokio::test]
async fn file_over_limit_becomes_notice_and_cursor_moves_on() {
    let scenario = Scenario::new("grande");
    let store = scenario.store();
    let org = create_org(&store, "c");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "5511933334444");
    store.matrix().enable(org).expect("liga");

    let mut large = scenario.attachment(org, "video.mp4", "video/mp4", PNG);
    large.size_bytes = ASSET_BYTES_LIMIT + 1;
    customer_msg(&store, org, conversation, "olha o vídeo", &[large]);
    customer_msg(&store, org, conversation, "e depois isso", &[]);

    let fake = Fake::new(Refusal::Nothing);
    let report = mirror(&scenario, Arc::clone(&fake))
        .cycle()
        .await
        .expect("ciclo");

    assert!(
        fake.media_items().is_empty(),
        "nada de mídia acima do limite"
    );
    let notices = fake.notices();
    assert_eq!(notices.len(), 1, "um aviso na sala");
    assert!(
        notices[0].body.starts_with(PREFIX_NOTICE),
        "aviso: {}",
        notices[0].body
    );
    assert!(
        notices[0].body.contains("16 MB"),
        "aviso: {}",
        notices[0].body
    );
    assert_eq!(report.asset_notices, 1);
    assert_eq!(
        notices[0].sender,
        format!("@ph_ag_{agent}:matrix-dev.local"),
        "o aviso é do fantasma do agente, dono da sala"
    );
    assert_eq!(report.sent, 2, "as duas legendas passaram");
    assert_eq!(cursor(&store, org), max_rowid_and_id(&store));
}

#[tokio::test]
async fn attachment_without_disk_blob_becomes_notice_without_blocking_cursor() {
    let scenario = Scenario::new("sem-blob");
    let store = scenario.store();
    let org = create_org(&store, "d");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "5511955556666");
    store.matrix().enable(org).expect("liga");

    let attachment = scenario.attachment(org, "foto.jpg", "image/jpeg", PNG);
    let path = scenario
        .blobs
        .join(org.to_string())
        .join(&attachment.sha256);
    fs::remove_file(&path).expect("apaga o blob");
    customer_msg(&store, org, conversation, "olha", &[attachment]);

    let fake = Fake::new(Refusal::Nothing);
    let report = mirror(&scenario, Arc::clone(&fake))
        .cycle()
        .await
        .expect("ciclo");

    assert!(fake.media_items().is_empty());
    let notices = fake.notices();
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0].body.contains("não pôde ser lido"),
        "aviso: {}",
        notices[0].body
    );
    assert_eq!(report.pending, 0);
    assert_eq!(cursor(&store, org), max_rowid_and_id(&store));
}

#[tokio::test]
async fn definitively_refused_media_leaves_notice_before_isolation() {
    let scenario = Scenario::new("recusada");
    let store = scenario.store();
    let org = create_org(&store, "e");
    let agent = create_agent(&store, org);
    let conversation = create_conversation(&store, org, agent, "5511977778888");
    store.matrix().enable(org).expect("liga");
    let pdf = scenario.attachment(org, "laudo.pdf", "application/pdf", PDF);
    customer_msg(&store, org, conversation, "laudo", &[pdf]);

    let fake = Fake::new(Refusal::DefinitiveMedia(u32::MAX as usize));
    let case_mirror = mirror(&scenario, Arc::clone(&fake));
    let mut cycles = 0;
    while cursor(&store, org).0 == 0 && cycles < 10 {
        case_mirror.cycle().await.expect("ciclo");
        cycles += 1;
    }

    let notices = fake.notices();
    assert_eq!(notices.len(), 1, "o aviso entrou uma vez só: {notices:?}");
    assert!(
        notices[0].body.contains("recusou o arquivo"),
        "aviso: {}",
        notices[0].body
    );
    assert_eq!(cursor(&store, org), max_rowid_and_id(&store));
    let failures = store.matrix().list_failures(org, 10).expect("lê as falhas");
    assert_eq!(failures.len(), 1, "uma mensagem isolada: {failures:?}");
    assert_eq!(failures[0].motivo, "M_FORBIDDEN");
    assert_eq!(cycles, 3, "as tentativas definitivas do laço");
}
