use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get, post};
use poph_server::ServerState;
use poph_server::evolution::{EvolutionChannel, channels_from_config};
use poph_store::{NewAgent, NewOrg, Page, Store};
use poph_types::config::{EvolutionConfig, EvolutionInstanceConfig, GlobalTokenConfig};
use poph_types::{AgentId, HarnessConfig, IntegrationId, OrgId, Secret};
use rand_core::OsRng;
use reqwest::{Client, Method};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tracing_subscriber::fmt::MakeWriter;

const TOKEN_GLOBAL: &str = "pophg_teste-global-hr44b";
const EVO_KEY: &str = "chave-evo-hr44b-ABC123";
const WEBHOOK_SECRET: &str = "segredo-webhook-hr44b";
const SECRET_VAR: &str = "POPH_EVO_SEGREDO_HR44B";
const PHONE: &str = "5511999990000";

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

#[derive(Debug, Default)]
struct Instancia {
    exists: bool,
    open_conversation: bool,
    created: usize,
    pairings: usize,
    states: usize,
    deslogadas: usize,
    fail_on_create: bool,
}

#[derive(Clone, Default)]
struct FakeEvolution(Arc<Mutex<Instancia>>);

impl FakeEvolution {
    fn com<T>(&self, f: impl FnOnce(&mut Instancia) -> T) -> T {
        f(&mut self.0.lock().expect("estado da Evolution falsa"))
    }
    fn read<T>(&self, f: impl FnOnce(&Instancia) -> T) -> T {
        f(&self.0.lock().expect("estado da Evolution falsa"))
    }
}

fn authorized(headers: &HeaderMap) -> bool {
    headers.get("apikey").and_then(|value| value.to_str().ok()) == Some(EVO_KEY)
}

async fn criar(State(evo): State<FakeEvolution>, headers: HeaderMap) -> (StatusCode, Json<Value>) {
    if !authorized(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"message": "Unauthorized"})),
        );
    }
    evo.com(|instancia| {
        instancia.created += 1;
        if instancia.fail_on_create {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "message": format!("apikey {EVO_KEY} recusada para o número {PHONE}")
                })),
            );
        }
        if instancia.exists {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"message": "Instance \"loja\" already in use"})),
            );
        }
        instancia.exists = true;
        (
            StatusCode::CREATED,
            Json(json!({"instance": {"instanceName": "loja", "status": "created"}})),
        )
    })
}

async fn conectar(
    State(evo): State<FakeEvolution>,
    Path(instancia): Path<String>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if !authorized(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"message": "Unauthorized"})),
        );
    }
    evo.com(|estado| {
        estado.pairings += 1;
        if !estado.exists {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"message": "Instance does not exist"})),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "instance": instancia,
                "base64": "data:image/png;base64,QR-DE-TESTE",
                "code": "2@codigo-do-qr",
                "count": 3,
                "pairingCode": "ABCD-1234",
            })),
        )
    })
}

async fn connection_state(
    State(evo): State<FakeEvolution>,
    Path(instancia): Path<String>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if !authorized(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"message": "Unauthorized"})),
        );
    }
    evo.com(|estado| {
        estado.states += 1;
        if !estado.exists {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"message": "Instance does not exist"})),
            );
        }
        let open_flag = estado.open_conversation;
        (
            StatusCode::OK,
            Json(json!({
                "instance": {
                    "instanceName": instancia,
                    "state": if open_flag { "open" } else { "close" },
                }
            })),
        )
    })
}

async fn desconectar(
    State(evo): State<FakeEvolution>,
    Path(_instance): Path<String>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if !authorized(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"message": "Unauthorized"})),
        );
    }
    evo.com(|estado| {
        estado.deslogadas += 1;
        estado.open_conversation = false;
    });
    (StatusCode::OK, Json(json!({"status": "SUCCESS"})))
}

async fn start_fake_evolution() -> (String, FakeEvolution) {
    let evo = FakeEvolution::default();
    let router = Router::new()
        .route("/instance/create", post(criar))
        .route("/instance/connect/{instancia}", get(conectar))
        .route(
            "/instance/connectionState/{instancia}",
            get(connection_state),
        )
        .route("/instance/logout/{instancia}", delete(desconectar))
        .with_state(evo.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera da Evolution falsa");
    let endereco = listener.local_addr().expect("endereço da Evolution falsa");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{endereco}"), evo)
}

struct Scenario {
    base: String,
    client: Client,
    folder: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    integracao: IntegrationId,
    loose_integration: IntegrationId,
    sms_integration: IntegrationId,
    evo: FakeEvolution,
    logs: Arc<Mutex<Vec<u8>>>,
    desligar: Option<oneshot::Sender<()>>,
}

struct Response {
    status: StatusCode,
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
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let folder = std::env::temp_dir().join(format!(
            "poph-whatsapp-admin-{label}-{}-{stamp}",
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

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Loja WhatsApp".to_owned(),
                slug: "loja-whatsapp".to_owned(),
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
        let integracao: IntegrationId = store
            .create_integration(org, agente, "whatsapp".to_owned(), "active".to_owned())
            .expect("cria a integração")
            .id
            .parse()
            .expect("id da integração");
        let loose_integration: IntegrationId = store
            .create_integration(org, agente, "whatsapp".to_owned(), "active".to_owned())
            .expect("cria a integração solta")
            .id
            .parse()
            .expect("id da integração solta");
        let sms_integration: IntegrationId = store
            .create_integration(org, agente, "sms".to_owned(), "active".to_owned())
            .expect("cria a integração sms")
            .id
            .parse()
            .expect("id da integração sms");

        let (url_evo, evo) = start_fake_evolution().await;
        config.evolution = Some(EvolutionConfig {
            url_base: url_evo,
            instances: vec![EvolutionInstanceConfig {
                org_id: org.to_string(),
                instance: "loja".to_owned(),
                integration_id: integracao.to_string(),
                secret_env: SECRET_VAR.to_owned(),
            }],
        });
        config.validar().expect("configuração com Evolution válida");

        let channels: Vec<EvolutionChannel> = channels_from_config(
            &config,
            Some(Secret::new(EVO_KEY.to_owned())),
            &|variable| (variable == SECRET_VAR).then(|| WEBHOOK_SECRET.to_owned()),
        )
        .expect("canais da configuração");
        assert_eq!(channels.len(), 1, "só a instância declarada vira canal");

        let estado = ServerState::new(Arc::clone(&store), config)
            .expect("estado do servidor")
            .with_evolution_channels(channels);

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
            org,
            agente,
            integracao,
            loose_integration,
            sms_integration,
            evo,
            logs,
            desligar: Some(desligar),
        };
        scenario.wait_for_start().await;
        scenario
    }

    async fn wait_for_start(&self) {
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

    fn route(&self, integracao: IntegrationId, fim: &str) -> String {
        format!(
            "/admin/v1/orgs/{}/agents/{}/integrations/{}/whatsapp/{fim}",
            self.org, self.agente, integracao
        )
    }

    async fn ask(
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

    fn stored_state(&self, integracao: IntegrationId) -> String {
        self.store
            .get_integration(self.org, self.agente, integracao)
            .expect("integração")
            .connection_state
    }

    fn versao(&self, integracao: IntegrationId) -> u64 {
        self.store
            .get_integration(self.org, self.agente, integracao)
            .expect("integração")
            .version
    }

    async fn new_token(&self, org: OrgId, label: &str) -> String {
        let response = self
            .ask(
                Method::POST,
                &format!("/admin/v1/orgs/{org}/tokens"),
                TOKEN_GLOBAL,
                Some(json!({"label": label})),
                None,
            )
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.raw);
        response.body["value"]
            .as_str()
            .expect("valor do token")
            .to_owned()
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

#[tokio::test]
async fn pairing_returns_qr_and_stores_state() {
    let c = Scenario::new("pareamento").await;
    let route = c.route(c.integracao, "qrcode");

    let without_if_match = c.ask(Method::POST, &route, TOKEN_GLOBAL, None, None).await;
    assert_eq!(without_if_match.status, StatusCode::PRECONDITION_REQUIRED);
    assert_eq!(code(&without_if_match.body), "precondition_required");
    assert_eq!(c.evo.read(|e| e.created), 0, "nada foi criado na Evolution");

    let response = c
        .ask(
            Method::POST,
            &route,
            TOKEN_GLOBAL,
            Some(json!({"number": PHONE})),
            Some(1),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.raw);
    assert_eq!(response.body["instance"], "loja");
    assert_eq!(response.body["state"], "connecting");
    assert_eq!(response.body["connection_state"], "awaiting_scan");
    assert_eq!(
        response.body["qrcode"]["base64"],
        "data:image/png;base64,QR-DE-TESTE"
    );
    assert_eq!(response.body["qrcode"]["code"], "2@codigo-do-qr");
    assert_eq!(response.body["qrcode"]["count"], 3);
    assert_eq!(response.body["pairing_code"], "ABCD-1234");
    assert_eq!(
        response.body["expira_em"],
        Value::Null,
        "a Evolution não informa prazo do QR"
    );
    assert!(!response.raw.contains(EVO_KEY), "a chave não pode sair");
    assert!(!response.raw.contains(PHONE), "o número não pode sair");

    assert_eq!(c.stored_state(c.integracao), "awaiting_scan");
    assert_eq!(c.versao(c.integracao), 1, "estado não é edição de versão");

    let records = c
        .store
        .audit()
        .list(c.org, Page::default())
        .expect("auditoria da org");
    let line = records
        .iter()
        .find(|registro| registro.acao == "whatsapp_qrcode")
        .expect("registro de auditoria do pareamento");
    assert_eq!(line.entity, "integration");
    assert_eq!(
        line.entity_id.as_deref(),
        Some(c.integracao.to_string().as_str())
    );
    assert!(
        !format!("{line:?}").contains("QR-DE-TESTE"),
        "o QR não pode ir para a auditoria"
    );
    assert_eq!(c.evo.read(|e| e.created), 1);
    assert_eq!(c.evo.read(|e| e.pairings), 1);

    let denovo = c
        .ask(Method::POST, &route, TOKEN_GLOBAL, None, Some(1))
        .await;
    assert_eq!(denovo.status, StatusCode::OK, "{}", denovo.raw);
    assert_eq!(c.evo.read(|e| e.created), 2, "tentou criar de novo");
    assert_eq!(c.evo.read(|e| e.pairings), 2);

    c.evo.com(|e| e.open_conversation = true);
    let connected = c
        .ask(Method::POST, &route, TOKEN_GLOBAL, None, Some(1))
        .await;
    assert_eq!(connected.status, StatusCode::OK, "{}", connected.raw);
    assert_eq!(connected.body["state"], "open");
    assert_eq!(connected.body["connection_state"], "connected");
    assert_eq!(connected.body["qrcode"], Value::Null);
    assert_eq!(c.stored_state(c.integracao), "connected");
}

#[tokio::test]
async fn live_state_reflects_in_integration() {
    let c = Scenario::new("estado").await;
    let route = c.route(c.integracao, "state");
    c.evo.com(|e| {
        e.exists = true;
        e.open_conversation = false;
    });

    let closed = c.ask(Method::GET, &route, TOKEN_GLOBAL, None, None).await;
    assert_eq!(closed.status, StatusCode::OK, "{}", closed.raw);
    assert_eq!(closed.body["state"], "close");
    assert_eq!(closed.body["connection_state"], "disconnected");
    assert_eq!(c.stored_state(c.integracao), "disconnected");

    c.evo.com(|e| e.open_conversation = true);
    let open_flag = c.ask(Method::GET, &route, TOKEN_GLOBAL, None, None).await;
    assert_eq!(open_flag.status, StatusCode::OK, "{}", open_flag.raw);
    assert_eq!(open_flag.body["state"], "open");
    assert_eq!(open_flag.body["connection_state"], "connected");
    assert_eq!(open_flag.body["integration_id"], c.integracao.to_string());
    assert_eq!(c.stored_state(c.integracao), "connected");
    assert_eq!(c.versao(c.integracao), 1, "ler estado não muda a versão");

    c.evo.com(|e| e.exists = false);
    let without_instance = c.ask(Method::GET, &route, TOKEN_GLOBAL, None, None).await;
    assert_eq!(without_instance.status, StatusCode::BAD_GATEWAY);
    assert_eq!(code(&without_instance.body), "evolution_recusou");
    assert_eq!(without_instance.body["details"]["evolution_status"], 404);
    assert_eq!(
        c.stored_state(c.integracao),
        "connected",
        "erro não sobrescreve o último estado bom"
    );
}

#[tokio::test]
async fn disconnect_requires_if_match_and_calls_logout() {
    let c = Scenario::new("desconectar").await;
    let route = c.route(c.integracao, "disconnect");
    c.evo.com(|e| {
        e.exists = true;
        e.open_conversation = true;
    });

    let without_if_match = c.ask(Method::POST, &route, TOKEN_GLOBAL, None, None).await;
    assert_eq!(without_if_match.status, StatusCode::PRECONDITION_REQUIRED);
    assert_eq!(c.evo.read(|e| e.deslogadas), 0);

    c.store
        .update_integration(
            c.org,
            c.agente,
            c.integracao,
            poph_store::IntegrationChange {
                version: 1,
                status: Some("inactive"),
                ..Default::default()
            },
        )
        .expect("edição pela API/CLI concorrente");
    assert_eq!(c.versao(c.integracao), 2);

    let old = c
        .ask(Method::POST, &route, TOKEN_GLOBAL, None, Some(1))
        .await;
    assert_eq!(old.status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(code(&old.body), "version_conflict");
    assert_eq!(old.body["details"]["expected_version"], 1);
    assert_eq!(old.body["details"]["current_version"], 2);
    assert_eq!(c.evo.read(|e| e.deslogadas), 0, "nada foi desconectado");

    let certo = c
        .ask(Method::POST, &route, TOKEN_GLOBAL, None, Some(2))
        .await;
    assert_eq!(certo.status, StatusCode::OK, "{}", certo.raw);
    assert_eq!(certo.body["connection_state"], "disconnected");
    assert_eq!(certo.body["disconnected"], true);
    assert_eq!(c.evo.read(|e| e.deslogadas), 1);
    assert!(
        !c.evo.read(|e| e.open_conversation),
        "o aparelho continua conectado"
    );
    assert_eq!(c.stored_state(c.integracao), "disconnected");
    assert_eq!(c.versao(c.integracao), 2, "desconectar não sobe a versão");
    let records = c
        .store
        .audit()
        .list(c.org, Page::default())
        .expect("auditoria da org");
    assert!(
        records
            .iter()
            .any(|registro| registro.acao == "whatsapp_disconnect"
                && registro.entity == "integration"),
        "a desconexão precisa ficar na auditoria"
    );
}

#[tokio::test]
async fn missing_integration_gives_404_on_three_routes() {
    let c = Scenario::new("inexistente").await;
    let solta = IntegrationId::generate();
    for (method, fim) in [
        (Method::POST, "qrcode"),
        (Method::GET, "state"),
        (Method::POST, "disconnect"),
    ] {
        let response = c
            .ask(method, &c.route(solta, fim), TOKEN_GLOBAL, None, Some(1))
            .await;
        assert_eq!(
            response.status,
            StatusCode::NOT_FOUND,
            "rota `{fim}`: {}",
            response.raw
        );
        assert_eq!(code(&response.body), "not_found");
    }
    assert_eq!(c.evo.read(|e| e.created + e.deslogadas), 0);
}

#[tokio::test]
async fn undeclared_instance_integration_gives_409() {
    let c = Scenario::new("sem-instancia").await;
    for (method, fim) in [
        (Method::POST, "qrcode"),
        (Method::GET, "state"),
        (Method::POST, "disconnect"),
    ] {
        let response = c
            .ask(
                method,
                &c.route(c.loose_integration, fim),
                TOKEN_GLOBAL,
                None,
                Some(1),
            )
            .await;
        assert_eq!(
            response.status,
            StatusCode::CONFLICT,
            "rota `{fim}`: {}",
            response.raw
        );
        assert_eq!(code(&response.body), "whatsapp_nao_configurado");
    }
    assert_eq!(c.evo.read(|e| e.created + e.deslogadas), 0);
}

#[tokio::test]
async fn non_whatsapp_integration_gives_409() {
    let c = Scenario::new("sms").await;
    let response = c
        .ask(
            Method::POST,
            &c.route(c.sms_integration, "qrcode"),
            TOKEN_GLOBAL,
            None,
            Some(1),
        )
        .await;
    assert_eq!(response.status, StatusCode::CONFLICT);
    assert_eq!(code(&response.body), "integracao_nao_e_whatsapp");
    assert_eq!(response.body["details"]["kind"], "sms");
}

#[tokio::test]
async fn secret_and_number_do_not_leak_in_response_or_log() {
    let c = Scenario::new("segredo").await;
    c.evo.com(|e| e.fail_on_create = true);
    let response = c
        .ask(
            Method::POST,
            &c.route(c.integracao, "qrcode"),
            TOKEN_GLOBAL,
            Some(json!({"number": PHONE})),
            Some(1),
        )
        .await;
    assert_eq!(response.status, StatusCode::BAD_GATEWAY, "{}", response.raw);
    assert_eq!(code(&response.body), "evolution_recusou");
    assert!(
        !response.raw.contains(EVO_KEY),
        "a chave vazou na resposta: {}",
        response.raw
    );
    assert!(
        !response.raw.contains(PHONE),
        "o número vazou na resposta: {}",
        response.raw
    );
    let logs = String::from_utf8_lossy(&c.logs.lock().unwrap().clone()).into_owned();
    assert!(!logs.contains(EVO_KEY), "a chave vazou no log do servidor");
    assert!(!logs.contains(PHONE), "o número vazou no log do servidor");
    assert_eq!(c.stored_state(c.integracao), "disconnected");
}

#[tokio::test]
async fn other_org_token_gives_403() {
    let c = Scenario::new("outra-org").await;
    let other = c
        .store
        .orgs()
        .create(NewOrg {
            name: "Outra Loja".to_owned(),
            slug: "outra-loja".to_owned(),
            config_json: String::new(),
        })
        .expect("cria a outra org")
        .id;
    let token = c.new_token(other, "outra org").await;
    for (method, fim) in [
        (Method::POST, "qrcode"),
        (Method::GET, "state"),
        (Method::POST, "disconnect"),
    ] {
        let response = c
            .ask(method, &c.route(c.integracao, fim), &token, None, Some(1))
            .await;
        assert_eq!(
            response.status,
            StatusCode::FORBIDDEN,
            "rota `{fim}`: {}",
            response.raw
        );
    }
    assert_eq!(c.evo.read(|e| e.created + e.deslogadas), 0);
}
