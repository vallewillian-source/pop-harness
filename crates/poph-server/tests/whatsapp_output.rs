use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use poph_agent::Backoff;
use poph_evolution::{ATTACHMENT_CAP, ConfigEvolution, EvolutionClient};
use poph_server::evolution::EvolutionChannel;
use poph_server::whatsapp_output::{WhatsAppOutput, WhatsAppOutputOptions};
use poph_store::{
    NewAgent, NewConversation, NewOrg, NewOutboxItem, OutboxItem, OutboxState, Store,
};
use poph_types::{ConversationId, HarnessConfig, IntegrationId, OrgId, Secret, TurnId};
use serde_json::{Value, json};
use tracing_subscriber::fmt::MakeWriter;

const CHAVE: &str = "chave-evo-saida-26c-teste";
const PHONE: &str = "5511999990000";
const CAP: u64 = ATTACHMENT_CAP;

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

fn captured_logs() -> Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
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

#[derive(Clone, Debug)]
struct Registro {
    path: String,
    apikey: Option<String>,
    body: Value,
}

#[derive(Clone)]
struct FakeEvolution {
    records: Arc<Mutex<Vec<Registro>>>,
    script: Arc<Mutex<VecDeque<(u16, Value)>>>,
    calls: Arc<AtomicUsize>,
}

async fn send_fake(
    State(evo): State<FakeEvolution>,
    Path(instancia): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    evo.calls.fetch_add(1, Ordering::SeqCst);
    let apikey = headers
        .get("apikey")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    evo.records.lock().unwrap().push(Registro {
        path: format!("instancia={instancia}"),
        apikey,
        body,
    });
    let (status, response) = evo
        .script
        .lock()
        .unwrap()
        .pop_front()
        .unwrap_or((200, json!({"key": {"id": "MSG-SAIDA-1"}})));
    (
        StatusCode::from_u16(status).expect("status do roteiro"),
        Json(response),
    )
}

async fn start_fake_evolution(
    script: Vec<(u16, Value)>,
) -> (String, FakeEvolution, tokio::task::JoinHandle<()>) {
    let evo = FakeEvolution {
        records: Arc::new(Mutex::new(Vec::new())),
        script: Arc::new(Mutex::new(script.into_iter().collect())),
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let router = Router::new()
        .route("/message/sendText/{instancia}", post(send_fake))
        .route("/message/sendMedia/{instancia}", post(send_fake))
        .with_state(evo.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera da Evolution falsa");
    let endereco = listener.local_addr().expect("endereço da Evolution falsa");
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{endereco}"), evo, task)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Instancia {
    Normal,
    MissingKey,
    ClosedPort,
}

struct Scenario {
    store: Arc<Store>,
    org: OrgId,
    wa_conversation: ConversationId,
    console_conversation: ConversationId,
    unconfigured_conversation: ConversationId,
    blobs: PathBuf,
    evo: FakeEvolution,
    saida: WhatsAppOutput,
    folder: PathBuf,
    evo_task: Option<tokio::task::JoinHandle<()>>,
}

impl Scenario {
    async fn new(label: &str, mode: Instancia, script: Vec<(u16, Value)>) -> Self {
        captured_logs();
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let folder = std::env::temp_dir().join(format!(
            "poph-saida-wa-{label}-{}-{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(folder.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = folder.join("dados");
        config.server.port = 7481;
        config.validar().expect("configuração válida");
        let blobs = config.blobs();

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Loja Saída".to_owned(),
                slug: "loja-saida".to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Atendente Saída".to_owned(),
                    slug: "atendente-saida".to_owned(),
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
        let outside_config: IntegrationId = store
            .create_integration(org, agente, "whatsapp".to_owned(), "active".to_owned())
            .expect("cria a integração fora da configuração")
            .id
            .parse()
            .expect("id da integração");

        let wa_conversation = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente,
                    integration_id: Some(integracao),
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some(PHONE.to_owned()),
                    target_department: None,
                },
            )
            .expect("cria a conversa de WhatsApp")
            .id;
        let console_conversation = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente,
                    integration_id: None,
                    canal: "console".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("cria a conversa do console")
            .id;
        let unconfigured_conversation = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente,
                    integration_id: Some(outside_config),
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some("5511888880000".to_owned()),
                    target_department: None,
                },
            )
            .expect("cria a conversa fora da configuração")
            .id;

        let (url, evo, evo_task) = start_fake_evolution(script).await;
        let endereco = match mode {
            Instancia::ClosedPort => "http://127.0.0.1:1".to_owned(),
            _ => url,
        };
        let chave = if mode == Instancia::MissingKey {
            None
        } else {
            Some(Secret::new(CHAVE.to_owned()))
        };
        let client = chave.map(|chave| {
            EvolutionClient::new(
                ConfigEvolution::new(endereco, "loja", chave).expect("config da Evolution falsa"),
            )
            .expect("cliente da Evolution falsa")
        });
        let channels = vec![EvolutionChannel {
            org,
            instance: "loja".to_owned(),
            integration: integracao,
            secret: None,
            client,
            attachment_limit: ATTACHMENT_CAP,
        }];
        let options = WhatsAppOutputOptions {
            backoff: Backoff {
                base: Duration::from_millis(1),
                factor: 2,
                cap: Duration::from_millis(5),
            },
            ..WhatsAppOutputOptions::default()
        };
        let saida = WhatsAppOutput::new(Arc::clone(&store), &channels, blobs.clone(), options);

        Self {
            store,
            org,
            wa_conversation,
            console_conversation,
            unconfigured_conversation,
            blobs,
            evo,
            saida,
            folder,
            evo_task: Some(evo_task),
        }
    }

    fn enqueue(
        &self,
        conversa: ConversationId,
        seq: u32,
        tipo: &str,
        content: Value,
    ) -> OutboxItem {
        let (_, item) = self
            .store
            .outbox()
            .enqueue(
                self.org,
                NewOutboxItem {
                    conversation_id: conversa,
                    turn_id: TurnId::generate(),
                    seq,
                    tipo: tipo.to_owned(),
                    content_json: content.to_string(),
                },
            )
            .expect("enfileira a saída");
        item
    }

    fn texto(&self, texto: &str) -> OutboxItem {
        self.enqueue(self.wa_conversation, 0, "text", json!({ "texto": texto }))
    }

    fn estado(&self, item: &OutboxItem) -> OutboxItem {
        self.store
            .outbox()
            .get(self.org, item.id)
            .expect("lê o item de saída")
    }

    fn records(&self) -> Vec<Registro> {
        self.evo.records.lock().unwrap().clone()
    }

    fn calls(&self) -> usize {
        self.evo.calls.load(Ordering::SeqCst)
    }

    fn write_blob(&self, sha: &str, bytes: &[u8]) -> String {
        let relative = format!("{}/{sha}", self.org);
        let path = self.blobs.join(&relative);
        fs::create_dir_all(path.parent().unwrap()).expect("pasta do blob");
        fs::write(&path, bytes).expect("grava o blob");
        relative
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        if let Some(task) = self.evo_task.take() {
            task.abort();
        }
        let _ = fs::remove_dir_all(&self.folder);
    }
}

#[tokio::test]
async fn text_goes_out_once_in_channel_format_with_key_only_in_header() {
    let scenario = Scenario::new("feliz", Instancia::Normal, vec![]).await;
    let item = scenario.texto("olá, tudo bem?");

    let report = scenario.saida.cycle().await.expect("ciclo");

    assert_eq!(report.sent, 1, "{report:?}");
    assert_eq!(scenario.estado(&item).state, OutboxState::Sent);
    let records = scenario.records();
    assert_eq!(records.len(), 1, "uma requisição, sem repetição");
    assert_eq!(records[0].path, "instancia=loja");
    assert_eq!(records[0].apikey.as_deref(), Some(CHAVE));
    assert_eq!(records[0].body["number"], PHONE);
    assert_eq!(records[0].body["text"], "olá, tudo bem?");
    assert!(
        !records[0].body.to_string().contains(CHAVE),
        "a chave não vai no corpo"
    );
}

#[tokio::test]
async fn text_over_4000_is_cut_into_one_message() {
    let scenario = Scenario::new("longo", Instancia::Normal, vec![]).await;
    let texto = "a".repeat(5000);
    let item = scenario.texto(&texto);

    let report = scenario.saida.cycle().await.expect("ciclo");

    assert_eq!(report.sent, 1, "{report:?}");
    assert_eq!(scenario.estado(&item).state, OutboxState::Sent);
    let records = scenario.records();
    assert_eq!(records.len(), 1, "uma mensagem só, nunca dividida");
    let sent = records[0].body["text"].as_str().expect("texto");
    assert_eq!(sent.chars().count(), 4000, "corta no limite do canal");
    assert!(sent.ends_with('…'), "termina em reticências");
}

#[tokio::test]
async fn library_file_goes_out_as_media_with_blob() {
    let scenario = Scenario::new("midia", Instancia::Normal, vec![]).await;
    let content = b"%PDF-1.4 conteudo de teste".to_vec();
    let blob = scenario.write_blob(&"b".repeat(64), &content);
    let item = scenario.enqueue(
        scenario.wa_conversation,
        1,
        "file",
        json!({
            "blob_ref": blob,
            "arquivo_id": "01a1f6a5-cd9b-7000-8000-000000000000",
            "nome": "manual.pdf",
            "mime": "application/pdf",
            "tamanho_bytes": content.len(),
            "sha256": "b".repeat(64),
        }),
    );

    let report = scenario.saida.cycle().await.expect("ciclo");

    assert_eq!(report.sent, 1, "{report:?}");
    assert_eq!(scenario.estado(&item).state, OutboxState::Sent);
    let records = scenario.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].body["mediatype"], "document");
    assert_eq!(records[0].body["fileName"], "manual.pdf");
    let media = records[0].body["media"]
        .as_str()
        .expect("conteúdo em base64");
    assert_eq!(
        media.len(),
        content.len().div_ceil(3) * 4,
        "o tamanho em base64 bate com o blob"
    );
}

#[tokio::test]
async fn file_over_16mb_is_not_sent_and_turn_text_continues() {
    let scenario = Scenario::new("teto", Instancia::Normal, vec![]).await;
    let texto = scenario.enqueue(
        scenario.wa_conversation,
        0,
        "text",
        json!({ "texto": "segue o arquivo" }),
    );
    let large = scenario.enqueue(
        scenario.wa_conversation,
        1,
        "file",
        json!({
            "blob_ref": format!("{}/{}", scenario.org, "c".repeat(64)),
            "arquivo_id": "01a1f6a5-cd9b-7000-8000-000000000001",
            "nome": "catalogo.pdf",
            "mime": "application/pdf",
            "tamanho_bytes": CAP + 1,
            "sha256": "c".repeat(64),
        }),
    );

    let report = scenario.saida.cycle().await.expect("ciclo");

    assert_eq!(report.sent, 1, "o texto sai: {report:?}");
    assert_eq!(
        report.permanent_failures, 1,
        "o arquivo não sai: {report:?}"
    );
    assert_eq!(scenario.estado(&texto).state, OutboxState::Sent);
    let file = scenario.estado(&large);
    assert_eq!(file.state, OutboxState::Failed);
    assert!(file.ultimo_erro.is_some(), "o motivo fica registrado");
    assert_eq!(
        scenario.calls(),
        1,
        "só o texto chegou à Evolution; o arquivo foi recusado antes da chamada"
    );
}

#[tokio::test]
async fn error_5xx_returns_to_queue_and_retries_only_on_next_attempt() {
    let scenario = Scenario::new(
        "transitoria",
        Instancia::Normal,
        vec![(500, json!({"message": "erro interno da Evolution"}))],
    )
    .await;
    let item = scenario.texto("tentativa que falha");

    let first = scenario.saida.cycle().await.expect("primeiro ciclo");

    assert_eq!(first.transient_failures, 1, "{first:?}");
    assert_eq!(
        scenario.calls(),
        1,
        "um 5xx não dispara uma segunda requisição dentro da mesma tentativa"
    );
    let after = scenario.estado(&item);
    assert_eq!(after.state, OutboxState::Sending, "volta para a fila");
    assert_eq!(after.attempts, 1);
    assert!(after.ultimo_erro.is_some());

    tokio::time::sleep(Duration::from_millis(30)).await;
    let second = scenario.saida.cycle().await.expect("segundo ciclo");

    assert_eq!(second.sent, 1, "{second:?}");
    assert_eq!(scenario.estado(&item).state, OutboxState::Sent);
    assert_eq!(
        scenario.calls(),
        2,
        "a repetição veio só na tentativa seguinte"
    );
}

#[tokio::test]
async fn transient_failure_that_never_clears_exhausts_attempts_and_fails() {
    let script = (0..8)
        .map(|_| (500, json!({"message": "indisponível"})))
        .collect();
    let scenario = Scenario::new("esgota", Instancia::Normal, script).await;
    let item = scenario.texto("nunca passa");

    for _ in 0..8 {
        scenario.saida.cycle().await.expect("ciclo");
        tokio::time::sleep(Duration::from_millis(8)).await;
    }
    assert_eq!(scenario.calls(), 8, "uma requisição por tentativa");
    assert_eq!(scenario.estado(&item).attempts, 8);

    let report = scenario.saida.cycle().await.expect("ciclo final");

    assert_eq!(report.exhausted, 1, "{report:?}");
    assert_eq!(scenario.calls(), 8, "depois de esgotar, nada mais sai");
    let after = scenario.estado(&item);
    assert_eq!(after.state, OutboxState::Failed);
    assert_eq!(
        after.ultimo_erro.as_deref(),
        Some("tentativas de envio esgotadas")
    );
}

#[tokio::test]
async fn error_4xx_is_final_no_retry_and_no_phone_leak() {
    let scenario = Scenario::new(
        "definitivo",
        Instancia::Normal,
        vec![(
            400,
            json!({"message": format!("o número {PHONE}@s.whatsapp.net não existe no WhatsApp")}),
        )],
    )
    .await;
    let item = scenario.texto("para um número inválido");

    let first = scenario.saida.cycle().await.expect("primeiro ciclo");

    assert_eq!(first.permanent_failures, 1, "{first:?}");
    let after = scenario.estado(&item);
    assert_eq!(after.state, OutboxState::Failed);
    let error = after.ultimo_erro.clone().unwrap_or_default();
    assert!(
        !error.contains(PHONE),
        "o telefone não vai para o banco: {error}"
    );
    assert!(
        !error.contains("s.whatsapp"),
        "o JID não vai para o banco: {error}"
    );

    tokio::time::sleep(Duration::from_millis(30)).await;
    scenario.saida.cycle().await.expect("segundo ciclo");

    assert_eq!(scenario.calls(), 1, "4xx nunca é repetido");
    assert_eq!(scenario.estado(&item).state, OutboxState::Failed);
}

#[tokio::test]
async fn network_down_is_transient() {
    let scenario = Scenario::new("rede", Instancia::ClosedPort, vec![]).await;
    let item = scenario.texto("sem rede");

    let report = scenario.saida.cycle().await.expect("ciclo");

    assert_eq!(report.transient_failures, 1, "{report:?}");
    assert_eq!(scenario.estado(&item).state, OutboxState::Sending);
}

#[tokio::test]
async fn console_and_unconfigured_integration_conversations_are_untouched() {
    let scenario = Scenario::new("alheios", Instancia::Normal, vec![]).await;
    let console = scenario.enqueue(
        scenario.console_conversation,
        0,
        "text",
        json!({"texto": "do console"}),
    );
    let outside = scenario.enqueue(
        scenario.unconfigured_conversation,
        0,
        "text",
        json!({"texto": "de outra integração"}),
    );

    let report = scenario.saida.cycle().await.expect("ciclo");

    assert_eq!(
        report,
        Default::default(),
        "nada foi contado como deste consumidor"
    );
    assert_eq!(scenario.calls(), 0);
    assert_eq!(scenario.estado(&console).state, OutboxState::Pending);
    assert_eq!(scenario.estado(&console).attempts, 0);
    assert_eq!(scenario.estado(&outside).state, OutboxState::Pending);
    assert_eq!(scenario.estado(&outside).attempts, 0);
}

#[tokio::test]
async fn without_evolution_key_message_stays_pending_and_does_not_fail() {
    let scenario = Scenario::new("sem-chave", Instancia::MissingKey, vec![]).await;
    let item = scenario.texto("espera a chave");

    let report = scenario.saida.cycle().await.expect("ciclo");

    assert_eq!(report.deferred, 1, "{report:?}");
    let after = scenario.estado(&item);
    assert_eq!(after.state, OutboxState::Pending, "não vira falha");
    assert_eq!(after.attempts, 0, "nenhuma tentativa gasta");
    assert_eq!(scenario.calls(), 0);
}

#[tokio::test]
async fn key_and_phone_do_not_appear_in_log() {
    let scenario = Scenario::new(
        "logs",
        Instancia::Normal,
        vec![(
            400,
            json!({"message": format!("número {PHONE}@s.whatsapp.net inválido")}),
        )],
    )
    .await;
    scenario.texto("mensagem para o número inválido");
    scenario.saida.cycle().await.expect("ciclo");
    let transient = Scenario::new(
        "logs-5xx",
        Instancia::Normal,
        vec![(503, json!({"message": "indisponível"}))],
    )
    .await;
    transient.texto("mensagem com falha transitória");
    transient.saida.cycle().await.expect("ciclo");
    tracing::warn!(
        mark = "marca-de-teste-saida-26c",
        "linha de controle do teste"
    );

    let logs = String::from_utf8_lossy(&captured_logs().lock().unwrap()).to_string();
    assert!(
        logs.contains("marca-de-teste-saida-26c"),
        "a captura de logs precisa estar ativa para o teste valer"
    );
    assert!(!logs.contains(CHAVE), "a chave não aparece em log");
    assert!(!logs.contains(PHONE), "o telefone não aparece em log");
    assert!(!logs.contains("s.whatsapp"), "o JID não aparece em log");
}

fn line_with(registro: &str, phrase: &str, chunk: &str) -> String {
    registro
        .lines()
        .find(|line| line.contains(phrase) && line.contains(chunk))
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn sent_and_deferred_send_leave_one_line_each_without_sensitive_data() {
    const TEXT_SENT: &str = "mensagem que vai sair";
    const TEXT_DEFERRED: &str = "mensagem que vai ser adiada";

    let sent = Scenario::new("log-enviado", Instancia::Normal, vec![]).await;
    let item_ok = sent.texto(TEXT_SENT);
    sent.saida.cycle().await.expect("ciclo");
    assert_eq!(sent.estado(&item_ok).state, OutboxState::Sent);

    let deferred = Scenario::new(
        "log-adiado",
        Instancia::Normal,
        vec![(503, json!({"message": "indisponível"}))],
    )
    .await;
    let deferred_item = deferred.texto(TEXT_DEFERRED);
    deferred.saida.cycle().await.expect("ciclo");
    assert_eq!(deferred.estado(&deferred_item).state, OutboxState::Sending);

    let registro = String::from_utf8_lossy(&captured_logs().lock().unwrap()).into_owned();
    let ok_line = line_with(
        &registro,
        "item de saída do WhatsApp enviado",
        &sent.wa_conversation.to_string(),
    );
    assert!(
        !ok_line.is_empty(),
        "o envio não deixou linha de log (ou a captura está vazia)"
    );
    assert!(ok_line.contains("texto"), "tipo ausente: {ok_line}");

    let deferred_line = line_with(
        &registro,
        "envio do WhatsApp adiado",
        &deferred.wa_conversation.to_string(),
    );
    assert!(
        !deferred_line.is_empty(),
        "o adiamento não deixou linha de log"
    );
    assert!(
        deferred_line.contains("tentativa=1"),
        "a linha não diz o número da tentativa: {deferred_line}"
    );
    assert!(
        deferred_line.contains("indisponível"),
        "a linha não diz o motivo: {deferred_line}"
    );

    for forbidden in [CHAVE, PHONE, "s.whatsapp", TEXT_SENT, TEXT_DEFERRED] {
        assert!(
            !registro.contains(forbidden),
            "o log de saída vazou `{forbidden}`"
        );
    }
}

const EVO_ID: &str = "MSG-SAIDA-1";

#[tokio::test]
async fn sent_text_stores_id_returned_by_evolution() {
    let scenario = Scenario::new("id-texto", Instancia::Normal, vec![]).await;
    let item = scenario.texto("olá com id");

    let report = scenario.saida.cycle().await.expect("ciclo");

    assert_eq!(report.sent, 1, "{report:?}");
    let stored = scenario
        .store
        .outbox()
        .external_id(scenario.org, item.id)
        .expect("lê o id externo");
    assert_eq!(
        stored.as_deref(),
        Some(EVO_ID),
        "o id da resposta do envio tem de ficar na outbox"
    );
    assert_eq!(scenario.estado(&item).state, OutboxState::Sent);
}

#[tokio::test]
async fn sent_file_stores_id_returned_by_evolution() {
    let scenario = Scenario::new(
        "id-arquivo",
        Instancia::Normal,
        vec![(201, json!({"key": {"id": "MSG-ARQUIVO-7"}}))],
    )
    .await;
    let content = b"%PDF-1.4 com id".to_vec();
    let blob = scenario.write_blob(&"d".repeat(64), &content);
    let item = scenario.enqueue(
        scenario.wa_conversation,
        2,
        "file",
        json!({
            "blob_ref": blob,
            "arquivo_id": "01a1f6a5-cd9b-7000-8000-000000000002",
            "nome": "contrato.pdf",
            "mime": "application/pdf",
            "tamanho_bytes": content.len(),
            "sha256": "d".repeat(64),
        }),
    );

    let report = scenario.saida.cycle().await.expect("ciclo");

    assert_eq!(report.sent, 1, "{report:?}");
    assert_eq!(
        scenario
            .store
            .outbox()
            .external_id(scenario.org, item.id)
            .expect("lê o id externo")
            .as_deref(),
        Some("MSG-ARQUIVO-7"),
        "o envio de arquivo também guarda o id"
    );
}

#[tokio::test]
async fn failed_send_stores_no_id() {
    let refused = Scenario::new(
        "id-recusado",
        Instancia::Normal,
        vec![(400, json!({"message": "número inválido"}))],
    )
    .await;
    let refused_item = refused.texto("não vai sair");
    let report = refused.saida.cycle().await.expect("ciclo");
    assert_eq!(report.permanent_failures, 1, "{report:?}");
    assert_eq!(
        refused
            .store
            .outbox()
            .external_id(refused.org, refused_item.id)
            .expect("lê o id externo"),
        None,
        "envio recusado não pode ficar com id"
    );

    let deferred = Scenario::new(
        "id-adiado",
        Instancia::Normal,
        vec![(503, json!({"message": "indisponível"}))],
    )
    .await;
    let deferred_item = deferred.texto("tenta depois");
    let report = deferred.saida.cycle().await.expect("ciclo");
    assert_eq!(report.transient_failures, 1, "{report:?}");
    assert_eq!(
        deferred
            .store
            .outbox()
            .external_id(deferred.org, deferred_item.id)
            .expect("lê o id externo"),
        None,
        "envio adiado não pode ficar com id"
    );
}

#[tokio::test]
async fn channel_without_id_in_response_sends_and_stays_without_id() {
    let scenario = Scenario::new(
        "id-ausente",
        Instancia::Normal,
        vec![(200, json!({"status": "ok"}))],
    )
    .await;
    let item = scenario.texto("sem id na resposta");

    let report = scenario.saida.cycle().await.expect("ciclo");

    assert_eq!(report.sent, 1, "{report:?}");
    assert_eq!(scenario.estado(&item).state, OutboxState::Sent);
    assert_eq!(
        scenario
            .store
            .outbox()
            .external_id(scenario.org, item.id)
            .expect("lê o id externo"),
        None
    );
}
