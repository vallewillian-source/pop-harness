use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::{CipherKey, ServerState};
use poph_store::{NewAgent, NewOrg, Store};
use poph_types::config::GlobalTokenConfig;
use poph_types::{AgentId, DeclaredPlugin, HarnessConfig, OrgId};
use rand_core::OsRng;
use reqwest::{Client, Method};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tracing_subscriber::fmt::MakeWriter;

const TOKEN_GLOBAL: &str = "pophg_teste-global-hr44c";
const CIPHER_KEY: &str = "chave-de-cifra-do-teste-hr44c";
const SECRET: &str = "segredo-que-nao-pode-vazar-hr44c";

#[derive(Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();

fn captured_logs() -> Arc<Mutex<Vec<u8>>> {
    LOGS.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let _ = tracing_subscriber::fmt()
            .with_writer(Capture(Arc::clone(&buffer)))
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .try_init();
        buffer
    })
    .clone()
}

fn fake_manifest() -> Value {
    json!({
        "nome": "Plugin de teste",
        "kind": "fake",
        "versao": "0.1.0-teste",
        "protocolo": 1,
        "capacidades": { "text": true },
        "limites": {},
        "configuracao": {
            "type": "object",
            "properties": {
                "remetente": { "type": "string", "minLength": 1, "title": "Remetente" },
                "modo": { "type": "string", "enum": ["inverter", "ecoar"] },
                "tentativas": { "type": "integer", "minimum": 1, "maximum": 5 },
                "avisar": { "type": "boolean" }
            },
            "required": ["remetente"]
        },
        "segredos": {
            "type": "object",
            "properties": {
                "token": { "type": "string", "title": "Token" }
            },
            "required": ["token"]
        }
    })
}

fn write_plugin(folder: &std::path::Path) -> PathBuf {
    let counter = folder.join("subidas.txt");
    let manifesto = serde_json::to_string(&fake_manifest()).expect("manifesto em JSON");
    let script = folder.join("plugin_falso.py");
    let code = format!(
        r#"import io, json, sys
with io.open({counter:?}, "a", encoding="utf-8") as registro:
    registro.write("subiu\n")
manifesto = json.loads({manifesto:?})
for linha in sys.stdin:
    linha = linha.strip()
    if not linha:
        continue
    mensagem = json.loads(linha)
    if mensagem.get("tipo") == "handshake":
        print(json.dumps({{"tipo": "handshake_ok", "protocolo": 1, "manifesto": manifesto}}), flush=True)
    elif mensagem.get("tipo") == "encerrar":
        sys.exit(0)
    # Qualquer outra mensagem é ignorada: este falso só serve para o manifesto.
"#,
        counter = counter.display().to_string(),
        manifesto = manifesto,
    );
    fs::write(&script, code).expect("plugin falso");
    script
}

struct Scenario {
    base: String,
    client: Client,
    folder: PathBuf,
    org: OrgId,
    agente: AgentId,
    counter: PathBuf,
    logs: Arc<Mutex<Vec<u8>>>,
    desligar: Option<oneshot::Sender<()>>,
}

struct Response {
    status: reqwest::StatusCode,
    body: Value,
    raw: String,
}

fn hash_argon2(value: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

impl Scenario {
    async fn new(label: &str) -> Self {
        let logs = captured_logs();
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let folder = std::env::temp_dir().join(format!(
            "poph-manifestos-{label}-{}-{timestamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(folder.join("dados")).expect("pasta de dados");
        let script = write_plugin(&folder);

        let mut config = HarnessConfig::default();
        config.data.directory = folder.join("dados");
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.plugins.integrations = vec![
            DeclaredPlugin {
                kind: "fake".to_owned(),
                program: PathBuf::from("python3"),
                arguments: vec![script.display().to_string()],
            },
            DeclaredPlugin {
                kind: "quebrado".to_owned(),
                program: PathBuf::from("/bin/false"),
                arguments: Vec::new(),
            },
        ];
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Org do manifesto".to_owned(),
                slug: "org-do-manifesto".to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Atendente".to_owned(),
                    slug: "atendente".to_owned(),
                    ..Default::default()
                },
            )
            .expect("cria o agente")
            .id;

        let estado = ServerState::new(Arc::clone(&store), config)
            .expect("estado do servidor")
            .with_encryption_key(Some(CipherKey::derive(CIPHER_KEY).expect("chave de cifra")));
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
                .timeout(Duration::from_secs(30))
                .build()
                .expect("cliente HTTP"),
            folder: folder.clone(),
            org,
            agente,
            counter: folder.join("subidas.txt"),
            logs,
            desligar: Some(desligar),
        };
        scenario.wait_boot().await;
        scenario
    }

    async fn wait_boot(&self) {
        for _ in 0..500 {
            if let Ok(r) = self
                .client
                .get(format!("{}/healthz", self.base))
                .send()
                .await
                && r.status().is_success()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("servidor não subiu");
    }

    fn integrations_route(&self) -> String {
        format!(
            "/admin/v1/orgs/{}/agents/{}/integrations",
            self.org, self.agente
        )
    }

    fn manifest_route(&self, kind: &str) -> String {
        format!("/admin/v1/orgs/{}/manifests/{kind}", self.org)
    }

    fn list_route(&self) -> String {
        format!("/admin/v1/orgs/{}/manifests", self.org)
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Option<Value>,
        if_match: Option<u64>,
    ) -> Response {
        let mut request_body = self.client.request(method, format!("{}{path}", self.base));
        if !token.is_empty() {
            request_body = request_body.bearer_auth(token);
        }
        if let Some(versao) = if_match {
            request_body = request_body.header("If-Match", versao.to_string());
        }
        if let Some(value) = body {
            request_body = request_body.json(&value);
        }
        let response = request_body.send().await.expect("pedido HTTP");
        let status = response.status();
        let raw = response.text().await.unwrap_or_default();
        let body = serde_json::from_str(&raw).unwrap_or(Value::Null);
        Response { status, body, raw }
    }

    fn plugin_starts(&self) -> usize {
        fs::read_to_string(&self.counter)
            .map(|texto| texto.lines().count())
            .unwrap_or(0)
    }

    fn bytes_on_disk(&self) -> Vec<u8> {
        let mut all = Vec::new();
        let folder = self.folder.join("dados");
        for input in fs::read_dir(&folder).expect("pasta de dados") {
            let path = input.expect("entrada").path();
            if path.is_file() {
                all.extend_from_slice(&fs::read(&path).unwrap_or_default());
            }
        }
        assert!(!all.is_empty(), "a pasta de dados não tem arquivo");
        all
    }

    fn logs(&self) -> String {
        String::from_utf8_lossy(&self.logs.lock().unwrap()).to_string()
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
    body["code"].as_str().unwrap_or_default()
}

#[tokio::test]
async fn manifest_comes_from_handshake_and_is_cached() {
    let c = Scenario::new("manifesto").await;
    let (status, body) = {
        let r = c
            .request(
                Method::GET,
                &c.manifest_route("fake"),
                TOKEN_GLOBAL,
                None,
                None,
            )
            .await;
        (r.status, r.body)
    };
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    assert_eq!(body["kind"], "fake");
    assert_eq!(body["nome"], "Plugin de teste");
    assert_eq!(
        body["configuracao"]["required"],
        json!(["remetente"]),
        "o esquema da configuração é o do plugin"
    );
    assert_eq!(body["segredos"]["required"], json!(["token"]));
    assert_eq!(
        c.plugin_starts(),
        1,
        "a primeira leitura sobe o plugin uma vez"
    );

    let r = c
        .request(
            Method::GET,
            &c.manifest_route("fake"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(r.status, reqwest::StatusCode::OK);
    assert_eq!(c.plugin_starts(), 1, "o manifesto vem do cache");
}

#[tokio::test]
async fn type_without_plugin_is_404_and_broken_plugin_is_503() {
    let c = Scenario::new("falhas-manifesto").await;
    let r = c
        .request(
            Method::GET,
            &c.manifest_route("sms"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(r.status, reqwest::StatusCode::NOT_FOUND, "{}", r.raw);
    assert_eq!(code(&r.body), "not_found");

    let r = c
        .request(
            Method::GET,
            &c.manifest_route("quebrado"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(
        r.status,
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        r.raw
    );
    assert_eq!(code(&r.body), "manifesto_indisponivel");
    assert!(
        !r.raw.contains("/bin/false") && !r.raw.contains("No such file"),
        "a resposta não expõe caminho nem erro do sistema: {}",
        r.raw
    );

    let r = c
        .request(
            Method::POST,
            &c.integrations_route(),
            TOKEN_GLOBAL,
            Some(json!({"kind": "quebrado", "config_json": {"remetente": "Loja"}})),
            None,
        )
        .await;
    assert_eq!(
        r.status,
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        r.raw
    );
    assert_eq!(code(&r.body), "manifesto_indisponivel");
}

#[tokio::test]
async fn create_stores_config_and_encrypted_secret_without_exposing_it() {
    let c = Scenario::new("criacao").await;
    let r = c
        .request(
            Method::POST,
            &c.integrations_route(),
            TOKEN_GLOBAL,
            Some(json!({
                "kind": "fake",
                "config_json": {"remetente": "Loja", "modo": "inverter", "tentativas": 2},
                "secrets": {"token": SECRET}
            })),
            None,
        )
        .await;
    assert_eq!(r.status, reqwest::StatusCode::CREATED, "{}", r.raw);
    assert_eq!(r.body["version"], 1, "criação não gasta versão extra");
    assert_eq!(r.body["configuracao"]["remetente"], "Loja");
    assert_eq!(r.body["tem_segredo"], true);
    assert_eq!(r.body["segredos_preenchidos"], json!(["token"]));
    assert!(
        !r.raw.contains(SECRET),
        "o segredo não volta na resposta de criação: {}",
        r.raw
    );
    let id = r.body["id"].as_str().expect("id").to_owned();

    let detail = c
        .request(
            Method::GET,
            &format!("{}/{id}", c.integrations_route()),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(detail.status, reqwest::StatusCode::OK);
    assert_eq!(detail.body["configuracao"]["tentativas"], 2);
    assert!(!detail.raw.contains(SECRET), "{}", detail.raw);
    let list = c
        .request(
            Method::GET,
            &c.integrations_route(),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert!(!list.raw.contains(SECRET), "{}", list.raw);

    let database = c.bytes_on_disk();
    let contem = database
        .windows(SECRET.len())
        .any(|window| window == SECRET.as_bytes());
    assert!(!contem, "o segredo não pode estar em claro em `harness.db`");

    assert!(
        !c.logs().contains(SECRET),
        "o segredo não pode ir para o log"
    );
}

#[tokio::test]
async fn config_and_secret_are_validated_field_by_field() {
    let c = Scenario::new("validacao").await;
    let cases: Vec<(Value, &str)> = vec![
        (
            json!({"remetente": "Loja", "tentativas": "duas"}),
            "campo `config_json.tentativas`",
        ),
        (
            json!({"remetente": "Loja", "modo": "outro"}),
            "campo `config_json.modo`",
        ),
        (
            json!({"remetente": "Loja", "inventado": 1}),
            "campo `config_json.inventado`",
        ),
        (json!({"modo": "inverter"}), "campo `config_json.remetente`"),
    ];
    for (configuracao, campo) in cases {
        let r = c
            .request(
                Method::POST,
                &c.integrations_route(),
                TOKEN_GLOBAL,
                Some(
                    json!({"kind": "fake", "config_json": configuracao, "secrets": {"token": "x"}}),
                ),
                None,
            )
            .await;
        assert_eq!(r.status, reqwest::StatusCode::BAD_REQUEST, "{}", r.raw);
        assert_eq!(code(&r.body), "validation_error", "{}", r.raw);
        assert!(
            r.body["message"]
                .as_str()
                .unwrap_or_default()
                .contains(campo),
            "esperava {campo}: {}",
            r.raw
        );
        assert_eq!(r.body["details"]["field"], &campo[7..campo.len() - 1]);
    }

    let r = c
        .request(
            Method::POST,
            &c.integrations_route(),
            TOKEN_GLOBAL,
            Some(json!({"kind": "fake", "config_json": {"remetente": "Loja"}})),
            None,
        )
        .await;
    assert_eq!(r.status, reqwest::StatusCode::BAD_REQUEST, "{}", r.raw);
    assert_eq!(r.body["details"]["field"], "secrets.token");

    let r = c
        .request(
            Method::POST,
            &c.integrations_route(),
            TOKEN_GLOBAL,
            Some(json!({"kind": "fake"})),
            None,
        )
        .await;
    assert_eq!(r.status, reqwest::StatusCode::BAD_REQUEST, "{}", r.raw);
    assert_eq!(r.body["details"]["field"], "config_json.remetente");

    let list = c
        .request(
            Method::GET,
            &c.integrations_route(),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(
        list.body["items"].as_array().map(Vec::len),
        Some(0),
        "{}",
        list.raw
    );
}

#[tokio::test]
async fn config_without_manifest_stays_refused() {
    let c = Scenario::new("sem-manifesto").await;
    let r = c
        .request(
            Method::POST,
            &c.integrations_route(),
            TOKEN_GLOBAL,
            Some(json!({"kind": "sms", "config_json": {"qualquer": 1}})),
            None,
        )
        .await;
    assert_eq!(
        r.status,
        reqwest::StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        r.raw
    );
    assert_eq!(code(&r.body), "manifest_indisponivel");

    let r = c
        .request(
            Method::POST,
            &c.integrations_route(),
            TOKEN_GLOBAL,
            Some(json!({"kind": "sms"})),
            None,
        )
        .await;
    assert_eq!(r.status, reqwest::StatusCode::CREATED, "{}", r.raw);
    assert_eq!(r.body["configuracao"], json!({}));
    assert_eq!(r.body["tem_segredo"], false);
}

#[tokio::test]
async fn patch_replaces_config_merges_secret_and_requires_if_match() {
    let c = Scenario::new("patch").await;
    let created = c
        .request(
            Method::POST,
            &c.integrations_route(),
            TOKEN_GLOBAL,
            Some(json!({
                "kind": "fake",
                "config_json": {"remetente": "Loja"},
                "secrets": {"token": SECRET}
            })),
            None,
        )
        .await;
    assert_eq!(
        created.status,
        reqwest::StatusCode::CREATED,
        "{}",
        created.raw
    );
    let id = created.body["id"].as_str().expect("id").to_owned();
    let route = format!("{}/{id}", c.integrations_route());

    let r = c
        .request(
            Method::PATCH,
            &route,
            TOKEN_GLOBAL,
            Some(json!({"config_json": {"remetente": "Nova"}})),
            None,
        )
        .await;
    assert_eq!(
        r.status,
        reqwest::StatusCode::PRECONDITION_REQUIRED,
        "{}",
        r.raw
    );
    let r = c
        .request(
            Method::PATCH,
            &route,
            TOKEN_GLOBAL,
            Some(json!({"config_json": {"remetente": "Nova"}})),
            Some(9),
        )
        .await;
    assert_eq!(
        r.status,
        reqwest::StatusCode::PRECONDITION_FAILED,
        "{}",
        r.raw
    );
    assert_eq!(code(&r.body), "version_conflict");

    let r = c
        .request(
            Method::PATCH,
            &route,
            TOKEN_GLOBAL,
            Some(json!({"config_json": {"remetente": "Nova", "avisar": true}})),
            Some(1),
        )
        .await;
    assert_eq!(r.status, reqwest::StatusCode::OK, "{}", r.raw);
    assert_eq!(r.body["version"], 2);
    assert_eq!(
        r.body["configuracao"],
        json!({"remetente": "Nova", "avisar": true})
    );
    assert_eq!(
        r.body["segredos_preenchidos"],
        json!(["token"]),
        "o PATCH da configuração não mexe no segredo"
    );

    let r = c
        .request(
            Method::PATCH,
            &route,
            TOKEN_GLOBAL,
            Some(json!({"secrets": {"token": "outro-valor"}})),
            Some(2),
        )
        .await;
    assert_eq!(r.status, reqwest::StatusCode::OK, "{}", r.raw);
    assert_eq!(r.body["version"], 3);
    assert_eq!(r.body["segredos_preenchidos"], json!(["token"]));
    assert!(!r.raw.contains("outro-valor"), "{}", r.raw);

    let r = c
        .request(
            Method::PATCH,
            &route,
            TOKEN_GLOBAL,
            Some(json!({"secrets": {"token": ""}})),
            Some(3),
        )
        .await;
    assert_eq!(r.status, reqwest::StatusCode::BAD_REQUEST, "{}", r.raw);
    assert_eq!(r.body["details"]["field"], "secrets.token");

    let detail = c
        .request(Method::GET, &route, TOKEN_GLOBAL, None, None)
        .await;
    assert_eq!(detail.body["version"], 3);
    assert_eq!(detail.body["configuracao"]["remetente"], "Nova");
}

#[tokio::test]
async fn other_org_token_cannot_read_manifest() {
    let c = Scenario::new("isolamento").await;
    let org = c
        .request(
            Method::POST,
            "/admin/v1/orgs",
            TOKEN_GLOBAL,
            Some(json!({"name": "Outra", "slug": "outra"})),
            None,
        )
        .await;
    assert_eq!(org.status, reqwest::StatusCode::CREATED, "{}", org.raw);
    let id = org.body["id"].as_str().expect("id da org");
    let token = c
        .request(
            Method::POST,
            &format!("/admin/v1/orgs/{id}/tokens"),
            TOKEN_GLOBAL,
            Some(json!({"label": "outra"})),
            None,
        )
        .await;
    assert_eq!(token.status, reqwest::StatusCode::CREATED, "{}", token.raw);
    let value = token.body["value"].as_str().expect("valor").to_owned();
    let r = c
        .request(Method::GET, &c.manifest_route("fake"), &value, None, None)
        .await;
    assert_eq!(r.status, reqwest::StatusCode::FORBIDDEN, "{}", r.raw);
    assert_eq!(c.plugin_starts(), 0, "nada subiu para um pedido barrado");
}

#[test]
fn public_manifest_types_serialize() {
    let manifesto = fake_manifest();
    let texto = serde_json::to_string(&manifesto).expect("JSON");
    assert!(texto.contains("configuracao"));
}

#[tokio::test]
async fn list_returns_declared_types_and_flags_broken_one() {
    let c = Scenario::new("lista").await;
    let r = c
        .request(Method::GET, &c.list_route(), TOKEN_GLOBAL, None, None)
        .await;
    assert_eq!(r.status, reqwest::StatusCode::OK, "{}", r.raw);
    let items = r.body["items"].as_array().expect("lista de itens");
    assert_eq!(items.len(), 2, "os dois tipos declarados: {}", r.raw);

    assert_eq!(items[0]["kind"], "fake");
    assert_eq!(items[0]["situacao"], "ok");
    assert_eq!(items[0]["nome"], "Plugin de teste");
    assert_eq!(items[0]["versao"], "0.1.0-teste");
    assert!(
        items[0].get("mensagem").is_none(),
        "plugin ok não traz frase de erro: {}",
        r.raw
    );

    assert_eq!(items[1]["kind"], "quebrado");
    assert_eq!(items[1]["situacao"], "indisponivel");
    assert!(items[1].get("nome").is_none(), "{}", r.raw);
    assert!(items[1].get("versao").is_none(), "{}", r.raw);
    assert!(
        items[1]["mensagem"]
            .as_str()
            .unwrap_or_default()
            .contains("não respondeu ao handshake"),
        "o item quebrado traz a frase de operação: {}",
        r.raw
    );

    for forbidden in [
        "/bin/false",
        "python3",
        "plugin_falso.py",
        "\"programa\"",
        "\"argumentos\"",
        CIPHER_KEY,
        SECRET,
    ] {
        assert!(
            !r.raw.contains(forbidden),
            "a lista não pode expor `{forbidden}`: {}",
            r.raw
        );
    }

    let r = c
        .request(Method::GET, &c.list_route(), TOKEN_GLOBAL, None, None)
        .await;
    assert_eq!(r.status, reqwest::StatusCode::OK, "{}", r.raw);
    assert_eq!(
        c.plugin_starts(),
        1,
        "a listagem reaproveita o cache da consulta por tipo"
    );
}

#[tokio::test]
async fn list_requires_token_and_respects_token_org() {
    let c = Scenario::new("lista-isolamento").await;
    let no_token = c
        .request(Method::GET, &c.list_route(), "", None, None)
        .await;
    assert_eq!(
        no_token.status,
        reqwest::StatusCode::UNAUTHORIZED,
        "{}",
        no_token.raw
    );

    let org = c
        .request(
            Method::POST,
            "/admin/v1/orgs",
            TOKEN_GLOBAL,
            Some(json!({"name": "Outra", "slug": "outra"})),
            None,
        )
        .await;
    assert_eq!(org.status, reqwest::StatusCode::CREATED, "{}", org.raw);
    let id = org.body["id"].as_str().expect("id da org");
    let token = c
        .request(
            Method::POST,
            &format!("/admin/v1/orgs/{id}/tokens"),
            TOKEN_GLOBAL,
            Some(json!({"label": "outra"})),
            None,
        )
        .await;
    assert_eq!(token.status, reqwest::StatusCode::CREATED, "{}", token.raw);
    let value = token.body["value"].as_str().expect("valor").to_owned();

    let r = c
        .request(Method::GET, &c.list_route(), &value, None, None)
        .await;
    assert_eq!(r.status, reqwest::StatusCode::FORBIDDEN, "{}", r.raw);
    assert_eq!(c.plugin_starts(), 0, "nada subiu para um pedido barrado");
}
