//! Conteúdo de arquivo pela API administrativa (HR-46b).
//!
//! Duas rotas baixam bytes: o arquivo da biblioteca do agente e o anexo de uma mensagem do chat de
//! teste. O que se prova aqui:
//!
//! - `200` com os **bytes idênticos** ao que foi enviado, o SHA-256 conferido dos dois lados, o
//!   `Content-Type` do MIME gravado e o `Content-Disposition` com o nome sanitizado;
//! - `401` sem token, `403` com token de outra org, `404` para id inexistente, para arquivo de
//!   outro agente da mesma org, para anexo fora do caminho (org + agente + conversa + mensagem) e
//!   para blob ausente no disco;
//! - nenhuma resposta carrega o caminho do blob (`data/blobs`, `<org>/<sha256>`), e a rota de
//!   metadados continua sem `blob_path`.
//!
//! Servidor real, banco temporário, porta efêmera. Nenhum teste toca rede externa.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_files::blob::sha256_hex;
use poph_server::Estado;
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

/// PNG mínimo (assinatura + IHDR) — o mesmo do resto dos testes.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89";

fn hash_argon2(valor: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(valor.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

struct Cenario {
    base: String,
    cliente: Client,
    pasta: PathBuf,
    store: std::sync::Arc<Store>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Cenario {
    async fn novo(rotulo: &str) -> Self {
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duracao| duracao.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-conteudo-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        config.server.port = 7480;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = std::sync::Arc::new(Store::open(&config).expect("abre o banco"));
        let estado =
            Estado::novo(std::sync::Arc::clone(&store), config).expect("estado do servidor");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, sinal) = oneshot::channel();
        tokio::spawn(async move {
            let _ = poph_server::serve_at(listener, estado, async {
                let _ = sinal.await;
            })
            .await;
        });

        let cenario = Self {
            base: format!("http://{endereco}"),
            cliente: Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("cliente HTTP"),
            pasta,
            store,
            desligar: Some(desligar),
        };
        cenario.esperar_subir().await;
        cenario
    }

    async fn esperar_subir(&self) {
        let inicio = Instant::now();
        loop {
            match self
                .cliente
                .get(format!("{}/healthz", self.base))
                .send()
                .await
            {
                Ok(resposta) if resposta.status().is_success() => return,
                _ if inicio.elapsed() < Duration::from_secs(10) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                outro => panic!("servidor não subiu: {outro:?}"),
            }
        }
    }

    fn store(&self) -> &Store {
        &self.store
    }

    fn url(&self, caminho: &str) -> String {
        format!("{}{caminho}", self.base)
    }

    async fn pedir(
        &self,
        metodo: Method,
        caminho: &str,
        token: &str,
        corpo: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut pedido = self.cliente.request(metodo, self.url(caminho));
        if !token.is_empty() {
            pedido = pedido.bearer_auth(token);
        }
        if let Some(valor) = corpo {
            pedido = pedido.json(&valor);
        }
        let resposta = pedido.send().await.expect("pedido HTTP");
        let status = resposta.status();
        let texto = resposta.text().await.unwrap_or_default();
        let json = if texto.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&texto).unwrap_or(Value::String(texto))
        };
        (status, json)
    }

    /// `GET` que devolve os bytes crus e os cabeçalhos (o corpo do conteúdo não é JSON).
    async fn baixar(
        &self,
        caminho: &str,
        token: &str,
    ) -> (StatusCode, reqwest::header::HeaderMap, Vec<u8>) {
        let mut pedido = self.cliente.get(self.url(caminho));
        if !token.is_empty() {
            pedido = pedido.bearer_auth(token);
        }
        let resposta = pedido.send().await.expect("pedido HTTP");
        let status = resposta.status();
        let cabecalhos = resposta.headers().clone();
        let bytes = resposta.bytes().await.expect("corpo").to_vec();
        (status, cabecalhos, bytes)
    }

    async fn criar_org(&self, nome: &str, slug: &str) -> String {
        let (status, corpo) = self
            .pedir(
                Method::POST,
                "/admin/v1/orgs",
                TOKEN_GLOBAL,
                Some(json!({ "name": nome, "slug": slug })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar org: {corpo}");
        corpo["id"].as_str().unwrap().to_owned()
    }

    async fn criar_token(&self, org_id: &str, rotulo: &str) -> String {
        let (status, corpo) = self
            .pedir(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/tokens"),
                TOKEN_GLOBAL,
                Some(json!({ "label": rotulo })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar token: {corpo}");
        corpo["value"].as_str().unwrap().to_owned()
    }

    async fn criar_agente(&self, org: &str, token: &str, slug: &str) -> String {
        let (status, corpo) = self
            .pedir(
                Method::POST,
                &format!("/admin/v1/orgs/{org}/agents"),
                token,
                Some(json!({ "name": format!("Agente {slug}"), "slug": slug })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar agente: {corpo}");
        corpo["id"].as_str().unwrap().to_owned()
    }

    /// Põe os bytes na biblioteca do agente (mesma rota que o Console usa).
    async fn subir_arquivo(&self, org: &str, agente: &str, token: &str, bytes: &[u8]) -> Value {
        let resposta = self
            .cliente
            .post(self.url(&format!(
                "/admin/v1/orgs/{org}/agents/{agente}/files?filename=procedimento.png&name=Procedimento%20t%C3%A9cnico.png"
            )))
            .bearer_auth(token)
            .header("content-type", "application/octet-stream")
            .body(bytes.to_vec())
            .send()
            .await
            .expect("upload HTTP");
        let status = resposta.status();
        let corpo: Value = resposta.json().await.expect("JSON do upload");
        assert_eq!(status, StatusCode::CREATED, "upload: {corpo}");
        corpo
    }

    /// Conversa de teste criada pela API e uma mensagem do cliente com um anexo apontando para o
    /// blob que já existe na org (o anexo do caminho é o mesmo formato que o recebimento grava).
    fn semear_anexo(&self, org: &str, agente: &str, sha256: &str) -> (ConversationId, Mensagem) {
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

impl Drop for Cenario {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn codigo(corpo: &Value) -> &str {
    corpo["code"].as_str().unwrap_or("")
}

/// Id de arquivo que não existe (UUID v7 válido).
fn id_inexistente() -> String {
    poph_types::FileId::generate().to_string()
}

#[tokio::test]
async fn biblioteca_conteudo_entrega_os_bytes_com_cabecalhos() {
    let c = Cenario::novo("biblioteca").await;
    let org = c.criar_org("Org A", "org-a").await;
    let token = c.criar_token(&org, "org a").await;
    let agente = c.criar_agente(&org, &token, "agente-a").await;
    let arquivo = c.subir_arquivo(&org, &agente, &token, PNG).await;
    let id = arquivo["id"].as_str().unwrap().to_owned();
    let sha = sha256_hex(PNG);
    assert_eq!(arquivo["sha256"], sha, "hash do upload");

    let rota = format!("/admin/v1/orgs/{org}/agents/{agente}/files/{id}/content");
    let (status, cabecalhos, bytes) = c.baixar(&rota, &token).await;
    assert_eq!(status, StatusCode::OK, "conteúdo do arquivo");
    assert_eq!(bytes, PNG, "bytes idênticos aos enviados");
    assert_eq!(
        sha256_hex(&bytes),
        sha,
        "SHA-256 conferido dos dois lados (resposta e envio)"
    );
    let cabecalho = |nome: &str| {
        cabecalhos
            .get(nome)
            .unwrap_or_else(|| panic!("cabeçalho `{nome}` ausente"))
            .to_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(cabecalho("content-type"), "image/png", "MIME gravado");
    assert_eq!(cabecalho("x-content-sha256"), sha, "sha256 no cabeçalho");
    assert_eq!(cabecalho("etag"), format!("\"{sha}\""), "ETag");
    assert_eq!(
        cabecalho("content-length"),
        PNG.len().to_string(),
        "tamanho"
    );
    let disposicao = cabecalho("content-disposition");
    assert!(
        disposicao.starts_with("inline"),
        "imagem abre no lugar: {disposicao}"
    );
    assert!(
        disposicao.contains("filename=\"Procedimento t_cnico.png\""),
        "nome ASCII sanitizado: {disposicao}"
    );
    assert!(
        disposicao.contains("filename*=UTF-8''Procedimento%20t%C3%A9cnico.png"),
        "nome com acento em UTF-8: {disposicao}"
    );
    assert_eq!(cabecalho("x-content-type-options"), "nosniff");
    assert!(cabecalho("content-security-policy").contains("sandbox"));

    // A rota de metadados continua sem caminho interno de blob.
    let (status, meta) = c
        .pedir(
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
async fn biblioteca_conteudo_isola_org_agente_e_id() {
    let c = Cenario::novo("biblioteca-isola").await;
    let org_a = c.criar_org("Org A", "org-a").await;
    let org_b = c.criar_org("Org B", "org-b").await;
    let token_a = c.criar_token(&org_a, "org a").await;
    let token_b = c.criar_token(&org_b, "org b").await;
    let agente_a = c.criar_agente(&org_a, &token_a, "agente-a").await;
    let agente_a2 = c.criar_agente(&org_a, &token_a, "agente-a2").await;
    let arquivo = c.subir_arquivo(&org_a, &agente_a, &token_a, PNG).await;
    let id = arquivo["id"].as_str().unwrap().to_owned();
    let rota_a = format!("/admin/v1/orgs/{org_a}/agents/{agente_a}/files/{id}/content");

    // Sem token e com token de outra org.
    let (status, _, corpo) = c.baixar(&rota_a, "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        codigo(&serde_json::from_slice(&corpo).unwrap()),
        "unauthorized"
    );
    let (status, _, corpo) = c.baixar(&rota_a, &token_b).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "token de outra org");
    assert_eq!(
        codigo(&serde_json::from_slice(&corpo).unwrap()),
        "forbidden"
    );

    // Caminho da org do token, ids da vítima: nunca serve o arquivo.
    let (status, _, corpo) = c
        .baixar(
            &format!("/admin/v1/orgs/{org_b}/agents/{agente_a}/files/{id}/content"),
            &token_b,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "agente de outra org");
    let texto = String::from_utf8_lossy(&corpo);
    assert_eq!(
        codigo(&serde_json::from_slice(&corpo).unwrap()),
        "not_found",
        "corpo de erro"
    );
    assert!(
        !texto.contains("blobs") && !texto.contains("dados/"),
        "resposta sem caminho interno: {texto}"
    );

    // Id inexistente e arquivo de outro agente da mesma org.
    let (status, _, _) = c
        .baixar(
            &format!(
                "/admin/v1/orgs/{org_a}/agents/{agente_a}/files/{}/content",
                id_inexistente()
            ),
            &token_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "id inexistente");
    let (status, _, _) = c
        .baixar(
            &format!("/admin/v1/orgs/{org_a}/agents/{agente_a2}/files/{id}/content"),
            &token_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "arquivo de outro agente");

    // Linha na biblioteca sem blob no disco: 404 genérico, sem caminho.
    let sem_blob = c
        .store()
        .library()
        .create(
            org_a.parse().unwrap(),
            poph_store::NewLibraryFile {
                agent_id: agente_a.parse().unwrap(),
                name: "Sumiu".to_owned(),
                description: String::new(),
                mime: "application/pdf".to_owned(),
                size_bytes: 10,
                sha256: "b".repeat(64),
                blob_path: format!("{org_a}/{}", "b".repeat(64)),
            },
        )
        .expect("arquivo sem blob");
    let (status, _, corpo) = c
        .baixar(
            &format!(
                "/admin/v1/orgs/{org_a}/agents/{agente_a}/files/{}/content",
                sem_blob.id
            ),
            &token_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "blob ausente");
    let texto = String::from_utf8_lossy(&corpo);
    assert_eq!(
        codigo(&serde_json::from_slice(&corpo).unwrap()),
        "not_found"
    );
    assert!(
        !texto.contains("blobs") && !texto.contains(&"b".repeat(64)),
        "mensagem sem caminho nem hash: {texto}"
    );
}

#[tokio::test]
async fn anexo_conteudo_entrega_os_bytes_do_chat_de_teste() {
    let c = Cenario::novo("anexo").await;
    let org_a = c.criar_org("Org A", "org-a").await;
    let org_b = c.criar_org("Org B", "org-b").await;
    let token_a = c.criar_token(&org_a, "org a").await;
    let token_b = c.criar_token(&org_b, "org b").await;
    let agente_a = c.criar_agente(&org_a, &token_a, "agente-a").await;
    let arquivo = c.subir_arquivo(&org_a, &agente_a, &token_a, PNG).await;
    let sha = arquivo["sha256"].as_str().unwrap().to_owned();

    let (conversa, mensagem) = c.semear_anexo(&org_a, &agente_a, &sha);
    let anexos = c
        .store()
        .messages()
        .attachments(org_a.parse().unwrap(), mensagem.id)
        .expect("anexos da mensagem");
    assert_eq!(anexos.len(), 1);
    let anexo = anexos[0].id;
    let rota = format!(
        "/admin/v1/orgs/{org_a}/agents/{agente_a}/test-chat/{conversa}/messages/{}/attachments/{anexo}/content",
        mensagem.id
    );

    let (status, cabecalhos, bytes) = c.baixar(&rota, &token_a).await;
    assert_eq!(status, StatusCode::OK, "conteúdo do anexo");
    assert_eq!(bytes, PNG, "bytes idênticos ao blob");
    assert_eq!(sha256_hex(&bytes), sha, "sha256 conferido");
    let cabecalho = |nome: &str| {
        cabecalhos
            .get(nome)
            .unwrap_or_else(|| panic!("cabeçalho `{nome}` ausente"))
            .to_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(cabecalho("content-type"), "image/png");
    assert_eq!(cabecalho("x-content-sha256"), sha);
    assert!(
        cabecalho("content-disposition")
            .contains("filename*=UTF-8''Procedimento%20t%C3%A9cnico.png"),
        "nome com acento: {}",
        cabecalho("content-disposition")
    );

    // Sem token e com token de outra org.
    let (status, _, _) = c.baixar(&rota, "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = c.baixar(&rota, &token_b).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Anexo que não é da mensagem, mensagem de outra conversa, anexo inexistente.
    let outra_conversa = c
        .store()
        .conversations()
        .create(
            org_a.parse().unwrap(),
            NewConversation {
                agent_id: agente_a.parse().unwrap(),
                integration_id: None,
                canal: "test".to_owned(),
                peer_externo: None,
                target_department: None,
            },
        )
        .expect("outra conversa");
    let (status, _, _) = c
        .baixar(
            &format!(
                "/admin/v1/orgs/{org_a}/agents/{agente_a}/test-chat/{}/messages/{}/attachments/{anexo}/content",
                outra_conversa.id, mensagem.id
            ),
            &token_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "mensagem de outra conversa");
    let (status, _, _) = c
        .baixar(
            &format!(
                "/admin/v1/orgs/{org_a}/agents/{agente_a}/test-chat/{conversa}/messages/{}/attachments/{}/content",
                mensagem.id,
                poph_types::AttachmentId::generate()
            ),
            &token_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "anexo inexistente");
    let (status, _, _) = c
        .baixar(
            &format!(
                "/admin/v1/orgs/{org_a}/agents/{agente_a}/test-chat/{conversa}/messages/{}/attachments/{anexo}/content",
                poph_types::MessageId::generate()
            ),
            &token_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "mensagem inexistente");

    // Anexo apontando para um blob que não existe: 404 sem caminho.
    let (conversa2, mensagem2) = c.semear_anexo(&org_a, &agente_a, &"c".repeat(64));
    let anexo2 = c
        .store()
        .messages()
        .attachments(org_a.parse().unwrap(), mensagem2.id)
        .expect("anexos")
        .remove(0)
        .id;
    let (status, _, corpo) = c
        .baixar(
            &format!(
                "/admin/v1/orgs/{org_a}/agents/{agente_a}/test-chat/{conversa2}/messages/{}/attachments/{anexo2}/content",
                mensagem2.id
            ),
            &token_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "blob ausente");
    let texto = String::from_utf8_lossy(&corpo);
    assert!(
        !texto.contains("blobs") && !texto.contains(&"c".repeat(64)),
        "mensagem sem caminho nem hash: {texto}"
    );
}
