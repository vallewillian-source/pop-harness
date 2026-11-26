//! Cliente da Evolution e canal WhatsApp contra um servidor falso local (HR-26).
//!
//! Nada de rede externa: o servidor é um `TcpListener` em porta efêmera com roteiro combinado, como
//! nos testes do Jev. O que se prova aqui é o que o POP Atende faz na prática — enviar texto no
//! formato do WhatsApp, cortar no limite, mandar arquivo, baixar mídia com teto, recusar mídia
//! expirada, autenticar o webhook — e que a chave nunca aparece em erro.

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

// ───────────────────────────── servidor falso ─────────────────────────────

struct Roteiro {
    codigo: u16,
    corpo: String,
}

fn ok(corpo: Value) -> Roteiro {
    Roteiro {
        codigo: 200,
        corpo: corpo.to_string(),
    }
}

fn erro(codigo: u16, mensagem: &str) -> Roteiro {
    Roteiro {
        codigo,
        corpo: json!({"message": mensagem}).to_string(),
    }
}

/// O que o servidor recebeu de fato.
#[derive(Debug, Clone)]
struct Recebido {
    linha: String,
    cabecalhos: BTreeMap<String, String>,
    corpo: String,
}

impl Recebido {
    fn trajeto(&self) -> &str {
        self.linha.split_whitespace().nth(1).unwrap_or_default()
    }

    fn metodo(&self) -> &str {
        self.linha.split_whitespace().next().unwrap_or_default()
    }

    fn corpo_json(&self) -> Value {
        serde_json::from_str(&self.corpo).unwrap_or(Value::Null)
    }
}

fn servidor(passos: Vec<Roteiro>) -> (String, Arc<Mutex<Vec<Recebido>>>) {
    let lista = TcpListener::bind("127.0.0.1:0").expect("porta efêmera");
    let porta = lista.local_addr().expect("endereço local").port();
    let recebidos = Arc::new(Mutex::new(Vec::new()));
    let registro = Arc::clone(&recebidos);
    std::thread::spawn(move || {
        for passo in passos {
            let Ok((fluxo, _)) = lista.accept() else {
                return;
            };
            let recebido = ler_pedido(&fluxo);
            if let Ok(mut guarda) = registro.lock() {
                guarda.push(recebido);
            }
            let _ = responder(&fluxo, passo.codigo, &passo.corpo);
        }
    });
    (format!("http://127.0.0.1:{porta}"), recebidos)
}

fn ler_pedido(fluxo: &TcpStream) -> Recebido {
    let mut leitor = BufReader::new(fluxo);
    let mut linha = String::new();
    let _ = leitor.read_line(&mut linha);
    let mut cabecalhos = BTreeMap::new();
    loop {
        let mut cabecalho = String::new();
        if leitor.read_line(&mut cabecalho).unwrap_or(0) == 0 {
            break;
        }
        if cabecalho == "\r\n" || cabecalho == "\n" {
            break;
        }
        if let Some((nome, valor)) = cabecalho.split_once(':') {
            cabecalhos.insert(nome.trim().to_ascii_lowercase(), valor.trim().to_owned());
        }
    }
    let tamanho: usize = cabecalhos
        .get("content-length")
        .and_then(|valor| valor.parse().ok())
        .unwrap_or(0);
    let mut corpo = vec![0u8; tamanho];
    if tamanho > 0 {
        let _ = leitor.read_exact(&mut corpo);
    }
    Recebido {
        linha: linha.trim().to_owned(),
        cabecalhos,
        corpo: String::from_utf8_lossy(&corpo).into_owned(),
    }
}

fn responder(fluxo: &TcpStream, codigo: u16, corpo: &str) -> std::io::Result<()> {
    let resposta = format!(
        "HTTP/1.1 {codigo} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{corpo}",
        corpo.len()
    );
    let mut fluxo = fluxo;
    fluxo.write_all(resposta.as_bytes())?;
    fluxo.flush()
}

fn cliente(url: &str) -> EvolutionClient {
    EvolutionClient::new(
        ConfigEvolution::new(url, "pop-atende", Secret::new(CHAVE.to_owned()))
            .expect("configuração válida")
            .with_timeout(Duration::from_secs(5)),
    )
    .expect("cliente")
}

fn canal(url: &str) -> WhatsAppEvolution {
    WhatsAppEvolution::new(
        cliente(url),
        ChannelConfig::new(
            IntegrationId::generate(),
            Secret::new("segredo-do-webhook".to_owned()),
        )
        .expect("configuração do canal"),
        Arc::new(DiskBlobs::new("/tmp/poph-evolution-testes-sem-blobs")),
    )
}

// ───────────────────────────── cliente ─────────────────────────────

#[tokio::test]
async fn texto_sai_no_formato_do_whatsapp_com_a_chave_no_cabecalho() {
    let (url, recebidos) = servidor(vec![ok(json!({"key": {"id": "ID-1"}}))]);
    let enviado = cliente(&url)
        .send_text("5511999999999", "**negrito** e _italico_")
        .await
        .expect("envio");
    assert_eq!(enviado.as_deref(), Some("ID-1"));

    let pedidos = recebidos.lock().unwrap();
    let pedido = &pedidos[0];
    assert_eq!(pedido.metodo(), "POST");
    assert_eq!(pedido.trajeto(), "/message/sendText/pop-atende");
    assert_eq!(
        pedido.cabecalhos.get("apikey").map(String::as_str),
        Some(CHAVE)
    );
    let corpo = pedido.corpo_json();
    assert_eq!(corpo["number"], "5511999999999");
    assert_eq!(corpo["text"], "*negrito* e _italico_");
}

#[tokio::test]
async fn texto_longo_sai_cortado_no_limite_do_canal() {
    let (url, recebidos) = servidor(vec![ok(json!({}))]);
    let longo = "palavra ".repeat(1000);
    cliente(&url)
        .send_text("5511999999999", &longo)
        .await
        .expect("envio");
    let pedidos = recebidos.lock().unwrap();
    let texto = pedidos[0].corpo_json()["text"]
        .as_str()
        .expect("texto")
        .to_owned();
    assert_eq!(texto.chars().count(), poph_evolution::TEXT_LIMIT);
    assert!(texto.ends_with('…'));
}

// A segunda resposta da fila é 200: se o cliente repetisse o envio, ele teria sucesso e o teste
// reprovaria pelo `expect_err`. Por isso a fila tem duas respostas e o envio tem de parar na 1ª.
#[tokio::test]
async fn envio_de_texto_nao_repete_em_500() {
    let (url, recebidos) = servidor(vec![
        erro(500, "instabilidade"),
        ok(json!({"key": {"id": "ID-2"}})),
    ]);
    let erro = cliente(&url)
        .send_text("5511999999999", "oi")
        .await
        .expect_err("500 no envio não pode ser repetido");
    assert!(matches!(erro, EvolutionError::Response { status: 500, .. }));
    assert_eq!(recebidos.lock().unwrap().len(), 1, "o envio foi repetido");
}

#[tokio::test]
async fn envio_de_arquivo_nao_repete_em_503() {
    let (url, recebidos) = servidor(vec![
        erro(503, "indisponível"),
        ok(json!({"key": {"id": "ID-4"}})),
    ]);
    cliente(&url)
        .send_attachment("5511999999999", b"pdf", "application/pdf", "a.pdf", None)
        .await
        .expect_err("503 no envio de arquivo não pode ser repetido");
    assert_eq!(
        recebidos.lock().unwrap().len(),
        1,
        "o arquivo foi enviado duas vezes"
    );
}

#[tokio::test]
async fn presenca_e_download_de_midia_nao_repetem() {
    let (url, recebidos) = servidor(vec![
        erro(500, "instabilidade"),
        ok(json!({"base64": "aGk="})),
    ]);
    cliente(&url)
        .send_presence("5511999999999", true)
        .await
        .expect_err("presença não é repetida");
    assert_eq!(recebidos.lock().unwrap().len(), 1);

    let (url, recebidos) = servidor(vec![
        erro(500, "instabilidade"),
        ok(json!({"base64": "aGk="})),
    ]);
    let dados = json!({"key": {"id": "MSG-9"}, "message": {}});
    cliente(&url)
        .download_attachment("MSG-9", &dados, ATTACHMENT_CAP)
        .await
        .expect_err("download não é repetido");
    assert_eq!(recebidos.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn leitura_repete_uma_vez_em_500() {
    let (url, recebidos) = servidor(vec![
        erro(500, "instabilidade"),
        ok(json!({"instance": {"state": "open"}})),
    ]);
    let estado = cliente(&url)
        .connection_state()
        .await
        .expect("leitura repetida com sucesso");
    assert_eq!(estado, "open");
    assert_eq!(recebidos.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn contadores_contam_chamadas_e_falhas() {
    let registro = poph_types::Registry::new();
    let metricas = poph_types::BaseMetrics::register(&registro).expect("catálogo");
    let (url, _) = servidor(vec![
        ok(json!({"key": {"id": "ID-5"}})),
        erro(400, "número inválido"),
    ]);
    let cliente = EvolutionClient::new(
        ConfigEvolution::new(&url, "pop-atende", Secret::new(CHAVE.to_owned()))
            .expect("configuração")
            .with_metrics(metricas.clone()),
    )
    .expect("cliente");
    cliente
        .send_text("5511999999999", "oi")
        .await
        .expect("feliz");
    assert_eq!(metricas.evolution_calls.value(), 1);
    assert_eq!(metricas.evolution_errors.value(), 0);

    cliente
        .send_text("5511999999999", "oi")
        .await
        .expect_err("400");
    assert_eq!(metricas.evolution_calls.value(), 2);
    assert_eq!(metricas.evolution_errors.value(), 1);
}

#[tokio::test]
async fn erro_do_provedor_nao_leva_numero_jid_nem_chave() {
    let (url, _) = servidor(vec![erro(
        400,
        &format!("Number 5511999998888@s.whatsapp.net is not registered for {CHAVE}"),
    )]);
    let erro = cliente(&url)
        .send_text("5511999998888", "oi")
        .await
        .expect_err("400");
    let texto = format!("{erro} {erro:?}");
    assert!(!texto.contains("5511999998888"), "número vazou: {texto}");
    assert!(!texto.contains("whatsapp.net"), "JID vazou: {texto}");
    assert!(!texto.contains(CHAVE), "a chave vazou: {texto}");
    assert!(texto.contains("[identificador ocultado]"));
}

#[tokio::test]
async fn mensagem_do_provedor_e_cortada() {
    let longa = "palavra ".repeat(400);
    let (url, _) = servidor(vec![erro(400, &longa)]);
    let erro = cliente(&url)
        .send_text("5511999999999", "oi")
        .await
        .expect_err("400");
    let EvolutionError::Response { message, .. } = erro else {
        panic!("esperava erro de resposta");
    };
    assert!(
        message.chars().count() <= 120,
        "mensagem longa demais: {}",
        message.len()
    );
}

#[tokio::test]
async fn erro_de_rede_nao_mostra_o_endereco() {
    let porta = {
        let lista = TcpListener::bind("127.0.0.1:0").expect("porta livre");
        lista.local_addr().expect("endereço").port()
    };
    let cliente = EvolutionClient::new(
        ConfigEvolution::new(
            format!("http://127.0.0.1:{porta}"),
            "pop-atende",
            Secret::new(CHAVE.to_owned()),
        )
        .expect("configuração")
        .with_timeout(Duration::from_millis(300)),
    )
    .expect("cliente");
    let erro = cliente.connection_state().await.expect_err("sem servidor");
    let texto = format!("{erro} {erro:?}");
    assert!(!texto.contains("127.0.0.1"), "o endereço apareceu: {texto}");
    assert!(
        !texto.contains("pop-atende"),
        "a instância apareceu: {texto}"
    );
}

#[tokio::test]
async fn resposta_de_sucesso_acima_do_teto_para_de_ler() {
    // Servidor que manda 32 MB em pedaços e para quando o cliente fecha a conexão.
    let lista = TcpListener::bind("127.0.0.1:0").expect("porta efêmera");
    let porta = lista.local_addr().expect("endereço").port();
    let escritos = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let contagem = Arc::clone(&escritos);
    std::thread::spawn(move || {
        let Ok((fluxo, _)) = lista.accept() else {
            return;
        };
        let _ = ler_pedido(&fluxo);
        let mut fluxo = &fluxo;
        let _ = fluxo.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        );
        let pedaco = vec![b'a'; 65_536];
        for _ in 0..512 {
            let quadro = format!("{:x}\r\n", pedaco.len());
            if fluxo.write_all(quadro.as_bytes()).is_err()
                || fluxo.write_all(&pedaco).is_err()
                || fluxo.write_all(b"\r\n").is_err()
            {
                break;
            }
            contagem.fetch_add(pedaco.len() as u64, std::sync::atomic::Ordering::SeqCst);
        }
    });
    let cliente = EvolutionClient::new(
        ConfigEvolution::new(
            format!("http://127.0.0.1:{porta}"),
            "pop-atende",
            Secret::new(CHAVE.to_owned()),
        )
        .expect("configuração")
        .with_timeout(Duration::from_secs(10)),
    )
    .expect("cliente");
    let erro = cliente
        .connection_state()
        .await
        .expect_err("passou do teto");
    assert!(
        matches!(erro, EvolutionError::ResponseTooLarge { cap } if cap == poph_evolution::DEFAULT_BODY_CAP)
    );
    let enviados = escritos.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        enviados < 32 * 1024 * 1024,
        "o cliente leu a resposta inteira ({enviados} bytes)"
    );
}

#[tokio::test]
async fn conectar_leva_o_numero_e_devolve_qr_e_codigo() {
    let (url, recebidos) = servidor(vec![
        ok(json!({
            "pairingCode": "ABCD-1234",
            "code": "2@texto-do-qr",
            "base64": "data:image/png;base64,AAAA",
            "count": 7,
        })),
        ok(json!({"code": "2@outro", "pairingCode": null, "qrcode": {"count": "8"}})),
    ]);
    let com = cliente(&url)
        .connect(Some("+55 (11) 99999-8888"))
        .await
        .expect("pareamento");
    assert_eq!(com.qr_base64.as_deref(), Some("data:image/png;base64,AAAA"));
    assert_eq!(com.qr_text.as_deref(), Some("2@texto-do-qr"));
    assert_eq!(com.pairing_code.as_deref(), Some("ABCD-1234"));
    assert_eq!(com.counter, Some(7));
    let sem = cliente(&url).connect(None).await.expect("sem número");
    assert_eq!(sem.pairing_code, None);
    assert_eq!(sem.counter, Some(8), "`count` em texto também é lido");
    let pedidos = recebidos.lock().unwrap();
    assert_eq!(
        pedidos[0].trajeto(),
        "/instance/connect/pop-atende?number=5511999998888"
    );
    assert_eq!(pedidos[1].trajeto(), "/instance/connect/pop-atende");
}

#[tokio::test]
async fn expiracao_so_com_o_texto_da_evolution() {
    let dados = json!({"key": {"id": "MSG-7"}, "message": {}});
    let (url, _) = servidor(vec![erro(400, "Invalid media type for this message")]);
    let erro = cliente(&url)
        .download_attachment("MSG-7", &dados, ATTACHMENT_CAP)
        .await
        .expect_err("400 sem sinal de expiração");
    assert!(
        matches!(erro, EvolutionError::Response { status: 400, .. }),
        "não é expiração: {erro:?}"
    );
}

#[tokio::test]
async fn erro_400_nao_e_retentado_e_nao_vaza_a_chave() {
    let (url, recebidos) = servidor(vec![erro(400, "número inválido")]);
    let erro = cliente(&url)
        .send_text("5511999999999", "oi")
        .await
        .expect_err("400 é definitivo");
    assert_eq!(recebidos.lock().unwrap().len(), 1);
    assert!(matches!(erro, EvolutionError::Response { status: 400, .. }));
    let texto = format!("{erro} {erro:?}");
    assert!(!texto.contains(CHAVE), "a chave apareceu no erro: {texto}");
    assert!(!erro.is_transient());
    assert!(matches!(
        erro.to_integration("enviar_texto"),
        IntegrationError::Refused {
            transient: false,
            ..
        }
    ));
}

#[tokio::test]
async fn queda_de_rede_nao_trava_e_e_transitoria() {
    // Porta fechada: ninguém escutando.
    let porta = {
        let lista = TcpListener::bind("127.0.0.1:0").expect("porta livre");
        lista.local_addr().expect("endereço").port()
    };
    let cliente = EvolutionClient::new(
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
    let erro = cliente
        .send_text("5511999999999", "oi")
        .await
        .expect_err("sem servidor");
    assert!(matches!(erro, EvolutionError::Network(_)));
    assert!(erro.is_transient());
    assert!(matches!(
        erro.to_integration("enviar_texto"),
        IntegrationError::Unavailable { .. }
    ));
}

#[tokio::test]
async fn arquivo_sai_como_midia_com_tipo_certo() {
    let (url, recebidos) = servidor(vec![ok(json!({"key": {"id": "ID-3"}}))]);
    cliente(&url)
        .send_attachment(
            "5511999999999",
            b"%PDF-1.4 conteudo",
            "application/pdf",
            "orcamento.pdf",
            Some("segue o orçamento"),
        )
        .await
        .expect("envio de mídia");
    let pedidos = recebidos.lock().unwrap();
    assert_eq!(pedidos[0].trajeto(), "/message/sendMedia/pop-atende");
    let corpo = pedidos[0].corpo_json();
    assert_eq!(corpo["mediatype"], "document");
    assert_eq!(corpo["mimetype"], "application/pdf");
    assert_eq!(corpo["fileName"], "orcamento.pdf");
    assert_eq!(corpo["caption"], "segue o orçamento");
    assert!(
        corpo["media"]
            .as_str()
            .expect("base64")
            .starts_with("JVBERi0")
    );
}

#[tokio::test]
async fn arquivo_acima_do_teto_e_recusado_antes_de_sair() {
    let (url, recebidos) = servidor(vec![ok(json!({}))]);
    let grande = vec![0u8; (ATTACHMENT_CAP + 1) as usize];
    let erro = cliente(&url)
        .send_attachment(
            "5511999999999",
            &grande,
            "application/pdf",
            "grande.pdf",
            None,
        )
        .await
        .expect_err("passa do teto");
    assert!(matches!(erro, EvolutionError::AttachmentTooLarge { .. }));
    assert!(
        recebidos.lock().unwrap().is_empty(),
        "nada deveria ter sido enviado"
    );
}

#[tokio::test]
async fn midia_e_baixada_e_o_teto_e_conferido_antes_de_decodificar() {
    let conteudo = b"%PDF-1.4 nota";
    let codificado = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(conteudo)
    };
    let dados = json!({
        "key": {"id": "MSG-1"},
        "message": {"documentMessage": {"mimetype": "application/pdf"}},
    });

    let (url, recebidos) = servidor(vec![ok(json!({
        "base64": codificado,
        "mimetype": "application/pdf",
        "fileName": "nota.pdf",
    }))]);
    let baixada = cliente(&url)
        .download_attachment("MSG-1", &dados, ATTACHMENT_CAP)
        .await
        .expect("download");
    assert_eq!(baixada.bytes, conteudo);
    assert_eq!(baixada.mime, "application/pdf");
    assert_eq!(baixada.name.as_deref(), Some("nota.pdf"));
    {
        let pedidos = recebidos.lock().unwrap();
        assert_eq!(
            pedidos[0].trajeto(),
            "/chat/getBase64FromMediaMessage/pop-atende"
        );
        assert_eq!(pedidos[0].corpo_json()["message"]["key"]["id"], "MSG-1");
    }

    // Teto minúsculo: recusa sem decodificar.
    let (url, _) = servidor(vec![ok(json!({"base64": codificado}))]);
    let erro = cliente(&url)
        .download_attachment("MSG-1", &dados, 4)
        .await
        .expect_err("passa do teto");
    assert!(matches!(erro, EvolutionError::AttachmentTooLarge { .. }));
}

#[tokio::test]
async fn midia_expirada_na_evolution_vira_erro_definitivo() {
    let dados = json!({"key": {"id": "MSG-2"}, "message": {}});
    let (url, _) = servidor(vec![erro(404, "Message not found")]);
    let erro = cliente(&url)
        .download_attachment("MSG-2", &dados, ATTACHMENT_CAP)
        .await
        .expect_err("expirada");
    assert!(matches!(erro, EvolutionError::AttachmentExpired(_)));
    assert!(!erro.is_transient(), "repetir não traz a mídia de volta");
    assert!(matches!(
        erro.to_integration("baixar_midia"),
        IntegrationError::Refused {
            ref code,
            transient: false,
            ..
        } if code == "midia_expirada"
    ));
}

#[tokio::test]
async fn estado_da_conexao_e_lido_e_a_saude_segue() {
    let (url, _) = servidor(vec![ok(json!({"instance": {"state": "open"}}))]);
    let aberto = cliente(&url).connection_state().await.expect("estado");
    assert_eq!(aberto, "open");

    let (url, _) = servidor(vec![ok(json!({"instance": {"state": "close"}}))]);
    let fechado = cliente(&url).connection_state().await.expect("estado");
    assert_eq!(fechado, "close");
}

// ───────────────────────────── canal (Integration) ─────────────────────────────

#[tokio::test]
async fn canal_pronto_entrega_texto_ao_servidor() {
    let (url, recebidos) = servidor(vec![ok(json!({"key": {"id": "ID-4"}}))]);
    let canal = canal(&url);
    canal
        .send_text(SendText {
            idempotency: "turno-1:1".to_owned(),
            peer: "5511999999999".to_owned(),
            text: "**olá**".to_owned(),
        })
        .await
        .expect("envio pelo contrato");
    let pedidos = recebidos.lock().unwrap();
    assert_eq!(pedidos[0].corpo_json()["text"], "*olá*");
}

#[tokio::test]
async fn arquivo_do_agente_sai_da_biblioteca_e_recusa_blob_ausente() {
    let raiz = std::env::temp_dir().join(format!("poph-evolution-{}", std::process::id()));
    let org = raiz.join("org_teste");
    std::fs::create_dir_all(&org).expect("pasta de blobs");
    let sha = "a".repeat(64);
    std::fs::write(org.join(&sha), b"conteudo do arquivo").expect("blob");

    let (url, recebidos) = servidor(vec![ok(json!({"key": {"id": "ID-5"}}))]);
    let canal = WhatsAppEvolution::new(
        cliente(&url),
        ChannelConfig::new(
            IntegrationId::generate(),
            Secret::new("segredo-do-webhook".to_owned()),
        )
        .expect("configuração"),
        Arc::new(DiskBlobs::new(&raiz)),
    );
    canal
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
        let pedidos = recebidos.lock().unwrap();
        assert_eq!(pedidos[0].corpo_json()["fileName"], "contrato.pdf");
    }

    let erro = canal
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
    assert!(matches!(erro, IntegrationError::Refused { .. }));
    let _ = std::fs::remove_dir_all(&raiz);
}

#[tokio::test]
async fn presenca_recusada_nao_derruba_a_mensagem() {
    let (url, _) = servidor(vec![erro(400, "presence not supported")]);
    let canal = canal(&url);
    canal
        .set_presence(SetPresence {
            peer: "5511999999999".to_owned(),
            state: Presence::Typing,
        })
        .await
        .expect("presença é opcional");
}

// MX-40: o id da mensagem enviada é o mesmo pelos dois caminhos, e o contrato do canal (que não
// carrega id) continua funcionando. Quem precisa do id — a saída do WhatsApp — usa o recibo.
#[tokio::test]
async fn recibo_devolve_o_id_do_texto_e_da_midia_e_presenca_sem_id() {
    let (url, _) = servidor(vec![
        ok(json!({"key": {"id": "MSG-TEXTO-1"}})),
        ok(json!({"key": {"id": "MSG-MIDIA-2"}})),
        ok(json!({})),
    ]);
    let canal = canal(&url);

    let texto = canal
        .send_with_receipt(OutboundCommand::Text(SendText {
            idempotency: "turno-3:1".to_owned(),
            peer: "5511999999999".to_owned(),
            text: "com id".to_owned(),
        }))
        .await
        .expect("envio de texto");
    assert_eq!(texto.as_deref(), Some("MSG-TEXTO-1"));

    // Arquivo: blob em pasta própria, porque o canal do teste nasce com uma raiz sem blobs.
    let raiz = std::env::temp_dir().join(format!("poph-evolution-recibo-{}", std::process::id()));
    let org = raiz.join("org_recibo");
    std::fs::create_dir_all(&org).expect("pasta de blobs");
    let sha = "e".repeat(64);
    std::fs::write(org.join(&sha), b"conteudo").expect("blob");
    let canal_midia = WhatsAppEvolution::new(
        cliente(&url),
        ChannelConfig::new(
            IntegrationId::generate(),
            Secret::new("segredo-do-webhook".to_owned()),
        )
        .expect("configuração"),
        Arc::new(DiskBlobs::new(&raiz)),
    );
    let midia = canal_midia
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
    assert_eq!(midia.as_deref(), Some("MSG-MIDIA-2"));
    let _ = std::fs::remove_dir_all(&raiz);

    // Presença não tem id: devolve `None`, sem erro.
    let presenca = canal
        .send_with_receipt(OutboundCommand::Presence(SetPresence {
            peer: "5511999999999".to_owned(),
            state: Presence::Paused,
        }))
        .await
        .expect("presença é opcional");
    assert_eq!(presenca, None);
}

#[tokio::test]
async fn iniciar_marca_pronto_e_aponta_o_webhook() {
    let (url, recebidos) = servidor(vec![
        ok(json!({"instance": {"state": "open"}})),
        ok(json!({"webhook": {"enabled": true}})),
    ]);
    let canal = WhatsAppEvolution::new(
        cliente(&url),
        ChannelConfig::new(
            IntegrationId::generate(),
            Secret::new("segredo-do-webhook".to_owned()),
        )
        .expect("configuração")
        .with_webhook("https://harness.exemplo/v1/webhooks/whatsapp"),
        Arc::new(DiskBlobs::new("/tmp/poph-evolution-sem-blobs")),
    );
    assert_eq!(canal.state(), IntegrationState::Starting);
    let estado = canal.start().await.expect("iniciar");
    assert_eq!(estado, "open");
    assert_eq!(canal.state(), IntegrationState::Ready);

    let pedidos = recebidos.lock().unwrap();
    assert_eq!(pedidos[0].trajeto(), "/instance/connectionState/pop-atende");
    assert_eq!(pedidos[1].trajeto(), "/webhook/set/pop-atende");
    let corpo = pedidos[1].corpo_json();
    assert_eq!(
        corpo["webhook"]["url"],
        "https://harness.exemplo/v1/webhooks/whatsapp"
    );
    assert_eq!(
        corpo["webhook"]["headers"]["X-Evolution-Webhook-Secret"],
        "segredo-do-webhook"
    );
}

#[tokio::test]
async fn instancia_fechada_deixa_o_canal_em_iniciando() {
    let (url, _) = servidor(vec![ok(json!({"instance": {"state": "connecting"}}))]);
    let canal = canal(&url);
    assert_eq!(canal.start().await.expect("iniciar"), "connecting");
    assert_eq!(canal.state(), IntegrationState::Starting);
}

#[tokio::test]
async fn saude_segue_o_estado_da_instancia() {
    let (url, _) = servidor(vec![ok(json!({"instance": {"state": "open"}}))]);
    let saude = canal(&url).health().await.expect("saúde");
    assert!(saude.ok);

    let (url, _) = servidor(vec![ok(json!({"instance": {"state": "close"}}))]);
    let saude = canal(&url).health().await.expect("saúde");
    assert!(!saude.ok);

    // Duas respostas 500: a retentativa também falha e o erro final é do provedor (transitório).
    let (url, _) = servidor(vec![erro(500, "fora do ar"), erro(500, "fora do ar")]);
    let erro = canal(&url).health().await.expect_err("erro do provedor");
    assert!(matches!(
        erro,
        IntegrationError::Refused {
            transient: true,
            ..
        }
    ));
}

// ───────────────────────────── webhook no canal ─────────────────────────────

#[test]
fn webhook_com_segredo_errado_e_recusado_antes_de_ler() {
    let canal = canal("http://127.0.0.1:1");
    let corpo = json!({"event": "messages.upsert", "data": {}});
    assert!(canal.receive_webhook(None, &corpo).is_err());
    assert!(
        canal
            .receive_webhook(Some("outro-segredo"), &corpo)
            .is_err()
    );
    // A conferência do segredo é a mesma função exportada (comparação em tempo constante).
    assert!(
        check_secret(
            &Secret::new("segredo-do-webhook".to_owned()),
            Some("segredo-do-webhook")
        )
        .is_ok()
    );
}

#[test]
fn webhook_valido_atualiza_o_estado_do_canal() {
    let canal = canal("http://127.0.0.1:1");
    let conectado = json!({"event": "connection.update", "data": {"state": "open"}});
    canal
        .receive_webhook(Some("segredo-do-webhook"), &conectado)
        .expect("webhook autenticado");
    assert_eq!(canal.state(), IntegrationState::Ready);

    let desconectado = json!({"event": "connection.update", "data": {"state": "close"}});
    canal
        .receive_webhook(Some("segredo-do-webhook"), &desconectado)
        .expect("webhook autenticado");
    assert_eq!(canal.state(), IntegrationState::Starting);
}

/// Receptor que guarda o que chegou (é o que a `inbox` faria na HR-12).
#[derive(Default)]
struct Coletor {
    eventos: Mutex<Vec<IntegrationEvent>>,
}

impl EventSink for Coletor {
    fn receive(&self, evento: IntegrationEvent) {
        self.eventos.lock().unwrap().push(evento);
    }
}

#[tokio::test]
async fn mensagem_com_imagem_e_entregue_com_o_anexo_baixado() {
    let conteudo = b"PNGDADOS";
    let codificado = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(conteudo)
    };
    let (url, _) = servidor(vec![ok(json!({
        "base64": codificado,
        "mimetype": "image/png",
    }))]);
    let canal = canal(&url);
    let corpo = json!({
        "event": "messages.upsert",
        "instance": "pop-atende",
        "data": {
            "key": {"id": "MSG-9", "remoteJid": "5511999999999@s.whatsapp.net", "fromMe": false},
            "message": {"imageMessage": {"mimetype": "image/png", "caption": "olha o produto"}},
            "messageTimestamp": 1_795_000_000u64,
        }
    });
    let decisao = canal
        .receive_webhook(Some("segredo-do-webhook"), &corpo)
        .expect("webhook");
    let coletor = Coletor::default();
    canal.receive_and_deliver(decisao, &coletor).await;

    let eventos = coletor.eventos.lock().unwrap();
    assert_eq!(eventos.len(), 1);
    let IntegrationEvent::Inbound(evento) = &eventos[0] else {
        panic!("esperava entrada");
    };
    assert_eq!(evento.text, "olha o produto");
    assert_eq!(evento.attachments.len(), 1);
    assert_eq!(evento.validar(), Ok(()));
}

#[tokio::test]
async fn midia_que_falha_nao_impede_a_mensagem_de_chegar() {
    // A Evolution responde 404: mídia expirada. O texto do cliente não pode se perder por isso.
    let (url, _) = servidor(vec![erro(404, "Message not found")]);
    let canal = canal(&url);
    let corpo = json!({
        "event": "messages.upsert",
        "data": {
            "key": {"id": "MSG-10", "remoteJid": "5511999999999@s.whatsapp.net"},
            "message": {"documentMessage": {"mimetype": "application/pdf", "fileName": "nota.pdf",
                        "caption": "a nota que eu recebi"}},
            "messageTimestamp": 1_795_000_000u64,
        }
    });
    let decisao = canal
        .receive_webhook(Some("segredo-do-webhook"), &corpo)
        .expect("webhook");
    let coletor = Coletor::default();
    canal.receive_and_deliver(decisao, &coletor).await;
    let eventos = coletor.eventos.lock().unwrap();
    let IntegrationEvent::Inbound(evento) = &eventos[0] else {
        panic!("esperava entrada");
    };
    assert_eq!(evento.text, "a nota que eu recebi");
    assert!(evento.attachments.is_empty(), "sem anexo, mas com o texto");
}

#[tokio::test]
async fn evento_de_estado_do_webhook_e_repassado_ao_receptor() {
    let canal = canal("http://127.0.0.1:1");
    let corpo = json!({"event": "connection.update", "data": {"state": "open"}});
    let decisao = canal
        .receive_webhook(Some("segredo-do-webhook"), &corpo)
        .expect("webhook");
    let coletor = Coletor::default();
    canal.receive_and_deliver(decisao, &coletor).await;
    let eventos = coletor.eventos.lock().unwrap();
    assert!(matches!(eventos[0], IntegrationEvent::Estado(_)));
}

#[test]
fn corpo_sem_mensagem_do_cliente_nao_vira_evento() {
    let corpo = json!({"event": "messages.upsert", "data": {
        "key": {"id": "MSG-11", "remoteJid": "5511999999999@s.whatsapp.net"},
        "message": {"reactionMessage": {"text": "👍"}},
        "messageTimestamp": 1_795_000_000u64,
    }});
    let politica = poph_evolution::Policy::default();
    assert!(matches!(
        interpret(IntegrationId::generate(), &corpo, &politica),
        poph_evolution::Decision::Ignored(_)
    ));
}

#[test]
fn fonte_de_midia_aceita_so_caminho_relativo() {
    let fonte = DiskBlobs::new("/tmp");
    assert!(matches!(
        AttachmentSource::read(&fonte, "../../etc/passwd"),
        Err(EvolutionError::BlobUnreadable { .. })
    ));
}
