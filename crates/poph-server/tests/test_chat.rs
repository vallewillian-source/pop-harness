use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_agent::actors::{ActorOptions, Actors};
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::entry_output::{FlowOptions, Fluxo};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::turno::{DefaultRegistry, ToolRegistry, TurnEngine, TurnOptions, TurnProcessor};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, LlmProvider, Request, Script};
use poph_files::FileConfig;
use poph_server::ServerState;
use poph_server::api::test_chat::CHANNEL_OF_TEST;
use poph_store::{ConversationStatus, Direction, NewAgent, NewOrg, Page, Store, TurnState};
use poph_types::config::GlobalTokenConfig;
use poph_types::{
    ConversationId, FileId, HarnessConfig, OrgId, StructuredInstructions, ToolConfig,
};
use rand_core::OsRng;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-hr22";
const RESPONSE_TEXT: &str = "O prazo é de 5 dias úteis.";

fn hash_argon2(value: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

const EXTRATOR: &str = "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{\"ok\":true,\"texto\":\"POLITICA INTERNA: nunca prometa entrega em 24 horas\",\"caracteres\":54}'\n";

fn fake_extractor(folder: &Path) -> PathBuf {
    let path = folder.join("extrator-falso.sh");
    fs::write(&path, EXTRATOR).expect("script do extrator");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("permissão do script");
    path
}

fn fixture(nome: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../poph-files/tests/fixtures")
        .join(nome);
    fs::read(&path).unwrap_or_else(|error| panic!("fixture {nome}: {error}"))
}

#[derive(Debug, Clone)]
struct SseEvent {
    nome: String,
    id: Option<String>,
    dados: Value,
}

fn events(texto: &str) -> Vec<SseEvent> {
    let mut saida = Vec::new();
    let mut nome: Option<String> = None;
    let mut id: Option<String> = None;
    let mut dados = String::new();
    for line in texto.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if let Some(nome) = nome.take() {
                saida.push(SseEvent {
                    nome,
                    id: id.take(),
                    dados: serde_json::from_str(&dados).unwrap_or(Value::Null),
                });
                dados.clear();
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("event:") {
            nome = Some(rest.trim().to_owned());
        } else if let Some(rest) = line.strip_prefix("id:") {
            id = Some(rest.trim().to_owned());
        } else if let Some(rest) = line.strip_prefix("data:") {
            dados.push_str(rest.trim_start());
        }
    }
    saida
}

fn names(events: &[SseEvent]) -> Vec<String> {
    events.iter().map(|e| e.nome.clone()).collect()
}

fn multipart(texto: Option<&str>, files: &[(&str, &[u8])]) -> (String, Vec<u8>) {
    let limit = "hr22-boundary";
    let mut body = Vec::new();
    if let Some(texto) = texto {
        body.extend_from_slice(
            format!(
                "--{limit}\r\nContent-Disposition: form-data; name=\"text\"\r\n\r\n{texto}\r\n"
            )
            .as_bytes(),
        );
    }
    for (nome, bytes) in files {
        body.extend_from_slice(
            format!(
                "--{limit}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{nome}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{limit}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={limit}"), body)
}

struct Scenario {
    base: String,
    client: Client,
    store: Arc<Store>,
    org: OrgId,
    agente: poph_types::AgentId,
    llm: Arc<FakeProvider>,
    file: Option<FileId>,
    _folder: PathBuf,
    desligar: Option<oneshot::Sender<()>>,
}

impl Drop for Scenario {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        let _ = fs::remove_dir_all(&self._folder);
    }
}

impl Scenario {
    async fn new(label: &str, scripts: Vec<Script>, enable_turn: bool) -> Self {
        Self::build(label, enable_turn, None, |_| scripts).await
    }

    async fn with_library(
        label: &str,
        content: &[u8],
        scripts: impl FnOnce(FileId) -> Vec<Script>,
    ) -> Self {
        Self::build(label, true, Some(content), |id| {
            scripts(id.expect("arquivo semeado"))
        })
        .await
    }

    async fn build(
        label: &str,
        enable_turn: bool,
        seed: Option<&[u8]>,
        scripts: impl FnOnce(Option<FileId>) -> Vec<Script>,
    ) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let folder = std::env::temp_dir().join(format!(
            "poph-teste-chat-{label}-{}-{timestamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(folder.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = folder.join("dados");
        config.server.port = 7483;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Org HR-22".to_owned(),
                slug: "org-hr22".to_owned(),
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

        let file = seed.map(|bytes| {
            let blobs = folder.join("dados").join("blobs");
            let recorded = poph_files::blob::write(&blobs, org, bytes).expect("grava o blob");
            let file = store
                .library()
                .create(
                    org,
                    poph_store::NewLibraryFile {
                        agent_id: agente,
                        name: "catalogo.pdf".to_owned(),
                        description: "catálogo de serviços".to_owned(),
                        mime: "application/pdf".to_owned(),
                        size_bytes: bytes.len() as u64,
                        sha256: recorded.sha256.clone(),
                        blob_path: format!("{org}/{}", recorded.sha256),
                    },
                )
                .expect("arquivo na biblioteca");
            let instructions = StructuredInstructions {
                tools: vec![ToolConfig {
                    name: poph_agent::NAME_SEND_FILE.to_owned(),
                    enabled: true,
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
                    "# Bia\n\nVocê é a atendente da FixProLar.",
                    true,
                )
                .expect("instruções com a ferramenta");
            file.id
        });

        let fake = Arc::new(FakeProvider::with_scripts(scripts(file)));
        let mut estado = ServerState::new(Arc::clone(&store), config).expect("estado do servidor");
        let mut files = FileConfig::default(folder.join("dados").join("blobs"));
        files.extractor_path = Some(fake_extractor(&folder));
        estado = estado.with_files(files);

        if enable_turn {
            let provedor: Arc<dyn LlmProvider> = fake.clone();
            let mut engine = TurnEngine::new(
                Arc::clone(&store),
                ConfigHot::new(
                    Arc::clone(&store),
                    HotConfigOptions::default(),
                    MetricsMemory::own(),
                ),
                Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own())),
                provedor,
                TurnOptions::default(),
                MetricsTurn::own(),
            );
            if file.is_some() {
                let registro =
                    poph_agent::library::register(DefaultRegistry::new(), Arc::clone(&store));
                engine = engine.with_tools(Arc::new(registro) as Arc<dyn ToolRegistry>);
            }
            let engine = Arc::new(engine);
            let processor = Arc::new(TurnProcessor::new(Arc::clone(&engine)));
            let actors = Actors::new(
                processor.clone(),
                ActorOptions::default(),
                MetricsMemory::own(),
            );
            let fluxo = Fluxo::new(Arc::clone(&store), actors, FlowOptions::default());
            processor.attach_io(fluxo.clone());
            estado = estado
                .with_engine(engine)
                .with_turn(fluxo, Duration::from_millis(20));
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, signal) = oneshot::channel::<()>();
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
            store,
            org,
            agente,
            llm: fake,
            file,
            _folder: folder,
            desligar: Some(desligar),
        };
        scenario.wait_boot().await;
        scenario
    }

    async fn wait_boot(&self) {
        let start = Instant::now();
        loop {
            match self
                .client
                .get(format!("{}/healthz", self.base))
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => return,
                _ if start.elapsed() < Duration::from_secs(10) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                other => panic!("servidor não subiu: {other:?}"),
            }
        }
    }

    fn root(&self) -> String {
        format!(
            "/admin/v1/orgs/{}/agents/{}/test-chat",
            self.org, self.agente
        )
    }

    async fn create_conversation(&self) -> String {
        let response = self
            .client
            .post(format!("{}{}", self.base, self.root()))
            .bearer_auth(TOKEN_GLOBAL)
            .send()
            .await
            .expect("criar conversa");
        assert_eq!(response.status(), StatusCode::CREATED);
        let body: Value = response.json().await.expect("json da conversa");
        assert_eq!(body["channel"], CHANNEL_OF_TEST);
        assert_eq!(body["status"], "open");
        assert_eq!(body["agent_id"], self.agente.to_string());
        body["id"].as_str().expect("id da conversa").to_owned()
    }

    fn messages_url(&self, conversa: &str) -> String {
        format!("{}{}/{conversa}/messages", self.base, self.root())
    }

    async fn send(
        &self,
        conversa: &str,
        texto: Option<&str>,
        files: &[(&str, &[u8])],
    ) -> reqwest::Response {
        let (tipo, body) = multipart(texto, files);
        self.client
            .post(self.messages_url(conversa))
            .bearer_auth(TOKEN_GLOBAL)
            .header("content-type", tipo)
            .body(body)
            .send()
            .await
            .expect("envia a mensagem")
    }

    async fn history(&self, conversa: &str) -> Value {
        self.client
            .get(format!("{}{}/{conversa}", self.base, self.root()))
            .bearer_auth(TOKEN_GLOBAL)
            .send()
            .await
            .expect("histórico")
            .json()
            .await
            .expect("json do histórico")
    }

    async fn wait_turn_finish(&self, conversa: &str) -> Value {
        let id: ConversationId = conversa.parse().expect("id da conversa");
        let start = Instant::now();
        loop {
            let turns = self
                .store
                .turns()
                .list(self.org, id, Page::with_limit(50))
                .expect("turnos");
            if let Some(turno) = turns.last()
                && turno.state != TurnState::Running
            {
                return json!({
                    "state": turno.state.as_text(),
                    "error": turno.error,
                });
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "o turno não terminou a tempo: {turns:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn outbound_messages(&self, conversa: &str) -> usize {
        let id: ConversationId = conversa.parse().expect("id da conversa");
        self.store
            .messages()
            .list(self.org, id, Page::with_limit(200))
            .expect("mensagens")
            .iter()
            .filter(|m| m.direction == Direction::Outbound)
            .count()
    }
}

fn default_script() -> Vec<Script> {
    vec![Script::Events(vec![
        LlmEvent::Texto("O prazo é ".into()),
        LlmEvent::Texto("de 5 dias úteis.".into()),
        LlmEvent::Fim(FinishReason::Completed),
    ])]
}

fn request_text(request: &Request) -> String {
    let mut texto = String::new();
    for mensagem in &request.messages {
        match mensagem {
            poph_core::llm::Message::User(parts) => {
                for part in parts {
                    if let poph_core::llm::Part::Texto(excerpt) = part {
                        texto.push_str(excerpt);
                        texto.push('\n');
                    }
                }
            }
            poph_core::llm::Message::System(instructions) => texto.push_str(instructions),
            _ => {}
        }
    }
    texto
}

#[tokio::test]
async fn test_conversation_gets_sse_in_order_and_attachment_text_in_prompt() {
    let c = Scenario::new("stream", default_script(), true).await;
    let conversa = c.create_conversation().await;

    let docx = fixture("procedimento.docx");
    let response = c
        .send(
            &conversa,
            Some("qual o prazo?"),
            &[("procedimento.docx", &docx)],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream")),
        "content-type do SSE"
    );
    let body = response.text().await.expect("corpo do SSE");
    let events = events(&body);

    assert_eq!(events[0].nome, "mensagem", "eventos: {}", body);
    assert_eq!(
        events[0].dados["attachments"][0]["name"],
        "procedimento.docx"
    );
    assert_eq!(events[0].dados["attachments"][0]["extraction_status"], "ok");
    let expected: Vec<String> = ["mensagem", "inicio", "texto", "texto", "fim"]
        .iter()
        .map(|nome| (*nome).to_owned())
        .collect();
    assert_eq!(names(&events), expected, "eventos: {body}");
    let ids: Vec<_> = events.iter().filter_map(|e| e.id.clone()).collect();
    assert_eq!(ids, vec!["1", "2", "3", "4", "5"], "ids em sequência");
    assert_eq!(events[2].dados["delta"], "O prazo é ");
    assert_eq!(events[3].dados["delta"], "de 5 dias úteis.");
    assert_eq!(events[4].dados["state"], "done");
    assert_eq!(events[4].dados["text"], RESPONSE_TEXT);
    assert!(events[4].dados["turn_id"].is_string());
    assert_eq!(events[4].dados["timings"]["tool_rounds"], 0);

    let request = &c.llm.received()[0];
    let prompt = request_text(request);
    assert!(
        prompt.contains("POLITICA INTERNA: nunca prometa entrega em 24 horas"),
        "texto extraído ausente do prompt: {prompt}"
    );
    assert!(prompt.contains(poph_files::envelope::UNTRUSTED_NOTICE));
    assert!(prompt.contains("qual o prazo?"));

    let history = c.history(&conversa).await;
    let messages = history["messages"].as_array().expect("mensagens");
    assert_eq!(messages.len(), 2, "{history}");
    assert_eq!(messages[0]["direction"], "in");
    assert_eq!(messages[0]["attachments"][0]["name"], "procedimento.docx");
    assert_eq!(messages[0]["attachments"][0]["has_text"], true);
    assert_eq!(messages[1]["direction"], "out");
    assert_eq!(messages[1]["text"], RESPONSE_TEXT);
    let turns = history["turns"].as_array().expect("turnos");
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0]["state"], "done");

    assert_eq!(
        c.store
            .outbox()
            .list_pending(c.org, POPH_BATCH)
            .expect("fila de saída")
            .len(),
        0,
        "nada pendente na outbox"
    );
}

const POPH_BATCH: u32 = 200;

#[tokio::test]
async fn two_connections_same_conversation_do_not_make_two_turns() {
    let c = Scenario::new(
        "duas-abas",
        vec![
            Script::Pending {
                events: vec![LlmEvent::Texto("parcial ".into())],
                after: Duration::from_secs(30),
                error: poph_core::llm::LlmError::TimedOut { seconds: 30 },
            },
            Script::Events(vec![
                LlmEvent::Texto("resposta da segunda".into()),
                LlmEvent::Fim(FinishReason::Completed),
            ]),
        ],
        true,
    )
    .await;
    let conversa = c.create_conversation().await;

    let first = c.send(&conversa, Some("primeira aba"), &[]).await;
    assert_eq!(first.status(), StatusCode::OK);
    let second = c.send(&conversa, Some("segunda aba"), &[]).await;
    assert_eq!(second.status(), StatusCode::CONFLICT);
    let body: Value = second.json().await.expect("erro");
    assert_eq!(body["code"], "turno_em_andamento");

    drop(first);
    let estado = c.wait_turn_finish(&conversa).await;
    assert_eq!(estado["state"], "failed", "cancelado: {estado}");
    assert_eq!(c.outbound_messages(&conversa), 0, "sem resposta parcial");

    let third = c.send(&conversa, Some("agora vai"), &[]).await;
    assert_eq!(third.status(), StatusCode::OK);
    let body = third.text().await.expect("SSE");
    let events = events(&body);
    assert_eq!(
        events
            .last()
            .map(|e| (e.nome.as_str(), e.dados["state"].as_str())),
        Some(("fim", Some("done"))),
        "eventos: {body}"
    );
    assert_eq!(c.outbound_messages(&conversa), 1);
}

#[tokio::test]
async fn disconnect_cancels_turn_without_stuck_queue() {
    let c = Scenario::new(
        "cancelamento",
        vec![Script::Pending {
            events: vec![LlmEvent::Texto("começou ".into())],
            after: Duration::from_secs(30),
            error: poph_core::llm::LlmError::TimedOut { seconds: 30 },
        }],
        true,
    )
    .await;
    let conversa = c.create_conversation().await;

    let response = c.send(&conversa, Some("me responde"), &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    use futures::StreamExt;
    let mut fluxo = response.bytes_stream();
    let mut received = String::new();
    let deadline = Instant::now();
    while !received.contains("começou") {
        let chunk = tokio::time::timeout(Duration::from_secs(5), fluxo.next())
            .await
            .expect("o stream precisa abrir")
            .expect("o SSE precisa emitir")
            .expect("bytes do SSE");
        received.push_str(&String::from_utf8_lossy(&chunk));
        assert!(
            deadline.elapsed() < Duration::from_secs(10),
            "SSE: {received}"
        );
    }
    drop(fluxo);

    let estado = c.wait_turn_finish(&conversa).await;
    assert_eq!(estado["state"], "failed", "{estado}");
    let error = estado["error"].as_str().unwrap_or_default().to_lowercase();
    assert!(
        error.contains("cancel"),
        "o turno precisa registrar o cancelamento: {estado}"
    );
    assert_eq!(c.outbound_messages(&conversa), 0);
    assert_eq!(
        c.store
            .outbox()
            .list_pending(c.org, POPH_BATCH)
            .expect("fila")
            .len(),
        0
    );
}

#[tokio::test]
async fn route_without_turn_responds_503() {
    let c = Scenario::new("sem-turno", Vec::new(), false).await;
    let conversa = c.create_conversation().await;
    let response = c.send(&conversa, Some("oi"), &[]).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = response.json().await.expect("erro");
    assert_eq!(body["code"], "turno_indisponivel");
}

#[tokio::test]
async fn org_isolation_and_reopen_with_if_match() {
    let c = Scenario::new("isolamento", Vec::new(), false).await;
    let conversa = c.create_conversation().await;

    let other = c
        .store
        .orgs()
        .create(NewOrg {
            name: "Outra".to_owned(),
            slug: "outra-org-hr22".to_owned(),
            config_json: "{}".to_owned(),
        })
        .expect("org")
        .id;
    let token = c
        .client
        .post(format!("{}/admin/v1/orgs/{other}/tokens", c.base))
        .bearer_auth(TOKEN_GLOBAL)
        .json(&json!({"label": "outra"}))
        .send()
        .await
        .expect("token da outra org")
        .json::<Value>()
        .await
        .expect("json do token")["value"]
        .as_str()
        .expect("valor do token")
        .to_owned();
    for (method, url) in [
        ("GET", format!("{}{}", c.base, c.root())),
        ("GET", format!("{}{}/{}", c.base, c.root(), conversa)),
        ("POST", format!("{}{}", c.base, c.root())),
    ] {
        let request = if method == "GET" {
            c.client.get(url)
        } else {
            c.client.post(url)
        };
        let response = request.bearer_auth(&token).send().await.expect("pedido");
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method}");
    }
    let other_agent = c
        .store
        .agents()
        .create(
            c.org,
            NewAgent {
                name: "Outro".to_owned(),
                slug: "outro-agente-hr22".to_owned(),
                ..Default::default()
            },
        )
        .expect("agente")
        .id;
    let response = c
        .client
        .get(format!(
            "{}/admin/v1/orgs/{}/agents/{other_agent}/test-chat/{conversa}",
            c.base, c.org
        ))
        .bearer_auth(TOKEN_GLOBAL)
        .send()
        .await
        .expect("histórico com outro agente");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let id: ConversationId = conversa.parse().expect("id");
    c.store
        .conversations()
        .change_status(c.org, id, 1, ConversationStatus::Transferred, None)
        .expect("transfere");
    let url = format!("{}{}/{conversa}/reopen", c.base, c.root());
    let without_if_match = c
        .client
        .post(&url)
        .bearer_auth(TOKEN_GLOBAL)
        .send()
        .await
        .expect("sem If-Match");
    assert_eq!(without_if_match.status(), StatusCode::PRECONDITION_REQUIRED);
    let old = c
        .client
        .post(&url)
        .bearer_auth(TOKEN_GLOBAL)
        .header("If-Match", "1")
        .send()
        .await
        .expect("versão velha");
    assert_eq!(old.status(), StatusCode::PRECONDITION_FAILED);
    let certa = c
        .client
        .post(&url)
        .bearer_auth(TOKEN_GLOBAL)
        .header("If-Match", "2")
        .send()
        .await
        .expect("versão certa");
    assert_eq!(certa.status(), StatusCode::OK);
    let body: Value = certa.json().await.expect("json");
    assert_eq!(body["status"], "open");
}

#[tokio::test]
async fn test_conversation_list_newest_first_with_cursor() {
    let c = Scenario::new("listagem", default_script(), true).await;
    let first = c.create_conversation().await;
    let second = c.create_conversation().await;
    let third = c.create_conversation().await;

    let url = format!("{}{}?limit=2", c.base, c.root());
    let page: Value = c
        .client
        .get(&url)
        .bearer_auth(TOKEN_GLOBAL)
        .send()
        .await
        .expect("lista")
        .json()
        .await
        .expect("json da lista");
    let items = page["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["id"], third);
    assert_eq!(items[1]["id"], second);
    let cursor = page["next_cursor"].as_str().expect("cursor").to_owned();

    let next: Value = c
        .client
        .get(format!("{url}&cursor={cursor}"))
        .bearer_auth(TOKEN_GLOBAL)
        .send()
        .await
        .expect("lista com cursor")
        .json()
        .await
        .expect("json");
    let items = next["items"].as_array().expect("items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], first);
    assert_eq!(next["next_cursor"], Value::Null);

    let response = c.send(&first, None, &[]).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.expect("erro");
    assert_eq!(body["code"], "validation_error");
    assert_eq!(body["details"]["field"], "text");
}

#[tokio::test]
async fn refused_file_returns_in_file_field_and_writes_no_message() {
    let c = Scenario::new("arquivo-recusado", default_script(), true).await;
    let conversa = c.create_conversation().await;
    let response = c
        .send(
            &conversa,
            Some("segue a nota"),
            &[("nota.txt", b"oi, tudo bem?")],
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.expect("erro");
    assert_eq!(body["code"], "validation_error");
    assert_eq!(body["details"]["field"], "file");
    let history = c.history(&conversa).await;
    assert_eq!(
        history["messages"].as_array().map(|m| m.len()),
        Some(0),
        "nada gravado: {history}"
    );
    let ok = c.send(&conversa, Some("agora vai"), &[]).await;
    assert_eq!(ok.status(), StatusCode::OK);
    let body = ok.text().await.expect("SSE");
    assert!(body.contains("event: fim"), "{body}");
}

fn script_sends_file(file: FileId) -> Vec<Script> {
    vec![
        Script::Events(vec![
            LlmEvent::Tool(poph_core::llm::ToolCall {
                id: "call-catalogo".to_owned(),
                nome: poph_agent::NAME_SEND_FILE.to_owned(),
                arguments: json!({ "id": file.to_string() }),
            }),
            LlmEvent::Fim(FinishReason::ToolCalls),
        ]),
        Script::Events(vec![
            LlmEvent::Texto("Estou enviando o catálogo.".into()),
            LlmEvent::Fim(FinishReason::Completed),
        ]),
    ]
}

#[tokio::test]
async fn history_shows_agent_file_and_download_returns_bytes() {
    const CONTENT: &[u8] = b"%PDF-1.4\ncatalogo de servicos HR-46d\n";
    let c = Scenario::with_library("arquivo-enviado", CONTENT, script_sends_file).await;
    let conversa = c.create_conversation().await;

    let response = c.send(&conversa, Some("manda o catálogo"), &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("SSE");
    let events = events(&body);
    assert_eq!(
        events
            .last()
            .map(|e| (e.nome.as_str(), e.dados["state"].as_str())),
        Some(("fim", Some("done"))),
        "eventos: {body}"
    );

    let history = c.history(&conversa).await;
    let raw = history.to_string();
    assert!(
        !raw.contains("blob_ref"),
        "o caminho do blob nunca sai na resposta: {raw}"
    );
    assert!(!raw.contains("blobs/"), "nenhum caminho de pasta: {raw}");
    assert!(
        !raw.contains("catalogo de servicos"),
        "nenhum conteúdo do arquivo na resposta: {raw}"
    );

    let turns = history["turns"].as_array().expect("turnos");
    assert_eq!(turns.len(), 1, "um turno: {history}");
    let files = turns[0]["files_out"].as_array().expect("files_out");
    assert_eq!(files.len(), 1, "um arquivo enviado: {history}");
    let sent = &files[0];
    assert_eq!(sent["arquivo_id"], c.file.unwrap().to_string());
    assert_eq!(sent["name"], "catalogo.pdf");
    assert_eq!(sent["mime"], "application/pdf");
    assert_eq!(sent["size_bytes"], CONTENT.len());
    assert!(
        ["pending", "sent"].contains(&sent["state"].as_str().unwrap_or("")),
        "{sent}"
    );
    assert_eq!(sent["turn_id"], turns[0]["id"]);
    assert!(sent["seq"].as_u64().unwrap_or(0) >= 1, "{sent}");
    assert!(sent["created_at"].as_i64().unwrap_or(0) > 0, "{sent}");

    let url = format!(
        "{}/admin/v1/orgs/{}/agents/{}/files/{}/content",
        c.base,
        c.org,
        c.agente,
        sent["arquivo_id"].as_str().expect("arquivo_id")
    );
    let response = c
        .client
        .get(&url)
        .bearer_auth(TOKEN_GLOBAL)
        .send()
        .await
        .expect("download");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-content-sha256")
            .and_then(|v| v.to_str().ok()),
        sent["sha256"].as_str(),
        "o sha256 da resposta é o mesmo do histórico"
    );
    let downloaded = response.bytes().await.expect("bytes").to_vec();
    assert_eq!(downloaded, CONTENT, "bytes idênticos");
    assert_eq!(
        poph_files::blob::sha256_hex(&downloaded),
        sent["sha256"].as_str().unwrap_or_default()
    );

    let other = c
        .store
        .orgs()
        .create(NewOrg {
            name: "Outra".to_owned(),
            slug: "outra-org-hr46d".to_owned(),
            config_json: "{}".to_owned(),
        })
        .expect("org")
        .id;
    let token = c
        .client
        .post(format!("{}/admin/v1/orgs/{other}/tokens", c.base))
        .bearer_auth(TOKEN_GLOBAL)
        .json(&json!({"label": "outra"}))
        .send()
        .await
        .expect("token da outra org")
        .json::<Value>()
        .await
        .expect("json do token")["value"]
        .as_str()
        .expect("valor do token")
        .to_owned();
    let foreign = c
        .client
        .get(format!("{}{}/{conversa}", c.base, c.root()))
        .bearer_auth(&token)
        .send()
        .await
        .expect("histórico alheio");
    assert_eq!(foreign.status(), StatusCode::FORBIDDEN);

    let no_file = c.create_conversation().await;
    let response = c.send(&no_file, Some("só texto"), &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = response.text().await.expect("SSE");
    let history = c.history(&no_file).await;
    let turns = history["turns"].as_array().expect("turnos");
    assert_eq!(turns.len(), 1);
    assert_eq!(
        turns[0]["files_out"].as_array().map(Vec::len),
        Some(0),
        "lista vazia: {history}"
    );
}

#[tokio::test]
async fn history_carries_tool_call_record() {
    const CONTENT: &[u8] = b"%PDF-1.4\ncatalogo de servicos HR-47b\n";
    let c = Scenario::with_library("registro-chamadas", CONTENT, script_sends_file).await;
    let conversa = c.create_conversation().await;

    let response = c.send(&conversa, Some("manda o catálogo"), &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = response.text().await.expect("SSE");

    let history = c.history(&conversa).await;
    let turns = history["turns"].as_array().expect("turnos");
    assert_eq!(turns.len(), 1, "um turno: {history}");
    let turno = &turns[0];

    assert_eq!(
        turno["tools"],
        json!([poph_agent::NAME_SEND_FILE]),
        "{history}"
    );

    let calls = turno["tool_calls"].as_array().expect("tool_calls");
    assert_eq!(calls.len(), 1, "uma chamada: {history}");
    let call = &calls[0];
    assert_eq!(call["nome"], poph_agent::NAME_SEND_FILE);
    assert_eq!(call["estado"], "executada");
    assert!(call["motivo"].is_null());
    assert_eq!(call["rodada"], 0);
    assert_eq!(call["ordem"], 1);
    assert!(
        call["argumentos"]
            .as_str()
            .expect("argumentos")
            .contains(&c.file.expect("arquivo").to_string()),
        "o argumento traz o id que o modelo enviou: {call}"
    );
    assert!(
        call["resultado"]
            .as_str()
            .expect("resultado")
            .contains("Arquivo enfileirado"),
        "o resultado é o texto devolvido ao modelo: {call}"
    );

    let no_tool = c.create_conversation().await;
    let response = c.send(&no_tool, Some("só texto"), &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = response.text().await.expect("SSE");
    let history = c.history(&no_tool).await;
    let turns = history["turns"].as_array().expect("turnos");
    assert_eq!(
        turns[0]["tool_calls"].as_array().map(Vec::len),
        Some(0),
        "lista vazia: {history}"
    );
}
