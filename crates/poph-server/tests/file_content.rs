use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_files::blob::sha256_hex;
use poph_server::ServerState;
use poph_store::{
    Direction, Mensagem, NewChannelAttachment, NewConversation, NewMessage, Role, Store,
};
use poph_types::config::GlobalTokenConfig;
use poph_types::{ConversationId, HarnessConfig, OrgId};
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-hr46b";

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89";

fn hash_argon2(value: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

struct Scenario {
    base: String,
    client: Client,
    folder: PathBuf,
    store: std::sync::Arc<Store>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Scenario {
    async fn new(label: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let folder = std::env::temp_dir().join(format!(
            "poph-conteudo-{label}-{}-{timestamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(folder.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = folder.join("dados");
        config.server.port = 7480;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = std::sync::Arc::new(Store::open(&config).expect("abre o banco"));
        let estado =
            ServerState::new(std::sync::Arc::clone(&store), config).expect("estado do servidor");

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

    fn store(&self) -> &Store {
        &self.store
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    async fn ask(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = self.client.request(method, self.url(path));
        if !token.is_empty() {
            request = request.bearer_auth(token);
        }
        if let Some(value) = body {
            request = request.json(&value);
        }
        let response = request.send().await.expect("pedido HTTP");
        let status = response.status();
        let texto = response.text().await.unwrap_or_default();
        let json = if texto.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&texto).unwrap_or(Value::String(texto))
        };
        (status, json)
    }

    async fn download(
        &self,
        path: &str,
        token: &str,
    ) -> (StatusCode, reqwest::header::HeaderMap, Vec<u8>) {
        let mut request = self.client.get(self.url(path));
        if !token.is_empty() {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.expect("pedido HTTP");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.bytes().await.expect("corpo").to_vec();
        (status, headers, bytes)
    }

    async fn create_org(&self, nome: &str, slug: &str) -> String {
        let (status, body) = self
            .ask(
                Method::POST,
                "/admin/v1/orgs",
                TOKEN_GLOBAL,
                Some(json!({ "name": nome, "slug": slug })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar org: {body}");
        body["id"].as_str().unwrap().to_owned()
    }

    async fn create_token(&self, org_id: &str, label: &str) -> String {
        let (status, body) = self
            .ask(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/tokens"),
                TOKEN_GLOBAL,
                Some(json!({ "label": label })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar token: {body}");
        body["value"].as_str().unwrap().to_owned()
    }

    async fn create_agent(&self, org: &str, token: &str, slug: &str) -> String {
        let (status, body) = self
            .ask(
                Method::POST,
                &format!("/admin/v1/orgs/{org}/agents"),
                token,
                Some(json!({ "name": format!("Agente {slug}"), "slug": slug })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar agente: {body}");
        body["id"].as_str().unwrap().to_owned()
    }

    async fn upload_file(&self, org: &str, agente: &str, token: &str, bytes: &[u8]) -> Value {
        let response = self
            .client
            .post(self.url(&format!(
                "/admin/v1/orgs/{org}/agents/{agente}/files?filename=procedimento.png&name=Procedimento%20t%C3%A9cnico.png"
            )))
            .bearer_auth(token)
            .header("content-type", "application/octet-stream")
            .body(bytes.to_vec())
            .send()
            .await
            .expect("upload HTTP");
        let status = response.status();
        let body: Value = response.json().await.expect("JSON do upload");
        assert_eq!(status, StatusCode::CREATED, "upload: {body}");
        body
    }

    fn seed_attachment(&self, org: &str, agente: &str, sha256: &str) -> (ConversationId, Mensagem) {
        let org_id: OrgId = org.parse().expect("org");
        let conversa = self
            .store()
            .conversations()
            .create(
                org_id,
                NewConversation {
                    agent_id: agente.parse().expect("agente"),
                    integration_id: None,
                    canal: "test".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("conversa");
        let mensagem = self
            .store()
            .messages()
            .write_with_attachments(
                org_id,
                NewMessage {
                    conversation_id: conversa.id,
                    direction: Direction::Inbound,
                    role: Role::User,
                    texto: "segue o arquivo".to_owned(),
                    attachments_json: "[]".to_owned(),
                    external_id: None,
                    turn_id: None,
                    origin: poph_store::Origin::Agente,
                    author_name: None,
                },
                &[NewChannelAttachment {
                    name: "Procedimento técnico.png".to_owned(),
                    mime: "image/png".to_owned(),
                    size_bytes: PNG.len() as u64,
                    sha256: sha256.to_owned(),
                    blob_path: format!("{org}/{sha256}"),
                    extracted_text: None,
                    extraction_status: "skipped",
                }],
            )
            .expect("mensagem com anexo");
        (conversa.id, mensagem)
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

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or("")
}

fn unknown_id() -> String {
    poph_types::FileId::generate().to_string()
}

#[tokio::test]
async fn library_content_serves_bytes_with_headers() {
    let c = Scenario::new("biblioteca").await;
    let org = c.create_org("Org A", "org-a").await;
    let token = c.create_token(&org, "org a").await;
    let agente = c.create_agent(&org, &token, "agente-a").await;
    let file = c.upload_file(&org, &agente, &token, PNG).await;
    let id = file["id"].as_str().unwrap().to_owned();
    let sha = sha256_hex(PNG);
    assert_eq!(file["sha256"], sha, "hash do upload");

    let route = format!("/admin/v1/orgs/{org}/agents/{agente}/files/{id}/content");
    let (status, headers, bytes) = c.download(&route, &token).await;
    assert_eq!(status, StatusCode::OK, "conteúdo do arquivo");
    assert_eq!(bytes, PNG, "bytes idênticos aos enviados");
    assert_eq!(
        sha256_hex(&bytes),
        sha,
        "SHA-256 conferido dos dois lados (resposta e envio)"
    );
    let header = |nome: &str| {
        headers
            .get(nome)
            .unwrap_or_else(|| panic!("cabeçalho `{nome}` ausente"))
            .to_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(header("content-type"), "image/png", "MIME gravado");
    assert_eq!(header("x-content-sha256"), sha, "sha256 no cabeçalho");
    assert_eq!(header("etag"), format!("\"{sha}\""), "ETag");
    assert_eq!(header("content-length"), PNG.len().to_string(), "tamanho");
    let disposition = header("content-disposition");
    assert!(
        disposition.starts_with("inline"),
        "imagem abre no lugar: {disposition}"
    );
    assert!(
        disposition.contains("filename=\"Procedimento t_cnico.png\""),
        "nome ASCII sanitizado: {disposition}"
    );
    assert!(
        disposition.contains("filename*=UTF-8''Procedimento%20t%C3%A9cnico.png"),
        "nome com acento em UTF-8: {disposition}"
    );
    assert_eq!(header("x-content-type-options"), "nosniff");
    assert!(header("content-security-policy").contains("sandbox"));

    let (status, meta) = c
        .ask(
            Method::GET,
            &format!("/admin/v1/orgs/{org}/agents/{agente}/files/{id}"),
            &token,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        meta.get("blob_path").is_none(),
        "metadados sem blob_path: {meta}"
    );
}

#[tokio::test]
async fn library_content_isolates_org_agent_and_id() {
    let c = Scenario::new("biblioteca-isola").await;
    let org_a = c.create_org("Org A", "org-a").await;
    let org_b = c.create_org("Org B", "org-b").await;
    let token_a = c.create_token(&org_a, "org a").await;
    let token_b = c.create_token(&org_b, "org b").await;
    let agent_a = c.create_agent(&org_a, &token_a, "agente-a").await;
    let agent_a2 = c.create_agent(&org_a, &token_a, "agente-a2").await;
    let file = c.upload_file(&org_a, &agent_a, &token_a, PNG).await;
    let id = file["id"].as_str().unwrap().to_owned();
    let route_a = format!("/admin/v1/orgs/{org_a}/agents/{agent_a}/files/{id}/content");

    let (status, _, body) = c.download(&route_a, "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        code(&serde_json::from_slice(&body).unwrap()),
        "unauthorized"
    );
    let (status, _, body) = c.download(&route_a, &token_b).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "token de outra org");
    assert_eq!(code(&serde_json::from_slice(&body).unwrap()), "forbidden");

    let (status, _, body) = c
        .download(
            &format!("/admin/v1/orgs/{org_b}/agents/{agent_a}/files/{id}/content"),
            &token_b,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "agente de outra org");
    let texto = String::from_utf8_lossy(&body);
    assert_eq!(
        code(&serde_json::from_slice(&body).unwrap()),
        "not_found",
        "corpo de erro"
    );
    assert!(
        !texto.contains("blobs") && !texto.contains("dados/"),
        "resposta sem caminho interno: {texto}"
    );

    let (status, _, _) = c
        .download(
            &format!(
                "/admin/v1/orgs/{org_a}/agents/{agent_a}/files/{}/content",
                unknown_id()
            ),
            &token_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "id inexistente");
    let (status, _, _) = c
        .download(
            &format!("/admin/v1/orgs/{org_a}/agents/{agent_a2}/files/{id}/content"),
            &token_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "arquivo de outro agente");

    let no_blob = c
        .store()
        .library()
        .create(
            org_a.parse().unwrap(),
            poph_store::NewLibraryFile {
                agent_id: agent_a.parse().unwrap(),
                name: "Sumiu".to_owned(),
                description: String::new(),
                mime: "application/pdf".to_owned(),
                size_bytes: 10,
                sha256: "b".repeat(64),
                blob_path: format!("{org_a}/{}", "b".repeat(64)),
            },
        )
        .expect("arquivo sem blob");
    let (status, _, body) = c
        .download(
            &format!(
                "/admin/v1/orgs/{org_a}/agents/{agent_a}/files/{}/content",
                no_blob.id
            ),
            &token_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "blob ausente");
    let texto = String::from_utf8_lossy(&body);
    assert_eq!(code(&serde_json::from_slice(&body).unwrap()), "not_found");
    assert!(
        !texto.contains("blobs") && !texto.contains(&"b".repeat(64)),
        "mensagem sem caminho nem hash: {texto}"
    );
}

#[tokio::test]
async fn attachment_content_serves_bytes_from_the_test_chat() {
    let c = Scenario::new("anexo").await;
    let org_a = c.create_org("Org A", "org-a").await;
    let org_b = c.create_org("Org B", "org-b").await;
    let token_a = c.create_token(&org_a, "org a").await;
    let token_b = c.create_token(&org_b, "org b").await;
    let agent_a = c.create_agent(&org_a, &token_a, "agente-a").await;
    let file = c.upload_file(&org_a, &agent_a, &token_a, PNG).await;
    let sha = file["sha256"].as_str().unwrap().to_owned();

    let (conversa, mensagem) = c.seed_attachment(&org_a, &agent_a, &sha);
    let anexos = c
        .store()
        .messages()
        .attachments(org_a.parse().unwrap(), mensagem.id)
        .expect("anexos da mensagem");
    assert_eq!(anexos.len(), 1);
    let attachment = anexos[0].id;
    let route = format!(
        "/admin/v1/orgs/{org_a}/agents/{agent_a}/test-chat/{conversa}/messages/{}/attachments/{attachment}/content",
        mensagem.id
    );

    let (status, headers, bytes) = c.download(&route, &token_a).await;
    assert_eq!(status, StatusCode::OK, "conteúdo do anexo");
    assert_eq!(bytes, PNG, "bytes idênticos ao blob");
    assert_eq!(sha256_hex(&bytes), sha, "sha256 conferido");
    let header = |nome: &str| {
        headers
            .get(nome)
            .unwrap_or_else(|| panic!("cabeçalho `{nome}` ausente"))
            .to_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(header("content-type"), "image/png");
    assert_eq!(header("x-content-sha256"), sha);
    assert!(
        header("content-disposition").contains("filename*=UTF-8''Procedimento%20t%C3%A9cnico.png"),
        "nome com acento: {}",
        header("content-disposition")
    );

    let (status, _, _) = c.download(&route, "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = c.download(&route, &token_b).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let other_conversation = c
        .store()
        .conversations()
        .create(
            org_a.parse().unwrap(),
            NewConversation {
                agent_id: agent_a.parse().unwrap(),
                integration_id: None,
                canal: "test".to_owned(),
                peer_externo: None,
                target_department: None,
            },
        )
        .expect("outra conversa");
    let (status, _, _) = c
        .download(
            &format!(
                "/admin/v1/orgs/{org_a}/agents/{agent_a}/test-chat/{}/messages/{}/attachments/{attachment}/content",
                other_conversation.id, mensagem.id
            ),
            &token_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "mensagem de outra conversa");
    let (status, _, _) = c
        .download(
            &format!(
                "/admin/v1/orgs/{org_a}/agents/{agent_a}/test-chat/{conversa}/messages/{}/attachments/{}/content",
                mensagem.id,
                poph_types::AttachmentId::generate()
            ),
            &token_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "anexo inexistente");
    let (status, _, _) = c
        .download(
            &format!(
                "/admin/v1/orgs/{org_a}/agents/{agent_a}/test-chat/{conversa}/messages/{}/attachments/{attachment}/content",
                poph_types::MessageId::generate()
            ),
            &token_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "mensagem inexistente");

    let (conversation2, message2) = c.seed_attachment(&org_a, &agent_a, &"c".repeat(64));
    let attachment2 = c
        .store()
        .messages()
        .attachments(org_a.parse().unwrap(), message2.id)
        .expect("anexos")
        .remove(0)
        .id;
    let (status, _, body) = c
        .download(
            &format!(
                "/admin/v1/orgs/{org_a}/agents/{agent_a}/test-chat/{conversation2}/messages/{}/attachments/{attachment2}/content",
                message2.id
            ),
            &token_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "blob ausente");
    let texto = String::from_utf8_lossy(&body);
    assert!(
        !texto.contains("blobs") && !texto.contains(&"c".repeat(64)),
        "mensagem sem caminho nem hash: {texto}"
    );
}
