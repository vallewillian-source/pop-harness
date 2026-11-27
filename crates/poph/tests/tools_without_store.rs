use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_store::{NewAgent, NewOrg, Store};
use poph_types::config::GlobalTokenConfig;
use poph_types::{HarnessConfig, StructuredInstructions, ToolConfig};
use rand_core::OsRng;
use serde_json::{Value, json};

const TOKEN_GLOBAL: &str = "pophg_teste-hr46f";
const FAKE_KEY: &str = "chave-de-teste-hr46f";

fn hash_argon2(value: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

fn fixture(nome: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../poph-files/tests/fixtures")
        .join(nome);
    fs::read(&path).unwrap_or_else(|error| panic!("fixture {nome}: {error}"))
}

fn test_dir(label: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "poph-sem-base-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(dir.join("dados")).expect("pasta de dados");
    dir
}

struct Servidor(Child);

impl Drop for Servidor {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn read_request(fluxo: &mut TcpStream) -> Option<String> {
    let mut received = Vec::new();
    let mut chunk = [0u8; 4096];
    let headers_end = loop {
        let read = fluxo.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        received.extend_from_slice(&chunk[..read]);
        if let Some(position) = received.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers = String::from_utf8_lossy(&received[..headers_end]).to_lowercase();
    let length: usize = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    while received.len() < headers_end + length {
        let read = fluxo.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        received.extend_from_slice(&chunk[..read]);
    }
    Some(String::from_utf8_lossy(&received[headers_end..]).into_owned())
}

fn evento(fluxo: &mut TcpStream, object: Value) {
    let dados = format!("data: {object}\n\n");
    let _ = write!(fluxo, "{:x}\r\n{dados}\r\n", dados.len());
    let _ = fluxo.flush();
}

fn fake_provider(file_id: String) -> (u16, Arc<Mutex<Vec<Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("porta do provedor falso");
    let porta = listener.local_addr().expect("endereço do provedor").port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let stored = Arc::clone(&requests);
    thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(mut fluxo) = connection else { continue };
            let Some(body) = read_request(&mut fluxo) else {
                continue;
            };
            let request: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let numero = {
                let mut list = stored.lock().expect("lista de pedidos");
                list.push(request);
                list.len()
            };
            let _ = write!(
                fluxo,
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                 cache-control: no-cache\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n"
            );
            if numero == 1 {
                evento(
                    &mut fluxo,
                    json!({"choices": [{"delta": {"tool_calls": [{
                        "index": 0,
                        "id": "call-tabela",
                        "type": "function",
                        "function": {
                            "name": "enviar_arquivo",
                            "arguments": json!({"id": file_id}).to_string(),
                        },
                    }]}}]}),
                );
                evento(
                    &mut fluxo,
                    json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}),
                );
            } else {
                for chunk in ["Enviei a ", "tabela de preços", " agora."] {
                    evento(
                        &mut fluxo,
                        json!({"choices": [{"delta": {"content": chunk}}]}),
                    );
                }
                evento(
                    &mut fluxo,
                    json!({"choices": [{"delta": {}, "finish_reason": "stop"}],
                           "usage": {"prompt_tokens": 120, "completion_tokens": 16}}),
                );
            }
            let fim = "data: [DONE]\n\n";
            let _ = write!(fluxo, "{:x}\r\n{fim}\r\n0\r\n\r\n", fim.len());
            let _ = fluxo.flush();
        }
    });
    (porta, requests)
}

fn multipart(texto: &str) -> (String, Vec<u8>) {
    let boundary = "hr46f-boundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"text\"\r\n\r\n{texto}\r\n\
         --{boundary}--\r\n"
    )
    .into_bytes();
    (format!("multipart/form-data; boundary={boundary}"), body)
}

struct Scenario {
    dir: PathBuf,
    base: String,
    org: String,
    agente: String,
    file_id: String,
    bytes: Vec<u8>,
    requests: Arc<Mutex<Vec<Value>>>,
    client: reqwest::Client,
    _server: Servidor,
}

impl Scenario {
    async fn new(label: &str) -> Self {
        let dir = test_dir(label);
        let dados = dir.join("dados");
        let bytes = fixture("tabela.pdf");

        let (org, agente, file_id) = {
            let store = Store::open_at(&dados.join("harness.db"), &dados.join("rag.db"))
                .expect("abre o banco de teste");
            let org = store
                .orgs()
                .create(NewOrg {
                    name: "Org HR-46f".to_owned(),
                    slug: "org-hr46f".to_owned(),
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
            let instructions = StructuredInstructions {
                tools: vec![
                    ToolConfig::new("enviar_arquivo"),
                    ToolConfig::new("consultar_base"),
                    ToolConfig::new("buscar_produto"),
                ],
                ..StructuredInstructions::default()
            };
            store
                .instructions()
                .save(
                    org,
                    agente,
                    0,
                    &instructions.to_json(),
                    &poph_agent::compose_instructions(&instructions),
                    true,
                )
                .expect("instruções");
            let written = poph_files::blob::write(&dados.join("blobs"), org, &bytes).expect("blob");
            let file = store
                .library()
                .create(
                    org,
                    poph_store::NewLibraryFile {
                        agent_id: agente,
                        name: "Tabela de precos".to_owned(),
                        description: "tabela de preços".to_owned(),
                        mime: "application/pdf".to_owned(),
                        size_bytes: bytes.len() as u64,
                        sha256: written.sha256.clone(),
                        blob_path: format!("{org}/{}", written.sha256),
                    },
                )
                .expect("arquivo na biblioteca");
            (org, agente, file.id.to_string())
        };

        let (llm_port, requests) = fake_provider(file_id.clone());
        let porta = {
            let listener = TcpListener::bind("127.0.0.1:0").expect("porta do servidor");
            listener.local_addr().expect("endereço do servidor").port()
        };

        let mut config = HarnessConfig::default();
        config.data.directory = dados.clone();
        config.server.port = porta;
        config.llm.url_base = format!("http://127.0.0.1:{llm_port}/v1");
        config.llm.retries = 0;
        config.llm.timeout_seconds = 10;
        config.turn.dispatch_interval_ms = 100;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste-hr46f".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        fs::write(
            dir.join("poph.toml"),
            toml::to_string(&config).expect("configuração em TOML"),
        )
        .expect("grava a configuração");

        let servidor = Servidor(
            Command::new(env!("CARGO_BIN_EXE_poph"))
                .args(["serve", "--config", "poph.toml"])
                .current_dir(&dir)
                .env_clear()
                .env("ANTIGRAVITY_API_KEY", FAKE_KEY)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("sobe o poph serve"),
        );

        let scenario = Self {
            dir,
            base: format!("http://127.0.0.1:{porta}"),
            org: org.to_string(),
            agente: agente.to_string(),
            file_id,
            bytes,
            requests,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("cliente HTTP"),
            _server: servidor,
        };
        scenario.wait_until_up().await;
        scenario
    }

    async fn wait_until_up(&self) {
        let started = Instant::now();
        loop {
            if let Ok(response) = self
                .client
                .get(format!("{}/healthz", self.base))
                .send()
                .await
                && response.status().is_success()
            {
                return;
            }
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "o servidor não subiu: {}",
                self.dir.display()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn root(&self) -> String {
        format!(
            "{}/admin/v1/orgs/{}/agents/{}/test-chat",
            self.base, self.org, self.agente
        )
    }

    async fn create_conversation(&self) -> String {
        let response = self
            .client
            .post(self.root())
            .bearer_auth(TOKEN_GLOBAL)
            .send()
            .await
            .expect("criar conversa");
        assert_eq!(response.status(), 201, "criar conversa");
        let body: Value = response.json().await.expect("json da conversa");
        body["id"].as_str().expect("id da conversa").to_owned()
    }

    async fn send(&self, conversa: &str) -> String {
        let (tipo, body) = multipart("manda a tabela de preços por favor");
        let response = self
            .client
            .post(format!("{}/{conversa}/messages", self.root()))
            .bearer_auth(TOKEN_GLOBAL)
            .header("content-type", tipo)
            .body(body)
            .send()
            .await
            .expect("envia a mensagem");
        assert_eq!(response.status(), 200, "envia a mensagem");
        response.text().await.expect("corpo do SSE")
    }

    async fn history(&self, conversa: &str) -> (Value, String) {
        let response = self
            .client
            .get(format!("{}/{conversa}", self.root()))
            .bearer_auth(TOKEN_GLOBAL)
            .send()
            .await
            .expect("histórico");
        assert_eq!(response.status(), 200, "histórico");
        let texto = response.text().await.expect("corpo do histórico");
        let json: Value = serde_json::from_str(&texto).expect("histórico em JSON");
        (json, texto)
    }

    async fn download(&self, file: &str) -> reqwest::Response {
        self.client
            .get(format!(
                "{}/admin/v1/orgs/{}/agents/{}/files/{file}/content",
                self.base, self.org, self.agente
            ))
            .bearer_auth(TOKEN_GLOBAL)
            .send()
            .await
            .expect("download do arquivo")
    }
}

#[tokio::test]
async fn without_embedding_model_turn_offers_file_tool_and_file_is_sent() {
    let scenario = Scenario::new("enviar-arquivo").await;
    let conversa = scenario.create_conversation().await;
    let sse = scenario.send(&conversa).await;

    let names: Vec<&str> = sse
        .lines()
        .filter_map(|line| line.strip_prefix("event: "))
        .collect();
    assert_eq!(
        names,
        vec![
            "mensagem",
            "inicio",
            "ferramenta",
            "texto",
            "texto",
            "texto",
            "fim"
        ],
        "eventos: {sse}"
    );
    assert!(sse.contains("\"state\":\"done\""), "fim do turno: {sse}");

    let requests = scenario.requests.lock().expect("pedidos").clone();
    assert_eq!(requests.len(), 2, "o turno faz duas chamadas ao modelo");
    let tools: Vec<String> = requests[0]["tools"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(
        tools,
        vec!["enviar_arquivo".to_owned()],
        "ferramentas oferecidas sem base: {tools:?}"
    );
    assert!(
        requests[1]["messages"].to_string().contains("\"tool\""),
        "o resultado da ferramenta não voltou ao modelo"
    );

    let (history, texto) = scenario.history(&conversa).await;
    let turno = &history["turns"][0];
    assert_eq!(turno["state"], "done");
    let files = turno["files_out"].as_array().expect("files_out");
    assert_eq!(files.len(), 1, "files_out: {files:?}");
    assert_eq!(files[0]["arquivo_id"], scenario.file_id.as_str());
    assert_eq!(files[0]["mime"], "application/pdf");

    for forbidden in ["blob_ref", "blobs/", "conteudo_json"] {
        assert!(
            !texto.contains(forbidden),
            "{forbidden} apareceu no histórico"
        );
    }

    let response = scenario.download(&scenario.file_id).await;
    assert_eq!(response.status(), 200, "download");
    let downloaded = response.bytes().await.expect("bytes do download");
    assert_eq!(
        downloaded.as_ref(),
        scenario.bytes.as_slice(),
        "bytes idênticos"
    );
}

#[tokio::test]
async fn without_embedding_model_disabled_tool_stays_out() {
    let dir = test_dir("filtro");
    let dados = dir.join("dados");
    let (org, agente) = {
        let store = Store::open_at(&dados.join("harness.db"), &dados.join("rag.db"))
            .expect("abre o banco de teste");
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Org HR-46f filtro".to_owned(),
                slug: "org-hr46f-filtro".to_owned(),
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
        let instructions = StructuredInstructions {
            tools: vec![ToolConfig {
                name: "enviar_arquivo".to_owned(),
                enabled: false,
                when_to_use: String::new(),
            }],
            ..StructuredInstructions::default()
        };
        store
            .instructions()
            .save(
                org,
                agente,
                0,
                &instructions.to_json(),
                &poph_agent::compose_instructions(&instructions),
                true,
            )
            .expect("instruções");
        (org, agente)
    };

    let (llm_port, requests) = fake_provider("nao-usado".to_owned());
    let porta = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("porta do servidor");
        listener.local_addr().expect("endereço").port()
    };
    let mut config = HarnessConfig::default();
    config.data.directory = dados;
    config.server.port = porta;
    config.llm.url_base = format!("http://127.0.0.1:{llm_port}/v1");
    config.llm.retries = 0;
    config.turn.dispatch_interval_ms = 100;
    config.admin.tokens_globais = vec![GlobalTokenConfig {
        label: "teste-hr46f".to_owned(),
        hash: hash_argon2(TOKEN_GLOBAL),
    }];
    fs::write(
        dir.join("poph.toml"),
        toml::to_string(&config).expect("TOML"),
    )
    .expect("configuração");
    let _server = Servidor(
        Command::new(env!("CARGO_BIN_EXE_poph"))
            .args(["serve", "--config", "poph.toml"])
            .current_dir(&dir)
            .env_clear()
            .env("ANTIGRAVITY_API_KEY", FAKE_KEY)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("sobe o poph serve"),
    );

    let base = format!("http://127.0.0.1:{porta}");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("cliente HTTP");
    let started = Instant::now();
    loop {
        if let Ok(response) = client.get(format!("{base}/healthz")).send().await
            && response.status().is_success()
        {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "servidor não subiu"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let root = format!("{base}/admin/v1/orgs/{org}/agents/{agente}/test-chat");
    let conversa: Value = client
        .post(&root)
        .bearer_auth(TOKEN_GLOBAL)
        .send()
        .await
        .expect("criar conversa")
        .json()
        .await
        .expect("json da conversa");
    let conversa = conversa["id"].as_str().expect("id").to_owned();
    let (tipo, body) = multipart("olá");
    let response = client
        .post(format!("{root}/{conversa}/messages"))
        .bearer_auth(TOKEN_GLOBAL)
        .header("content-type", tipo)
        .body(body)
        .send()
        .await
        .expect("envia a mensagem");
    assert_eq!(response.status(), 200);
    let _ = response.text().await.expect("SSE");

    let requests = requests.lock().expect("pedidos").clone();
    let tools: Vec<String> = requests[0]["tools"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_owned))
        .collect();
    assert!(
        tools.is_empty(),
        "ferramenta desabilitada foi oferecida: {tools:?}"
    );
}
