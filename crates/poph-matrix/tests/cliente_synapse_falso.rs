//! Testes do cliente da API do Matrix contra um Synapse **falso** (MX-09).
//!
//! O servidor aqui é um `axum` de teste: grava o que chegou (método, caminho, consulta, corpo e
//! se o `Authorization` bateu com o token esperado) e responde o que o roteiro mandar. Nenhum
//! token é guardado no registro: só o booleano da comparação.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_matrix::cliente::{ClienteMatrix, ConfigCliente, CriarSala};
use poph_matrix::erro::{Classificacao, ErroChamada};
use poph_types::Segredo;
use serde_json::{Value, json};

/// Valor-canário: se aparecer em Debug, Display, log, erro ou URL, o teste acusa.
const CANARIO: &str = "CANARIO-MATRIX-MX09-nao-pode-vazar";

// ───────────────────────────── Synapse falso ─────────────────────────────

#[derive(Clone, Debug)]
struct Resposta {
    status: u16,
    corpo: String,
    atraso: Duration,
}

impl Resposta {
    fn ok(corpo: Value) -> Self {
        Self {
            status: 200,
            corpo: corpo.to_string(),
            atraso: Duration::ZERO,
        }
    }

    fn erro(status: u16, corpo: Value) -> Self {
        Self {
            status,
            corpo: corpo.to_string(),
            atraso: Duration::ZERO,
        }
    }

    fn cru(status: u16, corpo: impl Into<String>) -> Self {
        Self {
            status,
            corpo: corpo.into(),
            atraso: Duration::ZERO,
        }
    }

    fn com_atraso(mut self, atraso: Duration) -> Self {
        self.atraso = atraso;
        self
    }
}

#[derive(Clone, Debug, Default)]
struct Registro {
    metodo: String,
    caminho: String,
    consulta: Option<String>,
    autorizacao_presente: bool,
    autorizacao_bate: bool,
    corpo: String,
}

#[derive(Default)]
struct Estado {
    roteiro: VecDeque<Resposta>,
    padrao: Option<Resposta>,
    registros: Vec<Registro>,
    token_esperado: String,
}

async fn atender(
    State(estado): State<Arc<Mutex<Estado>>>,
    metodo: Method,
    uri: Uri,
    cabecalhos: HeaderMap,
    corpo: String,
) -> Response {
    let resposta = {
        let mut guarda = estado.lock().expect("trava do Synapse falso");
        let autorizacao = cabecalhos.get(AUTHORIZATION).and_then(|v| v.to_str().ok());
        let registro = Registro {
            metodo: metodo.to_string(),
            caminho: uri.path().to_owned(),
            consulta: uri.query().map(str::to_owned),
            autorizacao_presente: autorizacao.is_some(),
            autorizacao_bate: autorizacao
                .map(|v| v == format!("Bearer {}", guarda.token_esperado))
                .unwrap_or(false),
            corpo,
        };
        guarda.registros.push(registro);
        guarda
            .roteiro
            .pop_front()
            .or_else(|| guarda.padrao.clone())
            .unwrap_or_else(|| Resposta::ok(json!({})))
    };

    if !resposta.atraso.is_zero() {
        tokio::time::sleep(resposta.atraso).await;
    }
    let status = StatusCode::from_u16(resposta.status).expect("status válido");
    (
        status,
        [("content-type", "application/json")],
        resposta.corpo,
    )
        .into_response()
}

struct Cenario {
    estado: Arc<Mutex<Estado>>,
    cliente: ClienteMatrix,
}

impl Cenario {
    fn enfileirar(&self, resposta: Resposta) {
        self.estado
            .lock()
            .expect("trava")
            .roteiro
            .push_back(resposta);
    }

    fn registros(&self) -> Vec<Registro> {
        self.estado.lock().expect("trava").registros.clone()
    }

    fn registro(&self, indice: usize) -> Registro {
        self.registros()
            .get(indice)
            .cloned()
            .unwrap_or_else(|| panic!("não houve a requisição {indice}"))
    }
}

async fn cenario_com(token: &str, timeout: Duration) -> Cenario {
    let estado = Arc::new(Mutex::new(Estado {
        token_esperado: token.to_owned(),
        ..Default::default()
    }));
    let app = Router::new()
        .fallback(atender)
        .with_state(Arc::clone(&estado));
    let ouvinte = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = ouvinte.local_addr().expect("endereço local");
    tokio::spawn(async move {
        axum::serve(ouvinte, app)
            .await
            .expect("Synapse falso no ar");
    });
    let base = format!("http://{endereco}");
    let config = ConfigCliente::novo(base, "matrix-dev.local", Segredo::new(token.to_owned()))
        .expect("configuração do cliente")
        .com_timeout(timeout);
    let cliente = ClienteMatrix::novo(config).expect("cliente");
    Cenario { estado, cliente }
}

async fn cenario() -> Cenario {
    cenario_com(CANARIO, Duration::from_secs(5)).await
}

/// Consulta da requisição como texto (vazio quando não houve consulta).
fn consulta(registro: &Registro) -> String {
    registro.consulta.clone().unwrap_or_default()
}

// ───────────────────────────── salas ─────────────────────────────

#[tokio::test]
async fn criar_sala_disfarca_usuario_e_manda_token_so_no_cabecalho() {
    let c = cenario().await;
    c.enfileirar(Resposta::ok(json!({"room_id": "!sala1:matrix-dev.local"})));

    let sala = c
        .cliente
        .criar_sala(
            "@ph_as_bot:matrix-dev.local",
            &CriarSala {
                room_alias_name: Some("ph_c_1".to_owned()),
                name: Some("Cliente Teste".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("sala criada");
    assert_eq!(sala, "!sala1:matrix-dev.local");

    let r = c.registro(0);
    assert_eq!(r.metodo, "POST");
    assert_eq!(r.caminho, "/_matrix/client/v3/createRoom");
    assert!(consulta(&r).contains("user_id="), "sem disfarce: {r:?}");
    assert!(consulta(&r).contains("ph_as_bot"), "usuário errado: {r:?}");
    assert!(
        r.autorizacao_presente && r.autorizacao_bate,
        "token não foi no cabeçalho"
    );
    // O token nunca sai do cabeçalho: nem no caminho, nem na consulta, nem no corpo.
    for campo in [&r.caminho, &consulta(&r), &r.corpo] {
        assert!(
            !campo.contains(CANARIO),
            "token vazou na requisição: {campo}"
        );
    }
    let corpo: Value = serde_json::from_str(&r.corpo).expect("corpo JSON");
    assert_eq!(corpo["room_alias_name"], "ph_c_1");
    assert_eq!(corpo["name"], "Cliente Teste");
}

#[tokio::test]
async fn convidar_entrar_e_listar_membros() {
    let c = cenario().await;

    c.enfileirar(Resposta::ok(json!({})));
    c.cliente
        .convidar(
            "@ph_as_bot:matrix-dev.local",
            "!s:matrix-dev.local",
            "@ph_wa_1:matrix-dev.local",
        )
        .await
        .expect("convite");
    let convite = c.registro(0);
    assert_eq!(convite.metodo, "POST");
    assert_eq!(
        convite.caminho,
        "/_matrix/client/v3/rooms/!s:matrix-dev.local/invite"
    );
    assert!(consulta(&convite).contains("user_id="));
    let corpo: Value = serde_json::from_str(&convite.corpo).expect("corpo JSON");
    assert_eq!(corpo["user_id"], "@ph_wa_1:matrix-dev.local");

    c.enfileirar(Resposta::ok(json!({"room_id": "!s:matrix-dev.local"})));
    let entrou = c
        .cliente
        .entrar("@ph_wa_1:matrix-dev.local", "!s:matrix-dev.local")
        .await
        .expect("entrada");
    assert_eq!(entrou, "!s:matrix-dev.local");
    let entrada = c.registro(1);
    assert_eq!(
        entrada.caminho,
        "/_matrix/client/v3/join/!s:matrix-dev.local"
    );
    assert!(
        consulta(&entrada).contains("ph_wa_1"),
        "entrou como quem? {entrada:?}"
    );

    c.enfileirar(Resposta::ok(json!({
        "joined": {
            "@ph_as_bot:matrix-dev.local": {"display_name": "Bot"},
            "@ph_wa_1:matrix-dev.local": {"display_name": "Cliente (WhatsApp)"}
        }
    })));
    let membros = c
        .cliente
        .membros("@ph_as_bot:matrix-dev.local", "!s:matrix-dev.local")
        .await
        .expect("membros");
    assert_eq!(membros.len(), 2);
    assert_eq!(
        membros["@ph_wa_1:matrix-dev.local"].display_name.as_deref(),
        Some("Cliente (WhatsApp)")
    );
    assert_eq!(
        c.registro(2).caminho,
        "/_matrix/client/v3/rooms/!s:matrix-dev.local/joined_members"
    );
}

// ───────────────────────────── eventos ─────────────────────────────

#[tokio::test]
async fn evento_leva_txn_id_no_caminho_e_devolve_event_id() {
    let c = cenario().await;
    c.enfileirar(Resposta::ok(json!({"event_id": "$evento1"})));

    let evento = c
        .cliente
        .enviar_evento(
            "@ph_wa_1:matrix-dev.local",
            "!s:matrix-dev.local",
            "m.room.message",
            "txn-1",
            &json!({"msgtype": "m.text", "body": "olá"}),
        )
        .await
        .expect("envio");
    assert_eq!(evento, "$evento1");
    let r = c.registro(0);
    assert_eq!(r.metodo, "PUT");
    assert_eq!(
        r.caminho,
        "/_matrix/client/v3/rooms/!s:matrix-dev.local/send/m.room.message/txn-1"
    );
    let corpo: Value = serde_json::from_str(&r.corpo).expect("corpo JSON");
    assert_eq!(corpo["body"], "olá");
}

#[tokio::test]
async fn estado_escreve_com_chave_e_le_de_volta() {
    let c = cenario().await;

    c.enfileirar(Resposta::ok(json!({"event_id": "$estado1"})));
    let evento = c
        .cliente
        .enviar_estado(
            "@ph_as_bot:matrix-dev.local",
            "!s:matrix-dev.local",
            "ph.atendimento.metadata",
            None,
            &json!({"status": "active"}),
        )
        .await
        .expect("estado escrito");
    assert_eq!(evento, "$estado1");
    assert_eq!(
        c.registro(0).caminho,
        "/_matrix/client/v3/rooms/!s:matrix-dev.local/state/ph.atendimento.metadata"
    );

    c.enfileirar(Resposta::ok(json!({"status": "active"})));
    let valor = c
        .cliente
        .ler_estado(
            "@ph_as_bot:matrix-dev.local",
            "!s:matrix-dev.local",
            "ph.atendimento.metadata",
            Some(""),
        )
        .await
        .expect("estado lido");
    assert_eq!(valor["status"], "active");
    assert_eq!(c.registro(1).metodo, "GET");
}

#[tokio::test]
async fn espaco_filho_usa_a_sala_como_chave_e_o_server_name_como_via() {
    let c = cenario().await;
    c.enfileirar(Resposta::ok(json!({"event_id": "$filho"})));

    c.cliente
        .espaco_filho(
            "@ph_as_bot:matrix-dev.local",
            "!espaco:matrix-dev.local",
            "!sala:matrix-dev.local",
            Some("10"),
        )
        .await
        .expect("m.space.child");

    let r = c.registro(0);
    assert_eq!(
        r.caminho,
        "/_matrix/client/v3/rooms/!espaco:matrix-dev.local/state/m.space.child/!sala:matrix-dev.local"
    );
    let corpo: Value = serde_json::from_str(&r.corpo).expect("corpo JSON");
    assert_eq!(corpo["via"], json!(["matrix-dev.local"]));
    assert_eq!(corpo["order"], "10");
}

// ───────────────────────────── perfil e registro ─────────────────────────────

#[tokio::test]
async fn perfil_e_nome_de_exibicao() {
    let c = cenario().await;

    c.enfileirar(Resposta::ok(json!({"displayname": "Cliente (WhatsApp)"})));
    let perfil = c
        .cliente
        .perfil("@ph_wa_1:matrix-dev.local")
        .await
        .expect("perfil");
    assert_eq!(perfil.displayname.as_deref(), Some("Cliente (WhatsApp)"));
    let leitura = c.registro(0);
    assert_eq!(leitura.metodo, "GET");
    assert_eq!(
        leitura.caminho,
        "/_matrix/client/v3/profile/@ph_wa_1:matrix-dev.local"
    );
    assert!(
        leitura.consulta.is_none(),
        "leitura não precisa de disfarce"
    );

    c.enfileirar(Resposta::ok(json!({})));
    c.cliente
        .definir_nome_exibicao(
            "@ph_wa_1:matrix-dev.local",
            "@ph_wa_1:matrix-dev.local",
            "Cliente (WhatsApp)",
        )
        .await
        .expect("nome definido");
    let escrita = c.registro(1);
    assert_eq!(escrita.metodo, "PUT");
    assert_eq!(
        escrita.caminho,
        "/_matrix/client/v3/profile/@ph_wa_1:matrix-dev.local/displayname"
    );
    assert!(consulta(&escrita).contains("ph_wa_1"));
    let corpo: Value = serde_json::from_str(&escrita.corpo).expect("corpo JSON");
    assert_eq!(corpo["displayname"], "Cliente (WhatsApp)");
}

#[tokio::test]
async fn registro_de_usuario_virtual_usa_m_login_application_service() {
    let c = cenario().await;
    c.enfileirar(Resposta::ok(
        json!({"user_id": "@ph_wa_1:matrix-dev.local"}),
    ));

    let usuario = c
        .cliente
        .registrar_usuario("ph_wa_1")
        .await
        .expect("registro");
    assert_eq!(usuario, "@ph_wa_1:matrix-dev.local");

    let r = c.registro(0);
    assert_eq!(r.metodo, "POST");
    assert_eq!(r.caminho, "/_matrix/client/v3/register");
    assert!(consulta(&r).contains("kind=user"), "sem kind=user: {r:?}");
    assert!(
        r.consulta
            .as_deref()
            .unwrap_or_default()
            .contains("kind=user")
    );
    let corpo: Value = serde_json::from_str(&r.corpo).expect("corpo JSON");
    assert_eq!(corpo["type"], "m.login.application_service");
    assert_eq!(corpo["username"], "ph_wa_1");
    // Usuário virtual nasce sem token de acesso próprio (o módulo usa o disfarce do AS).
    assert_eq!(corpo["inhibit_login"], true);
}

// ───────────────────────────── erros ─────────────────────────────

#[tokio::test]
async fn respeita_limite_429_uma_vez_e_mede_a_espera() {
    let c = cenario().await;
    c.enfileirar(Resposta::erro(
        429,
        json!({
            "errcode": "M_LIMIT_EXCEEDED",
            "error": "Too Many Requests",
            "retry_after_ms": 150
        }),
    ));
    c.enfileirar(Resposta::ok(json!({"room_id": "!sala:matrix-dev.local"})));

    let inicio = Instant::now();
    let sala = c
        .cliente
        .criar_sala("@ph_as_bot:matrix-dev.local", &CriarSala::default())
        .await
        .expect("segunda tentativa deu certo");
    let decorrido = inicio.elapsed();

    assert_eq!(sala, "!sala:matrix-dev.local");
    assert!(
        decorrido >= Duration::from_millis(140),
        "não esperou o retry_after_ms: {decorrido:?}"
    );
    assert_eq!(c.registros().len(), 2, "deveria ter repetido uma vez");
}

#[tokio::test]
async fn limite_429_sem_espera_declarada_nao_repete() {
    let c = cenario().await;
    c.enfileirar(Resposta::erro(
        429,
        json!({"errcode": "M_LIMIT_EXCEEDED", "error": "Too Many Requests"}),
    ));

    let erro = c
        .cliente
        .criar_sala("@ph_as_bot:matrix-dev.local", &CriarSala::default())
        .await
        .expect_err("429 sem espera volta para o laço");
    assert!(erro.transitorio());
    assert_eq!(erro.retry_after_ms(), None);
    assert_eq!(c.registros().len(), 1);
}

#[tokio::test]
async fn limite_429_com_espera_longa_nao_trava_o_cliente() {
    let c = cenario().await;
    c.enfileirar(Resposta::erro(
        429,
        json!({
            "errcode": "M_LIMIT_EXCEEDED",
            "error": "Too Many Requests",
            "retry_after_ms": 120_000
        }),
    ));

    let inicio = Instant::now();
    let erro = c
        .cliente
        .criar_sala("@ph_as_bot:matrix-dev.local", &CriarSala::default())
        .await
        .expect_err("espera longa fica para o laço");
    assert!(
        inicio.elapsed() < Duration::from_secs(2),
        "o cliente esperou demais"
    );
    assert_eq!(erro.retry_after_ms(), Some(120_000));
    assert_eq!(c.registros().len(), 1);
}

#[tokio::test]
async fn erro_5xx_e_transitorio() {
    let c = cenario().await;
    c.enfileirar(Resposta::erro(
        503,
        json!({"errcode": "M_UNKNOWN", "error": "indisponível"}),
    ));

    let erro = c
        .cliente
        .criar_sala("@ph_as_bot:matrix-dev.local", &CriarSala::default())
        .await
        .expect_err("503 é falha");
    match &erro {
        ErroChamada::Resposta { status, .. } => assert_eq!(*status, 503),
        outro => panic!("esperado erro de resposta, veio {outro:?}"),
    }
    assert!(erro.transitorio());
    assert_eq!(c.registros().len(), 1, "5xx não repete dentro do cliente");
}

#[tokio::test]
async fn erro_4xx_e_definitivo_e_nao_vaza_token_nem_corpo_inteiro() {
    let c = cenario().await;
    let mensagem = "You are not allowed to invite this user ".repeat(20);
    c.enfileirar(Resposta::erro(
        403,
        json!({"errcode": "M_FORBIDDEN", "error": mensagem}),
    ));

    let erro = c
        .cliente
        .convidar(
            "@ph_as_bot:matrix-dev.local",
            "!s:matrix-dev.local",
            "@x:dev",
        )
        .await
        .expect_err("403 é definitivo");
    assert_eq!(erro.classificacao(), Classificacao::Definitiva);
    match &erro {
        ErroChamada::Resposta {
            status,
            errcode,
            mensagem,
            ..
        } => {
            assert_eq!(*status, 403);
            assert_eq!(errcode.as_deref(), Some("M_FORBIDDEN"));
            assert!(
                mensagem.chars().count() <= 161,
                "mensagem sem recorte: {mensagem}"
            );
        }
        outro => panic!("esperado erro de resposta, veio {outro:?}"),
    }
    assert!(!erro.to_string().contains(CANARIO));
}

#[tokio::test]
async fn prazo_devolve_erro_transitorio() {
    let c = cenario_com(CANARIO, Duration::from_millis(100)).await;
    c.enfileirar(Resposta::ok(json!({"room_id": "!s:dev"})).com_atraso(Duration::from_millis(600)));

    let erro = c
        .cliente
        .criar_sala("@ph_as_bot:matrix-dev.local", &CriarSala::default())
        .await
        .expect_err("deveria estourar o prazo");
    match erro {
        ErroChamada::Prazo { ms, .. } => assert_eq!(ms, 100),
        outro => panic!("esperado prazo, veio {outro:?}"),
    }
    assert!(erro.transitorio());
}

#[tokio::test]
async fn falha_de_rede_e_transitoria() {
    // Pega uma porta livre e fecha em seguida: a conexão é recusada.
    let ouvinte = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = ouvinte.local_addr().expect("endereço");
    drop(ouvinte);

    let config = ConfigCliente::novo(
        format!("http://{endereco}"),
        "matrix-dev.local",
        Segredo::new(CANARIO.to_owned()),
    )
    .expect("configuração")
    .com_timeout(Duration::from_secs(2));
    let cliente = ClienteMatrix::novo(config).expect("cliente");
    let erro = cliente
        .criar_sala("@ph_as_bot:matrix-dev.local", &CriarSala::default())
        .await
        .expect_err("sem servidor não há resposta");
    assert!(matches!(erro, ErroChamada::Rede { .. }), "{erro:?}");
    assert!(erro.transitorio());
    assert!(!erro.to_string().contains(CANARIO));
}

#[tokio::test]
async fn resposta_malformada_e_definitiva() {
    let c = cenario().await;
    c.enfileirar(Resposta::cru(200, "isto não é JSON"));
    let erro = c
        .cliente
        .criar_sala("@ph_as_bot:matrix-dev.local", &CriarSala::default())
        .await
        .expect_err("corpo que não é JSON");
    match erro {
        ErroChamada::RespostaInvalida { motivo, .. } => {
            assert!(motivo.contains("JSON"), "{motivo}")
        }
        outro => panic!("esperado resposta inválida, veio {outro:?}"),
    }

    // JSON válido, mas sem o campo que o contrato promete.
    c.enfileirar(Resposta::ok(json!({"nada": true})));
    let erro = c
        .cliente
        .criar_sala("@ph_as_bot:matrix-dev.local", &CriarSala::default())
        .await
        .expect_err("sem room_id");
    match erro {
        ErroChamada::RespostaInvalida { motivo, .. } => {
            assert!(motivo.contains("room_id"), "{motivo}")
        }
        outro => panic!("esperado resposta inválida, veio {outro:?}"),
    }
}

// ───────────────────────────── segredo ─────────────────────────────

#[tokio::test]
async fn token_canario_nunca_aparece_em_debug_display_log_ou_erro() {
    let c = cenario().await;

    let debug_cliente = format!("{:?}", c.cliente);
    assert!(
        debug_cliente.contains("[oculto]"),
        "Debug sem máscara: {debug_cliente}"
    );
    assert!(
        !debug_cliente.contains(CANARIO),
        "vazou no Debug: {debug_cliente}"
    );

    c.enfileirar(Resposta::erro(
        401,
        json!({"errcode": "M_UNKNOWN_TOKEN", "error": "Invalid access token"}),
    ));
    let erro = c
        .cliente
        .criar_sala("@ph_as_bot:matrix-dev.local", &CriarSala::default())
        .await
        .expect_err("401 é erro");
    assert!(
        !format!("{erro:?}").contains(CANARIO),
        "vazou no Debug do erro"
    );
    assert!(
        !erro.to_string().contains(CANARIO),
        "vazou no Display do erro"
    );

    // Log de verdade: assinante que grava tudo o que for emitido.
    let capturado = Arc::new(Mutex::new(Vec::<u8>::new()));
    let assinante = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer({
            let capturado = Arc::clone(&capturado);
            move || Escritor(Arc::clone(&capturado))
        })
        .finish();
    tracing::subscriber::with_default(assinante, || {
        tracing::info!(cliente = ?c.cliente, erro = %erro, "chamada ao Synapse falhou");
    });
    let log = String::from_utf8(capturado.lock().expect("trava").clone()).expect("utf-8");
    assert!(
        log.contains("chamada ao Synapse falhou"),
        "log não saiu: {log}"
    );
    assert!(!log.contains(CANARIO), "vazou no log: {log}");
}

/// Escritor de log em memória para o teste.
struct Escritor(Arc<Mutex<Vec<u8>>>);

impl Write for Escritor {
    fn write(&mut self, dados: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("trava").extend_from_slice(dados);
        Ok(dados.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
