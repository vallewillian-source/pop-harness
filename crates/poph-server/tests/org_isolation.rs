use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use futures::StreamExt;
use futures::future::BoxFuture;
use poph_agent::actors::{ActorOptions, Actors};
use poph_agent::base_knowledge::{
    KnowledgeBase, KnowledgeSearcher, QueryKnowledge, SearchParameters,
};
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::entry_output::{FlowOptions, Fluxo};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::products::{ProductCatalog, ProductParameters, ProductSearcher, SearchProduct};
use poph_agent::turno::{TurnEngine, TurnOptions, TurnProcessor};
use poph_core::llm::{
    FakeProvider, FinishReason, LlmError, LlmEvent, LlmProvider, LlmStream, Message, Request,
    Script, ToolCall,
};
use poph_files::FileConfig;
use poph_rag::{FakeEmbedder, Index, IndexConfig};
use poph_server::ServerState;
use poph_store::{
    AuditEntry, Direction, NewChannelAttachment, NewMessage, OutboxState, Page, Role, Store,
};
use poph_types::HarnessConfig;
use poph_types::config::GlobalTokenConfig;
use poph_types::{ConversationId, OrgId};
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-hr54";
const MARKER_A: &str = "MARCADOR-ORG-A";
const MARKER_B: &str = "MARCADOR-ORG-B";
const TOOLS: &[&str] = &[
    "consultar_base",
    "buscar_produto",
    "enviar_arquivo",
    "cart_add_item",
    "cart_summary",
];
const NON_TENANT: &[(&str, &str)] = &[
    ("get", "/healthz"),
    ("get", "/metrics"),
    ("post", "/admin/v1/cluster/reload"),
];

fn hash_argon2(value: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

fn png() -> &'static [u8] {
    b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89"
}

struct TestServer {
    base: String,
    client: Client,
    directory: PathBuf,
    #[allow(dead_code)]
    store: Arc<Store>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl TestServer {
    async fn new(
        label: &str,
        server_state: impl FnOnce(&Arc<Store>, &HarnessConfig, &PathBuf) -> ServerState,
    ) -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let directory = std::env::temp_dir().join(format!(
            "poph-isolamento-{label}-{}-{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(directory.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = directory.join("dados");
        config.server.port = 7490;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let server_state = server_state(&store, &config, &directory);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let address = listener.local_addr().expect("endereço local");
        let (shutdown, signal) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = poph_server::serve_at(listener, server_state, async {
                let _ = signal.await;
            })
            .await;
        });
        let server = Self {
            base: format!("http://{address}"),
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("cliente HTTP"),
            directory,
            store,
            shutdown: Some(shutdown),
        };
        server.wait_until_ready().await;
        server
    }

    async fn wait_until_ready(&self) {
        let started_at = Instant::now();
        loop {
            match self
                .client
                .get(format!("{}/healthz", self.base))
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => return,
                _ if started_at.elapsed() < Duration::from_secs(10) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                other => panic!("servidor não subiu: {other:?}"),
            }
        }
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: RequestBody,
        if_match: Option<u64>,
    ) -> TestResponse {
        let mut request = self.client.request(method, format!("{}{path}", self.base));
        if !token.is_empty() {
            request = request.bearer_auth(token);
        }
        if let Some(version) = if_match {
            request = request.header("If-Match", version.to_string());
        }
        request = match body {
            RequestBody::NoneExpected => request,
            RequestBody::Json(value) => request.json(&value),
            RequestBody::Multipart(text) => {
                let (kind, bytes) = multipart(&text);
                request.header("content-type", kind).body(bytes)
            }
            RequestBody::File(bytes) => request
                .header("content-type", "application/octet-stream")
                .body(bytes),
        };
        let response = request.send().await.expect("pedido HTTP");
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        let json = serde_json::from_str(&text).unwrap_or(Value::Null);
        TestResponse { status, text, json }
    }

    async fn create(&self, path: &str, token: &str, body: Value, expected: StatusCode) -> Value {
        let r = self
            .call(Method::POST, path, token, RequestBody::Json(body), None)
            .await;
        assert_eq!(r.status, expected, "criar em {path}: {}", r.text);
        r.json
    }

    async fn snapshot(&self, org: &Org) -> Vec<String> {
        let mut paths = vec![
            format!("/admin/v1/orgs/{}", org.id),
            format!("/admin/v1/orgs/{}/agents", org.id),
            format!("/admin/v1/orgs/{}/agents/{}", org.id, org.agent),
            format!(
                "/admin/v1/orgs/{}/agents/{}/instructions/current",
                org.id, org.agent
            ),
            format!("/admin/v1/orgs/{}/agents/{}/files", org.id, org.agent),
            format!(
                "/admin/v1/orgs/{}/agents/{}/files/{}",
                org.id, org.agent, org.file
            ),
            format!(
                "/admin/v1/orgs/{}/agents/{}/integrations",
                org.id, org.agent
            ),
            format!(
                "/admin/v1/orgs/{}/agents/{}/integrations/{}",
                org.id, org.agent, org.integration
            ),
            format!("/admin/v1/orgs/{}/agents/{}/test-chat", org.id, org.agent),
            format!(
                "/admin/v1/orgs/{}/agents/{}/test-chat/{}",
                org.id, org.agent, org.conversation
            ),
            format!("/admin/v1/orgs/{}/conversations", org.id),
            format!(
                "/admin/v1/orgs/{}/conversations/{}/messages",
                org.id, org.conversation
            ),
            format!(
                "/admin/v1/orgs/{}/conversations/{}/turns",
                org.id, org.conversation
            ),
            format!("/admin/v1/orgs/{}/categories", org.id),
            format!("/admin/v1/orgs/{}/categories/{}", org.id, org.category),
            format!("/admin/v1/orgs/{}/departments", org.id),
            format!("/admin/v1/orgs/{}/departments/{}", org.id, org.department),
            format!("/admin/v1/orgs/{}/products", org.id),
            format!("/admin/v1/orgs/{}/products/{}", org.id, org.product),
            format!("/admin/v1/orgs/{}/knowledge", org.id),
            format!("/admin/v1/orgs/{}/knowledge/{}", org.id, org.group),
            format!("/admin/v1/orgs/{}/knowledge/{}/items", org.id, org.group),
            format!(
                "/admin/v1/orgs/{}/knowledge/{}/items/{}",
                org.id, org.group, org.item
            ),
            format!("/admin/v1/orgs/{}/tokens", org.id),
            format!("/admin/v1/orgs/{}/tokens/{}", org.id, org.token_id),
            format!("/admin/v1/orgs/{}/flows", org.id),
            format!("/admin/v1/orgs/{}/flows/{}", org.id, org.flow),
            format!("/admin/v1/orgs/{}/flows/{}/revisions", org.id, org.flow),
            format!("/admin/v1/orgs/{}/fichas", org.id),
            format!("/admin/v1/orgs/{}/fichas/{}", org.id, org.service_case),
            format!(
                "/admin/v1/orgs/{}/fichas/{}/events",
                org.id, org.service_case
            ),
        ];
        paths.sort();
        let mut snapshots = Vec::new();
        for path in paths {
            let r = self
                .call(
                    Method::GET,
                    &path,
                    &org.token,
                    RequestBody::NoneExpected,
                    None,
                )
                .await;
            snapshots.push(format!("{path} -> {} {}", r.status, r.text));
        }
        snapshots
    }

    fn audit(&self, org: &Org) -> Vec<AuditEntry> {
        let id = org.id.parse::<OrgId>().expect("id da org");
        self.store
            .audit()
            .list(id, Page::with_limit(200))
            .expect("auditoria da org")
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let _ = fs::remove_dir_all(&self.directory);
    }
}

enum RequestBody {
    NoneExpected,
    Json(Value),
    Multipart(String),
    File(Vec<u8>),
}

struct TestResponse {
    status: StatusCode,
    text: String,
    json: Value,
}

fn multipart(text: &str) -> (String, Vec<u8>) {
    let limit = "hr54-boundary";
    let body = format!(
        "--{limit}\r\nContent-Disposition: form-data; name=\"text\"\r\n\r\n{text}\r\n--{limit}--\r\n"
    );
    (
        format!("multipart/form-data; boundary={limit}"),
        body.into_bytes(),
    )
}

#[derive(Clone)]
struct Org {
    id: String,
    token: String,
    token_id: String,
    agent: String,
    group: String,
    item: String,
    file: String,
    integration: String,
    category: String,
    department: String,
    product: String,
    conversation: String,
    flow: String,
    version_agent: u64,
    service_case: String,
    execution: String,
    message: String,
    attachment: String,
    site: String,
    site_turn: String,
    marker: &'static str,
}

impl Org {
    fn params(&self) -> Vec<(&'static str, String)> {
        vec![
            ("agent_id", self.agent.clone()),
            ("group_id", self.group.clone()),
            ("item_id", self.item.clone()),
            ("file_id", self.file.clone()),
            ("integration_id", self.integration.clone()),
            ("category_id", self.category.clone()),
            ("department_id", self.department.clone()),
            ("product_id", self.product.clone()),
            ("conversation_id", self.conversation.clone()),
            ("message_id", self.message.clone()),
            ("attachment_id", self.attachment.clone()),
            ("token_id", self.token_id.clone()),
            ("flow_id", self.flow.clone()),
            ("ficha_id", self.service_case.clone()),
            ("run_id", self.execution.clone()),
            ("id", self.site.clone()),
            ("turn_id", self.site_turn.clone()),
            ("kind", "sms".to_owned()),
        ]
    }

    fn ids(&self) -> Vec<String> {
        let mut ids = vec![self.id.clone()];
        ids.extend(
            self.params()
                .into_iter()
                .filter(|(display_name, _)| *display_name != "kind")
                .map(|(_, value)| value),
        );
        ids
    }

    fn path(&self, model: &str, org_id: &str) -> String {
        let mut path = model.replace("{org_id}", org_id).replace("{number}", "1");
        for (display_name, value) in self.params() {
            path = path.replace(&format!("{{{display_name}}}"), &value);
        }
        path
    }
}

async fn seed_org(s: &TestServer, slug: &str, marker: &'static str) -> Org {
    let org = s
        .create(
            "/admin/v1/orgs",
            TOKEN_GLOBAL,
            json!({"name": format!("Org {marker}"), "slug": slug}),
            StatusCode::CREATED,
        )
        .await;
    let id = org["id"].as_str().unwrap().to_owned();
    let token_created = s
        .create(
            &format!("/admin/v1/orgs/{id}/tokens"),
            TOKEN_GLOBAL,
            json!({"label": format!("token {marker}")}),
            StatusCode::CREATED,
        )
        .await;
    let token = token_created["value"].as_str().unwrap().to_owned();
    let token_id = token_created["token"]["id"].as_str().unwrap().to_owned();
    let root = format!("/admin/v1/orgs/{id}");

    let agent = s
        .create(
            &format!("{root}/agents"),
            &token,
            json!({"name": format!("Agente {marker}"), "slug": format!("agente-{slug}")}),
            StatusCode::CREATED,
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    s.create(
        &format!("{root}/agents/{agent}/instructions"),
        &token,
        json!({"structured": {"tools": TOOLS.iter().map(|display_name| json!({"name": display_name, "enabled": true})).collect::<Vec<_>>()}}),
        StatusCode::CREATED,
    )
    .await;
    let category = s
        .create(
            &format!("{root}/categories"),
            &token,
            json!({"name": format!("Categoria {marker}")}),
            StatusCode::CREATED,
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let department = s
        .create(
            &format!("{root}/departments"),
            &token,
            json!({"name": format!("Departamento {marker}")}),
            StatusCode::CREATED,
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let product = s
        .create(
            &format!("{root}/products"),
            &token,
            json!({"name": format!("Produto {marker}"), "price_cents": 12345, "category_id": category}),
            StatusCode::CREATED,
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let group = s
        .create(
            &format!("{root}/knowledge"),
            &token,
            json!({"name": format!("Grupo {marker}")}),
            StatusCode::CREATED,
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let item = s
        .create(
            &format!("{root}/knowledge/{group}/items"),
            &token,
            json!({"title": format!("Item {marker}"), "content": format!("Conteúdo sigiloso {marker}")}),
            StatusCode::CREATED,
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    s.create(
        &format!("{root}/agents/{agent}/knowledge/{group}"),
        &token,
        json!({}),
        StatusCode::OK,
    )
    .await;
    let file = s
        .call(
            Method::POST,
            &format!("{root}/agents/{agent}/files?filename=arquivo-{marker}.png"),
            &token,
            RequestBody::File(png().to_vec()),
            None,
        )
        .await;
    assert_eq!(file.status, StatusCode::CREATED, "arquivo: {}", file.text);
    let file = file.json["id"].as_str().unwrap().to_owned();
    let integration = s
        .create(
            &format!("{root}/agents/{agent}/integrations"),
            &token,
            json!({"kind": "sms", "status": "active"}),
            StatusCode::CREATED,
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let conversation = s
        .call(
            Method::POST,
            &format!("{root}/agents/{agent}/test-chat"),
            &token,
            RequestBody::NoneExpected,
            None,
        )
        .await;
    assert_eq!(
        conversation.status,
        StatusCode::CREATED,
        "conversa: {}",
        conversation.text
    );
    let conversation_id: ConversationId =
        conversation.json["id"].as_str().unwrap().parse().unwrap();
    let conversation = conversation_id.to_string();
    let org_id: OrgId = id.parse().unwrap();
    let message = s
        .store
        .messages()
        .write_with_attachments(
            org_id,
            NewMessage {
                conversation_id,
                direction: Direction::Inbound,
                role: Role::User,
                texto: format!("anexo {marker}"),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
            &[NewChannelAttachment {
                name: format!("arquivo-{marker}.png"),
                mime: "image/png".to_owned(),
                size_bytes: png().len() as u64,
                sha256: poph_files::blob::sha256_hex(png()),
                blob_path: format!("{id}/{}", poph_files::blob::sha256_hex(png())),
                extracted_text: None,
                extraction_status: "skipped",
            }],
        )
        .expect("mensagem com anexo");
    let attachment = s
        .store
        .messages()
        .attachments(org_id, message.id)
        .expect("anexos da mensagem")
        .remove(0)
        .id
        .to_string();

    let flow = s
        .create(
            &format!("{root}/flows"),
            &token,
            json!({"source": OTHER_ORG_FLOW, "publish": true}),
            StatusCode::CREATED,
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let version_before = s
        .call(
            Method::GET,
            &format!("{root}/agents/{agent}"),
            &token,
            RequestBody::NoneExpected,
            None,
        )
        .await
        .json["version"]
        .as_u64()
        .unwrap();
    let released = s
        .call(
            Method::PUT,
            &format!("{root}/agents/{agent}/flows"),
            &token,
            RequestBody::Json(json!({"flow_ids": [flow]})),
            Some(version_before),
        )
        .await;
    assert_eq!(
        released.status,
        StatusCode::OK,
        "liberar fluxo: {}",
        released.text
    );
    let version_agent = released.json["version"].as_u64().unwrap();
    let service_case = s
        .call(
            Method::GET,
            &format!("{root}/fichas"),
            &token,
            RequestBody::NoneExpected,
            None,
        )
        .await
        .json["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let version_case = s
        .call(
            Method::GET,
            &format!("{root}/fichas/{service_case}"),
            &token,
            RequestBody::NoneExpected,
            None,
        )
        .await
        .json["version"]
        .as_u64()
        .unwrap();
    let opening = s
        .call(
            Method::POST,
            &format!("{root}/fichas/{service_case}/flow-runs"),
            &token,
            RequestBody::Json(json!({"fluxo_id": flow, "chave_idempotencia": "montagem-vitima"})),
            Some(version_case),
        )
        .await;
    assert_eq!(
        opening.status,
        StatusCode::CREATED,
        "abrir fluxo: {}",
        opening.text
    );
    let execution = opening.json["id"].as_str().unwrap().to_owned();

    let agent_site = s
        .create(
            &format!("{root}/agents"),
            &token,
            json!({"name": format!("Agente site {marker}"), "slug": format!("site-{slug}")}),
            StatusCode::CREATED,
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let site = s
        .store
        .site_chat()
        .create(org_id, agent_site.parse().unwrap(), format!("isola-{slug}"))
        .expect("conversa do site")
        .0
        .to_string();
    let site_turn = uuid::Uuid::new_v4().to_string();

    Org {
        id,
        token,
        token_id,
        agent,
        group,
        item,
        file,
        integration,
        category,
        department,
        product,
        conversation,
        flow,
        version_agent,
        service_case,
        execution,
        message: message.id.to_string(),
        attachment,
        site,
        site_turn,
        marker,
    }
}

#[derive(Clone, Copy)]
enum CaseKind {
    Empty,
    Json(&'static str),
    Multipart,
    File,
}

const OTHER_ORG_FLOW: &str = r#"fluxo(
    id = "agendar_visita",
    nome = "Agendar visita",
    descricao = "Marca uma visita técnica no endereço do cliente.",
    inicio = "coleta",
    gatilho = gatilho(
        quando = "O cliente quer marcar, remarcar ou cancelar uma visita técnica.",
        nao_quando = "Só quer saber preço ou prazo, sem pedir visita.",
        exemplos = ["queria agendar uma visita", "pode vir aqui amanhã?"],
    ),
)

etapa(
    id = "coleta",
    nome = "Coletar dados",
    instrucao = "Pergunte o endereço e o período preferido. Uma pergunta por vez.",
    campos = [
        campo("endereco", "Endereço completo", tipo = "texto", obrigatorio = True),
    ],
    acoes = [
        acao("confirmar", "Dados completos", exige = ["endereco"], vai_para = fim("concluida", resultado = "Visita agendada")),
    ],
)
"#;

#[derive(Clone)]
struct IsolationCase {
    method: &'static str,
    model: String,
    kind: CaseKind,
}

fn isolation_case(method: &'static str, model: &str, kind: CaseKind) -> IsolationCase {
    IsolationCase {
        method,
        model: model.to_owned(),
        kind,
    }
}

fn cases() -> Vec<IsolationCase> {
    use CaseKind::*;
    const ORG: &str = "/admin/v1/orgs/{org_id}";
    const AG: &str = "/admin/v1/orgs/{org_id}/agents/{agent_id}";
    const AGF: &str = "/admin/v1/orgs/{org_id}/agents/{agent_id}/flows";
    const KN: &str = "/admin/v1/orgs/{org_id}/knowledge/{group_id}";
    const IT: &str = "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}";
    const IG: &str = "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}";
    const WA: &str =
        "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp";
    const TC: &str = "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat";
    const CV: &str = "/admin/v1/orgs/{org_id}/conversations/{conversation_id}";
    const MX: &str = "/admin/v1/orgs/{org_id}/matrix";
    const MXESP: &str = "/admin/v1/orgs/{org_id}/matrix/spaces";
    const FL: &str = "/admin/v1/orgs/{org_id}/flows/{flow_id}";
    const FI: &str = "/admin/v1/orgs/{org_id}/fichas/{ficha_id}";
    const FR: &str = "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs/{run_id}";
    vec![
        isolation_case("get", "/admin/v1/orgs", Empty),
        isolation_case("post", "/admin/v1/orgs", Json(r#"{"name":"Invadido"}"#)),
        isolation_case("get", ORG, Empty),
        isolation_case("patch", ORG, Json(r#"{"name":"Invadido"}"#)),
        isolation_case("delete", ORG, Empty),
        isolation_case("get", "/admin/v1/orgs/{org_id}/agents", Empty),
        isolation_case(
            "post",
            "/admin/v1/orgs/{org_id}/agents",
            Json(r#"{"name":"Invadido","slug":"invadido"}"#),
        ),
        isolation_case("get", AG, Empty),
        isolation_case("patch", AG, Json(r#"{"description":"Invadido"}"#)),
        isolation_case("delete", AG, Empty),
        isolation_case("get", AGF, Empty),
        isolation_case("put", AGF, Json(r#"{"flow_ids":[]}"#)),
        isolation_case("get", &format!("{AG}/instructions"), Empty),
        isolation_case(
            "post",
            &format!("{AG}/instructions"),
            Json(r#"{"markdown":"Invadido"}"#),
        ),
        isolation_case("get", &format!("{AG}/instructions/current"), Empty),
        isolation_case(
            "patch",
            &format!("{AG}/instructions/current"),
            Json(r#"{"markdown":"Invadido"}"#),
        ),
        isolation_case("delete", &format!("{AG}/instructions/current"), Empty),
        isolation_case("get", "/admin/v1/orgs/{org_id}/knowledge", Empty),
        isolation_case(
            "post",
            "/admin/v1/orgs/{org_id}/knowledge",
            Json(r#"{"name":"Invadido"}"#),
        ),
        isolation_case("get", KN, Empty),
        isolation_case("patch", KN, Json(r#"{"name":"Invadido"}"#)),
        isolation_case("delete", KN, Empty),
        isolation_case("get", &format!("{KN}/items"), Empty),
        isolation_case(
            "post",
            &format!("{KN}/items"),
            Json(r#"{"title":"Invadido","content":"Invadido"}"#),
        ),
        isolation_case("get", IT, Empty),
        isolation_case("patch", IT, Json(r#"{"title":"Invadido"}"#)),
        isolation_case("delete", IT, Empty),
        isolation_case("post", &format!("{AG}/knowledge/{{group_id}}"), Empty),
        isolation_case("delete", &format!("{AG}/knowledge/{{group_id}}"), Empty),
        isolation_case("post", &format!("{AG}/knowledge/reindex"), Empty),
        isolation_case("get", &format!("{AG}/integrations"), Empty),
        isolation_case(
            "post",
            &format!("{AG}/integrations"),
            Json(r#"{"kind":"sms","status":"active"}"#),
        ),
        isolation_case("get", IG, Empty),
        isolation_case("patch", IG, Json(r#"{"status":"inactive"}"#)),
        isolation_case("delete", IG, Empty),
        isolation_case("post", &format!("{WA}/qrcode"), Empty),
        isolation_case("get", &format!("{WA}/state"), Empty),
        isolation_case("post", &format!("{WA}/disconnect"), Empty),
        isolation_case("get", "/admin/v1/orgs/{org_id}/backup", Empty),
        isolation_case(
            "post",
            "/admin/v1/orgs/{org_id}/backup/import",
            Json(
                r#"{"formato":"poph-backup-org","versao_formato":1,"org":{},"tabelas":{},"blobs":[]}"#,
            ),
        ),
        isolation_case("get", "/admin/v1/orgs/{org_id}/manifests", Empty),
        isolation_case("get", "/admin/v1/orgs/{org_id}/manifests/{kind}", Empty),
        isolation_case("get", "/admin/v1/orgs/{org_id}/tokens", Empty),
        isolation_case(
            "post",
            "/admin/v1/orgs/{org_id}/tokens",
            Json(r#"{"label":"Invadido"}"#),
        ),
        isolation_case("get", "/admin/v1/orgs/{org_id}/tokens/{token_id}", Empty),
        isolation_case("delete", "/admin/v1/orgs/{org_id}/tokens/{token_id}", Empty),
        isolation_case("get", "/admin/v1/orgs/{org_id}/categories", Empty),
        isolation_case(
            "post",
            "/admin/v1/orgs/{org_id}/categories",
            Json(r#"{"name":"Invadido"}"#),
        ),
        isolation_case(
            "get",
            "/admin/v1/orgs/{org_id}/categories/{category_id}",
            Empty,
        ),
        isolation_case(
            "patch",
            "/admin/v1/orgs/{org_id}/categories/{category_id}",
            Json(r#"{"name":"Invadido"}"#),
        ),
        isolation_case(
            "delete",
            "/admin/v1/orgs/{org_id}/categories/{category_id}",
            Empty,
        ),
        isolation_case("get", "/admin/v1/orgs/{org_id}/conversations", Empty),
        isolation_case("get", CV, Empty),
        isolation_case("get", &format!("{CV}/messages"), Empty),
        isolation_case("get", &format!("{CV}/turns"), Empty),
        isolation_case("post", &format!("{CV}/reopen"), Empty),
        isolation_case("post", &format!("{CV}/close"), Empty),
        isolation_case(
            "post",
            &format!("{CV}/transfer"),
            Json(r#"{"department_id":"018f0000-0000-7000-8000-000000000002","reason":"Invadido"}"#),
        ),
        isolation_case("get", "/admin/v1/orgs/{org_id}/departments", Empty),
        isolation_case(
            "post",
            "/admin/v1/orgs/{org_id}/departments",
            Json(r#"{"name":"Invadido"}"#),
        ),
        isolation_case(
            "get",
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
            Empty,
        ),
        isolation_case(
            "patch",
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
            Json(r#"{"name":"Invadido"}"#),
        ),
        isolation_case(
            "delete",
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
            Empty,
        ),
        isolation_case("get", &format!("{AG}/files"), Empty),
        isolation_case("post", &format!("{AG}/files"), File),
        isolation_case("get", &format!("{AG}/files/{{file_id}}"), Empty),
        isolation_case("get", &format!("{AG}/files/{{file_id}}/content"), Empty),
        isolation_case("delete", &format!("{AG}/files/{{file_id}}"), Empty),
        isolation_case("get", TC, Empty),
        isolation_case("post", TC, Empty),
        isolation_case("get", &format!("{TC}/{{conversation_id}}"), Empty),
        isolation_case(
            "post",
            &format!("{TC}/{{conversation_id}}/messages"),
            Multipart,
        ),
        isolation_case("post", &format!("{TC}/{{conversation_id}}/reopen"), Empty),
        isolation_case(
            "get",
            &format!(
                "{TC}/{{conversation_id}}/messages/{{message_id}}/attachments/{{attachment_id}}/content"
            ),
            Empty,
        ),
        isolation_case("get", "/admin/v1/orgs/{org_id}/products", Empty),
        isolation_case(
            "post",
            "/admin/v1/orgs/{org_id}/products",
            Json(r#"{"name":"Invadido","price_cents":100}"#),
        ),
        isolation_case(
            "get",
            "/admin/v1/orgs/{org_id}/products/{product_id}",
            Empty,
        ),
        isolation_case(
            "patch",
            "/admin/v1/orgs/{org_id}/products/{product_id}",
            Json(r#"{"name":"Invadido"}"#),
        ),
        isolation_case(
            "delete",
            "/admin/v1/orgs/{org_id}/products/{product_id}",
            Empty,
        ),
        isolation_case("get", MX, Empty),
        isolation_case("put", MX, Json(r#"{"enabled":true}"#)),
        isolation_case("get", MXESP, Empty),
        isolation_case(
            "put",
            MXESP,
            Json(r#"{"department_id":"00000000-0000-0000-0000-000000000000"}"#),
        ),
        isolation_case("delete", &format!("{MXESP}/{{department_id}}"), Empty),
        isolation_case(
            "post",
            "/admin/v1/orgs/{org_id}/flows/validate",
            Json(r#"{"source":"Invadido"}"#),
        ),
        isolation_case(
            "post",
            "/admin/v1/orgs/{org_id}/flows/format",
            Json(r#"{"source":"Invadido"}"#),
        ),
        isolation_case("get", "/admin/v1/orgs/{org_id}/flows", Empty),
        isolation_case(
            "post",
            "/admin/v1/orgs/{org_id}/flows",
            Json(r#"{"source":"Invadido"}"#),
        ),
        isolation_case("get", FL, Empty),
        isolation_case("patch", FL, Json(r#"{"name":"Invadido"}"#)),
        isolation_case("delete", FL, Empty),
        isolation_case("post", &format!("{FL}/publish"), Json(r#"{}"#)),
        isolation_case("get", &format!("{FL}/revisions"), Empty),
        isolation_case("get", &format!("{FL}/revisions/{{number}}"), Empty),
        isolation_case("get", &format!("{FL}/diff"), Empty),
        isolation_case("get", &format!("{FL}/preview"), Empty),
        isolation_case("get", "/admin/v1/orgs/{org_id}/fichas", Empty),
        isolation_case("get", FI, Empty),
        isolation_case("get", &format!("{FI}/events"), Empty),
        isolation_case(
            "post",
            &format!("{FI}/flow-runs"),
            Json(
                r#"{"fluxo_id":"018f0000-0000-7000-8000-000000000001","chave_idempotencia":"Invadido"}"#,
            ),
        ),
        isolation_case(
            "post",
            &format!("{FR}/actions"),
            Json(r#"{"acao":"Invadido","chave_idempotencia":"Invadido"}"#),
        ),
        isolation_case(
            "post",
            &format!("{FR}/abandon"),
            Json(r#"{"motivo":"Invadido","chave_idempotencia":"Invadido"}"#),
        ),
        isolation_case("post", &format!("{FI}/close"), Empty),
        isolation_case(
            "get",
            "/admin/v1/orgs/{org_id}/analytics/conversations",
            Empty,
        ),
        isolation_case("get", "/admin/v1/orgs/{org_id}/analytics/sla", Empty),
        isolation_case("get", "/admin/v1/orgs/{org_id}/usage/tokens", Empty),
        isolation_case(
            "get",
            "/admin/v1/orgs/{org_id}/analytics/operational-summary",
            Empty,
        ),
        isolation_case(
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/site-conversations",
            Json(r#"{"external_ref":"Invadido"}"#),
        ),
        isolation_case(
            "get",
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/messages",
            Empty,
        ),
        isolation_case(
            "post",
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/messages",
            Json(r#"{"text":"Invadido","idempotency_key":"invasao"}"#),
        ),
        isolation_case(
            "get",
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/turns/{turn_id}",
            Empty,
        ),
        isolation_case(
            "post",
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/turns/{turn_id}/cancel",
            Empty,
        ),
        isolation_case(
            "get",
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/events",
            Empty,
        ),
        isolation_case(
            "get",
            "/admin/v1/orgs/{org_id}/site-conversations/{id}/events/stream",
            Empty,
        ),
        isolation_case(
            "post",
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/adapter-conversations",
            Json(r#"{"channel":"email","external_ref":"Invadido"}"#),
        ),
        isolation_case(
            "get",
            "/admin/v1/orgs/{org_id}/adapter-conversations/{id}",
            Empty,
        ),
        isolation_case(
            "post",
            "/admin/v1/orgs/{org_id}/adapter-conversations/{id}/messages",
            Json(r#"{"text":"Invadido","origin":"customer","idempotency_key":"invasao"}"#),
        ),
        isolation_case(
            "get",
            "/admin/v1/orgs/{org_id}/adapter-conversations/{id}/messages",
            Empty,
        ),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Op {
    if_match: bool,
    force: bool,
}

fn published(doc: &Value) -> BTreeMap<(String, String), Op> {
    let mut map = BTreeMap::new();
    for (path, methods) in doc["paths"].as_object().expect("paths da OpenAPI") {
        for method in ["get", "post", "put", "patch", "delete"] {
            let Some(operation) = methods.get(method) else {
                continue;
            };
            let parameters = operation["parameters"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let has_match = |display_name: &str, at: &str| {
                parameters
                    .iter()
                    .any(|p| p["name"] == display_name && p["in"] == at)
            };
            map.insert(
                (method.to_owned(), path.clone()),
                Op {
                    if_match: has_match("If-Match", "header"),
                    force: has_match("force", "query"),
                },
            );
        }
    }
    map
}

fn coverage_gaps(
    published: &BTreeMap<(String, String), Op>,
    cases: &[IsolationCase],
) -> Vec<String> {
    let mut errors = Vec::new();
    let tenant: BTreeSet<(String, String)> = published
        .keys()
        .filter(|(_, path)| path.starts_with("/admin/v1/orgs"))
        .cloned()
        .collect();
    let non_tenant: BTreeSet<(String, String)> = published
        .keys()
        .filter(|(_, path)| !path.starts_with("/admin/v1/orgs"))
        .cloned()
        .collect();
    let expected_non_tenant: BTreeSet<(String, String)> = NON_TENANT
        .iter()
        .map(|(m, c)| ((*m).to_owned(), (*c).to_owned()))
        .collect();
    if non_tenant != expected_non_tenant {
        errors.push(format!(
            "operações fora de tenant mudaram: publicadas {non_tenant:?}, esperadas {expected_non_tenant:?}"
        ));
    }
    let with_case: BTreeSet<(String, String)> = cases
        .iter()
        .map(|c| (c.method.to_owned(), c.model.clone()))
        .collect();
    for op in tenant.difference(&with_case) {
        errors.push(format!(
            "operação publicada sem caso na suíte: {} {}",
            op.0, op.1
        ));
    }
    for op in with_case.difference(&tenant) {
        errors.push(format!(
            "caso sem operação publicada na OpenAPI: {} {}",
            op.0, op.1
        ));
    }
    errors
}

fn http_method(method: &str) -> Method {
    Method::from_bytes(method.to_uppercase().as_bytes()).expect("método HTTP")
}

fn case_path(isolation_case: &IsolationCase, target: &Org, org_id: &str) -> String {
    let path = target.path(&isolation_case.model, org_id);
    match isolation_case.kind {
        CaseKind::File => format!("{path}?filename=invadido.png"),
        _ => path,
    }
}

fn case_body(kind: CaseKind) -> RequestBody {
    match kind {
        CaseKind::Empty => RequestBody::NoneExpected,
        CaseKind::Json(text) => {
            RequestBody::Json(serde_json::from_str(text).expect("JSON do caso"))
        }
        CaseKind::Multipart => RequestBody::Multipart("Invadido".to_owned()),
        CaseKind::File => RequestBody::File(b"invadido".to_vec()),
    }
}

fn leak(r: &TestResponse, target: &Org) -> Option<String> {
    if r.text.contains(target.marker) {
        return Some(format!(
            "marcador `{}` da outra org aparece na resposta",
            target.marker
        ));
    }
    if r.status.is_success() {
        for id in target.ids() {
            if r.text.contains(&id) {
                return Some(format!(
                    "id `{id}` da outra org aparece numa resposta {}",
                    r.status
                ));
            }
        }
    }
    None
}

fn route_missing(r: &TestResponse) -> bool {
    r.status == StatusCode::METHOD_NOT_ALLOWED
        || (r.status == StatusCode::NOT_FOUND && r.json["details"]["path"].is_string())
}

fn expected_control(method: &str, model: &str) -> Option<u16> {
    if method != "get" {
        return None;
    }
    if model.ends_with("/whatsapp/state") {
        return Some(409);
    }
    if model.contains("/manifests/") {
        return Some(404);
    }
    if model.ends_with("/analytics/sla") {
        return Some(422);
    }
    if model.ends_with("/analytics/conversations") {
        return Some(422);
    }
    if model.ends_with("/usage/tokens") {
        return Some(422);
    }
    if model.ends_with("/analytics/operational-summary") {
        return Some(422);
    }
    if model.ends_with("/site-conversations/{id}/events/stream") {
        return None;
    }
    if model.ends_with("/site-conversations/{id}/turns/{turn_id}") {
        return None;
    }
    if model.contains("/adapter-conversations/{id}") {
        return Some(404);
    }
    Some(200)
}

struct Verdict {
    failures: Vec<String>,
    requests: usize,
}

fn with_query(path: String, query: &str) -> String {
    if query.is_empty() {
        path
    } else if path.contains('?') {
        format!("{path}&{query}")
    } else {
        format!("{path}?{query}")
    }
}

fn has_other_org_id(model: &str, other_org: &Org) -> bool {
    other_org.params().iter().any(|(display_name, _)| {
        *display_name != "kind" && model.contains(&format!("{{{display_name}}}"))
    })
}

async fn check_operation(
    s: &TestServer,
    isolation_case: &IsolationCase,
    op: Op,
    a: &Org,
    b: &Org,
) -> Verdict {
    let mut failures = Vec::new();
    let mut requests = 0usize;
    let method = http_method(isolation_case.method);
    let label = format!(
        "{} {}",
        isolation_case.method.to_uppercase(),
        isolation_case.model
    );
    let if_match = op.if_match.then_some(1u64);
    let variantes: Vec<&str> = if op.force && isolation_case.method == "delete" {
        vec!["", "force=true"]
    } else {
        vec![""]
    };

    if !isolation_case.model.contains("{org_id}") {
        if isolation_case.method == "get" {
            let r = s
                .call(
                    method.clone(),
                    &isolation_case.model,
                    &a.token,
                    RequestBody::NoneExpected,
                    None,
                )
                .await;
            requests += 1;
            if r.status != StatusCode::OK {
                failures.push(format!(
                    "{label} (token de A): esperado 200, veio {}",
                    r.status
                ));
            }
            let ids: Vec<String> = r.json["items"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|i| i["id"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            if ids != vec![a.id.clone()] {
                failures.push(format!(
                    "{label}: token de A deveria ver só a própria org, viu {ids:?}"
                ));
            }
            if let Some(reason) = leak(&r, b) {
                failures.push(format!("{label}: {reason}"));
            }
        } else {
            let r = s
                .call(
                    method.clone(),
                    &isolation_case.model,
                    &a.token,
                    case_body(isolation_case.kind),
                    None,
                )
                .await;
            requests += 1;
            if r.status != StatusCode::FORBIDDEN {
                failures.push(format!(
                    "{label} (token de org): esperado 403, veio {} {}",
                    r.status, r.text
                ));
            }
        }
        return Verdict { failures, requests };
    }

    for target in [b, a] {
        let path = case_path(isolation_case, target, &target.id);
        let r = s
            .call(
                method.clone(),
                &path,
                "",
                case_body(isolation_case.kind),
                None,
            )
            .await;
        requests += 1;
        if r.status != StatusCode::UNAUTHORIZED {
            failures.push(format!(
                "{label} sem token: esperado 401, veio {}",
                r.status
            ));
        }
    }

    for (atacante, other_org) in [(a, b), (b, a)] {
        for query in &variantes {
            let path = with_query(case_path(isolation_case, other_org, &other_org.id), query);
            let r = s
                .call(
                    method.clone(),
                    &path,
                    &atacante.token,
                    case_body(isolation_case.kind),
                    if_match,
                )
                .await;
            requests += 1;
            if r.status != StatusCode::FORBIDDEN {
                failures.push(format!(
                    "{label} [{query}] [atacante {}, vítima {}, org da vítima]: esperado 403, veio {} {}",
                    atacante.marker, other_org.marker, r.status, r.text
                ));
            }
            if let Some(reason) = leak(&r, other_org) {
                failures.push(format!("{label} [{query}] [org da vítima]: {reason}"));
            }

            if has_other_org_id(&isolation_case.model, other_org) {
                let path = with_query(case_path(isolation_case, other_org, &atacante.id), query);
                let r = s
                    .call(
                        method.clone(),
                        &path,
                        &atacante.token,
                        case_body(isolation_case.kind),
                        if_match,
                    )
                    .await;
                requests += 1;
                if !matches!(r.status.as_u16(), 403 | 404 | 409) {
                    failures.push(format!(
                        "{label} [{query}] [org própria, ids de {}]: esperado 403/404/409, veio {} {}",
                        other_org.marker, r.status, r.text
                    ));
                }
                if route_missing(&r) {
                    failures.push(format!(
                        "{label} [{query}]: o caminho montado não existe no roteador"
                    ));
                }
                if let Some(reason) = leak(&r, other_org) {
                    failures.push(format!(
                        "{label} [{query}] [org própria, ids da vítima]: {reason}"
                    ));
                }
            }
        }

        if isolation_case.method == "get" && !has_other_org_id(&isolation_case.model, other_org) {
            let path = case_path(isolation_case, atacante, &atacante.id);
            let r = s
                .call(
                    method.clone(),
                    &path,
                    &atacante.token,
                    RequestBody::NoneExpected,
                    None,
                )
                .await;
            requests += 1;
            if let Some(reason) = leak(&r, other_org) {
                failures.push(format!(
                    "{label} [lista própria de {}]: {reason}",
                    atacante.marker
                ));
            }
        }
    }

    if let Some(expected) = expected_control(isolation_case.method, &isolation_case.model) {
        let path = case_path(isolation_case, a, &a.id);
        let r = s
            .call(
                method.clone(),
                &path,
                &a.token,
                RequestBody::NoneExpected,
                None,
            )
            .await;
        requests += 1;
        if r.status.as_u16() != expected {
            failures.push(format!(
                "{label} [controle do dono]: esperado {expected}, veio {} {}",
                r.status, r.text
            ));
        }
        if route_missing(&r) {
            failures.push(format!(
                "{label} [controle]: caminho não existe no roteador"
            ));
        }
    }
    Verdict { failures, requests }
}

fn leak_without_echo(r: &TestResponse, target: &Org, eco: &str) -> Option<String> {
    let text = r.text.replace(eco, "");
    if text.contains(target.marker) {
        return Some(format!(
            "marcador `{}` da outra org aparece na resposta",
            target.marker
        ));
    }
    None
}

#[tokio::test]
async fn org_isolation_generated_from_openapi() {
    let s = TestServer::new("matriz", |store, config, _| {
        ServerState::new(Arc::clone(store), config.clone()).expect("estado do servidor")
    })
    .await;
    let doc = s
        .call(
            Method::GET,
            "/admin/v1/openapi.json",
            "",
            RequestBody::NoneExpected,
            None,
        )
        .await
        .json;
    let published = published(&doc);
    let cases = cases();
    let differences = coverage_gaps(&published, &cases);
    assert!(
        differences.is_empty(),
        "cobertura da OpenAPI incompleta:\n{}",
        differences.join("\n")
    );

    let a = seed_org(&s, "isola-a", MARKER_A).await;
    let b = seed_org(&s, "isola-b", MARKER_B).await;
    let before_a = s.snapshot(&a).await;
    let before_b = s.snapshot(&b).await;
    let audit_before_a = s.audit(&a);
    let audit_before_b = s.audit(&b);
    assert!(
        !audit_before_a.is_empty() && !audit_before_b.is_empty(),
        "fixture sem auditoria: a comparação de auditoria não provaria nada"
    );
    for (display_name, snapshots) in [("A", &before_a), ("B", &before_b)] {
        for line in snapshots.iter() {
            if !line.contains(" -> 200 ") {
                panic!("fixture da org {display_name} incompleta, o dono não lê: {line}");
            }
        }
    }

    let s_ref = &s;
    let published_ref = &published;
    let veredictos: Vec<Verdict> = futures::stream::iter(cases.iter())
        .map(|isolation_case| {
            let op = published_ref[&(
                isolation_case.method.to_owned(),
                isolation_case.model.clone(),
            )];
            check_operation(s_ref, isolation_case, op, &a, &b)
        })
        .buffer_unordered(8)
        .collect()
        .await;
    let mut failures: Vec<String> = Vec::new();
    let mut requests = 0usize;
    for v in veredictos {
        failures.extend(v.failures);
        requests += v.requests;
    }

    let cruzadas: Vec<(&str, String, Value, Option<u64>, String)> = vec![
        (
            "POST",
            format!("/admin/v1/orgs/{}/products", a.id),
            json!({"name": "Cruzado", "price_cents": 100, "category_id": b.category}),
            None,
            String::new(),
        ),
        (
            "POST",
            format!("/admin/v1/orgs/{}/agents", a.id),
            json!({"name": "Cruzado", "slug": "cruzado-categoria", "product_category_ids": [b.category]}),
            None,
            String::new(),
        ),
        (
            "POST",
            format!("/admin/v1/orgs/{}/agents", a.id),
            json!({"name": "Cruzado", "slug": "cruzado-departamento", "department": format!("Departamento {MARKER_B}")}),
            None,
            format!("Departamento {MARKER_B}"),
        ),
        (
            "PATCH",
            format!("/admin/v1/orgs/{}/agents/{}", a.id, a.agent),
            json!({"product_category_ids": [b.category]}),
            Some(a.version_agent),
            String::new(),
        ),
    ];
    for (method, path, body, if_match, eco) in cruzadas {
        let r = s
            .call(
                http_method(method),
                &path,
                &a.token,
                RequestBody::Json(body.clone()),
                if_match,
            )
            .await;
        requests += 1;
        if !matches!(r.status.as_u16(), 400 | 404 | 409 | 422) {
            failures.push(format!(
                "cruzamento {method} {path} com {body}: esperado erro de referência (400/404/409/422), veio {} {}",
                r.status, r.text
            ));
        }
        let vazou = if eco.is_empty() {
            leak(&r, &b)
        } else {
            leak_without_echo(&r, &b, &eco)
        };
        if let Some(reason) = vazou {
            failures.push(format!("cruzamento {method} {path}: {reason}"));
        }
    }

    let path_conversation_b = format!("/admin/v1/orgs/{}/conversations/{}", b.id, b.conversation);
    let conversation_before = s
        .call(
            Method::GET,
            &path_conversation_b,
            &b.token,
            RequestBody::NoneExpected,
            None,
        )
        .await
        .json;
    let transfer = s
        .call(
            Method::POST,
            &format!(
                "/admin/v1/orgs/{}/conversations/{}/transfer",
                a.id, b.conversation
            ),
            &a.token,
            RequestBody::Json(json!({"department_id": a.department, "reason": "atravessado"})),
            None,
        )
        .await;
    requests += 1;
    if transfer.status != StatusCode::NOT_FOUND {
        failures.push(format!(
            "transferência atravessada (conversa de {}, departamento de {}): esperado 404, veio {} {}",
            b.marker, a.marker, transfer.status, transfer.text
        ));
    }
    let conversation_after = s
        .call(
            Method::GET,
            &path_conversation_b,
            &b.token,
            RequestBody::NoneExpected,
            None,
        )
        .await
        .json;
    requests += 1;
    if conversation_before != conversation_after {
        failures.push(format!(
            "conversa de {} mudou com a transferência atravessada:\n  antes: {conversation_before}\n  depois: {conversation_after}",
            b.marker
        ));
    }

    let after_a = s.snapshot(&a).await;
    let after_b = s.snapshot(&b).await;
    for (before, after, display_name) in [(&before_a, &after_a, "A"), (&before_b, &after_b, "B")] {
        if before.len() != after.len() {
            failures.push(format!(
                "estado da org {display_name}: número de leituras mudou"
            ));
        }
        for (x, y) in before.iter().zip(after.iter()) {
            if x != y {
                failures.push(format!(
                    "estado da org {display_name} mudou após os ataques:\n  antes: {x}\n  depois: {y}"
                ));
            }
        }
    }
    for (before, after, display_name) in [
        (&audit_before_a, &s.audit(&a), "A"),
        (&audit_before_b, &s.audit(&b), "B"),
    ] {
        if before != after {
            failures.push(format!(
                "auditoria da org {display_name} mudou após os ataques: {} registro(s) antes, {} depois",
                before.len(),
                after.len()
            ));
        }
    }

    println!(
        "operações exercitadas: {}; pedidos de ataque, controle e cruzamento: {requests}",
        cases.len()
    );
    assert!(
        failures.is_empty(),
        "{} falha(s) de isolamento:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn missing_case_fails_coverage() {
    let doc = json!({
        "paths": {
            "/admin/v1/orgs/{org_id}/agents": {"get": {}, "post": {}},
            "/admin/v1/orgs/{org_id}/rota-nova": {
                "get": {"parameters": [{"name": "If-Match", "in": "header"}]}
            },
            "/healthz": {"get": {}},
            "/metrics": {"get": {}},
            "/admin/v1/cluster/reload": {"post": {}}
        }
    });
    let published = published(&doc);
    assert_eq!(
        published.get(&(
            "get".to_owned(),
            "/admin/v1/orgs/{org_id}/rota-nova".to_owned()
        )),
        Some(&Op {
            if_match: true,
            force: false
        }),
        "If-Match declarado na OpenAPI tem de ser lido"
    );
    let cases = vec![
        isolation_case("get", "/admin/v1/orgs/{org_id}/agents", CaseKind::Empty),
        isolation_case("get", "/admin/v1/orgs/{org_id}/fantasma", CaseKind::Empty),
    ];
    let errors = coverage_gaps(&published, &cases);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("sem caso") && e.contains("post /admin/v1/orgs/{org_id}/agents")),
        "operação publicada sem caso precisa reprovar: {errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("sem caso") && e.contains("rota-nova")),
        "rota nova sem caso precisa reprovar: {errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("caso sem operação") && e.contains("fantasma")),
        "caso sem operação precisa reprovar: {errors:?}"
    );
    assert_eq!(
        errors.len(),
        3,
        "exatamente as três divergências esperadas: {errors:?}"
    );

    let completos = vec![
        isolation_case("get", "/admin/v1/orgs/{org_id}/agents", CaseKind::Empty),
        isolation_case("post", "/admin/v1/orgs/{org_id}/agents", CaseKind::Empty),
        isolation_case("get", "/admin/v1/orgs/{org_id}/rota-nova", CaseKind::Empty),
    ];
    assert!(
        coverage_gaps(&published, &completos).is_empty(),
        "cobertura completa não pode dar divergência"
    );
}

struct DeferredProvider(std::sync::OnceLock<Arc<FakeProvider>>);

impl LlmProvider for DeferredProvider {
    fn stream<'a>(&'a self, request: Request) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            match self.0.get() {
                Some(fake) => fake.stream(request).await,
                None => Err(LlmError::Configuracao(
                    "provedor falso sem roteiro".to_owned(),
                )),
            }
        })
    }
}

async fn start_with_turn(
    label: &str,
) -> (
    TestServer,
    Arc<DeferredProvider>,
    Arc<KnowledgeBase>,
    Arc<ProductCatalog>,
) {
    let repassa = Arc::new(DeferredProvider(std::sync::OnceLock::new()));
    let provider: Arc<dyn LlmProvider> = Arc::clone(&repassa) as Arc<dyn LlmProvider>;
    let mut output: Option<(Arc<KnowledgeBase>, Arc<ProductCatalog>)> = None;
    let server = TestServer::new(label, |store, config, directory| {
        let blobs = directory.join("dados").join("blobs");
        let server_state = ServerState::new(Arc::clone(store), config.clone())
            .expect("estado do servidor")
            .with_files(FileConfig::default(blobs.clone()));
        let runtime_config = ConfigHot::new(
            Arc::clone(store),
            HotConfigOptions::default(),
            MetricsMemory::own(),
        );
        let limiter = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
        let engine = TurnEngine::new(
            Arc::clone(store),
            runtime_config,
            limiter,
            provider,
            TurnOptions::default(),
            MetricsTurn::own(),
        )
        .with_default_model("modelo-teste")
        .with_files(FileConfig::default(blobs));
        let index = Index::open(
            store.rag_path(),
            Arc::new(FakeEmbedder::new()),
            IndexConfig::default(),
        )
        .expect("índice de teste")
        .0;
        let index = Arc::new(index);
        let base = KnowledgeBase::new(Arc::clone(store), Arc::clone(&index));
        let catalog = ProductCatalog::new(Arc::clone(store), index);
        let search: Arc<dyn KnowledgeSearcher> = Arc::clone(&base) as Arc<dyn KnowledgeSearcher>;
        let record = QueryKnowledge::registro(Arc::clone(engine.hot_config()), Arc::clone(&search));
        let record = record.com(SearchProduct::new(
            Arc::clone(engine.hot_config()),
            Arc::clone(&catalog) as Arc<dyn ProductSearcher>,
        ));
        let record = poph_agent::cart::register(record, Arc::clone(store));
        let record = poph_agent::library::register(record, Arc::clone(store));
        let engine = Arc::new(engine.with_tools(Arc::new(record)).with_search(search));
        let processor = Arc::new(TurnProcessor::new(Arc::clone(&engine)));
        let actors = Actors::new(
            processor.clone(),
            ActorOptions::default(),
            MetricsMemory::own(),
        );
        let flow = Fluxo::new(Arc::clone(store), actors, FlowOptions::default());
        processor.attach_io(flow.clone());
        output = Some((base, catalog));
        server_state
            .with_engine(engine)
            .with_turn(flow, Duration::from_millis(20))
    })
    .await;
    let (base, catalog) = output.expect("base e catálogo montados");
    (server, repassa, base, catalog)
}

fn tool_call(id: &str, display_name: &str, arguments: Value) -> LlmEvent {
    LlmEvent::Tool(ToolCall {
        id: id.to_owned(),
        nome: display_name.to_owned(),
        arguments,
    })
}

fn results(requests: &[Request], prefix: &str) -> Vec<(String, String)> {
    let mut seen: Vec<(String, String)> = Vec::new();
    for request in requests {
        for message in &request.messages {
            if let Message::Resultado { id, content } = message
                && id.starts_with(prefix)
                && !seen.iter().any(|(visto, _)| visto == id)
            {
                seen.push((id.clone(), content.clone()));
            }
        }
    }
    seen
}

fn outbox_count(s: &TestServer, org: poph_types::OrgId) -> u64 {
    [
        OutboxState::Pending,
        OutboxState::Sending,
        OutboxState::Sent,
        OutboxState::Failed,
    ]
    .into_iter()
    .map(|server_state| {
        s.store
            .outbox()
            .count_by_state(org, server_state)
            .expect("contagem da caixa de saída")
    })
    .sum()
}

async fn send_message(s: &TestServer, org: &Org, text: &str) -> TestResponse {
    let path = format!(
        "/admin/v1/orgs/{}/agents/{}/test-chat/{}/messages",
        org.id, org.agent, org.conversation
    );
    s.call(
        Method::POST,
        &path,
        &org.token,
        RequestBody::Multipart(text.to_owned()),
        None,
    )
    .await
}

fn id<T: std::str::FromStr>(text: &str) -> T
where
    T::Err: std::fmt::Debug,
{
    text.parse().expect("id válido")
}

#[tokio::test]
async fn llm_tools_cannot_access_other_org() {
    let (s, repassa, base, catalog) = start_with_turn("ferramentas").await;
    let a = seed_org(&s, "ferr-a", MARKER_A).await;
    let b = seed_org(&s, "ferr-b", MARKER_B).await;
    let (oa, ob): (poph_types::OrgId, poph_types::OrgId) = (id(&a.id), id(&b.id));
    let (aa, ab): (poph_types::AgentId, poph_types::AgentId) = (id(&a.agent), id(&b.agent));
    base.synchronise_agent(oa, aa)
        .await
        .expect("indexa base de A");
    base.synchronise_agent(ob, ab)
        .await
        .expect("indexa base de B");
    catalog
        .synchronise_agent(oa, aa)
        .await
        .expect("indexa produtos de A");
    catalog
        .synchronise_agent(ob, ab)
        .await
        .expect("indexa produtos de B");

    let scripts = vec![
        Script::Events(vec![
            tool_call("a1", "enviar_arquivo", json!({"id": b.file})),
            LlmEvent::Fim(FinishReason::ToolCalls),
        ]),
        Script::Events(vec![
            tool_call(
                "a2",
                "cart_add_item",
                json!({"produto_id": b.product, "quantidade": 1}),
            ),
            LlmEvent::Fim(FinishReason::ToolCalls),
        ]),
        Script::Events(vec![
            tool_call("a3", "consultar_base", json!({"consulta": MARKER_B})),
            LlmEvent::Fim(FinishReason::ToolCalls),
        ]),
        Script::Events(vec![
            tool_call("a4", "buscar_produto", json!({"termo": MARKER_B})),
            LlmEvent::Fim(FinishReason::ToolCalls),
        ]),
        Script::Events(vec![
            LlmEvent::Texto("Não tenho nada para enviar.".to_owned()),
            LlmEvent::Fim(FinishReason::Completed),
        ]),
        Script::Events(vec![
            tool_call("b1", "enviar_arquivo", json!({"id": b.file})),
            LlmEvent::Fim(FinishReason::ToolCalls),
        ]),
        Script::Events(vec![
            tool_call("b2", "consultar_base", json!({"consulta": MARKER_B})),
            LlmEvent::Fim(FinishReason::ToolCalls),
        ]),
        Script::Events(vec![
            tool_call("b3", "buscar_produto", json!({"termo": MARKER_B})),
            LlmEvent::Fim(FinishReason::ToolCalls),
        ]),
        Script::Events(vec![
            LlmEvent::Texto("Pronto.".to_owned()),
            LlmEvent::Fim(FinishReason::Completed),
        ]),
    ];
    let fake = Arc::new(FakeProvider::with_scripts(scripts));
    assert!(repassa.0.set(Arc::clone(&fake)).is_ok());

    let mut failures: Vec<String> = Vec::new();
    let r_a = send_message(&s, &a, "Quero o arquivo, o produto e o que diz a base.").await;
    if r_a.status != StatusCode::OK {
        failures.push(format!(
            "turno de A: esperado 200 (SSE), veio {}: {}",
            r_a.status, r_a.text
        ));
    }
    let requests = fake.received();
    let res_a = results(&requests, "a");
    println!("resultados das ferramentas no turno de A (atacante):");
    for (id, text) in &res_a {
        println!("  {id}: {}", text.lines().next().unwrap_or(""));
    }
    for (id, text) in &res_a {
        if text.contains(MARKER_B) {
            failures.push(format!(
                "ferramenta {id} do turno de A devolveu dado da outra org"
            ));
        }
        if text.contains("enfileirado para envio") || text.contains("adicionado") {
            failures.push(format!(
                "ferramenta {id} do turno de A agiu com id da outra org: {text}"
            ));
        }
    }
    if res_a.len() != 4 {
        failures.push(format!(
            "turno de A: esperava 4 resultados de ferramenta, vieram {}",
            res_a.len()
        ));
    }
    let cart_a = s
        .store
        .carts()
        .summary(oa, id(&a.conversation))
        .expect("carrinho de A");
    if !cart_a.items.is_empty() {
        failures.push(format!(
            "o carrinho de A recebeu item do produto de B: {:?}",
            cart_a.items
        ));
    }
    let output_a_before = outbox_count(&s, oa);
    if output_a_before != 1 {
        failures.push(format!(
            "caixa de saída de A deveria ter só a resposta de texto (1 item), tem {output_a_before}"
        ));
    }

    let r_b = send_message(&s, &b, "Quero o arquivo, o produto e o que diz a base.").await;
    if r_b.status != StatusCode::OK {
        failures.push(format!(
            "turno de B (controle): esperado 200, veio {}",
            r_b.status
        ));
    }
    let requests = fake.received();
    let res_b = results(&requests, "b");
    println!("resultados das ferramentas no turno de B (controle):");
    for (id, text) in &res_b {
        println!("  {id}: {}", text.lines().next().unwrap_or(""));
    }
    let control = [
        (
            "enviar_arquivo acha o arquivo próprio",
            res_b
                .iter()
                .any(|(_, t)| t.contains("enfileirado para envio")),
        ),
        (
            "consultar_base acha o item próprio",
            res_b
                .iter()
                .any(|(_, t)| t.contains(&format!("Conteúdo sigiloso {MARKER_B}"))),
        ),
        (
            "buscar_produto acha o produto próprio",
            res_b
                .iter()
                .any(|(_, t)| t.contains(&format!("Produto {MARKER_B}"))),
        ),
    ];
    for (display_name, ok) in control {
        if !ok {
            failures.push(format!(
                "controle falhou ({display_name}): sem ele o teste do atacante não prova nada"
            ));
        }
    }
    if outbox_count(&s, ob) < 2 {
        failures.push("controle: o arquivo de B não entrou na caixa de saída de B".to_owned());
    }
    if outbox_count(&s, oa) != output_a_before {
        failures.push("a caixa de A mudou com o turno de B".to_owned());
    }

    assert!(
        failures.is_empty(),
        "{} falha(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[tokio::test]
async fn knowledge_search_excludes_other_org_chunks() {
    let (s, _repassa, base, catalog) = start_with_turn("rag").await;
    let a = seed_org(&s, "rag-a", MARKER_A).await;
    let b = seed_org(&s, "rag-b", MARKER_B).await;
    let (oa, ob): (poph_types::OrgId, poph_types::OrgId) = (id(&a.id), id(&b.id));
    let (aa, ab): (poph_types::AgentId, poph_types::AgentId) = (id(&a.agent), id(&b.agent));
    base.synchronise_agent(oa, aa).await.expect("indexa A");
    base.synchronise_agent(ob, ab).await.expect("indexa B");
    catalog.synchronise_agent(oa, aa).await.expect("produtos A");
    catalog.synchronise_agent(ob, ab).await.expect("produtos B");

    let params = SearchParameters {
        top_k: 10,
        cutoff: 0.0,
    };
    let mut failures = Vec::new();

    let own_b = base
        .search(ob, ab, MARKER_B.to_owned(), params)
        .await
        .expect("busca B");
    if !own_b.iter().any(|t| t.texto.contains(MARKER_B)) {
        failures.push(
            "controle: a busca de B não acha o próprio item (o teste ficaria vazio)".to_owned(),
        );
    }
    let own_a = base
        .search(oa, aa, MARKER_A.to_owned(), params)
        .await
        .expect("busca A");
    if !own_a.iter().any(|t| t.texto.contains(MARKER_A)) {
        failures.push("controle: a busca de A não acha o próprio item".to_owned());
    }

    let scenarios = [
        ("org de A, agente de A, marcador de B", oa, aa),
        ("org de A, agente de B", oa, ab),
        ("org de B, agente de A", ob, aa),
    ];
    for (display_name, org, agent) in scenarios {
        let chunks = base
            .search(org, agent, MARKER_B.to_owned(), params)
            .await
            .expect("busca");
        for t in &chunks {
            if t.texto.contains(MARKER_B) || t.title.contains(MARKER_B) {
                failures.push(format!("{display_name}: trecho de B voltou: {}", t.texto));
            }
        }
        if display_name == "org de A, agente de B" && !chunks.is_empty() {
            failures.push(format!(
                "{display_name}: agente de B não deveria achar nada sob a org de A"
            ));
        }
    }
    let products_a = ProductSearcher::search(
        catalog.as_ref(),
        oa,
        aa,
        MARKER_B.to_owned(),
        None,
        ProductParameters {
            top_k: 10,
            cutoff: 0.0,
        },
    )
    .await
    .expect("busca de produtos de A");
    if products_a.iter().any(|p| p.nome.contains(MARKER_B)) {
        failures.push("catálogo de A devolveu produto de B".to_owned());
    }
    let products_b = ProductSearcher::search(
        catalog.as_ref(),
        ob,
        ab,
        MARKER_B.to_owned(),
        None,
        ProductParameters {
            top_k: 10,
            cutoff: 0.0,
        },
    )
    .await
    .expect("busca de produtos de B");
    if !products_b.iter().any(|p| p.nome.contains(MARKER_B)) {
        failures.push("controle: o catálogo de B não acha o próprio produto".to_owned());
    }

    assert!(
        failures.is_empty(),
        "{} falha(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn leak_detector_checks_ids_only_in_success_responses() {
    let org = Org {
        id: "11111111-aaaa".to_owned(),
        token: String::new(),
        token_id: "22222222-bbbb".to_owned(),
        agent: "33333333-cccc".to_owned(),
        group: "44444444-dddd".to_owned(),
        item: "55555555-eeee".to_owned(),
        file: "66666666-ffff".to_owned(),
        integration: "77777777-0000".to_owned(),
        category: "88888888-1111".to_owned(),
        department: "99999999-2222".to_owned(),
        product: "aaaaaaaa-3333".to_owned(),
        conversation: "bbbbbbbb-4444".to_owned(),
        flow: "eeeeeeee-7777".to_owned(),
        version_agent: 1,
        service_case: "ffffffff-8888".to_owned(),
        execution: "00000000-9999".to_owned(),
        message: "cccccccc-5555".to_owned(),
        attachment: "dddddddd-6666".to_owned(),
        site: "eeeeeeee-9999".to_owned(),
        site_turn: "ffffffff-0000".to_owned(),
        marker: MARKER_B,
    };
    let response = |status: u16, text: &str| TestResponse {
        status: StatusCode::from_u16(status).unwrap(),
        text: text.to_owned(),
        json: Value::Null,
    };
    assert!(
        leak(
            &response(200, &format!("{{\"nome\":\"{MARKER_B}\"}}")),
            &org
        )
        .is_some(),
        "marcador em 200 é vazamento"
    );
    assert!(
        leak(&response(404, &format!("não existe: {MARKER_B}")), &org).is_some(),
        "marcador em 404 também é vazamento"
    );
    assert!(
        leak(&response(200, "{\"id\":\"33333333-cccc\"}"), &org).is_some(),
        "id da vítima em sucesso é vazamento"
    );
    assert!(
        leak(
            &response(404, "{\"message\":\"agente 33333333-cccc não existe\"}"),
            &org
        )
        .is_none(),
        "id ecoado num 404 não é dado"
    );
    assert!(
        leak(&response(403, "proibido"), &org).is_none(),
        "resposta limpa não é vazamento"
    );
}

#[tokio::test]
async fn snapshot_comparison_detects_write() {
    let s = TestServer::new("estado", |store, config, _| {
        ServerState::new(Arc::clone(store), config.clone()).expect("estado do servidor")
    })
    .await;
    let b = seed_org(&s, "estado-b", MARKER_B).await;
    let before = s.snapshot(&b).await;
    let audit_before = s.audit(&b);
    let r = s
        .call(
            Method::PATCH,
            &format!("/admin/v1/orgs/{}/agents/{}", b.id, b.agent),
            &b.token,
            RequestBody::Json(json!({"description": "mudança legítima do dono"})),
            Some(b.version_agent),
        )
        .await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "o dono pode alterar o próprio agente: {}",
        r.text
    );
    let after = s.snapshot(&b).await;
    assert_ne!(
        before, after,
        "a comparação de estado tem de acusar uma escrita do dono"
    );
    assert_ne!(
        audit_before,
        s.audit(&b),
        "a comparação de auditoria tem de acusar uma escrita do dono"
    );
}
