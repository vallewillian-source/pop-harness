use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::{CipherKey, ServerState};
use poph_store::Store;
use poph_types::HarnessConfig;
use poph_types::config::GlobalTokenConfig;
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-hr48e1";
const CIPHER_KEY: &str = "chave-de-teste-hr48e1";
const SENTINEL_SECRET: &str = "SENTINELA-SEGREDO-hr48e1";

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
    store: Arc<Store>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Scenario {
    async fn new(label: &str) -> Self {
        Self::with_model(label, false).await
    }

    async fn with_model(label: &str, with_model: bool) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let folder = std::env::temp_dir().join(format!(
            "poph-backup-http-{label}-{}-{timestamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(folder.join("dados")).expect("pasta de dados");
        if with_model && model_installed() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(repo_models_folder(), folder.join("dados").join("modelos"))
                .expect("liga a pasta de modelos");
        }

        let mut config = HarnessConfig::default();
        config.data.directory = folder.join("dados");
        config.server.port = 7480;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let chave = CipherKey::derive(CIPHER_KEY).expect("chave de cifra");
        let estado = ServerState::new(Arc::clone(&store), config)
            .expect("estado do servidor")
            .with_encryption_key(Some(chave));

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

        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("cliente HTTP");
        let scenario = Self {
            base: format!("http://{endereco}"),
            client,
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
                _ if start.elapsed() < Duration::from_secs(20) => {
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

    async fn send(
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

    async fn download_backup(&self, org: &str, token: &str, query: &str) -> (StatusCode, Backups) {
        let response = self
            .client
            .get(self.url(&format!("/admin/v1/orgs/{org}/backup{query}")))
            .bearer_auth(token)
            .send()
            .await
            .expect("pedido HTTP");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.bytes().await.expect("corpo").to_vec();
        (status, Backups { headers, bytes })
    }

    async fn import(&self, org: &str, token: &str, bytes: &[u8]) -> (StatusCode, Value) {
        self.import_with(org, token, bytes, "").await
    }

    async fn import_with(
        &self,
        org: &str,
        token: &str,
        bytes: &[u8],
        query: &str,
    ) -> (StatusCode, Value) {
        let response = self
            .client
            .post(self.url(&format!("/admin/v1/orgs/{org}/backup/import{query}")))
            .bearer_auth(token)
            .header("content-type", "application/json")
            .body(bytes.to_vec())
            .send()
            .await
            .expect("pedido HTTP");
        let status = response.status();
        let texto = response.text().await.unwrap_or_default();
        let json = if texto.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&texto).unwrap_or(Value::String(texto))
        };
        (status, json)
    }

    async fn create_org(&self, nome: &str, slug: &str) -> Value {
        let (status, body) = self
            .send(
                Method::POST,
                "/admin/v1/orgs",
                TOKEN_GLOBAL,
                Some(json!({ "name": nome, "slug": slug })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar org: {body}");
        body
    }

    async fn create_token(&self, org_id: &str, label: &str) -> String {
        let (status, body) = self
            .send(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/tokens"),
                TOKEN_GLOBAL,
                Some(json!({ "label": label })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar token: {body}");
        body["value"].as_str().expect("valor do token").to_owned()
    }

    async fn criar(&self, path: &str, token: &str, body: Value) -> Value {
        let (status, json) = self.send(Method::POST, path, token, Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{path}: {json}");
        json
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

struct Backups {
    headers: reqwest::header::HeaderMap,
    bytes: Vec<u8>,
}

impl Backups {
    fn texto(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).expect("arquivo de backup é JSON")
    }

    fn header(&self, nome: &str) -> String {
        self.headers
            .get(nome)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned()
    }
}

fn repo_models_folder() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/modelos"))
}

fn model_installed() -> bool {
    let models = repo_models_folder();
    models.join("model_int8.onnx").is_file() && models.join("tokenizer.json").is_file()
}

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or("")
}

async fn build_org(c: &Scenario, slug: &str) -> (String, String, Value) {
    let org = c.create_org(&format!("Org {slug}"), slug).await;
    let id = org["id"].as_str().unwrap().to_owned();
    let token = c.create_token(&id, slug).await;
    let root = format!("/admin/v1/orgs/{id}");
    let agente = c
        .criar(
            &format!("{root}/agents"),
            &token,
            json!({"name": "Bia", "slug": format!("bia-{slug}")}),
        )
        .await;
    let agent_id = agente["id"].as_str().unwrap().to_owned();
    c.criar(
        &format!("{root}/agents/{agent_id}/instructions"),
        &token,
        json!({"markdown": "Atenda com simpatia."}),
    )
    .await;
    let category = c
        .criar(
            &format!("{root}/categories"),
            &token,
            json!({"name": format!("Categoria {slug}")}),
        )
        .await;
    c.criar(
        &format!("{root}/departments"),
        &token,
        json!({"name": format!("Departamento {slug}")}),
    )
    .await;
    c.criar(
        &format!("{root}/products"),
        &token,
        json!({"name": format!("Produto {slug}"), "price_cents": 1234, "category_id": category["id"]}),
    )
    .await;
    let group = c
        .criar(
            &format!("{root}/knowledge"),
            &token,
            json!({"name": format!("Grupo {slug}")}),
        )
        .await;
    c.criar(
        &format!("{root}/knowledge/{}/items", group["id"].as_str().unwrap()),
        &token,
        json!({"title": "Prazo de entrega", "content": "O prazo é de 5 dias úteis."}),
    )
    .await;
    let (status, link) = c
        .send(
            Method::POST,
            &format!(
                "{root}/agents/{agent_id}/knowledge/{}",
                group["id"].as_str().unwrap()
            ),
            &token,
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "vínculo do grupo: {link}");
    let cifrado = CipherKey::derive(CIPHER_KEY)
        .expect("chave")
        .cifrar(&std::collections::BTreeMap::from([(
            "token".to_owned(),
            SENTINEL_SECRET.to_owned(),
        )]))
        .expect("cifra");
    c.store()
        .create_integration_with_config(
            id.parse().expect("org id"),
            agent_id.parse().expect("agent id"),
            "eco".to_owned(),
            "active".to_owned(),
            json!({"instancia": "bia"}),
            Some(cifrado),
        )
        .expect("integração com segredo");
    let response = c
        .client
        .post(c.url(&format!(
            "{root}/agents/{agent_id}/files?filename=catalogo.png"
        )))
        .bearer_auth(&token)
        .header("content-type", "application/octet-stream")
        .body(PNG.to_vec())
        .send()
        .await
        .expect("upload");
    assert_eq!(response.status(), StatusCode::CREATED, "upload do arquivo");
    let file: Value = response.json().await.expect("json do arquivo");
    (id, token, file)
}

#[tokio::test]
async fn export_returns_org_file_without_secret_or_path() {
    let c = Scenario::new("exporta").await;
    let (org, token, file) = build_org(&c, "origem").await;

    let (status, downloaded_file) = c.download_backup(&org, &token, "").await;
    assert_eq!(status, StatusCode::OK, "{}", downloaded_file.texto());
    assert_eq!(downloaded_file.header("content-type"), "application/json");
    assert_eq!(downloaded_file.header("x-content-type-options"), "nosniff");
    let disposition = downloaded_file.header("content-disposition");
    assert!(
        disposition.starts_with("attachment; filename=\"origem-backup-"),
        "disposição: {disposition}"
    );
    assert!(
        disposition.ends_with(".json\""),
        "disposição: {disposition}"
    );

    let document = downloaded_file.json();
    assert_eq!(document["formato"], "poph-backup-org");
    assert_eq!(document["versao_formato"], 2);
    assert_eq!(document["org_origem"]["id"], org.as_str());
    assert_eq!(document["org_origem"]["slug"], "origem");
    assert_eq!(document["com_conversas"], false);
    for table in [
        "conversations",
        "messages",
        "turns",
        "outbox",
        "inbox",
        "attachments",
        "carts",
        "cart_items",
    ] {
        assert!(
            document["tabelas"].get(table).is_none(),
            "tabela `{table}` não deveria estar no arquivo sem com_conversas"
        );
    }
    assert_eq!(document["tabelas"]["agents"].as_array().unwrap().len(), 1);
    assert_eq!(document["tabelas"]["products"].as_array().unwrap().len(), 1);
    let integracao = &document["tabelas"]["integrations"][0];
    assert_eq!(integracao["tem_segredo"], true);
    assert_eq!(integracao["segredos_campos"], json!(["token"]));
    assert_eq!(
        document["tabelas"]["file_library"][0]["sha256"],
        file["sha256"]
    );
    assert!(!document["blobs"].as_array().unwrap().is_empty());

    let raw = downloaded_file.texto();
    for forbidden in [
        SENTINEL_SECRET,
        "secrets_enc",
        "blob_path",
        "blobs/",
        CIPHER_KEY,
    ] {
        assert!(
            !raw.contains(forbidden),
            "o arquivo exportado não pode conter `{forbidden}`"
        );
    }

    let conversa = c
        .store()
        .conversations()
        .create(
            org.parse().expect("org id"),
            poph_store::NewConversation {
                agent_id: document["tabelas"]["agents"][0]["id"]
                    .as_str()
                    .unwrap()
                    .parse()
                    .expect("agent id"),
                integration_id: None,
                canal: "cli".into(),
                peer_externo: None,
                target_department: None,
            },
        )
        .expect("conversa");
    c.store()
        .messages()
        .write(
            org.parse().expect("org id"),
            poph_store::NewMessage {
                conversation_id: conversa.id,
                direction: poph_store::Direction::Inbound,
                role: poph_store::Role::User,
                texto: "tudo bem?".to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
        )
        .expect("mensagem");
    let (status, com_conversas) = c.download_backup(&org, &token, "?com_conversas=true").await;
    assert_eq!(status, StatusCode::OK);
    let document = com_conversas.json();
    assert_eq!(document["com_conversas"], true);
    assert_eq!(
        document["tabelas"]["conversations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(document["tabelas"]["messages"].as_array().unwrap().len(), 1);

    let (status, error) = c
        .send(
            Method::GET,
            &format!("/admin/v1/orgs/{org}/backup?com_conversas=talvez"),
            &token,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(code(&error), "validation_error", "{error}");
}

async fn export_source(label: &str) -> (Vec<u8>, String) {
    let source = Scenario::new(&format!("origem-{label}")).await;
    let (org, token, _) = build_org(&source, "origem").await;
    let (status, file) = source.download_backup(&org, &token, "").await;
    assert_eq!(status, StatusCode::OK, "{}", file.texto());
    let sha = file.json()["tabelas"]["file_library"][0]["sha256"]
        .as_str()
        .unwrap()
        .to_owned();
    (file.bytes, sha)
}

#[tokio::test]
async fn import_rebuilds_org_keeps_blob_and_does_not_duplicate() {
    let (bytes, sha) = export_source("importa").await;
    let c = Scenario::new("destino-importa").await;

    let target = c.create_org("Destino", "destino").await;
    let target_id = target["id"].as_str().unwrap().to_owned();
    let target_token = c.create_token(&target_id, "destino").await;
    let neighbor = c.create_org("Vizinha", "vizinha").await;
    let neighbor_id = neighbor["id"].as_str().unwrap().to_owned();
    let neighbor_token = c.create_token(&neighbor_id, "vizinha").await;
    let (_, before_neighbor) = c
        .send(
            Method::GET,
            &format!("/admin/v1/orgs/{neighbor_id}/agents"),
            &neighbor_token,
            None,
        )
        .await;

    let (status, report) = c.import(&target_id, &target_token, &bytes).await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(report["org"]["id"], target_id.as_str());
    assert_eq!(report["org"]["criada"], false);
    assert_eq!(report["com_conversas"], false);
    assert!(
        report["total"].as_u64().unwrap_or(0) > 0,
        "relatório sem total: {report}"
    );
    assert_eq!(report["segredos_a_redigitar"], 1);
    if !model_installed() {
        assert!(
            report["aviso"]
                .as_str()
                .is_some_and(|a| a.contains("poph kb reindex")),
            "faltou o aviso de reindexação: {report}"
        );
        assert_eq!(report["indice"]["base_conhecimento"]["trechos"], 0);
    }

    let (_, agents) = c
        .send(
            Method::GET,
            &format!("/admin/v1/orgs/{target_id}/agents"),
            &target_token,
            None,
        )
        .await;
    assert_eq!(agents["items"].as_array().unwrap().len(), 1, "{agents}");
    let agente = agents["items"][0]["id"].as_str().unwrap().to_owned();
    let (_, products) = c
        .send(
            Method::GET,
            &format!("/admin/v1/orgs/{target_id}/products"),
            &target_token,
            None,
        )
        .await;
    assert_eq!(products["items"].as_array().unwrap().len(), 1);
    let (_, files) = c
        .send(
            Method::GET,
            &format!("/admin/v1/orgs/{target_id}/agents/{agente}/files"),
            &target_token,
            None,
        )
        .await;
    assert_eq!(files["items"].as_array().unwrap().len(), 1);
    assert_eq!(files["items"][0]["sha256"], sha.as_str());
    let response = c
        .client
        .get(c.url(&format!(
            "/admin/v1/orgs/{target_id}/agents/{agente}/files/{}/content",
            files["items"][0]["id"].as_str().unwrap()
        )))
        .bearer_auth(&target_token)
        .send()
        .await
        .expect("download");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-content-sha256")
            .and_then(|v| v.to_str().ok()),
        Some(sha.as_str())
    );
    assert_eq!(
        response.bytes().await.expect("bytes").to_vec(),
        PNG.to_vec()
    );

    let (_, integrations) = c
        .send(
            Method::GET,
            &format!("/admin/v1/orgs/{target_id}/agents/{agente}/integrations"),
            &target_token,
            None,
        )
        .await;
    let integracao = &integrations["items"][0];
    assert_eq!(
        integracao["segredos_preenchidos"],
        json!([]),
        "{integracao}"
    );
    assert_eq!(integracao["configuracao"]["instancia"], "bia");

    let (status, refusal) = c.import(&target_id, &target_token, &bytes).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refusal}");
    assert_eq!(code(&refusal), "importacao_sobrescreve");
    assert!(
        refusal["details"]["linhas"].as_u64().unwrap_or(0) > 0,
        "{refusal}"
    );
    assert!(
        !refusal["details"]["exemplos"]
            .as_array()
            .expect("exemplos")
            .is_empty(),
        "{refusal}"
    );
    let (_, agents_after_refusal) = c
        .send(
            Method::GET,
            &format!("/admin/v1/orgs/{target_id}/agents"),
            &target_token,
            None,
        )
        .await;
    assert_eq!(agents_after_refusal["items"].as_array().unwrap().len(), 1);

    let (status, second) = c
        .import_with(&target_id, &target_token, &bytes, "?sobrescrever=true")
        .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    let inserted: u64 = second["tabelas"]
        .as_object()
        .unwrap()
        .values()
        .map(|t| t["inseridas"].as_u64().unwrap_or(0))
        .sum();
    assert_eq!(inserted, 0, "a segunda importação inseriu linha: {second}");
    let (_, agents_after) = c
        .send(
            Method::GET,
            &format!("/admin/v1/orgs/{target_id}/agents"),
            &target_token,
            None,
        )
        .await;
    assert_eq!(agents_after["items"].as_array().unwrap().len(), 1);

    let (_, after_neighbor) = c
        .send(
            Method::GET,
            &format!("/admin/v1/orgs/{neighbor_id}/agents"),
            &neighbor_token,
            None,
        )
        .await;
    assert_eq!(after_neighbor, before_neighbor);
}

#[tokio::test]
async fn slug_conflict_is_409_and_writes_nothing() {
    let (bytes, _) = export_source("conflito").await;
    let c = Scenario::new("destino-conflito").await;

    let target = c.create_org("Destino", "destino").await;
    let target_id = target["id"].as_str().unwrap().to_owned();
    let target_token = c.create_token(&target_id, "destino").await;
    c.criar(
        &format!("/admin/v1/orgs/{target_id}/agents"),
        &target_token,
        json!({"name": "Outra Bia", "slug": "bia-origem"}),
    )
    .await;

    let (status, error) = c.import(&target_id, &target_token, &bytes).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(code(&error), "conflict");
    let conflicts = error["details"]["conflicts"]
        .as_array()
        .expect("lista de conflitos");
    assert!(!conflicts.is_empty(), "{error}");
    assert!(
        conflicts[0].as_str().unwrap().contains("bia-origem"),
        "{error}"
    );

    let (status, with_flag) = c
        .import_with(&target_id, &target_token, &bytes, "?sobrescrever=true")
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{with_flag}");
    assert_eq!(code(&with_flag), "conflict", "{with_flag}");
    assert!(
        !with_flag["details"]["conflicts"]
            .as_array()
            .expect("conflitos")
            .is_empty(),
        "{with_flag}"
    );

    let (_, categories) = c
        .send(
            Method::GET,
            &format!("/admin/v1/orgs/{target_id}/categories"),
            &target_token,
            None,
        )
        .await;
    assert!(categories["items"].as_array().unwrap().is_empty());
    let (_, products) = c
        .send(
            Method::GET,
            &format!("/admin/v1/orgs/{target_id}/products"),
            &target_token,
            None,
        )
        .await;
    assert!(products["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn import_same_instance_other_org_is_409() {
    let c = Scenario::new("mesma-instancia").await;
    let (source, source_token, _) = build_org(&c, "origem").await;
    let (_, file) = c.download_backup(&source, &source_token, "").await;
    let other = c.create_org("Outra", "outra").await;
    let other_id = other["id"].as_str().unwrap().to_owned();
    let other_token = c.create_token(&other_id, "outra").await;

    let (status, error) = c.import(&other_id, &other_token, &file.bytes).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(code(&error), "conflict");
    assert!(
        error["details"]["conflicts"][0]
            .as_str()
            .unwrap_or("")
            .contains("já existe nesta instância"),
        "{error}"
    );
    let (_, categories) = c
        .send(
            Method::GET,
            &format!("/admin/v1/orgs/{other_id}/categories"),
            &other_token,
            None,
        )
        .await;
    assert!(categories["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn import_that_would_update_rows_is_refused_without_overwrite() {
    let c = Scenario::new("sobrescreve").await;
    let (org, token, _) = build_org(&c, "sobrescreve").await;
    let root = format!("/admin/v1/orgs/{org}");
    let (status, file) = c.download_backup(&org, &token, "").await;
    assert_eq!(status, StatusCode::OK, "{}", file.texto());
    let bytes = file.bytes;

    let (_, agents) = c
        .send(Method::GET, &format!("{root}/agents"), &token, None)
        .await;
    let agente = agents["items"][0].clone();
    let agent_id = agente["id"].as_str().unwrap().to_owned();
    let versao = agente["version"].as_i64().expect("versão do agente");
    let response = c
        .client
        .patch(c.url(&format!("{root}/agents/{agent_id}")))
        .bearer_auth(&token)
        .header("if-match", versao.to_string())
        .json(&json!({"name": "Nome Depois Do Backup"}))
        .send()
        .await
        .expect("altera o agente");
    assert_eq!(response.status(), StatusCode::OK);

    let (status, error) = c.import(&org, &token, &bytes).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(code(&error), "importacao_sobrescreve", "{error}");
    let rows = error["details"]["linhas"].as_u64().unwrap_or(0);
    assert!(rows > 0, "{error}");
    let tabelas = error["details"]["tabelas"].as_object().expect("tabelas");
    assert_eq!(
        tabelas.get("agents").and_then(Value::as_u64),
        Some(1),
        "{error}"
    );
    let examples = error["details"]["exemplos"].as_array().expect("exemplos");
    assert!(!examples.is_empty() && examples.len() <= 5, "{error}");
    assert!(
        examples
            .iter()
            .any(|e| e.as_str().unwrap_or("").starts_with("agents: ")),
        "{error}"
    );
    let sum: u64 = tabelas.values().filter_map(Value::as_u64).sum();
    assert_eq!(sum, rows, "o total bate com a soma das tabelas: {error}");
    let texto = error.to_string();
    for forbidden in [SENTINEL_SECRET, "secrets_enc", "blob_path"] {
        assert!(!texto.contains(forbidden), "vazou `{forbidden}`: {texto}");
    }
    let (_, after) = c
        .send(
            Method::GET,
            &format!("{root}/agents/{agent_id}"),
            &token,
            None,
        )
        .await;
    assert_eq!(
        after["name"], "Nome Depois Do Backup",
        "a recusa não pode ter revertido o agente: {after}"
    );

    let (status, invalid) = c
        .import_with(&org, &token, &bytes, "?sobrescrever=xyz")
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{invalid}");
    assert_eq!(code(&invalid), "validation_error", "{invalid}");

    let (status, report) = c
        .import_with(&org, &token, &bytes, "?sobrescrever=true")
        .await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(report["org"]["id"], org.as_str());
    let inserted: u64 = report["tabelas"]
        .as_object()
        .unwrap()
        .values()
        .map(|t| t["inseridas"].as_u64().unwrap_or(0))
        .sum();
    assert_eq!(inserted, 0, "reimportar não insere: {report}");
    let updated: u64 = report["tabelas"]
        .as_object()
        .unwrap()
        .values()
        .map(|t| t["atualizadas"].as_u64().unwrap_or(0))
        .sum();
    assert_eq!(
        updated,
        report["total"].as_u64().expect("total"),
        "{report}"
    );
    let (_, revertido) = c
        .send(
            Method::GET,
            &format!("{root}/agents/{agent_id}"),
            &token,
            None,
        )
        .await;
    assert_eq!(
        revertido["name"], agente["name"],
        "com sobrescrever o agente volta ao valor do arquivo: {revertido}"
    );

    let clean = Scenario::new("sobrescreve-limpa").await;
    let target = clean.create_org("Destino", "destino").await;
    let target_id = target["id"].as_str().unwrap().to_owned();
    let target_token = clean.create_token(&target_id, "destino").await;
    let (status, report) = clean.import(&target_id, &target_token, &bytes).await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert!(report["total"].as_u64().unwrap_or(0) > 0, "{report}");
}

#[tokio::test]
async fn corrupt_file_and_unknown_version_are_400_without_writing() {
    let c = Scenario::new("corrompido").await;
    let target = c.create_org("Destino", "destino").await;
    let target_id = target["id"].as_str().unwrap().to_owned();
    let token = c.create_token(&target_id, "destino").await;

    let (status, error) = c
        .import(&target_id, &token, b"isto nao e um backup {")
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(code(&error), "invalid_backup");

    let new_version = json!({
        "formato": "poph-backup-org",
        "versao_formato": 99,
        "org": {"id": target_id},
        "tabelas": {},
        "blobs": [],
    });
    let (status, error) = c
        .import(&target_id, &token, new_version.to_string().as_bytes())
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(code(&error), "invalid_backup");
    assert_eq!(error["details"]["version"], 99);

    let (_, agents) = c
        .send(
            Method::GET,
            &format!("/admin/v1/orgs/{target_id}/agents"),
            &token,
            None,
        )
        .await;
    assert!(agents["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn requires_token_and_respects_token_org() {
    let c = Scenario::new("escopo").await;
    let org_a = c.create_org("Org A", "org-a").await;
    let org_b = c.create_org("Org B", "org-b").await;
    let (a, b) = (
        org_a["id"].as_str().unwrap().to_owned(),
        org_b["id"].as_str().unwrap().to_owned(),
    );
    let token_a = c.create_token(&a, "a").await;

    let (status, _) = c.download_backup(&b, "", "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = c.import(&b, "", b"{}").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, error) = c.download_backup(&b, &token_a, "").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{}", error.texto());
    let (status, error) = c.import(&b, &token_a, b"{}").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{error}");
    assert_eq!(code(&error), "forbidden");

    let (status, _) = c.download_backup(&a, &token_a, "").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn body_over_limit_is_413() {
    let c = Scenario::new("limite").await;
    let org = c.create_org("Org", "org-limite").await;
    let id = org["id"].as_str().unwrap().to_owned();
    let token = c.create_token(&id, "limite").await;

    let large = vec![b' '; poph_server::api::LIMIT_IMPORT + 1];
    let (status, error) = c.import(&id, &token, &large).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{error}");
    assert_eq!(code(&error), "payload_too_large");
    assert!(
        error["message"].as_str().unwrap_or("").contains("limite"),
        "{error}"
    );
}

#[tokio::test]
async fn with_model_installed_imported_base_is_searchable() {
    if !model_installed() {
        return;
    }
    let source = Scenario::new("origem-modelo").await;
    let (org, source_token, _) = build_org(&source, "origem").await;
    let (_, file) = source.download_backup(&org, &source_token, "").await;

    let c = Scenario::with_model("modelo", true).await;
    let target = c.create_org("Destino", "destino").await;
    let target_id = target["id"].as_str().unwrap().to_owned();
    let target_token = c.create_token(&target_id, "destino").await;
    let (status, report) = c.import(&target_id, &target_token, &file.bytes).await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert!(report["aviso"].is_null(), "aviso inesperado: {report}");
    assert!(
        report["indice"]["base_conhecimento"]["trechos"]
            .as_u64()
            .unwrap_or(0)
            > 0,
        "índice vazio depois da importação: {report}"
    );
}
