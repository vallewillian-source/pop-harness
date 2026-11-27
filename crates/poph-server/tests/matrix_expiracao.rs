//! Expiração da mídia na sala (MX-46): fim a fim, com o banco de verdade e um Synapse falso.
//!
//! O que este arquivo prova, e por quê:
//!
//! - **antes do prazo nada acontece**: com a mídia mais nova que `matrix.midia_retencao_dias`, a
//!   passagem não faz chamada nenhuma e a linha continua pendente;
//! - **depois do prazo, o evento é redigido e o arquivo apagado — uma vez só**: a segunda passagem
//!   no mesmo instante não repete chamada (cada etapa tem carimbo no banco), e o que sai na sala é o
//!   bloco que o Element lê;
//! - **a redação que falha não impede o apagamento**: a retenção vale mais do que a arrumação da
//!   conversa, e a próxima passagem tenta só o que faltou;
//! - **falha não vira laço apertado**: o item adiado não volta antes da hora e a espera dobra;
//! - **org com o módulo desligado não é tocada**;
//! - **o token de administração não aparece no log** (valor-canário), nem quando a chamada falha;
//! - **sem o token a rotina fica desligada com aviso claro**, e sem token nenhum não há chamada.
//!
//! O relógio é injetado ([`ExpiracaoMidia::com_relogio`]): o teste simula dias sem dormir. O Synapse
//! é falso e com estado (deduplica redação por `txnId`, exige o token de administrador na API admin
//! e responde o status que o teste pedir). O banco é o de verdade, numa pasta temporária.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_matrix::client::{ClientConfig, MatrixClient};
use poph_matrix::config::VAR_ADMIN_TOKEN;
use poph_matrix::expiration::{CATEGORY_FIELD, EXPIRED_BLOCK, NAME_FIELD, SENT_AT_FIELD};
use poph_server::matrix_beginning::module_expiry;
use poph_server::matrix_expiry::{AssetExpiry, ExpiryReport};
use poph_server::matrix_metrics::MetricsMatrix;
use poph_store::{
    CategoryAsset, Conversa, NewAgent, NewAsset, NewConversation, NewOrg, Org, Store,
};
use poph_types::{AttachmentId, MatrixConfig, Secret};
use serde_json::{Value, json};
use tracing_subscriber::fmt::MakeWriter;

// ───────────────────────────────── constantes ─────────────────────────────────

const PREFIXO: &str = "ph";
const SERVIDOR: &str = "matrix-dev.local";
const SALA: &str = "!sala-mx46:matrix-dev.local";
const AGENTE: &str = "agt_1";
const AS_TOKEN: &str = "as-token-mx46";
/// Valor-canário do token de administração: se aparecer em log, o teste acusa.
const ADMIN_TOKEN: &str = "CANARIO-ADMIN-mx46-c0f1-TAO-SECRETO";

const DIA: i64 = 86_400_000;

// ───────────────────────────────── captura de logs ─────────────────────────────────

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

/// Um coletor de logs para o processo de testes inteiro (o subscriber é global).
fn logs_capturados() -> Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
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

fn texto_dos_logs() -> String {
    let logs = logs_capturados();
    let guarda = logs.lock().unwrap();
    String::from_utf8_lossy(&guarda).into_owned()
}

// ───────────────────────────────── Synapse falso ─────────────────────────────────

#[derive(Debug, Clone)]
struct Redacao {
    room: String,
    event: String,
    txn: String,
    como: Option<String>,
    body: Value,
}

#[derive(Clone, Default)]
struct Falso {
    redacoes: Arc<Mutex<Vec<Redacao>>>,
    apagadas: Arc<Mutex<Vec<(String, String)>>>,
    autorizacoes: Arc<Mutex<Vec<String>>>,
    chamadas: Arc<AtomicUsize>,
    /// Status que a próxima redação responde (uma vez). `None` = aceita.
    falha_da_redacao: Arc<Mutex<Option<u16>>>,
    /// Status que o próximo apagamento responde (uma vez). `None` = aceita.
    falha_do_apagamento: Arc<Mutex<Option<u16>>>,
}

fn responder(status: u16, body: Value) -> Response {
    (
        StatusCode::from_u16(status).expect("status válido"),
        [("content-type", "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// Percent-decoding de um valor da consulta (o `user_id` chega com `%40` e `%3A`).
fn decodificar(valor: &str) -> String {
    let bytes = valor.as_bytes();
    let mut saida = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                saida.push(byte);
                i += 3;
                continue;
            }
        }
        saida.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&saida).into_owned()
}

async fn atender(
    State(falso): State<Falso>,
    metodo: Method,
    uri: Uri,
    cabecalhos: HeaderMap,
    body: String,
) -> Response {
    falso.chamadas.fetch_add(1, Ordering::SeqCst);
    let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let consulta: HashMap<String, String> = uri
        .query()
        .map(|bruta| {
            bruta
                .split('&')
                .filter_map(|par| par.split_once('='))
                .map(|(chave, valor)| (chave.to_owned(), decodificar(valor)))
                .collect()
        })
        .unwrap_or_default();
    let autorizacao = cabecalhos
        .get("authorization")
        .and_then(|valor| valor.to_str().ok())
        .unwrap_or_default()
        .to_owned();

    // API administrativa: `DELETE /_synapse/admin/v1/media/{servidor}/{id}`.
    if metodo == Method::DELETE && uri.path().starts_with("/_synapse/admin/v1/media/") {
        let partes: Vec<&str> = uri
            .path()
            .trim_start_matches("/_synapse/admin/v1/media/")
            .split('/')
            .collect();
        falso.autorizacoes.lock().unwrap().push(autorizacao.clone());
        if autorizacao != format!("Bearer {ADMIN_TOKEN}") {
            return responder(403, json!({ "error": "You are not a server admin" }));
        }
        if let Some(status) = falso.falha_do_apagamento.lock().unwrap().take() {
            return responder(status, json!({ "error": "indisponível" }));
        }
        if partes.len() != 2 {
            return responder(400, json!({ "error": "caminho de mídia inválido" }));
        }
        falso
            .apagadas
            .lock()
            .unwrap()
            .push((partes[0].to_owned(), partes[1].to_owned()));
        return responder(200, json!({ "deleted_media": [partes[1]] }));
    }

    // API cliente: `PUT /rooms/{sala}/redact/{evento}/{txn}`.
    if metodo == Method::PUT && uri.path().contains("/redact/") {
        if autorizacao != format!("Bearer {AS_TOKEN}") {
            return responder(401, json!({ "errcode": "M_UNKNOWN_TOKEN" }));
        }
        if let Some(status) = falso.falha_da_redacao.lock().unwrap().take() {
            return responder(
                status,
                json!({ "errcode": "M_UNKNOWN", "error": "indisponível" }),
            );
        }
        let partes: Vec<&str> = uri
            .path()
            .trim_start_matches("/_matrix/client/v3/rooms/")
            .split('/')
            .collect();
        if partes.len() != 4 || partes[1] != "redact" {
            return responder(404, json!({ "errcode": "M_UNRECOGNIZED" }));
        }
        let mut redacoes = falso.redacoes.lock().unwrap();
        let new = !redacoes
            .iter()
            .any(|r| r.room == partes[0] && r.txn == partes[3]);
        if new {
            redacoes.push(Redacao {
                room: partes[0].to_owned(),
                event: partes[2].to_owned(),
                txn: partes[3].to_owned(),
                como: consulta.get("user_id").cloned(),
                body,
            });
        }
        let posicao = redacoes.len();
        return responder(
            200,
            json!({ "event_id": format!("$redacao-{posicao}:matrix-dev.local") }),
        );
    }

    responder(
        404,
        json!({ "errcode": "M_UNRECOGNIZED", "error": "rota não existe no Synapse falso" }),
    )
}

struct Synapse {
    url: String,
    falso: Falso,
    desligar: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Synapse {
    async fn subir() -> Self {
        let falso = Falso::default();
        let app = Router::new().fallback(atender).with_state(falso.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let url = format!("http://{}", listener.local_addr().expect("endereço"));
        let (desligar, sinal) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = sinal.await;
                })
                .await;
        });
        Self {
            url,
            falso,
            desligar: Some(desligar),
        }
    }

    fn client(&self) -> MatrixClient {
        MatrixClient::new(
            ClientConfig::new(
                self.url.clone(),
                SERVIDOR.to_owned(),
                Secret::new(AS_TOKEN.to_owned()),
            )
            .expect("config do cliente"),
        )
        .expect("cliente do Matrix")
    }

    fn redacoes(&self) -> Vec<Redacao> {
        self.falso.redacoes.lock().unwrap().clone()
    }

    fn apagadas(&self) -> Vec<(String, String)> {
        self.falso.apagadas.lock().unwrap().clone()
    }

    fn autorizacoes(&self) -> Vec<String> {
        self.falso.autorizacoes.lock().unwrap().clone()
    }

    fn chamadas(&self) -> usize {
        self.falso.chamadas.load(Ordering::SeqCst)
    }

    fn recusar_redacao_com(&self, status: u16) {
        *self.falso.falha_da_redacao.lock().unwrap() = Some(status);
    }

    fn recusar_apagamento_com(&self, status: u16) {
        *self.falso.falha_do_apagamento.lock().unwrap() = Some(status);
    }
}

impl Drop for Synapse {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
    }
}

// ───────────────────────────────── cenário ─────────────────────────────────

struct Cenario {
    pasta: PathBuf,
    relogio: Arc<AtomicI64>,
    store: Option<Arc<Store>>,
    org: Org,
    conversation: Conversa,
    synapse: Synapse,
}

impl Cenario {
    async fn new(rotulo: &str, agora: i64) -> Self {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta = std::env::temp_dir().join(format!(
            "poph-expiracao-{rotulo}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("pasta do teste");
        let relogio = Arc::new(AtomicI64::new(agora));
        let agora_do_store = Arc::clone(&relogio);
        let store = Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db"))
            .expect("abre o banco")
            .with_clock(Arc::new(move || agora_do_store.load(Ordering::SeqCst)));
        let store = Arc::new(store);

        let org = store
            .orgs()
            .create(NewOrg {
                name: "Loja MX-46".to_owned(),
                slug: "loja-mx46".to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org");
        let agent = store
            .agents()
            .create(
                org.id,
                NewAgent {
                    name: "Atendente".to_owned(),
                    slug: "atendente".to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente");
        let conversation = store
            .conversations()
            .create(
                org.id,
                NewConversation {
                    agent_id: agent.id,
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some("5511999990000".to_owned()),
                    target_department: None,
                },
            )
            .expect("cria a conversa");
        store.matrix().enable(org.id).expect("liga o módulo");
        store
            .matrix()
            .ensure_room(org.id, conversation.id, SALA, None)
            .expect("grava a sala");
        store
            .matrix()
            .mark_room_ready(org.id, conversation.id)
            .expect("sala pronta");

        logs_capturados();
        Self {
            pasta,
            relogio,
            store: Some(store),
            org,
            conversation,
            synapse: Synapse::subir().await,
        }
    }

    fn store(&self) -> &Arc<Store> {
        self.store.as_ref().expect("store viva")
    }

    fn agora(&self) -> i64 {
        self.relogio.load(Ordering::SeqCst)
    }

    fn avancar(&self, ms: i64) {
        self.relogio.fetch_add(ms, Ordering::SeqCst);
    }

    /// Grava uma mídia publicada neste cenário, com o carimbo escolhido.
    fn publicar(&self, name: &str, sent_at: i64) -> AttachmentId {
        let id = AttachmentId::generate();
        let category = if name.ends_with(".jpg") {
            CategoryAsset::Image
        } else {
            CategoryAsset::Document
        };
        let mime = if category == CategoryAsset::Image {
            "image/jpeg"
        } else {
            "application/pdf"
        };
        let registro = NewAsset {
            id,
            conversation: self.conversation.id,
            agent: AGENTE.to_owned(),
            room: SALA.to_owned(),
            evento: format!("$midia-{id}:matrix-dev.local"),
            mxc: format!("mxc://{SERVIDOR}/media-{id}"),
            nome: name.to_owned(),
            mime: mime.to_owned(),
            category,
            size_bytes: 2_048,
            sent_at,
        };
        assert!(
            self.store()
                .matrix_asset()
                .register(self.org.id, &registro)
                .expect("grava a mídia"),
            "a primeira gravação é nova"
        );
        id
    }

    fn config(&self) -> MatrixConfig {
        MatrixConfig {
            enabled: true,
            homeserver_url: self.synapse.url.clone(),
            server_name: SERVIDOR.to_owned(),
            prefix: PREFIXO.to_owned(),
            attachment_retention_days: 4,
            ..MatrixConfig::default()
        }
    }

    /// O serviço de expiração, com o relógio do cenário e esperas curtas para o teste não dormir.
    fn servico(&self) -> AssetExpiry {
        let relogio = Arc::clone(&self.relogio);
        AssetExpiry::new(
            Arc::clone(self.store()),
            self.synapse.client(),
            Secret::new(ADMIN_TOKEN.to_owned()),
            &self.config(),
        )
        .expect("monta a expiração")
        .with_clock(Arc::new(move || relogio.load(Ordering::SeqCst)))
    }

    fn pendentes(&self) -> u32 {
        self.store()
            .matrix_asset()
            .org_pending(self.org.id)
            .expect("conta as pendentes")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

// ───────────────────────────────── testes ─────────────────────────────────

#[tokio::test]
async fn antes_do_prazo_nada_acontece() {
    let cenario = Cenario::new("antes", 1_000 * DIA).await;
    // Três dias: dentro da retenção de quatro.
    let id = cenario.publicar("laudo.pdf", cenario.agora() - 3 * DIA);

    let relato = cenario.servico().pass().await;
    assert_eq!(relato, ExpiryReport::default(), "{relato:?}");
    assert_eq!(
        cenario.synapse.chamadas(),
        0,
        "antes do prazo não há chamada nenhuma"
    );
    assert!(cenario.synapse.redacoes().is_empty());
    assert!(cenario.synapse.apagadas().is_empty());
    assert_eq!(cenario.pendentes(), 1, "a linha continua pendente");
    let linha = cenario
        .store()
        .matrix_asset()
        .pending(cenario.org.id, cenario.agora(), cenario.agora(), 10)
        .expect("lê a fila");
    assert_eq!(linha[0].id, id);
    assert!(linha[0].still_to_draft() && linha[0].still_to_delete());
}

#[tokio::test]
async fn depois_do_prazo_redige_e_apaga_uma_vez_so() {
    let cenario = Cenario::new("depois", 1_000 * DIA).await;
    let sent_at = cenario.agora() - 5 * DIA;
    let id = cenario.publicar("laudo.pdf", sent_at);
    let servico = cenario.servico();

    let relato = servico.pass().await;
    assert_eq!(relato.expired, 1);
    assert_eq!(relato.redacted, 1);
    assert_eq!(relato.deleted, 1);
    assert_eq!(relato.deferred, 0);

    // A redação: como o fantasma do agente, no evento certo, com o bloco do frontend.
    let redacoes = cenario.synapse.redacoes();
    assert_eq!(redacoes.len(), 1, "{redacoes:?}");
    assert_eq!(redacoes[0].room, SALA);
    assert_eq!(redacoes[0].event, format!("$midia-{id}:matrix-dev.local"));
    assert_eq!(
        redacoes[0].como.as_deref(),
        Some("@ph_ag_agt_1:matrix-dev.local"),
        "quem redige é o fantasma do agente (moderador da sala)"
    );
    let bloco = &redacoes[0].body[EXPIRED_BLOCK];
    assert_eq!(bloco[NAME_FIELD], json!("laudo.pdf"));
    assert_eq!(bloco[CATEGORY_FIELD], json!("document"));
    // 1.000 dias antes: o carimbo em ISO-8601 é o que o Element formata como "dd/mm".
    assert_eq!(bloco[SENT_AT_FIELD], json!("1972-09-22T00:00:00Z"));

    // O apagamento: pela API admin, com o token de administrador.
    assert_eq!(
        cenario.synapse.apagadas(),
        vec![(SERVIDOR.to_owned(), format!("media-{id}"))]
    );
    for autorizacao in cenario.synapse.autorizacoes() {
        assert_eq!(autorizacao, format!("Bearer {ADMIN_TOKEN}"));
    }

    // A segunda passagem no mesmo instante não repete nada: os dois carimbos estão no banco.
    let chamadas = cenario.synapse.chamadas();
    let segunda = servico.pass().await;
    assert_eq!(segunda, ExpiryReport::default(), "{segunda:?}");
    assert_eq!(
        cenario.synapse.chamadas(),
        chamadas,
        "mídia completa não volta ao Synapse"
    );
    assert_eq!(cenario.pendentes(), 0);
}

#[tokio::test]
async fn a_redacao_que_falha_nao_impede_apagar_e_o_item_volta_depois() {
    let cenario = Cenario::new("redacao-falha", 1_000 * DIA).await;
    cenario.publicar("foto.jpg", cenario.agora() - 5 * DIA);
    let servico = cenario.servico();
    cenario.synapse.recusar_redacao_com(500);

    let relato = servico.pass().await;
    assert_eq!(relato.redacted, 0, "{relato:?}");
    assert_eq!(relato.deleted, 1, "a retenção vale mais do que a redação");
    assert_eq!(relato.deferred, 1);
    assert!(cenario.synapse.redacoes().is_empty());
    assert_eq!(cenario.synapse.apagadas().len(), 1);
    assert_eq!(cenario.pendentes(), 1, "falta só redigir");

    // No mesmo instante o item não volta (espera crescente).
    let chamadas = cenario.synapse.chamadas();
    let segunda = servico.pass().await;
    assert_eq!(segunda.expired, 0, "{segunda:?}");
    assert_eq!(cenario.synapse.chamadas(), chamadas);

    // Passada a primeira espera, tenta de novo — e agora dá certo.
    cenario.avancar(61_000);
    let terceira = servico.pass().await;
    assert_eq!(terceira.redacted, 1, "{terceira:?}");
    assert_eq!(terceira.deleted, 0, "o arquivo já tinha sido apagado");
    assert_eq!(cenario.synapse.redacoes().len(), 1);
    assert_eq!(cenario.pendentes(), 0);
}

#[tokio::test]
async fn falha_no_apagamento_nao_vira_laco_apertado_e_a_espera_cresce() {
    let cenario = Cenario::new("espera", 1_000 * DIA).await;
    cenario.publicar("laudo.pdf", cenario.agora() - 5 * DIA);
    let servico = cenario.servico();
    cenario.synapse.recusar_apagamento_com(503);

    // Primeira passagem: redige e falha ao apagar.
    let primeira = servico.pass().await;
    assert_eq!(primeira.redacted, 1, "{primeira:?}");
    assert_eq!(primeira.deleted, 0);
    assert_eq!(primeira.deferred, 1);
    assert_eq!(cenario.synapse.apagadas().len(), 0);

    // Três passagens no mesmo instante: nenhuma chamada nova (o item está adiado).
    let depois_da_primeira = cenario.synapse.chamadas();
    for _ in 0..3 {
        let relato = servico.pass().await;
        assert_eq!(relato.expired, 0, "{relato:?}");
    }
    assert_eq!(
        cenario.synapse.chamadas(),
        depois_da_primeira,
        "o item adiado não é chamado de novo antes da hora"
    );

    // A primeira espera é de 60 s; o item volta só depois dela.
    cenario.avancar(61_000);
    cenario.synapse.recusar_apagamento_com(503);
    let segunda = servico.pass().await;
    assert_eq!(segunda.expired, 1, "{segunda:?}");
    assert_eq!(segunda.deferred, 1);

    // Depois da segunda falha a espera dobra: 60 s não bastam mais, 120 s bastam.
    cenario.avancar(61_000);
    let cedo = servico.pass().await;
    assert_eq!(cedo.expired, 0, "a espera dobrou: {cedo:?}");
    cenario.avancar(60_000);
    let na_hora = servico.pass().await;
    assert_eq!(
        na_hora.expired, 1,
        "na hora marcada o item volta: {na_hora:?}"
    );
}

#[tokio::test]
async fn org_com_o_modulo_desligado_nao_e_tocada() {
    let cenario = Cenario::new("desligada", 1_000 * DIA).await;
    cenario.publicar("laudo.pdf", cenario.agora() - 30 * DIA);
    cenario
        .store()
        .matrix()
        .disable(cenario.org.id)
        .expect("desliga o módulo na org");

    let relato = cenario.servico().pass().await;
    assert_eq!(relato, ExpiryReport::default(), "{relato:?}");
    assert_eq!(cenario.synapse.chamadas(), 0);
    assert_eq!(cenario.pendentes(), 1, "a linha fica como estava");
}

#[tokio::test]
async fn o_token_de_administrador_nao_aparece_no_log() {
    let cenario = Cenario::new("canario", 1_000 * DIA).await;
    cenario.publicar("laudo.pdf", cenario.agora() - 5 * DIA);
    let servico = cenario.servico();
    // Falhas nas duas etapas: o caminho que mais escreve log.
    cenario.synapse.recusar_redacao_com(403);
    cenario.synapse.recusar_apagamento_com(403);

    let relato = servico.pass().await;
    assert_eq!(relato.deferred, 1, "{relato:?}");

    // O serviço também não mostra o token em `Debug`.
    assert!(!format!("{servico:?}").contains(ADMIN_TOKEN));

    let logs = texto_dos_logs();
    assert!(
        logs.contains("matrix_expiracao"),
        "o log da rotina saiu: {logs}"
    );
    assert!(
        !logs.contains(ADMIN_TOKEN),
        "o token de administração vazou no log"
    );
    assert!(
        !logs.contains("Bearer"),
        "nenhum cabeçalho de autorização no log"
    );
}

#[tokio::test]
async fn sem_token_a_rotina_fica_desligada_com_aviso_e_sem_chamada() {
    let cenario = Cenario::new("sem-token", 1_000 * DIA).await;
    cenario.publicar("laudo.pdf", cenario.agora() - 5 * DIA);

    let sem_token = module_expiry(
        Arc::clone(cenario.store()),
        cenario.synapse.client(),
        &cenario.config(),
        None,
        None,
    );
    assert!(sem_token.is_none(), "sem token a rotina não existe");

    let logs = texto_dos_logs();
    assert!(
        logs.contains("expiração da mídia desligada"),
        "o log avisa que a rotina está desligada: {logs}"
    );
    assert!(
        logs.contains(VAR_ADMIN_TOKEN),
        "o aviso diz qual variável falta: {logs}"
    );
    assert_eq!(
        cenario.synapse.chamadas(),
        0,
        "sem token não há chamada ao Synapse"
    );

    // Com o token, a rotina existe e funciona.
    let com_token = module_expiry(
        Arc::clone(cenario.store()),
        cenario.synapse.client(),
        &cenario.config(),
        None,
        Some(Secret::new(ADMIN_TOKEN.to_owned())),
    )
    .expect("com token a rotina é montada");
    let relogio = Arc::clone(&cenario.relogio);
    let com_token = com_token.with_clock(Arc::new(move || relogio.load(Ordering::SeqCst)));
    let relato = com_token.pass().await;
    assert_eq!(relato.redacted, 1, "{relato:?}");
    assert_eq!(relato.deleted, 1);
}

#[tokio::test]
async fn as_metricas_do_modulo_contam_por_entrada() {
    use poph_types::Registry;

    let cenario = Cenario::new("metricas", 1_000 * DIA).await;
    cenario.publicar("laudo.pdf", cenario.agora() - 5 * DIA);
    cenario.publicar("foto.jpg", cenario.agora() - 6 * DIA);
    cenario.publicar("novo.pdf", cenario.agora() - DIA);

    let registro = Registry::new();
    let metricas = Arc::new(MetricsMatrix::register(&registro).expect("registra"));
    let relogio = Arc::clone(&cenario.relogio);
    let servico = cenario
        .servico()
        .with_metrics(Arc::clone(&metricas))
        .with_clock(Arc::new(move || relogio.load(Ordering::SeqCst)));

    let relato = servico.pass().await;
    assert_eq!(relato.redacted, 2, "{relato:?}");
    assert_eq!(relato.deleted, 2);

    let text = registro.render();
    assert!(
        text.contains("poph_matrix_midias_redigidas_total 2\n"),
        "{text}"
    );
    assert!(
        text.contains("poph_matrix_midias_apagadas_total 2\n"),
        "{text}"
    );
    // A que ainda não venceu fica contada como pendente.
    assert!(
        text.contains("poph_matrix_midias_pendentes_expiracao 1\n"),
        "{text}"
    );
    assert!(
        text.contains("poph_matrix_midias_expiracao_adiadas_total 0\n"),
        "{text}"
    );
}

#[tokio::test]
async fn o_prazo_configurado_manda_no_corte() {
    let cenario = Cenario::new("prazo", 1_000 * DIA).await;
    cenario.publicar("laudo.pdf", cenario.agora() - 2 * DIA);
    let relogio = Arc::clone(&cenario.relogio);

    // Um dia de retenção: a mídia de dois dias já venceu.
    let mut config = cenario.config();
    config.attachment_retention_days = 1;
    let servico = AssetExpiry::new(
        Arc::clone(cenario.store()),
        cenario.synapse.client(),
        Secret::new(ADMIN_TOKEN.to_owned()),
        &config,
    )
    .expect("monta")
    .with_clock(Arc::new(move || relogio.load(Ordering::SeqCst)));

    let relato = servico.pass().await;
    assert_eq!(relato.redacted, 1, "{relato:?}");

    // Com o padrão de quatro dias, a mesma mídia estaria dentro do prazo.
    let outro = Cenario::new("prazo-padrao", 1_000 * DIA).await;
    let id = outro.publicar("laudo2.pdf", outro.agora() - 2 * DIA);
    let relogio = Arc::clone(&outro.relogio);
    let padrao = AssetExpiry::new(
        Arc::clone(outro.store()),
        outro.synapse.client(),
        Secret::new(ADMIN_TOKEN.to_owned()),
        &outro.config(),
    )
    .expect("monta")
    .with_clock(Arc::new(move || relogio.load(Ordering::SeqCst)));
    assert_eq!(padrao.pass().await.redacted, 0);
    let pendente = outro
        .store()
        .matrix_asset()
        .pending(outro.org.id, outro.agora(), outro.agora(), 10)
        .expect("lê a fila");
    assert_eq!(pendente[0].id, id);
}

/// O laço de fundo para no desligamento, como os outros do módulo.
#[tokio::test]
async fn o_laco_de_fundo_para_quando_pedido() {
    let cenario = Cenario::new("parada", 1_000 * DIA).await;
    let servico = Arc::new(cenario.servico().with_period(Duration::from_millis(5)));
    let tarefa = tokio::spawn(Arc::clone(&servico).run());
    tokio::time::sleep(Duration::from_millis(20)).await;
    servico.ask_stop();
    tokio::time::timeout(Duration::from_secs(2), tarefa)
        .await
        .expect("o laço termina no desligamento")
        .expect("a tarefa não estoura");
}
