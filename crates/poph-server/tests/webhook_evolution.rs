//! Entrada do WhatsApp pelo webhook da Evolution (HR-26b), de ponta a ponta.
//!
//! Servidor de verdade (porta efêmera) sobre banco temporário, cliente HTTP de verdade e uma
//! Evolution falsa em `127.0.0.1` que só responde o download de mídia. Nada de rede externa e
//! nenhum número de WhatsApp.
//!
//! O que se prova: texto grava inbox, conversa e mensagem; reentrega não duplica nem baixa mídia
//! de novo; segredo ausente ou errado dá 401 sem gravar nada; instância não configurada dá 404;
//! integração inexistente dá 404 para mensagem e 200 sem criar linha para estado; estado de
//! conexão atualiza a integração; imagem vira anexo com blob no disco; mídia expirada mantém o
//! texto; falha de gravação dá 503 e a reentrega depois entra inteira; a configuração resolve os
//! segredos pelo nome da variável.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::post;
use poph_evolution::{ClienteEvolution, ConfigEvolution, TETO_MIDIA};
use poph_server::Estado;
use poph_server::evolution::{CanalEvolution, canais_da_configuracao};
use poph_store::{NewAgent, NewOrg, Page, Store};
use poph_types::config::{EvolutionConfig, EvolutionInstanceConfig};
use poph_types::{AgentId, HarnessConfig, IntegrationId, MessageId, OrgId, Secret};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tracing_subscriber::fmt::MakeWriter;

/// Segredo do webhook usado nos testes (só no ambiente de teste).
const SEGREDO: &str = "segredo-webhook-teste-hr26b";
/// Chave da Evolution usada nos testes (só para o download de mídia).
const CHAVE_EVO: &str = "chave-evo-teste-123";
/// Número de cliente falso. Não é de ninguém.
const TELEFONE: &str = "5511999990000";
/// 1×1 PNG, o menor arquivo de imagem válido que serve para o teste.
const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

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

/// Instala uma vez, no processo de testes, o coletor de logs. Todo teste deste arquivo usa o mesmo.
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

// ───────────────────────── Evolution falsa (só o download de mídia) ─────────────────────────

#[derive(Clone)]
struct EvolucaoFalsa {
    downloads: Arc<AtomicUsize>,
}

async fn baixar_midia_falsa(
    State(evo): State<EvolucaoFalsa>,
    Path(instancia): Path<String>,
    Json(corpo): Json<Value>,
) -> (StatusCode, Json<Value>) {
    evo.downloads.fetch_add(1, Ordering::SeqCst);
    if instancia != "loja" {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"message": "instância desconhecida"})),
        );
    }
    match corpo["message"]["key"]["id"].as_str().unwrap_or_default() {
        "MSG-IMG-1" => (
            StatusCode::OK,
            Json(json!({"base64": PNG_B64, "mimetype": "image/png"})),
        ),
        // A Evolution não guardou mais a mídia: 404 com este texto vira "mídia expirada".
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({"status": 404, "error": "Not Found", "message": "Message not found"})),
        ),
    }
}

async fn subir_evolucao_falsa() -> (String, Arc<AtomicUsize>) {
    let downloads = Arc::new(AtomicUsize::new(0));
    let roteador = Router::new()
        .route(
            "/chat/getBase64FromMediaMessage/{instancia}",
            post(baixar_midia_falsa),
        )
        .with_state(EvolucaoFalsa {
            downloads: Arc::clone(&downloads),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera da Evolution falsa");
    let endereco = listener.local_addr().expect("endereço da Evolution falsa");
    tokio::spawn(async move {
        let _ = axum::serve(listener, roteador).await;
    });
    (format!("http://{endereco}"), downloads)
}

// ───────────────────────── cenário: servidor no ar, org/agente/integração prontos ─────────────────────────

struct Cenario {
    base: String,
    cliente: Client,
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    integracao: IntegrationId,
    downloads: Arc<AtomicUsize>,
    desligar: Option<oneshot::Sender<()>>,
}

/// Resposta do webhook: status, corpo JSON (ou texto cru) e o texto exato, para checar vazamento.
struct Resposta {
    status: StatusCode,
    corpo: Value,
    cru: String,
}

impl Cenario {
    async fn novo(rotulo: &str) -> Self {
        logs_capturados();
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-webhook-evo-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        config.server.port = 7480;
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Loja Webhook".to_owned(),
                slug: "loja-webhook".to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Atendente Loja".to_owned(),
                    slug: "atendente-loja".to_owned(),
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

        let (url_evo, downloads) = subir_evolucao_falsa().await;
        let cliente_evo = ClienteEvolution::novo(
            ConfigEvolution::novo(url_evo, "loja", Secret::new(CHAVE_EVO.to_owned()))
                .expect("config da Evolution falsa"),
        )
        .expect("cliente da Evolution falsa");

        let canais = vec![
            CanalEvolution {
                org,
                instancia: "loja".to_owned(),
                integracao,
                segredo: Some(Secret::new(SEGREDO.to_owned())),
                cliente: Some(cliente_evo),
                teto_midia: TETO_MIDIA,
            },
            // Instância cuja integração não existe no banco.
            CanalEvolution {
                org,
                instancia: "orfa".to_owned(),
                integracao: IntegrationId::generate(),
                segredo: Some(Secret::new(SEGREDO.to_owned())),
                cliente: None,
                teto_midia: TETO_MIDIA,
            },
            // Instância sem segredo no ambiente: sempre 401.
            CanalEvolution {
                org,
                instancia: "sem-segredo".to_owned(),
                integracao,
                segredo: None,
                cliente: None,
                teto_midia: TETO_MIDIA,
            },
        ];
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
            downloads,
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

    async fn webhook(
        &self,
        instancia: &str,
        org: &str,
        segredo: Option<&str>,
        corpo: Value,
    ) -> Resposta {
        let mut pedido = self
            .cliente
            .post(format!("{}/webhook/evolution/{org}/{instancia}", self.base));
        if let Some(segredo) = segredo {
            pedido = pedido.header("X-Evolution-Webhook-Secret", segredo);
        }
        let resposta = pedido
            .json(&corpo)
            .send()
            .await
            .expect("resposta do webhook");
        let status = resposta.status();
        let cru = resposta.text().await.expect("corpo da resposta");
        let corpo = serde_json::from_str(&cru).unwrap_or(Value::Null);
        Resposta { status, corpo, cru }
    }

    /// Entrega normal à instância `loja`, com o segredo certo.
    async fn entregar(&self, corpo: Value) -> Resposta {
        self.webhook("loja", &self.org.to_string(), Some(SEGREDO), corpo)
            .await
    }

    fn pendentes(&self) -> u64 {
        self.store
            .inbox()
            .count_pending(self.org)
            .expect("contagem da inbox")
    }

    fn conversas(&self) -> Vec<poph_store::Conversa> {
        self.store
            .conversations()
            .list(
                self.org,
                None,
                Page {
                    after: None,
                    limit: 50,
                },
            )
            .expect("lista conversas")
    }

    /// Liga o módulo Matrix desta org — a chave que faz o webhook aceitar `fromMe` (MX-41).
    fn ligar_matrix(&self) {
        self.store
            .matrix()
            .enable(self.org)
            .expect("liga o módulo Matrix da org");
    }

    /// Conta linhas lidas direto do banco (`inbox`, `outbox`, `messages`, `audit_log`…).
    fn contar(&self, sql: &str) -> i64 {
        rusqlite::Connection::open(self.store.harness_path())
            .expect("abre o banco para conferir")
            .query_row(sql, [], |linha| linha.get(0))
            .expect("conta")
    }

    /// Valores de uma linha só (`SELECT a, b, ... WHERE id = ?`), com nulo como `None`.
    fn uma_linha(&self, sql: &str, parametro: &str) -> Vec<Option<String>> {
        rusqlite::Connection::open(self.store.harness_path())
            .expect("abre o banco para conferir")
            .query_row(sql, [parametro], |linha| {
                let mut valores = Vec::new();
                for i in 0..linha.as_ref().column_count() {
                    valores.push(match linha.get_ref(i)? {
                        rusqlite::types::ValueRef::Null => None,
                        rusqlite::types::ValueRef::Integer(n) => Some(n.to_string()),
                        rusqlite::types::ValueRef::Real(n) => Some(n.to_string()),
                        rusqlite::types::ValueRef::Text(t) => {
                            Some(String::from_utf8_lossy(t).into_owned())
                        }
                        rusqlite::types::ValueRef::Blob(_) => Some("<blob>".to_owned()),
                    });
                }
                Ok(valores)
            })
            .expect("lê a linha")
    }

    fn mensagem_id(resposta: &Resposta) -> MessageId {
        resposta.corpo["mensagem_id"]
            .as_str()
            .expect("mensagem_id na resposta")
            .parse()
            .expect("id de mensagem")
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

// ───────────────────────── corpos de webhook (formato da Evolution) ─────────────────────────

fn evento_texto(id: &str, texto: &str) -> Value {
    json!({
        "event": "messages.upsert",
        "instance": "loja",
        "apikey": CHAVE_EVO,
        "destination": "http://harness.teste/webhook/evolution/x/loja",
        "data": {
            "key": {"id": id, "remoteJid": format!("{TELEFONE}@s.whatsapp.net"), "fromMe": false},
            "pushName": "Cliente Teste",
            "messageType": "conversation",
            "message": {"conversation": texto},
            "messageTimestamp": 1795177000
        }
    })
}

fn evento_imagem(id: &str, legenda: &str) -> Value {
    json!({
        "event": "messages.upsert",
        "instance": "loja",
        "apikey": CHAVE_EVO,
        "destination": "http://harness.teste/webhook/evolution/x/loja",
        "data": {
            "key": {"id": id, "remoteJid": format!("{TELEFONE}@s.whatsapp.net"), "fromMe": false},
            "pushName": "Cliente Teste",
            "messageType": "imageMessage",
            "message": {"imageMessage": {"mimetype": "image/png", "caption": legenda}},
            "messageTimestamp": 1795177000
        }
    })
}

fn evento_estado(estado: &str) -> Value {
    json!({
        "event": "connection.update",
        "instance": "loja",
        "apikey": CHAVE_EVO,
        "data": {"state": estado}
    })
}

/// Nada do cliente, do número ou da chave pode aparecer numa resposta de erro ou de sucesso.
fn sem_dado_do_cliente(resposta: &Resposta) {
    assert!(
        !resposta.cru.contains(TELEFONE),
        "telefone na resposta: {}",
        resposta.cru
    );
    assert!(!resposta.cru.contains("@s.whatsapp.net"), "JID na resposta");
    assert!(!resposta.cru.contains(CHAVE_EVO), "chave na resposta");
    assert!(!resposta.cru.contains(SEGREDO), "segredo na resposta");
}

// ───────────────────────── cenários ─────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn texto_grava_inbox_conversa_e_mensagem_e_reentrega_nao_duplica() {
    let c = Cenario::novo("texto").await;
    let primeira = c
        .entregar(evento_texto("MSG-TXT-1", "Oi, quero um orçamento"))
        .await;
    assert_eq!(primeira.status, StatusCode::OK, "{}", primeira.cru);
    assert_eq!(primeira.corpo["situacao"], "nova");
    sem_dado_do_cliente(&primeira);

    assert_eq!(c.pendentes(), 1, "uma entrada na inbox");
    let conversas = c.conversas();
    assert_eq!(conversas.len(), 1, "uma conversa para o contato");
    assert_eq!(conversas[0].canal, "whatsapp");
    assert_eq!(conversas[0].agent_id, c.agente);
    assert_eq!(conversas[0].peer_externo.as_deref(), Some(TELEFONE));
    assert_eq!(
        c.store
            .messages()
            .count_rows(c.org, conversas[0].id)
            .expect("conta mensagens"),
        1
    );

    // A Evolution reenvia a mesma entrega: responde 200 com o mesmo item e não grava nada novo.
    let reentrega = c
        .entregar(evento_texto("MSG-TXT-1", "Oi, quero um orçamento"))
        .await;
    assert_eq!(reentrega.status, StatusCode::OK, "{}", reentrega.cru);
    assert_eq!(reentrega.corpo["situacao"], "duplicada");
    assert_eq!(reentrega.corpo["entrada_id"], primeira.corpo["entrada_id"]);
    assert_eq!(
        reentrega.corpo["mensagem_id"],
        primeira.corpo["mensagem_id"]
    );
    assert_eq!(c.pendentes(), 1, "reentrega não duplica a entrada");
    assert_eq!(
        c.store
            .messages()
            .count_rows(c.org, conversas[0].id)
            .expect("conta mensagens"),
        1,
        "reentrega não duplica a mensagem"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn segredo_ausente_ou_errado_da_401_sem_gravar_nada() {
    let c = Cenario::novo("segredo").await;
    let org = c.org.to_string();

    let sem = c
        .webhook("loja", &org, None, evento_texto("MSG-SEG-1", "oi"))
        .await;
    assert_eq!(sem.status, StatusCode::UNAUTHORIZED);
    assert_eq!(sem.corpo["code"], "webhook_nao_autorizado");

    let errado = c
        .webhook(
            "loja",
            &org,
            Some("segredo-errado"),
            evento_texto("MSG-SEG-2", "oi"),
        )
        .await;
    assert_eq!(errado.status, StatusCode::UNAUTHORIZED);
    // Não diz qual dos dois falhou: a mensagem é a mesma nos dois casos.
    assert_eq!(sem.corpo["message"], errado.corpo["message"]);

    // Instância sem segredo no ambiente: nem o segredo certo passa.
    let sem_env = c
        .webhook(
            "sem-segredo",
            &org,
            Some(SEGREDO),
            evento_texto("MSG-SEG-3", "oi"),
        )
        .await;
    assert_eq!(sem_env.status, StatusCode::UNAUTHORIZED);

    assert_eq!(c.pendentes(), 0, "nada entra sem segredo certo");
    assert!(c.conversas().is_empty(), "nenhuma conversa aberta");
    sem_dado_do_cliente(&sem);
    sem_dado_do_cliente(&errado);
}

#[tokio::test(flavor = "multi_thread")]
async fn instancia_ou_org_nao_configurada_da_404() {
    let c = Cenario::novo("instancia").await;
    let org = c.org.to_string();

    let outra = c
        .webhook(
            "outra-instancia",
            &org,
            Some(SEGREDO),
            evento_texto("MSG-404-1", "oi"),
        )
        .await;
    assert_eq!(outra.status, StatusCode::NOT_FOUND);
    assert_eq!(outra.corpo["code"], "not_found");

    let org_desconhecida = c
        .webhook(
            "loja",
            "nao-e-um-id",
            Some(SEGREDO),
            evento_texto("MSG-404-2", "oi"),
        )
        .await;
    assert_eq!(org_desconhecida.status, StatusCode::NOT_FOUND);
    assert_eq!(c.pendentes(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn integracao_inexistente_da_404_para_mensagem_e_200_sem_criar_para_estado() {
    let c = Cenario::novo("integracao").await;
    let org = c.org.to_string();

    // A instância existe na configuração, mas a integração não existe no banco.
    let mensagem = c
        .webhook("orfa", &org, Some(SEGREDO), evento_texto("MSG-ORF-1", "oi"))
        .await;
    assert_eq!(mensagem.status, StatusCode::NOT_FOUND, "{}", mensagem.cru);
    assert_eq!(c.pendentes(), 0, "mensagem sem agente dono não entra");

    let estado = c
        .webhook("orfa", &org, Some(SEGREDO), evento_estado("open"))
        .await;
    assert_eq!(estado.status, StatusCode::OK, "{}", estado.cru);
    assert_eq!(estado.corpo["efeito"], "integracao_inexistente");

    let integracoes = c
        .store
        .list_integrations(
            c.org,
            c.agente,
            Page {
                after: None,
                limit: 50,
            },
        )
        .expect("lista integrações");
    assert_eq!(integracoes.len(), 1, "o estado não criou integração nova");
}

#[tokio::test(flavor = "multi_thread")]
async fn estado_de_conexao_atualiza_a_integracao_sem_mexer_na_versao() {
    let c = Cenario::novo("estado").await;

    let aberto = c.entregar(evento_estado("open")).await;
    assert_eq!(aberto.status, StatusCode::OK, "{}", aberto.cru);
    assert_eq!(aberto.corpo["efeito"], "estado");
    let depois_aberto = c
        .store
        .get_integration(c.org, c.agente, c.integracao)
        .expect("lê a integração");
    assert_eq!(depois_aberto.connection_state, "connected");
    assert_eq!(
        depois_aberto.version, 1,
        "estado de conexão não sobe a versão"
    );

    let fechado = c.entregar(evento_estado("close")).await;
    assert_eq!(fechado.status, StatusCode::OK);
    let depois_fechado = c
        .store
        .get_integration(c.org, c.agente, c.integracao)
        .expect("lê a integração");
    assert_eq!(depois_fechado.connection_state, "disconnected");
}

#[tokio::test(flavor = "multi_thread")]
async fn imagem_vira_anexo_com_blob_no_disco_e_reentrega_nao_baixa_de_novo() {
    let c = Cenario::novo("imagem").await;
    let primeira = c
        .entregar(evento_imagem("MSG-IMG-1", "foto do produto"))
        .await;
    assert_eq!(primeira.status, StatusCode::OK, "{}", primeira.cru);
    assert_eq!(
        c.downloads.load(Ordering::SeqCst),
        1,
        "uma baixada da mídia"
    );

    let mensagem = Cenario::mensagem_id(&primeira);
    let anexos = c
        .store
        .messages()
        .attachments(c.org, mensagem)
        .expect("lista anexos");
    assert_eq!(anexos.len(), 1, "a imagem virou um anexo");
    assert_eq!(anexos[0].mime, "image/png");
    assert_eq!(anexos[0].extraction_status, "skipped");
    assert!(anexos[0].size_bytes > 0);

    let blob = c
        .pasta
        .join("dados")
        .join("blobs")
        .join(&anexos[0].blob_path);
    assert!(blob.is_file(), "blob gravado em {}", blob.display());
    assert_eq!(
        fs::metadata(&blob).expect("metadados do blob").len(),
        anexos[0].size_bytes
    );

    let texto = c
        .store
        .messages()
        .get(c.org, c.conversas()[0].id, mensagem)
        .expect("lê a mensagem");
    assert_eq!(
        texto.texto, "foto do produto",
        "a legenda é o texto da mensagem"
    );

    // Reentrega: não baixa a mídia de novo nem grava outro anexo.
    let reentrega = c
        .entregar(evento_imagem("MSG-IMG-1", "foto do produto"))
        .await;
    assert_eq!(reentrega.status, StatusCode::OK);
    assert_eq!(reentrega.corpo["situacao"], "duplicada");
    assert_eq!(
        c.downloads.load(Ordering::SeqCst),
        1,
        "reentrega não baixa de novo"
    );
    assert_eq!(
        c.store
            .messages()
            .attachments(c.org, mensagem)
            .expect("lista anexos")
            .len(),
        1
    );
    sem_dado_do_cliente(&primeira);
}

#[tokio::test(flavor = "multi_thread")]
async fn midia_expirada_preserva_o_texto_da_mensagem() {
    let c = Cenario::novo("expirada").await;
    let resposta = c
        .entregar(evento_imagem(
            "MSG-EXP-1",
            "segue a legenda mesmo sem a imagem",
        ))
        .await;
    assert_eq!(resposta.status, StatusCode::OK, "{}", resposta.cru);
    assert_eq!(resposta.corpo["situacao"], "nova");
    assert_eq!(
        c.downloads.load(Ordering::SeqCst),
        1,
        "tentou baixar uma vez"
    );

    let mensagem = Cenario::mensagem_id(&resposta);
    let gravada = c
        .store
        .messages()
        .get(c.org, c.conversas()[0].id, mensagem)
        .expect("lê a mensagem");
    assert_eq!(gravada.texto, "segue a legenda mesmo sem a imagem");
    assert!(
        c.store
            .messages()
            .attachments(c.org, mensagem)
            .expect("lista anexos")
            .is_empty(),
        "sem anexo quando a mídia expirou"
    );
    assert_eq!(c.pendentes(), 1, "a mensagem entrou na inbox");
}

#[tokio::test(flavor = "multi_thread")]
async fn falha_de_gravacao_da_503_e_a_reentrega_depois_entra_inteira() {
    let c = Cenario::novo("falha").await;

    // Simula falha do banco no meio da gravação: a mensagem não pode ser gravada.
    {
        let conexao = rusqlite::Connection::open(c.store.harness_path()).expect("abre o banco");
        conexao
            .execute_batch(
                "CREATE TRIGGER falha_teste BEFORE INSERT ON messages
                 BEGIN SELECT RAISE(ABORT, 'falha simulada'); END;",
            )
            .expect("cria o gatilho de falha");
    }

    let falha = c
        .entregar(evento_texto("MSG-FALHA-1", "oi com falha"))
        .await;
    assert_eq!(
        falha.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        falha.cru
    );
    assert_eq!(falha.corpo["code"], "indisponivel");
    assert_eq!(
        c.pendentes(),
        0,
        "a inbox também volta: nada fica pela metade"
    );
    sem_dado_do_cliente(&falha);

    {
        let conexao = rusqlite::Connection::open(c.store.harness_path()).expect("abre o banco");
        conexao
            .execute_batch("DROP TRIGGER falha_teste;")
            .expect("remove o gatilho");
    }

    // A Evolution reenvia: agora entra inteira, sem duplicata.
    let reenvio = c
        .entregar(evento_texto("MSG-FALHA-1", "oi com falha"))
        .await;
    assert_eq!(reenvio.status, StatusCode::OK, "{}", reenvio.cru);
    assert_eq!(reenvio.corpo["situacao"], "nova");
    assert_eq!(c.pendentes(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn corpo_que_nao_e_objeto_json_da_400() {
    let c = Cenario::novo("corpo").await;
    let org = c.org.to_string();
    let resposta = c
        .cliente
        .post(format!("{}/webhook/evolution/{org}/loja", c.base))
        .header("X-Evolution-Webhook-Secret", SEGREDO)
        .body("isto não é json")
        .send()
        .await
        .expect("resposta");
    assert_eq!(resposta.status(), StatusCode::BAD_REQUEST);
    let texto = resposta.text().await.expect("corpo");
    assert!(
        texto.contains("invalid_request"),
        "formato de erro da API: {texto}"
    );
    assert_eq!(c.pendentes(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn evento_sem_efeito_da_200_e_nao_grava() {
    let c = Cenario::novo("sem-efeito").await;
    let resposta = c
        .entregar(json!({"event": "send.message", "instance": "loja", "data": {}}))
        .await;
    assert_eq!(resposta.status, StatusCode::OK, "{}", resposta.cru);
    assert_eq!(resposta.corpo["efeito"], "nenhum");
    assert_eq!(c.pendentes(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn logs_e_respostas_nao_carregam_telefone_nem_chave() {
    let c = Cenario::novo("logs").await;
    let resposta = c
        .entregar(evento_texto("MSG-LOG-1", "oi, sou o cliente de teste"))
        .await;
    assert_eq!(resposta.status, StatusCode::OK);
    // Recusa com segredo errado também não pode ecoar nada.
    let recusa = c
        .webhook(
            "loja",
            &c.org.to_string(),
            Some("nada"),
            evento_texto("MSG-LOG-2", "oi"),
        )
        .await;
    assert_eq!(recusa.status, StatusCode::UNAUTHORIZED);

    tokio::time::sleep(Duration::from_millis(50)).await;
    let registro = String::from_utf8_lossy(&logs_capturados().lock().unwrap()).into_owned();
    assert!(
        registro.contains("mensagem do WhatsApp registrada"),
        "a captura de logs precisa ter visto a entrega"
    );
    for proibido in [
        TELEFONE,
        "@s.whatsapp.net",
        CHAVE_EVO,
        SEGREDO,
        "oi, sou o cliente de teste",
    ] {
        assert!(!registro.contains(proibido), "log vazou `{proibido}`");
    }
    sem_dado_do_cliente(&resposta);
    sem_dado_do_cliente(&recusa);
}

#[tokio::test(flavor = "multi_thread")]
async fn estado_de_conexao_deixa_uma_linha_de_log_com_o_estado_novo() {
    let c = Cenario::novo("log-estado").await;
    let resposta = c.entregar(evento_estado("open")).await;
    assert_eq!(resposta.status, StatusCode::OK, "{}", resposta.cru);
    assert_eq!(resposta.corpo["efeito"], "estado");

    tokio::time::sleep(Duration::from_millis(50)).await;
    let registro = String::from_utf8_lossy(&logs_capturados().lock().unwrap()).into_owned();
    // Captura viva: sem a linha de controle, a ausência da linha de estado não provaria nada.
    // Filtra pela org do cenário: outros testes deste arquivo também atualizam estado.
    let linha = registro
        .lines()
        .find(|linha| {
            linha.contains("estado de conexão do WhatsApp atualizado")
                && linha.contains(&c.org.to_string())
        })
        .unwrap_or_default()
        .to_owned();
    assert!(
        !linha.is_empty(),
        "a atualização de estado não deixou linha de log"
    );
    assert!(
        linha.contains("connected"),
        "a linha não diz o estado novo: {linha}"
    );
    assert!(
        linha.contains("loja"),
        "a linha não diz a instância: {linha}"
    );
    for proibido in [TELEFONE, "@s.whatsapp.net", CHAVE_EVO, SEGREDO] {
        assert!(!registro.contains(proibido), "log vazou `{proibido}`");
    }
    sem_dado_do_cliente(&resposta);
}

// ───────────────────────── configuração (sem servidor) ─────────────────────────

fn config_com_instancias(instancias: Vec<EvolutionInstanceConfig>) -> HarnessConfig {
    HarnessConfig {
        evolution: Some(EvolutionConfig {
            url_base: "http://127.0.0.1:18081".to_owned(),
            instances: instancias,
        }),
        ..Default::default()
    }
}

fn instancia(segredo_env: &str) -> EvolutionInstanceConfig {
    EvolutionInstanceConfig {
        org_id: OrgId::generate().to_string(),
        instance: "loja".to_owned(),
        integration_id: IntegrationId::generate().to_string(),
        secret_env: segredo_env.to_owned(),
    }
}

#[test]
fn configuracao_resolve_segredo_pelo_nome_da_variavel_e_nao_aceita_nome_inseguro() {
    let config = config_com_instancias(vec![instancia("POPH_EVO_SEGREDO_LOJA")]);
    config.validar().expect("configuração válida");

    let com_valor =
        canais_da_configuracao(&config, Some(Secret::new(CHAVE_EVO.to_owned())), &|nome| {
            (nome == "POPH_EVO_SEGREDO_LOJA").then(|| "  valor-do-segredo  ".to_owned())
        })
        .expect("canais");
    assert_eq!(com_valor.len(), 1);
    assert!(com_valor[0].segredo.is_some());
    assert!(
        com_valor[0].cliente.is_some(),
        "com chave, a instância baixa mídia"
    );

    let sem_valor = canais_da_configuracao(&config, None, &|_| None).expect("canais");
    assert!(
        sem_valor[0].segredo.is_none(),
        "variável ausente vira instância sem segredo"
    );
    assert!(sem_valor[0].cliente.is_none(), "sem chave, não baixa mídia");

    let mut minuscula = config_com_instancias(vec![instancia("poph_evo_segredo")]);
    assert!(
        minuscula.validar().is_err(),
        "nome de variável em minúsculas é recusado"
    );
    minuscula.evolution.as_mut().unwrap().instances[0].secret_env = "POPH_EVO_OK".to_owned();
    minuscula.evolution.as_mut().unwrap().instances[0].instance = "loja/../x".to_owned();
    assert!(
        minuscula.validar().is_err(),
        "nome de instância com barra é recusado"
    );
}

#[test]
fn configuracao_recusa_mesma_instancia_repetida_na_mesma_org() {
    let uma = instancia("POPH_EVO_A");
    let mut repetida = uma.clone();
    repetida.integration_id = IntegrationId::generate().to_string();
    let config = config_com_instancias(vec![uma, repetida]);
    assert!(config.validar().is_err());
}

#[test]
fn mensagem_de_id_invalido_nao_passa_pela_configuracao() {
    // Protege o caminho da URL: o id de org precisa ser o formato do Harness.
    let mut config = config_com_instancias(vec![instancia("POPH_EVO_A")]);
    config.evolution.as_mut().unwrap().instances[0].org_id = "nao-e-um-id".to_owned();
    assert!(config.validar().is_err());
}

/// Corpos no formato que a EvolutionAPI 2.3.7 realmente entregou no ambiente isolado (HR-26d):
/// `connection.update` com `data.state` e `qrcode.updated` com `data.qrcode`. O número e a chave
/// são fictícios; a forma do JSON é a medida.
#[tokio::test(flavor = "multi_thread")]
async fn corpos_reais_da_evolution_223_mudam_o_estado_certo() {
    let c = Cenario::novo("formato-real").await;

    let qr = c
        .entregar(json!({
            "event": "qrcode.updated",
            "instance": "loja",
            "data": {"qrcode": {
                "instance": "loja",
                "pairingCode": null,
                "code": "2@teste-codigo-de-qr",
                "base64": "data:image/png;base64,iVBORw0KGgo="
            }},
            "destination": "http://harness.teste/webhook/evolution/x/loja",
            "date_time": "2026-11-20T08:34:28.673Z",
            "server_url": "http://127.0.0.1:18081",
            "apikey": CHAVE_EVO
        }))
        .await;
    assert_eq!(qr.status, StatusCode::OK, "{}", qr.cru);
    assert_eq!(qr.corpo["efeito"], "estado");
    let aguardando = c
        .store
        .get_integration(c.org, c.agente, c.integracao)
        .expect("lê a integração");
    assert_eq!(aguardando.connection_state, "awaiting_scan");

    // Estado `connecting` (transitório) não muda o que já estava gravado.
    let conectando = c
        .entregar(json!({
            "event": "connection.update",
            "instance": "loja",
            "data": {"instance": "loja", "state": "connecting", "statusReason": 200},
            "destination": "http://harness.teste/webhook/evolution/x/loja",
            "server_url": "http://127.0.0.1:18081",
            "apikey": CHAVE_EVO
        }))
        .await;
    assert_eq!(conectando.status, StatusCode::OK, "{}", conectando.cru);
    let depois = c
        .store
        .get_integration(c.org, c.agente, c.integracao)
        .expect("lê a integração");
    assert_eq!(depois.connection_state, "awaiting_scan");

    let fechado = c
        .entregar(json!({
            "event": "connection.update",
            "instance": "loja",
            "data": {"instance": "loja", "state": "close", "statusReason": 401},
            "apikey": CHAVE_EVO
        }))
        .await;
    assert_eq!(fechado.status, StatusCode::OK, "{}", fechado.cru);
    let desconectada = c
        .store
        .get_integration(c.org, c.agente, c.integracao)
        .expect("lê a integração");
    assert_eq!(desconectada.connection_state, "disconnected");
    sem_dado_do_cliente(&qr);
    assert!(
        !qr.cru.contains("2@teste-codigo-de-qr"),
        "código do QR não volta na resposta"
    );
}

// ───────────────────────── nome do cliente (MX-32) ─────────────────────────

/// Mensagem de texto com o `pushName` escolhido (o id muda para não virar reentrega).
fn evento_com_nome(id: &str, texto: &str, nome: Value) -> Value {
    let mut corpo = evento_texto(id, texto);
    corpo["data"]["pushName"] = nome;
    corpo
}

#[tokio::test(flavor = "multi_thread")]
async fn push_name_cria_a_conversa_ja_com_o_nome_do_cliente() {
    let c = Cenario::novo("nome-criar").await;
    let resposta = c
        .entregar(evento_com_nome("MSG-NOME-1", "oi", json!("Cliente Teste")))
        .await;
    assert_eq!(resposta.status, StatusCode::OK, "{}", resposta.cru);
    sem_dado_do_cliente(&resposta);

    let conversas = c.conversas();
    assert_eq!(conversas.len(), 1);
    assert_eq!(conversas[0].customer_name.as_deref(), Some("Cliente Teste"));
    assert_eq!(conversas[0].version, 1, "o nome não sobe a versão");
}

#[tokio::test(flavor = "multi_thread")]
async fn ultimo_push_name_vence_sem_apagar_quando_a_mensagem_nao_traz_nome() {
    let c = Cenario::novo("nome-vence").await;
    c.entregar(evento_com_nome("MSG-NV-1", "oi", json!("Cliente Teste")))
        .await;
    c.entregar(evento_com_nome(
        "MSG-NV-2",
        "de novo",
        json!("Cliente Novo"),
    ))
    .await;
    assert_eq!(
        c.conversas()[0].customer_name.as_deref(),
        Some("Cliente Novo")
    );

    // Mensagem sem `pushName`: o nome anterior fica.
    let sem_nome = evento_texto("MSG-NV-3", "sem nome");
    let mut sem_nome = sem_nome;
    sem_nome["data"]
        .as_object_mut()
        .expect("dados")
        .remove("pushName");
    c.entregar(sem_nome).await;
    let conversa = &c.conversas()[0];
    assert_eq!(conversa.customer_name.as_deref(), Some("Cliente Novo"));
    assert_eq!(conversa.version, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn push_name_e_limpo_antes_de_gravar() {
    let c = Cenario::novo("nome-limpo").await;
    let bruto = format!("  Ana\u{200B} Maria \u{202E}\n{}", "x".repeat(150));
    c.entregar(evento_com_nome("MSG-LIMPO-1", "oi", json!(bruto)))
        .await;
    let nome = c.conversas()[0]
        .customer_name
        .clone()
        .expect("nome gravado");
    assert!(nome.starts_with("Ana Maria x"), "{nome}");
    assert_eq!(nome.chars().count(), 100, "corta em 100 caracteres");
    assert!(!nome.contains('\u{200B}') && !nome.contains('\u{202E}'));
}

#[tokio::test(flavor = "multi_thread")]
async fn grupo_e_eco_do_proprio_aparelho_nao_gravam_nome() {
    let c = Cenario::novo("nome-grupo").await;

    // Grupo: o `pushName` é de quem escreveu, não do cliente da conversa do grupo.
    let mut grupo = evento_com_nome("MSG-GRP-1", "oi do grupo", json!("Membro do Grupo"));
    grupo["data"]["key"]["remoteJid"] = json!("120363000000000000@g.us");
    let resposta = c.entregar(grupo).await;
    assert_eq!(resposta.status, StatusCode::OK, "{}", resposta.cru);
    let conversas = c.conversas();
    assert_eq!(conversas.len(), 1, "o grupo abre a própria conversa");
    assert_eq!(conversas[0].customer_name, None);

    // Eco da nossa própria mensagem: descartado antes de qualquer gravação.
    let mut eco = evento_com_nome("MSG-ECO-1", "eco", json!("Dono da Loja"));
    eco["data"]["key"]["fromMe"] = json!(true);
    c.entregar(eco).await;
    assert_eq!(c.conversas().len(), 1, "o eco não abre conversa");
}

#[tokio::test(flavor = "multi_thread")]
async fn falha_ao_gravar_o_nome_nao_impede_a_mensagem_de_entrar() {
    let c = Cenario::novo("nome-falha-webhook").await;
    {
        let conexao = rusqlite::Connection::open(c.store.harness_path()).expect("abre o banco");
        conexao
            .execute_batch(
                "CREATE TRIGGER falha_nome BEFORE UPDATE OF client_name ON conversations
                 BEGIN SELECT RAISE(ABORT, 'falha simulada'); END;",
            )
            .expect("cria o gatilho de falha");
    }

    let falha = c
        .entregar(evento_com_nome(
            "MSG-FN-1",
            "oi com falha no nome",
            json!("Cliente Teste"),
        ))
        .await;
    assert_eq!(falha.status, StatusCode::OK, "{}", falha.cru);
    assert_eq!(falha.corpo["situacao"], "nova");
    assert_eq!(c.pendentes(), 1, "a mensagem entrou na inbox");
    assert_eq!(
        c.conversas()[0].customer_name,
        None,
        "o nome ficou para depois"
    );

    {
        let conexao = rusqlite::Connection::open(c.store.harness_path()).expect("abre o banco");
        conexao
            .execute_batch("DROP TRIGGER falha_nome;")
            .expect("remove o gatilho");
    }
    // A próxima mensagem com nome grava normalmente.
    c.entregar(evento_com_nome(
        "MSG-FN-2",
        "agora vai",
        json!("Cliente Teste"),
    ))
    .await;
    assert_eq!(
        c.conversas()[0].customer_name.as_deref(),
        Some("Cliente Teste")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn nome_do_cliente_nao_aparece_em_log_nem_em_resposta() {
    const NOME: &str = "NOME-CANARIO-MX32-Zeca";
    let c = Cenario::novo("nome-canario").await;
    let resposta = c
        .entregar(evento_com_nome("MSG-CAN-1", "oi, canário", json!(NOME)))
        .await;
    assert_eq!(resposta.status, StatusCode::OK);
    assert!(!resposta.cru.contains(NOME), "nome na resposta");

    // Falha de gravação do nome também não pode ecoar o nome no log.
    {
        let conexao = rusqlite::Connection::open(c.store.harness_path()).expect("abre o banco");
        conexao
            .execute_batch(
                "CREATE TRIGGER falha_canario BEFORE UPDATE OF client_name ON conversations
                 BEGIN SELECT RAISE(ABORT, 'falha simulada'); END;",
            )
            .expect("cria o gatilho de falha");
    }
    c.entregar(evento_com_nome(
        "MSG-CAN-2",
        "outra",
        json!("OUTRO-CANARIO-MX32"),
    ))
    .await;

    tokio::time::sleep(Duration::from_millis(50)).await;
    let registro = String::from_utf8_lossy(&logs_capturados().lock().unwrap()).into_owned();
    assert!(
        registro.contains("mensagem do WhatsApp registrada"),
        "a captura de logs precisa ter visto a entrega"
    );
    assert!(
        !registro.contains(NOME),
        "o nome do cliente vazou para o log"
    );
    assert!(
        !registro.contains("OUTRO-CANARIO-MX32"),
        "o nome do cliente vazou para o log na falha"
    );
}

// ───────────────────── mensagem digitada no aparelho (MX-41) ─────────────────────

/// A mesma entrega de texto, com `fromMe = true`: é o que a Evolution manda para o que saiu do
/// aparelho do dono da instância (o nosso próprio envio **e** o que o cliente digitou no celular).
fn evento_do_aparelho(id: &str, texto: &str) -> Value {
    let mut corpo = evento_texto(id, texto);
    corpo["data"]["key"]["fromMe"] = json!(true);
    corpo
}

/// Aceite: com o módulo ligado, a mensagem do aparelho entra **uma** vez, como saída do
/// dispositivo, sem `inbox` (não há turno) e sem `outbox` (já saiu pelo celular), e a conversa fica
/// `transferred` com a auditoria `device_takeover`. A reentrega não duplica.
#[tokio::test(flavor = "multi_thread")]
async fn mensagem_do_aparelho_grava_uma_vez_e_pausa_o_agente() {
    let c = Cenario::novo("aparelho").await;
    c.ligar_matrix();

    let resposta = c
        .entregar(evento_do_aparelho("MSG-AP-1", "já respondi pelo celular"))
        .await;
    assert_eq!(resposta.status, StatusCode::OK, "{}", resposta.cru);
    assert_eq!(resposta.corpo["efeito"], "aparelho");
    assert_eq!(resposta.corpo["situacao"], "nova");
    let conversa = resposta.corpo["conversa_id"]
        .as_str()
        .expect("conversa na resposta")
        .to_owned();

    // A mensagem é saída do dispositivo, sem turno.
    let mensagem = c.uma_linha(
        "SELECT direction, role, origin, turn_id, text FROM messages WHERE external_id = ?1",
        "MSG-AP-1",
    );
    assert_eq!(mensagem[0].as_deref(), Some("out"), "direção");
    assert_eq!(mensagem[1].as_deref(), Some("assistant"), "papel");
    assert_eq!(mensagem[2].as_deref(), Some("device"), "origem");
    assert_eq!(mensagem[3], None, "sem turno");
    assert_eq!(mensagem[4].as_deref(), Some("já respondi pelo celular"));

    // A conversa do contato foi pausada: transferida, com a versão subindo uma vez.
    let estado = c.uma_linha(
        "SELECT status, version FROM conversations WHERE id = ?1",
        &conversa,
    );
    assert_eq!(estado[0].as_deref(), Some("transferred"));
    assert_eq!(estado[1].as_deref(), Some("2"));

    // Nada para o turno pegar e nada para a Evolution enviar de novo.
    assert_eq!(c.pendentes(), 0, "inbox sem a mensagem do aparelho");
    assert_eq!(c.contar("SELECT COUNT(*) FROM outbox"), 0, "outbox vazia");

    // Auditoria da tomada do aparelho.
    let auditoria = c.uma_linha(
        "SELECT actor_kind, action FROM audit_log WHERE entity_id = ?1",
        &conversa,
    );
    assert_eq!(auditoria[0].as_deref(), Some("system"));
    assert_eq!(auditoria[1].as_deref(), Some("device_takeover"));

    // Reentrega do mesmo webhook: nada novo.
    let repetida = c
        .entregar(evento_do_aparelho("MSG-AP-1", "já respondi pelo celular"))
        .await;
    assert_eq!(repetida.status, StatusCode::OK, "{}", repetida.cru);
    assert_eq!(repetida.corpo["situacao"], "duplicada");
    assert_eq!(c.contar("SELECT COUNT(*) FROM messages"), 1, "sem duplicar");
    assert_eq!(
        c.contar("SELECT COUNT(*) FROM conversations"),
        1,
        "sem abrir outra conversa"
    );
    assert_eq!(
        c.contar("SELECT COUNT(*) FROM audit_log WHERE action = 'device_takeover'"),
        1,
        "uma auditoria só"
    );
    assert_eq!(c.pendentes(), 0);
    assert_eq!(c.contar("SELECT COUNT(*) FROM outbox"), 0);
}

/// Aceite: o eco do nosso próprio envio (o id que a MX-40 guardou) é descartado, sem duplicar
/// mensagem nem fila.
#[tokio::test(flavor = "multi_thread")]
async fn eco_do_proprio_envio_e_descartado() {
    let c = Cenario::novo("aparelho-eco").await;
    c.ligar_matrix();

    // Uma mensagem do cliente abre a conversa (e é o único histórico que deve existir).
    c.entregar(evento_texto("MSG-C-1", "oi")).await;
    let conversa = c.conversas()[0].id.to_string();

    // O envio do agente saiu e a Evolution devolveu o id — o que a MX-40 grava.
    {
        let conexao = rusqlite::Connection::open(c.store.harness_path()).expect("banco");
        conexao
            .execute(
                "INSERT INTO outbox
                    (id, org_id, conversation_id, turn_id, seq, kind, payload_json, state,
                     attempts, created_at, sent_at, external_message_id)
                 VALUES ('out-eco-1', ?1, ?2, 'turno-eco-1', 0, 'text', '{\"texto\":\"oi\"}',
                     'sent', 1, 1795177000000, 1795177000000, 'EVO-ECO-1')",
                rusqlite::params![c.org.to_string(), &conversa],
            )
            .expect("grava o envio confirmado");
    }

    let eco = c.entregar(evento_do_aparelho("EVO-ECO-1", "oi")).await;
    assert_eq!(eco.status, StatusCode::OK, "{}", eco.cru);
    assert_eq!(eco.corpo["efeito"], "eco");

    assert_eq!(
        c.contar("SELECT COUNT(*) FROM messages"),
        1,
        "sem mensagem nova"
    );
    assert_eq!(c.contar("SELECT COUNT(*) FROM conversations"), 1);
    assert_eq!(c.contar("SELECT COUNT(*) FROM outbox"), 1, "sem envio novo");
    assert_eq!(
        c.contar("SELECT COUNT(*) FROM audit_log WHERE action = 'device_takeover'"),
        0,
        "eco não pausa o agente"
    );
    let estado = c.uma_linha("SELECT status FROM conversations WHERE id = ?1", &conversa);
    assert_eq!(
        estado[0].as_deref(),
        Some("open"),
        "a conversa segue aberta"
    );
}

/// Aceite: org **sem** o módulo Matrix ligado mantém o comportamento de hoje — `fromMe` é
/// descartado na leitura e não grava nada.
#[tokio::test(flavor = "multi_thread")]
async fn org_sem_modulo_continua_descartando_from_me() {
    let c = Cenario::novo("aparelho-sem-modulo").await;

    let resposta = c
        .entregar(evento_do_aparelho("MSG-AP-2", "digitada no celular"))
        .await;
    assert_eq!(resposta.status, StatusCode::OK, "{}", resposta.cru);
    assert_eq!(resposta.corpo["efeito"], "nenhum");

    assert_eq!(c.contar("SELECT COUNT(*) FROM messages"), 0, "nada gravado");
    assert_eq!(c.contar("SELECT COUNT(*) FROM conversations"), 0);
    assert_eq!(c.contar("SELECT COUNT(*) FROM outbox"), 0);
    assert_eq!(c.pendentes(), 0, "nada na inbox");
    assert_eq!(
        c.contar("SELECT COUNT(*) FROM audit_log WHERE action = 'device_takeover'"),
        0
    );
}
