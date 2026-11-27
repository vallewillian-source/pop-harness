use axum::body::Body;
use axum::http::{HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

use crate::error::ApiError;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/console_embutido.rs"));
}

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                   img-src 'self' data:; font-src 'self'; connect-src 'self'; \
                   frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

pub fn has_bundle() -> bool {
    embedded::TEM_BUNDLE
}

pub fn server_path(path: &str) -> bool {
    const ROOTS: [&str; 6] = [
        "/admin",
        "/healthz",
        "/metrics",
        "/webhook",
        "/openapi.json",
        "/_matrix",
    ];
    ROOTS.iter().any(|root| {
        path == *root
            || path
                .strip_prefix(root)
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

fn procurar(path: &str) -> Option<(&'static str, &'static [u8])> {
    let target = path.trim_start_matches('/');
    embedded::ARQUIVOS
        .iter()
        .find(|(nome, _, _)| *nome == target)
        .map(|(_, mime, bytes)| (*mime, *bytes))
}

fn looks_like_file(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|segment| segment.contains('.'))
}

fn response(content: &'static [u8], mime: &'static str, cache: &'static str) -> Response {
    let mut response = Response::new(Body::from(content));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

pub fn serve(uri: &Uri, method: &Method) -> Response {
    let path = uri.path();
    if server_path(path) {
        return ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("a rota `{path}` não existe"),
        )
        .with_details(serde_json::json!({ "path": path }))
        .into_response();
    }

    let (mime, bytes, cache) = if let Some((mime, bytes)) = procurar(path) {
        let cache = if path.starts_with("/assets/") || path.starts_with("/res/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        (mime, bytes, cache)
    } else if looks_like_file(path) {
        return ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("o arquivo `{path}` não existe no Console"),
        )
        .into_response();
    } else {
        match procurar("/index.html") {
            Some((mime, bytes)) => (mime, bytes, "no-cache"),
            None => {
                return ApiError::new(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "este binário não tem o Console compilado",
                )
                .into_response();
            }
        }
    };

    let mut response = response(bytes, mime, cache);
    if method == Method::HEAD {
        *response.body_mut() = Body::empty();
    }
    response
}
