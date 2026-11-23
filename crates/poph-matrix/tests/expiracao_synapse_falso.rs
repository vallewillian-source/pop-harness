//! As duas chamadas da expiração contra um Synapse **falso** (MX-46).
//!
//! O que se prova aqui, sem banco e sem laço:
//!
//! - a redação vai para `PUT /rooms/{sala}/redact/{evento}/{txn}` **como o fantasma do agente**
//!   (`?user_id=`), com o bloco que o frontend lê no corpo e um `txnId` estável por anexo;
//! - o `txnId` repetido não cria uma segunda redação (o Synapse falso deduplica por `txn`, como o
//!   de verdade);
//! - o apagamento vai para `DELETE /_synapse/admin/v1/media/{servidor}/{id}` com o **token de
//!   administrador** no `Authorization` — e não o `as_token` do Application Service;
//! - mídia que já não existe (`404`) conta como apagada, e não como falha;
//! - `403` volta como falha definitiva, `503` como transitória: quem decide esperar é a rotina;
//! - o token de administração não aparece em `Debug`, `Display` nem na mensagem de erro.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_matrix::cliente::{ClienteMatrix, ConfigCliente};
use poph_matrix::erro::{Classificacao, ErroChamada, ErroMidia};
use poph_matrix::expiracao::{
    self, BLOCO_EXPIRADO, CAMPO_CATEGORIA, CAMPO_ENVIADA_EM, CAMPO_NOME, MidiaParaExpirar,
};
use poph_matrix::usuarios::{Fantasma, IdsFantasmas};
use poph_types::Segredo;
use serde_json::{Value, json};

/// Token do Application Service (o Harness apresenta ao Synapse).
const AS_TOKEN: &str = "as-token-do-teste";
/// Token de administração (o que apaga mídia). Valor-canário: não pode vazar em log nem em erro.
const ADMIN_TOKEN: &str = "CANARIO-ADMIN-mx46-7c1d-TAO-SECRETO";
const SERVIDOR: &str = "matrix-dev.local";
const PREFIXO: &str = "ph";
const SALA: &str = "!sala-mx46:matrix-dev.local";
const EVENTO: &str = "$midia-mx46:matrix-dev.local";
const ANEXO: &str = "anx-mx46";
const MXC: &str = "mxc://matrix-dev.local/abc123";

// ───────────────────────────── Synapse falso ─────────────────────────────

#[derive(Debug, Clone)]
struct Redacao {
    sala: String,
    evento: String,
    txn: String,
    /// Usuário em nome de quem a redação foi pedida (`?user_id=`).
    como: Option<String>,
    corpo: Value,
}

#[derive(Clone, Default)]
struct Falso {
    redacoes: Arc<Mutex<Vec<Redacao>>>,
    /// `(servidor, id)` de cada `DELETE` de mídia que chegou.
    apagadas: Arc<Mutex<Vec<(String, String)>>>,
    /// Cabeçalho `Authorization` de cada `DELETE`.
    autorizacoes: Arc<Mutex<Vec<String>>>,
    chamadas: Arc<AtomicUsize>,
    /// Próximas `n` chamadas de apagamento respondem este status.
    status_do_delete: Arc<Mutex<Option<u16>>>,
}

fn responder(status: u16, corpo: Value) -> Response {
    (
        StatusCode::from_u16(status).expect("status válido"),
        [("content-type", "application/json")],
        corpo.to_string(),
    )
        .into_response()
}

async fn atender(
    State(falso): State<Falso>,
    metodo: Method,
    uri: Uri,
    cabecalhos: HeaderMap,
    corpo: String,
) -> Response {
    falso.chamadas.fetch_add(1, Ordering::SeqCst);
    let corpo: Value = serde_json::from_str(&corpo).unwrap_or(Value::Null);
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
        if partes.len() != 2 {
            return responder(400, json!({ "error": "caminho de mídia inválido" }));
        }
        let status = falso.status_do_delete.lock().expect("trava").to_owned();
        falso
            .autorizacoes
            .lock()
            .expect("trava")
            .push(autorizacao.clone());
        if let Some(status) = status {
            return match status {
                404 => responder(404, json!({ "error": "Unknown media" })),
                403 => responder(403, json!({ "error": "You are not a server admin" })),
                outro => responder(outro, json!({ "error": "indisponível" })),
            };
        }
        if autorizacao != format!("Bearer {ADMIN_TOKEN}") {
            // O `as_token` não serve para a API administrativa, como no Synapse de verdade.
            return responder(403, json!({ "error": "You are not a server admin" }));
        }
        falso
            .apagadas
            .lock()
            .expect("trava")
            .push((partes[0].to_owned(), partes[1].to_owned()));
        return responder(200, json!({ "deleted_media": [partes[1]] }));
    }

    // API cliente: `PUT /rooms/{sala}/redact/{evento}/{txn}`.
    if metodo == Method::PUT && uri.path().contains("/redact/") {
        let partes: Vec<&str> = uri
            .path()
            .trim_start_matches("/_matrix/client/v3/rooms/")
            .split('/')
            .collect();
        if partes.len() != 4 || partes[1] != "redact" {
            return responder(404, json!({ "errcode": "M_UNRECOGNIZED" }));
        }
        if autorizacao != format!("Bearer {AS_TOKEN}") {
            return responder(401, json!({ "errcode": "M_UNKNOWN_TOKEN" }));
        }
        let mut redacoes = falso.redacoes.lock().expect("trava");
        // Como o Synapse: o mesmo `txn` na mesma sala não cria outra redação.
        if !redacoes
            .iter()
            .any(|r| r.sala == partes[0] && r.txn == partes[3])
        {
            redacoes.push(Redacao {
                sala: partes[0].to_owned(),
                evento: partes[2].to_owned(),
                txn: partes[3].to_owned(),
                como: consulta.get("user_id").cloned(),
                corpo,
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

    fn cliente(&self) -> ClienteMatrix {
        ClienteMatrix::novo(
            ConfigCliente::novo(self.url.clone(), SERVIDOR.to_owned(), segredo(AS_TOKEN))
                .expect("config do cliente"),
        )
        .expect("cliente do Matrix")
    }

    fn token_admin(&self) -> Segredo {
        segredo(ADMIN_TOKEN)
    }

    fn redacoes(&self) -> Vec<Redacao> {
        self.falso.redacoes.lock().expect("trava").clone()
    }

    fn apagadas(&self) -> Vec<(String, String)> {
        self.falso.apagadas.lock().expect("trava").clone()
    }

    fn autorizacoes(&self) -> Vec<String> {
        self.falso.autorizacoes.lock().expect("trava").clone()
    }

    fn chamadas(&self) -> usize {
        self.falso.chamadas.load(Ordering::SeqCst)
    }

    fn recusar_delete_com(&self, status: Option<u16>) {
        *self.falso.status_do_delete.lock().expect("trava") = status;
    }
}

impl Drop for Synapse {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
    }
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

fn segredo(valor: &str) -> Segredo {
    Segredo::new(valor.to_owned())
}

fn midia() -> MidiaParaExpirar<'static> {
    MidiaParaExpirar {
        id: ANEXO,
        nome: "laudo.pdf",
        mime: "application/pdf",
        categoria: "document",
        tamanho_bytes: 2_048,
        enviada_em: 1_797_900_000_000,
    }
}

fn fantasma_do_agente() -> Fantasma {
    IdsFantasmas::novo(PREFIXO, SERVIDOR)
        .expect("ids dos fantasmas")
        .agente("agt_1")
        .expect("fantasma do agente")
}

// ───────────────────────────── redação ─────────────────────────────

#[tokio::test]
async fn a_redacao_vai_como_o_fantasma_do_agente_com_o_bloco_do_frontend() {
    let synapse = Synapse::subir().await;
    let cliente = synapse.cliente();
    let agente = fantasma_do_agente();

    let evento_da_redacao = expiracao::redigir(&cliente, &agente, SALA, EVENTO, &midia())
        .await
        .expect("redige");
    assert_eq!(evento_da_redacao, "$redacao-1:matrix-dev.local");

    let redacoes = synapse.redacoes();
    assert_eq!(redacoes.len(), 1, "uma redação: {redacoes:?}");
    let redacao = &redacoes[0];
    assert_eq!(redacao.sala, SALA);
    assert_eq!(redacao.evento, EVENTO);
    assert_eq!(redacao.txn, expiracao::txn_da_redacao(ANEXO));
    assert_eq!(
        redacao.como.as_deref(),
        Some(agente.user_id()),
        "quem redige é o fantasma do agente (moderador da sala)"
    );
    let bloco = &redacao.corpo[BLOCO_EXPIRADO];
    assert_eq!(bloco[CAMPO_NOME], json!("laudo.pdf"));
    assert_eq!(bloco[CAMPO_CATEGORIA], json!("document"));
    assert_eq!(bloco[CAMPO_ENVIADA_EM], json!("2026-12-22T00:40:00Z"));
    assert_eq!(bloco["mimetype"], json!("application/pdf"));
    assert_eq!(bloco["size"], json!(2_048));
}

#[tokio::test]
async fn repetir_a_redacao_nao_cria_segunda_redacao_no_synapse() {
    let synapse = Synapse::subir().await;
    let cliente = synapse.cliente();
    let agente = fantasma_do_agente();

    for _ in 0..3 {
        expiracao::redigir(&cliente, &agente, SALA, EVENTO, &midia())
            .await
            .expect("redige");
    }
    assert_eq!(
        synapse.redacoes().len(),
        1,
        "o mesmo txnId não duplica a redação (é o que o Synapse faz de verdade)"
    );
}

#[tokio::test]
async fn redacao_recusada_volta_classificada_sem_o_token_no_erro() {
    let synapse = Synapse::subir().await;
    let agente = fantasma_do_agente();
    // Cliente com outro `as_token`: o Synapse falso responde 401.
    let intruso = ClienteMatrix::novo(
        ConfigCliente::novo(
            synapse.url.clone(),
            SERVIDOR.to_owned(),
            segredo("outro-token"),
        )
        .expect("config"),
    )
    .expect("cliente");

    let erro = expiracao::redigir(&intruso, &agente, SALA, EVENTO, &midia())
        .await
        .expect_err("401 é falha");
    assert_eq!(erro.classificacao(), Classificacao::Definitiva);
    assert!(
        !erro.to_string().contains("outro-token"),
        "o erro não leva token: {erro}"
    );
    assert!(synapse.redacoes().is_empty());
}

// ───────────────────────────── apagamento ─────────────────────────────

#[tokio::test]
async fn apagar_usa_a_api_administrativa_com_o_token_de_administrador() {
    let synapse = Synapse::subir().await;
    let cliente = synapse.cliente();

    let apagou = expiracao::apagar(&cliente, &synapse.token_admin(), MXC)
        .await
        .expect("apaga");
    assert!(apagou, "o arquivo existia");
    assert_eq!(
        synapse.apagadas(),
        vec![(SERVIDOR.to_owned(), "abc123".to_owned())]
    );
    for autorizacao in synapse.autorizacoes() {
        assert_eq!(
            autorizacao,
            format!("Bearer {ADMIN_TOKEN}"),
            "o apagamento usa o token de administrador, não o do Application Service"
        );
    }
}

#[tokio::test]
async fn midia_que_ja_nao_existe_conta_como_apagada() {
    let synapse = Synapse::subir().await;
    let cliente = synapse.cliente();
    synapse.recusar_delete_com(Some(404));

    let apagou = expiracao::apagar(&cliente, &synapse.token_admin(), MXC)
        .await
        .expect("404 não é erro");
    assert!(!apagou, "já não existia: nada foi apagado agora");
}

#[tokio::test]
async fn sem_permissao_e_falha_definitiva_e_indisponivel_e_transitoria() {
    let synapse = Synapse::subir().await;
    let cliente = synapse.cliente();

    for (status, esperada) in [
        (403, Classificacao::Definitiva),
        (503, Classificacao::Transitoria),
        (429, Classificacao::Transitoria),
    ] {
        synapse.recusar_delete_com(Some(status));
        let erro = expiracao::apagar(&cliente, &synapse.token_admin(), MXC)
            .await
            .expect_err("status de erro não é sucesso");
        assert_eq!(erro.classificacao(), esperada, "status {status}: {erro}");

        let codigo = expiracao::codigo_do_erro(&erro);
        assert!(
            codigo.len() <= 64 && !codigo.contains("mxc://") && !codigo.contains(ADMIN_TOKEN),
            "código do erro curto e sem segredo: {codigo}"
        );
    }
}

#[tokio::test]
async fn endereco_de_midia_invalido_nao_vira_chamada() {
    let synapse = Synapse::subir().await;
    let cliente = synapse.cliente();
    let antes = synapse.chamadas();

    let erro = expiracao::apagar(&cliente, &synapse.token_admin(), "https://outro/servidor")
        .await
        .expect_err("endereço que não é mxc:// é recusado");
    assert!(matches!(erro, ErroMidia::Endereco(_)), "{erro:?}");
    assert_eq!(
        synapse.chamadas(),
        antes,
        "nada foi chamado: o endereço é recusado antes da rede"
    );
}

// ───────────────────────────── segredo ─────────────────────────────

#[test]
fn o_token_de_administrador_nunca_aparece_em_debug_display_nem_no_erro() {
    let token = segredo(ADMIN_TOKEN);
    assert_eq!(format!("{token:?}"), "[oculto]");
    assert_eq!(format!("{token}"), "[oculto]");

    let erro = ErroChamada::Configuracao(
        "o token de administrador do Synapse tem caractere que não cabe em cabeçalho HTTP"
            .to_owned(),
    );
    assert!(!erro.to_string().contains(ADMIN_TOKEN));
}
