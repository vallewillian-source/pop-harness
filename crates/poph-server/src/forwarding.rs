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

use crate::api::LIMIT_IMPORT_BYTES;
use crate::cluster::LiveCluster;
use crate::error::ApiError;
use crate::state::ServerState;

pub const INSTANCE_HEADER: &str = "x-poph-instance";
pub const TIMESTAMP_HEADER: &str = "x-poph-timestamp";
pub const SIGNATURE_HEADER: &str = "x-poph-signature";
pub const HOP_HEADER: &str = "x-poph-hop";
pub const MAP_HEADER: &str = "x-poph-mapa";
pub const NONCE_HEADER: &str = "x-poph-nonce";

const PEER_HEADERS: [&str; 6] = [
    INSTANCE_HEADER,
    TIMESTAMP_HEADER,
    SIGNATURE_HEADER,
    HOP_HEADER,
    MAP_HEADER,
    NONCE_HEADER,
];

pub const NONCE_SIZE: usize = 16;

pub const SIGNATURE_WINDOW: Duration = Duration::from_secs(60);

pub const REPLAY_WINDOW: Duration = Duration::from_secs(120);

pub const MAX_NONCES: usize = 10_000;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

pub const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

const DEFAULT_BODY_LIMIT: usize = 2 * 1024 * 1024;

const TEST_CHAT_BODY_LIMIT: usize = 24 * 1024 * 1024;

fn request_target(uri: &axum::http::Uri) -> String {
    uri.path_and_query()
        .map(|peer| peer.as_str().to_owned())
        .unwrap_or_else(|| uri.path().to_owned())
}

pub fn org_from_path(path: &str) -> Option<&str> {
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if segments.len() == 4 && segments[0] == "webhook" && segments[1] == "evolution" {
        let org = segments[2];
        return (!org.is_empty()).then_some(org);
    }
    if segments.len() >= 4 && segments[0] == "admin" && segments[1] == "v1" && segments[2] == "orgs"
    {
        let org = segments[3];
        return (!org.is_empty()).then_some(org);
    }
    None
}

fn body_limit_for_path(path: &str) -> usize {
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if segments.len() >= 2 && segments[0] == "webhook" && segments[1] == "evolution" {
        return poph_evolution::DEFAULT_BODY_CAP as usize;
    }
    if path.ends_with("/backup/import") {
        return LIMIT_IMPORT_BYTES as usize;
    }
    if path.contains("/test-chat/") && path.ends_with("/messages") {
        return TEST_CHAT_BODY_LIMIT;
    }
    DEFAULT_BODY_LIMIT
}

pub fn join_url(base: &str, target: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    let target = target.trim();
    if target.is_empty() {
        return base.to_owned();
    }
    let target = if target.starts_with('/') {
        target.to_owned()
    } else {
        format!("/{target}")
    };
    format!("{base}{target}")
}

fn now_in_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    let bytes = text.trim().as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut output = Vec::with_capacity(bytes.len() / 2);
    for peer in bytes.chunks(2) {
        let alto = (peer[0] as char).to_digit(16)?;
        let baixo = (peer[1] as char).to_digit(16)?;
        output.push(u8::try_from(alto * 16 + baixo).ok()?);
    }
    Some(output)
}

pub fn body_hash(body: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, body);
    encode_hex(digest.as_ref())
}

pub fn signature_input(
    method: &Method,
    target: &str,
    body_hash: &str,
    timestamp: i64,
    instance: &str,
    hop: u32,
    nonce: &str,
) -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}",
        method.as_str(),
        target,
        body_hash,
        timestamp,
        instance,
        hop,
        nonce
    )
}

pub fn new_nonce() -> String {
    use rand_core::RngCore;
    let mut bytes = [0u8; NONCE_SIZE];
    rand_core::OsRng.fill_bytes(&mut bytes);
    encode_hex(&bytes)
}

pub fn is_valid_nonce(nonce: &str) -> bool {
    nonce.len() == NONCE_SIZE * 2 && nonce.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[allow(clippy::too_many_arguments)]
pub fn sign(
    secret: &Secret,
    method: &Method,
    target: &str,
    body: &[u8],
    timestamp: i64,
    instance: &str,
    hop: u32,
    nonce: &str,
) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.expose().as_bytes());
    let text = signature_input(
        method,
        target,
        &body_hash(body),
        timestamp,
        instance,
        hop,
        nonce,
    );
    let tag = hmac::sign(&key, text.as_bytes());
    encode_hex(tag.as_ref())
}

#[allow(clippy::too_many_arguments)]
pub fn verify(
    secret: &Secret,
    method: &Method,
    target: &str,
    body: &[u8],
    timestamp: i64,
    instance: &str,
    hop: u32,
    nonce: &str,
    signature: &str,
) -> bool {
    let Some(bytes) = decode_hex(signature) else {
        return false;
    };
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.expose().as_bytes());
    let text = signature_input(
        method,
        target,
        &body_hash(body),
        timestamp,
        instance,
        hop,
        nonce,
    );
    hmac::verify(&key, text.as_bytes(), &bytes).is_ok()
}

fn has_peer_header(headers: &HeaderMap) -> bool {
    PEER_HEADERS.iter().any(|name| headers.contains_key(*name))
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, ApiError> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "assinatura_invalida",
                format!("cabeçalho `{name}` ausente ou ilegível no encaminhamento"),
            )
        })
}

fn invalid_signature(reason: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::UNAUTHORIZED, "assinatura_invalida", reason)
}

fn routing_loop(reason: impl Into<String>, details: serde_json::Value) -> ApiError {
    ApiError::new(StatusCode::MISDIRECTED_REQUEST, "rota_em_laco", reason).with_details(details)
}

fn map_mismatch(received: &str, local: &str) -> ApiError {
    ApiError::new(
        StatusCode::MISDIRECTED_REQUEST,
        "mapa_divergente",
        "o mapa do cluster de quem encaminhou não é o mesmo desta instância; \
         atualize o cluster.toml dos dois lados e recarregue",
    )
    .with_details(json!({
        "fingerprint_recebido": received,
        "fingerprint_local": local,
    }))
}

fn instance_unavailable(instance: &str, url: &str, reason: &str) -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "instancia_fora",
        format!("a instância `{instance}` não respondeu: {reason}"),
    )
    .with_details(json!({ "instancia": instance, "url": url }))
}

pub async fn middleware(
    State(state): State<ServerState>,
    request: Request,
    next: Next,
) -> Response {
    let cluster = state.cluster();
    if has_peer_header(request.headers()) {
        return handle_peer_request(state, cluster, request, next).await;
    }
    if !cluster.declares_cluster() {
        return next.run(request).await;
    }
    let Some(org) = org_from_path(request.uri().path()) else {
        return next.run(request).await;
    };
    let local = cluster.config.instance_id.trim().to_owned();
    let destination = cluster
        .config
        .org_instance(org)
        .map(str::trim)
        .unwrap_or(local.as_str())
        .to_owned();
    if destination == local {
        return next.run(request).await;
    }
    let org = org.to_owned();
    forward(&cluster, &org, &destination, request).await
}

async fn handle_peer_request(
    state: ServerState,
    cluster: LiveCluster,
    request: Request,
    next: Next,
) -> Response {
    let (parts, body) = request.into_parts();

    let instance = match header(&parts.headers, INSTANCE_HEADER) {
        Ok(value) => value.to_owned(),
        Err(error) => return error.into_response(),
    };
    let timestamp = match header(&parts.headers, TIMESTAMP_HEADER).and_then(|text| {
        text.parse::<i64>()
            .map_err(|_| invalid_signature("carimbo de tempo do encaminhamento inválido"))
    }) {
        Ok(value) => value,
        Err(error) => return error.into_response(),
    };
    let signature = match header(&parts.headers, SIGNATURE_HEADER) {
        Ok(value) => value.to_owned(),
        Err(error) => return error.into_response(),
    };
    let hop = match header(&parts.headers, HOP_HEADER).and_then(|text| {
        text.parse::<u32>()
            .map_err(|_| invalid_signature("salto do encaminhamento inválido"))
    }) {
        Ok(value) => value,
        Err(error) => return error.into_response(),
    };
    let map = match header(&parts.headers, MAP_HEADER) {
        Ok(value) => value.to_owned(),
        Err(error) => return error.into_response(),
    };
    let nonce = match header(&parts.headers, NONCE_HEADER) {
        Ok(value) if is_valid_nonce(value) => value.to_owned(),
        Ok(_) => {
            return invalid_signature(
                "nonce do encaminhamento malformado (esperado: 32 caracteres hexadecimais)",
            )
            .into_response();
        }
        Err(error) => return error.into_response(),
    };

    let Some(secret) = cluster.secret.as_ref() else {
        return invalid_signature(
            "esta instância não tem segredo compartilhado configurado e não aceita pedido \
             encaminhado",
        )
        .into_response();
    };
    if !cluster
        .config
        .instances
        .iter()
        .any(|declared| declared.id.trim() == instance)
    {
        return invalid_signature(format!(
            "a instância `{instance}` não está declarada no mapa desta instância"
        ))
        .into_response();
    }
    let now = now_in_seconds();
    let window = i64::try_from(SIGNATURE_WINDOW.as_secs()).unwrap_or(i64::MAX);
    if now.abs_diff(timestamp) > u64::try_from(window).unwrap_or(u64::MAX) {
        return invalid_signature(
            "carimbo de tempo do encaminhamento fora da janela aceita (60 s)",
        )
        .into_response();
    }

    let target = request_target(&parts.uri);
    let cap = body_limit_for_path(parts.uri.path());
    let bytes = match axum::body::to_bytes(body, cap).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                "o corpo do pedido encaminhado passa do limite desta rota",
            )
            .into_response();
        }
    };

    if !verify(
        secret,
        &parts.method,
        &target,
        &bytes,
        timestamp,
        &instance,
        hop,
        &nonce,
        &signature,
    ) {
        return invalid_signature("assinatura do encaminhamento inválida").into_response();
    }

    if !state.register_nonce(&nonce) {
        return invalid_signature(
            "nonce repetido dentro da janela aceita (proteção contra repetição)",
        )
        .into_response();
    }

    if hop > 1 {
        return routing_loop(
            "o pedido chegou encaminhado e precisaria ser encaminhado de novo",
            json!({ "hop": hop, "instancia": cluster.config.instance_id.trim() }),
        )
        .into_response();
    }

    let fingerprint_local = cluster.config.map_fingerprint();
    if map != fingerprint_local {
        return map_mismatch(&map, &fingerprint_local).into_response();
    }

    if let Some(org) = org_from_path(parts.uri.path()) {
        let local = cluster.config.instance_id.trim();
        let destination = cluster
            .config
            .org_instance(org)
            .map(str::trim)
            .unwrap_or(local);
        if destination != local {
            return routing_loop(
                format!(
                    "a org `{org}` não vive nesta instância (`{local}`); a dona é `{destination}` — \
                     o pedido foi encaminhado para a instância errada"
                ),
                json!({
                    "org": org,
                    "instancia_local": local,
                    "instancia_dona": destination,
                    "hop": hop,
                }),
            )
            .into_response();
        }
    }

    let request = Request::from_parts(parts, Body::from(bytes));
    next.run(request).await
}

fn client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .build()
                .unwrap_or_default()
        })
        .clone()
}

async fn forward(
    cluster: &LiveCluster,
    org: &str,
    destination: &str,
    request: Request,
) -> Response {
    let Some(base_url) = cluster
        .config
        .instances
        .iter()
        .find(|declared| declared.id.trim() == destination)
        .map(|declared| declared.url.trim().to_owned())
    else {
        return instance_unavailable(
            destination,
            "",
            "a instância não está declarada no mapa desta instância",
        )
        .into_response();
    };
    let Some(secret) = cluster.secret.as_ref() else {
        return ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "o cluster não tem segredo compartilhado configurado; reinicie com \
             POPH_CLUSTER_SECRET",
        )
        .into_response();
    };

    let (parts, body) = request.into_parts();
    let target = request_target(&parts.uri);
    let cap = body_limit_for_path(parts.uri.path());
    let bytes = match axum::body::to_bytes(body, cap).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                "o corpo do pedido passa do limite desta rota",
            )
            .into_response();
        }
    };

    let local_instance = cluster.config.instance_id.trim().to_owned();
    let timestamp = now_in_seconds();
    let hop = 1u32;
    let nonce = new_nonce();
    let signature = sign(
        secret,
        &parts.method,
        &target,
        &bytes,
        timestamp,
        &local_instance,
        hop,
        &nonce,
    );
    let url = join_url(&base_url, &target);

    let mut headers = HeaderMap::new();
    for (name, value) in parts.headers.iter() {
        if name == HOST || name == CONTENT_LENGTH || name == CONNECTION || name == TRANSFER_ENCODING
        {
            continue;
        }
        if PEER_HEADERS
            .iter()
            .any(|from_peer| name.as_str().eq_ignore_ascii_case(from_peer))
        {
            continue;
        }
        headers.append(name.clone(), value.clone());
    }
    let mut with_peer_headers = |name: &'static str, value: String| {
        if let Ok(value) = HeaderValue::from_str(&value) {
            headers.insert(axum::http::HeaderName::from_static(name), value);
        }
    };
    with_peer_headers(INSTANCE_HEADER, local_instance.clone());
    with_peer_headers(TIMESTAMP_HEADER, timestamp.to_string());
    with_peer_headers(SIGNATURE_HEADER, signature);
    with_peer_headers(HOP_HEADER, hop.to_string());
    with_peer_headers(MAP_HEADER, cluster.config.map_fingerprint());
    with_peer_headers(NONCE_HEADER, nonce);

    let target_request = client()
        .request(parts.method.clone(), &url)
        .headers(headers)
        .body(bytes);
    let response = match tokio::time::timeout(RESPONSE_TIMEOUT, target_request.send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            tracing::warn!(
                target: "poph_server::encaminhamento",
                org = %org,
                instance = %destination,
                url = %base_url,
                error = %error,
                "encaminhamento recusado: a instância dona não respondeu"
            );
            return instance_unavailable(destination, &base_url, &error.to_string())
                .into_response();
        }
        Err(_) => {
            tracing::warn!(
                target: "poph_server::encaminhamento",
                org = %org,
                instance = %destination,
                url = %base_url,
                "encaminhamento recusado: a instância dona não respondeu no prazo"
            );
            return instance_unavailable(
                destination,
                &base_url,
                &format!("sem resposta em {} s", RESPONSE_TIMEOUT.as_secs().max(1)),
            )
            .into_response();
        }
    };

    tracing::info!(
        target: "poph_server::encaminhamento",
        org = %org,
        instance = %destination,
        method = %parts.method,
        path = %parts.uri.path(),
        status = response.status().as_u16(),
        "pedido encaminhado para a instância dona"
    );

    forwarded_response(response)
}

fn forwarded_response(response: reqwest::Response) -> Response {
    let status = response.status();
    let headers = response.headers().clone();
    let body = Body::from_stream(response.bytes_stream().map_err(std::io::Error::other));
    let mut build = Response::builder().status(status);
    for (name, value) in headers.iter() {
        if name == CONTENT_LENGTH || name == TRANSFER_ENCODING || name == CONNECTION {
            continue;
        }
        build = build.header(name, value);
    }
    build.body(body).unwrap_or_else(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "internal",
            "não consegui montar a resposta encaminhada",
        )
        .into_response()
    })
}

#[cfg(test)]
fn bytes(text: &str) -> axum::body::Bytes {
    axum::body::Bytes::from(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use poph_types::config::{ClusterConfig, ClusterInstance};

    fn secret() -> Secret {
        Secret::new("segredo-compartilhado-da-hr50b".to_owned())
    }

    fn map(local: &str) -> ClusterConfig {
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
    fn extracts_org_only_from_org_routes() {
        assert_eq!(
            org_from_path("/webhook/evolution/org_01/loja"),
            Some("org_01")
        );
        assert_eq!(
            org_from_path("/admin/v1/orgs/org_01/agents/agt_1"),
            Some("org_01")
        );
        assert_eq!(org_from_path("/admin/v1/orgs/org_01"), Some("org_01"));
        assert_eq!(org_from_path("/admin/v1/orgs"), None);
        assert_eq!(org_from_path("/admin/v1/orgs/"), None);
        assert_eq!(org_from_path("/admin/v1/cluster/reload"), None);
        assert_eq!(org_from_path("/healthz"), None);
        assert_eq!(org_from_path("/metrics"), None);
        assert_eq!(org_from_path("/openapi.json"), None);
        assert_eq!(org_from_path("/admin/v1/openapi.json"), None);
        assert_eq!(org_from_path("/orgs/org_01/agents"), None);
        assert_eq!(org_from_path("/webhook/evolution/org_01"), None);
    }

    #[test]
    fn body_limit_matches_the_route() {
        assert_eq!(
            body_limit_for_path("/webhook/evolution/org_01/loja"),
            poph_evolution::DEFAULT_BODY_CAP as usize
        );
        assert_eq!(
            body_limit_for_path("/admin/v1/orgs/org_01/backup/import"),
            LIMIT_IMPORT_BYTES as usize
        );
        assert_eq!(
            body_limit_for_path("/admin/v1/orgs/org_01/agents/agt_1/test-chat/c/messages"),
            TEST_CHAT_BODY_LIMIT
        );
        assert_eq!(
            body_limit_for_path("/admin/v1/orgs/org_01/agents"),
            DEFAULT_BODY_LIMIT
        );
    }

    #[test]
    fn join_url_keeps_prefix_without_double_slashes() {
        assert_eq!(
            join_url("http://host:7480", "/admin/v1/orgs/org_01"),
            "http://host:7480/admin/v1/orgs/org_01"
        );
        assert_eq!(
            join_url("http://host:7480/base/", "/admin/v1/orgs/org_01"),
            "http://host:7480/base/admin/v1/orgs/org_01"
        );
        assert_eq!(
            join_url("https://harness.exemplo", "webhook/evolution/org_01/loja"),
            "https://harness.exemplo/webhook/evolution/org_01/loja"
        );
        assert_eq!(
            join_url("http://host:7480", "/x?y=1"),
            "http://host:7480/x?y=1"
        );
    }

    #[test]
    fn signature_round_trips_and_covers_each_part() {
        let target = "/admin/v1/orgs/org_01/agents?limit=2";
        let body = bytes("{\"nome\":\"Loja\"}");
        let nonce = "0123456789abcdef0123456789abcdef";
        let other_nonce = "fedcba9876543210fedcba9876543210";
        let signature = sign(
            &secret(),
            &Method::POST,
            target,
            &body,
            1_800,
            "a",
            1,
            nonce,
        );
        let valid = |body: &[u8],
                     method: Method,
                     target: &str,
                     timestamp: i64,
                     instance: &str,
                     hop: u32,
                     nonce: &str| {
            verify(
                &secret(),
                &method,
                target,
                body,
                timestamp,
                instance,
                hop,
                nonce,
                &signature,
            )
        };
        assert!(valid(&body, Method::POST, target, 1_800, "a", 1, nonce));
        assert!(!valid(
            b"{\"nome\":\"Outra\"}",
            Method::POST,
            target,
            1_800,
            "a",
            1,
            nonce
        ));
        assert!(!valid(&body, Method::GET, target, 1_800, "a", 1, nonce));
        assert!(!valid(
            &body,
            Method::POST,
            "/admin/v1/orgs/org_01/agents",
            1_800,
            "a",
            1,
            nonce
        ));
        assert!(!valid(&body, Method::POST, target, 1_801, "a", 1, nonce));
        assert!(!valid(&body, Method::POST, target, 1_800, "b", 1, nonce));
        assert!(!valid(&body, Method::POST, target, 1_800, "a", 2, nonce));
        assert!(!valid(
            &body,
            Method::POST,
            target,
            1_800,
            "a",
            1,
            other_nonce
        ));
        assert!(!verify(
            &Secret::new("outro".to_owned()),
            &Method::POST,
            target,
            &body,
            1_800,
            "a",
            1,
            nonce,
            &signature
        ));
        assert!(!verify(
            &secret(),
            &Method::POST,
            target,
            &body,
            1_800,
            "a",
            1,
            nonce,
            "zz"
        ));
        assert!(!verify(
            &secret(),
            &Method::POST,
            target,
            &body,
            1_800,
            "a",
            1,
            nonce,
            "abc"
        ));
    }

    #[test]
    fn nonce_is_unique_and_well_formed() {
        let first = new_nonce();
        let second = new_nonce();
        assert_eq!(first.len(), NONCE_SIZE * 2);
        assert!(is_valid_nonce(&first));
        assert_ne!(
            first, second,
            "dois pedidos seguidos não podem ter o mesmo nonce"
        );
        assert!(!is_valid_nonce(""));
        assert!(!is_valid_nonce("abc"));
        assert!(!is_valid_nonce("zz3456789abcdef0123456789abcdef"));
        assert!(!is_valid_nonce("0123456789abcdef0123456789abcde"));
        assert!(!is_valid_nonce("0123456789abcdef0123456789abcdef0"));
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(encode_hex(&[0x00, 0x0f, 0xff]), "000fff");
        assert_eq!(decode_hex("000fff"), Some(vec![0x00, 0x0f, 0xff]));
        assert_eq!(decode_hex("000FFF"), Some(vec![0x00, 0x0f, 0xff]));
        assert_eq!(decode_hex("0"), None);
        assert_eq!(decode_hex(""), None);
        assert_eq!(decode_hex("zz"), None);
    }

    #[test]
    fn map_fingerprint_excludes_local_instance_and_covers_other_fields() {
        let a = map("a");
        let b = map("b");
        assert_eq!(a.map_fingerprint(), b.map_fingerprint());
        assert_ne!(a.fingerprint(), b.fingerprint());

        let mut with_changed_org = map("a");
        with_changed_org
            .orgs
            .insert("org_01".to_owned(), "b".to_owned());
        assert_ne!(a.map_fingerprint(), with_changed_org.map_fingerprint());
        assert!(!a.normalized_map().to_lowercase().contains("secret"));
    }
}
