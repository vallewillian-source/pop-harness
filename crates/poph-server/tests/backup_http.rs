//! Testes de ponta a ponta das rotas de exportar e importar organização (HR-48e-1).
//!
//! Servidor de verdade (porta efêmera), banco, pasta de blobs e cliente HTTP de verdade: nada de
//! chamar o handler direto. O que se prova aqui:
//!
//! - a exportação devolve o mesmo formato da CLI (`poph-backup-org` v2, org de origem, `tabelas`,
//!   `blobs`), como anexo, sem conversas por padrão e com `com_conversas=true` quando pedido;
//! - segredo de integração não sai no arquivo (nem em claro, nem cifrado) e caminho de blob não sai;
//! - a importação reconstrói a org de destino, preserva o sha256 dos blobs (e o download devolve os
//!   mesmos bytes), pede os segredos de novo e não duplica na segunda vez;
//! - conflito de chave natural é `409` **sem gravar nada**; arquivo corrompido e versão desconhecida
//!   são `400` sem gravar nada;
//! - `401` sem token e `403` com token de outra org nas duas rotas;
//! - corpo acima do limite é `413`;
//! - sem modelo de embedding a importação termina com aviso e o índice pendente (nunca um índice
//!   vazio silencioso); com o modelo instalado, a base da org importada fica buscável.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::{CipherKey, Estado};
use poph_store::Store;
use poph_types::HarnessConfig;
use poph_types::config::GlobalTokenConfig;
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

/// Valor do token global usado nos testes (o servidor guarda só o hash).
const TOKEN_GLOBAL: &str = "pophg_teste-global-hr48e1";
/// Chave de cifra do cenário: é o que deixa o segredo da integração decifrável (nomes dos campos).
const CHAVE_CIFRA: &str = "chave-de-teste-hr48e1";
/// Segredo que **não** pode aparecer no arquivo exportado nem na resposta.
const SENTINELA_SEGREDO: &str = "SENTINELA-SEGREDO-hr48e1";

/// PNG mínimo (a assinatura é o que a detecção por conteúdo usa).
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89";

fn hash_argon2(valor: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(valor.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

/// Servidor no ar, com banco, blobs e configuração próprios. O `Drop` desliga e apaga a pasta.
struct Cenario {
    base: String,
    cliente: Client,
    pasta: PathBuf,
    store: Arc<Store>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Cenario {
    async fn novo(rotulo: &str) -> Self {
        Self::com_modelo(rotulo, false).await
    }

    /// `com_modelo` liga a pasta de modelos do repositório ao diretório de dados (só quando ela
    /// existe), como os testes da CLI fazem.
    async fn com_modelo(rotulo: &str, com_modelo: bool) -> Self {
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duracao| duracao.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-backup-http-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");
        if com_modelo && modelo_instalado() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(pasta_modelos_repo(), pasta.join("dados").join("modelos"))
                .expect("liga a pasta de modelos");
        }

        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        config.server.port = 7480;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let chave = CipherKey::derive(CHAVE_CIFRA).expect("chave de cifra");
        let estado = Estado::novo(Arc::clone(&store), config)
            .expect("estado do servidor")
            .com_chave_de_cifra(Some(chave));

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

        let cliente = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("cliente HTTP");
        let cenario = Self {
            base: format!("http://{endereco}"),
            cliente,
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
                _ if inicio.elapsed() < Duration::from_secs(20) => {
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

    /// Requisição com corpo JSON (ou sem corpo) e resposta como texto + JSON.
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

    /// `GET …/backup`: devolve status, cabeçalhos e o corpo cru (o arquivo).
    async fn baixar_backup(&self, org: &str, token: &str, query: &str) -> (StatusCode, Backups) {
        let resposta = self
            .cliente
            .get(self.url(&format!("/admin/v1/orgs/{org}/backup{query}")))
            .bearer_auth(token)
            .send()
            .await
            .expect("pedido HTTP");
        let status = resposta.status();
        let cabecalhos = resposta.headers().clone();
        let bytes = resposta.bytes().await.expect("corpo").to_vec();
        (status, Backups { cabecalhos, bytes })
    }

    /// `POST …/backup/import` com o arquivo no corpo cru.
    async fn importar(&self, org: &str, token: &str, bytes: &[u8]) -> (StatusCode, Value) {
        self.importar_com(org, token, bytes, "").await
    }

    /// `POST …/backup/import` com query própria (`?sobrescrever=true`, ou valor inválido).
    async fn importar_com(
        &self,
        org: &str,
        token: &str,
        bytes: &[u8],
        query: &str,
    ) -> (StatusCode, Value) {
        let resposta = self
            .cliente
            .post(self.url(&format!("/admin/v1/orgs/{org}/backup/import{query}")))
            .bearer_auth(token)
            .header("content-type", "application/json")
            .body(bytes.to_vec())
            .send()
            .await
            .expect("pedido HTTP");
        let status = resposta.status();
        let texto = resposta.text().await.unwrap_or_default();
        let json = if texto.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&texto).unwrap_or(Value::String(texto))
        };
        (status, json)
    }

    async fn criar_org(&self, nome: &str, slug: &str) -> Value {
        let (status, corpo) = self
            .pedir(
                Method::POST,
                "/admin/v1/orgs",
                TOKEN_GLOBAL,
                Some(json!({ "name": nome, "slug": slug })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar org: {corpo}");
        corpo
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
        corpo["value"].as_str().expect("valor do token").to_owned()
    }

    async fn criar(&self, caminho: &str, token: &str, corpo: Value) -> Value {
        let (status, json) = self.pedir(Method::POST, caminho, token, Some(corpo)).await;
        assert_eq!(status, StatusCode::CREATED, "{caminho}: {json}");
        json
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

struct Backups {
    cabecalhos: reqwest::header::HeaderMap,
    bytes: Vec<u8>,
}

impl Backups {
    fn texto(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).expect("arquivo de backup é JSON")
    }

    fn cabecalho(&self, nome: &str) -> String {
        self.cabecalhos
            .get(nome)
            .and_then(|valor| valor.to_str().ok())
            .unwrap_or("")
            .to_owned()
    }
}

fn pasta_modelos_repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/modelos"))
}

fn modelo_instalado() -> bool {
    let modelos = pasta_modelos_repo();
    modelos.join("model_int8.onnx").is_file() && modelos.join("tokenizer.json").is_file()
}

fn codigo(corpo: &Value) -> &str {
    corpo["code"].as_str().unwrap_or("")
}

/// Org com o material que o backup tem de levar: agente com instruções, categoria, departamento,
/// produto, grupo e item de base, arquivo na biblioteca (blob de verdade) e integração com segredo.
async fn montar_org(c: &Cenario, slug: &str) -> (String, String, Value) {
    let org = c.criar_org(&format!("Org {slug}"), slug).await;
    let id = org["id"].as_str().unwrap().to_owned();
    let token = c.criar_token(&id, slug).await;
    let raiz = format!("/admin/v1/orgs/{id}");
    let agente = c
        .criar(
            &format!("{raiz}/agents"),
            &token,
            json!({"name": "Bia", "slug": format!("bia-{slug}")}),
        )
        .await;
    let agente_id = agente["id"].as_str().unwrap().to_owned();
    c.criar(
        &format!("{raiz}/agents/{agente_id}/instructions"),
        &token,
        json!({"markdown": "Atenda com simpatia."}),
    )
    .await;
    let categoria = c
        .criar(
            &format!("{raiz}/categories"),
            &token,
            json!({"name": format!("Categoria {slug}")}),
        )
        .await;
    c.criar(
        &format!("{raiz}/departments"),
        &token,
        json!({"name": format!("Departamento {slug}")}),
    )
    .await;
    c.criar(
        &format!("{raiz}/products"),
        &token,
        json!({"name": format!("Produto {slug}"), "price_cents": 1234, "category_id": categoria["id"]}),
    )
    .await;
    let grupo = c
        .criar(
            &format!("{raiz}/knowledge"),
            &token,
            json!({"name": format!("Grupo {slug}")}),
        )
        .await;
    c.criar(
        &format!("{raiz}/knowledge/{}/items", grupo["id"].as_str().unwrap()),
        &token,
        json!({"title": "Prazo de entrega", "content": "O prazo é de 5 dias úteis."}),
    )
    .await;
    // Sem o vínculo, o agente não enxerga o grupo e a base fica sem trechos para reindexar.
    let (status, vinculo) = c
        .pedir(
            Method::POST,
            &format!(
                "{raiz}/agents/{agente_id}/knowledge/{}",
                grupo["id"].as_str().unwrap()
            ),
            &token,
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "vínculo do grupo: {vinculo}");
    // Integração com segredo gravado e cifrado (o valor nunca pode sair no arquivo).
    let cifrado = CipherKey::derive(CHAVE_CIFRA)
        .expect("chave")
        .cifrar(&std::collections::BTreeMap::from([(
            "token".to_owned(),
            SENTINELA_SEGREDO.to_owned(),
        )]))
        .expect("cifra");
    c.store()
        .create_integration_with_config(
            id.parse().expect("org id"),
            agente_id.parse().expect("agent id"),
            "eco".to_owned(),
            "active".to_owned(),
            json!({"instancia": "bia"}),
            Some(cifrado),
        )
        .expect("integração com segredo");
    // Arquivo da biblioteca com bytes de verdade: o blob é lido do disco na exportação.
    let resposta = c
        .cliente
        .post(c.url(&format!(
            "{raiz}/agents/{agente_id}/files?filename=catalogo.png"
        )))
        .bearer_auth(&token)
        .header("content-type", "application/octet-stream")
        .body(PNG.to_vec())
        .send()
        .await
        .expect("upload");
    assert_eq!(resposta.status(), StatusCode::CREATED, "upload do arquivo");
    let arquivo: Value = resposta.json().await.expect("json do arquivo");
    (id, token, arquivo)
}

// ───────────────────────────────── Exportar ─────────────────────────────────

#[tokio::test]
async fn exportar_devolve_o_arquivo_da_org_sem_segredo_nem_caminho() {
    let c = Cenario::novo("exporta").await;
    let (org, token, arquivo) = montar_org(&c, "origem").await;

    let (status, arquivo_baixado) = c.baixar_backup(&org, &token, "").await;
    assert_eq!(status, StatusCode::OK, "{}", arquivo_baixado.texto());
    assert_eq!(
        arquivo_baixado.cabecalho("content-type"),
        "application/json"
    );
    assert_eq!(
        arquivo_baixado.cabecalho("x-content-type-options"),
        "nosniff"
    );
    let disposicao = arquivo_baixado.cabecalho("content-disposition");
    assert!(
        disposicao.starts_with("attachment; filename=\"origem-backup-"),
        "disposição: {disposicao}"
    );
    assert!(disposicao.ends_with(".json\""), "disposição: {disposicao}");

    let documento = arquivo_baixado.json();
    assert_eq!(documento["formato"], "poph-backup-org");
    assert_eq!(documento["versao_formato"], 2);
    assert_eq!(documento["org_origem"]["id"], org.as_str());
    assert_eq!(documento["org_origem"]["slug"], "origem");
    assert_eq!(documento["com_conversas"], false);
    // Sem `com_conversas`, nenhuma tabela de histórico entra.
    for tabela in [
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
            documento["tabelas"].get(tabela).is_none(),
            "tabela `{tabela}` não deveria estar no arquivo sem com_conversas"
        );
    }
    assert_eq!(documento["tabelas"]["agents"].as_array().unwrap().len(), 1);
    assert_eq!(
        documento["tabelas"]["products"].as_array().unwrap().len(),
        1
    );
    // A integração leva a marca e os nomes dos campos — nunca o valor.
    let integracao = &documento["tabelas"]["integrations"][0];
    assert_eq!(integracao["tem_segredo"], true);
    assert_eq!(integracao["segredos_campos"], json!(["token"]));
    assert_eq!(
        documento["tabelas"]["file_library"][0]["sha256"],
        arquivo["sha256"]
    );
    assert!(!documento["blobs"].as_array().unwrap().is_empty());

    // Nada de segredo em claro, de segredo cifrado e de caminho de blob no arquivo.
    let bruto = arquivo_baixado.texto();
    for proibido in [
        SENTINELA_SEGREDO,
        "secrets_enc",
        "blob_path",
        "blobs/",
        CHAVE_CIFRA,
    ] {
        assert!(
            !bruto.contains(proibido),
            "o arquivo exportado não pode conter `{proibido}`"
        );
    }

    // `com_conversas=true` leva o histórico.
    let conversa = c
        .store()
        .conversations()
        .create(
            org.parse().expect("org id"),
            poph_store::NewConversation {
                agent_id: documento["tabelas"]["agents"][0]["id"]
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
    let (status, com_conversas) = c.baixar_backup(&org, &token, "?com_conversas=true").await;
    assert_eq!(status, StatusCode::OK);
    let documento = com_conversas.json();
    assert_eq!(documento["com_conversas"], true);
    assert_eq!(
        documento["tabelas"]["conversations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        documento["tabelas"]["messages"].as_array().unwrap().len(),
        1
    );

    // Valor inválido é recusado no formato da API.
    let (status, erro) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{org}/backup?com_conversas=talvez"),
            &token,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(codigo(&erro), "validation_error", "{erro}");
}

// ───────────────────────────────── Importar ─────────────────────────────────

/// Exporta a org de um cenário e devolve os bytes do arquivo e o sha256 do arquivo da biblioteca.
async fn exportar_origem(rotulo: &str) -> (Vec<u8>, String) {
    let origem = Cenario::novo(&format!("origem-{rotulo}")).await;
    let (org, token, _) = montar_org(&origem, "origem").await;
    let (status, arquivo) = origem.baixar_backup(&org, &token, "").await;
    assert_eq!(status, StatusCode::OK, "{}", arquivo.texto());
    let sha = arquivo.json()["tabelas"]["file_library"][0]["sha256"]
        .as_str()
        .unwrap()
        .to_owned();
    (arquivo.bytes, sha)
}

#[tokio::test]
async fn importar_reconstroi_a_org_preserva_o_blob_e_nao_duplica() {
    // A instância de destino é outra (outro banco): é o uso real — restaurar um arquivo numa
    // instalação. Importar na MESMA instância para outra org é recusado de propósito, porque os
    // ids são preservados (coberto em `importar_na_mesma_instancia_em_outra_org_da_409`).
    let (bytes, sha) = exportar_origem("importa").await;
    let c = Cenario::novo("destino-importa").await;

    let destino = c.criar_org("Destino", "destino").await;
    let destino_id = destino["id"].as_str().unwrap().to_owned();
    let token_destino = c.criar_token(&destino_id, "destino").await;
    let vizinha = c.criar_org("Vizinha", "vizinha").await;
    let vizinha_id = vizinha["id"].as_str().unwrap().to_owned();
    let token_vizinha = c.criar_token(&vizinha_id, "vizinha").await;
    let (_, antes_vizinha) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{vizinha_id}/agents"),
            &token_vizinha,
            None,
        )
        .await;

    let (status, relatorio) = c.importar(&destino_id, &token_destino, &bytes).await;
    assert_eq!(status, StatusCode::OK, "{relatorio}");
    assert_eq!(relatorio["org"]["id"], destino_id.as_str());
    assert_eq!(relatorio["org"]["criada"], false);
    assert_eq!(relatorio["com_conversas"], false);
    assert!(
        relatorio["total"].as_u64().unwrap_or(0) > 0,
        "relatório sem total: {relatorio}"
    );
    assert_eq!(relatorio["segredos_a_redigitar"], 1);
    // Sem modelo de embedding no diretório de dados: aviso explícito e índice pendente.
    if !modelo_instalado() {
        assert!(
            relatorio["aviso"]
                .as_str()
                .is_some_and(|a| a.contains("poph kb reindex")),
            "faltou o aviso de reindexação: {relatorio}"
        );
        assert_eq!(relatorio["indice"]["base_conhecimento"]["trechos"], 0);
    }

    // O estado reconstruído: agente, produto, base e biblioteca com o mesmo sha256.
    let (_, agentes) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{destino_id}/agents"),
            &token_destino,
            None,
        )
        .await;
    assert_eq!(agentes["items"].as_array().unwrap().len(), 1, "{agentes}");
    let agente = agentes["items"][0]["id"].as_str().unwrap().to_owned();
    let (_, produtos) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{destino_id}/products"),
            &token_destino,
            None,
        )
        .await;
    assert_eq!(produtos["items"].as_array().unwrap().len(), 1);
    let (_, arquivos) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{destino_id}/agents/{agente}/files"),
            &token_destino,
            None,
        )
        .await;
    assert_eq!(arquivos["items"].as_array().unwrap().len(), 1);
    assert_eq!(arquivos["items"][0]["sha256"], sha.as_str());
    // O download no destino devolve os mesmos bytes do arquivo original.
    let resposta = c
        .cliente
        .get(c.url(&format!(
            "/admin/v1/orgs/{destino_id}/agents/{agente}/files/{}/content",
            arquivos["items"][0]["id"].as_str().unwrap()
        )))
        .bearer_auth(&token_destino)
        .send()
        .await
        .expect("download");
    assert_eq!(resposta.status(), StatusCode::OK);
    assert_eq!(
        resposta
            .headers()
            .get("x-content-sha256")
            .and_then(|v| v.to_str().ok()),
        Some(sha.as_str())
    );
    assert_eq!(
        resposta.bytes().await.expect("bytes").to_vec(),
        PNG.to_vec()
    );

    // A integração veio sem o segredo e a lista diz que precisa digitar de novo.
    let (_, integracoes) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{destino_id}/agents/{agente}/integrations"),
            &token_destino,
            None,
        )
        .await;
    let integracao = &integracoes["items"][0];
    assert_eq!(
        integracao["segredos_preenchidos"],
        json!([]),
        "{integracao}"
    );
    assert_eq!(integracao["configuracao"]["instancia"], "bia");

    // Segunda importação sem `?sobrescrever=true`: recusada antes de gravar (HR-48e-3).
    let (status, recusa) = c.importar(&destino_id, &token_destino, &bytes).await;
    assert_eq!(status, StatusCode::CONFLICT, "{recusa}");
    assert_eq!(codigo(&recusa), "importacao_sobrescreve");
    assert!(
        recusa["details"]["linhas"].as_u64().unwrap_or(0) > 0,
        "{recusa}"
    );
    assert!(
        !recusa["details"]["exemplos"]
            .as_array()
            .expect("exemplos")
            .is_empty(),
        "{recusa}"
    );
    let (_, agentes_apos_recusa) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{destino_id}/agents"),
            &token_destino,
            None,
        )
        .await;
    assert_eq!(agentes_apos_recusa["items"].as_array().unwrap().len(), 1);

    // Com `?sobrescrever=true`: nada de novo, mesmas contagens (idempotência).
    let (status, segunda) = c
        .importar_com(&destino_id, &token_destino, &bytes, "?sobrescrever=true")
        .await;
    assert_eq!(status, StatusCode::OK, "{segunda}");
    let inseridas: u64 = segunda["tabelas"]
        .as_object()
        .unwrap()
        .values()
        .map(|t| t["inseridas"].as_u64().unwrap_or(0))
        .sum();
    assert_eq!(
        inseridas, 0,
        "a segunda importação inseriu linha: {segunda}"
    );
    let (_, agentes_depois) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{destino_id}/agents"),
            &token_destino,
            None,
        )
        .await;
    assert_eq!(agentes_depois["items"].as_array().unwrap().len(), 1);

    // A vizinha não mudou.
    let (_, depois_vizinha) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{vizinha_id}/agents"),
            &token_vizinha,
            None,
        )
        .await;
    assert_eq!(depois_vizinha, antes_vizinha);
}

#[tokio::test]
async fn conflito_de_slug_da_409_e_nao_grava_nada() {
    let (bytes, _) = exportar_origem("conflito").await;
    let c = Cenario::novo("destino-conflito").await;

    let destino = c.criar_org("Destino", "destino").await;
    let destino_id = destino["id"].as_str().unwrap().to_owned();
    let token_destino = c.criar_token(&destino_id, "destino").await;
    // Mesmo slug de agente, id diferente: a importação recusa antes de gravar.
    c.criar(
        &format!("/admin/v1/orgs/{destino_id}/agents"),
        &token_destino,
        json!({"name": "Outra Bia", "slug": "bia-origem"}),
    )
    .await;

    let (status, erro) = c.importar(&destino_id, &token_destino, &bytes).await;
    assert_eq!(status, StatusCode::CONFLICT, "{erro}");
    assert_eq!(codigo(&erro), "conflict");
    let conflitos = erro["details"]["conflicts"]
        .as_array()
        .expect("lista de conflitos");
    assert!(!conflitos.is_empty(), "{erro}");
    assert!(
        conflitos[0].as_str().unwrap().contains("bia-origem"),
        "{erro}"
    );

    // `?sobrescrever=true` não transforma conflito em "merge": chave natural com outro id continua
    // recusada, e nada é gravado.
    let (status, com_flag) = c
        .importar_com(&destino_id, &token_destino, &bytes, "?sobrescrever=true")
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{com_flag}");
    assert_eq!(codigo(&com_flag), "conflict", "{com_flag}");
    assert!(
        !com_flag["details"]["conflicts"]
            .as_array()
            .expect("conflitos")
            .is_empty(),
        "{com_flag}"
    );

    // Nada foi gravado: o destino continua sem categoria e sem produto.
    let (_, categorias) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{destino_id}/categories"),
            &token_destino,
            None,
        )
        .await;
    assert!(categorias["items"].as_array().unwrap().is_empty());
    let (_, produtos) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{destino_id}/products"),
            &token_destino,
            None,
        )
        .await;
    assert!(produtos["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn importar_na_mesma_instancia_em_outra_org_da_409() {
    // O mesmo arquivo na mesma instância, em outra org: os ids preservados já existem ali, então
    // a importação recusa com a mensagem própria — nada é gravado pela metade.
    let c = Cenario::novo("mesma-instancia").await;
    let (origem, token_origem, _) = montar_org(&c, "origem").await;
    let (_, arquivo) = c.baixar_backup(&origem, &token_origem, "").await;
    let outra = c.criar_org("Outra", "outra").await;
    let outra_id = outra["id"].as_str().unwrap().to_owned();
    let token_outra = c.criar_token(&outra_id, "outra").await;

    let (status, erro) = c.importar(&outra_id, &token_outra, &arquivo.bytes).await;
    assert_eq!(status, StatusCode::CONFLICT, "{erro}");
    assert_eq!(codigo(&erro), "conflict");
    assert!(
        erro["details"]["conflicts"][0]
            .as_str()
            .unwrap_or("")
            .contains("já existe nesta instância"),
        "{erro}"
    );
    // Nada gravado na org de destino.
    let (_, categorias) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{outra_id}/categories"),
            &token_outra,
            None,
        )
        .await;
    assert!(categorias["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn importacao_que_atualizaria_linhas_e_recusada_sem_sobrescrever() {
    // Importar de volta na MESMA org reverteria o que mudou depois do arquivo. Por padrão a
    // importação recusa antes de gravar; só `?sobrescrever=true` autoriza.
    let c = Cenario::novo("sobrescreve").await;
    let (org, token, _) = montar_org(&c, "sobrescreve").await;
    let raiz = format!("/admin/v1/orgs/{org}");
    let (status, arquivo) = c.baixar_backup(&org, &token, "").await;
    assert_eq!(status, StatusCode::OK, "{}", arquivo.texto());
    let bytes = arquivo.bytes;

    // Depois da exportação, o agente muda de nome (é o ajuste que uma reimportação reverteria).
    let (_, agentes) = c
        .pedir(Method::GET, &format!("{raiz}/agents"), &token, None)
        .await;
    let agente = agentes["items"][0].clone();
    let agente_id = agente["id"].as_str().unwrap().to_owned();
    let versao = agente["version"].as_i64().expect("versão do agente");
    let resposta = c
        .cliente
        .patch(c.url(&format!("{raiz}/agents/{agente_id}")))
        .bearer_auth(&token)
        .header("if-match", versao.to_string())
        .json(&json!({"name": "Nome Depois Do Backup"}))
        .send()
        .await
        .expect("altera o agente");
    assert_eq!(resposta.status(), StatusCode::OK);

    // Sem o parâmetro: 409 com o que seria atualizado, e nada gravado.
    let (status, erro) = c.importar(&org, &token, &bytes).await;
    assert_eq!(status, StatusCode::CONFLICT, "{erro}");
    assert_eq!(codigo(&erro), "importacao_sobrescreve", "{erro}");
    let linhas = erro["details"]["linhas"].as_u64().unwrap_or(0);
    assert!(linhas > 0, "{erro}");
    let tabelas = erro["details"]["tabelas"].as_object().expect("tabelas");
    assert_eq!(
        tabelas.get("agents").and_then(Value::as_u64),
        Some(1),
        "{erro}"
    );
    let exemplos = erro["details"]["exemplos"].as_array().expect("exemplos");
    assert!(!exemplos.is_empty() && exemplos.len() <= 5, "{erro}");
    assert!(
        exemplos
            .iter()
            .any(|e| e.as_str().unwrap_or("").starts_with("agents: ")),
        "{erro}"
    );
    let soma: u64 = tabelas.values().filter_map(Value::as_u64).sum();
    assert_eq!(soma, linhas, "o total bate com a soma das tabelas: {erro}");
    // Nenhum segredo, caminho de blob ou conteúdo de arquivo na recusa.
    let texto = erro.to_string();
    for proibido in [SENTINELA_SEGREDO, "secrets_enc", "blob_path"] {
        assert!(!texto.contains(proibido), "vazou `{proibido}`: {texto}");
    }
    let (_, depois) = c
        .pedir(
            Method::GET,
            &format!("{raiz}/agents/{agente_id}"),
            &token,
            None,
        )
        .await;
    assert_eq!(
        depois["name"], "Nome Depois Do Backup",
        "a recusa não pode ter revertido o agente: {depois}"
    );

    // Parâmetro que não é booleano: 400 claro.
    let (status, invalido) = c
        .importar_com(&org, &token, &bytes, "?sobrescrever=xyz")
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{invalido}");
    assert_eq!(codigo(&invalido), "validation_error", "{invalido}");

    // Com a autorização explícita: importa e o agente volta ao estado do arquivo.
    let (status, relatorio) = c
        .importar_com(&org, &token, &bytes, "?sobrescrever=true")
        .await;
    assert_eq!(status, StatusCode::OK, "{relatorio}");
    assert_eq!(relatorio["org"]["id"], org.as_str());
    let inseridas: u64 = relatorio["tabelas"]
        .as_object()
        .unwrap()
        .values()
        .map(|t| t["inseridas"].as_u64().unwrap_or(0))
        .sum();
    assert_eq!(inseridas, 0, "reimportar não insere: {relatorio}");
    let atualizadas: u64 = relatorio["tabelas"]
        .as_object()
        .unwrap()
        .values()
        .map(|t| t["atualizadas"].as_u64().unwrap_or(0))
        .sum();
    assert_eq!(
        atualizadas,
        relatorio["total"].as_u64().expect("total"),
        "{relatorio}"
    );
    let (_, revertido) = c
        .pedir(
            Method::GET,
            &format!("{raiz}/agents/{agente_id}"),
            &token,
            None,
        )
        .await;
    assert_eq!(
        revertido["name"], agente["name"],
        "com sobrescrever o agente volta ao valor do arquivo: {revertido}"
    );

    // Destino limpo continua entrando sem o parâmetro (nada a sobrescrever).
    let limpa = Cenario::novo("sobrescreve-limpa").await;
    let destino = limpa.criar_org("Destino", "destino").await;
    let destino_id = destino["id"].as_str().unwrap().to_owned();
    let token_destino = limpa.criar_token(&destino_id, "destino").await;
    let (status, relatorio) = limpa.importar(&destino_id, &token_destino, &bytes).await;
    assert_eq!(status, StatusCode::OK, "{relatorio}");
    assert!(relatorio["total"].as_u64().unwrap_or(0) > 0, "{relatorio}");
}

#[tokio::test]
async fn arquivo_corrompido_e_versao_desconhecida_dao_400_sem_gravar() {
    let c = Cenario::novo("corrompido").await;
    let destino = c.criar_org("Destino", "destino").await;
    let destino_id = destino["id"].as_str().unwrap().to_owned();
    let token = c.criar_token(&destino_id, "destino").await;

    let (status, erro) = c
        .importar(&destino_id, &token, b"isto nao e um backup {")
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{erro}");
    assert_eq!(codigo(&erro), "invalid_backup");

    let versao_nova = json!({
        "formato": "poph-backup-org",
        "versao_formato": 99,
        "org": {"id": destino_id},
        "tabelas": {},
        "blobs": [],
    });
    let (status, erro) = c
        .importar(&destino_id, &token, versao_nova.to_string().as_bytes())
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{erro}");
    assert_eq!(codigo(&erro), "invalid_backup");
    assert_eq!(erro["details"]["version"], 99);

    // Nada gravado: a org continua vazia.
    let (_, agentes) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{destino_id}/agents"),
            &token,
            None,
        )
        .await;
    assert!(agentes["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn exige_token_e_respeita_a_org_do_token() {
    let c = Cenario::novo("escopo").await;
    let org_a = c.criar_org("Org A", "org-a").await;
    let org_b = c.criar_org("Org B", "org-b").await;
    let (a, b) = (
        org_a["id"].as_str().unwrap().to_owned(),
        org_b["id"].as_str().unwrap().to_owned(),
    );
    let token_a = c.criar_token(&a, "a").await;

    // Sem token: 401 nas duas rotas.
    let (status, _) = c.baixar_backup(&b, "", "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = c.importar(&b, "", b"{}").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Token da org A no caminho da B: 403 nas duas (antes de ler o corpo).
    let (status, erro) = c.baixar_backup(&b, &token_a, "").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{}", erro.texto());
    let (status, erro) = c.importar(&b, &token_a, b"{}").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{erro}");
    assert_eq!(codigo(&erro), "forbidden");

    // Token da A no próprio caminho: a exportação responde 200.
    let (status, _) = c.baixar_backup(&a, &token_a, "").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn corpo_acima_do_limite_da_413() {
    let c = Cenario::novo("limite").await;
    let org = c.criar_org("Org", "org-limite").await;
    let id = org["id"].as_str().unwrap().to_owned();
    let token = c.criar_token(&id, "limite").await;

    // Um byte além do teto: o servidor recusa em vez de carregar o corpo.
    let grande = vec![b' '; poph_server::api::LIMIT_IMPORT + 1];
    let (status, erro) = c.importar(&id, &token, &grande).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{erro}");
    assert_eq!(codigo(&erro), "payload_too_large");
    assert!(
        erro["message"].as_str().unwrap_or("").contains("limite"),
        "{erro}"
    );
}

#[tokio::test]
async fn com_modelo_instalado_a_base_importada_fica_buscavel() {
    if !modelo_instalado() {
        // Sem o modelo nesta máquina: o caminho "sem modelo" está coberto no teste de importação.
        return;
    }
    let origem = Cenario::novo("origem-modelo").await;
    let (org, token_origem, _) = montar_org(&origem, "origem").await;
    let (_, arquivo) = origem.baixar_backup(&org, &token_origem, "").await;

    let c = Cenario::com_modelo("modelo", true).await;
    let destino = c.criar_org("Destino", "destino").await;
    let destino_id = destino["id"].as_str().unwrap().to_owned();
    let token_destino = c.criar_token(&destino_id, "destino").await;
    let (status, relatorio) = c
        .importar(&destino_id, &token_destino, &arquivo.bytes)
        .await;
    assert_eq!(status, StatusCode::OK, "{relatorio}");
    assert!(
        relatorio["aviso"].is_null(),
        "aviso inesperado: {relatorio}"
    );
    assert!(
        relatorio["indice"]["base_conhecimento"]["trechos"]
            .as_u64()
            .unwrap_or(0)
            > 0,
        "índice vazio depois da importação: {relatorio}"
    );
}
