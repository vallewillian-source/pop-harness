//! Testes do receptor do Application Service do Matrix (MX-16), com servidor e cliente HTTP de
//! verdade.
//!
//! O que se prova aqui, com o Synapse no papel de cliente:
//! - transação válida → `200` **depois** de a entrega receber os eventos;
//! - `401` sem token, `403` com token errado (e nada processado nos dois);
//! - corpo acima do teto e JSON inválido → `200` sem processar (nunca `4xx`, que derrubaria o AS);
//! - a mesma transação duas vezes → `200` e a entrega chamada uma vez só;
//! - falha da entrega (banco fora) → `5xx`, e a transação **não** fica marcada (o reenvio processa);
//! - módulo desligado → `404` nas quatro rotas, nunca a página do Console;
//! - as variantes sem `/v1` respondem igual à canônica;
//! - usuário e alias respondem pelo namespace da instância.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::{EntregaDeEventos, ErroEntrega, Estado, FuturoDaEntrega, ReceptorAs, TETO_CORPO};
use poph_store::Store;
use poph_types::config::GlobalTokenConfig;
use poph_types::{HarnessConfig, MatrixConfig, Sensitive};
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

/// Valor do token global da API administrativa (o servidor guarda só o hash).
const TOKEN_GLOBAL: &str = "pophg_teste-global-mx16";

/// Valor-canário do `hs_token`: não pode aparecer em resposta nem em log.
const HS_TOKEN: &str = "CANARIO-HS-TOKEN-mx16-9d2b-TAO-SECRETO";

/// Prefixo e domínio do módulo nos testes.
const PREFIXO: &str = "ph";
const SERVER_NAME: &str = "matrix-dev.local";

fn hash_argon2(valor: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(valor.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

/// O que a entrega recebeu (uma linha por chamada).
#[derive(Debug, Clone)]
struct Call {
    transacao: String,
    events: Vec<Value>,
}

/// Entrega de eventos de teste: anota o que chegou e, quando pedido, finge banco fora do ar.
#[derive(Default)]
struct Anotadora {
    chamadas: Mutex<Vec<Call>>,
    falhar: AtomicBool,
}

impl Anotadora {
    fn chamadas(&self) -> Vec<Call> {
        self.chamadas.lock().expect("trava").clone()
    }
}

impl EntregaDeEventos for Anotadora {
    fn entregar(&self, transacao: &str, events: Vec<Value>) -> FuturoDaEntrega<'_> {
        if self.falhar.load(Ordering::SeqCst) {
            return Box::pin(async { Err(ErroEntrega::new("banco indisponível")) });
        }
        self.chamadas.lock().expect("trava").push(Call {
            transacao: transacao.to_owned(),
            events,
        });
        Box::pin(async { Ok(()) })
    }
}

/// Servidor no ar, com banco e pasta próprios. O `Drop` desliga e apaga a pasta.
struct Cenario {
    base: String,
    client: Client,
    pasta: PathBuf,
    entrega: Arc<Anotadora>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Cenario {
    /// Servidor com o módulo Matrix ligado (receptor injetado).
    async fn ligado(rotulo: &str) -> Self {
        let entrega = Arc::new(Anotadora::default());
        let receptor = ReceptorAs::new(
            PREFIXO,
            SERVER_NAME,
            Sensitive::new(HS_TOKEN.to_owned()),
            Arc::clone(&entrega) as Arc<dyn EntregaDeEventos>,
        );
        Self::montar(rotulo, true, Some(receptor), entrega).await
    }

    /// Servidor com o módulo desligado (nenhum receptor injetado).
    async fn desligado(rotulo: &str) -> Self {
        Self::montar(rotulo, false, None, Arc::new(Anotadora::default())).await
    }

    async fn montar(
        rotulo: &str,
        matrix_ligado: bool,
        receptor: Option<ReceptorAs>,
        entrega: Arc<Anotadora>,
    ) -> Self {
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duracao| duracao.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-matrix-as-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.matrix = Some(MatrixConfig {
            enabled: matrix_ligado,
            homeserver_url: "http://127.0.0.1:18008".to_owned(),
            server_name: SERVER_NAME.to_owned(),
            prefix: PREFIXO.to_owned(),
            ..Default::default()
        });
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let mut state = Estado::novo(store, config).expect("estado do servidor");
        if let Some(receptor) = receptor {
            state = state.com_matrix_as(Arc::new(receptor));
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, sinal) = oneshot::channel();
        tokio::spawn(async move {
            let _ = poph_server::servir_em(listener, state, async {
                let _ = sinal.await;
            })
            .await;
        });

        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("cliente HTTP");
        let cenario = Self {
            base: format!("http://{endereco}"),
            client,
            pasta,
            entrega,
            desligar: Some(desligar),
        };
        cenario.esperar_subir().await;
        cenario
    }

    async fn esperar_subir(&self) {
        let inicio = Instant::now();
        loop {
            match self
                .client
                .get(format!("{}/healthz", self.base))
                .send()
                .await
            {
                Ok(resposta) if resposta.status().is_success() => return,
                _ if inicio.elapsed() < Duration::from_secs(10) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                outro => panic!("servidor não subiu: {outro:?}"),
            }
        }
    }

    fn url(&self, caminho: &str) -> String {
        format!("{}{caminho}", self.base)
    }

    /// `PUT`/`GET` com o `Authorization` que o Synapse manda (`Bearer <hs_token>`).
    async fn pedir(
        &self,
        metodo: Method,
        caminho: &str,
        token: Option<&str>,
    ) -> (StatusCode, String) {
        let mut pedido = self.client.request(metodo, self.url(caminho));
        if let Some(token) = token {
            pedido = pedido.bearer_auth(token);
        }
        let resposta = pedido.send().await.expect("pedido HTTP");
        let status = resposta.status();
        (status, resposta.text().await.unwrap_or_default())
    }

    /// `PUT` de transação com o corpo dado (`Content-Length` normal).
    async fn transacao(
        &self,
        caminho: &str,
        token: Option<&str>,
        body: String,
    ) -> (StatusCode, String) {
        let mut pedido = self
            .client
            .put(self.url(caminho))
            .header("content-type", "application/json")
            .body(body);
        if let Some(token) = token {
            pedido = pedido.bearer_auth(token);
        }
        let resposta = pedido.send().await.expect("pedido HTTP");
        let status = resposta.status();
        (status, resposta.text().await.unwrap_or_default())
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

/// Corpo de uma transação do Synapse com um evento de mensagem.
fn corpo_da_transacao(event: &str) -> String {
    json!({
        "events": [
            {
                "type": "m.room.message",
                "room_id": "!sala:matrix-dev.local",
                "sender": "@atendente:matrix-dev.local",
                "event_id": event,
                "content": { "msgtype": "m.text", "body": "oi" }
            }
        ]
    })
    .to_string()
}

const ROTA: &str = "/_matrix/app/v1/transactions/1";

// ───────────────────────────── caminho feliz ─────────────────────────────

#[tokio::test]
async fn transacao_valida_responde_200_depois_de_entregar_os_eventos() {
    let c = Cenario::ligado("valida").await;
    let (status, body) = c
        .transacao(ROTA, Some(HS_TOKEN), corpo_da_transacao("$a"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.trim(), "{}", "o Synapse espera corpo vazio de objeto");

    let chamadas = c.entrega.chamadas();
    assert_eq!(chamadas.len(), 1, "a entrega tinha de ser chamada uma vez");
    assert_eq!(chamadas[0].transacao, "1");
    assert_eq!(chamadas[0].events.len(), 1);
    assert_eq!(chamadas[0].events[0]["event_id"], json!("$a"));
}

#[tokio::test]
async fn variantes_sem_v1_atendem_igual_a_canonica() {
    let c = Cenario::ligado("variantes").await;
    for (indice, caminho) in ["/_matrix/app/transactions/v1menos", "/transactions/raiz"]
        .into_iter()
        .enumerate()
    {
        let (status, _) = c
            .transacao(
                caminho,
                Some(HS_TOKEN),
                corpo_da_transacao(&format!("$v{indice}")),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "rota `{caminho}`");
    }
    assert_eq!(c.entrega.chamadas().len(), 2);
}

#[tokio::test]
async fn rotas_com_prefixo_matrix_atendem_igual_as_sem_prefixo() {
    let c = Cenario::ligado("prefixo").await;

    // O registro do Synapse de desenvolvimento aponta para uma base com `/matrix` e o Synapse
    // chama `<url>/_matrix/app/v1/…`: sem esta rota a entrega respondia 404 (MX-31, item 3).
    let (status, body) = c
        .transacao(
            "/matrix/_matrix/app/v1/transactions/9",
            Some(HS_TOKEN),
            corpo_da_transacao("$p1"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "transação pela rota com prefixo");
    assert_eq!(body.trim(), "{}");
    assert_eq!(c.entrega.chamadas().len(), 1);
    assert_eq!(c.entrega.chamadas()[0].transacao, "9");

    // A idempotência é a mesma das duas famílias: repetir o mesmo `txnId` pela rota sem prefixo não
    // entrega de novo.
    let (status, _) = c
        .transacao(
            "/_matrix/app/v1/transactions/9",
            Some(HS_TOKEN),
            corpo_da_transacao("$p1"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        c.entrega.chamadas().len(),
        1,
        "o `txnId` já processado não é entregue de novo por outro caminho"
    );

    // A variante sem `/v1` também atende com o prefixo.
    let (status, _) = c
        .transacao(
            "/matrix/_matrix/app/transactions/10",
            Some(HS_TOKEN),
            corpo_da_transacao("$p2"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "rota com prefixo e sem /v1");
    assert_eq!(c.entrega.chamadas().len(), 2);

    // Autenticação igual: sem token é 401 nas duas famílias, e o `hs_token` nunca aparece.
    for caminho in [
        "/matrix/_matrix/app/v1/transactions/11",
        "/_matrix/app/v1/transactions/11",
    ] {
        let (status, body) = c.transacao(caminho, None, corpo_da_transacao("$p3")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "rota `{caminho}`");
        assert!(!body.contains(HS_TOKEN), "vazou token: {body}");
    }

    // As consultas também respondem pelo caminho com prefixo.
    let (status, _) = c
        .pedir(Method::POST, "/matrix/_matrix/app/v1/ping", Some(HS_TOKEN))
        .await;
    assert_eq!(status, StatusCode::OK, "ping com prefixo");
    let (status, _) = c
        .pedir(
            Method::GET,
            "/matrix/_matrix/app/v1/users/@ph_wa_5511999999999:matrix-dev.local",
            Some(HS_TOKEN),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "consulta de usuário com prefixo");
}

#[tokio::test]
async fn ping_usuarios_e_alias_respondem_pelo_namespace() {
    let c = Cenario::ligado("consultas").await;

    let (status, body) = c
        .pedir(Method::POST, "/_matrix/app/v1/ping", Some(HS_TOKEN))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.trim(), "{}");

    let (status, _) = c
        .pedir(
            Method::GET,
            "/_matrix/app/v1/users/@ph_wa_5511999999999:matrix-dev.local",
            Some(HS_TOKEN),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "fantasma do prefixo é da instância");

    let (status, body) = c
        .pedir(
            Method::GET,
            "/_matrix/app/v1/users/@wa_5511999999999:matrix-dev.local",
            Some(HS_TOKEN),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "prefixo de outro registro");
    assert!(
        body.contains("M_NOT_FOUND"),
        "o 404 leva o errcode do Matrix: {body}"
    );

    let (status, _) = c
        .pedir(
            Method::GET,
            "/_matrix/app/v1/rooms/%23ph_c_0123456789abcdef:matrix-dev.local",
            Some(HS_TOKEN),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "alias do módulo");

    let (status, _) = c
        .pedir(
            Method::GET,
            "/_matrix/app/v1/rooms/%23dep_vendas:matrix-dev.local",
            Some(HS_TOKEN),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "alias #dep_ não é exclusivo");
}

// ───────────────────────────── autenticação ─────────────────────────────

#[tokio::test]
async fn sem_token_da_401_e_token_errado_da_403_sem_processar() {
    let c = Cenario::ligado("auth").await;

    let (status, body) = c.transacao(ROTA, None, corpo_da_transacao("$a")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!body.contains(HS_TOKEN), "vazou token na resposta: {body}");

    let (status, body) = c
        .transacao(ROTA, Some("token-errado"), corpo_da_transacao("$a"))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!body.contains(HS_TOKEN), "vazou token na resposta: {body}");

    // As consultas também exigem o token.
    let (status, _) = c
        .pedir(
            Method::GET,
            "/_matrix/app/v1/users/@ph_wa_1:matrix-dev.local",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    assert!(c.entrega.chamadas().is_empty(), "nada podia ser processado");
}

#[tokio::test]
async fn token_canario_nao_aparece_na_resposta() {
    let c = Cenario::ligado("canario").await;
    let mut respostas = Vec::new();
    for (metodo, caminho, token) in [
        (Method::PUT, ROTA, Some(HS_TOKEN)),
        (Method::PUT, ROTA, Some("errado")),
        (Method::PUT, ROTA, None),
        (
            Method::GET,
            "/_matrix/app/v1/users/@ph_wa_1:matrix-dev.local",
            Some("errado"),
        ),
        (Method::POST, "/_matrix/app/v1/ping", Some("errado")),
    ] {
        let (_, body) = match metodo {
            Method::PUT => c.transacao(caminho, token, corpo_da_transacao("$a")).await,
            _ => c.pedir(metodo, caminho, token).await,
        };
        respostas.push(body);
    }
    for body in respostas {
        assert!(!body.contains(HS_TOKEN), "o hs_token vazou: {body}");
    }
}

// ───────────────────────────── corpo e idempotência ─────────────────────────────

/// Transação **válida** (JSON certo com um evento) e maior que o teto: só o tamanho a recusa.
fn corpo_valido_acima_do_teto() -> String {
    let enchimento = "g".repeat(TETO_CORPO);
    json!({
        "events": [
            {
                "type": "m.room.message",
                "room_id": "!sala:matrix-dev.local",
                "sender": "@atendente:matrix-dev.local",
                "event_id": "$grande",
                "content": { "msgtype": "m.text", "body": enchimento }
            }
        ]
    })
    .to_string()
}

#[tokio::test]
async fn corpo_gigante_responde_200_sem_processar() {
    let c = Cenario::ligado("gigante").await;
    let enorme = corpo_valido_acima_do_teto();
    assert!(
        enorme.len() > TETO_CORPO,
        "o corpo do teste precisa passar do teto"
    );
    let (status, _) = c.transacao(ROTA, Some(HS_TOKEN), enorme).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "corpo grande não pode dar 4xx: o Synapse marcaria o AS como fora do ar"
    );
    assert!(c.entrega.chamadas().is_empty(), "nada podia ser processado");
}

#[tokio::test]
async fn corpo_grande_sem_content_length_responde_200_sem_processar() {
    let c = Cenario::ligado("gigante-stream").await;
    // Corpo em fluxo (`chunked`): o teto não pode depender do `Content-Length`.
    // JSON válido (passa do teto só pelo tamanho), em dois pedaços.
    let body = corpo_valido_acima_do_teto().into_bytes();
    let meio = body.len() / 2;
    let pedacos = vec![
        Ok::<_, std::io::Error>(axum::body::Bytes::from(body[..meio].to_vec())),
        Ok(axum::body::Bytes::from(body[meio..].to_vec())),
    ];
    let resposta = c
        .client
        .put(c.url(ROTA))
        .bearer_auth(HS_TOKEN)
        .body(reqwest::Body::wrap_stream(futures::stream::iter(pedacos)))
        .send()
        .await
        .expect("pedido HTTP");
    assert_eq!(resposta.status(), StatusCode::OK);
    assert!(c.entrega.chamadas().is_empty(), "nada podia ser processado");
}

#[tokio::test]
async fn json_invalido_responde_200_sem_processar() {
    let c = Cenario::ligado("json").await;
    for body in ["isto não é json".to_owned(), "{}".to_owned(), String::new()] {
        let (status, _) = c.transacao(ROTA, Some(HS_TOKEN), body).await;
        assert_eq!(status, StatusCode::OK);
    }
    assert!(c.entrega.chamadas().is_empty());
}

#[tokio::test]
async fn retransmissao_da_mesma_transacao_processa_uma_vez() {
    let c = Cenario::ligado("repetida").await;
    for _ in 0..3 {
        let (status, _) = c
            .transacao(ROTA, Some(HS_TOKEN), corpo_da_transacao("$a"))
            .await;
        assert_eq!(status, StatusCode::OK);
    }
    let chamadas = c.entrega.chamadas();
    assert_eq!(
        chamadas.len(),
        1,
        "o Synapse reenviou a mesma transação: tinha de processar uma vez"
    );

    // Transação nova processa normalmente.
    let (status, _) = c
        .transacao(
            "/_matrix/app/v1/transactions/2",
            Some(HS_TOKEN),
            corpo_da_transacao("$b"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(c.entrega.chamadas().len(), 2);
}

#[tokio::test]
async fn falha_de_entrega_da_5xx_e_o_reenvio_processa() {
    let c = Cenario::ligado("falha").await;

    c.entrega.falhar.store(true, Ordering::SeqCst);
    let (status, body) = c
        .transacao(ROTA, Some(HS_TOKEN), corpo_da_transacao("$a"))
        .await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "erro transitório nosso tem de ser 5xx para o Synapse reenviar"
    );
    assert!(
        status.is_server_error(),
        "um 4xx aqui bloquearia o AS por minutos"
    );
    assert!(!body.contains(HS_TOKEN));

    c.entrega.falhar.store(false, Ordering::SeqCst);
    let (status, _) = c
        .transacao(ROTA, Some(HS_TOKEN), corpo_da_transacao("$a"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        c.entrega.chamadas().len(),
        1,
        "o reenvio da transação que falhou tem de ser processado"
    );
}

// ───────────────────────────── módulo desligado ─────────────────────────────

#[tokio::test]
async fn modulo_desligado_da_404_nas_quatro_rotas() {
    let c = Cenario::desligado("desligado").await;

    let rotas = [
        (Method::PUT, "/_matrix/app/v1/transactions/1"),
        (
            Method::GET,
            "/_matrix/app/v1/users/@ph_wa_1:matrix-dev.local",
        ),
        (
            Method::GET,
            "/_matrix/app/v1/rooms/%23ph_c_1:matrix-dev.local",
        ),
        (Method::POST, "/_matrix/app/v1/ping"),
        // As variantes também não existem.
        (Method::PUT, "/_matrix/app/transactions/1"),
        (Method::PUT, "/transactions/raiz"),
    ];
    for (metodo, caminho) in rotas {
        let (status, body) = match metodo {
            Method::PUT => {
                c.transacao(caminho, Some(HS_TOKEN), corpo_da_transacao("$a"))
                    .await
            }
            _ => c.pedir(metodo.clone(), caminho, Some(HS_TOKEN)).await,
        };
        assert_eq!(status, StatusCode::NOT_FOUND, "rota `{caminho}`");
        if metodo == Method::GET {
            assert!(
                body.contains("not_found"),
                "o GET do módulo desligado não pode cair na página do Console: {body}"
            );
        }
    }

    // Caminho do Matrix que não é rota nenhuma: 404 do servidor, nunca o `index.html`.
    let (status, body) = c
        .pedir(Method::GET, "/_matrix/inexistente", Some(HS_TOKEN))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!body.contains("<html"), "veio a página do Console: {body}");

    assert!(c.entrega.chamadas().is_empty());
}
