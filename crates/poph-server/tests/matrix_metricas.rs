//! Métricas e logs do módulo Matrix (MX-26).
//!
//! Banco de verdade (`Store` numa pasta temporária, relógio controlado), Synapse **falso** em
//! memória (o mesmo duplo de `matrix_espelho.rs`, só com o que estes testes pedem), e, para o
//! `/metrics`, o servidor HTTP de verdade.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poph_matrix::error::CallError;
use poph_matrix::format::Outbound;
use poph_matrix::rooms::{ConversationToRoom, RoomError};
use poph_matrix::users::GhostIds;
use poph_server::matrix_metrics::MetricsMatrix;
use poph_server::matrix_mirror::{MatrixMirror, MatrixTarget, PublishedAsset};
use poph_server::{AttendantResponse, EventDelivery, HarnessResponsePort, ServerState};
use poph_store::{Direction, NewAgent, NewConversation, NewMessage, NewOrg, Origin, Role, Store};
use poph_types::{AgentId, ConversationId, HarnessConfig, MatrixConfig, OrgId, Registry};
use serde_json::{Value, json};

const SERVIDOR: &str = "matrix-dev.local";

// ───────────────────────────── cenário ─────────────────────────────

struct Cenario {
    pasta: PathBuf,
    store: Option<Arc<Store>>,
}

impl Cenario {
    fn new(rotulo: &str) -> Self {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta =
            std::env::temp_dir().join(format!("poph-metricas-{rotulo}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("cria a pasta do teste");
        // Relógio do banco: anda 1 ms a cada gravação, então a ordem de criação é estrita.
        let relogio = Arc::new(AtomicI64::new(1_000));
        let store = Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db"))
            .expect("abre o banco")
            .with_clock(Arc::new(move || relogio.fetch_add(1, Ordering::SeqCst)));
        Self {
            pasta,
            store: Some(Arc::new(store)),
        }
    }

    fn store(&self) -> Arc<Store> {
        Arc::clone(self.store.as_ref().expect("store viva"))
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn config() -> MatrixConfig {
    MatrixConfig {
        enabled: true,
        homeserver_url: "http://127.0.0.1:1".to_owned(),
        server_name: SERVIDOR.to_owned(),
        prefix: "ph".to_owned(),
        concurrency: 8,
        interval_ms: 500,
        ..MatrixConfig::default()
    }
}

fn criar_org(store: &Store, slug: &str) -> OrgId {
    store
        .orgs()
        .create(NewOrg {
            name: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id
}

fn criar_agente(store: &Store, org: OrgId) -> AgentId {
    store
        .agents()
        .create(
            org,
            NewAgent {
                name: "Bia".to_owned(),
                slug: "bia".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente")
        .id
}

fn criar_conversa(store: &Store, org: OrgId, agent: AgentId, peer: &str) -> ConversationId {
    store
        .conversations()
        .create(
            org,
            NewConversation {
                agent_id: agent,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: Some(peer.to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa")
        .id
}

fn msg_cliente(store: &Store, org: OrgId, conversation: ConversationId, text: &str) -> String {
    store
        .messages()
        .write(
            org,
            NewMessage {
                conversation_id: conversation,
                direction: Direction::Inbound,
                role: Role::User,
                origin: Origin::Agente,
                author_name: None,
                texto: text.to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
            },
        )
        .expect("grava a mensagem")
        .id
        .to_string()
}

fn metricas_novas() -> Arc<MetricsMatrix> {
    Arc::new(MetricsMatrix::register(&Registry::new()).expect("registra as métricas"))
}

// ───────────────────────────── Synapse falso ─────────────────────────────

#[derive(Clone, Copy)]
enum Modo {
    /// Aceita tudo.
    Normal,
    /// Toda chamada falha de forma transitória (Synapse fora do ar).
    Fora,
    /// Toda chamada responde `429` (limite de taxa).
    Limite,
    /// Corpo com o texto é recusado com `403` (definitivo); o resto é aceito.
    Venenoso(&'static str),
}

struct Falso {
    modo: Mutex<Modo>,
    /// Texto que o Synapse devolve dentro do erro. Usado pelo teste do valor-canário.
    mensagem_de_erro: String,
    /// Chamadas de envio feitas (inclusive as recusadas).
    envios: AtomicUsize,
}

impl Falso {
    fn new(modo: Modo, mensagem_de_erro: &str) -> Arc<Self> {
        Arc::new(Self {
            modo: Mutex::new(modo),
            mensagem_de_erro: mensagem_de_erro.to_owned(),
            envios: AtomicUsize::new(0),
        })
    }

    fn definir_modo(&self, modo: Modo) {
        *self.modo.lock().unwrap() = modo;
    }

    fn error(&self, status: u16, errcode: &str) -> CallError {
        CallError::Response {
            operation: "send",
            status,
            errcode: Some(errcode.to_owned()),
            message: self.mensagem_de_erro.clone(),
            retry_after_ms: None,
        }
    }
}

/// O duplo que o espelho recebe: o espelho fica com uma cópia do `Arc`, o teste com outra.
struct Sinal(Arc<Falso>);

impl MatrixTarget for Sinal {
    fn ensure_room(
        &self,
        dados: &ConversationToRoom,
    ) -> impl std::future::Future<Output = Result<String, RoomError>> + Send {
        let room = format!("!{}:{SERVIDOR}", dados.conversation);
        async move { Ok(room) }
    }

    async fn ensure_device(
        &self,
        _dados: &ConversationToRoom,
        _sala: &str,
        _aparelho: &poph_matrix::users::Ghost,
    ) -> Result<(), RoomError> {
        Ok(())
    }

    fn send(
        &self,
        _sala: &str,
        send: &Outbound,
    ) -> impl std::future::Future<Output = Result<(), CallError>> + Send {
        let falso = &self.0;
        falso.envios.fetch_add(1, Ordering::SeqCst);
        let modo = *falso.modo.lock().unwrap();
        let resultado = match modo {
            Modo::Normal => Ok(()),
            Modo::Fora => Err(falso.error(503, "M_UNKNOWN")),
            Modo::Limite => Err(falso.error(429, "M_LIMIT_EXCEEDED")),
            Modo::Venenoso(text) if send.body.contains(text) => {
                Err(falso.error(403, "M_FORBIDDEN"))
            }
            Modo::Venenoso(_) => Ok(()),
        };
        async move { resultado }
    }

    async fn send_asset(
        &self,
        _sala: &str,
        _envio: &poph_server::matrix_mirror::AssetSend,
    ) -> Result<PublishedAsset, CallError> {
        // Este destino não publica mídia: nenhuma mensagem do teste tem anexo.
        Err(CallError::Configuration(
            "sem mídia neste destino".to_owned(),
        ))
    }

    async fn send_notice(
        &self,
        _sala: &str,
        _aviso: &poph_server::matrix_mirror::RoomNotice,
    ) -> Result<(), CallError> {
        Ok(())
    }
}

// ───────────────────────────── captura de log ─────────────────────────────

/// Escritor que guarda tudo o que o `tracing` imprime, para o teste conferir o texto.
#[derive(Clone, Default)]
struct Captura(Arc<Mutex<Vec<u8>>>);

impl Captura {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl Write for Captura {
    fn write(&mut self, dados: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(dados);
        Ok(dados.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Liga a captura só nesta thread (os testes são de `current_thread`) até o guarda cair.
fn capturar_logs() -> (Captura, tracing::subscriber::DefaultGuard) {
    let captura = Captura::default();
    let destino = captura.clone();
    let assinante = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .without_time()
        .with_writer(move || destino.clone())
        .finish();
    let guarda = tracing::subscriber::set_default(assinante);
    (captura, guarda)
}

// ───────────────────────────── testes do espelho ─────────────────────────────

#[tokio::test]
async fn atraso_do_cursor_sobe_com_synapse_fora_e_volta_a_zero() {
    let cenario = Cenario::new("atraso");
    let store = cenario.store();
    let org = criar_org(&store, "atraso");
    let agent = criar_agente(&store, org);
    let conversation = criar_conversa(&store, org, agent, "5521977770001");
    store.matrix().enable(org).expect("liga o módulo");
    msg_cliente(&store, org, conversation, "primeira");
    msg_cliente(&store, org, conversation, "segunda");

    let falso = Falso::new(Modo::Fora, "indisponível");
    let metricas = metricas_novas();
    // Relógio do laço: 31 s depois da primeira mensagem, depois 91 s.
    let agora = Arc::new(AtomicI64::new(31_000));
    let relogio = Arc::clone(&agora);
    let espelho = MatrixMirror::new(Arc::clone(&store), Sinal(Arc::clone(&falso)), &config())
        .expect("monta o espelho")
        .with_metrics(Arc::clone(&metricas))
        .with_clock(Arc::new(move || relogio.load(Ordering::SeqCst)));

    espelho.cycle().await.expect("ciclo com o Synapse fora");
    let inicial = metricas.cursor_age_seconds.value();
    assert!(
        (29..=31).contains(&inicial),
        "com o Synapse fora, a primeira mensagem tem ~30 s de atraso; veio {inicial}"
    );

    agora.store(91_000, Ordering::SeqCst);
    espelho
        .cycle()
        .await
        .expect("ciclo com o Synapse ainda fora");
    let depois = metricas.cursor_age_seconds.value();
    assert!(
        depois > inicial && (89..=91).contains(&depois),
        "o atraso tem de subir enquanto o Synapse está fora; veio {depois} (antes {inicial})"
    );

    falso.definir_modo(Modo::Normal);
    espelho.cycle().await.expect("ciclo com o Synapse de volta");
    assert_eq!(
        metricas.cursor_age_seconds.value(),
        0,
        "com o Synapse de volta e tudo espelhado, o atraso volta a zero"
    );
    assert_eq!(metricas.mirrored_messages.value(), 2);
}

#[tokio::test]
async fn contadores_do_espelho_contam_tentativas_erros_e_429() {
    let cenario = Cenario::new("contadores");
    let store = cenario.store();
    let org = criar_org(&store, "contadores");
    let agent = criar_agente(&store, org);
    let conversation = criar_conversa(&store, org, agent, "5521977770002");
    store.matrix().enable(org).expect("liga o módulo");
    msg_cliente(&store, org, conversation, "um");
    msg_cliente(&store, org, conversation, "dois");

    let falso = Falso::new(Modo::Limite, "limite");
    let metricas = metricas_novas();
    let espelho = MatrixMirror::new(Arc::clone(&store), Sinal(Arc::clone(&falso)), &config())
        .expect("monta o espelho")
        .with_metrics(Arc::clone(&metricas));

    // Primeiro ciclo: a sala é garantida (1 tentativa) e o primeiro envio leva 429 (1 tentativa).
    espelho.cycle().await.expect("ciclo com limite de taxa");
    assert_eq!(metricas.attempts.value(), 2);
    assert_eq!(metricas.responses_429.value(), 1, "o 429 conta uma vez");
    assert_eq!(
        metricas.transient_errors.value(),
        1,
        "o 429 é transitório: conta como erro transitório"
    );
    assert_eq!(metricas.definitive_errors.value(), 0);
    assert_eq!(metricas.mirrored_messages.value(), 0);

    // Segundo ciclo, sem limite: sala (1) + duas mensagens (2) = 3 tentativas a mais.
    falso.definir_modo(Modo::Normal);
    espelho.cycle().await.expect("ciclo sem limite");
    assert_eq!(metricas.attempts.value(), 5);
    assert_eq!(metricas.mirrored_messages.value(), 2);
    assert_eq!(
        metricas.responses_429.value(),
        1,
        "sem novo 429, o contador fica"
    );
}

#[tokio::test]
async fn mensagem_venenosa_isolada_conta_e_loga_org_e_conversa() {
    let cenario = Cenario::new("isolada");
    let store = cenario.store();
    let org = criar_org(&store, "isolada");
    let agent = criar_agente(&store, org);
    let conversation = criar_conversa(&store, org, agent, "5521977770003");
    store.matrix().enable(org).expect("liga o módulo");
    msg_cliente(&store, org, conversation, "VENENO aqui");
    msg_cliente(&store, org, conversation, "depois");

    let falso = Falso::new(Modo::Venenoso("VENENO"), "recusado");
    let metricas = metricas_novas();
    let espelho = MatrixMirror::new(Arc::clone(&store), Sinal(Arc::clone(&falso)), &config())
        .expect("monta o espelho")
        .with_metrics(Arc::clone(&metricas));
    let (captura, _guarda) = capturar_logs();

    // Três falhas definitivas seguidas isolam a mensagem venenosa; a seguinte sai no ciclo depois.
    for _ in 0..4 {
        espelho.cycle().await.expect("ciclo");
    }

    assert_eq!(metricas.definitive_errors.value(), 3);
    assert_eq!(metricas.isolated_messages.value(), 1);
    assert_eq!(
        metricas.mirrored_messages.value(),
        1,
        "a mensagem seguinte sai"
    );
    let log = captura.text();
    // A linha do aviso de isolamento (não a de nova tentativa) é que leva org e conversa.
    let linha = log
        .lines()
        .find(|linha| linha.contains("mensagem isolada"))
        .unwrap_or_else(|| panic!("falta o aviso de isolamento no log:\n{log}"));
    assert!(
        linha.contains(&conversation.to_string()) && linha.contains(&org.to_string()),
        "o aviso de isolamento leva org e conversa; linha: {linha}"
    );
}

// ───────────────────────────── valor-canário ─────────────────────────────

/// Valores que não podem aparecer em log: telefone, texto de mensagem e token.
const CANARIO_TELEFONE: &str = "5521900418273";
const CANARIO_TEXTO: &str = "CANARIO-TEXTO-7a3f9";
const CANARIO_TOKEN: &str = "CANARIO-TOKEN-x81kq";

#[tokio::test]
async fn valor_canario_nunca_aparece_no_log_do_modulo() {
    let cenario = Cenario::new("canario");
    let store = cenario.store();
    let org = criar_org(&store, "canario");
    let agent = criar_agente(&store, org);
    let conversation = criar_conversa(&store, org, agent, CANARIO_TELEFONE);
    store.matrix().enable(org).expect("liga o módulo");
    msg_cliente(&store, org, conversation, CANARIO_TEXTO);
    // O Synapse devolve o token-canário dentro da mensagem de erro (não pode ir para o log).
    let falso = Falso::new(Modo::Venenoso(CANARIO_TEXTO), CANARIO_TOKEN);
    let espelho = MatrixMirror::new(Arc::clone(&store), Sinal(Arc::clone(&falso)), &config())
        .expect("monta o espelho")
        .with_metrics(metricas_novas());

    // Caminho do espelho: falhas definitivas até isolar, e uma transitória antes.
    let (captura, guarda) = capturar_logs();
    falso.definir_modo(Modo::Fora);
    espelho.cycle().await.expect("ciclo transitório");
    falso.definir_modo(Modo::Venenoso(CANARIO_TEXTO));
    for _ in 0..3 {
        espelho.cycle().await.expect("ciclo definitivo");
    }

    // Caminho da resposta do atendente: uma resposta humana numa sala conhecida.
    let room = "!canario-sala:matrix-dev.local";
    store
        .matrix()
        .ensure_room(org, conversation, room, None)
        .expect("grava a sala");
    store
        .matrix()
        .mark_room_ready(org, conversation)
        .expect("sala pronta");
    let ids = GhostIds::of_config(&config()).expect("ids");
    let entrega = AttendantResponse::with_caps(
        Arc::new(HarnessResponsePort::fresh(Arc::clone(&store))),
        None,
        ids,
        16,
        16,
        Duration::from_secs(300),
    )
    .with_metrics(metricas_novas());
    let event: Value = json!({
        "type": "m.room.message",
        "room_id": room,
        "event_id": "$canario-1:matrix-dev.local",
        "sender": "@willian:matrix-dev.local",
        "content": { "msgtype": "m.text", "body": CANARIO_TEXTO },
    });
    entrega
        .deliver("txn-canario", vec![event])
        .await
        .expect("entrega aceita a resposta");
    drop(guarda);

    let log = captura.text();
    assert!(
        log.contains("isolada") || log.contains("resposta do atendente enfileirada"),
        "o teste precisa ter logs de verdade para valer; log:\n{log}"
    );
    for canario in [CANARIO_TELEFONE, CANARIO_TEXTO, CANARIO_TOKEN] {
        assert!(
            !log.contains(canario),
            "o valor-canário `{canario}` vazou para o log:\n{log}"
        );
    }
}

// ───────────────────── contadores da resposta do atendente ─────────────────────

/// Os contadores que a MX-18 já mantinha em memória viram métrica: enfileiradas, duplicadas, sala
/// desconhecida, conversa encerrada e aviso de mídia.
#[tokio::test]
async fn contadores_da_resposta_do_atendente_viram_metrica() {
    let cenario = Cenario::new("contadores-resposta");
    let store = cenario.store();
    let org = criar_org(&store, "contadores-resposta");
    let agent = criar_agente(&store, org);
    let conversation = criar_conversa(&store, org, agent, "5521977770004");
    store.matrix().enable(org).expect("liga o módulo");
    let room = "!contadores-sala:matrix-dev.local";
    store
        .matrix()
        .ensure_room(org, conversation, room, None)
        .expect("grava a sala");
    store
        .matrix()
        .mark_room_ready(org, conversation)
        .expect("sala pronta");

    let metricas = metricas_novas();
    let ids = GhostIds::of_config(&config()).expect("ids");
    let entrega = AttendantResponse::with_caps(
        Arc::new(HarnessResponsePort::fresh(Arc::clone(&store))),
        None,
        ids,
        16,
        16,
        Duration::from_secs(300),
    )
    .with_metrics(Arc::clone(&metricas));

    let text = |event: &str, room: &str, body: Value| {
        json!({
            "type": "m.room.message",
            "room_id": room,
            "event_id": event,
            "sender": "@atendente:matrix-dev.local",
            "content": body,
        })
    };
    let humano = || json!({ "msgtype": "m.text", "body": "olá" });

    // 1) Resposta nova: enfileirada uma vez.
    entrega
        .deliver("txn-1", vec![text("$c-1:matrix-dev.local", room, humano())])
        .await
        .expect("entrega");
    assert_eq!(metricas.responses_queued.value(), 1);

    // 2) Mesmo evento de novo (reenvio do Synapse): duplicada, nada novo enfileirado.
    entrega
        .deliver("txn-2", vec![text("$c-1:matrix-dev.local", room, humano())])
        .await
        .expect("entrega");
    assert_eq!(metricas.responses_queued.value(), 1);
    assert_eq!(metricas.responses_duplicated.value(), 1);

    // 3) Sala que ninguém conhece.
    entrega
        .deliver(
            "txn-3",
            vec![text(
                "$c-2:matrix-dev.local",
                "!desconhecida:matrix-dev.local",
                humano(),
            )],
        )
        .await
        .expect("entrega");
    assert_eq!(metricas.rooms_unknown.value(), 1);

    // 4) Mídia do atendente: só o aviso na sala.
    entrega
        .deliver(
            "txn-4",
            vec![text(
                "$c-3:matrix-dev.local",
                room,
                json!({ "msgtype": "m.image", "body": "foto.jpg" }),
            )],
        )
        .await
        .expect("entrega");
    assert_eq!(metricas.asset_notices.value(), 1);
    assert_eq!(
        metricas.responses_queued.value(),
        1,
        "mídia não vira resposta ao cliente"
    );

    // 5) Conversa encerrada: nada enviado, aviso na sala.
    let versao = store
        .conversations()
        .get(org, conversation)
        .expect("lê a conversa")
        .version;
    store
        .conversations()
        .close_audited(org, conversation, versao, "teste".to_owned())
        .expect("encerra a conversa");
    entrega
        .deliver("txn-5", vec![text("$c-4:matrix-dev.local", room, humano())])
        .await
        .expect("entrega");
    assert_eq!(metricas.responses_conversation_closed.value(), 1);
    assert_eq!(
        metricas.responses_queued.value(),
        1,
        "conversa encerrada não enfileira para o cliente"
    );
}

// ───────────────────────────── /metrics ─────────────────────────────

/// Sobe o servidor com o estado dado e devolve o texto de `/metrics`.
async fn metricas_http(state: ServerState) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = listener.local_addr().expect("endereço");
    let (desligar, sinal) = tokio::sync::oneshot::channel::<()>();
    let servidor = tokio::spawn(async move {
        let _ = poph_server::serve_at(listener, state, async {
            let _ = sinal.await;
        })
        .await;
    });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("cliente HTTP");
    let mut body = None;
    for _ in 0..100 {
        if let Ok(resposta) = client
            .get(format!("http://{endereco}/metrics"))
            .send()
            .await
        {
            body = Some(resposta.text().await.expect("corpo do /metrics"));
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let _ = desligar.send(());
    let _ = servidor.await;
    body.expect("o /metrics respondeu")
}

fn estado_com_modulo(rotulo: &str, ligado: bool) -> (ServerState, PathBuf) {
    let pasta = std::env::temp_dir().join(format!(
        "poph-metricas-http-{rotulo}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");
    let mut cfg = HarnessConfig::default();
    cfg.data.directory = pasta.join("dados");
    cfg.matrix = Some(MatrixConfig {
        enabled: ligado,
        ..config()
    });
    let store = Arc::new(Store::open(&cfg).expect("abre o banco"));
    let state = ServerState::new(store, cfg).expect("estado do servidor");
    (state, pasta)
}

const NOMES_MATRIX: [&str; 15] = [
    "poph_matrix_atraso_cursor_segundos",
    "poph_matrix_salas_prontas_total",
    "poph_matrix_mensagens_espelhadas_total",
    "poph_matrix_midias_espelhadas_total",
    "poph_matrix_respostas_enfileiradas_total",
    "poph_matrix_respostas_duplicadas_total",
    "poph_matrix_respostas_conversa_encerrada_total",
    "poph_matrix_salas_desconhecidas_total",
    "poph_matrix_avisos_de_midia_total",
    "poph_matrix_erros_transitorios_total",
    "poph_matrix_erros_definitivos_total",
    "poph_matrix_respostas_429_total",
    "poph_matrix_tentativas_total",
    "poph_matrix_mensagens_isoladas_total",
    "poph_matrix_latencia_synapse_segundos",
];

#[tokio::test]
async fn metricas_do_modulo_nao_aparecem_com_o_modulo_desligado() {
    let (state, pasta) = estado_com_modulo("desligado", false);
    // Módulo desligado: nem quem prepara o módulo consegue declarar as métricas.
    assert!(
        state.matrix_metrics().expect("sem erro").is_none(),
        "com [matrix] desligado, as métricas do módulo não são declaradas"
    );
    let text = metricas_http(state).await;
    let _ = fs::remove_dir_all(&pasta);
    assert!(
        text.contains("poph_memoria_processo_bytes"),
        "sanidade: o /metrics responde"
    );
    assert!(
        !text.contains("poph_matrix_"),
        "com o módulo desligado nenhuma poph_matrix_* pode aparecer:\n{text}"
    );
}

#[tokio::test]
async fn metricas_do_modulo_aparecem_uma_vez_quando_preparado() {
    let (state, pasta) = estado_com_modulo("ligado", true);
    // Preparar duas vezes (como a raiz faria em reinícios internos) não duplica nada na saída.
    let primeira = state
        .matrix_metrics()
        .expect("prepara as métricas")
        .expect("módulo ligado");
    let segunda = state
        .matrix_metrics()
        .expect("segunda chamada")
        .expect("módulo ligado");
    assert!(
        Arc::ptr_eq(&primeira, &segunda),
        "mesmo handle nas duas chamadas"
    );
    primeira.mirrored_messages.increment(3);
    let text = metricas_http(state).await;
    let _ = fs::remove_dir_all(&pasta);
    for name in NOMES_MATRIX {
        assert_eq!(
            text.matches(&format!("# HELP {name} ")).count(),
            1,
            "{name} deve aparecer exatamente uma vez no /metrics"
        );
    }
    assert!(
        text.contains("poph_matrix_mensagens_espelhadas_total 3\n"),
        "o valor incrementado aparece no /metrics"
    );
}

// ───────────────────────────── registro do processo ─────────────────────────────

#[test]
fn metricas_descartaveis_nao_aparecem_no_registro_do_processo() {
    // Handles descartáveis (usados quando ninguém injeta os números do processo) ficam fora do
    // `/metrics` do processo.
    let processo = Registry::new();
    let _ = MetricsMatrix::register(&processo).expect("registra no processo");
    let descartavel = MetricsMatrix::unregistered();
    descartavel.definitive_errors.increment(1);
    assert!(
        !processo
            .render()
            .contains("poph_matrix_erros_definitivos_total 1")
    );
}
