//! Encaminhamento assinado entre instâncias (HR-50b-1): webhook e API administrativa.
//!
//! O mapa do `cluster.toml` (HR-50a) diz em que instância cada org vive. Quando chega um pedido de
//! org que vive em **outra** instância, esta instância encaminha o pedido à dona, assinado com
//! HMAC-SHA256 e carimbo de tempo. A dona atende como se o pedido tivesse chegado nela: o
//! `Authorization` original (token de org ou global) segue junto, e é ela quem confere o escopo.
//!
//! ## O que é encaminhado
//!
//! - `POST /webhook/evolution/{org_id}/{instancia}` (a entrada do WhatsApp);
//! - todas as rotas `/admin/v1/orgs/{org_id}/…`.
//!
//! **Não** são encaminhadas: `/healthz`, `/metrics`, `/openapi.json`, `/admin/v1/cluster/reload`,
//! `/admin/v1/orgs` (sem org no caminho) e o Console — nenhuma delas tem org no caminho, e o
//! resolvedor de org só reconhece os dois formatos acima.
//!
//! ## Assinatura
//!
//! Cabeçalhos de par (nunca presentes num pedido de cliente comum):
//!
//! | Cabeçalho | Conteúdo |
//! | --- | --- |
//! | `x-poph-instance` | `instance_id` de quem encaminha |
//! | `x-poph-timestamp` | instante do envio, em segundos Unix |
//! | `x-poph-signature` | HMAC-SHA256 em hexadecimal |
//! | `x-poph-hop` | `1` no primeiro (e único) salto |
//! | `x-poph-mapa` | impressão digital **do mapa** de quem encaminha |
//! | `x-poph-nonce` | 16 bytes aleatórios em hexadecimal (32 caracteres), um por pedido |
//!
//! Texto assinado (linhas separadas por `\n`, sem linha no fim):
//!
//! ```text
//! método
//! caminho+query
//! hex(sha256(corpo))
//! timestamp
//! instância
//! hop
//! nonce
//! ```
//!
//! A chave é o `POPH_CLUSTER_SECRET`. O recebedor confere a assinatura, a janela de
//! [`JANELA_ASSINATURA`] (para trás e para frente) e recusa a repetição do mesmo **nonce** dentro
//! da janela — o nonce é o que torna cada pedido único, então dois pedidos idênticos no mesmo
//! segundo passam os dois, e um pedido capturado e reenviado é recusado. Pedido **sem** cabeçalhos
//! de par segue o caminho normal de token, sem nenhuma mudança.
//!
//! O nonce ausente ou fora do formato (`32` caracteres hexadecimais) é `401 assinatura_invalida`.
//!
//! ## Erros
//!
//! - `401 assinatura_invalida` — cabeçalho de par ausente/malformado, assinatura errada, carimbo
//!   fora da janela, instância desconhecida ou assinatura repetida;
//! - `421 rota_em_laco` — o pedido já chegou encaminhado (`x-poph-hop > 1`) ou a org não pertence a
//!   esta instância segundo o mapa (encaminhar de novo seria um laço);
//! - `421 mapa_divergente` — o mapa de quem encaminhou não é o mapa desta instância (os dois
//!   fingerprints vão em `details`);
//! - `503 instancia_fora` — a dona não respondeu (conexão, prazo ou endereço ausente do mapa);
//! - `413 payload_too_large` — o corpo passa do teto da própria rota.
//!
//! ## Streaming
//!
//! A **resposta** da dona volta em streaming (nada é acumulado). O **corpo do pedido** é lido
//! inteiro antes de seguir, porque a assinatura usa o sha256 dele; o teto é o da própria rota
//! (webhook 1 MB, chat de teste 24 MB, importação de backup 64 MB, demais 2 MB).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::{CONNECTION, CONTENT_LENGTH, HOST, TRANSFER_ENCODING};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use futures::TryStreamExt;
use poph_types::Secret;
use ring::hmac;
use serde_json::json;

use crate::api::LIMITE_IMPORTACAO_BYTES;
use crate::cluster::ClusterVivo;
use crate::erro::ApiError;
use crate::estado::Estado;

/// Nome da instância que encaminhou.
pub const CABECALHO_INSTANCIA: &str = "x-poph-instance";
/// Instante do envio (segundos Unix).
pub const CABECALHO_TIMESTAMP: &str = "x-poph-timestamp";
/// HMAC-SHA256 do pedido, em hexadecimal.
pub const CABECALHO_ASSINATURA: &str = "x-poph-signature";
/// Salto do encaminhamento (`1` no primeiro e único salto).
pub const CABECALHO_HOP: &str = "x-poph-hop";
/// Impressão digital do mapa de quem encaminha.
pub const CABECALHO_MAPA: &str = "x-poph-mapa";
/// Nonce do pedido: 16 bytes aleatórios em hexadecimal (32 caracteres). Um por pedido, dentro do
/// texto assinado — é o que torna cada pedido único e permite recusar só a repetição.
pub const CABECALHO_NONCE: &str = "x-poph-nonce";

/// Todos os cabeçalhos de par: a presença de qualquer um já exige assinatura válida.
const CABECALHOS_DE_PAR: [&str; 6] = [
    CABECALHO_INSTANCIA,
    CABECALHO_TIMESTAMP,
    CABECALHO_ASSINATURA,
    CABECALHO_HOP,
    CABECALHO_MAPA,
    CABECALHO_NONCE,
];

/// Tamanho do nonce em bytes (16) e, portanto, em caracteres hexadecimais (32).
pub const TAMANHO_DO_NONCE: usize = 16;

/// Janela aceita para o carimbo de tempo, para trás e para frente.
pub const JANELA_ASSINATURA: Duration = Duration::from_secs(60);

/// Quanto tempo o nonce fica na memória contra repetição.
///
/// Vale o dobro da janela: passada a janela, o próprio carimbo já recusa a repetição, então não há
/// motivo para guardar mais tempo.
pub const JANELA_DE_REPLAY: Duration = Duration::from_secs(120);

/// Teto de nonces guardados por processo. Ao estourar, a memória de repetição é zerada (o pior caso
/// é uma repetição dentro da janela passar uma vez) e o evento vai para o log.
pub const MAXIMO_DE_NONCES: usize = 10_000;

/// Tempo máximo para abrir a conexão com a instância dona.
pub const TEMPO_DE_CONEXAO: Duration = Duration::from_secs(5);

/// Tempo máximo para a dona começar a responder (cabeçalho da resposta). O corpo, depois disso,
/// continua em streaming sem prazo próprio.
pub const TEMPO_DE_RESPOSTA: Duration = Duration::from_secs(60);

/// Teto de corpo para as rotas que não declaram um limite próprio (o mesmo do axum).
const TETO_DE_CORPO_PADRAO: usize = 2 * 1024 * 1024;

/// Teto do corpo da mensagem do chat de teste (o mesmo do `DefaultBodyLimit` da rota).
const TETO_DO_CHAT_DE_TESTE: usize = 24 * 1024 * 1024;

/// Caminho do pedido, com a query, como entra na assinatura.
fn alvo_do_pedido(uri: &axum::http::Uri) -> String {
    uri.path_and_query()
        .map(|par| par.as_str().to_owned())
        .unwrap_or_else(|| uri.path().to_owned())
}

/// Org do caminho, quando a rota é de uma org.
///
/// Reconhece só os dois formatos com org: o webhook do Evolution e `/admin/v1/orgs/{org_id}/…`.
/// Qualquer outra rota (saúde, métricas, OpenAPI, recarga, Console, `/admin/v1/orgs`) devolve
/// `None` e é atendida localmente, como sempre.
pub fn org_do_caminho(caminho: &str) -> Option<&str> {
    let segmentos: Vec<&str> = caminho.trim_start_matches('/').split('/').collect();
    if segmentos.len() == 4 && segmentos[0] == "webhook" && segmentos[1] == "evolution" {
        let org = segmentos[2];
        return (!org.is_empty()).then_some(org);
    }
    if segmentos.len() >= 4
        && segmentos[0] == "admin"
        && segmentos[1] == "v1"
        && segmentos[2] == "orgs"
    {
        let org = segmentos[3];
        return (!org.is_empty()).then_some(org);
    }
    None
}

/// Teto de corpo da rota, espelhando o `DefaultBodyLimit` que a rota já usa.
fn teto_do_caminho(caminho: &str) -> usize {
    let segmentos: Vec<&str> = caminho.trim_start_matches('/').split('/').collect();
    if segmentos.len() >= 2 && segmentos[0] == "webhook" && segmentos[1] == "evolution" {
        return poph_evolution::DEFAULT_BODY_CAP as usize;
    }
    if caminho.ends_with("/backup/import") {
        return LIMITE_IMPORTACAO_BYTES as usize;
    }
    if caminho.contains("/test-chat/") && caminho.ends_with("/messages") {
        return TETO_DO_CHAT_DE_TESTE;
    }
    TETO_DE_CORPO_PADRAO
}

/// Junta o endereço base ao caminho, preservando um prefixo de caminho no endereço.
///
/// `http://host/base` + `/admin/v1/…` vira `http://host/base/admin/v1/…`; barras repetidas na
/// junção são normalizadas.
pub fn juntar_url(base: &str, alvo: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    let alvo = alvo.trim();
    if alvo.is_empty() {
        return base.to_owned();
    }
    let alvo = if alvo.starts_with('/') {
        alvo.to_owned()
    } else {
        format!("/{alvo}")
    };
    format!("{base}{alvo}")
}

/// Instante atual em segundos Unix.
fn agora_em_segundos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duracao| i64::try_from(duracao.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Hexadecimal minúsculo.
fn em_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut texto = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(texto, "{byte:02x}");
    }
    texto
}

/// Lê hexadecimal (só minúsculas e maiúsculas pares de dígitos).
fn de_hex(texto: &str) -> Option<Vec<u8>> {
    let bytes = texto.trim().as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut saida = Vec::with_capacity(bytes.len() / 2);
    for par in bytes.chunks(2) {
        let alto = (par[0] as char).to_digit(16)?;
        let baixo = (par[1] as char).to_digit(16)?;
        saida.push(u8::try_from(alto * 16 + baixo).ok()?);
    }
    Some(saida)
}

/// sha256 do corpo, em hexadecimal (é o que entra na assinatura).
pub fn hash_do_corpo(corpo: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, corpo);
    em_hex(digest.as_ref())
}

/// Texto canônico assinado. Formato fixo: mudá-lo invalida assinaturas entre versões.
pub fn texto_assinado(
    metodo: &Method,
    alvo: &str,
    hash_do_corpo: &str,
    carimbo: i64,
    instancia: &str,
    hop: u32,
    nonce: &str,
) -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}",
        metodo.as_str(),
        alvo,
        hash_do_corpo,
        carimbo,
        instancia,
        hop,
        nonce
    )
}

/// Gera o nonce do pedido: 16 bytes do gerador do sistema operacional, em hexadecimal.
pub fn nonce_novo() -> String {
    use rand_core::RngCore;
    let mut bytes = [0u8; TAMANHO_DO_NONCE];
    rand_core::OsRng.fill_bytes(&mut bytes);
    em_hex(&bytes)
}

/// `true` quando o nonce tem o formato esperado: 32 caracteres hexadecimais.
pub fn nonce_valido(nonce: &str) -> bool {
    nonce.len() == TAMANHO_DO_NONCE * 2 && nonce.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Assina o pedido com o segredo compartilhado. Devolve o HMAC em hexadecimal.
#[allow(clippy::too_many_arguments)]
pub fn assinar(
    segredo: &Secret,
    metodo: &Method,
    alvo: &str,
    corpo: &[u8],
    carimbo: i64,
    instancia: &str,
    hop: u32,
    nonce: &str,
) -> String {
    let chave = hmac::Key::new(hmac::HMAC_SHA256, segredo.expose().as_bytes());
    let texto = texto_assinado(
        metodo,
        alvo,
        &hash_do_corpo(corpo),
        carimbo,
        instancia,
        hop,
        nonce,
    );
    let tag = hmac::sign(&chave, texto.as_bytes());
    em_hex(tag.as_ref())
}

/// Confere a assinatura do pedido em tempo constante.
#[allow(clippy::too_many_arguments)]
pub fn conferir(
    segredo: &Secret,
    metodo: &Method,
    alvo: &str,
    corpo: &[u8],
    carimbo: i64,
    instancia: &str,
    hop: u32,
    nonce: &str,
    assinatura: &str,
) -> bool {
    let Some(bytes) = de_hex(assinatura) else {
        return false;
    };
    let chave = hmac::Key::new(hmac::HMAC_SHA256, segredo.expose().as_bytes());
    let texto = texto_assinado(
        metodo,
        alvo,
        &hash_do_corpo(corpo),
        carimbo,
        instancia,
        hop,
        nonce,
    );
    hmac::verify(&chave, texto.as_bytes(), &bytes).is_ok()
}

/// `true` quando o pedido traz qualquer cabeçalho de par (mesmo um só).
fn tem_cabecalho_de_par(cabecalhos: &HeaderMap) -> bool {
    CABECALHOS_DE_PAR
        .iter()
        .any(|nome| cabecalhos.contains_key(*nome))
}

/// Valor de um cabeçalho de par como texto ASCII, ou `401 assinatura_invalida`.
fn cabecalho<'a>(cabecalhos: &'a HeaderMap, nome: &str) -> Result<&'a str, ApiError> {
    cabecalhos
        .get(nome)
        .and_then(|valor| valor.to_str().ok())
        .map(str::trim)
        .filter(|valor| !valor.is_empty())
        .ok_or_else(|| {
            ApiError::novo(
                StatusCode::UNAUTHORIZED,
                "assinatura_invalida",
                format!("cabeçalho `{nome}` ausente ou ilegível no encaminhamento"),
            )
        })
}

fn assinatura_invalida(motivo: impl Into<String>) -> ApiError {
    ApiError::novo(StatusCode::UNAUTHORIZED, "assinatura_invalida", motivo)
}

fn rota_em_laco(motivo: impl Into<String>, detalhes: serde_json::Value) -> ApiError {
    ApiError::novo(StatusCode::MISDIRECTED_REQUEST, "rota_em_laco", motivo).com_detalhes(detalhes)
}

fn mapa_divergente(recebido: &str, local: &str) -> ApiError {
    ApiError::novo(
        StatusCode::MISDIRECTED_REQUEST,
        "mapa_divergente",
        "o mapa do cluster de quem encaminhou não é o mesmo desta instância; \
         atualize o cluster.toml dos dois lados e recarregue",
    )
    .com_detalhes(json!({
        "fingerprint_recebido": recebido,
        "fingerprint_local": local,
    }))
}

fn instancia_fora(instancia: &str, url: &str, motivo: &str) -> ApiError {
    ApiError::novo(
        StatusCode::SERVICE_UNAVAILABLE,
        "instancia_fora",
        format!("a instância `{instancia}` não respondeu: {motivo}"),
    )
    .com_detalhes(json!({ "instancia": instancia, "url": url }))
}

/// Middleware do roteamento entre instâncias: verifica quem chega de par, encaminha quem é de
/// outra instância e deixa o resto passar como sempre.
pub async fn middleware(State(estado): State<Estado>, pedido: Request, proximo: Next) -> Response {
    let cluster = estado.cluster();
    if tem_cabecalho_de_par(pedido.headers()) {
        return atender_de_par(estado, cluster, pedido, proximo).await;
    }
    if !cluster.declara_cluster() {
        return proximo.run(pedido).await;
    }
    let Some(org) = org_do_caminho(pedido.uri().path()) else {
        return proximo.run(pedido).await;
    };
    let local = cluster.config.instance_id.trim().to_owned();
    let destino = cluster
        .config
        .org_instance(org)
        .map(str::trim)
        .unwrap_or(local.as_str())
        .to_owned();
    if destino == local {
        return proximo.run(pedido).await;
    }
    let org = org.to_owned();
    encaminhar(&cluster, &org, &destino, pedido).await
}

/// Atende um pedido que chegou encaminhado: confere a assinatura e a coerência do mapa e segue
/// para a rota local.
async fn atender_de_par(
    estado: Estado,
    cluster: ClusterVivo,
    pedido: Request,
    proximo: Next,
) -> Response {
    let (partes, corpo) = pedido.into_parts();

    let instancia = match cabecalho(&partes.headers, CABECALHO_INSTANCIA) {
        Ok(valor) => valor.to_owned(),
        Err(erro) => return erro.into_response(),
    };
    let carimbo = match cabecalho(&partes.headers, CABECALHO_TIMESTAMP).and_then(|texto| {
        texto
            .parse::<i64>()
            .map_err(|_| assinatura_invalida("carimbo de tempo do encaminhamento inválido"))
    }) {
        Ok(valor) => valor,
        Err(erro) => return erro.into_response(),
    };
    let assinatura = match cabecalho(&partes.headers, CABECALHO_ASSINATURA) {
        Ok(valor) => valor.to_owned(),
        Err(erro) => return erro.into_response(),
    };
    let hop = match cabecalho(&partes.headers, CABECALHO_HOP).and_then(|texto| {
        texto
            .parse::<u32>()
            .map_err(|_| assinatura_invalida("salto do encaminhamento inválido"))
    }) {
        Ok(valor) => valor,
        Err(erro) => return erro.into_response(),
    };
    let mapa = match cabecalho(&partes.headers, CABECALHO_MAPA) {
        Ok(valor) => valor.to_owned(),
        Err(erro) => return erro.into_response(),
    };
    let nonce = match cabecalho(&partes.headers, CABECALHO_NONCE) {
        Ok(valor) if nonce_valido(valor) => valor.to_owned(),
        Ok(_) => {
            return assinatura_invalida(
                "nonce do encaminhamento malformado (esperado: 32 caracteres hexadecimais)",
            )
            .into_response();
        }
        Err(erro) => return erro.into_response(),
    };

    let Some(segredo) = cluster.segredo.as_ref() else {
        return assinatura_invalida(
            "esta instância não tem segredo compartilhado configurado e não aceita pedido \
             encaminhado",
        )
        .into_response();
    };
    if !cluster
        .config
        .instances
        .iter()
        .any(|declarada| declarada.id.trim() == instancia)
    {
        return assinatura_invalida(format!(
            "a instância `{instancia}` não está declarada no mapa desta instância"
        ))
        .into_response();
    }
    let agora = agora_em_segundos();
    let janela = i64::try_from(JANELA_ASSINATURA.as_secs()).unwrap_or(i64::MAX);
    if agora.abs_diff(carimbo) > u64::try_from(janela).unwrap_or(u64::MAX) {
        return assinatura_invalida(
            "carimbo de tempo do encaminhamento fora da janela aceita (60 s)",
        )
        .into_response();
    }

    let alvo = alvo_do_pedido(&partes.uri);
    let teto = teto_do_caminho(partes.uri.path());
    let bytes = match axum::body::to_bytes(corpo, teto).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return ApiError::novo(
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                "o corpo do pedido encaminhado passa do limite desta rota",
            )
            .into_response();
        }
    };

    if !conferir(
        segredo,
        &partes.method,
        &alvo,
        &bytes,
        carimbo,
        &instancia,
        hop,
        &nonce,
        &assinatura,
    ) {
        return assinatura_invalida("assinatura do encaminhamento inválida").into_response();
    }

    // A memória anti-repetição é indexada pelo nonce: dois pedidos idênticos no mesmo segundo têm
    // nonces diferentes e passam os dois; um pedido capturado e reenviado com o mesmo nonce não.
    if !estado.registrar_nonce(&nonce) {
        return assinatura_invalida(
            "nonce repetido dentro da janela aceita (proteção contra repetição)",
        )
        .into_response();
    }

    if hop > 1 {
        return rota_em_laco(
            "o pedido chegou encaminhado e precisaria ser encaminhado de novo",
            json!({ "hop": hop, "instancia": cluster.config.instance_id.trim() }),
        )
        .into_response();
    }

    let fingerprint_local = cluster.config.map_fingerprint();
    if mapa != fingerprint_local {
        return mapa_divergente(&mapa, &fingerprint_local).into_response();
    }

    if let Some(org) = org_do_caminho(partes.uri.path()) {
        let local = cluster.config.instance_id.trim();
        let destino = cluster
            .config
            .org_instance(org)
            .map(str::trim)
            .unwrap_or(local);
        if destino != local {
            return rota_em_laco(
                format!(
                    "a org `{org}` não vive nesta instância (`{local}`); a dona é `{destino}` — \
                     o pedido foi encaminhado para a instância errada"
                ),
                json!({
                    "org": org,
                    "instancia_local": local,
                    "instancia_dona": destino,
                    "hop": hop,
                }),
            )
            .into_response();
        }
    }

    let pedido = Request::from_parts(partes, Body::from(bytes));
    proximo.run(pedido).await
}

/// Cliente HTTP do encaminhamento, um por processo (reaproveita conexões entre pedidos).
///
/// Sem prazo total: a resposta pode ser longa e vai em streaming. O prazo de conexão é curto e o
/// de obter a resposta é aplicado por pedido, em [`encaminhar`].
fn cliente() -> reqwest::Client {
    static CLIENTE: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENTE
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(TEMPO_DE_CONEXAO)
                .build()
                .unwrap_or_default()
        })
        .clone()
}

/// Encaminha o pedido para a instância dona da org, assinado.
async fn encaminhar(cluster: &ClusterVivo, org: &str, destino: &str, pedido: Request) -> Response {
    let Some(url_base) = cluster
        .config
        .instances
        .iter()
        .find(|declarada| declarada.id.trim() == destino)
        .map(|declarada| declarada.url.trim().to_owned())
    else {
        return instancia_fora(
            destino,
            "",
            "a instância não está declarada no mapa desta instância",
        )
        .into_response();
    };
    let Some(segredo) = cluster.segredo.as_ref() else {
        return ApiError::novo(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "o cluster não tem segredo compartilhado configurado; reinicie com \
             POPH_CLUSTER_SECRET",
        )
        .into_response();
    };

    let (partes, corpo) = pedido.into_parts();
    let alvo = alvo_do_pedido(&partes.uri);
    let teto = teto_do_caminho(partes.uri.path());
    let bytes = match axum::body::to_bytes(corpo, teto).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return ApiError::novo(
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                "o corpo do pedido passa do limite desta rota",
            )
            .into_response();
        }
    };

    let instancia_local = cluster.config.instance_id.trim().to_owned();
    let carimbo = agora_em_segundos();
    let hop = 1u32;
    let nonce = nonce_novo();
    let assinatura = assinar(
        segredo,
        &partes.method,
        &alvo,
        &bytes,
        carimbo,
        &instancia_local,
        hop,
        &nonce,
    );
    let url = juntar_url(&url_base, &alvo);

    let mut cabecalhos = HeaderMap::new();
    for (nome, valor) in partes.headers.iter() {
        if nome == HOST || nome == CONTENT_LENGTH || nome == CONNECTION || nome == TRANSFER_ENCODING
        {
            continue;
        }
        if CABECALHOS_DE_PAR
            .iter()
            .any(|de_par| nome.as_str().eq_ignore_ascii_case(de_par))
        {
            continue;
        }
        cabecalhos.append(nome.clone(), valor.clone());
    }
    let mut com_par = |nome: &'static str, valor: String| {
        if let Ok(valor) = HeaderValue::from_str(&valor) {
            cabecalhos.insert(axum::http::HeaderName::from_static(nome), valor);
        }
    };
    com_par(CABECALHO_INSTANCIA, instancia_local.clone());
    com_par(CABECALHO_TIMESTAMP, carimbo.to_string());
    com_par(CABECALHO_ASSINATURA, assinatura);
    com_par(CABECALHO_HOP, hop.to_string());
    com_par(CABECALHO_MAPA, cluster.config.map_fingerprint());
    com_par(CABECALHO_NONCE, nonce);

    let pedido_destino = cliente()
        .request(partes.method.clone(), &url)
        .headers(cabecalhos)
        .body(bytes);
    let resposta = match tokio::time::timeout(TEMPO_DE_RESPOSTA, pedido_destino.send()).await {
        Ok(Ok(resposta)) => resposta,
        Ok(Err(erro)) => {
            tracing::warn!(
                target: "poph_server::encaminhamento",
                org = %org,
                instancia = %destino,
                url = %url_base,
                erro = %erro,
                "encaminhamento recusado: a instância dona não respondeu"
            );
            return instancia_fora(destino, &url_base, &erro.to_string()).into_response();
        }
        Err(_) => {
            tracing::warn!(
                target: "poph_server::encaminhamento",
                org = %org,
                instancia = %destino,
                url = %url_base,
                "encaminhamento recusado: a instância dona não respondeu no prazo"
            );
            return instancia_fora(
                destino,
                &url_base,
                &format!("sem resposta em {} s", TEMPO_DE_RESPOSTA.as_secs().max(1)),
            )
            .into_response();
        }
    };

    tracing::info!(
        target: "poph_server::encaminhamento",
        org = %org,
        instancia = %destino,
        metodo = %partes.method,
        caminho = %partes.uri.path(),
        status = resposta.status().as_u16(),
        "pedido encaminhado para a instância dona"
    );

    resposta_encaminhada(resposta)
}

/// Converte a resposta da dona em resposta para o cliente, com o corpo em streaming.
fn resposta_encaminhada(resposta: reqwest::Response) -> Response {
    let status = resposta.status();
    let cabecalhos = resposta.headers().clone();
    let corpo = Body::from_stream(resposta.bytes_stream().map_err(std::io::Error::other));
    let mut construir = Response::builder().status(status);
    for (nome, valor) in cabecalhos.iter() {
        if nome == CONTENT_LENGTH || nome == TRANSFER_ENCODING || nome == CONNECTION {
            continue;
        }
        construir = construir.header(nome, valor);
    }
    construir.body(corpo).unwrap_or_else(|_| {
        ApiError::novo(
            StatusCode::BAD_GATEWAY,
            "internal",
            "não consegui montar a resposta encaminhada",
        )
        .into_response()
    })
}

/// Corpo já lido, para os testes do módulo.
#[cfg(test)]
fn bytes(texto: &str) -> axum::body::Bytes {
    axum::body::Bytes::from(texto.to_owned())
}

#[cfg(test)]
mod testes {
    use super::*;
    use poph_types::config::{ClusterConfig, ClusterInstance};

    fn segredo() -> Secret {
        Secret::new("segredo-compartilhado-da-hr50b".to_owned())
    }

    fn mapa(local: &str) -> ClusterConfig {
        ClusterConfig {
            instance_id: local.to_owned(),
            default_instance: Some("b".to_owned()),
            instances: vec![
                ClusterInstance {
                    id: "a".to_owned(),
                    url: "http://127.0.0.1:7101/base".to_owned(),
                },
                ClusterInstance {
                    id: "b".to_owned(),
                    url: "http://127.0.0.1:7102".to_owned(),
                },
            ],
            orgs: [("org_01".to_owned(), "a".to_owned())]
                .into_iter()
                .collect(),
        }
    }

    #[test]
    fn reconhece_org_so_nas_rotas_com_org() {
        assert_eq!(
            org_do_caminho("/webhook/evolution/org_01/loja"),
            Some("org_01")
        );
        assert_eq!(
            org_do_caminho("/admin/v1/orgs/org_01/agents/agt_1"),
            Some("org_01")
        );
        assert_eq!(org_do_caminho("/admin/v1/orgs/org_01"), Some("org_01"));
        assert_eq!(org_do_caminho("/admin/v1/orgs"), None);
        assert_eq!(org_do_caminho("/admin/v1/orgs/"), None);
        assert_eq!(org_do_caminho("/admin/v1/cluster/reload"), None);
        assert_eq!(org_do_caminho("/healthz"), None);
        assert_eq!(org_do_caminho("/metrics"), None);
        assert_eq!(org_do_caminho("/openapi.json"), None);
        assert_eq!(org_do_caminho("/admin/v1/openapi.json"), None);
        assert_eq!(org_do_caminho("/orgs/org_01/agents"), None);
        assert_eq!(org_do_caminho("/webhook/evolution/org_01"), None);
    }

    #[test]
    fn teto_de_corpo_espelha_o_da_rota() {
        assert_eq!(
            teto_do_caminho("/webhook/evolution/org_01/loja"),
            poph_evolution::DEFAULT_BODY_CAP as usize
        );
        assert_eq!(
            teto_do_caminho("/admin/v1/orgs/org_01/backup/import"),
            LIMITE_IMPORTACAO_BYTES as usize
        );
        assert_eq!(
            teto_do_caminho("/admin/v1/orgs/org_01/agents/agt_1/test-chat/c/messages"),
            TETO_DO_CHAT_DE_TESTE
        );
        assert_eq!(
            teto_do_caminho("/admin/v1/orgs/org_01/agents"),
            TETO_DE_CORPO_PADRAO
        );
    }

    #[test]
    fn junta_url_preservando_prefixo_e_sem_barra_dupla() {
        assert_eq!(
            juntar_url("http://host:7480", "/admin/v1/orgs/org_01"),
            "http://host:7480/admin/v1/orgs/org_01"
        );
        assert_eq!(
            juntar_url("http://host:7480/base/", "/admin/v1/orgs/org_01"),
            "http://host:7480/base/admin/v1/orgs/org_01"
        );
        assert_eq!(
            juntar_url("https://harness.exemplo", "webhook/evolution/org_01/loja"),
            "https://harness.exemplo/webhook/evolution/org_01/loja"
        );
        assert_eq!(
            juntar_url("http://host:7480", "/x?y=1"),
            "http://host:7480/x?y=1"
        );
    }

    #[test]
    fn assinatura_redonda_e_sensivel_a_cada_parte() {
        let alvo = "/admin/v1/orgs/org_01/agents?limit=2";
        let corpo = bytes("{\"nome\":\"Loja\"}");
        let nonce = "0123456789abcdef0123456789abcdef";
        let outro_nonce = "fedcba9876543210fedcba9876543210";
        let assinatura = assinar(
            &segredo(),
            &Method::POST,
            alvo,
            &corpo,
            1_800,
            "a",
            1,
            nonce,
        );
        let confere = |corpo: &[u8],
                       metodo: Method,
                       alvo: &str,
                       carimbo: i64,
                       instancia: &str,
                       hop: u32,
                       nonce: &str| {
            conferir(
                &segredo(),
                &metodo,
                alvo,
                corpo,
                carimbo,
                instancia,
                hop,
                nonce,
                &assinatura,
            )
        };
        assert!(confere(&corpo, Method::POST, alvo, 1_800, "a", 1, nonce));
        // Corpo, caminho, método, carimbo, instância, salto e nonce entram na assinatura.
        assert!(!confere(
            b"{\"nome\":\"Outra\"}",
            Method::POST,
            alvo,
            1_800,
            "a",
            1,
            nonce
        ));
        assert!(!confere(&corpo, Method::GET, alvo, 1_800, "a", 1, nonce));
        assert!(!confere(
            &corpo,
            Method::POST,
            "/admin/v1/orgs/org_01/agents",
            1_800,
            "a",
            1,
            nonce
        ));
        assert!(!confere(&corpo, Method::POST, alvo, 1_801, "a", 1, nonce));
        assert!(!confere(&corpo, Method::POST, alvo, 1_800, "b", 1, nonce));
        assert!(!confere(&corpo, Method::POST, alvo, 1_800, "a", 2, nonce));
        assert!(!confere(
            &corpo,
            Method::POST,
            alvo,
            1_800,
            "a",
            1,
            outro_nonce
        ));
        // Segredo diferente recusa.
        assert!(!conferir(
            &Secret::new("outro".to_owned()),
            &Method::POST,
            alvo,
            &corpo,
            1_800,
            "a",
            1,
            nonce,
            &assinatura
        ));
        // Assinatura ilegível recusa sem pânico.
        assert!(!conferir(
            &segredo(),
            &Method::POST,
            alvo,
            &corpo,
            1_800,
            "a",
            1,
            nonce,
            "zz"
        ));
        assert!(!conferir(
            &segredo(),
            &Method::POST,
            alvo,
            &corpo,
            1_800,
            "a",
            1,
            nonce,
            "abc"
        ));
    }

    #[test]
    fn nonce_e_unico_e_no_formato_esperado() {
        let primeiro = nonce_novo();
        let segundo = nonce_novo();
        assert_eq!(primeiro.len(), TAMANHO_DO_NONCE * 2);
        assert!(nonce_valido(&primeiro));
        assert_ne!(
            primeiro, segundo,
            "dois pedidos seguidos não podem ter o mesmo nonce"
        );
        assert!(!nonce_valido(""));
        assert!(!nonce_valido("abc"));
        assert!(!nonce_valido("zz3456789abcdef0123456789abcdef"));
        assert!(!nonce_valido("0123456789abcdef0123456789abcde"));
        assert!(!nonce_valido("0123456789abcdef0123456789abcdef0"));
    }

    #[test]
    fn hex_ida_e_volta() {
        assert_eq!(em_hex(&[0x00, 0x0f, 0xff]), "000fff");
        assert_eq!(de_hex("000fff"), Some(vec![0x00, 0x0f, 0xff]));
        assert_eq!(de_hex("000FFF"), Some(vec![0x00, 0x0f, 0xff]));
        assert_eq!(de_hex("0"), None);
        assert_eq!(de_hex(""), None);
        assert_eq!(de_hex("zz"), None);
    }

    #[test]
    fn fingerprint_do_mapa_ignora_a_instancia_local_e_sensibiliza_o_resto() {
        let a = mapa("a");
        let b = mapa("b");
        assert_eq!(a.map_fingerprint(), b.map_fingerprint());
        assert_ne!(a.fingerprint(), b.fingerprint());

        let mut com_org_trocada = mapa("a");
        com_org_trocada
            .orgs
            .insert("org_01".to_owned(), "b".to_owned());
        assert_ne!(a.map_fingerprint(), com_org_trocada.map_fingerprint());
        // Sem segredo no texto canônico.
        assert!(!a.normalized_map().to_lowercase().contains("secret"));
    }
}
