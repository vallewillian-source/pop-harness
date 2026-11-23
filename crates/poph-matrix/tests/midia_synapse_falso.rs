//! Subida de mídia contra um Synapse **falso** (MX-43).
//!
//! O que só aparece falando HTTP de verdade: a rota do `upload`, o disfarce `?user_id=`, o
//! `?filename=`, o `Content-Type` do arquivo, o corpo binário chegando inteiro e a rota nova sendo
//! tentada quando o servidor não reconhece a antiga. O evento é conferido pelo mesmo caminho: o
//! `PUT …/send/m.room.message/{txnId}` com o bloco `com.<marca>.attachment` e o `mxc://` que a
//! subida devolveu.
//!
//! O Synapse falso deduplica por `txnId`, como o de verdade (MX-02): repetir a mesma mídia devolve
//! o mesmo evento em vez de criar outro.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_matrix::cliente::{ClienteMatrix, ConfigCliente};
use poph_matrix::midia::{self, AnexoParaMidia, BLOCO_ANEXO, MSGTYPE_ARQUIVO, MSGTYPE_IMAGEM};
use poph_types::Segredo;
use serde_json::{Value, json};

/// Valor-canário do token: não pode aparecer em evento, em log nem em erro.
const CANARIO: &str = "CANARIO-MX43-nao-pode-vazar";

/// Domínio dos ids no teste.
const SERVIDOR: &str = "matrix-dev.local";

/// Um upload recebido pelo Synapse falso.
#[derive(Clone, Debug)]
struct Upload {
    caminho: String,
    consulta: String,
    mime: String,
    bytes: Vec<u8>,
    autorizacao: String,
}

/// Um evento recebido pelo Synapse falso.
#[derive(Clone, Debug)]
struct Evento {
    sala: String,
    tipo: String,
    txn: String,
    consulta: String,
    corpo: Value,
}

#[derive(Default)]
struct Estado {
    uploads: Mutex<Vec<Upload>>,
    /// Eventos por `(sala, txn)`: o mesmo par não cria outro evento.
    eventos: Mutex<BTreeMap<(String, String), Evento>>,
    /// Quando `true`, a rota antiga de upload responde `404 M_UNRECOGNIZED`.
    rota_antiga_fora: Mutex<bool>,
    /// Quando `true`, o `send` responde `403` (recusa definitiva).
    recusar_envio: Mutex<bool>,
}

fn responder(status: u16, corpo: Value) -> Response {
    (
        StatusCode::from_u16(status).expect("status válido"),
        [("content-type", "application/json")],
        corpo.to_string(),
    )
        .into_response()
}

/// Percent-decoding de um segmento (o `@` do `user_id` chega como `%40`).
fn decodificar(segmento: &str) -> String {
    let bytes = segmento.as_bytes();
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
    State(estado): State<Arc<Estado>>,
    metodo: Method,
    uri: Uri,
    cabecalhos: HeaderMap,
    corpo: Bytes,
) -> Response {
    let autorizacao = cabecalhos
        .get(AUTHORIZATION)
        .and_then(|valor| valor.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if autorizacao != format!("Bearer {CANARIO}") {
        return responder(
            401,
            json!({ "errcode": "M_UNKNOWN_TOKEN", "error": "Token inválido" }),
        );
    }
    let consulta = uri.query().unwrap_or_default().to_owned();
    let mime = cabecalhos
        .get(CONTENT_TYPE)
        .and_then(|valor| valor.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let caminho = uri.path().to_owned();

    // Upload de mídia (as duas rotas conhecidas).
    // As duas rotas conhecidas de upload: `/_matrix/media/v3/upload` e
    // `/_matrix/client/v1/media/upload`.
    if metodo == Method::POST && caminho.ends_with("/upload") && caminho.contains("/media/") {
        if caminho.starts_with("/_matrix/media/v3") && *estado.rota_antiga_fora.lock().unwrap() {
            return responder(
                404,
                json!({ "errcode": "M_UNRECOGNIZED", "error": "Unrecognized request" }),
            );
        }
        let n = {
            let mut uploads = estado.uploads.lock().unwrap();
            uploads.push(Upload {
                caminho: caminho.clone(),
                consulta: consulta.clone(),
                mime,
                bytes: corpo.to_vec(),
                autorizacao,
            });
            uploads.len()
        };
        return responder(
            200,
            json!({ "content_uri": format!("mxc://{SERVIDOR}/midia{n}") }),
        );
    }

    // Envio de evento: `PUT /_matrix/client/v3/rooms/{sala}/send/{tipo}/{txn}`.
    let partes: Vec<String> = caminho
        .split('/')
        .filter(|pedaco| !pedaco.is_empty())
        .map(decodificar)
        .collect();
    if metodo == Method::PUT
        && let Some(posicao) = partes.iter().position(|parte| parte == "send")
        && partes.len() == posicao + 3
    {
        if *estado.recusar_envio.lock().unwrap() {
            return responder(
                403,
                json!({ "errcode": "M_FORBIDDEN", "error": "recusado" }),
            );
        }
        let sala = partes[posicao - 1].clone();
        let tipo = partes[posicao + 1].clone();
        let txn = partes[posicao + 2].clone();
        let corpo: Value = serde_json::from_slice(&corpo).unwrap_or(Value::Null);
        let mut eventos = estado.eventos.lock().unwrap();
        let evento = eventos
            .entry((sala.clone(), txn.clone()))
            .or_insert_with(|| Evento {
                sala,
                tipo,
                txn,
                consulta,
                corpo,
            })
            .clone();
        return responder(200, json!({ "event_id": format!("${}", evento.txn) }));
    }

    responder(404, json!({ "errcode": "M_UNRECOGNIZED", "error": "?" }))
}

/// Sobe o Synapse falso numa porta livre e devolve a URL base com a tarefa que o serve.
async fn subir(estado: Arc<Estado>) -> (String, tokio::task::JoinHandle<()>) {
    let app = Router::new().fallback(atender).with_state(estado);
    let ouvinte = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta livre");
    let endereco = ouvinte.local_addr().expect("endereço").to_string();
    let tarefa = tokio::spawn(async move {
        let _ = axum::serve(ouvinte, app).await;
    });
    (format!("http://{endereco}"), tarefa)
}

fn cliente(base: &str) -> ClienteMatrix {
    ClienteMatrix::novo(
        ConfigCliente::novo(base, SERVIDOR, Segredo::new(CANARIO.to_owned()))
            .expect("configuração"),
    )
    .expect("cliente")
}

fn fantasma() -> poph_matrix::usuarios::Fantasma {
    poph_matrix::usuarios::IdsFantasmas::novo("ph", SERVIDOR)
        .expect("ids")
        .cliente("5521988887777")
        .expect("fantasma do cliente")
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nconteudo-de-imagem";
const PDF: &[u8] = b"%PDF-1.4\nlaudo\n%%EOF\n";

fn midia_de(nome: &str, mime: &str, bytes: &[u8]) -> midia::Midia {
    let tamanho = bytes.len() as u64;
    midia::decidir(&AnexoParaMidia {
        id: &format!("anx-{nome}"),
        nome,
        mime,
        tamanho_bytes: tamanho,
        fonte: "whatsapp",
    })
    .expect("decide a mídia")
}

// ───────────────────────────── testes ─────────────────────────────

#[tokio::test]
async fn imagem_e_pdf_sobem_pelo_fantasma_do_cliente_e_viram_evento_com_bloco() {
    let estado = Arc::new(Estado::default());
    let (base, tarefa) = subir(Arc::clone(&estado)).await;
    let cliente = cliente(&base);
    let remetente = fantasma();
    let sala = "!sala1:matrix-dev.local";

    let imagem = midia_de("foto.jpg", "image/jpeg", PNG);
    let evento_imagem = midia::enviar(&cliente, &remetente, sala, &imagem, PNG)
        .await
        .expect("imagem sobe e vira evento");

    let pdf = midia_de("laudo.pdf", "application/pdf", PDF);
    let evento_pdf = midia::enviar(&cliente, &remetente, sala, &pdf, PDF)
        .await
        .expect("pdf sobe e vira evento");
    assert_ne!(evento_imagem, evento_pdf, "dois eventos diferentes");

    // O upload: rota antiga (a que o Synapse de dev atende), disfarce do fantasma, nome do arquivo
    // e o corpo binário inteiro com o tipo do arquivo.
    let uploads = estado.uploads.lock().unwrap().clone();
    assert_eq!(uploads.len(), 2);
    assert_eq!(uploads[0].caminho, "/_matrix/media/v3/upload");
    assert_eq!(
        uploads[0].consulta,
        "filename=foto.jpg&user_id=%40ph_wa_5521988887777%3Amatrix-dev.local"
    );
    assert_eq!(uploads[0].mime, "image/jpeg");
    assert_eq!(uploads[0].bytes, PNG);
    assert_eq!(uploads[1].mime, "application/pdf");
    assert_eq!(uploads[1].bytes, PDF);
    // O token vai só no cabeçalho, e o conteúdo do arquivo não vaza para lugar nenhum.
    assert!(
        uploads[0].autorizacao.starts_with("Bearer "),
        "o token vai no `Authorization`"
    );

    // O evento: `m.image` e `m.file`, com o bloco e o `mxc://` da subida.
    let eventos = estado.eventos.lock().unwrap().clone();
    let imagem_na_sala = eventos
        .get(&(sala.to_owned(), midia::txn_da_midia("anx-foto.jpg")))
        .expect("evento da imagem")
        .clone();
    assert_eq!(imagem_na_sala.tipo, "m.room.message");
    assert_eq!(imagem_na_sala.sala, sala);
    assert!(
        imagem_na_sala.consulta.contains("user_id="),
        "o evento sai com o disfarce do fantasma"
    );
    assert_eq!(imagem_na_sala.corpo["msgtype"], json!(MSGTYPE_IMAGEM));
    assert_eq!(imagem_na_sala.corpo["body"], json!("foto.jpg"));
    assert_eq!(
        imagem_na_sala.corpo["url"],
        json!("mxc://matrix-dev.local/midia1")
    );
    assert_eq!(
        imagem_na_sala.corpo[BLOCO_ANEXO],
        json!({ "id": "anx-foto.jpg", "source": "whatsapp" })
    );

    let pdf_na_sala = eventos
        .get(&(sala.to_owned(), midia::txn_da_midia("anx-laudo.pdf")))
        .expect("evento do pdf")
        .clone();
    assert_eq!(pdf_na_sala.corpo["msgtype"], json!(MSGTYPE_ARQUIVO));
    assert_eq!(
        pdf_na_sala.corpo["url"],
        json!("mxc://matrix-dev.local/midia2")
    );
    assert_eq!(pdf_na_sala.corpo[BLOCO_ANEXO]["id"], json!("anx-laudo.pdf"));

    assert!(
        !format!("{eventos:?}").contains(CANARIO),
        "token nunca no evento"
    );
    tarefa.abort();
}

#[tokio::test]
async fn repetir_a_mesma_midia_nao_cria_segundo_evento() {
    let estado = Arc::new(Estado::default());
    let (base, tarefa) = subir(Arc::clone(&estado)).await;
    let cliente = cliente(&base);
    let remetente = fantasma();
    let sala = "!sala1:matrix-dev.local";
    let midia = midia_de("foto.png", "image/png", PNG);

    let primeiro = midia::enviar(&cliente, &remetente, sala, &midia, PNG)
        .await
        .expect("primeira");
    let segundo = midia::enviar(&cliente, &remetente, sala, &midia, PNG)
        .await
        .expect("repetição");
    assert_eq!(primeiro, segundo, "o mesmo `txnId` devolve o mesmo evento");
    assert_eq!(estado.eventos.lock().unwrap().len(), 1);
    tarefa.abort();
}

#[tokio::test]
async fn rota_antiga_fora_cai_na_rota_nova_de_upload() {
    let estado = Arc::new(Estado::default());
    *estado.rota_antiga_fora.lock().unwrap() = true;
    let (base, tarefa) = subir(Arc::clone(&estado)).await;
    let cliente = cliente(&base);
    let midia = midia_de("laudo.pdf", "application/pdf", PDF);

    midia::enviar(
        &cliente,
        &fantasma(),
        "!sala1:matrix-dev.local",
        &midia,
        PDF,
    )
    .await
    .expect("a rota nova aceita");

    let uploads = estado.uploads.lock().unwrap().clone();
    assert_eq!(uploads.len(), 1, "só a rota nova registrou o arquivo");
    assert_eq!(uploads[0].caminho, "/_matrix/client/v1/media/upload");
    tarefa.abort();
}

#[tokio::test]
async fn recusa_definitiva_do_evento_volta_como_erro_definitivo() {
    let estado = Arc::new(Estado::default());
    *estado.recusar_envio.lock().unwrap() = true;
    let (base, tarefa) = subir(Arc::clone(&estado)).await;
    let cliente = cliente(&base);
    let midia = midia_de("foto.jpg", "image/jpeg", PNG);

    let erro = midia::enviar(
        &cliente,
        &fantasma(),
        "!sala1:matrix-dev.local",
        &midia,
        PNG,
    )
    .await
    .expect_err("403 no envio");
    assert_eq!(
        erro.classificacao(),
        poph_matrix::erro::Classificacao::Definitiva
    );
    // O erro não carrega o token nem o conteúdo do arquivo.
    let texto = format!("{erro}");
    assert!(!texto.contains(CANARIO));
    assert!(!texto.contains("conteudo-de-imagem"));
    tarefa.abort();
}

#[tokio::test]
async fn token_errado_nao_sobe_arquivo() {
    let estado = Arc::new(Estado::default());
    let (base, tarefa) = subir(Arc::clone(&estado)).await;
    let cliente = ClienteMatrix::novo(
        ConfigCliente::novo(&base, SERVIDOR, Segredo::new("outro-token".to_owned()))
            .expect("configuração"),
    )
    .expect("cliente");
    let midia = midia_de("foto.jpg", "image/jpeg", PNG);

    let erro = midia::enviar(
        &cliente,
        &fantasma(),
        "!sala1:matrix-dev.local",
        &midia,
        PNG,
    )
    .await
    .expect_err("401");
    assert_eq!(
        erro.classificacao(),
        poph_matrix::erro::Classificacao::Definitiva
    );
    assert!(estado.uploads.lock().unwrap().is_empty());
    tarefa.abort();
}
