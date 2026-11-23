//! Download de mídia do Synapse contra um servidor **falso** (MX-44).
//!
//! O que só o servidor de verdade prova (o `?user_id=` valendo na rota de mídia, o conteúdo
//! chegando inteiro) está em `midia_synapse_real.rs`, `#[ignore]`. Aqui a prova é do que o cliente
//! controla, com um `axum` de teste que anota o caminho, a consulta e o cabeçalho de cada pedido:
//!
//! - o pedido vai na rota **nova** (`/_matrix/client/v1/media/download`), disfarçado de usuário e
//!   com `allow_redirect=false`;
//! - `404` na rota nova cai na antiga (servidor anterior), e o conteúdo volta;
//! - o teto de bytes vale pelo **tamanho declarado** e, sem ele, pela leitura em pedaços;
//! - **redirecionamento não é seguido**: `3xx` é falha definitiva, e o servidor falso registra um
//!   pedido só;
//! - `4xx` é definitivo, `5xx` é transitório e mídia vazia é recusa própria.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use poph_matrix::ErroMidia;
use poph_matrix::cliente::{ClienteMatrix, ConfigCliente};
use poph_matrix::erro::Classificacao;
use poph_types::Segredo;

/// Valor-canário: se aparecer em erro, log ou URL, o teste acusa.
const CANARIO: &str = "CANARIO-MATRIX-MX44-nao-pode-vazar";

const USUARIO: &str = "@ana:matrix-dev.local";

// ───────────────────────────── Synapse falso ─────────────────────────────

/// O que o servidor falso responde na rota de download.
#[derive(Clone, Debug)]
enum Resposta {
    /// Conteúdo de verdade, com o `Content-Type` que o servidor mandaria.
    Conteudo(Vec<u8>),
    /// Status de erro com o corpo JSON do Matrix.
    Erro(u16, &'static str),
    /// Redirecionamento para outro endereço.
    Redirecionar(&'static str),
    /// `200` sem `Content-Length`: exercita o teto na leitura em pedaços.
    SemTamanho(Vec<u8>),
}

#[derive(Clone, Debug)]
struct Registro {
    caminho: String,
    consulta: HashMap<String, String>,
    autorizacao_presente: bool,
}

#[derive(Clone, Default)]
struct Falso {
    /// Rota nova e antiga, cada uma com a sua resposta.
    novas: Arc<Mutex<Vec<Resposta>>>,
    antigas: Arc<Mutex<Vec<Resposta>>>,
    registros: Arc<Mutex<Vec<Registro>>>,
}

/// Consulta da URL como mapa (a mesma forma que o axum daria com o extrator `Query`).
///
/// O `query` é lido na mão porque a dependência de teste do `axum` aqui não habilita o extrator —
/// e uma dependência a mais no `Cargo.toml` só por causa do teste não se paga.
fn consulta_da(uri: &Uri) -> HashMap<String, String> {
    let mut mapa = HashMap::new();
    let Some(consulta) = uri.query() else {
        return mapa;
    };
    for par in consulta.split('&') {
        let (chave, valor) = par.split_once('=').unwrap_or((par, ""));
        mapa.insert(decodificar(chave), decodificar(valor));
    }
    mapa
}

/// Percent-decoding (`%40` → `@`), o bastante para conferir a consulta do pedido.
fn decodificar(texto: &str) -> String {
    let bytes = texto.as_bytes();
    let mut saida = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(valor) = u8::from_str_radix(&texto[i + 1..i + 3], 16)
        {
            saida.push(valor);
            i += 3;
            continue;
        }
        saida.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&saida).into_owned()
}

async fn atender(
    State(falso): State<Falso>,
    Path((_servidor, _id)): Path<(String, String)>,
    uri: Uri,
    cabecalhos: HeaderMap,
    nova: bool,
) -> Response {
    falso.registros.lock().unwrap().push(Registro {
        caminho: if nova {
            "nova".to_owned()
        } else {
            "antiga".to_owned()
        },
        consulta: consulta_da(&uri),
        autorizacao_presente: cabecalhos.contains_key("authorization"),
    });
    let fila = if nova {
        falso.novas.lock().unwrap().pop()
    } else {
        falso.antigas.lock().unwrap().pop()
    };
    match fila {
        Some(Resposta::Conteudo(bytes)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/octet-stream")],
            bytes,
        )
            .into_response(),
        Some(Resposta::SemTamanho(bytes)) => {
            // Corpo em fluxo: o servidor não declara o tamanho, e quem barra é a leitura em
            // pedaços do cliente (o caso de um `Content-Length` ausente).
            let metade = bytes.len() / 2;
            let partes: Vec<Result<axum::body::Bytes, std::io::Error>> = vec![
                Ok(axum::body::Bytes::from(bytes[..metade].to_vec())),
                Ok(axum::body::Bytes::from(bytes[metade..].to_vec())),
            ];
            Response::builder()
                .status(StatusCode::OK)
                .body(axum::body::Body::from_stream(futures::stream::iter(partes)))
                .expect("resposta em fluxo")
        }
        Some(Resposta::Erro(status, errcode)) => (
            StatusCode::from_u16(status).expect("status válido"),
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({ "errcode": errcode, "error": "recusado" }).to_string(),
        )
            .into_response(),
        Some(Resposta::Redirecionar(destino)) => Response::builder()
            .status(StatusCode::TEMPORARY_REDIRECT)
            .header(header::LOCATION, destino)
            .body(axum::body::Body::empty())
            .expect("redirecionamento"),
        None => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({ "errcode": "M_NOT_FOUND", "error": "Not found" }).to_string(),
        )
            .into_response(),
    }
}

async fn atender_novo(
    state: State<Falso>,
    path: Path<(String, String)>,
    uri: Uri,
    cabecalhos: HeaderMap,
) -> Response {
    atender(state, path, uri, cabecalhos, true).await
}

async fn atender_antigo(
    state: State<Falso>,
    path: Path<(String, String)>,
    uri: Uri,
    cabecalhos: HeaderMap,
) -> Response {
    atender(state, path, uri, cabecalhos, false).await
}

/// Sobe o Synapse falso e devolve o cliente apontado para ele.
async fn cenario(novas: Vec<Resposta>, antigas: Vec<Resposta>) -> (ClienteMatrix, Falso) {
    // A fila é consumida com `pop`, então o último entra primeiro.
    let falso = Falso::default();
    *falso.novas.lock().unwrap() = novas.into_iter().rev().collect();
    *falso.antigas.lock().unwrap() = antigas.into_iter().rev().collect();
    let roteador = Router::new()
        .route(
            "/_matrix/client/v1/media/download/{servidor}/{id}",
            get(atender_novo),
        )
        .route(
            "/_matrix/media/v3/download/{servidor}/{id}",
            get(atender_antigo),
        )
        .with_state(falso.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = listener.local_addr().expect("endereço");
    tokio::spawn(async move {
        let _ = axum::serve(listener, roteador).await;
    });
    let cliente = ClienteMatrix::novo(
        ConfigCliente::novo(
            format!("http://{endereco}"),
            "matrix-dev.local".to_owned(),
            Segredo::new(CANARIO.to_owned()),
        )
        .expect("configuração do cliente")
        .com_timeout_de_teste(),
    )
    .expect("cliente");
    (cliente, falso)
}

/// Atalho: a configuração com prazo curto, para o teste não esperar 10 s.
trait ComPrazo {
    fn com_timeout_de_teste(self) -> ConfigCliente;
}

impl ComPrazo for ConfigCliente {
    fn com_timeout_de_teste(mut self) -> ConfigCliente {
        self.timeout = Duration::from_secs(5);
        self
    }
}

fn registros(falso: &Falso) -> Vec<Registro> {
    falso.registros.lock().unwrap().clone()
}

// ───────────────────────────── testes ─────────────────────────────

#[tokio::test]
async fn baixa_pela_rota_nova_disfarcado_e_sem_redirecionar() {
    let (cliente, falso) = cenario(
        vec![Resposta::Conteudo(vec![0x89, b'P', b'N', b'G'])],
        vec![],
    )
    .await;

    let bytes = cliente
        .baixar_midia(USUARIO, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect("baixa");
    assert_eq!(bytes, vec![0x89, b'P', b'N', b'G']);

    let registros = registros(&falso);
    assert_eq!(registros.len(), 1, "uma chamada só: {registros:?}");
    assert_eq!(registros[0].caminho, "nova");
    assert!(registros[0].autorizacao_presente);
    assert_eq!(
        registros[0].consulta.get("user_id").map(String::as_str),
        Some(USUARIO),
        "disfarce de usuário na consulta"
    );
    assert_eq!(
        registros[0]
            .consulta
            .get("allow_redirect")
            .map(String::as_str),
        Some("false")
    );
}

#[tokio::test]
async fn rota_nova_ausente_cai_na_antiga_e_o_conteudo_volta() {
    let (cliente, falso) = cenario(
        vec![Resposta::Erro(404, "M_NOT_FOUND")],
        vec![Resposta::Conteudo(b"conteudo".to_vec())],
    )
    .await;

    let bytes = cliente
        .baixar_midia(USUARIO, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect("baixa pela rota antiga");
    assert_eq!(bytes, b"conteudo");
    let caminhos: Vec<String> = registros(&falso)
        .into_iter()
        .map(|registro| registro.caminho)
        .collect();
    assert_eq!(caminhos, vec!["nova".to_owned(), "antiga".to_owned()]);
}

#[tokio::test]
async fn midia_que_nao_existe_fica_com_a_resposta_da_rota_principal() {
    let (cliente, _falso) = cenario(
        vec![Resposta::Erro(404, "M_NOT_FOUND")],
        vec![Resposta::Erro(404, "M_NOT_FOUND")],
    )
    .await;

    let erro = cliente
        .baixar_midia(USUARIO, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect_err("não existe");
    assert!(!erro.transitorio(), "mídia apagada não volta: {erro}");
    assert_eq!(erro.classificacao(), Classificacao::Definitiva);
}

#[tokio::test]
async fn tamanho_declarado_acima_do_teto_recusa_sem_ler_o_conteudo() {
    let grande = vec![0u8; 4096];
    let (cliente, _falso) = cenario(vec![Resposta::Conteudo(grande)], vec![]).await;

    let erro = cliente
        .baixar_midia(USUARIO, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect_err("passa do teto");
    match erro {
        ErroMidia::Grande { bytes, teto } => {
            assert_eq!(teto, 1024);
            assert!(bytes > teto, "o tamanho declarado é o do arquivo: {bytes}");
        }
        outro => panic!("esperava recusa por tamanho, veio {outro:?}"),
    }
}

#[tokio::test]
async fn sem_tamanho_declarado_o_teto_vale_na_leitura_em_pedacos() {
    // Sem `Content-Length`: quem barra é a leitura do corpo.
    let grande = vec![7u8; 8192];
    let (cliente, _falso) = cenario(vec![Resposta::SemTamanho(grande)], vec![]).await;

    let erro = cliente
        .baixar_midia(USUARIO, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect_err("passa do teto na leitura");
    assert!(
        matches!(erro, ErroMidia::Grande { teto: 1024, .. }),
        "{erro:?}"
    );
}

#[tokio::test]
async fn redirecionamento_nao_e_seguido() {
    let (cliente, falso) = cenario(
        vec![Resposta::Redirecionar(
            "http://outro-servidor.invalid/arquivo",
        )],
        vec![],
    )
    .await;

    let erro = cliente
        .baixar_midia(USUARIO, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect_err("redirecionamento é recusado");
    assert!(!erro.transitorio(), "seguir não resolveria: {erro}");
    assert!(
        erro.to_string().contains("redirecionamento"),
        "o erro diz o que aconteceu: {erro}"
    );
    assert_eq!(
        registros(&falso).len(),
        1,
        "o endereço de destino não é visitado"
    );
}

#[tokio::test]
async fn erro_5xx_volta_no_proximo_ciclo_e_4xx_nao() {
    let (cliente, _falso) = cenario(vec![Resposta::Erro(503, "M_UNKNOWN")], vec![]).await;
    let erro = cliente
        .baixar_midia(USUARIO, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect_err("Synapse fora");
    assert!(erro.transitorio(), "{erro}");

    let (cliente, _falso) = cenario(vec![Resposta::Erro(403, "M_FORBIDDEN")], vec![]).await;
    let erro = cliente
        .baixar_midia(USUARIO, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect_err("sem permissão");
    assert!(!erro.transitorio(), "{erro}");
}

#[tokio::test]
async fn midia_vazia_e_endereco_invalido_sao_recusas_proprias() {
    let (cliente, _falso) = cenario(vec![Resposta::Conteudo(Vec::new())], vec![]).await;
    assert_eq!(
        cliente
            .baixar_midia(USUARIO, "mxc://matrix-dev.local/abc123", 1024)
            .await,
        Err(ErroMidia::Vazia)
    );

    // Endereço fora do contrato é recusado antes de qualquer pedido.
    let (cliente, falso) = cenario(vec![], vec![]).await;
    assert!(matches!(
        cliente
            .baixar_midia(USUARIO, "http://exemplo/arquivo", 1024)
            .await,
        Err(ErroMidia::Endereco(_))
    ));
    assert!(registros(&falso).is_empty(), "nada foi pedido ao servidor");
}

#[tokio::test]
async fn nenhum_erro_do_download_leva_o_token_para_a_mensagem() {
    let (cliente, _falso) = cenario(
        vec![Resposta::Erro(500, "M_UNKNOWN")],
        vec![Resposta::Erro(500, "M_UNKNOWN")],
    )
    .await;
    let erro = cliente
        .baixar_midia(USUARIO, "mxc://matrix-dev.local/abc123", 1024)
        .await
        .expect_err("falhou");
    let texto = format!("{erro} {erro:?}");
    assert!(!texto.contains(CANARIO), "token no erro: {texto}");
}
