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

use poph_matrix::erro::ErroChamada;
use poph_matrix::formato::Envio;
use poph_matrix::salas::{ConversaParaSala, ErroSala};
use poph_matrix::usuarios::IdsFantasmas;
use poph_server::matrix_espelho::{DestinoMatrix, EspelhoMatrix, MidiaPublicada};
use poph_server::matrix_metricas::MetricasMatrix;
use poph_server::{EntregaDeEventos, Estado, PortaDoHarness, RespostaDoAtendente};
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
    fn novo(rotulo: &str) -> Self {
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

fn criar_conversa(store: &Store, org: OrgId, agente: AgentId, peer: &str) -> ConversationId {
    store
        .conversations()
        .create(
            org,
            NewConversation {
                agent_id: agente,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: Some(peer.to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa")
        .id
}

fn msg_cliente(store: &Store, org: OrgId, conversa: ConversationId, texto: &str) -> String {
    store
        .messages()
        .write(
            org,
            NewMessage {
                conversation_id: conversa,
                direction: Direction::Inbound,
                role: Role::User,
                origin: Origin::Agente,
                author_name: None,
                texto: texto.to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
            },
        )
        .expect("grava a mensagem")
        .id
        .to_string()
}

fn metricas_novas() -> Arc<MetricasMatrix> {
    Arc::new(MetricasMatrix::registrar(&Registry::new()).expect("registra as métricas"))
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
    fn novo(modo: Modo, mensagem_de_erro: &str) -> Arc<Self> {
        Arc::new(Self {
            modo: Mutex::new(modo),
            mensagem_de_erro: mensagem_de_erro.to_owned(),
            envios: AtomicUsize::new(0),
        })
    }

    fn definir_modo(&self, modo: Modo) {
        *self.modo.lock().unwrap() = modo;
    }

    fn erro(&self, status: u16, errcode: &str) -> ErroChamada {
        ErroChamada::Resposta {
            operacao: "send",
            status,
            errcode: Some(errcode.to_owned()),
            mensagem: self.mensagem_de_erro.clone(),
            retry_after_ms: None,
        }
    }
}

/// O duplo que o espelho recebe: o espelho fica com uma cópia do `Arc`, o teste com outra.
struct Sinal(Arc<Falso>);

impl DestinoMatrix for Sinal {
    fn garantir_sala(
        &self,
        dados: &ConversaParaSala,
    ) -> impl std::future::Future<Output = Result<String, ErroSala>> + Send {
        let sala = format!("!{}:{SERVIDOR}", dados.conversa);
        async move { Ok(sala) }
    }

    async fn garantir_aparelho(
        &self,
        _dados: &ConversaParaSala,
        _sala: &str,
        _aparelho: &poph_matrix::usuarios::Fantasma,
    ) -> Result<(), ErroSala> {
        Ok(())
    }

    fn enviar(
        &self,
        _sala: &str,
        envio: &Envio,
    ) -> impl std::future::Future<Output = Result<(), ErroChamada>> + Send {
        let falso = &self.0;
        falso.envios.fetch_add(1, Ordering::SeqCst);
        let modo = *falso.modo.lock().unwrap();
        let resultado = match modo {
            Modo::Normal => Ok(()),
            Modo::Fora => Err(falso.erro(503, "M_UNKNOWN")),
            Modo::Limite => Err(falso.erro(429, "M_LIMIT_EXCEEDED")),
            Modo::Venenoso(texto) if envio.corpo.contains(texto) => {
                Err(falso.erro(403, "M_FORBIDDEN"))
            }
            Modo::Venenoso(_) => Ok(()),
        };
        async move { resultado }
    }

    async fn enviar_midia(
        &self,
        _sala: &str,
        _envio: &poph_server::matrix_espelho::EnvioMidia,
    ) -> Result<MidiaPublicada, ErroChamada> {
        // Este destino não publica mídia: nenhuma mensagem do teste tem anexo.
        Err(ErroChamada::Configuracao(
            "sem mídia neste destino".to_owned(),
        ))
    }

    async fn avisar(
        &self,
        _sala: &str,
        _aviso: &poph_server::matrix_espelho::AvisoNaSala,
    ) -> Result<(), ErroChamada> {
        Ok(())
    }
}

// ───────────────────────────── captura de log ─────────────────────────────

/// Escritor que guarda tudo o que o `tracing` imprime, para o teste conferir o texto.
#[derive(Clone, Default)]
struct Captura(Arc<Mutex<Vec<u8>>>);

impl Captura {
    fn texto(&self) -> String {
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
    let cenario = Cenario::novo("atraso");
    let store = cenario.store();
    let org = criar_org(&store, "atraso");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "5521977770001");
    store.matrix().enable(org).expect("liga o módulo");
    msg_cliente(&store, org, conversa, "primeira");
    msg_cliente(&store, org, conversa, "segunda");

    let falso = Falso::novo(Modo::Fora, "indisponível");
    let metricas = metricas_novas();
    // Relógio do laço: 31 s depois da primeira mensagem, depois 91 s.
    let agora = Arc::new(AtomicI64::new(31_000));
    let relogio = Arc::clone(&agora);
    let espelho = EspelhoMatrix::novo(Arc::clone(&store), Sinal(Arc::clone(&falso)), &config())
        .expect("monta o espelho")
        .com_metricas(Arc::clone(&metricas))
        .com_relogio(Arc::new(move || relogio.load(Ordering::SeqCst)));

    espelho.ciclo().await.expect("ciclo com o Synapse fora");
    let inicial = metricas.atraso_cursor_segundos.value();
    assert!(
        (29..=31).contains(&inicial),
        "com o Synapse fora, a primeira mensagem tem ~30 s de atraso; veio {inicial}"
    );

    agora.store(91_000, Ordering::SeqCst);
    espelho
        .ciclo()
        .await
        .expect("ciclo com o Synapse ainda fora");
    let depois = metricas.atraso_cursor_segundos.value();
    assert!(
        depois > inicial && (89..=91).contains(&depois),
        "o atraso tem de subir enquanto o Synapse está fora; veio {depois} (antes {inicial})"
    );

    falso.definir_modo(Modo::Normal);
    espelho.ciclo().await.expect("ciclo com o Synapse de volta");
    assert_eq!(
        metricas.atraso_cursor_segundos.value(),
        0,
        "com o Synapse de volta e tudo espelhado, o atraso volta a zero"
    );
    assert_eq!(metricas.mensagens_espelhadas.value(), 2);
}

#[tokio::test]
async fn contadores_do_espelho_contam_tentativas_erros_e_429() {
    let cenario = Cenario::novo("contadores");
    let store = cenario.store();
    let org = criar_org(&store, "contadores");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "5521977770002");
    store.matrix().enable(org).expect("liga o módulo");
    msg_cliente(&store, org, conversa, "um");
    msg_cliente(&store, org, conversa, "dois");

    let falso = Falso::novo(Modo::Limite, "limite");
    let metricas = metricas_novas();
    let espelho = EspelhoMatrix::novo(Arc::clone(&store), Sinal(Arc::clone(&falso)), &config())
        .expect("monta o espelho")
        .com_metricas(Arc::clone(&metricas));

    // Primeiro ciclo: a sala é garantida (1 tentativa) e o primeiro envio leva 429 (1 tentativa).
    espelho.ciclo().await.expect("ciclo com limite de taxa");
    assert_eq!(metricas.tentativas.value(), 2);
    assert_eq!(metricas.respostas_429.value(), 1, "o 429 conta uma vez");
    assert_eq!(
        metricas.erros_transitorios.value(),
        1,
        "o 429 é transitório: conta como erro transitório"
    );
    assert_eq!(metricas.erros_definitivos.value(), 0);
    assert_eq!(metricas.mensagens_espelhadas.value(), 0);

    // Segundo ciclo, sem limite: sala (1) + duas mensagens (2) = 3 tentativas a mais.
    falso.definir_modo(Modo::Normal);
    espelho.ciclo().await.expect("ciclo sem limite");
    assert_eq!(metricas.tentativas.value(), 5);
    assert_eq!(metricas.mensagens_espelhadas.value(), 2);
    assert_eq!(
        metricas.respostas_429.value(),
        1,
        "sem novo 429, o contador fica"
    );
}

#[tokio::test]
async fn mensagem_venenosa_isolada_conta_e_loga_org_e_conversa() {
    let cenario = Cenario::novo("isolada");
    let store = cenario.store();
    let org = criar_org(&store, "isolada");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "5521977770003");
    store.matrix().enable(org).expect("liga o módulo");
    msg_cliente(&store, org, conversa, "VENENO aqui");
    msg_cliente(&store, org, conversa, "depois");

    let falso = Falso::novo(Modo::Venenoso("VENENO"), "recusado");
    let metricas = metricas_novas();
    let espelho = EspelhoMatrix::novo(Arc::clone(&store), Sinal(Arc::clone(&falso)), &config())
        .expect("monta o espelho")
        .com_metricas(Arc::clone(&metricas));
    let (captura, _guarda) = capturar_logs();

    // Três falhas definitivas seguidas isolam a mensagem venenosa; a seguinte sai no ciclo depois.
    for _ in 0..4 {
        espelho.ciclo().await.expect("ciclo");
    }

    assert_eq!(metricas.erros_definitivos.value(), 3);
    assert_eq!(metricas.mensagens_isoladas.value(), 1);
    assert_eq!(
        metricas.mensagens_espelhadas.value(),
        1,
        "a mensagem seguinte sai"
    );
    let log = captura.texto();
    // A linha do aviso de isolamento (não a de nova tentativa) é que leva org e conversa.
    let linha = log
        .lines()
        .find(|linha| linha.contains("mensagem isolada"))
        .unwrap_or_else(|| panic!("falta o aviso de isolamento no log:\n{log}"));
    assert!(
        linha.contains(&conversa.to_string()) && linha.contains(&org.to_string()),
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
    let cenario = Cenario::novo("canario");
    let store = cenario.store();
    let org = criar_org(&store, "canario");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, CANARIO_TELEFONE);
    store.matrix().enable(org).expect("liga o módulo");
    msg_cliente(&store, org, conversa, CANARIO_TEXTO);
    // O Synapse devolve o token-canário dentro da mensagem de erro (não pode ir para o log).
    let falso = Falso::novo(Modo::Venenoso(CANARIO_TEXTO), CANARIO_TOKEN);
    let espelho = EspelhoMatrix::novo(Arc::clone(&store), Sinal(Arc::clone(&falso)), &config())
        .expect("monta o espelho")
        .com_metricas(metricas_novas());

    // Caminho do espelho: falhas definitivas até isolar, e uma transitória antes.
    let (captura, guarda) = capturar_logs();
    falso.definir_modo(Modo::Fora);
    espelho.ciclo().await.expect("ciclo transitório");
    falso.definir_modo(Modo::Venenoso(CANARIO_TEXTO));
    for _ in 0..3 {
        espelho.ciclo().await.expect("ciclo definitivo");
    }

    // Caminho da resposta do atendente: uma resposta humana numa sala conhecida.
    let sala = "!canario-sala:matrix-dev.local";
    store
        .matrix()
        .ensure_room(org, conversa, sala, None)
        .expect("grava a sala");
    store
        .matrix()
        .mark_room_ready(org, conversa)
        .expect("sala pronta");
    let ids = IdsFantasmas::da_config(&config()).expect("ids");
    let entrega = RespostaDoAtendente::com_tetos(
        Arc::new(PortaDoHarness::nova(Arc::clone(&store))),
        None,
        ids,
        16,
        16,
        Duration::from_secs(300),
    )
    .com_metricas(metricas_novas());
    let evento: Value = json!({
        "type": "m.room.message",
        "room_id": sala,
        "event_id": "$canario-1:matrix-dev.local",
        "sender": "@willian:matrix-dev.local",
        "content": { "msgtype": "m.text", "body": CANARIO_TEXTO },
    });
    entrega
        .entregar("txn-canario", vec![evento])
        .await
        .expect("entrega aceita a resposta");
    drop(guarda);

    let log = captura.texto();
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
    let cenario = Cenario::novo("contadores-resposta");
    let store = cenario.store();
    let org = criar_org(&store, "contadores-resposta");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "5521977770004");
    store.matrix().enable(org).expect("liga o módulo");
    let sala = "!contadores-sala:matrix-dev.local";
    store
        .matrix()
        .ensure_room(org, conversa, sala, None)
        .expect("grava a sala");
    store
        .matrix()
        .mark_room_ready(org, conversa)
        .expect("sala pronta");

    let metricas = metricas_novas();
    let ids = IdsFantasmas::da_config(&config()).expect("ids");
    let entrega = RespostaDoAtendente::com_tetos(
        Arc::new(PortaDoHarness::nova(Arc::clone(&store))),
        None,
        ids,
        16,
        16,
        Duration::from_secs(300),
    )
    .com_metricas(Arc::clone(&metricas));

    let texto = |evento: &str, sala: &str, corpo: Value| {
        json!({
            "type": "m.room.message",
            "room_id": sala,
            "event_id": evento,
            "sender": "@atendente:matrix-dev.local",
            "content": corpo,
        })
    };
    let humano = || json!({ "msgtype": "m.text", "body": "olá" });

    // 1) Resposta nova: enfileirada uma vez.
    entrega
        .entregar(
            "txn-1",
            vec![texto("$c-1:matrix-dev.local", sala, humano())],
        )
        .await
        .expect("entrega");
    assert_eq!(metricas.respostas_enfileiradas.value(), 1);

    // 2) Mesmo evento de novo (reenvio do Synapse): duplicada, nada novo enfileirado.
    entrega
        .entregar(
            "txn-2",
            vec![texto("$c-1:matrix-dev.local", sala, humano())],
        )
        .await
        .expect("entrega");
    assert_eq!(metricas.respostas_enfileiradas.value(), 1);
    assert_eq!(metricas.respostas_duplicadas.value(), 1);

    // 3) Sala que ninguém conhece.
    entrega
        .entregar(
            "txn-3",
            vec![texto(
                "$c-2:matrix-dev.local",
                "!desconhecida:matrix-dev.local",
                humano(),
            )],
        )
        .await
        .expect("entrega");
    assert_eq!(metricas.salas_desconhecidas.value(), 1);

    // 4) Mídia do atendente: só o aviso na sala.
    entrega
        .entregar(
            "txn-4",
            vec![texto(
                "$c-3:matrix-dev.local",
                sala,
                json!({ "msgtype": "m.image", "body": "foto.jpg" }),
            )],
        )
        .await
        .expect("entrega");
    assert_eq!(metricas.avisos_de_midia.value(), 1);
    assert_eq!(
        metricas.respostas_enfileiradas.value(),
        1,
        "mídia não vira resposta ao cliente"
    );

    // 5) Conversa encerrada: nada enviado, aviso na sala.
    let versao = store
        .conversations()
        .get(org, conversa)
        .expect("lê a conversa")
        .version;
    store
        .conversations()
        .close_audited(org, conversa, versao, "teste".to_owned())
        .expect("encerra a conversa");
    entrega
        .entregar(
            "txn-5",
            vec![texto("$c-4:matrix-dev.local", sala, humano())],
        )
        .await
        .expect("entrega");
    assert_eq!(metricas.respostas_conversa_encerrada.value(), 1);
    assert_eq!(
        metricas.respostas_enfileiradas.value(),
        1,
        "conversa encerrada não enfileira para o cliente"
    );
}

// ───────────────────────────── /metrics ─────────────────────────────

/// Sobe o servidor com o estado dado e devolve o texto de `/metrics`.
async fn metricas_http(estado: Estado) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = listener.local_addr().expect("endereço");
    let (desligar, sinal) = tokio::sync::oneshot::channel::<()>();
    let servidor = tokio::spawn(async move {
        let _ = poph_server::servir_em(listener, estado, async {
            let _ = sinal.await;
        })
        .await;
    });
    let cliente = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("cliente HTTP");
    let mut corpo = None;
    for _ in 0..100 {
        if let Ok(resposta) = cliente
            .get(format!("http://{endereco}/metrics"))
            .send()
            .await
        {
            corpo = Some(resposta.text().await.expect("corpo do /metrics"));
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let _ = desligar.send(());
    let _ = servidor.await;
    corpo.expect("o /metrics respondeu")
}

fn estado_com_modulo(rotulo: &str, ligado: bool) -> (Estado, PathBuf) {
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
    let estado = Estado::novo(store, cfg).expect("estado do servidor");
    (estado, pasta)
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
    let (estado, pasta) = estado_com_modulo("desligado", false);
    // Módulo desligado: nem quem prepara o módulo consegue declarar as métricas.
    assert!(
        estado.metricas_matrix().expect("sem erro").is_none(),
        "com [matrix] desligado, as métricas do módulo não são declaradas"
    );
    let texto = metricas_http(estado).await;
    let _ = fs::remove_dir_all(&pasta);
    assert!(
        texto.contains("poph_memoria_processo_bytes"),
        "sanidade: o /metrics responde"
    );
    assert!(
        !texto.contains("poph_matrix_"),
        "com o módulo desligado nenhuma poph_matrix_* pode aparecer:\n{texto}"
    );
}

#[tokio::test]
async fn metricas_do_modulo_aparecem_uma_vez_quando_preparado() {
    let (estado, pasta) = estado_com_modulo("ligado", true);
    // Preparar duas vezes (como a raiz faria em reinícios internos) não duplica nada na saída.
    let primeira = estado
        .metricas_matrix()
        .expect("prepara as métricas")
        .expect("módulo ligado");
    let segunda = estado
        .metricas_matrix()
        .expect("segunda chamada")
        .expect("módulo ligado");
    assert!(
        Arc::ptr_eq(&primeira, &segunda),
        "mesmo handle nas duas chamadas"
    );
    primeira.mensagens_espelhadas.increment(3);
    let texto = metricas_http(estado).await;
    let _ = fs::remove_dir_all(&pasta);
    for nome in NOMES_MATRIX {
        assert_eq!(
            texto.matches(&format!("# HELP {nome} ")).count(),
            1,
            "{nome} deve aparecer exatamente uma vez no /metrics"
        );
    }
    assert!(
        texto.contains("poph_matrix_mensagens_espelhadas_total 3\n"),
        "o valor incrementado aparece no /metrics"
    );
}

// ───────────────────────────── registro do processo ─────────────────────────────

#[test]
fn metricas_descartaveis_nao_aparecem_no_registro_do_processo() {
    // Handles descartáveis (usados quando ninguém injeta os números do processo) ficam fora do
    // `/metrics` do processo.
    let processo = Registry::new();
    let _ = MetricasMatrix::registrar(&processo).expect("registra no processo");
    let descartavel = MetricasMatrix::descartaveis();
    descartavel.erros_definitivos.increment(1);
    assert!(
        !processo
            .render()
            .contains("poph_matrix_erros_definitivos_total 1")
    );
}
