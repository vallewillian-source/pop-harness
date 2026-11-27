use std::path::Path;

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use poph_files::FileConfig;
use poph_types::OrgId;

use crate::error::ApiError;

pub const HEADER_SHA: &str = "x-content-sha256";
const LIMIT_NAME: usize = 200;

pub fn read_limit(folder_blobs: &Path) -> u64 {
    FileConfig::default(folder_blobs).byte_limit
}

pub fn read_blob(folder_blobs: &Path, org: OrgId, sha256: &str) -> Result<Vec<u8>, ApiError> {
    poph_files::blob::read(folder_blobs, org, sha256, read_limit(folder_blobs)).map_err(|error| {
        tracing::warn!(org = %org, reason = %error, "conteúdo de arquivo indisponível");
        ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "o conteúdo deste arquivo não está disponível",
        )
    })
}

pub fn response(bytes: Vec<u8>, mime: &str, name: &str, sha256: &str) -> Response {
    let disposition = disposition(mime, name);
    let size = bytes.len();
    let mut response = (StatusCode::OK, bytes).into_response();
    let header = response.headers_mut();

    let mime = HeaderValue::from_str(mime)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    header.insert(header::CONTENT_TYPE, mime);
    header.insert(header::CONTENT_DISPOSITION, disposition);
    if let Ok(value) = HeaderValue::from_str(&size.to_string()) {
        header.insert(header::CONTENT_LENGTH, value);
    }
    header.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    header.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    header.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=0"),
    );
    if let Ok(value) = HeaderValue::from_str(&format!("\"{sha256}\"")) {
        header.insert(header::ETAG, value);
    }
    if let Ok(value) = HeaderValue::from_str(sha256) {
        header.insert(HEADER_SHA, value);
    }
    response
}

fn disposition(mime: &str, name: &str) -> HeaderValue {
    let inline = mime.starts_with("image/") || mime == "application/pdf";
    let disposition_type = if inline { "inline" } else { "attachment" };
    let (ascii, utf8) = header_name(name);
    HeaderValue::from_str(&format!(
        "{disposition_type}; filename=\"{ascii}\"; filename*=UTF-8''{utf8}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}

fn header_name(name: &str) -> (String, String) {
    let cut: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(LIMIT_NAME)
        .collect();
    let ascii: String = cut
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ' | '(' | ')') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let ascii = ascii.trim().trim_matches('.').trim().to_owned();
    let ascii = if ascii.is_empty() {
        "arquivo".to_owned()
    } else {
        ascii
    };
    let utf8 = percent_encoding(cut.trim());
    let utf8 = if utf8.is_empty() {
        "arquivo".to_owned()
    } else {
        utf8
    };
    (ascii, utf8)
}

fn percent_encoding(name: &str) -> String {
    let mut output = String::with_capacity(name.len());
    for byte in name.trim().as_bytes() {
        let attr_char = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'!' | b'#' | b'$' | b'&' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
            );
        if attr_char {
            output.push(*byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_name_with_newline_does_not_leak_into_header() {
        let (ascii, utf8) = header_name("relatório \"final\"\r\n.zip");
        assert!(!ascii.contains('"'), "{ascii}");
        assert!(!ascii.contains('\n') && !ascii.contains('\r'), "{ascii}");
        assert!(ascii.starts_with("relat_rio"), "{ascii}");
        assert!(!utf8.contains('"') && !utf8.contains('\n'), "{utf8}");
        let value = format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{utf8}");
        assert!(HeaderValue::from_str(&value).is_ok(), "cabeçalho válido");
    }

    #[test]
    fn empty_name_becomes_file() {
        assert_eq!(header_name("").0, "arquivo");
        assert_eq!(header_name("...").0, "arquivo");
        let (ascii, utf8) = header_name("../../etc/passwd");
        assert!(
            !ascii.contains('/') && !utf8.contains('/'),
            "{ascii} / {utf8}"
        );
    }

    #[test]
    fn accented_name_gets_percent_encoding() {
        let (_, utf8) = header_name("procedimento técnico.pdf");
        assert_eq!(utf8, "procedimento%20t%C3%A9cnico.pdf");
    }

    #[test]
    fn image_and_pdf_open_in_place_rest_downloads() {
        for (mime, expected) in [
            ("image/png", "inline"),
            ("application/pdf", "inline"),
            (
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "attachment",
            ),
            ("application/octet-stream", "attachment"),
        ] {
            let value = disposition(mime, "arquivo");
            let text = value.to_str().unwrap();
            assert!(text.starts_with(expected), "{mime}: {text}");
        }
    }
}
