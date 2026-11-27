use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::post;
use poph_evolution::{ATTACHMENT_CAP, ConfigEvolution, EvolutionClient};
use poph_server::ServerState;
use poph_server::evolution::{EvolutionChannel, channels_from_config};
use poph_store::{NewAgent, NewOrg, Page, Store};
use poph_types::config::{EvolutionConfig, EvolutionInstanceConfig};
use poph_types::{AgentId, HarnessConfig, IntegrationId, MessageId, OrgId, Secret};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tracing_subscriber::fmt::MakeWriter;

const WEBHOOK_SECRET: &str = "segredo-webhook-teste-hr26b";
const EVOLUTION_KEY: &str = "chave-evo-teste-123";
const PHONE: &str = "5511999990000";
const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

#[derive(Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();

fn captured_logs() -> Arc<Mutex<Vec<u8>>> {
    LOGS.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let _ = tracing_subscriber::fmt()
            .with_writer(Capture(Arc::clone(&buffer)))
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .try_init();
        buffer
    })
    .clone()
}

#[derive(Clone)]
struct FakeEvolution {
    downloads: Arc<AtomicUsize>,
}

async fn download_fake_media(
    State(evo): State<FakeEvolution>,
    Path(instancia): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    evo.downloads.fetch_add(1, Ordering::SeqCst);
    if instancia != "loja" {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"message": "instância desconhecida"})),
        );
    }
    match body["message"]["key"]["id"].as_str().unwrap_or_default() {
        "MSG-IMG-1" => (
            StatusCode::OK,
            Json(json!({"base64": PNG_B64, "mimetype": "image/png"})),
        ),
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({"status": 404, "error": "Not Found", "message": "Message not found"})),
        ),
    }
}

async fn start_fake_evolution() -> (String, Arc<AtomicUsize>) {
    let downloads = Arc::new(AtomicUsize::new(0));
    let router = Router::new()
        .route(
            "/chat/getBase64FromMediaMessage/{instancia}",
            post(download_fake_media),
        )
        .with_state(FakeEvolution {
            downloads: Arc::clone(&downloads),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera da Evolution falsa");
    let endereco = listener.local_addr().expect("endereço da Evolution falsa");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{endereco}"), downloads)
}

struct Scenario {
    base: String,
    client: Client,
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    integracao: IntegrationId,
    downloads: Arc<AtomicUsize>,
    desligar: Option<oneshot::Sender<()>>,
}

struct Response {
    status: StatusCode,
    body: Value,
    raw: String,
}

impl Scenario {
    async fn new(label: &str) -> Self {
        captured_logs();
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let folder = std::env::temp_dir().join(format!(
            "poph-webhook-evo-{label}-{}-{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(folder.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = folder.join("dados");
        config.server.port = 7480;
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Loja Webhook".to_owned(),
                slug: "loja-webhook".to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Atendente Loja".to_owned(),
                    slug: "atendente-loja".to_owned(),
                    ..Default::default()
                },
            )
            .expect("cria o agente")
            .id;
        let integracao: IntegrationId = store
            .create_integration(org, agente, "whatsapp".to_owned(), "active".to_owned())
            .expect("cria a integração")
            .id
            .parse()
            .expect("id da integração");

        let (url_evo, downloads) = start_fake_evolution().await;
        let evolution_client = EvolutionClient::new(
            ConfigEvolution::new(url_evo, "loja", Secret::new(EVOLUTION_KEY.to_owned()))
                .expect("config da Evolution falsa"),
        )
        .expect("cliente da Evolution falsa");

        let channels = vec![
            EvolutionChannel {
                org,
                instance: "loja".to_owned(),
                integration: integracao,
                secret: Some(Secret::new(WEBHOOK_SECRET.to_owned())),
                client: Some(evolution_client),
                attachment_limit: ATTACHMENT_CAP,
            },
            EvolutionChannel {
                org,
                instance: "orfa".to_owned(),
                integration: IntegrationId::generate(),
                secret: Some(Secret::new(WEBHOOK_SECRET.to_owned())),
                client: None,
                attachment_limit: ATTACHMENT_CAP,
            },
            EvolutionChannel {
                org,
                instance: "sem-segredo".to_owned(),
                integration: integracao,
                secret: None,
                client: None,
                attachment_limit: ATTACHMENT_CAP,
            },
        ];
        let estado = ServerState::new(Arc::clone(&store), config)
            .expect("estado do servidor")
            .with_evolution_channels(channels);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, signal) = oneshot::channel();
        tokio::spawn(async move {
            let _ = poph_server::serve_at(listener, estado, async {
                let _ = signal.await;
            })
            .await;
        });

        let scenario = Self {
            base: format!("http://{endereco}"),
            client: Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("cliente HTTP"),
            folder,
            store,
            org,
            agente,
            integracao,
            downloads,
            desligar: Some(desligar),
        };
        scenario.wait_until_up().await;
        scenario
    }

    async fn wait_until_up(&self) {
        for _ in 0..500 {
            if let Ok(r) = self
                .client
                .get(format!("{}/healthz", self.base))
                .send()
                .await
                && r.status().is_success()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("servidor não subiu");
    }

    async fn webhook(
        &self,
        instancia: &str,
        org: &str,
        secret: Option<&str>,
        body: Value,
    ) -> Response {
        let mut http_request = self
            .client
            .post(format!("{}/webhook/evolution/{org}/{instancia}", self.base));
        if let Some(secret) = secret {
            http_request = http_request.header("X-Evolution-Webhook-Secret", secret);
        }
        let response = http_request
            .json(&body)
            .send()
            .await
            .expect("resposta do webhook");
        let status = response.status();
        let raw = response.text().await.expect("corpo da resposta");
        let body = serde_json::from_str(&raw).unwrap_or(Value::Null);
        Response { status, body, raw }
    }

    async fn deliver(&self, body: Value) -> Response {
        self.webhook("loja", &self.org.to_string(), Some(WEBHOOK_SECRET), body)
            .await
    }

    fn pending_count(&self) -> u64 {
        self.store
            .inbox()
            .count_pending(self.org)
            .expect("contagem da inbox")
    }

    fn conversas(&self) -> Vec<poph_store::Conversa> {
        self.store
            .conversations()
            .list(
                self.org,
                None,
                Page {
                    after: None,
                    limit: 50,
                },
            )
            .expect("lista conversas")
    }

    fn enable_matrix(&self) {
        self.store
            .matrix()
            .enable(self.org)
            .expect("liga o módulo Matrix da org");
    }

    fn count(&self, sql: &str) -> i64 {
        rusqlite::Connection::open(self.store.harness_path())
            .expect("abre o banco para conferir")
            .query_row(sql, [], |line| line.get(0))
            .expect("conta")
    }

    fn one_row(&self, sql: &str, parametro: &str) -> Vec<Option<String>> {
        rusqlite::Connection::open(self.store.harness_path())
            .expect("abre o banco para conferir")
            .query_row(sql, [parametro], |line| {
                let mut values = Vec::new();
                for i in 0..line.as_ref().column_count() {
                    values.push(match line.get_ref(i)? {
                        rusqlite::types::ValueRef::Null => None,
                        rusqlite::types::ValueRef::Integer(n) => Some(n.to_string()),
                        rusqlite::types::ValueRef::Real(n) => Some(n.to_string()),
                        rusqlite::types::ValueRef::Text(t) => {
                            Some(String::from_utf8_lossy(t).into_owned())
                        }
                        rusqlite::types::ValueRef::Blob(_) => Some("<blob>".to_owned()),
                    });
                }
                Ok(values)
            })
            .expect("lê a linha")
    }

    fn message_id(response: &Response) -> MessageId {
        response.body["mensagem_id"]
            .as_str()
            .expect("mensagem_id na resposta")
            .parse()
            .expect("id de mensagem")
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        let _ = fs::remove_dir_all(&self.folder);
    }
}

fn text_event(id: &str, texto: &str) -> Value {
    json!({
        "event": "messages.upsert",
        "instance": "loja",
        "apikey": EVOLUTION_KEY,
        "destination": "http://harness.teste/webhook/evolution/x/loja",
        "data": {
            "key": {"id": id, "remoteJid": format!("{PHONE}@s.whatsapp.net"), "fromMe": false},
            "pushName": "Cliente Teste",
            "messageType": "conversation",
            "message": {"conversation": texto},
            "messageTimestamp": 1795177000
        }
    })
}

fn image_event(id: &str, caption: &str) -> Value {
    json!({
        "event": "messages.upsert",
        "instance": "loja",
        "apikey": EVOLUTION_KEY,
        "destination": "http://harness.teste/webhook/evolution/x/loja",
        "data": {
            "key": {"id": id, "remoteJid": format!("{PHONE}@s.whatsapp.net"), "fromMe": false},
            "pushName": "Cliente Teste",
            "messageType": "imageMessage",
            "message": {"imageMessage": {"mimetype": "image/png", "caption": caption}},
            "messageTimestamp": 1795177000
        }
    })
}

fn state_event(estado: &str) -> Value {
    json!({
        "event": "connection.update",
        "instance": "loja",
        "apikey": EVOLUTION_KEY,
        "data": {"state": estado}
    })
}

fn no_client_data(response: &Response) {
    assert!(
        !response.raw.contains(PHONE),
        "telefone na resposta: {}",
        response.raw
    );
    assert!(!response.raw.contains("@s.whatsapp.net"), "JID na resposta");
    assert!(!response.raw.contains(EVOLUTION_KEY), "chave na resposta");
    assert!(
        !response.raw.contains(WEBHOOK_SECRET),
        "segredo na resposta"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn text_records_inbox_conversation_and_message_and_redelivery_does_not_duplicate() {
    let c = Scenario::new("texto").await;
    let first = c
        .deliver(text_event("MSG-TXT-1", "Oi, quero um orçamento"))
        .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.raw);
    assert_eq!(first.body["situacao"], "nova");
    no_client_data(&first);

    assert_eq!(c.pending_count(), 1, "uma entrada na inbox");
    let conversas = c.conversas();
    assert_eq!(conversas.len(), 1, "uma conversa para o contato");
    assert_eq!(conversas[0].canal, "whatsapp");
    assert_eq!(conversas[0].agent_id, c.agente);
    assert_eq!(conversas[0].peer_externo.as_deref(), Some(PHONE));
    assert_eq!(
        c.store
            .messages()
            .count_rows(c.org, conversas[0].id)
            .expect("conta mensagens"),
        1
    );

    let reentrega = c
        .deliver(text_event("MSG-TXT-1", "Oi, quero um orçamento"))
        .await;
    assert_eq!(reentrega.status, StatusCode::OK, "{}", reentrega.raw);
    assert_eq!(reentrega.body["situacao"], "duplicada");
    assert_eq!(reentrega.body["entrada_id"], first.body["entrada_id"]);
    assert_eq!(reentrega.body["mensagem_id"], first.body["mensagem_id"]);
    assert_eq!(c.pending_count(), 1, "reentrega não duplica a entrada");
    assert_eq!(
        c.store
            .messages()
            .count_rows(c.org, conversas[0].id)
            .expect("conta mensagens"),
        1,
        "reentrega não duplica a mensagem"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_or_wrong_secret_gives_401_without_recording() {
    let c = Scenario::new("segredo").await;
    let org = c.org.to_string();

    let missing_secret = c
        .webhook("loja", &org, None, text_event("MSG-SEG-1", "oi"))
        .await;
    assert_eq!(missing_secret.status, StatusCode::UNAUTHORIZED);
    assert_eq!(missing_secret.body["code"], "webhook_nao_autorizado");

    let wrong = c
        .webhook(
            "loja",
            &org,
            Some("segredo-errado"),
            text_event("MSG-SEG-2", "oi"),
        )
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert_eq!(missing_secret.body["message"], wrong.body["message"]);

    let without_env = c
        .webhook(
            "sem-segredo",
            &org,
            Some(WEBHOOK_SECRET),
            text_event("MSG-SEG-3", "oi"),
        )
        .await;
    assert_eq!(without_env.status, StatusCode::UNAUTHORIZED);

    assert_eq!(c.pending_count(), 0, "nada entra sem segredo certo");
    assert!(c.conversas().is_empty(), "nenhuma conversa aberta");
    no_client_data(&missing_secret);
    no_client_data(&wrong);
}

#[tokio::test(flavor = "multi_thread")]
async fn unconfigured_instance_or_org_gives_404() {
    let c = Scenario::new("instancia").await;
    let org = c.org.to_string();

    let other = c
        .webhook(
            "outra-instancia",
            &org,
            Some(WEBHOOK_SECRET),
            text_event("MSG-404-1", "oi"),
        )
        .await;
    assert_eq!(other.status, StatusCode::NOT_FOUND);
    assert_eq!(other.body["code"], "not_found");

    let unknown_org = c
        .webhook(
            "loja",
            "nao-e-um-id",
            Some(WEBHOOK_SECRET),
            text_event("MSG-404-2", "oi"),
        )
        .await;
    assert_eq!(unknown_org.status, StatusCode::NOT_FOUND);
    assert_eq!(c.pending_count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_integration_gives_404_for_message_and_200_without_creating_for_state() {
    let c = Scenario::new("integracao").await;
    let org = c.org.to_string();

    let mensagem = c
        .webhook(
            "orfa",
            &org,
            Some(WEBHOOK_SECRET),
            text_event("MSG-ORF-1", "oi"),
        )
        .await;
    assert_eq!(mensagem.status, StatusCode::NOT_FOUND, "{}", mensagem.raw);
    assert_eq!(c.pending_count(), 0, "mensagem sem agente dono não entra");

    let estado = c
        .webhook("orfa", &org, Some(WEBHOOK_SECRET), state_event("open"))
        .await;
    assert_eq!(estado.status, StatusCode::OK, "{}", estado.raw);
    assert_eq!(estado.body["efeito"], "integracao_inexistente");

    let integrations = c
        .store
        .list_integrations(
            c.org,
            c.agente,
            Page {
                after: None,
                limit: 50,
            },
        )
        .expect("lista integrações");
    assert_eq!(integrations.len(), 1, "o estado não criou integração nova");
}

#[tokio::test(flavor = "multi_thread")]
async fn connection_state_updates_integration_without_bumping_version() {
    let c = Scenario::new("estado").await;

    let open = c.deliver(state_event("open")).await;
    assert_eq!(open.status, StatusCode::OK, "{}", open.raw);
    assert_eq!(open.body["efeito"], "estado");
    let after_open = c
        .store
        .get_integration(c.org, c.agente, c.integracao)
        .expect("lê a integração");
    assert_eq!(after_open.connection_state, "connected");
    assert_eq!(after_open.version, 1, "estado de conexão não sobe a versão");

    let closed = c.deliver(state_event("close")).await;
    assert_eq!(closed.status, StatusCode::OK);
    let after_closed = c
        .store
        .get_integration(c.org, c.agente, c.integracao)
        .expect("lê a integração");
    assert_eq!(after_closed.connection_state, "disconnected");
}

#[tokio::test(flavor = "multi_thread")]
async fn image_becomes_attachment_with_blob_on_disk_and_redelivery_does_not_download_again() {
    let c = Scenario::new("imagem").await;
    let first = c.deliver(image_event("MSG-IMG-1", "foto do produto")).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.raw);
    assert_eq!(
        c.downloads.load(Ordering::SeqCst),
        1,
        "uma baixada da mídia"
    );

    let mensagem = Scenario::message_id(&first);
    let anexos = c
        .store
        .messages()
        .attachments(c.org, mensagem)
        .expect("lista anexos");
    assert_eq!(anexos.len(), 1, "a imagem virou um anexo");
    assert_eq!(anexos[0].mime, "image/png");
    assert_eq!(anexos[0].extraction_status, "skipped");
    assert!(anexos[0].size_bytes > 0);

    let blob = c
        .folder
        .join("dados")
        .join("blobs")
        .join(&anexos[0].blob_path);
    assert!(blob.is_file(), "blob gravado em {}", blob.display());
    assert_eq!(
        fs::metadata(&blob).expect("metadados do blob").len(),
        anexos[0].size_bytes
    );

    let texto = c
        .store
        .messages()
        .get(c.org, c.conversas()[0].id, mensagem)
        .expect("lê a mensagem");
    assert_eq!(
        texto.texto, "foto do produto",
        "a legenda é o texto da mensagem"
    );

    let reentrega = c.deliver(image_event("MSG-IMG-1", "foto do produto")).await;
    assert_eq!(reentrega.status, StatusCode::OK);
    assert_eq!(reentrega.body["situacao"], "duplicada");
    assert_eq!(
        c.downloads.load(Ordering::SeqCst),
        1,
        "reentrega não baixa de novo"
    );
    assert_eq!(
        c.store
            .messages()
            .attachments(c.org, mensagem)
            .expect("lista anexos")
            .len(),
        1
    );
    no_client_data(&first);
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_media_keeps_message_text() {
    let c = Scenario::new("expirada").await;
    let response = c
        .deliver(image_event(
            "MSG-EXP-1",
            "segue a legenda mesmo sem a imagem",
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.raw);
    assert_eq!(response.body["situacao"], "nova");
    assert_eq!(
        c.downloads.load(Ordering::SeqCst),
        1,
        "tentou baixar uma vez"
    );

    let mensagem = Scenario::message_id(&response);
    let stored = c
        .store
        .messages()
        .get(c.org, c.conversas()[0].id, mensagem)
        .expect("lê a mensagem");
    assert_eq!(stored.texto, "segue a legenda mesmo sem a imagem");
    assert!(
        c.store
            .messages()
            .attachments(c.org, mensagem)
            .expect("lista anexos")
            .is_empty(),
        "sem anexo quando a mídia expirou"
    );
    assert_eq!(c.pending_count(), 1, "a mensagem entrou na inbox");
}

#[tokio::test(flavor = "multi_thread")]
async fn recording_failure_gives_503_and_later_redelivery_enters_whole() {
    let c = Scenario::new("falha").await;

    {
        let connection = rusqlite::Connection::open(c.store.harness_path()).expect("abre o banco");
        connection
            .execute_batch(
                "CREATE TRIGGER falha_teste BEFORE INSERT ON messages
                 BEGIN SELECT RAISE(ABORT, 'falha simulada'); END;",
            )
            .expect("cria o gatilho de falha");
    }

    let failure = c.deliver(text_event("MSG-FALHA-1", "oi com falha")).await;
    assert_eq!(
        failure.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        failure.raw
    );
    assert_eq!(failure.body["code"], "indisponivel");
    assert_eq!(
        c.pending_count(),
        0,
        "a inbox também volta: nada fica pela metade"
    );
    no_client_data(&failure);

    {
        let connection = rusqlite::Connection::open(c.store.harness_path()).expect("abre o banco");
        connection
            .execute_batch("DROP TRIGGER falha_teste;")
            .expect("remove o gatilho");
    }

    let reenvio = c.deliver(text_event("MSG-FALHA-1", "oi com falha")).await;
    assert_eq!(reenvio.status, StatusCode::OK, "{}", reenvio.raw);
    assert_eq!(reenvio.body["situacao"], "nova");
    assert_eq!(c.pending_count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn body_that_is_not_json_object_gives_400() {
    let c = Scenario::new("corpo").await;
    let org = c.org.to_string();
    let response = c
        .client
        .post(format!("{}/webhook/evolution/{org}/loja", c.base))
        .header("X-Evolution-Webhook-Secret", WEBHOOK_SECRET)
        .body("isto não é json")
        .send()
        .await
        .expect("resposta");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let texto = response.text().await.expect("corpo");
    assert!(
        texto.contains("invalid_request"),
        "formato de erro da API: {texto}"
    );
    assert_eq!(c.pending_count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn event_without_effect_gives_200_and_records_nothing() {
    let c = Scenario::new("sem-efeito").await;
    let response = c
        .deliver(json!({"event": "send.message", "instance": "loja", "data": {}}))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.raw);
    assert_eq!(response.body["efeito"], "nenhum");
    assert_eq!(c.pending_count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn logs_and_responses_carry_no_phone_or_key() {
    let c = Scenario::new("logs").await;
    let response = c
        .deliver(text_event("MSG-LOG-1", "oi, sou o cliente de teste"))
        .await;
    assert_eq!(response.status, StatusCode::OK);
    let refusal = c
        .webhook(
            "loja",
            &c.org.to_string(),
            Some("nada"),
            text_event("MSG-LOG-2", "oi"),
        )
        .await;
    assert_eq!(refusal.status, StatusCode::UNAUTHORIZED);

    tokio::time::sleep(Duration::from_millis(50)).await;
    let registro = String::from_utf8_lossy(&captured_logs().lock().unwrap()).into_owned();
    assert!(
        registro.contains("mensagem do WhatsApp registrada"),
        "a captura de logs precisa ter visto a entrega"
    );
    for forbidden in [
        PHONE,
        "@s.whatsapp.net",
        EVOLUTION_KEY,
        WEBHOOK_SECRET,
        "oi, sou o cliente de teste",
    ] {
        assert!(!registro.contains(forbidden), "log vazou `{forbidden}`");
    }
    no_client_data(&response);
    no_client_data(&refusal);
}

#[tokio::test(flavor = "multi_thread")]
async fn connection_state_leaves_one_log_line_with_new_state() {
    let c = Scenario::new("log-estado").await;
    let response = c.deliver(state_event("open")).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.raw);
    assert_eq!(response.body["efeito"], "estado");

    tokio::time::sleep(Duration::from_millis(50)).await;
    let registro = String::from_utf8_lossy(&captured_logs().lock().unwrap()).into_owned();
    let line = registro
        .lines()
        .find(|line| {
            line.contains("estado de conexão do WhatsApp atualizado")
                && line.contains(&c.org.to_string())
        })
        .unwrap_or_default()
        .to_owned();
    assert!(
        !line.is_empty(),
        "a atualização de estado não deixou linha de log"
    );
    assert!(
        line.contains("connected"),
        "a linha não diz o estado novo: {line}"
    );
    assert!(line.contains("loja"), "a linha não diz a instância: {line}");
    for forbidden in [PHONE, "@s.whatsapp.net", EVOLUTION_KEY, WEBHOOK_SECRET] {
        assert!(!registro.contains(forbidden), "log vazou `{forbidden}`");
    }
    no_client_data(&response);
}

fn config_with_instances(instances: Vec<EvolutionInstanceConfig>) -> HarnessConfig {
    HarnessConfig {
        evolution: Some(EvolutionConfig {
            url_base: "http://127.0.0.1:18081".to_owned(),
            instances,
        }),
        ..Default::default()
    }
}

fn instancia(secret_env: &str) -> EvolutionInstanceConfig {
    EvolutionInstanceConfig {
        org_id: OrgId::generate().to_string(),
        instance: "loja".to_owned(),
        integration_id: IntegrationId::generate().to_string(),
        secret_env: secret_env.to_owned(),
    }
}

#[test]
fn configuration_reads_secret_from_variable_name_and_rejects_unsafe_name() {
    let config = config_with_instances(vec![instancia("POPH_EVO_SEGREDO_LOJA")]);
    config.validar().expect("configuração válida");

    let with_value = channels_from_config(
        &config,
        Some(Secret::new(EVOLUTION_KEY.to_owned())),
        &|nome| (nome == "POPH_EVO_SEGREDO_LOJA").then(|| "  valor-do-segredo  ".to_owned()),
    )
    .expect("canais");
    assert_eq!(with_value.len(), 1);
    assert!(with_value[0].secret.is_some());
    assert!(
        with_value[0].client.is_some(),
        "com chave, a instância baixa mídia"
    );

    let without_value = channels_from_config(&config, None, &|_| None).expect("canais");
    assert!(
        without_value[0].secret.is_none(),
        "variável ausente vira instância sem segredo"
    );
    assert!(
        without_value[0].client.is_none(),
        "sem chave, não baixa mídia"
    );

    let mut lowercase = config_with_instances(vec![instancia("poph_evo_segredo")]);
    assert!(
        lowercase.validar().is_err(),
        "nome de variável em minúsculas é recusado"
    );
    lowercase.evolution.as_mut().unwrap().instances[0].secret_env = "POPH_EVO_OK".to_owned();
    lowercase.evolution.as_mut().unwrap().instances[0].instance = "loja/../x".to_owned();
    assert!(
        lowercase.validar().is_err(),
        "nome de instância com barra é recusado"
    );
}

#[test]
fn configuration_rejects_repeated_instance_in_same_org() {
    let one = instancia("POPH_EVO_A");
    let mut repeated = one.clone();
    repeated.integration_id = IntegrationId::generate().to_string();
    let config = config_with_instances(vec![one, repeated]);
    assert!(config.validar().is_err());
}

#[test]
fn invalid_id_message_does_not_pass_configuration() {
    let mut config = config_with_instances(vec![instancia("POPH_EVO_A")]);
    config.evolution.as_mut().unwrap().instances[0].org_id = "nao-e-um-id".to_owned();
    assert!(config.validar().is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn real_evolution_223_bodies_change_right_state() {
    let c = Scenario::new("formato-real").await;

    let qr = c
        .deliver(json!({
            "event": "qrcode.updated",
            "instance": "loja",
            "data": {"qrcode": {
                "instance": "loja",
                "pairingCode": null,
                "code": "2@teste-codigo-de-qr",
                "base64": "data:image/png;base64,iVBORw0KGgo="
            }},
            "destination": "http://harness.teste/webhook/evolution/x/loja",
            "date_time": "2026-11-20T08:34:28.673Z",
            "server_url": "http://127.0.0.1:18081",
            "apikey": EVOLUTION_KEY
        }))
        .await;
    assert_eq!(qr.status, StatusCode::OK, "{}", qr.raw);
    assert_eq!(qr.body["efeito"], "estado");
    let waiting = c
        .store
        .get_integration(c.org, c.agente, c.integracao)
        .expect("lê a integração");
    assert_eq!(waiting.connection_state, "awaiting_scan");

    let conectando = c
        .deliver(json!({
            "event": "connection.update",
            "instance": "loja",
            "data": {"instance": "loja", "state": "connecting", "statusReason": 200},
            "destination": "http://harness.teste/webhook/evolution/x/loja",
            "server_url": "http://127.0.0.1:18081",
            "apikey": EVOLUTION_KEY
        }))
        .await;
    assert_eq!(conectando.status, StatusCode::OK, "{}", conectando.raw);
    let after = c
        .store
        .get_integration(c.org, c.agente, c.integracao)
        .expect("lê a integração");
    assert_eq!(after.connection_state, "awaiting_scan");

    let closed = c
        .deliver(json!({
            "event": "connection.update",
            "instance": "loja",
            "data": {"instance": "loja", "state": "close", "statusReason": 401},
            "apikey": EVOLUTION_KEY
        }))
        .await;
    assert_eq!(closed.status, StatusCode::OK, "{}", closed.raw);
    let disconnected = c
        .store
        .get_integration(c.org, c.agente, c.integracao)
        .expect("lê a integração");
    assert_eq!(disconnected.connection_state, "disconnected");
    no_client_data(&qr);
    assert!(
        !qr.raw.contains("2@teste-codigo-de-qr"),
        "código do QR não volta na resposta"
    );
}

fn event_with_name(id: &str, texto: &str, nome: Value) -> Value {
    let mut body = text_event(id, texto);
    body["data"]["pushName"] = nome;
    body
}

#[tokio::test(flavor = "multi_thread")]
async fn push_name_creates_conversation_with_customer_name() {
    let c = Scenario::new("nome-criar").await;
    let response = c
        .deliver(event_with_name("MSG-NOME-1", "oi", json!("Cliente Teste")))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.raw);
    no_client_data(&response);

    let conversas = c.conversas();
    assert_eq!(conversas.len(), 1);
    assert_eq!(conversas[0].customer_name.as_deref(), Some("Cliente Teste"));
    assert_eq!(conversas[0].version, 1, "o nome não sobe a versão");
}

#[tokio::test(flavor = "multi_thread")]
async fn last_push_name_wins_without_erasing_when_message_has_no_name() {
    let c = Scenario::new("nome-vence").await;
    c.deliver(event_with_name("MSG-NV-1", "oi", json!("Cliente Teste")))
        .await;
    c.deliver(event_with_name(
        "MSG-NV-2",
        "de novo",
        json!("Cliente Novo"),
    ))
    .await;
    assert_eq!(
        c.conversas()[0].customer_name.as_deref(),
        Some("Cliente Novo")
    );

    let without_name = text_event("MSG-NV-3", "sem nome");
    let mut without_name = without_name;
    without_name["data"]
        .as_object_mut()
        .expect("dados")
        .remove("pushName");
    c.deliver(without_name).await;
    let conversa = &c.conversas()[0];
    assert_eq!(conversa.customer_name.as_deref(), Some("Cliente Novo"));
    assert_eq!(conversa.version, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn push_name_is_cleaned_before_storing() {
    let c = Scenario::new("nome-limpo").await;
    let raw_name = format!("  Ana\u{200B} Maria \u{202E}\n{}", "x".repeat(150));
    c.deliver(event_with_name("MSG-LIMPO-1", "oi", json!(raw_name)))
        .await;
    let nome = c.conversas()[0]
        .customer_name
        .clone()
        .expect("nome gravado");
    assert!(nome.starts_with("Ana Maria x"), "{nome}");
    assert_eq!(nome.chars().count(), 100, "corta em 100 caracteres");
    assert!(!nome.contains('\u{200B}') && !nome.contains('\u{202E}'));
}

#[tokio::test(flavor = "multi_thread")]
async fn group_and_device_echo_do_not_record_name() {
    let c = Scenario::new("nome-grupo").await;

    let mut group = event_with_name("MSG-GRP-1", "oi do grupo", json!("Membro do Grupo"));
    group["data"]["key"]["remoteJid"] = json!("120363000000000000@g.us");
    let response = c.deliver(group).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.raw);
    let conversas = c.conversas();
    assert_eq!(conversas.len(), 1, "o grupo abre a própria conversa");
    assert_eq!(conversas[0].customer_name, None);

    let mut eco = event_with_name("MSG-ECO-1", "eco", json!("Dono da Loja"));
    eco["data"]["key"]["fromMe"] = json!(true);
    c.deliver(eco).await;
    assert_eq!(c.conversas().len(), 1, "o eco não abre conversa");
}

#[tokio::test(flavor = "multi_thread")]
async fn name_recording_failure_does_not_stop_message_from_entering() {
    let c = Scenario::new("nome-falha-webhook").await;
    {
        let connection = rusqlite::Connection::open(c.store.harness_path()).expect("abre o banco");
        connection
            .execute_batch(
                "CREATE TRIGGER falha_nome BEFORE UPDATE OF client_name ON conversations
                 BEGIN SELECT RAISE(ABORT, 'falha simulada'); END;",
            )
            .expect("cria o gatilho de falha");
    }

    let failure = c
        .deliver(event_with_name(
            "MSG-FN-1",
            "oi com falha no nome",
            json!("Cliente Teste"),
        ))
        .await;
    assert_eq!(failure.status, StatusCode::OK, "{}", failure.raw);
    assert_eq!(failure.body["situacao"], "nova");
    assert_eq!(c.pending_count(), 1, "a mensagem entrou na inbox");
    assert_eq!(
        c.conversas()[0].customer_name,
        None,
        "o nome ficou para depois"
    );

    {
        let connection = rusqlite::Connection::open(c.store.harness_path()).expect("abre o banco");
        connection
            .execute_batch("DROP TRIGGER falha_nome;")
            .expect("remove o gatilho");
    }
    c.deliver(event_with_name(
        "MSG-FN-2",
        "agora vai",
        json!("Cliente Teste"),
    ))
    .await;
    assert_eq!(
        c.conversas()[0].customer_name.as_deref(),
        Some("Cliente Teste")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn customer_name_appears_in_no_log_or_response() {
    const NOME: &str = "NOME-CANARIO-MX32-Zeca";
    let c = Scenario::new("nome-canario").await;
    let response = c
        .deliver(event_with_name("MSG-CAN-1", "oi, canário", json!(NOME)))
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(!response.raw.contains(NOME), "nome na resposta");

    {
        let connection = rusqlite::Connection::open(c.store.harness_path()).expect("abre o banco");
        connection
            .execute_batch(
                "CREATE TRIGGER falha_canario BEFORE UPDATE OF client_name ON conversations
                 BEGIN SELECT RAISE(ABORT, 'falha simulada'); END;",
            )
            .expect("cria o gatilho de falha");
    }
    c.deliver(event_with_name(
        "MSG-CAN-2",
        "outra",
        json!("OUTRO-CANARIO-MX32"),
    ))
    .await;

    tokio::time::sleep(Duration::from_millis(50)).await;
    let registro = String::from_utf8_lossy(&captured_logs().lock().unwrap()).into_owned();
    assert!(
        registro.contains("mensagem do WhatsApp registrada"),
        "a captura de logs precisa ter visto a entrega"
    );
    assert!(
        !registro.contains(NOME),
        "o nome do cliente vazou para o log"
    );
    assert!(
        !registro.contains("OUTRO-CANARIO-MX32"),
        "o nome do cliente vazou para o log na falha"
    );
}

fn device_event(id: &str, texto: &str) -> Value {
    let mut body = text_event(id, texto);
    body["data"]["key"]["fromMe"] = json!(true);
    body
}

#[tokio::test(flavor = "multi_thread")]
async fn device_message_records_once_and_pauses_agent() {
    let c = Scenario::new("aparelho").await;
    c.enable_matrix();

    let response = c
        .deliver(device_event("MSG-AP-1", "já respondi pelo celular"))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.raw);
    assert_eq!(response.body["efeito"], "aparelho");
    assert_eq!(response.body["situacao"], "nova");
    let conversa = response.body["conversa_id"]
        .as_str()
        .expect("conversa na resposta")
        .to_owned();

    let mensagem = c.one_row(
        "SELECT direction, role, origin, turn_id, text FROM messages WHERE external_id = ?1",
        "MSG-AP-1",
    );
    assert_eq!(mensagem[0].as_deref(), Some("out"), "direção");
    assert_eq!(mensagem[1].as_deref(), Some("assistant"), "papel");
    assert_eq!(mensagem[2].as_deref(), Some("device"), "origem");
    assert_eq!(mensagem[3], None, "sem turno");
    assert_eq!(mensagem[4].as_deref(), Some("já respondi pelo celular"));

    let estado = c.one_row(
        "SELECT status, version FROM conversations WHERE id = ?1",
        &conversa,
    );
    assert_eq!(estado[0].as_deref(), Some("transferred"));
    assert_eq!(estado[1].as_deref(), Some("2"));

    assert_eq!(c.pending_count(), 0, "inbox sem a mensagem do aparelho");
    assert_eq!(c.count("SELECT COUNT(*) FROM outbox"), 0, "outbox vazia");

    let audit = c.one_row(
        "SELECT actor_kind, action FROM audit_log WHERE entity_id = ?1",
        &conversa,
    );
    assert_eq!(audit[0].as_deref(), Some("system"));
    assert_eq!(audit[1].as_deref(), Some("device_takeover"));

    let repeated = c
        .deliver(device_event("MSG-AP-1", "já respondi pelo celular"))
        .await;
    assert_eq!(repeated.status, StatusCode::OK, "{}", repeated.raw);
    assert_eq!(repeated.body["situacao"], "duplicada");
    assert_eq!(c.count("SELECT COUNT(*) FROM messages"), 1, "sem duplicar");
    assert_eq!(
        c.count("SELECT COUNT(*) FROM conversations"),
        1,
        "sem abrir outra conversa"
    );
    assert_eq!(
        c.count("SELECT COUNT(*) FROM audit_log WHERE action = 'device_takeover'"),
        1,
        "uma auditoria só"
    );
    assert_eq!(c.pending_count(), 0);
    assert_eq!(c.count("SELECT COUNT(*) FROM outbox"), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn echo_of_own_send_is_discarded() {
    let c = Scenario::new("aparelho-eco").await;
    c.enable_matrix();

    c.deliver(text_event("MSG-C-1", "oi")).await;
    let conversa = c.conversas()[0].id.to_string();

    {
        let connection = rusqlite::Connection::open(c.store.harness_path()).expect("banco");
        connection
            .execute(
                "INSERT INTO outbox
                    (id, org_id, conversation_id, turn_id, seq, kind, payload_json, state,
                     attempts, created_at, sent_at, external_message_id)
                 VALUES ('out-eco-1', ?1, ?2, 'turno-eco-1', 0, 'text', '{\"texto\":\"oi\"}',
                     'sent', 1, 1795177000000, 1795177000000, 'EVO-ECO-1')",
                rusqlite::params![c.org.to_string(), &conversa],
            )
            .expect("grava o envio confirmado");
    }

    let eco = c.deliver(device_event("EVO-ECO-1", "oi")).await;
    assert_eq!(eco.status, StatusCode::OK, "{}", eco.raw);
    assert_eq!(eco.body["efeito"], "eco");

    assert_eq!(
        c.count("SELECT COUNT(*) FROM messages"),
        1,
        "sem mensagem nova"
    );
    assert_eq!(c.count("SELECT COUNT(*) FROM conversations"), 1);
    assert_eq!(c.count("SELECT COUNT(*) FROM outbox"), 1, "sem envio novo");
    assert_eq!(
        c.count("SELECT COUNT(*) FROM audit_log WHERE action = 'device_takeover'"),
        0,
        "eco não pausa o agente"
    );
    let estado = c.one_row("SELECT status FROM conversations WHERE id = ?1", &conversa);
    assert_eq!(
        estado[0].as_deref(),
        Some("open"),
        "a conversa segue aberta"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn org_without_module_keeps_discarding_from_me() {
    let c = Scenario::new("aparelho-sem-modulo").await;

    let response = c
        .deliver(device_event("MSG-AP-2", "digitada no celular"))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.raw);
    assert_eq!(response.body["efeito"], "nenhum");

    assert_eq!(c.count("SELECT COUNT(*) FROM messages"), 0, "nada gravado");
    assert_eq!(c.count("SELECT COUNT(*) FROM conversations"), 0);
    assert_eq!(c.count("SELECT COUNT(*) FROM outbox"), 0);
    assert_eq!(c.pending_count(), 0, "nada na inbox");
    assert_eq!(
        c.count("SELECT COUNT(*) FROM audit_log WHERE action = 'device_takeover'"),
        0
    );
}
