use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poph_evolution::{
    ATTACHMENT_CAP, AttachmentSource, ChannelConfig, ConfigEvolution, DiskBlobs, EvolutionClient,
    EvolutionError, WhatsAppEvolution, check_secret, interpret,
};
use poph_types::{
    EventSink, Integration, IntegrationError, IntegrationEvent, IntegrationId, IntegrationState,
    OutboundCommand, Presence, Secret, SendFile, SendText, SetPresence,
};
use serde_json::{Value, json};

const CHAVE: &str = "chave-da-evolution-que-nao-pode-vazar";

struct Script {
    code: u16,
    body: String,
}

fn ok(body: Value) -> Script {
    Script {
        code: 200,
        body: body.to_string(),
    }
}

fn error(code: u16, mensagem: &str) -> Script {
    Script {
        code,
        body: json!({"message": mensagem}).to_string(),
    }
}

#[derive(Debug, Clone)]
struct ReceivedRequest {
    request_line: String,
    headers: BTreeMap<String, String>,
    body: String,
}

impl ReceivedRequest {
    fn path(&self) -> &str {
        self.request_line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
    }

    fn method(&self) -> &str {
        self.request_line
            .split_whitespace()
            .next()
            .unwrap_or_default()
    }

    fn body_json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

fn server(steps: Vec<Script>) -> (String, Arc<Mutex<Vec<ReceivedRequest>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("porta efêmera");
    let porta = listener.local_addr().expect("endereço local").port();
    let received = Arc::new(Mutex::new(Vec::new()));
    let registro = Arc::clone(&received);
    std::thread::spawn(move || {
        for step in steps {
            let Ok((fluxo, _)) = listener.accept() else {
                return;
            };
            let captured = read_request(&fluxo);
            if let Ok(mut guard) = registro.lock() {
                guard.push(captured);
            }
            let _ = write_response(&fluxo, step.code, &step.body);
        }
    });
    (format!("http://127.0.0.1:{porta}"), received)
}

fn read_request(fluxo: &TcpStream) -> ReceivedRequest {
    let mut reader = BufReader::new(fluxo);
    let mut request_line = String::new();
    let _ = reader.read_line(&mut request_line);
    let mut headers = BTreeMap::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 {
            break;
        }
        if header == "\r\n" || header == "\n" {
            break;
        }
        if let Some((nome, value)) = header.split_once(':') {
            headers.insert(nome.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let length: usize = headers
        .get("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    if length > 0 {
        let _ = reader.read_exact(&mut body);
    }
    ReceivedRequest {
        request_line: request_line.trim().to_owned(),
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    }
}

fn write_response(fluxo: &TcpStream, code: u16, body: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {code} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut fluxo = fluxo;
    fluxo.write_all(response.as_bytes())?;
    fluxo.flush()
}

fn client(url: &str) -> EvolutionClient {
    EvolutionClient::new(
        ConfigEvolution::new(url, "pop-atende", Secret::new(CHAVE.to_owned()))
            .expect("configuração válida")
            .with_timeout(Duration::from_secs(5)),
    )
    .expect("cliente")
}

fn channel(url: &str) -> WhatsAppEvolution {
    WhatsAppEvolution::new(
        client(url),
        ChannelConfig::new(
            IntegrationId::generate(),
            Secret::new("segredo-do-webhook".to_owned()),
        )
        .expect("configuração do canal"),
        Arc::new(DiskBlobs::new("/tmp/poph-evolution-testes-sem-blobs")),
    )
}

#[tokio::test]
async fn text_goes_out_in_the_whatsapp_format_with_the_key_in_the_header() {
    let (url, received) = server(vec![ok(json!({"key": {"id": "ID-1"}}))]);
    let sent_id = client(&url)
        .send_text("5511999999999", "**negrito** e _italico_")
        .await
        .expect("envio");
    assert_eq!(sent_id.as_deref(), Some("ID-1"));

    let requests = received.lock().unwrap();
    let request = &requests[0];
    assert_eq!(request.method(), "POST");
    assert_eq!(request.path(), "/message/sendText/pop-atende");
    assert_eq!(
        request.headers.get("apikey").map(String::as_str),
        Some(CHAVE)
    );
    let body = request.body_json();
    assert_eq!(body["number"], "5511999999999");
    assert_eq!(body["text"], "*negrito* e _italico_");
}

#[tokio::test]
async fn long_text_goes_out_cut_at_the_channel_limit() {
    let (url, received) = server(vec![ok(json!({}))]);
    let long_text = "palavra ".repeat(1000);
    client(&url)
        .send_text("5511999999999", &long_text)
        .await
        .expect("envio");
    let requests = received.lock().unwrap();
    let texto = requests[0].body_json()["text"]
        .as_str()
        .expect("texto")
        .to_owned();
    assert_eq!(texto.chars().count(), poph_evolution::TEXT_LIMIT);
    assert!(texto.ends_with('…'));
}

#[tokio::test]
async fn text_send_is_not_retried_on_500() {
    let (url, received) = server(vec![
        error(500, "instabilidade"),
        ok(json!({"key": {"id": "ID-2"}})),
    ]);
    let error = client(&url)
        .send_text("5511999999999", "oi")
        .await
        .expect_err("500 no envio não pode ser repetido");
    assert!(matches!(
        error,
        EvolutionError::Response { status: 500, .. }
    ));
    assert_eq!(received.lock().unwrap().len(), 1, "o envio foi repetido");
}

#[tokio::test]
async fn file_send_is_not_retried_on_503() {
    let (url, received) = server(vec![
        error(503, "indisponível"),
        ok(json!({"key": {"id": "ID-4"}})),
    ]);
    client(&url)
        .send_attachment("5511999999999", b"pdf", "application/pdf", "a.pdf", None)
        .await
        .expect_err("503 no envio de arquivo não pode ser repetido");
    assert_eq!(
        received.lock().unwrap().len(),
        1,
        "o arquivo foi enviado duas vezes"
    );
}

#[tokio::test]
async fn presence_and_attachment_download_are_not_retried() {
    let (url, received) = server(vec![
        error(500, "instabilidade"),
        ok(json!({"base64": "aGk="})),
    ]);
    client(&url)
        .send_presence("5511999999999", true)
        .await
        .expect_err("presença não é repetida");
    assert_eq!(received.lock().unwrap().len(), 1);

    let (url, received) = server(vec![
        error(500, "instabilidade"),
        ok(json!({"base64": "aGk="})),
    ]);
    let dados = json!({"key": {"id": "MSG-9"}, "message": {}});
    client(&url)
        .download_attachment("MSG-9", &dados, ATTACHMENT_CAP)
        .await
        .expect_err("download não é repetido");
    assert_eq!(received.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn read_is_retried_once_on_500() {
    let (url, received) = server(vec![
        error(500, "instabilidade"),
        ok(json!({"instance": {"state": "open"}})),
    ]);
    let estado = client(&url)
        .connection_state()
        .await
        .expect("leitura repetida com sucesso");
    assert_eq!(estado, "open");
    assert_eq!(received.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn counters_count_calls_and_failures() {
    let registro = poph_types::Registry::new();
    let metrics = poph_types::BaseMetrics::register(&registro).expect("catálogo");
    let (url, _) = server(vec![
        ok(json!({"key": {"id": "ID-5"}})),
        error(400, "número inválido"),
    ]);
    let client = EvolutionClient::new(
        ConfigEvolution::new(&url, "pop-atende", Secret::new(CHAVE.to_owned()))
            .expect("configuração")
            .with_metrics(metrics.clone()),
    )
    .expect("cliente");
    client
        .send_text("5511999999999", "oi")
        .await
        .expect("feliz");
    assert_eq!(metrics.evolution_calls.value(), 1);
    assert_eq!(metrics.evolution_errors.value(), 0);

    client
        .send_text("5511999999999", "oi")
        .await
        .expect_err("400");
    assert_eq!(metrics.evolution_calls.value(), 2);
    assert_eq!(metrics.evolution_errors.value(), 1);
}

#[tokio::test]
async fn provider_error_carries_no_number_jid_or_key() {
    let (url, _) = server(vec![error(
        400,
        &format!("Number 5511999998888@s.whatsapp.net is not registered for {CHAVE}"),
    )]);
    let error = client(&url)
        .send_text("5511999998888", "oi")
        .await
        .expect_err("400");
    let texto = format!("{error} {error:?}");
    assert!(!texto.contains("5511999998888"), "número vazou: {texto}");
    assert!(!texto.contains("whatsapp.net"), "JID vazou: {texto}");
    assert!(!texto.contains(CHAVE), "a chave vazou: {texto}");
    assert!(texto.contains("[identificador ocultado]"));
}

#[tokio::test]
async fn provider_message_is_cut() {
    let long_message = "palavra ".repeat(400);
    let (url, _) = server(vec![error(400, &long_message)]);
    let error = client(&url)
        .send_text("5511999999999", "oi")
        .await
        .expect_err("400");
    let EvolutionError::Response { message, .. } = error else {
        panic!("esperava erro de resposta");
    };
    assert!(
        message.chars().count() <= 120,
        "mensagem longa demais: {}",
        message.len()
    );
}

#[tokio::test]
async fn network_error_does_not_show_the_address() {
    let porta = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("porta livre");
        listener.local_addr().expect("endereço").port()
    };
    let client = EvolutionClient::new(
        ConfigEvolution::new(
            format!("http://127.0.0.1:{porta}"),
            "pop-atende",
            Secret::new(CHAVE.to_owned()),
        )
        .expect("configuração")
        .with_timeout(Duration::from_millis(300)),
    )
    .expect("cliente");
    let error = client.connection_state().await.expect_err("sem servidor");
    let texto = format!("{error} {error:?}");
    assert!(!texto.contains("127.0.0.1"), "o endereço apareceu: {texto}");
    assert!(
        !texto.contains("pop-atende"),
        "a instância apareceu: {texto}"
    );
}

#[tokio::test]
async fn success_response_above_the_cap_stops_reading() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("porta efêmera");
    let porta = listener.local_addr().expect("endereço").port();
    let escritos = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let written = Arc::clone(&escritos);
    std::thread::spawn(move || {
        let Ok((fluxo, _)) = listener.accept() else {
            return;
        };
        let _ = read_request(&fluxo);
        let mut fluxo = &fluxo;
        let _ = fluxo.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        );
        let frame = vec![b'a'; 65_536];
        for _ in 0..512 {
            let chunk_header = format!("{:x}\r\n", frame.len());
            if fluxo.write_all(chunk_header.as_bytes()).is_err()
                || fluxo.write_all(&frame).is_err()
                || fluxo.write_all(b"\r\n").is_err()
            {
                break;
            }
            written.fetch_add(frame.len() as u64, std::sync::atomic::Ordering::SeqCst);
        }
    });
    let client = EvolutionClient::new(
        ConfigEvolution::new(
            format!("http://127.0.0.1:{porta}"),
            "pop-atende",
            Secret::new(CHAVE.to_owned()),
        )
        .expect("configuração")
        .with_timeout(Duration::from_secs(10)),
    )
    .expect("cliente");
    let error = client.connection_state().await.expect_err("passou do teto");
    assert!(
        matches!(error, EvolutionError::ResponseTooLarge { cap } if cap == poph_evolution::DEFAULT_BODY_CAP)
    );
    let sent_bytes = escritos.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        sent_bytes < 32 * 1024 * 1024,
        "o cliente leu a resposta inteira ({sent_bytes} bytes)"
    );
}

#[tokio::test]
async fn connect_carries_the_number_and_returns_qr_and_code() {
    let (url, received) = server(vec![
        ok(json!({
            "pairingCode": "ABCD-1234",
            "code": "2@texto-do-qr",
            "base64": "data:image/png;base64,AAAA",
            "count": 7,
        })),
        ok(json!({"code": "2@outro", "pairingCode": null, "qrcode": {"count": "8"}})),
    ]);
    let com = client(&url)
        .connect(Some("+55 (11) 99999-8888"))
        .await
        .expect("pareamento");
    assert_eq!(com.qr_base64.as_deref(), Some("data:image/png;base64,AAAA"));
    assert_eq!(com.qr_text.as_deref(), Some("2@texto-do-qr"));
    assert_eq!(com.pairing_code.as_deref(), Some("ABCD-1234"));
    assert_eq!(com.counter, Some(7));
    let without_number = client(&url).connect(None).await.expect("sem número");
    assert_eq!(without_number.pairing_code, None);
    assert_eq!(
        without_number.counter,
        Some(8),
        "`count` em texto também é lido"
    );
    let requests = received.lock().unwrap();
    assert_eq!(
        requests[0].path(),
        "/instance/connect/pop-atende?number=5511999998888"
    );
    assert_eq!(requests[1].path(), "/instance/connect/pop-atende");
}

#[tokio::test]
async fn expiration_only_with_the_evolution_text() {
    let dados = json!({"key": {"id": "MSG-7"}, "message": {}});
    let (url, _) = server(vec![error(400, "Invalid media type for this message")]);
    let error = client(&url)
        .download_attachment("MSG-7", &dados, ATTACHMENT_CAP)
        .await
        .expect_err("400 sem sinal de expiração");
    assert!(
        matches!(error, EvolutionError::Response { status: 400, .. }),
        "não é expiração: {error:?}"
    );
}

#[tokio::test]
async fn error_400_is_not_retried_and_does_not_leak_the_key() {
    let (url, received) = server(vec![error(400, "número inválido")]);
    let error = client(&url)
        .send_text("5511999999999", "oi")
        .await
        .expect_err("400 é definitivo");
    assert_eq!(received.lock().unwrap().len(), 1);
    assert!(matches!(
        error,
        EvolutionError::Response { status: 400, .. }
    ));
    let texto = format!("{error} {error:?}");
    assert!(!texto.contains(CHAVE), "a chave apareceu no erro: {texto}");
    assert!(!error.is_transient());
    assert!(matches!(
        error.to_integration("enviar_texto"),
        IntegrationError::Refused {
            transient: false,
            ..
        }
    ));
}

#[tokio::test]
async fn network_drop_does_not_hang_and_is_transient() {
    let porta = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("porta livre");
        listener.local_addr().expect("endereço").port()
    };
    let client = EvolutionClient::new(
        ConfigEvolution::new(
            format!("http://127.0.0.1:{porta}"),
            "pop-atende",
            Secret::new(CHAVE.to_owned()),
        )
        .expect("configuração")
        .with_retries(0)
        .with_timeout(Duration::from_millis(300)),
    )
    .expect("cliente");
    let error = client
        .send_text("5511999999999", "oi")
        .await
        .expect_err("sem servidor");
    assert!(matches!(error, EvolutionError::Network(_)));
    assert!(error.is_transient());
    assert!(matches!(
        error.to_integration("enviar_texto"),
        IntegrationError::Unavailable { .. }
    ));
}

#[tokio::test]
async fn file_goes_out_as_attachment_with_the_right_type() {
    let (url, received) = server(vec![ok(json!({"key": {"id": "ID-3"}}))]);
    client(&url)
        .send_attachment(
            "5511999999999",
            b"%PDF-1.4 conteudo",
            "application/pdf",
            "orcamento.pdf",
            Some("segue o orçamento"),
        )
        .await
        .expect("envio de mídia");
    let requests = received.lock().unwrap();
    assert_eq!(requests[0].path(), "/message/sendMedia/pop-atende");
    let body = requests[0].body_json();
    assert_eq!(body["mediatype"], "document");
    assert_eq!(body["mimetype"], "application/pdf");
    assert_eq!(body["fileName"], "orcamento.pdf");
    assert_eq!(body["caption"], "segue o orçamento");
    assert!(
        body["media"]
            .as_str()
            .expect("base64")
            .starts_with("JVBERi0")
    );
}

#[tokio::test]
async fn file_above_the_cap_is_refused_before_leaving() {
    let (url, received) = server(vec![ok(json!({}))]);
    let oversized = vec![0u8; (ATTACHMENT_CAP + 1) as usize];
    let error = client(&url)
        .send_attachment(
            "5511999999999",
            &oversized,
            "application/pdf",
            "grande.pdf",
            None,
        )
        .await
        .expect_err("passa do teto");
    assert!(matches!(error, EvolutionError::AttachmentTooLarge { .. }));
    assert!(
        received.lock().unwrap().is_empty(),
        "nada deveria ter sido enviado"
    );
}

#[tokio::test]
async fn attachment_is_downloaded_and_the_cap_is_checked_before_decoding() {
    let content = b"%PDF-1.4 nota";
    let encoded = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(content)
    };
    let dados = json!({
        "key": {"id": "MSG-1"},
        "message": {"documentMessage": {"mimetype": "application/pdf"}},
    });

    let (url, received) = server(vec![ok(json!({
        "base64": encoded,
        "mimetype": "application/pdf",
        "fileName": "nota.pdf",
    }))]);
    let downloaded = client(&url)
        .download_attachment("MSG-1", &dados, ATTACHMENT_CAP)
        .await
        .expect("download");
    assert_eq!(downloaded.bytes, content);
    assert_eq!(downloaded.mime, "application/pdf");
    assert_eq!(downloaded.name.as_deref(), Some("nota.pdf"));
    {
        let requests = received.lock().unwrap();
        assert_eq!(
            requests[0].path(),
            "/chat/getBase64FromMediaMessage/pop-atende"
        );
        assert_eq!(requests[0].body_json()["message"]["key"]["id"], "MSG-1");
    }

    let (url, _) = server(vec![ok(json!({"base64": encoded}))]);
    let error = client(&url)
        .download_attachment("MSG-1", &dados, 4)
        .await
        .expect_err("passa do teto");
    assert!(matches!(error, EvolutionError::AttachmentTooLarge { .. }));
}

#[tokio::test]
async fn expired_attachment_in_evolution_becomes_a_final_error() {
    let dados = json!({"key": {"id": "MSG-2"}, "message": {}});
    let (url, _) = server(vec![error(404, "Message not found")]);
    let error = client(&url)
        .download_attachment("MSG-2", &dados, ATTACHMENT_CAP)
        .await
        .expect_err("expirada");
    assert!(matches!(error, EvolutionError::AttachmentExpired(_)));
    assert!(!error.is_transient(), "repetir não traz a mídia de volta");
    assert!(matches!(
        error.to_integration("baixar_midia"),
        IntegrationError::Refused {
            ref code,
            transient: false,
            ..
        } if code == "midia_expirada"
    ));
}

#[tokio::test]
async fn connection_state_is_read_and_health_follows() {
    let (url, _) = server(vec![ok(json!({"instance": {"state": "open"}}))]);
    let opened = client(&url).connection_state().await.expect("estado");
    assert_eq!(opened, "open");

    let (url, _) = server(vec![ok(json!({"instance": {"state": "close"}}))]);
    let closed = client(&url).connection_state().await.expect("estado");
    assert_eq!(closed, "close");
}

#[tokio::test]
async fn ready_channel_delivers_text_to_the_server() {
    let (url, received) = server(vec![ok(json!({"key": {"id": "ID-4"}}))]);
    let channel = channel(&url);
    channel
        .send_text(SendText {
            idempotency: "turno-1:1".to_owned(),
            peer: "5511999999999".to_owned(),
            text: "**olá**".to_owned(),
        })
        .await
        .expect("envio pelo contrato");
    let requests = received.lock().unwrap();
    assert_eq!(requests[0].body_json()["text"], "*olá*");
}

#[tokio::test]
async fn agent_file_goes_out_from_the_library_and_refuses_a_missing_blob() {
    let root = std::env::temp_dir().join(format!("poph-evolution-{}", std::process::id()));
    let org = root.join("org_teste");
    std::fs::create_dir_all(&org).expect("pasta de blobs");
    let sha = "a".repeat(64);
    std::fs::write(org.join(&sha), b"conteudo do arquivo").expect("blob");

    let (url, received) = server(vec![ok(json!({"key": {"id": "ID-5"}}))]);
    let channel = WhatsAppEvolution::new(
        client(&url),
        ChannelConfig::new(
            IntegrationId::generate(),
            Secret::new("segredo-do-webhook".to_owned()),
        )
        .expect("configuração"),
        Arc::new(DiskBlobs::new(&root)),
    );
    channel
        .send_file(SendFile {
            idempotency: "turno-2:1".to_owned(),
            peer: "5511999999999".to_owned(),
            name: "contrato.pdf".to_owned(),
            mime: "application/pdf".to_owned(),
            size_bytes: 18,
            sha256: sha.clone(),
            blob_ref: format!("org_teste/{sha}"),
            caption: None,
        })
        .await
        .expect("envio de arquivo");
    {
        let requests = received.lock().unwrap();
        assert_eq!(requests[0].body_json()["fileName"], "contrato.pdf");
    }

    let error = channel
        .send_file(SendFile {
            idempotency: "turno-2:2".to_owned(),
            peer: "5511999999999".to_owned(),
            name: "sumiu.pdf".to_owned(),
            mime: "application/pdf".to_owned(),
            size_bytes: 1,
            sha256: "b".repeat(64),
            blob_ref: format!("org_teste/{}", "b".repeat(64)),
            caption: None,
        })
        .await
        .expect_err("blob ausente");
    assert!(matches!(error, IntegrationError::Refused { .. }));
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn refused_presence_does_not_bring_down_the_message() {
    let (url, _) = server(vec![error(400, "presence not supported")]);
    let channel = channel(&url);
    channel
        .set_presence(SetPresence {
            peer: "5511999999999".to_owned(),
            state: Presence::Typing,
        })
        .await
        .expect("presença é opcional");
}

#[tokio::test]
async fn receipt_returns_the_id_of_text_and_attachment_and_presence_without_id() {
    let (url, _) = server(vec![
        ok(json!({"key": {"id": "MSG-TEXTO-1"}})),
        ok(json!({"key": {"id": "MSG-MIDIA-2"}})),
        ok(json!({})),
    ]);
    let channel = channel(&url);

    let texto = channel
        .send_with_receipt(OutboundCommand::Text(SendText {
            idempotency: "turno-3:1".to_owned(),
            peer: "5511999999999".to_owned(),
            text: "com id".to_owned(),
        }))
        .await
        .expect("envio de texto");
    assert_eq!(texto.as_deref(), Some("MSG-TEXTO-1"));

    let root = std::env::temp_dir().join(format!("poph-evolution-recibo-{}", std::process::id()));
    let org = root.join("org_recibo");
    std::fs::create_dir_all(&org).expect("pasta de blobs");
    let sha = "e".repeat(64);
    std::fs::write(org.join(&sha), b"conteudo").expect("blob");
    let attachment_channel = WhatsAppEvolution::new(
        client(&url),
        ChannelConfig::new(
            IntegrationId::generate(),
            Secret::new("segredo-do-webhook".to_owned()),
        )
        .expect("configuração"),
        Arc::new(DiskBlobs::new(&root)),
    );
    let attachment = attachment_channel
        .send_with_receipt(OutboundCommand::File(SendFile {
            idempotency: "turno-3:2".to_owned(),
            peer: "5511999999999".to_owned(),
            name: "recibo.pdf".to_owned(),
            mime: "application/pdf".to_owned(),
            size_bytes: 8,
            sha256: sha.clone(),
            blob_ref: format!("org_recibo/{sha}"),
            caption: None,
        }))
        .await
        .expect("envio de arquivo");
    assert_eq!(attachment.as_deref(), Some("MSG-MIDIA-2"));
    let _ = std::fs::remove_dir_all(&root);

    let presence = channel
        .send_with_receipt(OutboundCommand::Presence(SetPresence {
            peer: "5511999999999".to_owned(),
            state: Presence::Paused,
        }))
        .await
        .expect("presença é opcional");
    assert_eq!(presence, None);
}

#[tokio::test]
async fn start_marks_ready_and_points_the_webhook() {
    let (url, received) = server(vec![
        ok(json!({"instance": {"state": "open"}})),
        ok(json!({"webhook": {"enabled": true}})),
    ]);
    let channel = WhatsAppEvolution::new(
        client(&url),
        ChannelConfig::new(
            IntegrationId::generate(),
            Secret::new("segredo-do-webhook".to_owned()),
        )
        .expect("configuração")
        .with_webhook("https://harness.exemplo/v1/webhooks/whatsapp"),
        Arc::new(DiskBlobs::new("/tmp/poph-evolution-sem-blobs")),
    );
    assert_eq!(channel.state(), IntegrationState::Starting);
    let estado = channel.start().await.expect("iniciar");
    assert_eq!(estado, "open");
    assert_eq!(channel.state(), IntegrationState::Ready);

    let requests = received.lock().unwrap();
    assert_eq!(requests[0].path(), "/instance/connectionState/pop-atende");
    assert_eq!(requests[1].path(), "/webhook/set/pop-atende");
    let body = requests[1].body_json();
    assert_eq!(
        body["webhook"]["url"],
        "https://harness.exemplo/v1/webhooks/whatsapp"
    );
    assert_eq!(
        body["webhook"]["headers"]["X-Evolution-Webhook-Secret"],
        "segredo-do-webhook"
    );
}

#[tokio::test]
async fn closed_instance_leaves_the_channel_starting() {
    let (url, _) = server(vec![ok(json!({"instance": {"state": "connecting"}}))]);
    let channel = channel(&url);
    assert_eq!(channel.start().await.expect("iniciar"), "connecting");
    assert_eq!(channel.state(), IntegrationState::Starting);
}

#[tokio::test]
async fn health_follows_the_instance_state() {
    let (url, _) = server(vec![ok(json!({"instance": {"state": "open"}}))]);
    let health = channel(&url).health().await.expect("saúde");
    assert!(health.ok);

    let (url, _) = server(vec![ok(json!({"instance": {"state": "close"}}))]);
    let health = channel(&url).health().await.expect("saúde");
    assert!(!health.ok);

    let (url, _) = server(vec![error(500, "fora do ar"), error(500, "fora do ar")]);
    let error = channel(&url).health().await.expect_err("erro do provedor");
    assert!(matches!(
        error,
        IntegrationError::Refused {
            transient: true,
            ..
        }
    ));
}

#[test]
fn webhook_with_a_wrong_secret_is_refused_before_reading() {
    let channel = channel("http://127.0.0.1:1");
    let body = json!({"event": "messages.upsert", "data": {}});
    assert!(channel.receive_webhook(None, &body).is_err());
    assert!(
        channel
            .receive_webhook(Some("outro-segredo"), &body)
            .is_err()
    );
    assert!(
        check_secret(
            &Secret::new("segredo-do-webhook".to_owned()),
            Some("segredo-do-webhook")
        )
        .is_ok()
    );
}

#[test]
fn valid_webhook_updates_the_channel_state() {
    let channel = channel("http://127.0.0.1:1");
    let connected = json!({"event": "connection.update", "data": {"state": "open"}});
    channel
        .receive_webhook(Some("segredo-do-webhook"), &connected)
        .expect("webhook autenticado");
    assert_eq!(channel.state(), IntegrationState::Ready);

    let disconnected = json!({"event": "connection.update", "data": {"state": "close"}});
    channel
        .receive_webhook(Some("segredo-do-webhook"), &disconnected)
        .expect("webhook autenticado");
    assert_eq!(channel.state(), IntegrationState::Starting);
}

#[derive(Default)]
struct Collector {
    events: Mutex<Vec<IntegrationEvent>>,
}

impl EventSink for Collector {
    fn receive(&self, evento: IntegrationEvent) {
        self.events.lock().unwrap().push(evento);
    }
}

#[tokio::test]
async fn message_with_image_is_delivered_with_the_downloaded_attachment() {
    let content = b"PNGDADOS";
    let encoded = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(content)
    };
    let (url, _) = server(vec![ok(json!({
        "base64": encoded,
        "mimetype": "image/png",
    }))]);
    let channel = channel(&url);
    let body = json!({
        "event": "messages.upsert",
        "instance": "pop-atende",
        "data": {
            "key": {"id": "MSG-9", "remoteJid": "5511999999999@s.whatsapp.net", "fromMe": false},
            "message": {"imageMessage": {"mimetype": "image/png", "caption": "olha o produto"}},
            "messageTimestamp": 1_795_000_000u64,
        }
    });
    let decisao = channel
        .receive_webhook(Some("segredo-do-webhook"), &body)
        .expect("webhook");
    let collector = Collector::default();
    channel.receive_and_deliver(decisao, &collector).await;

    let events = collector.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    let IntegrationEvent::Inbound(evento) = &events[0] else {
        panic!("esperava entrada");
    };
    assert_eq!(evento.text, "olha o produto");
    assert_eq!(evento.attachments.len(), 1);
    assert_eq!(evento.validar(), Ok(()));
}

#[tokio::test]
async fn failing_attachment_does_not_prevent_the_message_from_arriving() {
    let (url, _) = server(vec![error(404, "Message not found")]);
    let channel = channel(&url);
    let body = json!({
        "event": "messages.upsert",
        "data": {
            "key": {"id": "MSG-10", "remoteJid": "5511999999999@s.whatsapp.net"},
            "message": {"documentMessage": {"mimetype": "application/pdf", "fileName": "nota.pdf",
                        "caption": "a nota que eu recebi"}},
            "messageTimestamp": 1_795_000_000u64,
        }
    });
    let decisao = channel
        .receive_webhook(Some("segredo-do-webhook"), &body)
        .expect("webhook");
    let collector = Collector::default();
    channel.receive_and_deliver(decisao, &collector).await;
    let events = collector.events.lock().unwrap();
    let IntegrationEvent::Inbound(evento) = &events[0] else {
        panic!("esperava entrada");
    };
    assert_eq!(evento.text, "a nota que eu recebi");
    assert!(evento.attachments.is_empty(), "sem anexo, mas com o texto");
}

#[tokio::test]
async fn state_event_from_the_webhook_is_forwarded_to_the_sink() {
    let channel = channel("http://127.0.0.1:1");
    let body = json!({"event": "connection.update", "data": {"state": "open"}});
    let decisao = channel
        .receive_webhook(Some("segredo-do-webhook"), &body)
        .expect("webhook");
    let collector = Collector::default();
    channel.receive_and_deliver(decisao, &collector).await;
    let events = collector.events.lock().unwrap();
    assert!(matches!(events[0], IntegrationEvent::Estado(_)));
}

#[test]
fn body_without_a_customer_message_does_not_become_an_event() {
    let body = json!({"event": "messages.upsert", "data": {
        "key": {"id": "MSG-11", "remoteJid": "5511999999999@s.whatsapp.net"},
        "message": {"reactionMessage": {"text": "👍"}},
        "messageTimestamp": 1_795_000_000u64,
    }});
    let policy = poph_evolution::Policy::default();
    assert!(matches!(
        interpret(IntegrationId::generate(), &body, &policy),
        poph_evolution::Decision::Ignored(_)
    ));
}

#[test]
fn attachment_source_accepts_only_a_relative_path() {
    let fonte = DiskBlobs::new("/tmp");
    assert!(matches!(
        AttachmentSource::read(&fonte, "../../etc/passwd"),
        Err(EvolutionError::BlobUnreadable { .. })
    ));
}
