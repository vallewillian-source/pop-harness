//! Saída pelo WhatsApp (HR-26c): a `outbox` chega à Evolution, sem turno.
//!
//! Banco temporário, consumidor de verdade (`SaidaWhatsApp`) e uma Evolution falsa em `127.0.0.1`
//! que registra cada requisição e responde o roteiro que o teste escreveu. Nada de rede externa e
//! nenhum número de WhatsApp.
//!
//! O que se prova: texto sai uma vez, com a chave só no cabeçalho; texto acima de 4000 caracteres
//! sai cortado em uma mensagem só; arquivo sai como mídia; arquivo acima de 16 MB não sai nem sobe,
//! e o texto do mesmo turno segue; erro 5xx volta para a fila e só repete na tentativa seguinte;
//! erro 4xx é definitivo e não repete; conversa de outro canal ou sem chave fica como está; rede
//! fora do ar é transitória; nem a chave nem o telefone aparecem em erro ou log.

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use poph_agent::Backoff;
use poph_evolution::{ATTACHMENT_CAP, ConfigEvolution, EvolutionClient};
use poph_server::evolution::CanalEvolution;
use poph_server::saida_whatsapp::{OpcoesSaidaWhatsApp, SaidaWhatsApp};
use poph_store::{
    NewAgent, NewConversation, NewOrg, NewOutboxItem, OutboxItem, OutboxState, Store,
};
use poph_types::{ConversationId, HarnessConfig, IntegrationId, OrgId, Secret, TurnId};
use serde_json::{Value, json};
use tracing_subscriber::fmt::MakeWriter;

/// Chave da Evolution usada nos testes (só para os cabeçalhos das requisições falsas).
const CHAVE: &str = "chave-evo-saida-26c-teste";
/// Número de cliente falso. Não é de ninguém.
const TELEFONE: &str = "5511999990000";
/// Arquivo com 16 MB: o teto do canal (um byte a mais já é recusado).
const TETO: u64 = ATTACHMENT_CAP;

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

/// Instala, uma vez no processo de testes, o coletor de logs. Todo teste deste arquivo usa o mesmo.
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

// ───────────────────────── Evolution falsa: registra e responde o roteiro ─────────────────────────

/// Uma requisição que chegou na Evolution falsa.
#[derive(Clone, Debug)]
struct Registro {
    caminho: String,
    apikey: Option<String>,
    corpo: Value,
}

#[derive(Clone)]
struct EvolucaoFalsa {
    registros: Arc<Mutex<Vec<Registro>>>,
    roteiro: Arc<Mutex<VecDeque<(u16, Value)>>>,
    chamadas: Arc<AtomicUsize>,
}

async fn enviar_falso(
    State(evo): State<EvolucaoFalsa>,
    Path(instancia): Path<String>,
    cabecalhos: HeaderMap,
    Json(corpo): Json<Value>,
) -> (StatusCode, Json<Value>) {
    evo.chamadas.fetch_add(1, Ordering::SeqCst);
    let apikey = cabecalhos
        .get("apikey")
        .and_then(|valor| valor.to_str().ok())
        .map(str::to_owned);
    evo.registros.lock().unwrap().push(Registro {
        caminho: format!("instancia={instancia}"),
        apikey,
        corpo,
    });
    let (status, resposta) = evo
        .roteiro
        .lock()
        .unwrap()
        .pop_front()
        .unwrap_or((200, json!({"key": {"id": "MSG-SAIDA-1"}})));
    (
        StatusCode::from_u16(status).expect("status do roteiro"),
        Json(resposta),
    )
}

/// Sobe a Evolution falsa numa porta efêmera. Devolve o endereço e o que ela recebeu.
async fn subir_evolucao_falsa(
    roteiro: Vec<(u16, Value)>,
) -> (String, EvolucaoFalsa, tokio::task::JoinHandle<()>) {
    let evo = EvolucaoFalsa {
        registros: Arc::new(Mutex::new(Vec::new())),
        roteiro: Arc::new(Mutex::new(roteiro.into_iter().collect())),
        chamadas: Arc::new(AtomicUsize::new(0)),
    };
    let roteador = Router::new()
        .route("/message/sendText/{instancia}", post(enviar_falso))
        .route("/message/sendMedia/{instancia}", post(enviar_falso))
        .with_state(evo.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera da Evolution falsa");
    let endereco = listener.local_addr().expect("endereço da Evolution falsa");
    let tarefa = tokio::spawn(async move {
        let _ = axum::serve(listener, roteador).await;
    });
    (format!("http://{endereco}"), evo, tarefa)
}

// ───────────────────────── cenário: banco, org, integração e conversas ─────────────────────────

/// Como a instância da Evolution aparece para o consumidor neste teste.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Instancia {
    /// Chave presente e Evolution falsa no ar.
    Normal,
    /// Chave ausente no ambiente: o consumidor não consegue enviar.
    SemChave,
    /// Endereço sem nada escutando: falha de rede.
    PortaFechada,
}

struct Cenario {
    store: Arc<Store>,
    org: OrgId,
    /// Conversa de WhatsApp da integração configurada.
    conversa_wa: ConversationId,
    /// Conversa do console (sem integração).
    conversa_console: ConversationId,
    /// Conversa de uma integração que existe no banco, mas não na configuração do servidor.
    conversa_nao_configurada: ConversationId,
    blobs: PathBuf,
    evo: EvolucaoFalsa,
    saida: SaidaWhatsApp,
    pasta: PathBuf,
    tarefa_evo: Option<tokio::task::JoinHandle<()>>,
}

impl Cenario {
    async fn novo(rotulo: &str, modo: Instancia, roteiro: Vec<(u16, Value)>) -> Self {
        logs_capturados();
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-saida-wa-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        config.server.port = 7481;
        config.validar().expect("configuração válida");
        let blobs = config.blobs();

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Loja Saída".to_owned(),
                slug: "loja-saida".to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Atendente Saída".to_owned(),
                    slug: "atendente-saida".to_owned(),
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
        let fora_da_config: IntegrationId = store
            .create_integration(org, agente, "whatsapp".to_owned(), "active".to_owned())
            .expect("cria a integração fora da configuração")
            .id
            .parse()
            .expect("id da integração");

        let conversa_wa = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente,
                    integration_id: Some(integracao),
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some(TELEFONE.to_owned()),
                    target_department: None,
                },
            )
            .expect("cria a conversa de WhatsApp")
            .id;
        let conversa_console = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente,
                    integration_id: None,
                    canal: "console".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("cria a conversa do console")
            .id;
        let conversa_nao_configurada = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente,
                    integration_id: Some(fora_da_config),
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some("5511888880000".to_owned()),
                    target_department: None,
                },
            )
            .expect("cria a conversa fora da configuração")
            .id;

        let (url, evo, tarefa_evo) = subir_evolucao_falsa(roteiro).await;
        let endereco = match modo {
            Instancia::PortaFechada => "http://127.0.0.1:1".to_owned(),
            _ => url,
        };
        let chave = if modo == Instancia::SemChave {
            None
        } else {
            Some(Secret::new(CHAVE.to_owned()))
        };
        let cliente = chave.map(|chave| {
            EvolutionClient::new(
                ConfigEvolution::new(endereco, "loja", chave).expect("config da Evolution falsa"),
            )
            .expect("cliente da Evolution falsa")
        });
        let canais = vec![CanalEvolution {
            org,
            instancia: "loja".to_owned(),
            integracao,
            segredo: None,
            cliente,
            teto_midia: ATTACHMENT_CAP,
        }];
        let opcoes = OpcoesSaidaWhatsApp {
            // Espera curta para o teste de nova tentativa não depender de relógio.
            backoff: Backoff {
                base: Duration::from_millis(1),
                fator: 2,
                teto: Duration::from_millis(5),
            },
            ..OpcoesSaidaWhatsApp::default()
        };
        let saida = SaidaWhatsApp::novo(Arc::clone(&store), &canais, blobs.clone(), opcoes);

        Self {
            store,
            org,
            conversa_wa,
            conversa_console,
            conversa_nao_configurada,
            blobs,
            evo,
            saida,
            pasta,
            tarefa_evo: Some(tarefa_evo),
        }
    }

    fn enfileirar(
        &self,
        conversa: ConversationId,
        seq: u32,
        tipo: &str,
        conteudo: Value,
    ) -> OutboxItem {
        let (_, item) = self
            .store
            .outbox()
            .enqueue(
                self.org,
                NewOutboxItem {
                    conversation_id: conversa,
                    turn_id: TurnId::generate(),
                    seq,
                    tipo: tipo.to_owned(),
                    content_json: conteudo.to_string(),
                },
            )
            .expect("enfileira a saída");
        item
    }

    fn texto(&self, texto: &str) -> OutboxItem {
        self.enfileirar(self.conversa_wa, 0, "text", json!({ "texto": texto }))
    }

    fn estado(&self, item: &OutboxItem) -> OutboxItem {
        self.store
            .outbox()
            .get(self.org, item.id)
            .expect("lê o item de saída")
    }

    fn registros(&self) -> Vec<Registro> {
        self.evo.registros.lock().unwrap().clone()
    }

    fn chamadas(&self) -> usize {
        self.evo.chamadas.load(Ordering::SeqCst)
    }

    /// Grava um blob no lugar em que a biblioteca grava (`<pasta de blobs>/<org>/<sha256>`).
    fn gravar_blob(&self, sha: &str, bytes: &[u8]) -> String {
        let relativo = format!("{}/{sha}", self.org);
        let caminho = self.blobs.join(&relativo);
        fs::create_dir_all(caminho.parent().unwrap()).expect("pasta do blob");
        fs::write(&caminho, bytes).expect("grava o blob");
        relativo
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        if let Some(tarefa) = self.tarefa_evo.take() {
            tarefa.abort();
        }
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

// ───────────────────────── texto ─────────────────────────

#[tokio::test]
async fn texto_sai_uma_vez_no_formato_do_canal_com_a_chave_so_no_cabecalho() {
    let cenario = Cenario::novo("feliz", Instancia::Normal, vec![]).await;
    let item = cenario.texto("olá, tudo bem?");

    let relato = cenario.saida.ciclo().await.expect("ciclo");

    assert_eq!(relato.enviados, 1, "{relato:?}");
    assert_eq!(cenario.estado(&item).state, OutboxState::Sent);
    let registros = cenario.registros();
    assert_eq!(registros.len(), 1, "uma requisição, sem repetição");
    assert_eq!(registros[0].caminho, "instancia=loja");
    assert_eq!(registros[0].apikey.as_deref(), Some(CHAVE));
    assert_eq!(registros[0].corpo["number"], TELEFONE);
    assert_eq!(registros[0].corpo["text"], "olá, tudo bem?");
    assert!(
        !registros[0].corpo.to_string().contains(CHAVE),
        "a chave não vai no corpo"
    );
}

#[tokio::test]
async fn texto_acima_de_4000_sai_cortado_em_uma_mensagem_so() {
    let cenario = Cenario::novo("longo", Instancia::Normal, vec![]).await;
    let texto = "a".repeat(5000);
    let item = cenario.texto(&texto);

    let relato = cenario.saida.ciclo().await.expect("ciclo");

    assert_eq!(relato.enviados, 1, "{relato:?}");
    assert_eq!(cenario.estado(&item).state, OutboxState::Sent);
    let registros = cenario.registros();
    assert_eq!(registros.len(), 1, "uma mensagem só, nunca dividida");
    let enviado = registros[0].corpo["text"].as_str().expect("texto");
    assert_eq!(enviado.chars().count(), 4000, "corta no limite do canal");
    assert!(enviado.ends_with('…'), "termina em reticências");
}

// ───────────────────────── arquivo da biblioteca ─────────────────────────

#[tokio::test]
async fn arquivo_da_biblioteca_sai_como_midia_com_o_blob() {
    let cenario = Cenario::novo("midia", Instancia::Normal, vec![]).await;
    let conteudo = b"%PDF-1.4 conteudo de teste".to_vec();
    let blob = cenario.gravar_blob(&"b".repeat(64), &conteudo);
    let item = cenario.enfileirar(
        cenario.conversa_wa,
        1,
        "file",
        json!({
            "blob_ref": blob,
            "arquivo_id": "01a1f6a5-cd9b-7000-8000-000000000000",
            "nome": "manual.pdf",
            "mime": "application/pdf",
            "tamanho_bytes": conteudo.len(),
            "sha256": "b".repeat(64),
        }),
    );

    let relato = cenario.saida.ciclo().await.expect("ciclo");

    assert_eq!(relato.enviados, 1, "{relato:?}");
    assert_eq!(cenario.estado(&item).state, OutboxState::Sent);
    let registros = cenario.registros();
    assert_eq!(registros.len(), 1);
    assert_eq!(registros[0].corpo["mediatype"], "document");
    assert_eq!(registros[0].corpo["fileName"], "manual.pdf");
    let midia = registros[0].corpo["media"]
        .as_str()
        .expect("conteúdo em base64");
    assert_eq!(
        midia.len(),
        conteudo.len().div_ceil(3) * 4,
        "o tamanho em base64 bate com o blob"
    );
}

#[tokio::test]
async fn arquivo_acima_de_16_mb_nao_sai_e_o_texto_do_turno_segue() {
    let cenario = Cenario::novo("teto", Instancia::Normal, vec![]).await;
    let texto = cenario.enfileirar(
        cenario.conversa_wa,
        0,
        "text",
        json!({ "texto": "segue o arquivo" }),
    );
    let grande = cenario.enfileirar(
        cenario.conversa_wa,
        1,
        "file",
        json!({
            // Sem blob no disco de propósito: a recusa tem de vir antes de qualquer leitura.
            "blob_ref": format!("{}/{}", cenario.org, "c".repeat(64)),
            "arquivo_id": "01a1f6a5-cd9b-7000-8000-000000000001",
            "nome": "catalogo.pdf",
            "mime": "application/pdf",
            "tamanho_bytes": TETO + 1,
            "sha256": "c".repeat(64),
        }),
    );

    let relato = cenario.saida.ciclo().await.expect("ciclo");

    assert_eq!(relato.enviados, 1, "o texto sai: {relato:?}");
    assert_eq!(relato.definitivos, 1, "o arquivo não sai: {relato:?}");
    assert_eq!(cenario.estado(&texto).state, OutboxState::Sent);
    let arquivo = cenario.estado(&grande);
    assert_eq!(arquivo.state, OutboxState::Failed);
    assert!(arquivo.ultimo_erro.is_some(), "o motivo fica registrado");
    assert_eq!(
        cenario.chamadas(),
        1,
        "só o texto chegou à Evolution; o arquivo foi recusado antes da chamada"
    );
}

// ───────────────────────── falhas: transitória, definitiva, não repetição ─────────────────────────

#[tokio::test]
async fn erro_5xx_volta_para_a_fila_e_so_repete_na_tentativa_seguinte() {
    let cenario = Cenario::novo(
        "transitoria",
        Instancia::Normal,
        vec![(500, json!({"message": "erro interno da Evolution"}))],
    )
    .await;
    let item = cenario.texto("tentativa que falha");

    let primeiro = cenario.saida.ciclo().await.expect("primeiro ciclo");

    assert_eq!(primeiro.transitorios, 1, "{primeiro:?}");
    assert_eq!(
        cenario.chamadas(),
        1,
        "um 5xx não dispara uma segunda requisição dentro da mesma tentativa"
    );
    let depois = cenario.estado(&item);
    assert_eq!(depois.state, OutboxState::Sending, "volta para a fila");
    assert_eq!(depois.attempts, 1);
    assert!(depois.ultimo_erro.is_some());

    tokio::time::sleep(Duration::from_millis(30)).await;
    let segundo = cenario.saida.ciclo().await.expect("segundo ciclo");

    assert_eq!(segundo.enviados, 1, "{segundo:?}");
    assert_eq!(cenario.estado(&item).state, OutboxState::Sent);
    assert_eq!(
        cenario.chamadas(),
        2,
        "a repetição veio só na tentativa seguinte"
    );
}

#[tokio::test]
async fn falha_transitoria_que_nunca_passa_esgota_as_tentativas_e_vira_failed() {
    // Oito tentativas (o teto do motor) com 5xx; a nona não chega a ser feita.
    let roteiro = (0..8)
        .map(|_| (500, json!({"message": "indisponível"})))
        .collect();
    let cenario = Cenario::novo("esgota", Instancia::Normal, roteiro).await;
    let item = cenario.texto("nunca passa");

    for _ in 0..8 {
        cenario.saida.ciclo().await.expect("ciclo");
        tokio::time::sleep(Duration::from_millis(8)).await;
    }
    assert_eq!(cenario.chamadas(), 8, "uma requisição por tentativa");
    assert_eq!(cenario.estado(&item).attempts, 8);

    let relato = cenario.saida.ciclo().await.expect("ciclo final");

    assert_eq!(relato.esgotados, 1, "{relato:?}");
    assert_eq!(cenario.chamadas(), 8, "depois de esgotar, nada mais sai");
    let depois = cenario.estado(&item);
    assert_eq!(depois.state, OutboxState::Failed);
    assert_eq!(
        depois.ultimo_erro.as_deref(),
        Some("tentativas de envio esgotadas")
    );
}

#[tokio::test]
async fn erro_4xx_e_definitivo_nao_repete_e_nao_vaza_o_telefone() {
    let cenario = Cenario::novo(
        "definitivo",
        Instancia::Normal,
        vec![(
            400,
            json!({"message": format!("o número {TELEFONE}@s.whatsapp.net não existe no WhatsApp")}),
        )],
    )
    .await;
    let item = cenario.texto("para um número inválido");

    let primeiro = cenario.saida.ciclo().await.expect("primeiro ciclo");

    assert_eq!(primeiro.definitivos, 1, "{primeiro:?}");
    let depois = cenario.estado(&item);
    assert_eq!(depois.state, OutboxState::Failed);
    let erro = depois.ultimo_erro.clone().unwrap_or_default();
    assert!(
        !erro.contains(TELEFONE),
        "o telefone não vai para o banco: {erro}"
    );
    assert!(
        !erro.contains("s.whatsapp"),
        "o JID não vai para o banco: {erro}"
    );

    tokio::time::sleep(Duration::from_millis(30)).await;
    cenario.saida.ciclo().await.expect("segundo ciclo");

    assert_eq!(cenario.chamadas(), 1, "4xx nunca é repetido");
    assert_eq!(cenario.estado(&item).state, OutboxState::Failed);
}

#[tokio::test]
async fn rede_fora_do_ar_e_transitoria() {
    let cenario = Cenario::novo("rede", Instancia::PortaFechada, vec![]).await;
    let item = cenario.texto("sem rede");

    let relato = cenario.saida.ciclo().await.expect("ciclo");

    assert_eq!(relato.transitorios, 1, "{relato:?}");
    assert_eq!(cenario.estado(&item).state, OutboxState::Sending);
}

// ───────────────────────── o que não é deste consumidor fica como está ─────────────────────────

#[tokio::test]
async fn conversa_do_console_e_de_integracao_nao_configurada_nao_sao_tocadas() {
    let cenario = Cenario::novo("alheios", Instancia::Normal, vec![]).await;
    let console = cenario.enfileirar(
        cenario.conversa_console,
        0,
        "text",
        json!({"texto": "do console"}),
    );
    let fora = cenario.enfileirar(
        cenario.conversa_nao_configurada,
        0,
        "text",
        json!({"texto": "de outra integração"}),
    );

    let relato = cenario.saida.ciclo().await.expect("ciclo");

    assert_eq!(
        relato,
        Default::default(),
        "nada foi contado como deste consumidor"
    );
    assert_eq!(cenario.chamadas(), 0);
    assert_eq!(cenario.estado(&console).state, OutboxState::Pending);
    assert_eq!(cenario.estado(&console).attempts, 0);
    assert_eq!(cenario.estado(&fora).state, OutboxState::Pending);
    assert_eq!(cenario.estado(&fora).attempts, 0);
}

#[tokio::test]
async fn sem_chave_da_evolution_a_mensagem_fica_pendente_e_nao_falha() {
    let cenario = Cenario::novo("sem-chave", Instancia::SemChave, vec![]).await;
    let item = cenario.texto("espera a chave");

    let relato = cenario.saida.ciclo().await.expect("ciclo");

    assert_eq!(relato.adiados, 1, "{relato:?}");
    let depois = cenario.estado(&item);
    assert_eq!(depois.state, OutboxState::Pending, "não vira falha");
    assert_eq!(depois.attempts, 0, "nenhuma tentativa gasta");
    assert_eq!(cenario.chamadas(), 0);
}

// ───────────────────────── segredo e telefone fora dos logs ─────────────────────────

#[tokio::test]
async fn chave_e_telefone_nao_aparecem_em_log() {
    let cenario = Cenario::novo(
        "logs",
        Instancia::Normal,
        vec![(
            400,
            json!({"message": format!("número {TELEFONE}@s.whatsapp.net inválido")}),
        )],
    )
    .await;
    cenario.texto("mensagem para o número inválido");
    cenario.saida.ciclo().await.expect("ciclo");
    let transitoria = Cenario::novo(
        "logs-5xx",
        Instancia::Normal,
        vec![(503, json!({"message": "indisponível"}))],
    )
    .await;
    transitoria.texto("mensagem com falha transitória");
    transitoria.saida.ciclo().await.expect("ciclo");
    // Prova de que a captura funciona: sem isto, um coletor vazio faria o teste passar à toa.
    tracing::warn!(
        marca = "marca-de-teste-saida-26c",
        "linha de controle do teste"
    );

    let logs = String::from_utf8_lossy(&logs_capturados().lock().unwrap()).to_string();
    assert!(
        logs.contains("marca-de-teste-saida-26c"),
        "a captura de logs precisa estar ativa para o teste valer"
    );
    assert!(!logs.contains(CHAVE), "a chave não aparece em log");
    assert!(!logs.contains(TELEFONE), "o telefone não aparece em log");
    assert!(!logs.contains("s.whatsapp"), "o JID não aparece em log");
}

// ───────────────────────── rastro operacional: uma linha por item ─────────────────────────

/// Primeira linha do log que contém `frase` e `trecho`, ou vazio.
fn linha_com(registro: &str, frase: &str, trecho: &str) -> String {
    registro
        .lines()
        .find(|linha| linha.contains(frase) && linha.contains(trecho))
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn envio_enviado_e_adiado_deixam_uma_linha_cada_sem_dado_sensivel() {
    const TEXTO_ENVIADO: &str = "mensagem que vai sair";
    const TEXTO_ADIADO: &str = "mensagem que vai ser adiada";

    let enviado = Cenario::novo("log-enviado", Instancia::Normal, vec![]).await;
    let item_ok = enviado.texto(TEXTO_ENVIADO);
    enviado.saida.ciclo().await.expect("ciclo");
    assert_eq!(enviado.estado(&item_ok).state, OutboxState::Sent);

    let adiado = Cenario::novo(
        "log-adiado",
        Instancia::Normal,
        vec![(503, json!({"message": "indisponível"}))],
    )
    .await;
    let item_adiado = adiado.texto(TEXTO_ADIADO);
    adiado.saida.ciclo().await.expect("ciclo");
    assert_eq!(adiado.estado(&item_adiado).state, OutboxState::Sending);

    let registro = String::from_utf8_lossy(&logs_capturados().lock().unwrap()).into_owned();
    // Captura viva: as duas linhas precisam aparecer, senão a ausência de dado não prova nada.
    let linha_ok = linha_com(
        &registro,
        "item de saída do WhatsApp enviado",
        &enviado.conversa_wa.to_string(),
    );
    assert!(
        !linha_ok.is_empty(),
        "o envio não deixou linha de log (ou a captura está vazia)"
    );
    assert!(linha_ok.contains("texto"), "tipo ausente: {linha_ok}");

    let linha_adiada = linha_com(
        &registro,
        "envio do WhatsApp adiado",
        &adiado.conversa_wa.to_string(),
    );
    assert!(
        !linha_adiada.is_empty(),
        "o adiamento não deixou linha de log"
    );
    assert!(
        linha_adiada.contains("tentativa=1"),
        "a linha não diz o número da tentativa: {linha_adiada}"
    );
    assert!(
        linha_adiada.contains("indisponível"),
        "a linha não diz o motivo: {linha_adiada}"
    );

    for proibido in [CHAVE, TELEFONE, "s.whatsapp", TEXTO_ENVIADO, TEXTO_ADIADO] {
        assert!(
            !registro.contains(proibido),
            "o log de saída vazou `{proibido}`"
        );
    }
}

// ───────────────────────── id do envio (MX-40) ─────────────────────────

/// Id que a Evolution falsa devolve quando o teste não escreve outro no roteiro.
const ID_DA_EVOLUTION: &str = "MSG-SAIDA-1";

#[tokio::test]
async fn texto_enviado_grava_o_id_que_a_evolution_devolveu() {
    let cenario = Cenario::novo("id-texto", Instancia::Normal, vec![]).await;
    let item = cenario.texto("olá com id");

    let relato = cenario.saida.ciclo().await.expect("ciclo");

    assert_eq!(relato.enviados, 1, "{relato:?}");
    let gravado = cenario
        .store
        .outbox()
        .external_id(cenario.org, item.id)
        .expect("lê o id externo");
    assert_eq!(
        gravado.as_deref(),
        Some(ID_DA_EVOLUTION),
        "o id da resposta do envio tem de ficar na outbox"
    );
    // O id vem da resposta (`key.id`), não do pedido: a Evolution é quem nomeia a mensagem.
    assert_eq!(cenario.estado(&item).state, OutboxState::Sent);
}

#[tokio::test]
async fn arquivo_enviado_grava_o_id_que_a_evolution_devolveu() {
    let cenario = Cenario::novo(
        "id-arquivo",
        Instancia::Normal,
        vec![(201, json!({"key": {"id": "MSG-ARQUIVO-7"}}))],
    )
    .await;
    let conteudo = b"%PDF-1.4 com id".to_vec();
    let blob = cenario.gravar_blob(&"d".repeat(64), &conteudo);
    let item = cenario.enfileirar(
        cenario.conversa_wa,
        2,
        "file",
        json!({
            "blob_ref": blob,
            "arquivo_id": "01a1f6a5-cd9b-7000-8000-000000000002",
            "nome": "contrato.pdf",
            "mime": "application/pdf",
            "tamanho_bytes": conteudo.len(),
            "sha256": "d".repeat(64),
        }),
    );

    let relato = cenario.saida.ciclo().await.expect("ciclo");

    assert_eq!(relato.enviados, 1, "{relato:?}");
    assert_eq!(
        cenario
            .store
            .outbox()
            .external_id(cenario.org, item.id)
            .expect("lê o id externo")
            .as_deref(),
        Some("MSG-ARQUIVO-7"),
        "o envio de arquivo também guarda o id"
    );
}

#[tokio::test]
async fn envio_que_falha_nao_grava_id() {
    // Definitivo: o canal recusa o conteúdo.
    let recusado = Cenario::novo(
        "id-recusado",
        Instancia::Normal,
        vec![(400, json!({"message": "número inválido"}))],
    )
    .await;
    let item_recusado = recusado.texto("não vai sair");
    let relato = recusado.saida.ciclo().await.expect("ciclo");
    assert_eq!(relato.definitivos, 1, "{relato:?}");
    assert_eq!(
        recusado
            .store
            .outbox()
            .external_id(recusado.org, item_recusado.id)
            .expect("lê o id externo"),
        None,
        "envio recusado não pode ficar com id"
    );

    // Transitório: volta para a fila sem id.
    let adiado = Cenario::novo(
        "id-adiado",
        Instancia::Normal,
        vec![(503, json!({"message": "indisponível"}))],
    )
    .await;
    let item_adiado = adiado.texto("tenta depois");
    let relato = adiado.saida.ciclo().await.expect("ciclo");
    assert_eq!(relato.transitorios, 1, "{relato:?}");
    assert_eq!(
        adiado
            .store
            .outbox()
            .external_id(adiado.org, item_adiado.id)
            .expect("lê o id externo"),
        None,
        "envio adiado não pode ficar com id"
    );
}

#[tokio::test]
async fn canal_sem_id_na_resposta_envia_e_fica_sem_id() {
    // Evolution responde 200 sem id nenhum: o envio vale, e a coluna fica vazia (não é falha).
    let cenario = Cenario::novo(
        "id-ausente",
        Instancia::Normal,
        vec![(200, json!({"status": "ok"}))],
    )
    .await;
    let item = cenario.texto("sem id na resposta");

    let relato = cenario.saida.ciclo().await.expect("ciclo");

    assert_eq!(relato.enviados, 1, "{relato:?}");
    assert_eq!(cenario.estado(&item).state, OutboxState::Sent);
    assert_eq!(
        cenario
            .store
            .outbox()
            .external_id(cenario.org, item.id)
            .expect("lê o id externo"),
        None
    );
}
