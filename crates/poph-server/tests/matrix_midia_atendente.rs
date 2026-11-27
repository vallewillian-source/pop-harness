use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use poph_evolution::{ATTACHMENT_CAP, ConfigEvolution, EvolutionClient};
use poph_matrix::{ClientConfig, GhostIds, MatrixClient};
use poph_server::evolution::EvolutionChannel;
use poph_server::{AsReceiver, AttendantResponse, HarnessResponsePort, ServerState};
use poph_store::{NewAgent, NewConversation, NewOrg, Origin, OutboxState, Page, Store};
use poph_types::{ConversationId, HarnessConfig, IntegrationId, OrgId, Secret};
use reqwest::{Client, StatusCode as StatusHttp};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const PREFIX: &str = "ph";
const SERVER_NAME: &str = "matrix-dev.local";

const HS_TOKEN: &str = "CANARIO-HS-mx44-7b21-TAO-SECRETO";

const AS_TOKEN: &str = "CANARIO-AS-mx44-c4d8-TAO-SECRETO";

const INSTANCIA: &str = "loja";
const EVO_KEY: &str = "chave-evo-mx44-teste";
const WEBHOOK_SECRET: &str = "segredo-webhook-mx44";

const PHONE: &str = "5511999990000";

const ATTENDANT: &str = "@ana:matrix-dev.local";
const ATTENDANT_NAME: &str = "Ana Atendente";

const ROOM: &str = "!sala-a:matrix-dev.local";

const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

const PDF: &[u8] =
    b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n";

const ZIP: &[u8] = b"PK\x03\x04\x14\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";

const AUDIO_OGG: &[u8] = &[
    0x4f, 0x67, 0x67, 0x53, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x4f, 0x70, 0x75, 0x73, 0x48, 0x65, 0x61, 0x64, 0x01, 0x02, 0x38, 0x01,
    0x80, 0xbb, 0x00, 0x00,
];

const VIDEO_MP4: &[u8] = &[
    0x00, 0x00, 0x00, 0x18, 0x66, 0x74, 0x79, 0x70, 0x69, 0x73, 0x6f, 0x6d, 0x00, 0x00, 0x02, 0x00,
    0x69, 0x73, 0x6f, 0x6d, 0x69, 0x73, 0x6f, 0x32,
];

#[derive(Clone, Debug)]
struct Write {
    room: String,
    tipo: String,
    body: Value,
    masquerade: Option<String>,
}

const NAMESPACE: &str = "@ph_";

fn can_impersonate(user: Option<&str>) -> bool {
    match user {
        None => true,
        Some(user) => user.starts_with(NAMESPACE),
    }
}

fn masquerade_refusal() -> (StatusCode, Json<Value>) {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "errcode": "M_FORBIDDEN",
            "error": "Application service cannot masquerade as this user"
        })),
    )
}

#[derive(Clone, Default)]
struct FakeSynapse {
    profiles: Arc<Mutex<HashMap<String, String>>>,
    writes: Arc<Mutex<Vec<Write>>>,
    media_items: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    errors: Arc<Mutex<HashMap<String, u16>>>,
    downloads: Arc<Mutex<Vec<Option<String>>>>,
    masquerade_refusals: Arc<Mutex<u64>>,
}

async fn fake_profile(
    State(fake): State<FakeSynapse>,
    Path(user): Path<String>,
) -> (StatusCode, Json<Value>) {
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
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let masquerade = query.get("user_id").cloned();
    if !can_impersonate(masquerade.as_deref()) {
        *fake.masquerade_refusals.lock().unwrap() += 1;
        return masquerade_refusal();
    }
    let mut writes = fake.writes.lock().unwrap();
    let position = writes.len() + 1;
    writes.push(Write {
        room,
        tipo,
        body,
        masquerade,
    });
    let _ = txn;
    (
        StatusCode::OK,
        Json(json!({ "event_id": format!("$escrito-{position}:matrix-dev.local") })),
    )
}

async fn fetch_fake_new(
    state: State<FakeSynapse>,
    path: Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    fetch_fake(state, path, Query(query)).await
}

async fn fetch_fake(
    State(fake): State<FakeSynapse>,
    Path((_server, id)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let masquerade = query.get("user_id").cloned();
    fake.downloads.lock().unwrap().push(masquerade.clone());
    if !can_impersonate(masquerade.as_deref()) {
        *fake.masquerade_refusals.lock().unwrap() += 1;
        return masquerade_refusal().into_response();
    }
    if let Some(status) = fake.errors.lock().unwrap().get(&id).copied() {
        return (
            StatusCode::from_u16(status).expect("status válido"),
            Json(json!({ "errcode": "M_UNKNOWN", "error": "indisponível" })),
        )
            .into_response();
    }
    let bytes = fake.media_items.lock().unwrap().get(&id).cloned();
    match bytes {
        Some(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/octet-stream")],
            bytes,
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "errcode": "M_NOT_FOUND", "error": "Not found" })),
        )
            .into_response(),
    }
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
        .route(
            "/_matrix/client/v1/media/download/{server}/{id}",
            get(fetch_fake_new),
        )
        .route("/_matrix/media/v3/download/{server}/{id}", get(fetch_fake))
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
        if client.get(format!("{url}/pronto")).send().await.is_ok() {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "Synapse falso não subiu"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[derive(Clone, Debug)]
struct Send {
    route: String,
    body: Value,
}

#[derive(Clone, Default)]
struct FakeEvolution {
    sends: Arc<Mutex<Vec<Send>>>,
}

async fn send_via_whatsapp(
    State(fake): State<FakeEvolution>,
    Path((route, _instance)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let mut sends = fake.sends.lock().unwrap();
    sends.push(Send { route, body });
    let position = sends.len();
    (
        StatusCode::OK,
        Json(json!({ "key": { "id": format!("MSG-EVO-{position}") } })),
    )
}

async fn boot_fake_evolution() -> (String, FakeEvolution) {
    let fake = FakeEvolution::default();
    let router = Router::new()
        .route("/message/{rota}/{instance}", post(send_via_whatsapp))
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

struct Scenario {
    dir: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    conversation: ConversationId,
    synapse: FakeSynapse,
    evo: FakeEvolution,
    url_synapse: String,
    url_evo: String,
    blob_dir: PathBuf,
    diretorio: PathBuf,
}

impl Scenario {
    async fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "poph-matrix-midia-atendente-{label}-{}-{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("pasta do teste");
        let store = Arc::new(
            Store::open_at(&dir.join("harness.db"), &dir.join("rag.db")).expect("abre o banco"),
        );

        let org = store
            .orgs()
            .create(NewOrg {
                name: "Org do cenário".to_owned(),
                slug: "loja-a".to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agent = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Agente de Teste".to_owned(),
                    slug: "agente-teste".to_owned(),
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

        store.matrix().enable(org).expect("liga o módulo na org");
        store
            .matrix()
            .ensure_room(org, conversation, ROOM, None)
            .expect("grava a sala");
        store
            .matrix()
            .mark_room_ready(org, conversation)
            .expect("sala pronta");

        let (url_synapse, synapse) = boot_fake_synapse().await;
        wait_synapse(&url_synapse).await;
        let (url_evo, evo) = boot_fake_evolution().await;
        let diretorio = dir.join("dados");
        Self {
            blob_dir: diretorio.join("blobs"),
            diretorio,
            dir,
            store,
            org,
            conversation,
            synapse,
            evo,
            url_synapse,
            url_evo,
        }
    }

    fn refuse(&self, id: &str, status: u16) {
        self.synapse
            .errors
            .lock()
            .unwrap()
            .insert(id.to_owned(), status);
    }

    fn publicar(&self, id: &str, bytes: &[u8]) -> String {
        self.synapse
            .media_items
            .lock()
            .unwrap()
            .insert(id.to_owned(), bytes.to_vec());
        format!("mxc://{SERVER_NAME}/{id}")
    }

    fn channel(&self) -> EvolutionChannel {
        let client = EvolutionClient::new(
            ConfigEvolution::new(
                self.url_evo.clone(),
                INSTANCIA,
                Secret::new(EVO_KEY.to_owned()),
            )
            .expect("config da Evolution falsa"),
        )
        .expect("cliente da Evolution falsa");
        let integracao = self
            .store
            .conversations()
            .get(self.org, self.conversation)
            .expect("lê a conversa")
            .integration_id
            .expect("a conversa tem integração");
        EvolutionChannel {
            org: self.org,
            instance: INSTANCIA.to_owned(),
            integration: integracao,
            secret: Some(Secret::new(WEBHOOK_SECRET.to_owned())),
            client: Some(client),
            attachment_limit: ATTACHMENT_CAP,
        }
    }

    async fn boot(&self) -> Servidor {
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
            AttendantResponse::new(
                Arc::new(HarnessResponsePort::fresh(Arc::clone(&self.store))),
                Some(matrix_customer),
                ids,
            )
            .with_blob_folder(self.blob_dir.clone()),
        );

        let mut config = HarnessConfig::default();
        config.data.directory = self.diretorio.clone();
        let state = ServerState::new(Arc::clone(&self.store), config)
            .expect("estado do servidor")
            .with_evolution_channels(vec![self.channel()])
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

    fn messages(&self) -> Vec<poph_store::Mensagem> {
        self.store
            .messages()
            .list(self.org, self.conversation, Page::with_limit(50))
            .expect("lista as mensagens")
    }

    fn anexos(&self, index: usize) -> Vec<poph_store::AttachmentRecord> {
        let messages = self.messages();
        self.store
            .messages()
            .attachments(self.org, messages[index].id)
            .expect("lista os anexos")
    }

    fn files_in_queue(&self) -> Vec<poph_store::OutboxItem> {
        self.store
            .outbox()
            .list_files_of_conversation(self.org, self.conversation, 50)
            .expect("lista os itens de arquivo da fila")
    }

    fn sends(&self) -> Vec<Send> {
        self.evo.sends.lock().unwrap().clone()
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

    fn downloads(&self) -> Vec<Option<String>> {
        self.synapse.downloads.lock().unwrap().clone()
    }

    fn masquerade_refusals(&self) -> u64 {
        *self.synapse.masquerade_refusals.lock().unwrap()
    }

    fn notices_in_room(&self) -> Vec<Write> {
        self.synapse.writes.lock().unwrap().clone()
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

    async fn transaction(&self, txn: &str, body: &Value) -> StatusHttp {
        let reply = self
            .client
            .put(format!("{}/_matrix/app/v1/transactions/{txn}", self.base))
            .header("authorization", format!("Bearer {HS_TOKEN}"))
            .json(body)
            .send()
            .await
            .expect("entrega a transação");
        reply.status()
    }

    async fn attendant_file(
        &self,
        txn: &str,
        event: &str,
        msgtype: &str,
        name: &str,
        url: Option<&str>,
    ) -> StatusHttp {
        let mut content = json!({ "msgtype": msgtype, "body": name, "filename": name });
        if let Some(url) = url {
            content["url"] = json!(url);
        }
        self.transaction(
            txn,
            &json!({
                "events": [{
                    "type": "m.room.message",
                    "room_id": ROOM,
                    "sender": ATTENDANT,
                    "event_id": event,
                    "origin_server_ts": 1_795_300_000_000i64,
                    "content": content
                }]
            }),
        )
        .await
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

fn decode_base64(text: &str) -> Vec<u8> {
    fn value(character: u8) -> Option<u8> {
        match character {
            b'A'..=b'Z' => Some(character - b'A'),
            b'a'..=b'z' => Some(character - b'a' + 26),
            b'0'..=b'9' => Some(character - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut saida = Vec::new();
    let mut group = [0u8; 4];
    let mut size = 0;
    for byte in text.bytes() {
        if byte == b'=' || byte.is_ascii_whitespace() {
            continue;
        }
        group[size] = value(byte).unwrap_or_else(|| panic!("base64 inválido: {byte}"));
        size += 1;
        if size == 4 {
            saida.push((group[0] << 2) | (group[1] >> 4));
            saida.push((group[1] << 4) | (group[2] >> 2));
            saida.push((group[2] << 6) | group[3]);
            size = 0;
        }
    }
    match size {
        2 => saida.push((group[0] << 2) | (group[1] >> 4)),
        3 => {
            saida.push((group[0] << 2) | (group[1] >> 4));
            saida.push((group[1] << 4) | (group[2] >> 2));
        }
        _ => {}
    }
    saida
}

fn single_notice(scenario: &Scenario) -> String {
    let notices = scenario.notices_in_room();
    assert_eq!(notices.len(), 1, "esperava um aviso na sala: {notices:?}");
    assert_eq!(
        notices[0].room, ROOM,
        "o aviso vai para a sala do atendente"
    );
    assert_eq!(notices[0].tipo, "m.room.message");
    assert_eq!(notices[0].body["msgtype"], json!("m.notice"));
    assert_eq!(
        notices[0].masquerade.as_deref(),
        Some(scenario.agent_ghost().as_str()),
        "o aviso é assinado pelo fantasma do agente, nunca pelo atendente humano"
    );
    assert_eq!(
        scenario.masquerade_refusals(),
        0,
        "nenhum pedido pode ter sido recusado por disfarce fora do namespace do AS"
    );
    notices[0].body["body"]
        .as_str()
        .expect("corpo do aviso é texto")
        .to_owned()
}

#[tokio::test]
async fn element_image_and_pdf_reach_whatsapp_with_attendant_caption() {
    let scenario = Scenario::new("ok").await;
    let servidor = scenario.boot().await;
    let url_png = scenario.publicar("png-mx44", PNG);
    let url_pdf = scenario.publicar("pdf-mx44", PDF);

    let status = servidor
        .transaction(
            "txn-midia",
            &json!({
                "events": [
                    {
                        "type": "m.room.message",
                        "room_id": ROOM,
                        "sender": ATTENDANT,
                        "event_id": "$img-mx44:matrix-dev.local",
                        "content": {
                            "msgtype": "m.image",
                            "body": "foto.png",
                            "filename": "foto.png",
                            "url": url_png,
                            "info": { "mimetype": "image/png", "size": PNG.len() }
                        }
                    },
                    {
                        "type": "m.room.message",
                        "room_id": ROOM,
                        "sender": ATTENDANT,
                        "event_id": "$pdf-mx44:matrix-dev.local",
                        "content": {
                            "msgtype": "m.file",
                            "body": "laudo.pdf",
                            "filename": "laudo.pdf",
                            "url": url_pdf,
                            "info": { "mimetype": "application/pdf", "size": PDF.len() }
                        }
                    }
                ]
            }),
        )
        .await;
    assert_eq!(status, StatusHttp::OK);

    wait("os dois arquivos saírem para o cliente", || {
        scenario
            .sends()
            .iter()
            .filter(|send| send.route == "sendMedia")
            .count()
            == 2
    })
    .await;

    let sends = scenario.sends();
    assert!(
        sends.iter().all(|send| send.route == "sendMedia"),
        "nenhum texto deveria sair: {sends:?}"
    );
    let media_items: Vec<&Send> = sends
        .iter()
        .filter(|send| send.route == "sendMedia")
        .collect();
    let (first, second) = (media_items[0], media_items[1]);

    assert_eq!(first.body["mediatype"], json!("image"));
    assert_eq!(first.body["mimetype"], json!("image/png"));
    assert_eq!(first.body["fileName"], json!("foto.png"));
    assert_eq!(first.body["number"], json!(PHONE));
    assert_eq!(first.body["caption"], json!("*[Ana Atendente]*"));
    let sent = decode_base64(first.body["media"].as_str().expect("mídia em base64"));
    assert_eq!(sent, PNG, "os bytes chegam inteiros");

    assert_eq!(second.body["mediatype"], json!("document"));
    assert_eq!(second.body["mimetype"], json!("application/pdf"));
    assert_eq!(second.body["fileName"], json!("laudo.pdf"));
    assert_eq!(second.body["caption"], json!("*[Ana Atendente]*"));
    let sent = decode_base64(second.body["media"].as_str().expect("mídia em base64"));
    assert_eq!(sent, PDF, "os bytes chegam inteiros");

    assert!(
        scenario.notices_in_room().is_empty(),
        "sem aviso quando o arquivo vai: {:?}",
        scenario.notices_in_room()
    );

    let ghost = scenario.agent_ghost();
    assert_eq!(
        scenario.downloads(),
        vec![Some(ghost.clone()), Some(ghost)],
        "um download por arquivo, os dois disfarçados do fantasma do agente"
    );
    assert_eq!(
        scenario.masquerade_refusals(),
        0,
        "nenhum pedido recusado por disfarce"
    );

    let messages = scenario.messages();
    assert_eq!(messages.len(), 2);
    for (index, (name, mime, size)) in [
        ("foto.png", "image/png", PNG.len()),
        ("laudo.pdf", "application/pdf", PDF.len()),
    ]
    .into_iter()
    .enumerate()
    {
        let message = &messages[index];
        assert_eq!(message.origin, Origin::Human);
        assert_eq!(message.author_name.as_deref(), Some(ATTENDANT_NAME));
        assert_eq!(message.texto, "", "o atendente não digitou texto");
        assert_eq!(
            message.external_id.as_deref(),
            Some(if index == 0 {
                "$img-mx44:matrix-dev.local"
            } else {
                "$pdf-mx44:matrix-dev.local"
            })
        );
        let anexos = scenario.anexos(index);
        assert_eq!(anexos.len(), 1, "cada mensagem leva o seu anexo");
        assert_eq!(anexos[0].name, name);
        assert_eq!(anexos[0].mime, mime);
        assert_eq!(anexos[0].size_bytes, size as u64);
        assert_eq!(anexos[0].extraction_status, "skipped");
        let blob = scenario.blob_dir.join(&anexos[0].blob_path);
        assert!(blob.is_file(), "o blob ficou no disco: {}", blob.display());
    }

    let files = scenario.files_in_queue();
    assert_eq!(files.len(), 2);
    for (item, (name, mime)) in files
        .iter()
        .zip([("foto.png", "image/png"), ("laudo.pdf", "application/pdf")])
    {
        assert_eq!(item.tipo, "file");
        let payload: Value = serde_json::from_str(&item.content_json).expect("payload em JSON");
        assert_eq!(payload["nome"], json!(name));
        assert_eq!(payload["mime"], json!(mime));
        assert_eq!(payload["legenda"], json!("**[Ana Atendente]**"));
        assert!(
            payload["blob_ref"]
                .as_str()
                .is_some_and(|reference| reference.starts_with(&format!("{}/", scenario.org))),
            "blob_ref dentro da pasta da org: {}",
            payload["blob_ref"]
        );
        assert_eq!(payload["sha256"].as_str().map(str::len), Some(64));
        assert!(payload["arquivo_id"].is_string());
    }

    assert_eq!(
        scenario
            .store
            .conversations()
            .get(scenario.org, scenario.conversation)
            .expect("lê a conversa")
            .status,
        poph_store::ConversationStatus::Transferred
    );
    assert_eq!(
        scenario
            .store
            .outbox()
            .count_by_state(scenario.org, OutboxState::Sent)
            .expect("conta a fila"),
        2
    );

    let bodies: Vec<&Value> = sends.iter().map(|send| &send.body).collect();
    let for_customer = serde_json::to_string(&bodies).expect("serializa os envios");
    assert!(!for_customer.contains(HS_TOKEN));
    assert!(!for_customer.contains(AS_TOKEN));
}

#[tokio::test]
async fn fake_synapse_refuses_human_masquerade_and_delivers_to_agent_ghost() {
    let scenario = Scenario::new("disfarce").await;
    let url = scenario.publicar("png-disfarce", PNG);
    let ghost = scenario.agent_ghost();
    let client = MatrixClient::new(
        ClientConfig::new(
            scenario.url_synapse.clone(),
            SERVER_NAME.to_owned(),
            Secret::new(AS_TOKEN.to_owned()),
        )
        .expect("config do cliente do Matrix"),
    )
    .expect("cliente do Matrix");

    let negado = client
        .download_asset(ATTENDANT, &url, poph_matrix::asset::ASSET_BYTES_LIMIT)
        .await
        .expect_err("o humano está fora do namespace do Application Service");
    assert!(
        !negado.transient(),
        "recusa por namespace é definitiva: {negado:?}"
    );
    assert_eq!(
        scenario.masquerade_refusals(),
        1,
        "o Synapse falso recusou o disfarce de humano, como o de verdade"
    );

    let bytes = client
        .download_asset(&ghost, &url, poph_matrix::asset::ASSET_BYTES_LIMIT)
        .await
        .unwrap_or_else(|error| panic!("o fantasma do agente baixa o arquivo ({error:?})"));
    assert_eq!(
        bytes, PNG,
        "o conteúdo desce inteiro pelo fantasma do agente"
    );
}

#[tokio::test]
async fn redelivery_of_same_file_does_not_duplicate_send() {
    let scenario = Scenario::new("reentrega").await;
    let servidor = scenario.boot().await;
    let url = scenario.publicar("png-reentrega", PNG);

    let first = servidor
        .attendant_file(
            "txn-1",
            "$img-rep:matrix-dev.local",
            "m.image",
            "foto.png",
            Some(&url),
        )
        .await;
    let second = servidor
        .attendant_file(
            "txn-1",
            "$img-rep:matrix-dev.local",
            "m.image",
            "foto.png",
            Some(&url),
        )
        .await;
    let third = servidor
        .attendant_file(
            "txn-2",
            "$img-rep:matrix-dev.local",
            "m.image",
            "foto.png",
            Some(&url),
        )
        .await;
    assert_eq!(first, StatusHttp::OK);
    assert_eq!(second, StatusHttp::OK);
    assert_eq!(third, StatusHttp::OK);

    wait("o arquivo sair uma vez", || {
        scenario
            .store
            .outbox()
            .count_by_state(scenario.org, OutboxState::Sent)
            .unwrap_or(0)
            == 1
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let media_items = scenario
        .sends()
        .into_iter()
        .filter(|send| send.route == "sendMedia")
        .count();
    assert_eq!(media_items, 1, "reentrega não manda o arquivo de novo");
    assert_eq!(scenario.messages().len(), 1, "uma mensagem no histórico");
    assert_eq!(scenario.files_in_queue().len(), 1, "um item na fila");
    assert_eq!(
        servidor
            .delivery
            .counters()
            .duplicates
            .load(Ordering::SeqCst),
        1,
        "a transação nova com o mesmo evento foi reconhecida como duplicada"
    );
}

#[tokio::test]
async fn file_over_limit_becomes_room_notice_not_sent() {
    let scenario = Scenario::new("grande").await;
    let servidor = scenario.boot().await;
    let large = vec![0u8; (16 * 1024 * 1024) + 1];
    let url = scenario.publicar("grande-mx44", &large);

    let status = servidor
        .attendant_file(
            "txn-grande",
            "$img-grande:matrix-dev.local",
            "m.image",
            "foto.png",
            Some(&url),
        )
        .await;
    assert_eq!(
        status,
        StatusHttp::OK,
        "arquivo grande não é erro de transporte"
    );

    wait("o aviso do limite aparecer na sala", || {
        !scenario.notices_in_room().is_empty()
    })
    .await;
    assert_eq!(
        single_notice(&scenario),
        "Não consegui enviar o arquivo ao cliente: o arquivo passa de 16 MB"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        scenario.sends().is_empty(),
        "arquivo grande não vai ao cliente: {:?}",
        scenario.sends()
    );
    assert!(scenario.messages().is_empty(), "nada no histórico");
    assert!(scenario.files_in_queue().is_empty(), "nada na fila");
}

#[tokio::test]
async fn refused_type_becomes_room_notice_not_sent() {
    let scenario = Scenario::new("tipo").await;
    let servidor = scenario.boot().await;
    let url = scenario.publicar("zip-mx44", ZIP);

    let status = servidor
        .attendant_file(
            "txn-zip",
            "$zip-mx44:matrix-dev.local",
            "m.file",
            "laudo.pdf",
            Some(&url),
        )
        .await;
    assert_eq!(status, StatusHttp::OK);

    wait("o aviso de tipo aparecer na sala", || {
        !scenario.notices_in_room().is_empty()
    })
    .await;
    assert_eq!(
        single_notice(&scenario),
        "Não consegui enviar o arquivo ao cliente: o tipo deste arquivo não é aceito (o cliente recebe imagem, PDF, Office, áudio e vídeo)"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        scenario.sends().is_empty(),
        "tipo recusado não vai ao cliente"
    );
    assert!(scenario.messages().is_empty(), "nada no histórico");
}

#[tokio::test]
async fn file_without_url_becomes_room_notice() {
    let scenario = Scenario::new("sem-url").await;
    let servidor = scenario.boot().await;

    let status = servidor
        .attendant_file(
            "txn-sem-url",
            "$sem-url:matrix-dev.local",
            "m.image",
            "foto.png",
            None,
        )
        .await;
    assert_eq!(status, StatusHttp::OK);

    wait("o aviso aparecer na sala", || {
        !scenario.notices_in_room().is_empty()
    })
    .await;
    assert_eq!(
        single_notice(&scenario),
        "Não consegui enviar o arquivo ao cliente: o evento não trouxe o arquivo"
    );
    assert!(scenario.sends().is_empty());
}

#[tokio::test]
async fn closed_conversation_does_not_send_file_and_notifies_room() {
    let scenario = Scenario::new("encerrada").await;
    let url = scenario.publicar("png-encerrada", PNG);
    let versao = scenario
        .store
        .conversations()
        .get(scenario.org, scenario.conversation)
        .expect("lê a conversa")
        .version;
    scenario
        .store
        .conversations()
        .close_audited(
            scenario.org,
            scenario.conversation,
            versao,
            "teste".to_owned(),
        )
        .expect("encerra a conversa");

    let servidor = scenario.boot().await;
    let status = servidor
        .attendant_file(
            "txn-fechada",
            "$fechada:matrix-dev.local",
            "m.image",
            "foto.png",
            Some(&url),
        )
        .await;
    assert_eq!(
        status,
        StatusHttp::OK,
        "conversa encerrada não é erro de transporte"
    );

    wait("o aviso de conversa encerrada aparecer na sala", || {
        !scenario.notices_in_room().is_empty()
    })
    .await;
    assert_eq!(
        single_notice(&scenario),
        poph_server::matrix_response::CLOSED_CONVERSATION_NOTICE
    );
    assert!(
        scenario.sends().is_empty(),
        "conversa encerrada não manda o arquivo"
    );
    assert!(scenario.messages().is_empty(), "nada no histórico");
    assert_eq!(
        scenario
            .store
            .conversations()
            .get(scenario.org, scenario.conversation)
            .expect("lê a conversa")
            .status,
        poph_store::ConversationStatus::Closed
    );
}

#[tokio::test]
async fn synapse_down_no_notice_returns_on_resend() {
    let scenario = Scenario::new("transitoria").await;
    let servidor = scenario.boot().await;
    scenario.refuse("png-instavel", 503);
    let url = format!("mxc://{SERVER_NAME}/png-instavel");

    let status = servidor
        .attendant_file(
            "txn-instavel",
            "$instavel:matrix-dev.local",
            "m.image",
            "foto.png",
            Some(&url),
        )
        .await;
    assert_eq!(
        status,
        StatusHttp::SERVICE_UNAVAILABLE,
        "falha transitória do download é 5xx (o Synapse reenvia), nunca aviso"
    );
    assert!(
        scenario.notices_in_room().is_empty(),
        "sem aviso quando o arquivo pode voltar: {:?}",
        scenario.notices_in_room()
    );
    assert!(scenario.sends().is_empty());
    assert!(scenario.messages().is_empty());

    scenario.publicar("png-instavel", PNG);
    scenario.synapse.errors.lock().unwrap().clear();
    let status = servidor
        .attendant_file(
            "txn-instavel-2",
            "$instavel:matrix-dev.local",
            "m.image",
            "foto.png",
            Some(&url),
        )
        .await;
    assert_eq!(status, StatusHttp::OK);
    wait("o arquivo sair no reenvio", || {
        scenario
            .sends()
            .iter()
            .any(|send| send.route == "sendMedia")
    })
    .await;
    assert_eq!(scenario.messages().len(), 1, "uma mensagem, no reenvio");
}

#[tokio::test]
async fn attendant_audio_and_video_reach_whatsapp_without_notice() {
    let scenario = Scenario::new("audio-video").await;
    let servidor = scenario.boot().await;
    let url_audio = scenario.publicar("ogg-mx48", AUDIO_OGG);
    let url_video = scenario.publicar("mp4-mx48", VIDEO_MP4);

    let transaction = json!({
        "events": [
            {
                "type": "m.room.message",
                "room_id": ROOM,
                "sender": ATTENDANT,
                "event_id": "$audio-mx48:matrix-dev.local",
                "content": {
                    "msgtype": "m.audio",
                    "body": "voice-message.ogg",
                    "filename": "voice-message.ogg",
                    "url": url_audio,
                    "info": { "mimetype": "audio/ogg", "size": AUDIO_OGG.len() }
                }
            },
            {
                "type": "m.room.message",
                "room_id": ROOM,
                "sender": ATTENDANT,
                "event_id": "$video-mx48:matrix-dev.local",
                "content": {
                    "msgtype": "m.video",
                    "body": "video.mp4",
                    "filename": "video.mp4",
                    "url": url_video,
                    "info": { "mimetype": "video/mp4", "size": VIDEO_MP4.len() }
                }
            }
        ]
    });

    let status = servidor.transaction("txn-mx48", &transaction).await;
    assert_eq!(status, StatusHttp::OK);

    wait("o áudio e o vídeo saírem para o cliente", || {
        scenario
            .sends()
            .iter()
            .filter(|send| send.route == "sendMedia")
            .count()
            == 2
    })
    .await;

    let sends = scenario.sends();
    let media_items: Vec<&Send> = sends
        .iter()
        .filter(|send| send.route == "sendMedia")
        .collect();
    let (audio, video) = (media_items[0], media_items[1]);

    assert_eq!(audio.body["mediatype"], json!("document"));
    assert_eq!(audio.body["mimetype"], json!("audio/ogg"));
    assert_eq!(audio.body["fileName"], json!("voice-message.ogg"));
    assert_eq!(audio.body["number"], json!(PHONE));
    assert_eq!(audio.body["caption"], json!("*[Ana Atendente]*"));
    let sent = decode_base64(audio.body["media"].as_str().expect("mídia em base64"));
    assert_eq!(sent, AUDIO_OGG, "o áudio chega inteiro ao cliente");

    assert_eq!(video.body["mediatype"], json!("document"));
    assert_eq!(video.body["mimetype"], json!("video/mp4"));
    assert_eq!(video.body["fileName"], json!("video.mp4"));
    assert_eq!(video.body["caption"], json!("*[Ana Atendente]*"));
    let sent = decode_base64(video.body["media"].as_str().expect("mídia em base64"));
    assert_eq!(sent, VIDEO_MP4, "o vídeo chega inteiro ao cliente");

    assert!(
        scenario.notices_in_room().is_empty(),
        "áudio e vídeo não podem virar aviso: {:?}",
        scenario.notices_in_room()
    );

    let ghost = scenario.agent_ghost();
    assert_eq!(
        scenario.downloads(),
        vec![Some(ghost.clone()), Some(ghost)],
        "um download por arquivo, os dois disfarçados do fantasma do agente"
    );
    assert_eq!(
        scenario.masquerade_refusals(),
        0,
        "nenhum pedido recusado por disfarce fora do namespace do AS"
    );

    let messages = scenario.messages();
    assert_eq!(messages.len(), 2);
    for (index, (name, mime, size)) in [
        ("voice-message.ogg", "audio/ogg", AUDIO_OGG.len()),
        ("video.mp4", "video/mp4", VIDEO_MP4.len()),
    ]
    .into_iter()
    .enumerate()
    {
        let message = &messages[index];
        assert_eq!(message.origin, Origin::Human);
        assert_eq!(message.author_name.as_deref(), Some(ATTENDANT_NAME));
        let anexos = scenario.anexos(index);
        assert_eq!(anexos.len(), 1, "cada mensagem leva o seu anexo");
        assert_eq!(anexos[0].name, name);
        assert_eq!(anexos[0].mime, mime);
        assert_eq!(anexos[0].size_bytes, size as u64);
        assert_eq!(anexos[0].extraction_status, "skipped");
        let blob = scenario.blob_dir.join(&anexos[0].blob_path);
        assert!(blob.is_file(), "o blob ficou no disco: {}", blob.display());
    }

    let files = scenario.files_in_queue();
    assert_eq!(files.len(), 2);
    for (item, mime) in files.iter().zip(["audio/ogg", "video/mp4"]) {
        assert_eq!(item.tipo, "file");
        let payload: Value = serde_json::from_str(&item.content_json).expect("payload em JSON");
        assert_eq!(payload["mime"], json!(mime));
        assert_eq!(payload["legenda"], json!("**[Ana Atendente]**"));
    }

    let status = servidor.transaction("txn-mx48-2", &transaction).await;
    assert_eq!(status, StatusHttp::OK);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        scenario
            .sends()
            .into_iter()
            .filter(|send| send.route == "sendMedia")
            .count(),
        2,
        "reentrega de áudio e vídeo não manda os arquivos de novo"
    );
    assert_eq!(scenario.messages().len(), 2, "duas mensagens no histórico");
    assert_eq!(scenario.files_in_queue().len(), 2, "dois itens na fila");
    assert_eq!(
        servidor
            .delivery
            .counters()
            .duplicates
            .load(Ordering::SeqCst),
        2,
        "os dois eventos da transação nova foram reconhecidos como duplicados"
    );
}
