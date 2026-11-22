//! WhatsApp administrativo (HR-44b): pareamento com QR, estado ao vivo e desconexão, de ponta a
//! ponta.
//!
//! Servidor de verdade (porta efêmera) sobre banco temporário, cliente HTTP de verdade e uma
//! Evolution falsa em `127.0.0.1` que guarda o próprio estado (instância criada, aparelho aberto).
//! Nada de rede externa e nenhum número de WhatsApp.
//!
//! O que se prova: gerar QR cria a instância quando ela não existe e reaproveita quando existe;
//! a resposta traz QR, rodada e código de pareamento, com `expira_em: null` (a Evolution não
//! informa prazo); o estado ao vivo vira `connected`/`awaiting_scan`/`disconnected` e grava na
//! integração sem subir a versão; desconectar chama o logout e grava `disconnected`; as duas
//! escritas exigem `If-Match` (428 sem ele, 412 com versão velha); integração inexistente dá 404,
//! integração que não é WhatsApp e integração sem instância declarada dão 409; a chave da Evolution
//! e o número do cliente nunca aparecem na resposta nem no log; token de outra org dá 403.

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
use poph_server::Estado;
use poph_server::evolution::{CanalEvolution, canais_da_configuracao};
use poph_store::{NovaOrg, NovoAgente, Pagina, Store};
use poph_types::config::{EvolutionConfig, InstanciaEvolutionConfig, TokenGlobalConfig};
use poph_types::{AgentId, HarnessConfig, IntegrationId, OrgId, Segredo};
use rand_core::OsRng;
use reqwest::{Client, Method};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tracing_subscriber::fmt::MakeWriter;

/// Token global dos testes (o servidor guarda só o hash).
const TOKEN_GLOBAL: &str = "pophg_teste-global-hr44b";
/// Chave da Evolution usada nos testes (só no servidor falso).
const CHAVE_EVO: &str = "chave-evo-hr44b-ABC123";
/// Segredo do webhook (o mesmo esquema da HR-26b; aqui só para resolver o canal).
const SEGREDO_WEBHOOK: &str = "segredo-webhook-hr44b";
/// Variável de ambiente do segredo do webhook.
const VARIAVEL_SEGREDO: &str = "POPH_EVO_SEGREDO_HR44B";
/// Número do cliente fictício, usado no pedido de pareamento por número.
const TELEFONE: &str = "5511999990000";

// ───────────────────────── captura de logs (para provar o que NÃO sai) ─────────────────────────

#[derive(Clone)]
struct Captura(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captura {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Captura {
    type Writer = Captura;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();

fn logs_capturados() -> Arc<Mutex<Vec<u8>>> {
    LOGS.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let _ = tracing_subscriber::fmt()
            .with_writer(Captura(Arc::clone(&buffer)))
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .try_init();
        buffer
    })
    .clone()
}

// ───────────────────────── Evolution falsa ─────────────────────────

/// Estado da instância falsa, atrás de um mutex.
#[derive(Debug, Default)]
struct Instancia {
    existe: bool,
    aberta: bool,
    criadas: usize,
    pareamentos: usize,
    estados: usize,
    deslogadas: usize,
    /// Quando `true`, criar instância responde 500 ecoando a chave e o número (prova de limpeza).
    falhar_ao_criar: bool,
}

#[derive(Clone, Default)]
struct EvolucaoFalsa(Arc<Mutex<Instancia>>);

impl EvolucaoFalsa {
    fn com<T>(&self, f: impl FnOnce(&mut Instancia) -> T) -> T {
        f(&mut self.0.lock().expect("estado da Evolution falsa"))
    }
    fn ler<T>(&self, f: impl FnOnce(&Instancia) -> T) -> T {
        f(&self.0.lock().expect("estado da Evolution falsa"))
    }
}

/// A chave vai no cabeçalho `apikey`: sem ela certa, nada acontece (como na Evolution de verdade).
fn autorizado(cabecalhos: &HeaderMap) -> bool {
    cabecalhos
        .get("apikey")
        .and_then(|valor| valor.to_str().ok())
        == Some(CHAVE_EVO)
}

async fn criar(
    State(evo): State<EvolucaoFalsa>,
    cabecalhos: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if !autorizado(&cabecalhos) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"message": "Unauthorized"})),
        );
    }
    evo.com(|instancia| {
        instancia.criadas += 1;
        if instancia.falhar_ao_criar {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "message": format!("apikey {CHAVE_EVO} recusada para o número {TELEFONE}")
                })),
            );
        }
        if instancia.existe {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"message": "Instance \"loja\" already in use"})),
            );
        }
        instancia.existe = true;
        (
            StatusCode::CREATED,
            Json(json!({"instance": {"instanceName": "loja", "status": "created"}})),
        )
    })
}

async fn conectar(
    State(evo): State<EvolucaoFalsa>,
    Path(instancia): Path<String>,
    cabecalhos: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if !autorizado(&cabecalhos) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"message": "Unauthorized"})),
        );
    }
    evo.com(|estado| {
        estado.pareamentos += 1;
        if !estado.existe {
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

async fn estado_conexao(
    State(evo): State<EvolucaoFalsa>,
    Path(instancia): Path<String>,
    cabecalhos: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if !autorizado(&cabecalhos) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"message": "Unauthorized"})),
        );
    }
    evo.com(|estado| {
        estado.estados += 1;
        if !estado.existe {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"message": "Instance does not exist"})),
            );
        }
        let aberto = estado.aberta;
        (
            StatusCode::OK,
            Json(json!({
                "instance": {
                    "instanceName": instancia,
                    "state": if aberto { "open" } else { "close" },
                }
            })),
        )
    })
}

async fn desconectar(
    State(evo): State<EvolucaoFalsa>,
    Path(_instancia): Path<String>,
    cabecalhos: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if !autorizado(&cabecalhos) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"message": "Unauthorized"})),
        );
    }
    evo.com(|estado| {
        estado.deslogadas += 1;
        estado.aberta = false;
    });
    (StatusCode::OK, Json(json!({"status": "SUCCESS"})))
}

async fn subir_evolucao_falsa() -> (String, EvolucaoFalsa) {
    let evo = EvolucaoFalsa::default();
    let roteador = Router::new()
        .route("/instance/create", post(criar))
        .route("/instance/connect/{instancia}", get(conectar))
        .route("/instance/connectionState/{instancia}", get(estado_conexao))
        .route("/instance/logout/{instancia}", delete(desconectar))
        .with_state(evo.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera da Evolution falsa");
    let endereco = listener.local_addr().expect("endereço da Evolution falsa");
    tokio::spawn(async move {
        let _ = axum::serve(listener, roteador).await;
    });
    (format!("http://{endereco}"), evo)
}

// ───────────────────────── cenário ─────────────────────────

struct Cenario {
    base: String,
    cliente: Client,
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    /// Integração WhatsApp declarada em `[evolution].instancias`.
    integracao: IntegrationId,
    /// Integração WhatsApp que **não** foi declarada na configuração.
    integracao_solta: IntegrationId,
    /// Integração de outro tipo (`sms`).
    integracao_sms: IntegrationId,
    evo: EvolucaoFalsa,
    logs: Arc<Mutex<Vec<u8>>>,
    desligar: Option<oneshot::Sender<()>>,
}

struct Resposta {
    status: StatusCode,
    corpo: Value,
    cru: String,
}

fn hash_argon2(valor: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(valor.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

impl Cenario {
    async fn novo(rotulo: &str) -> Self {
        let logs = logs_capturados();
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-whatsapp-admin-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.dados.diretorio = pasta.join("dados");
        config.servidor.porta = 7480;
        config.admin.tokens_globais = vec![TokenGlobalConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::abrir(&config).expect("abre o banco"));
        let org = store
            .orgaos()
            .criar(NovaOrg {
                nome: "Loja WhatsApp".to_owned(),
                slug: "loja-whatsapp".to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agentes()
            .criar(
                org,
                NovoAgente {
                    nome: "Atendente".to_owned(),
                    slug: "atendente".to_owned(),
                    ..Default::default()
                },
            )
            .expect("cria o agente")
            .id;
        let integracao: IntegrationId = store
            .integracao_criar(org, agente, "whatsapp".to_owned(), "active".to_owned())
            .expect("cria a integração")
            .id
            .parse()
            .expect("id da integração");
        let integracao_solta: IntegrationId = store
            .integracao_criar(org, agente, "whatsapp".to_owned(), "active".to_owned())
            .expect("cria a integração solta")
            .id
            .parse()
            .expect("id da integração solta");
        let integracao_sms: IntegrationId = store
            .integracao_criar(org, agente, "sms".to_owned(), "active".to_owned())
            .expect("cria a integração sms")
            .id
            .parse()
            .expect("id da integração sms");

        let (url_evo, evo) = subir_evolucao_falsa().await;
        config.evolution = Some(EvolutionConfig {
            url_base: url_evo,
            instancias: vec![InstanciaEvolutionConfig {
                org_id: org.to_string(),
                instancia: "loja".to_owned(),
                integracao_id: integracao.to_string(),
                segredo_env: VARIAVEL_SEGREDO.to_owned(),
            }],
        });
        config.validar().expect("configuração com Evolution válida");

        // O caminho de verdade da subida: configuração + `EVOLUTION_API_KEY` do ambiente.
        let canais: Vec<CanalEvolution> = canais_da_configuracao(
            &config,
            Some(Segredo::new(CHAVE_EVO.to_owned())),
            &|variavel| (variavel == VARIAVEL_SEGREDO).then(|| SEGREDO_WEBHOOK.to_owned()),
        )
        .expect("canais da configuração");
        assert_eq!(canais.len(), 1, "só a instância declarada vira canal");

        let estado = Estado::novo(Arc::clone(&store), config)
            .expect("estado do servidor")
            .com_canais_evolution(canais);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, sinal) = oneshot::channel();
        tokio::spawn(async move {
            let _ = poph_server::servir_em(listener, estado, async {
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
            org,
            agente,
            integracao,
            integracao_solta,
            integracao_sms,
            evo,
            logs,
            desligar: Some(desligar),
        };
        cenario.esperar_subir().await;
        cenario
    }

    async fn esperar_subir(&self) {
        for _ in 0..500 {
            if let Ok(r) = self
                .cliente
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

    fn rota(&self, integracao: IntegrationId, fim: &str) -> String {
        format!(
            "/admin/v1/orgs/{}/agents/{}/integrations/{}/whatsapp/{fim}",
            self.org, self.agente, integracao
        )
    }

    async fn pedir(
        &self,
        metodo: Method,
        caminho: &str,
        token: &str,
        corpo: Option<Value>,
        if_match: Option<u64>,
    ) -> Resposta {
        let mut pedido = self
            .cliente
            .request(metodo, format!("{}{caminho}", self.base));
        if !token.is_empty() {
            pedido = pedido.bearer_auth(token);
        }
        if let Some(versao) = if_match {
            pedido = pedido.header("If-Match", versao.to_string());
        }
        if let Some(valor) = corpo {
            pedido = pedido.json(&valor);
        }
        let resposta = pedido.send().await.expect("pedido HTTP");
        let status = resposta.status();
        let cru = resposta.text().await.unwrap_or_default();
        let corpo = serde_json::from_str(&cru).unwrap_or(Value::Null);
        Resposta { status, corpo, cru }
    }

    fn estado_gravado(&self, integracao: IntegrationId) -> String {
        self.store
            .integracao_obter(self.org, self.agente, integracao)
            .expect("integração")
            .connection_state
    }

    fn versao(&self, integracao: IntegrationId) -> u64 {
        self.store
            .integracao_obter(self.org, self.agente, integracao)
            .expect("integração")
            .version
    }

    /// Token de org de verdade, criado pela API (o valor só existe na resposta da criação).
    async fn token_novo(&self, org: OrgId, rotulo: &str) -> String {
        let resposta = self
            .pedir(
                Method::POST,
                &format!("/admin/v1/orgs/{org}/tokens"),
                TOKEN_GLOBAL,
                Some(json!({"label": rotulo})),
                None,
            )
            .await;
        assert_eq!(resposta.status, StatusCode::CREATED, "{}", resposta.cru);
        resposta.corpo["value"]
            .as_str()
            .expect("valor do token")
            .to_owned()
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

#[tokio::test]
async fn pareamento_devolve_qr_e_grava_estado() {
    let c = Cenario::novo("pareamento").await;
    let rota = c.rota(c.integracao, "qrcode");

    // Sem `If-Match` não age: é escrita na instância da Evolution.
    let sem_if_match = c.pedir(Method::POST, &rota, TOKEN_GLOBAL, None, None).await;
    assert_eq!(sem_if_match.status, StatusCode::PRECONDITION_REQUIRED);
    assert_eq!(codigo(&sem_if_match.corpo), "precondition_required");
    assert_eq!(c.evo.ler(|e| e.criadas), 0, "nada foi criado na Evolution");

    let resposta = c
        .pedir(
            Method::POST,
            &rota,
            TOKEN_GLOBAL,
            Some(json!({"number": TELEFONE})),
            Some(1),
        )
        .await;
    assert_eq!(resposta.status, StatusCode::OK, "{}", resposta.cru);
    assert_eq!(resposta.corpo["instance"], "loja");
    assert_eq!(resposta.corpo["state"], "connecting");
    assert_eq!(resposta.corpo["connection_state"], "awaiting_scan");
    assert_eq!(
        resposta.corpo["qrcode"]["base64"],
        "data:image/png;base64,QR-DE-TESTE"
    );
    assert_eq!(resposta.corpo["qrcode"]["code"], "2@codigo-do-qr");
    assert_eq!(resposta.corpo["qrcode"]["count"], 3);
    assert_eq!(resposta.corpo["pairing_code"], "ABCD-1234");
    assert_eq!(
        resposta.corpo["expira_em"],
        Value::Null,
        "a Evolution não informa prazo do QR"
    );
    assert!(!resposta.cru.contains(CHAVE_EVO), "a chave não pode sair");
    assert!(!resposta.cru.contains(TELEFONE), "o número não pode sair");

    assert_eq!(c.estado_gravado(c.integracao), "awaiting_scan");
    assert_eq!(c.versao(c.integracao), 1, "estado não é edição de versão");

    // A geração do QR fica na auditoria, sem o conteúdo do QR.
    let registros = c
        .store
        .auditoria()
        .listar(c.org, Pagina::default())
        .expect("auditoria da org");
    let linha = registros
        .iter()
        .find(|registro| registro.acao == "whatsapp_qrcode")
        .expect("registro de auditoria do pareamento");
    assert_eq!(linha.entidade, "integration");
    assert_eq!(
        linha.entidade_id.as_deref(),
        Some(c.integracao.to_string().as_str())
    );
    assert!(
        !format!("{linha:?}").contains("QR-DE-TESTE"),
        "o QR não pode ir para a auditoria"
    );
    assert_eq!(c.evo.ler(|e| e.criadas), 1);
    assert_eq!(c.evo.ler(|e| e.pareamentos), 1);

    // Segundo clique: a instância já existe na Evolution, e o pareamento continua.
    let denovo = c
        .pedir(Method::POST, &rota, TOKEN_GLOBAL, None, Some(1))
        .await;
    assert_eq!(denovo.status, StatusCode::OK, "{}", denovo.cru);
    assert_eq!(c.evo.ler(|e| e.criadas), 2, "tentou criar de novo");
    assert_eq!(c.evo.ler(|e| e.pareamentos), 2);

    // Aparelho já conectado: não há QR, e o estado informado é o real.
    c.evo.com(|e| e.aberta = true);
    let conectado = c
        .pedir(Method::POST, &rota, TOKEN_GLOBAL, None, Some(1))
        .await;
    assert_eq!(conectado.status, StatusCode::OK, "{}", conectado.cru);
    assert_eq!(conectado.corpo["state"], "open");
    assert_eq!(conectado.corpo["connection_state"], "connected");
    assert_eq!(conectado.corpo["qrcode"], Value::Null);
    assert_eq!(c.estado_gravado(c.integracao), "connected");
}

#[tokio::test]
async fn estado_ao_vivo_reflete_na_integracao() {
    let c = Cenario::novo("estado").await;
    let rota = c.rota(c.integracao, "state");
    c.evo.com(|e| {
        e.existe = true;
        e.aberta = false;
    });

    let fechado = c.pedir(Method::GET, &rota, TOKEN_GLOBAL, None, None).await;
    assert_eq!(fechado.status, StatusCode::OK, "{}", fechado.cru);
    assert_eq!(fechado.corpo["state"], "close");
    assert_eq!(fechado.corpo["connection_state"], "disconnected");
    assert_eq!(c.estado_gravado(c.integracao), "disconnected");

    c.evo.com(|e| e.aberta = true);
    let aberto = c.pedir(Method::GET, &rota, TOKEN_GLOBAL, None, None).await;
    assert_eq!(aberto.status, StatusCode::OK, "{}", aberto.cru);
    assert_eq!(aberto.corpo["state"], "open");
    assert_eq!(aberto.corpo["connection_state"], "connected");
    assert_eq!(aberto.corpo["integration_id"], c.integracao.to_string());
    assert_eq!(c.estado_gravado(c.integracao), "connected");
    assert_eq!(c.versao(c.integracao), 1, "ler estado não muda a versão");

    // Evolution fora do ar: 503 com código estável, nada gravado por engano.
    // (instância inexistente responde 404 na Evolution, que vira 502 `evolution_recusou`.)
    c.evo.com(|e| e.existe = false);
    let sem_instancia = c.pedir(Method::GET, &rota, TOKEN_GLOBAL, None, None).await;
    assert_eq!(sem_instancia.status, StatusCode::BAD_GATEWAY);
    assert_eq!(codigo(&sem_instancia.corpo), "evolution_recusou");
    assert_eq!(sem_instancia.corpo["details"]["evolution_status"], 404);
    assert_eq!(
        c.estado_gravado(c.integracao),
        "connected",
        "erro não sobrescreve o último estado bom"
    );
}

#[tokio::test]
async fn desconectar_exige_if_match_e_chama_o_logout() {
    let c = Cenario::novo("desconectar").await;
    let rota = c.rota(c.integracao, "disconnect");
    c.evo.com(|e| {
        e.existe = true;
        e.aberta = true;
    });

    let sem_if_match = c.pedir(Method::POST, &rota, TOKEN_GLOBAL, None, None).await;
    assert_eq!(sem_if_match.status, StatusCode::PRECONDITION_REQUIRED);
    assert_eq!(c.evo.ler(|e| e.deslogadas), 0);

    // Outra pessoa edita a integração: a versão sobe para 2.
    c.store
        .integracao_atualizar(
            c.org,
            c.agente,
            c.integracao,
            poph_store::AlteracaoIntegracao {
                versao: 1,
                status: Some("inactive"),
                ..Default::default()
            },
        )
        .expect("edição pela API/CLI concorrente");
    assert_eq!(c.versao(c.integracao), 2);

    let velha = c
        .pedir(Method::POST, &rota, TOKEN_GLOBAL, None, Some(1))
        .await;
    assert_eq!(velha.status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(codigo(&velha.corpo), "version_conflict");
    assert_eq!(velha.corpo["details"]["expected_version"], 1);
    assert_eq!(velha.corpo["details"]["current_version"], 2);
    assert_eq!(c.evo.ler(|e| e.deslogadas), 0, "nada foi desconectado");

    let certo = c
        .pedir(Method::POST, &rota, TOKEN_GLOBAL, None, Some(2))
        .await;
    assert_eq!(certo.status, StatusCode::OK, "{}", certo.cru);
    assert_eq!(certo.corpo["connection_state"], "disconnected");
    assert_eq!(certo.corpo["disconnected"], true);
    assert_eq!(c.evo.ler(|e| e.deslogadas), 1);
    assert!(!c.evo.ler(|e| e.aberta), "o aparelho continua conectado");
    assert_eq!(c.estado_gravado(c.integracao), "disconnected");
    assert_eq!(c.versao(c.integracao), 2, "desconectar não sobe a versão");
    let registros = c
        .store
        .auditoria()
        .listar(c.org, Pagina::default())
        .expect("auditoria da org");
    assert!(
        registros
            .iter()
            .any(|registro| registro.acao == "whatsapp_disconnect"
                && registro.entidade == "integration"),
        "a desconexão precisa ficar na auditoria"
    );
}

#[tokio::test]
async fn integracao_inexistente_da_404_nas_tres_rotas() {
    let c = Cenario::novo("inexistente").await;
    let solta = IntegrationId::generate();
    for (metodo, fim) in [
        (Method::POST, "qrcode"),
        (Method::GET, "state"),
        (Method::POST, "disconnect"),
    ] {
        let resposta = c
            .pedir(metodo, &c.rota(solta, fim), TOKEN_GLOBAL, None, Some(1))
            .await;
        assert_eq!(
            resposta.status,
            StatusCode::NOT_FOUND,
            "rota `{fim}`: {}",
            resposta.cru
        );
        assert_eq!(codigo(&resposta.corpo), "not_found");
    }
    assert_eq!(c.evo.ler(|e| e.criadas + e.deslogadas), 0);
}

#[tokio::test]
async fn integracao_sem_instancia_declarada_da_409() {
    let c = Cenario::novo("sem-instancia").await;
    for (metodo, fim) in [
        (Method::POST, "qrcode"),
        (Method::GET, "state"),
        (Method::POST, "disconnect"),
    ] {
        let resposta = c
            .pedir(
                metodo,
                &c.rota(c.integracao_solta, fim),
                TOKEN_GLOBAL,
                None,
                Some(1),
            )
            .await;
        assert_eq!(
            resposta.status,
            StatusCode::CONFLICT,
            "rota `{fim}`: {}",
            resposta.cru
        );
        assert_eq!(codigo(&resposta.corpo), "whatsapp_nao_configurado");
    }
    assert_eq!(c.evo.ler(|e| e.criadas + e.deslogadas), 0);
}

#[tokio::test]
async fn integracao_que_nao_e_whatsapp_da_409() {
    let c = Cenario::novo("sms").await;
    let resposta = c
        .pedir(
            Method::POST,
            &c.rota(c.integracao_sms, "qrcode"),
            TOKEN_GLOBAL,
            None,
            Some(1),
        )
        .await;
    assert_eq!(resposta.status, StatusCode::CONFLICT);
    assert_eq!(codigo(&resposta.corpo), "integracao_nao_e_whatsapp");
    assert_eq!(resposta.corpo["details"]["kind"], "sms");
}

#[tokio::test]
async fn segredo_e_numero_nao_vazam_na_resposta_nem_no_log() {
    let c = Cenario::novo("segredo").await;
    c.evo.com(|e| e.falhar_ao_criar = true);
    let resposta = c
        .pedir(
            Method::POST,
            &c.rota(c.integracao, "qrcode"),
            TOKEN_GLOBAL,
            Some(json!({"number": TELEFONE})),
            Some(1),
        )
        .await;
    assert_eq!(resposta.status, StatusCode::BAD_GATEWAY, "{}", resposta.cru);
    assert_eq!(codigo(&resposta.corpo), "evolution_recusou");
    assert!(
        !resposta.cru.contains(CHAVE_EVO),
        "a chave vazou na resposta: {}",
        resposta.cru
    );
    assert!(
        !resposta.cru.contains(TELEFONE),
        "o número vazou na resposta: {}",
        resposta.cru
    );
    let logs = String::from_utf8_lossy(&c.logs.lock().unwrap().clone()).into_owned();
    assert!(
        !logs.contains(CHAVE_EVO),
        "a chave vazou no log do servidor"
    );
    assert!(
        !logs.contains(TELEFONE),
        "o número vazou no log do servidor"
    );
    assert_eq!(c.estado_gravado(c.integracao), "disconnected");
}

#[tokio::test]
async fn token_de_outra_org_da_403() {
    let c = Cenario::novo("outra-org").await;
    let outra = c
        .store
        .orgaos()
        .criar(NovaOrg {
            nome: "Outra Loja".to_owned(),
            slug: "outra-loja".to_owned(),
            config_json: String::new(),
        })
        .expect("cria a outra org")
        .id;
    let token = c.token_novo(outra, "outra org").await;
    for (metodo, fim) in [
        (Method::POST, "qrcode"),
        (Method::GET, "state"),
        (Method::POST, "disconnect"),
    ] {
        let resposta = c
            .pedir(metodo, &c.rota(c.integracao, fim), &token, None, Some(1))
            .await;
        assert_eq!(
            resposta.status,
            StatusCode::FORBIDDEN,
            "rota `{fim}`: {}",
            resposta.cru
        );
    }
    assert_eq!(c.evo.ler(|e| e.criadas + e.deslogadas), 0);
}
