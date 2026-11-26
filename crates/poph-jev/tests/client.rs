use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poph_jev::{
    JevClient, JevClientConfig, JevError, JevRequest, JevResponse, JevTransport, QuestionType,
};
use poph_types::{BaseMetrics, JevConfig, Registry, Secret};

const CHAVE: &str = "chave-do-jev-nao-pode-vazar";

struct Script {
    code: u16,
    body: String,
    delay: Duration,
}

fn ok(body: &str) -> Script {
    Script {
        code: 200,
        body: body.to_owned(),
        delay: Duration::ZERO,
    }
}

fn status(code: u16) -> Script {
    Script {
        code,
        body: "{\"erro\":\"falhou\"}".to_owned(),
        delay: Duration::ZERO,
    }
}

fn servidor(script: Vec<Script>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("porta efêmera");
    let porta = listener.local_addr().expect("endereço local").port();
    let received = Arc::new(Mutex::new(Vec::new()));
    let registro = Arc::clone(&received);
    std::thread::spawn(move || {
        for step in script {
            let Ok((fluxo, _)) = listener.accept() else {
                return;
            };
            let request = read_request(&fluxo);
            if let Ok(mut guard) = registro.lock() {
                guard.push(request);
            }
            if !step.delay.is_zero() {
                std::thread::sleep(step.delay);
            }
            let _ = respond(&fluxo, step.code, &step.body);
        }
    });
    (format!("http://127.0.0.1:{porta}"), received)
}

fn read_request(fluxo: &TcpStream) -> String {
    let mut reader = BufReader::new(fluxo);
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return headers;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        headers.push_str(&line);
    }
    let size = headers
        .lines()
        .find_map(|line| {
            let lowercase = line.to_ascii_lowercase();
            lowercase
                .strip_prefix("content-length:")
                .map(|value| value.trim().parse::<usize>().unwrap_or(0))
        })
        .unwrap_or(0);
    let mut body = vec![0_u8; size];
    let _ = reader.read_exact(&mut body);
    format!("{headers}\r\n{}", String::from_utf8_lossy(&body))
}

fn respond(mut fluxo: &TcpStream, code: u16, body: &str) -> std::io::Result<()> {
    let answer = format!(
        "HTTP/1.1 {code} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    fluxo.write_all(answer.as_bytes())?;
    fluxo.flush()
}

fn config(url_base: &str, timeout_ms: u64, retentativas: u32) -> JevClientConfig {
    let jev = JevConfig {
        enabled: true,
        url_base: url_base.to_owned(),
        model: "jev-latest".to_owned(),
        timeout_seconds: 8,
        retries: retentativas,
        ..JevConfig::default()
    };
    JevClientConfig::from_config(&jev, Secret::new(CHAVE.to_owned()))
        .expect("configuração do cliente")
        .with_timeout(Duration::from_millis(timeout_ms))
}

fn metrics() -> BaseMetrics {
    BaseMetrics::register(&Registry::new()).expect("métricas base")
}

fn sample_request() -> JevRequest {
    let mut rubric = std::collections::BTreeMap::new();
    rubric.insert("continuar_agente".to_owned(), "segue".to_owned());
    let mut question_map = std::collections::BTreeMap::new();
    question_map.insert(
        "g1_transferir".to_owned(),
        poph_jev::Question::choice("julgue", rubric),
    );
    JevRequest {
        estado: serde_json::json!({"canal": "cli"}),
        modelo: "jev-latest".to_owned(),
        questions: question_map,
    }
}

#[tokio::test]
async fn sends_the_contract_and_reads_the_answer() {
    let (url, received) = servidor(vec![ok(
        r#"{"answers":{"g1_transferir":{"type":"choice","choice":"transferir_agora","probabilities":{"transferir_agora":0.9,"continuar_agente":0.1},"confidence":0.9}},"usage":{"input_tokens":21}}"#,
    )]);
    let client = JevClient::new(config(&url, 2_000, 1), metrics()).expect("cliente");

    let batch = client.judge(sample_request()).await.expect("resposta");

    let answer = batch.answers.get("g1_transferir").expect("resposta do G1");
    assert_eq!(answer.choice.as_deref(), Some("transferir_agora"));
    assert_eq!(answer.confidence, 0.9);
    assert!((answer.margin() - 0.8).abs() < 1e-9);
    assert_eq!(batch.tokens, 21);

    let requests = received.lock().expect("registro").clone();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("POST /v1/systemone "),
        "início: {}",
        &requests[0][..40.min(requests[0].len())]
    );
    let lowercase = requests[0].to_ascii_lowercase();
    assert!(lowercase.contains(&format!("authorization: bearer {}", CHAVE.to_lowercase())));
    assert!(lowercase.contains("content-type: application/json"));
    assert!(requests[0].contains("\"questions\""));
    assert!(requests[0].contains("\"g1_transferir\""));
}

#[tokio::test]
async fn retries_once_on_transient_error() {
    let (url, received) = servidor(vec![
        status(429),
        ok(r#"{"answers":{"g1_transferir":{"choice":"continuar_agente","confidence":1.0}}}"#),
    ]);
    let client = JevClient::new(config(&url, 2_000, 1), metrics()).expect("cliente");

    let batch = client
        .judge(sample_request())
        .await
        .expect("a segunda tentativa passa");

    assert_eq!(
        batch
            .answers
            .get("g1_transferir")
            .and_then(|r| r.choice.clone()),
        Some("continuar_agente".to_owned())
    );
    assert_eq!(received.lock().expect("registro").len(), 2);
}

#[tokio::test]
async fn does_not_retry_definitive_error_and_does_not_leak_key() {
    let (url, received) = servidor(vec![status(400)]);
    let client = JevClient::new(config(&url, 2_000, 1), metrics()).expect("cliente");

    let error = client
        .judge(sample_request())
        .await
        .expect_err("400 não retenta");

    assert!(matches!(error, JevError::Status { code: 400, .. }));
    assert!(!error.to_string().contains(CHAVE));
    assert!(!format!("{error:?}").contains(CHAVE));
    assert_eq!(received.lock().expect("registro").len(), 1);
}

#[tokio::test]
async fn per_attempt_deadline_returns_timeout_error() {
    let (url, _received) = servidor(vec![Script {
        code: 200,
        body: "{}".to_owned(),
        delay: Duration::from_secs(3),
    }]);
    let client = JevClient::new(config(&url, 200, 0), metrics()).expect("cliente");

    let error = client
        .judge(sample_request())
        .await
        .expect_err("prazo estourado");

    assert!(
        matches!(error, JevError::TimedOut { .. }),
        "erro: {error:?}"
    );
    assert!(error.is_transient());
}

#[test]
fn config_accepts_https_rejects_invalid_address_and_empty_key() {
    let https = JevConfig {
        enabled: true,
        url_base: "https://api.typesafe.ai/".to_owned(),
        ..JevConfig::default()
    };
    let config =
        JevClientConfig::from_config(&https, Secret::new("x".to_owned())).expect("https aceito");
    assert_eq!(config.url_base, "https://api.typesafe.ai");
    assert_eq!(config.endpoint(), "https://api.typesafe.ai/v1/systemone");

    let without_scheme = JevConfig {
        enabled: true,
        url_base: "api.typesafe.ai".to_owned(),
        ..JevConfig::default()
    };
    let error = JevClientConfig::from_config(&without_scheme, Secret::new("x".to_owned()))
        .expect_err("endereço sem esquema");
    assert!(error.to_string().contains("http:// ou https://"));

    let http = JevConfig {
        enabled: true,
        url_base: "http://127.0.0.1:9".to_owned(),
        ..JevConfig::default()
    };
    let without_key = JevClientConfig::from_config(&http, Secret::new("   ".to_owned()))
        .expect_err("recusa chave vazia");
    assert!(matches!(without_key, JevError::MissingKey));
    assert!(!format!("{http:?}").contains(CHAVE));
}

#[test]
fn batch_reads_the_api_answer_format() {
    let body = serde_json::json!({
        "answers": {
            "a": {"type": "choice", "choice": "sim", "probabilities": {"sim": 0.8, "nao": 0.2}, "confidence": 0.8},
            "b": {"type": "score", "score": 0.42, "confidence": 0.5},
            "c": {"type": "noul", "noul": 0.11}
        },
        "usage": {"input_tokens": 10, "output_tokens": 5}
    });
    let batch = poph_jev::BatchResponse::from_json(&body).expect("lote");
    assert_eq!(batch.answers.len(), 3);
    assert_eq!(batch.tokens, 15);
    assert_eq!(batch.usage.input, Some(10));
    assert_eq!(batch.usage.output, Some(5));
    assert_eq!(
        batch.usage.total,
        Some(15),
        "sem total_tokens, soma entrada e saída"
    );
    let note: &JevResponse = &batch.answers["b"];
    assert_eq!(note.score, Some(0.42));
    assert!(note.choice.is_none());

    let question = poph_jev::Question::choice("julgue", Default::default());
    assert_eq!(question.tipo, QuestionType::Choice);
}

#[test]
fn reads_usage_in_variants_and_without_usage() {
    let alternativo = serde_json::json!({
        "answers": {},
        "usage": {"prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 11}
    });
    let batch = poph_jev::BatchResponse::from_json(&alternativo).expect("lote");
    assert_eq!(batch.usage.input, Some(7));
    assert_eq!(batch.usage.output, Some(3));
    assert_eq!(
        batch.usage.total,
        Some(11),
        "total_tokens informado manda sobre a soma"
    );
    assert_eq!(batch.tokens, 11);
    assert!(batch.usage.reported());

    let without_usage = serde_json::json!({"answers": {}});
    let batch = poph_jev::BatchResponse::from_json(&without_usage).expect("lote");
    assert!(!batch.usage.reported());
    assert_eq!(batch.usage.as_text(), "não informado");
    assert_eq!(batch.tokens, 0);

    let input_only = serde_json::json!({"answers": {}, "usage": {"input_tokens": 4}});
    let batch = poph_jev::BatchResponse::from_json(&input_only).expect("lote");
    assert_eq!(batch.usage.input, Some(4));
    assert_eq!(batch.usage.output, None);
    assert_eq!(
        batch.usage.total, None,
        "sem um dos lados, não inventa total"
    );
    assert_eq!(
        batch.usage.as_text(),
        "entrada 4 · saída não informado · total não informado"
    );
}
