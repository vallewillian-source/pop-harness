//! Encaminhamento assinado entre instâncias (HR-50b-1), de ponta a ponta.
//!
//! Duas instâncias **reais** (`poph_server::servir_em` em portas efêmeras), bancos temporários e
//! um `cluster.toml` lógico igual nos dois lados (só o `instance_id` muda), com mapa explícito:
//! a org de prova vive em `b`.
//!
//! O que se prova:
//! - chamada admin de org de `b` feita em `a` é atendida por `b` (mesmo corpo da chamada direta) e
//!   `a` não tem a org no próprio banco;
//! - webhook da org de `b` entregue em `a` é processado por `b` (resposta que só `b` sabe dar);
//! - assinatura válida passa; corpo alterado, carimbo fora da janela, instância desconhecida e
//!   assinatura repetida são recusados com `401 assinatura_invalida`;
//! - `hop > 1` → `421 rota_em_laco`; mapa diferente → `421 mapa_divergente` com os dois
//!   fingerprints; org que não vive na instância que recebeu o pedido encaminhado → `421`;
//! - dona fora do ar → `503 instancia_fora` com instância e endereço em `details`;
//! - modo instância única segue idêntico a hoje (nada é encaminhado; pedido com cabeçalho de par
//!   sem segredo configurado → `401`);
//! - o segredo compartilhado nunca aparece em resposta nem no log.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::Estado;
use poph_server::cluster::ClusterVivo;
use poph_server::encaminhamento::{
    CABECALHO_ASSINATURA, CABECALHO_HOP, CABECALHO_INSTANCIA, CABECALHO_MAPA, CABECALHO_NONCE,
    CABECALHO_TIMESTAMP, assinar, hash_do_corpo, nonce_novo,
};
use poph_server::evolution::CanalEvolution;
use poph_store::{NewOrg, Page, Store};
use poph_types::config::{ClusterConfig, ClusterInstance, GlobalTokenConfig};
use poph_types::{HarnessConfig, IntegrationId, OrgId, RecordStatus, Secret};
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tracing_subscriber::fmt::MakeWriter;

const TOKEN_GLOBAL: &str = "pophg_teste-global-hr50b";
const SEGREDO: &str = "SENTINELA-SEGREDO-ENCAMINHAMENTO-hr50b";
const SEGREDO_WEBHOOK: &str = "segredo-webhook-teste-hr50b";

// ───────────────────────── captura de logs (prova o que NÃO sai) ─────────────────────────

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

// ───────────────────────── instância de prova ─────────────────────────

struct Instancia {
    base: String,
    store: Arc<Store>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Drop for Instancia {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
    }
}

/// Banco e configuração montados antes de o mapa existir (a org de prova nasce aqui).
struct Preparo {
    store: Arc<Store>,
    config: HarnessConfig,
    #[allow(dead_code)]
    pasta: PathBuf,
}

fn hash_argon2(valor: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(valor.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

fn pasta(rotulo: &str) -> PathBuf {
    static CONTADOR: AtomicUsize = AtomicUsize::new(0);
    let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
    let carimbo = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pasta = std::env::temp_dir().join(format!(
        "poph-encaminhamento-{rotulo}-{}-{carimbo}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");
    pasta
}

fn preparar(rotulo: &str) -> Preparo {
    let pasta = pasta(rotulo);
    let mut config = HarnessConfig::default();
    config.data.directory = pasta.join("dados");
    config.admin.tokens_globais = vec![GlobalTokenConfig {
        label: "teste".to_owned(),
        hash: hash_argon2(TOKEN_GLOBAL),
    }];
    config.validar().expect("configuração válida");
    let store = Arc::new(Store::open(&config).expect("abre o banco"));
    Preparo {
        store,
        config,
        pasta,
    }
}

/// Mapa compartilhado: `a` e `b` com os endereços reais e a org de prova em `b`.
fn mapa(instance_id: &str, url_a: &str, url_b: &str, org: &str, destino: &str) -> ClusterConfig {
    ClusterConfig {
        instance_id: instance_id.to_owned(),
        default_instance: Some("b".to_owned()),
        instances: vec![
            ClusterInstance {
                id: "a".to_owned(),
                url: url_a.to_owned(),
            },
            ClusterInstance {
                id: "b".to_owned(),
                url: url_b.to_owned(),
            },
        ],
        orgs: BTreeMap::from([(org.to_owned(), destino.to_owned())]),
    }
}

async fn iniciar(
    preparo: Preparo,
    listener: tokio::net::TcpListener,
    mapa: ClusterConfig,
    segredo: Option<Secret>,
    canais: Vec<CanalEvolution>,
) -> Instancia {
    let estado = Estado::novo(Arc::clone(&preparo.store), preparo.config)
        .expect("estado do servidor")
        .com_cluster(ClusterVivo {
            caminho: preparo.pasta.join("cluster.toml"),
            config: mapa,
            segredo,
        })
        .com_canais_evolution(canais);
    let endereco = listener.local_addr().expect("endereço local");
    let (desligar, sinal) = oneshot::channel();
    tokio::spawn(async move {
        let _ = poph_server::serve_at(listener, estado, async {
            let _ = sinal.await;
        })
        .await;
    });
    let instancia = Instancia {
        base: format!("http://{endereco}"),
        store: preparo.store,
        desligar: Some(desligar),
    };
    let cliente = cliente();
    for _ in 0..500 {
        if let Ok(resposta) = cliente
            .get(format!("{}/healthz", instancia.base))
            .send()
            .await
            && resposta.status().is_success()
        {
            return instancia;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("instância não subiu");
}

fn cliente() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("cliente HTTP")
}

async fn listener_livre() -> (tokio::net::TcpListener, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = listener.local_addr().expect("endereço local");
    (listener, format!("http://{endereco}"))
}

fn agora() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    )
    .unwrap_or(0)
}

/// Resposta crua de uma chamada: status, corpo e texto (para grep).
struct Resposta {
    status: StatusCode,
    corpo: Value,
    cru: String,
}

impl Resposta {
    fn sem_segredo(&self) {
        assert!(
            !self.cru.contains(SEGREDO),
            "segredo na resposta: {}",
            self.cru
        );
        assert!(
            !self.cru.contains(&hash_do_corpo(SEGREDO.as_bytes())),
            "hash do segredo na resposta"
        );
    }
}

async fn pedir(
    metodo: Method,
    url: &str,
    token: Option<&str>,
    corpo: Option<&[u8]>,
    extras: &[(&str, String)],
) -> Resposta {
    let mut pedido = cliente().request(metodo, url);
    if let Some(token) = token {
        pedido = pedido.header("authorization", format!("Bearer {token}"));
    }
    for (nome, valor) in extras {
        pedido = pedido.header(*nome, valor);
    }
    if let Some(corpo) = corpo {
        pedido = pedido
            .header("content-type", "application/json")
            .body(corpo.to_vec());
    }
    let resposta = pedido.send().await.expect("resposta HTTP");
    let status = resposta.status();
    let cru = resposta.text().await.expect("corpo da resposta");
    let corpo = serde_json::from_str(&cru).unwrap_or(Value::Null);
    Resposta { status, corpo, cru }
}

/// Cabeçalhos de par assinados para um pedido a partir de `instancia`, com um nonce novo.
#[allow(clippy::too_many_arguments)]
fn cabecalhos_assinados(
    segredo: &Secret,
    metodo: &Method,
    alvo: &str,
    corpo: &[u8],
    instancia: &str,
    hop: u32,
    carimbo: i64,
    mapa: &str,
) -> Vec<(&'static str, String)> {
    cabecalhos_assinados_com_nonce(
        segredo,
        metodo,
        alvo,
        corpo,
        instancia,
        hop,
        carimbo,
        mapa,
        &nonce_novo(),
    )
}

/// Idem, com o nonce escolhido pelo teste (para provar repetição e nonce malformado).
#[allow(clippy::too_many_arguments)]
fn cabecalhos_assinados_com_nonce(
    segredo: &Secret,
    metodo: &Method,
    alvo: &str,
    corpo: &[u8],
    instancia: &str,
    hop: u32,
    carimbo: i64,
    mapa: &str,
    nonce: &str,
) -> Vec<(&'static str, String)> {
    let assinatura = assinar(segredo, metodo, alvo, corpo, carimbo, instancia, hop, nonce);
    vec![
        (CABECALHO_INSTANCIA, instancia.to_owned()),
        (CABECALHO_TIMESTAMP, carimbo.to_string()),
        (CABECALHO_ASSINATURA, assinatura),
        (CABECALHO_HOP, hop.to_string()),
        (CABECALHO_MAPA, mapa.to_owned()),
        (CABECALHO_NONCE, nonce.to_owned()),
    ]
}

/// Sobe duas instâncias com o mapa compartilhado e a org de prova criada em `b`.
async fn duas_instancias(
    rotulo: &str,
    com_canal_em_b: bool,
) -> (Instancia, Instancia, OrgId, ClusterConfig) {
    let (listener_a, url_a) = listener_livre().await;
    let (listener_b, url_b) = listener_livre().await;
    let preparo_a = preparar(&format!("{rotulo}-a"));
    let preparo_b = preparar(&format!("{rotulo}-b"));
    let org = preparo_b
        .store
        .orgs()
        .create(NewOrg {
            name: "Org da instância b".to_owned(),
            slug: "org-da-instancia-b".to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org em b")
        .id;
    let org_texto = org.to_string();

    let mapa_a = mapa("a", &url_a, &url_b, &org_texto, "b");
    let mapa_b = mapa("b", &url_a, &url_b, &org_texto, "b");
    let segredo = Secret::new(SEGREDO.to_owned());

    let mut canais = Vec::new();
    if com_canal_em_b {
        canais.push(CanalEvolution {
            org,
            instancia: "loja".to_owned(),
            integracao: IntegrationId::generate(),
            segredo: Some(Secret::new(SEGREDO_WEBHOOK.to_owned())),
            cliente: None,
            teto_midia: 1_048_576,
        });
    }

    let a = iniciar(
        preparo_a,
        listener_a,
        mapa_a,
        Some(segredo.clone()),
        Vec::new(),
    )
    .await;
    let b = iniciar(preparo_b, listener_b, mapa_b, Some(segredo), canais).await;
    (a, b, org, mapa("b", &url_a, &url_b, &org_texto, "b"))
}

// ───────────────────────── cenários ─────────────────────────

/// Chamada admin de org de `b` feita em `a` é atendida por `b`, e o webhook idem.
#[tokio::test(flavor = "multi_thread")]
async fn admin_e_webhook_de_org_remota_chegam_na_dona() {
    let (a, b, org, _mapa) = duas_instancias("remota", true).await;
    let caminho = format!("/admin/v1/orgs/{org}/agents");

    let na_dona = pedir(
        Method::GET,
        &format!("{}{caminho}", b.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(na_dona.status, StatusCode::OK, "{}", na_dona.cru);
    assert_eq!(na_dona.corpo["items"].as_array().map(Vec::len), Some(0));

    // Passando por `a`: o corpo tem de ser exatamente o da dona.
    let via_a = pedir(
        Method::GET,
        &format!("{}{caminho}", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(via_a.status, StatusCode::OK, "{}", via_a.cru);
    assert_eq!(via_a.corpo, na_dona.corpo);
    via_a.sem_segredo();

    // Com query na URL: a query entra na assinatura e tem de ser preservada no encaminhamento.
    let com_query = pedir(
        Method::GET,
        &format!("{}{caminho}?limit=1", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(com_query.status, StatusCode::OK, "{}", com_query.cru);
    assert_eq!(com_query.corpo, na_dona.corpo);

    // Arquivo grande (a exportação da org) volta inteiro pelo encaminhamento, com os cabeçalhos
    // do anexo: prova que a resposta vai em streaming e sem perder cabeçalho.
    let backup_direto = pedir(
        Method::GET,
        &format!("{}/admin/v1/orgs/{org}/backup", b.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(
        backup_direto.status,
        StatusCode::OK,
        "{}",
        backup_direto.cru
    );
    assert_eq!(backup_direto.corpo["formato"], json!("poph-backup-org"));
    let backup_por_a = pedir(
        Method::GET,
        &format!("{}/admin/v1/orgs/{org}/backup", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(backup_por_a.status, StatusCode::OK, "{}", backup_por_a.cru);
    // Só o instante da exportação difere entre as duas chamadas.
    assert_eq!(backup_por_a.corpo["org"], backup_direto.corpo["org"]);
    assert_eq!(
        backup_por_a.corpo["tabelas"],
        backup_direto.corpo["tabelas"]
    );
    assert_eq!(
        backup_por_a.corpo["org_origem"],
        backup_direto.corpo["org_origem"]
    );

    // `a` não tem a org no banco: quem respondeu foi `b`.
    assert!(
        a.store.orgs().by_slug("org-da-instancia-b").is_err(),
        "a org não pode existir no banco de `a`"
    );

    // Org que não existe em lugar nenhum: `b` responde 404, e é o 404 que volta por `a`.
    let inexistente = OrgId::generate();
    let outra = format!("/admin/v1/orgs/{inexistente}");
    let direto = pedir(
        Method::GET,
        &format!("{}{outra}", b.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(direto.status, StatusCode::NOT_FOUND);
    let por_a = pedir(
        Method::GET,
        &format!("{}{outra}", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(por_a.status, StatusCode::NOT_FOUND);
    assert_eq!(por_a.corpo["code"], json!("not_found"));

    // Webhook da org de `b` entregue em `a`: quem responde é `b` (integração que não existe lá).
    let evento = json!({
        "event": "connection.update",
        "instance": "loja",
        "data": {"state": "open"}
    })
    .to_string();
    let caminho_webhook = format!("/webhook/evolution/{org}/loja");
    let webhook = pedir(
        Method::POST,
        &format!("{}{caminho_webhook}", a.base),
        None,
        Some(evento.as_bytes()),
        &[("x-evolution-webhook-secret", SEGREDO_WEBHOOK.to_owned())],
    )
    .await;
    assert_eq!(webhook.status, StatusCode::OK, "{}", webhook.cru);
    assert_eq!(webhook.corpo["recebido"], json!(true));
    assert_eq!(webhook.corpo["efeito"], json!("integracao_inexistente"));

    // Sem o segredo do webhook: `b` recusa (401), e o 401 volta por `a` — prova de que `a` não
    // atendeu localmente (localmente seria 404, porque `a` não tem a instância declarada).
    // Corpo diferente do primeiro (pedido novo, não repetição).
    let outro_evento = json!({
        "event": "connection.update",
        "instance": "loja",
        "data": {"state": "connecting"}
    })
    .to_string();
    let sem_segredo = pedir(
        Method::POST,
        &format!("{}{caminho_webhook}", a.base),
        None,
        Some(outro_evento.as_bytes()),
        &[],
    )
    .await;
    assert_eq!(
        sem_segredo.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        sem_segredo.cru
    );
    assert_eq!(sem_segredo.corpo["code"], json!("webhook_nao_autorizado"));

    // O log inteiro do processo de testes não pode conter o segredo compartilhado.
    let logs = logs_capturados();
    let texto = String::from_utf8_lossy(&logs.lock().unwrap()).to_string();
    assert!(!texto.contains(SEGREDO), "segredo no log: {texto}");
}

/// Assinatura: válida passa; adulterada, fora da janela, instância desconhecida e repetida recusam.
#[tokio::test(flavor = "multi_thread")]
async fn assinatura_do_encaminhamento_e_conferida() {
    let (_a, b, org, mapa) = duas_instancias("assinatura", false).await;
    let segredo = Secret::new(SEGREDO.to_owned());
    let alvo = format!("/admin/v1/orgs/{org}/agents");
    let url = format!("{}{alvo}", b.base);
    let fingerprint = mapa.map_fingerprint();

    // Válida, sem corpo.
    let bons = cabecalhos_assinados(
        &segredo,
        &Method::GET,
        &alvo,
        b"",
        "a",
        1,
        agora(),
        &fingerprint,
    );
    let valida = pedir(Method::GET, &url, Some(TOKEN_GLOBAL), None, &bons).await;
    assert_eq!(valida.status, StatusCode::OK, "{}", valida.cru);

    // Mesmo pedido reenviado com o mesmo nonce: repetição, recusada.
    let repetida = pedir(Method::GET, &url, Some(TOKEN_GLOBAL), None, &bons).await;
    assert_eq!(
        repetida.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        repetida.cru
    );
    assert_eq!(repetida.corpo["code"], json!("assinatura_invalida"));

    // Corpo alterado: a assinatura não confere.
    let corpo = json!({"nome": "Loja", "slug": "loja"}).to_string();
    let assinado = cabecalhos_assinados(
        &segredo,
        &Method::POST,
        &format!("/admin/v1/orgs/{org}/agents"),
        corpo.as_bytes(),
        "a",
        1,
        agora(),
        &fingerprint,
    );
    let adulterado = pedir(
        Method::POST,
        &format!("{}/admin/v1/orgs/{org}/agents", b.base),
        Some(TOKEN_GLOBAL),
        Some(br#"{"nome":"Outra","slug":"outra"}"#),
        &assinado,
    )
    .await;
    assert_eq!(
        adulterado.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        adulterado.cru
    );
    assert_eq!(adulterado.corpo["code"], json!("assinatura_invalida"));

    // Carimbo fora da janela (dois minutos atrás).
    let velho = cabecalhos_assinados(
        &segredo,
        &Method::GET,
        &alvo,
        b"",
        "a",
        1,
        agora() - 120,
        &fingerprint,
    );
    let fora_da_janela = pedir(Method::GET, &url, Some(TOKEN_GLOBAL), None, &velho).await;
    assert_eq!(fora_da_janela.status, StatusCode::UNAUTHORIZED);
    assert_eq!(fora_da_janela.corpo["code"], json!("assinatura_invalida"));

    // Instância que não está no mapa.
    let desconhecida = cabecalhos_assinados(
        &segredo,
        &Method::GET,
        &alvo,
        b"",
        "z",
        1,
        agora(),
        &fingerprint,
    );
    let instancia_desconhecida =
        pedir(Method::GET, &url, Some(TOKEN_GLOBAL), None, &desconhecida).await;
    assert_eq!(instancia_desconhecida.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        instancia_desconhecida.corpo["code"],
        json!("assinatura_invalida")
    );

    // Sem cabeçalho de par, o caminho normal de token continua igual.
    let normal = pedir(Method::GET, &url, Some(TOKEN_GLOBAL), None, &[]).await;
    assert_eq!(normal.status, StatusCode::OK);
    let sem_token = pedir(Method::GET, &url, None, None, &[]).await;
    assert_eq!(sem_token.status, StatusCode::UNAUTHORIZED);
    assert_eq!(sem_token.corpo["code"], json!("unauthorized"));
}

/// `421`: salto repetido, mapa divergente e org que não vive na instância que recebeu.
#[tokio::test(flavor = "multi_thread")]
async fn laco_e_mapa_divergente_dao_421() {
    let (a, b, org, mapa) = duas_instancias("laco", false).await;
    let segredo = Secret::new(SEGREDO.to_owned());
    let alvo = format!("/admin/v1/orgs/{org}/agents");
    let url = format!("{}{alvo}", b.base);

    // hop = 2 no pedido já encaminhado.
    let dois_saltos = cabecalhos_assinados(
        &segredo,
        &Method::GET,
        &alvo,
        b"",
        "a",
        2,
        agora(),
        &mapa.map_fingerprint(),
    );
    let laco = pedir(Method::GET, &url, Some(TOKEN_GLOBAL), None, &dois_saltos).await;
    assert_eq!(laco.status, StatusCode::MISDIRECTED_REQUEST, "{}", laco.cru);
    assert_eq!(laco.corpo["code"], json!("rota_em_laco"));

    // Mapa de quem encaminhou diferente do local: a org aparece em `a`, não em `b`.
    let mut outro = mapa.clone();
    outro.instance_id = "a".to_owned();
    outro.orgs.insert(org.to_string(), "a".to_owned());
    let fingerprint_alheio = outro.map_fingerprint();
    assert_ne!(fingerprint_alheio, mapa.map_fingerprint());
    // Cada pedido leva um nonce novo, então o mesmo segundo não colide (o mapa não entra no
    // texto assinado).
    let divergente = cabecalhos_assinados(
        &segredo,
        &Method::GET,
        &alvo,
        b"",
        "a",
        1,
        agora() - 5,
        &fingerprint_alheio,
    );
    let recusado = pedir(Method::GET, &url, Some(TOKEN_GLOBAL), None, &divergente).await;
    assert_eq!(
        recusado.status,
        StatusCode::MISDIRECTED_REQUEST,
        "{}",
        recusado.cru
    );
    assert_eq!(recusado.corpo["code"], json!("mapa_divergente"));
    assert_eq!(
        recusado.corpo["details"]["fingerprint_recebido"],
        json!(fingerprint_alheio)
    );
    assert_eq!(
        recusado.corpo["details"]["fingerprint_local"],
        json!(mapa.map_fingerprint())
    );

    // Assinatura válida de um pedido que pertence a `b`, entregue em `a`: `a` não pode encaminhar
    // de novo (laço). Depois, o mesmo pedido entregue na dona (`b`) é atendido.
    let para_a = cabecalhos_assinados(
        &segredo,
        &Method::GET,
        &alvo,
        b"",
        "a",
        1,
        agora() - 10,
        &mapa.map_fingerprint(),
    );
    let na_instancia_errada = pedir(
        Method::GET,
        &format!("{}{alvo}", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &para_a,
    )
    .await;
    assert_eq!(
        na_instancia_errada.status,
        StatusCode::MISDIRECTED_REQUEST,
        "{}",
        na_instancia_errada.cru
    );
    assert_eq!(na_instancia_errada.corpo["code"], json!("rota_em_laco"));
    assert_eq!(
        na_instancia_errada.corpo["details"]["instancia_dona"],
        json!("b")
    );

    // A instância certa aceita o mesmo pedido (a memória anti-repetição é por processo).
    let na_dona = pedir(Method::GET, &url, Some(TOKEN_GLOBAL), None, &para_a).await;
    assert_eq!(na_dona.status, StatusCode::OK, "{}", na_dona.cru);
}

/// Dona fora do ar → `503 instancia_fora` com instância e endereço, sem segredo.
#[tokio::test(flavor = "multi_thread")]
async fn dona_fora_do_ar_da_503() {
    let (listener_a, url_a) = listener_livre().await;
    // Porta que ninguém escuta: pega uma porta livre e fecha em seguida.
    let (listener_b, url_b) = listener_livre().await;
    drop(listener_b);

    let preparo = preparar("fora-do-ar-a");
    let org = preparo
        .store
        .orgs()
        .create(NewOrg {
            name: "Org fora do ar".to_owned(),
            slug: "org-fora-do-ar".to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id;
    let mapa_a = mapa("a", &url_a, &url_b, &org.to_string(), "b");
    let a = iniciar(
        preparo,
        listener_a,
        mapa_a,
        Some(Secret::new(SEGREDO.to_owned())),
        Vec::new(),
    )
    .await;

    let resposta = pedir(
        Method::GET,
        &format!("{}/admin/v1/orgs/{org}/agents", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(
        resposta.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        resposta.cru
    );
    assert_eq!(resposta.corpo["code"], json!("instancia_fora"));
    assert_eq!(resposta.corpo["details"]["instancia"], json!("b"));
    assert_eq!(resposta.corpo["details"]["url"], json!(url_b));
    resposta.sem_segredo();
}

/// Modo instância única: nada muda em relação a hoje.
#[tokio::test(flavor = "multi_thread")]
async fn modo_instancia_unica_fica_igual() {
    let (listener, _url) = listener_livre().await;
    let a = iniciar(
        preparar("unica"),
        listener,
        ClusterConfig::default(),
        None,
        Vec::new(),
    )
    .await;

    // Admin local, como sempre.
    let local = pedir(
        Method::GET,
        &format!("{}/admin/v1/orgs", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(local.status, StatusCode::OK, "{}", local.cru);

    // Webhook de instância não configurada: 404 local (não é encaminhado para lugar nenhum).
    let webhook = pedir(
        Method::POST,
        &format!("{}/webhook/evolution/{}/loja", a.base, OrgId::generate()),
        None,
        Some(b"{\"event\":\"connection.update\",\"instance\":\"loja\",\"data\":{\"state\":\"open\"}}"),
        &[("x-evolution-webhook-secret", SEGREDO_WEBHOOK.to_owned())],
    )
    .await;
    assert_eq!(webhook.status, StatusCode::NOT_FOUND, "{}", webhook.cru);

    // Pedido com cabeçalho de par numa instância sem segredo compartilhado: 401, sem atender.
    let com_par = pedir(
        Method::GET,
        &format!("{}/admin/v1/orgs/{}/agents", a.base, OrgId::generate()),
        Some(TOKEN_GLOBAL),
        None,
        &[
            (CABECALHO_INSTANCIA, "a".to_owned()),
            (CABECALHO_TIMESTAMP, agora().to_string()),
            (CABECALHO_ASSINATURA, "00".to_owned()),
            (CABECALHO_HOP, "1".to_owned()),
            (CABECALHO_MAPA, "00".to_owned()),
        ],
    )
    .await;
    assert_eq!(com_par.status, StatusCode::UNAUTHORIZED, "{}", com_par.cru);
    assert_eq!(com_par.corpo["code"], json!("assinatura_invalida"));
}

/// Nonce: cada pedido é único (dois idênticos no mesmo segundo passam), a repetição do nonce é
/// recusada e nonce ausente ou malformado é `401 assinatura_invalida` (HR-50b-2).
#[tokio::test(flavor = "multi_thread")]
async fn nonce_torna_cada_pedido_unico_e_recusa_repeticao() {
    let (_a, b, org, mapa) = duas_instancias("nonce", false).await;
    let segredo = Secret::new(SEGREDO.to_owned());
    let alvo = format!("/admin/v1/orgs/{org}/agents");
    let url = format!("{}{alvo}", b.base);
    let fingerprint = mapa.map_fingerprint();
    let carimbo = agora();

    // Dois pedidos idênticos, no mesmo segundo, com nonces diferentes: os dois passam.
    let primeiro = cabecalhos_assinados(
        &segredo,
        &Method::GET,
        &alvo,
        b"",
        "a",
        1,
        carimbo,
        &fingerprint,
    );
    let segundo = cabecalhos_assinados(
        &segredo,
        &Method::GET,
        &alvo,
        b"",
        "a",
        1,
        carimbo,
        &fingerprint,
    );
    assert_ne!(
        primeiro.iter().find(|(nome, _)| *nome == CABECALHO_NONCE),
        segundo.iter().find(|(nome, _)| *nome == CABECALHO_NONCE),
        "cada pedido tem de levar um nonce próprio"
    );
    let um = pedir(Method::GET, &url, Some(TOKEN_GLOBAL), None, &primeiro).await;
    assert_eq!(um.status, StatusCode::OK, "{}", um.cru);
    let dois = pedir(Method::GET, &url, Some(TOKEN_GLOBAL), None, &segundo).await;
    assert_eq!(
        dois.status,
        StatusCode::OK,
        "pedido idêntico no mesmo segundo com nonce novo tem de passar: {}",
        dois.cru
    );

    // Repetição do mesmo nonce: recusada.
    let repetido = pedir(Method::GET, &url, Some(TOKEN_GLOBAL), None, &primeiro).await;
    assert_eq!(
        repetido.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        repetido.cru
    );
    assert_eq!(repetido.corpo["code"], json!("assinatura_invalida"));
    assert!(
        repetido.corpo["message"]
            .as_str()
            .unwrap_or_default()
            .contains("nonce"),
        "a mensagem precisa dizer que é o nonce: {}",
        repetido.cru
    );

    // Nonce ausente: 401.
    let sem_nonce: Vec<(&'static str, String)> = cabecalhos_assinados(
        &segredo,
        &Method::GET,
        &alvo,
        b"",
        "a",
        1,
        agora() - 3,
        &fingerprint,
    )
    .into_iter()
    .filter(|(nome, _)| *nome != CABECALHO_NONCE)
    .collect();
    let ausente = pedir(Method::GET, &url, Some(TOKEN_GLOBAL), None, &sem_nonce).await;
    assert_eq!(ausente.status, StatusCode::UNAUTHORIZED, "{}", ausente.cru);
    assert_eq!(ausente.corpo["code"], json!("assinatura_invalida"));
    assert!(
        ausente.corpo["message"]
            .as_str()
            .unwrap_or_default()
            .contains("x-poph-nonce"),
        "a mensagem precisa apontar o cabeçalho que falta: {}",
        ausente.cru
    );

    // Nonce malformado (assinado corretamente): 401 antes de qualquer processamento.
    let malformado = cabecalhos_assinados_com_nonce(
        &segredo,
        &Method::GET,
        &alvo,
        b"",
        "a",
        1,
        agora() - 6,
        &fingerprint,
        "abc",
    );
    let invalido = pedir(Method::GET, &url, Some(TOKEN_GLOBAL), None, &malformado).await;
    assert_eq!(
        invalido.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        invalido.cru
    );
    assert_eq!(invalido.corpo["code"], json!("assinatura_invalida"));
    assert!(
        invalido.corpo["message"]
            .as_str()
            .unwrap_or_default()
            .contains("malformado"),
        "a mensagem precisa dizer que o nonce está malformado: {}",
        invalido.cru
    );
    invalido.sem_segredo();
}

/// `GET /admin/v1/orgs?cluster=true` (HR-50b-2): soma as orgs das instâncias do mapa, marca cada
/// org com a sua instância e marca a instância fora do ar sem derrubar a lista; sem o parâmetro, a
/// resposta é exatamente a de hoje.
#[tokio::test(flavor = "multi_thread")]
async fn listagem_com_cluster_soma_instancias_e_marca_a_fora_do_ar() {
    let (a, b, org, _mapa) = duas_instancias("listagem-cluster", false).await;

    // Sem o parâmetro: formato de sempre (sem `instancias`, sem `instancia` nos itens).
    let normal = pedir(
        Method::GET,
        &format!("{}/admin/v1/orgs", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(normal.status, StatusCode::OK, "{}", normal.cru);
    assert_eq!(normal.corpo["items"], json!([]));
    assert!(normal.corpo["next_cursor"].is_null(), "{}", normal.cru);
    assert!(
        normal.corpo.get("instancias").is_none(),
        "sem `cluster=true` não pode aparecer `instancias`: {}",
        normal.cru
    );

    // Com o parâmetro: a org de `b` aparece marcada com a instância `b`, e as duas instâncias
    // aparecem em `instancias`.
    let com_cluster = pedir(
        Method::GET,
        &format!("{}/admin/v1/orgs?cluster=true", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(com_cluster.status, StatusCode::OK, "{}", com_cluster.cru);
    let itens = com_cluster.corpo["items"]
        .as_array()
        .unwrap_or_else(|| panic!("items: {}", com_cluster.cru));
    assert_eq!(itens.len(), 1, "{}", com_cluster.cru);
    assert_eq!(itens[0]["id"], json!(org.to_string()));
    assert_eq!(itens[0]["instancia"], json!("b"));
    assert!(com_cluster.corpo["next_cursor"].is_null());
    let instancias = com_cluster.corpo["instancias"]
        .as_array()
        .unwrap_or_else(|| panic!("instancias: {}", com_cluster.cru));
    assert_eq!(instancias.len(), 2, "{}", com_cluster.cru);
    let local = instancias
        .iter()
        .find(|instancia| instancia["id"] == json!("a"))
        .expect("instância local no relatório");
    assert_eq!(local["situacao"], json!("local"));
    let vizinha = instancias
        .iter()
        .find(|instancia| instancia["id"] == json!("b"))
        .expect("instância vizinha no relatório");
    assert_eq!(vizinha["situacao"], json!("ok"));
    assert_eq!(vizinha["orgs"], json!(1));
    com_cluster.sem_segredo();

    // Uma org nova direto no banco de `a`: aparece com `instancia: "a"` na mesma lista.
    let local_org = a
        .store
        .orgs()
        .create(NewOrg {
            name: "Org local".to_owned(),
            slug: "org-local-hr50b2".to_owned(),
            config_json: String::new(),
        })
        .expect("cria org local")
        .id;

    // `b` fora do ar: a lista continua, com as orgs de `a` e `b` marcada como inacessível.
    drop(b);
    let com_falha = pedir(
        Method::GET,
        &format!("{}/admin/v1/orgs?cluster=true", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(com_falha.status, StatusCode::OK, "{}", com_falha.cru);
    let itens = com_falha.corpo["items"].as_array().expect("items");
    assert_eq!(itens.len(), 1, "{}", com_falha.cru);
    assert_eq!(itens[0]["id"], json!(local_org.to_string()));
    assert_eq!(itens[0]["instancia"], json!("a"));
    let instancias = com_falha.corpo["instancias"]
        .as_array()
        .expect("instancias");
    let vizinha = instancias
        .iter()
        .find(|instancia| instancia["id"] == json!("b"))
        .expect("instância vizinha no relatório");
    assert_eq!(vizinha["situacao"], json!("inacessivel"));
    assert_eq!(vizinha["orgs"], json!(0));
    assert!(
        vizinha["mensagem"].as_str().unwrap_or_default().len() > 5,
        "a instância fora do ar precisa de mensagem: {}",
        com_falha.cru
    );
    com_falha.sem_segredo();
}

/// `agents_active_count` (CF-03a) atravessa a listagem do cluster: a contagem vem da instância dona
/// da org, no próprio payload do `/admin/v1/orgs` dela — quem lista não faz consulta por org.
#[tokio::test(flavor = "multi_thread")]
async fn listagem_com_cluster_traz_agentes_ativos_de_cada_instancia() {
    let (a, b, org, _mapa) = duas_instancias("agentes-ativos-cluster", false).await;

    // Na dona (`b`): 2 agentes ativos e 1 inativo na org de prova.
    for slug in ["agente-um", "agente-dois"] {
        b.store
            .agents()
            .create(
                org,
                poph_store::NewAgent {
                    name: format!("Agente {slug}"),
                    slug: slug.to_owned(),
                    ..poph_store::NewAgent::default()
                },
            )
            .expect("cria o agente em b");
    }
    let inativo = b
        .store
        .agents()
        .create(
            org,
            poph_store::NewAgent {
                name: "Agente inativo".to_owned(),
                slug: "agente-inativo".to_owned(),
                ..poph_store::NewAgent::default()
            },
        )
        .expect("cria o agente inativo em b");
    let lido = b.store.agents().get(org, inativo.id).expect("lê o agente");
    b.store
        .agents()
        .update(
            org,
            inativo.id,
            lido.version,
            poph_store::AgentChange {
                status: Some(poph_store::Status::Inactive),
                ..poph_store::AgentChange::default()
            },
        )
        .expect("inativa o agente em b");

    // Em `a`: 1 org local com 1 agente ativo. O mapa manda as orgs fora dele para a instância
    // `default_instance` (`b`), então a org local entra no mapa com o `id` dela em `a`.
    let local_org = a
        .store
        .orgs()
        .create(NewOrg {
            name: "Org local".to_owned(),
            slug: "org-local-cf03a".to_owned(),
            config_json: String::new(),
        })
        .expect("cria org local")
        .id;
    a.store
        .agents()
        .create(
            local_org,
            poph_store::NewAgent {
                name: "Agente local".to_owned(),
                slug: "agente-local".to_owned(),
                ..poph_store::NewAgent::default()
            },
        )
        .expect("cria o agente local");

    let com_cluster = pedir(
        Method::GET,
        &format!("{}/admin/v1/orgs?cluster=true", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(com_cluster.status, StatusCode::OK, "{}", com_cluster.cru);
    let itens = com_cluster.corpo["items"]
        .as_array()
        .unwrap_or_else(|| panic!("items: {}", com_cluster.cru));
    let remota = itens
        .iter()
        .find(|item| item["id"] == json!(org.to_string()))
        .unwrap_or_else(|| panic!("org de b fora da listagem: {}", com_cluster.cru));
    assert_eq!(remota["instancia"], json!("b"), "{}", com_cluster.cru);
    assert_eq!(
        remota["agents_active_count"],
        json!(2),
        "a contagem da org de b tem de vir no payload remoto, sem o inativo: {}",
        com_cluster.cru
    );
    let local = itens
        .iter()
        .find(|item| item["id"] == json!(local_org.to_string()))
        .unwrap_or_else(|| panic!("org local fora da listagem: {}", com_cluster.cru));
    assert_eq!(local["instancia"], json!("a"), "{}", com_cluster.cru);
    assert_eq!(
        local["agents_active_count"],
        json!(1),
        "{}",
        com_cluster.cru
    );
    com_cluster.sem_segredo();
}

/// Fluxo mínimo e válido, usado para provar que as rotas do módulo de Fluxos e Fichas passam pelo
/// encaminhamento (o mesmo formato dos testes da API de fluxos).
const FLUXO_DE_PROVA: &str = r#"fluxo(
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

/// As rotas de fluxos e Fichas (FX-27/29/30) são encaminhadas como as demais: quem responde é a
/// instância dona da org, e nada é gravado na instância que só repassa.
#[tokio::test(flavor = "multi_thread")]
async fn rotas_de_fluxos_e_fichas_sao_encaminhadas_para_a_dona() {
    let (a, b, org, _mapa) = duas_instancias("fluxos-fichas", false).await;
    let raiz = format!("/admin/v1/orgs/{org}");

    // `a` não tem a org: se atendesse localmente, tudo abaixo sairia 404.
    assert!(
        a.store.orgs().by_slug("org-da-instancia-b").is_err(),
        "a org não pode existir no banco de `a`"
    );

    // 1. Leitura pura: validar o script (FX-27) responde igual nas duas instâncias.
    let corpo = json!({"source": FLUXO_DE_PROVA}).to_string();
    let validado_na_dona = pedir(
        Method::POST,
        &format!("{}{raiz}/flows/validate", b.base),
        Some(TOKEN_GLOBAL),
        Some(corpo.as_bytes()),
        &[],
    )
    .await;
    assert_eq!(
        validado_na_dona.status,
        StatusCode::OK,
        "{}",
        validado_na_dona.cru
    );
    assert_eq!(validado_na_dona.corpo["valid"], json!(true));
    let validado_via_a = pedir(
        Method::POST,
        &format!("{}{raiz}/flows/validate", a.base),
        Some(TOKEN_GLOBAL),
        Some(corpo.as_bytes()),
        &[],
    )
    .await;
    assert_eq!(
        validado_via_a.status,
        StatusCode::OK,
        "{}",
        validado_via_a.cru
    );
    assert_eq!(validado_via_a.corpo, validado_na_dona.corpo);

    // 2. Escrita: criar e publicar o fluxo por `a` grava em `b` (e só lá).
    let criado = pedir(
        Method::POST,
        &format!("{}{raiz}/flows", a.base),
        Some(TOKEN_GLOBAL),
        Some(
            json!({"source": FLUXO_DE_PROVA, "publish": true})
                .to_string()
                .as_bytes(),
        ),
        &[],
    )
    .await;
    assert_eq!(criado.status, StatusCode::CREATED, "{}", criado.cru);
    assert_eq!(criado.corpo["slug"], json!("agendar_visita"));
    assert!(
        b.store
            .flows()
            .by_slug(org, "agendar_visita")
            .expect("consulta em b")
            .is_some(),
        "o fluxo criado por `a` tem de existir em `b`"
    );
    assert!(
        a.store
            .flows()
            .by_slug(org, "agendar_visita")
            .expect("consulta em a")
            .is_none(),
        "`a` não grava nada: a org nem existe no banco dela"
    );

    // 3. Atendimento: agente e conversa de teste criados por `a` viram Ficha em `b`.
    let agente = pedir(
        Method::POST,
        &format!("{}{raiz}/agents", a.base),
        Some(TOKEN_GLOBAL),
        Some(
            json!({"name": "Atendente fictício", "slug": "atendente-ficticio"})
                .to_string()
                .as_bytes(),
        ),
        &[],
    )
    .await;
    assert_eq!(agente.status, StatusCode::CREATED, "{}", agente.cru);
    let agente_id = agente.corpo["id"]
        .as_str()
        .expect("id do agente")
        .to_owned();
    let conversa = pedir(
        Method::POST,
        &format!("{}{raiz}/agents/{agente_id}/test-chat", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(conversa.status, StatusCode::CREATED, "{}", conversa.cru);

    // 4. Leitura da Ficha por `a` devolve o que a dona tem.
    let fichas_via_a = pedir(
        Method::GET,
        &format!("{}{raiz}/fichas", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(fichas_via_a.status, StatusCode::OK, "{}", fichas_via_a.cru);
    let itens = fichas_via_a.corpo["items"]
        .as_array()
        .expect("lista de Fichas");
    assert_eq!(itens.len(), 1, "a conversa de teste abriu uma Ficha");
    let ficha = itens[0]["id"].as_str().expect("id da Ficha").to_owned();
    let fichas_na_dona = pedir(
        Method::GET,
        &format!("{}{raiz}/fichas", b.base),
        Some(TOKEN_GLOBAL),
        None,
        &[],
    )
    .await;
    assert_eq!(fichas_na_dona.corpo, fichas_via_a.corpo);

    // 5. Escrita humana na Ficha (FX-30): fechar por `a` fecha em `b`.
    let fechada = pedir(
        Method::POST,
        &format!("{}{raiz}/fichas/{ficha}/close", a.base),
        Some(TOKEN_GLOBAL),
        None,
        &[("if-match", "1".to_owned())],
    )
    .await;
    assert_eq!(fechada.status, StatusCode::OK, "{}", fechada.cru);
    assert_eq!(fechada.corpo["status"], json!("closed"));
    let na_dona = b
        .store
        .cases()
        .get(org, ficha.parse().expect("id da Ficha"))
        .expect("Ficha em b");
    assert_eq!(na_dona.status, RecordStatus::Closed);
    assert_eq!(
        b.store
            .cases()
            .list(org, &Default::default(), Page::default())
            .expect("lista em b")
            .len(),
        1
    );
    assert_eq!(
        b.store
            .conversations()
            .list(org, None, Page::default())
            .expect("conversas em b")
            .len(),
        1,
        "a conversa também fecha, mas fica no histórico"
    );

    // Nada disso existe no banco de `a`: tudo foi gravado pela dona, via encaminhamento.
    assert!(
        a.store.orgs().get(org).is_err(),
        "a org não pode passar a existir em `a`"
    );
}
