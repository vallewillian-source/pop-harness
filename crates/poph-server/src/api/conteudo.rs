//! Entrega do conteúdo de arquivos pela API administrativa (HR-46b).
//!
//! Duas rotas baixam bytes: a biblioteca do agente (`…/files/{file_id}/content`) e o anexo de uma
//! mensagem do chat de teste (`…/messages/{message_id}/attachments/{attachment_id}/content`).
//! As duas passam por aqui, para a resposta ter sempre o mesmo formato e as mesmas defesas:
//!
//! - **Leitura só pelo [`poph_files::blob::ler`]**: o hash vindo do banco é validado (64 hex
//!   minúsculos) antes de tocar a pasta, e o tamanho é conferido. O handler nunca monta caminho.
//! - **Sem caminho interno na resposta**: blob ausente, hash inválido ou blob acima do limite
//!   viram `404 not_found` com mensagem genérica; o motivo real fica só no log.
//! - **Cabeçalhos**: `Content-Type` do MIME gravado, `Content-Disposition` com o nome sanitizado
//!   (ASCII + `filename*` para acento), `Content-Length`, `ETag`/`X-Content-SHA256` com o hash e
//!   `X-Content-Type-Options: nosniff` com CSP restritiva — o navegador não executa nada do
//!   arquivo, mesmo que o tipo gravado mude.
//!
//! O teto de leitura é o do recebimento (`ConfigArquivos::limite_bytes`, 16 MB por padrão): todo
//! arquivo aceito cabe, e um arquivo maior que o teto configurado não é servido.

use std::path::Path;

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use poph_files::FileConfig;
use poph_types::OrgId;

use crate::erro::ApiError;

/// Cabeçalho com o SHA-256 do conteúdo entregue.
pub const CABECALHO_SHA256: &str = "x-content-sha256";
/// Teto do nome no `Content-Disposition` (o mesmo teto do nome na biblioteca).
const LIMITE_NOME: usize = 200;

/// Teto de leitura do blob ao entregar o conteúdo.
pub fn limite_de_leitura(pasta_blobs: &Path) -> u64 {
    FileConfig::default(pasta_blobs).byte_limit
}

/// Bytes do blob da org. Qualquer falha de leitura (blob ausente, hash inválido, acima do teto)
/// vira `404` genérico: quem não tem o arquivo não descobre se ele existe nem onde está.
pub fn ler_blob(pasta_blobs: &Path, org: OrgId, sha256: &str) -> Result<Vec<u8>, ApiError> {
    poph_files::blob::read(pasta_blobs, org, sha256, limite_de_leitura(pasta_blobs)).map_err(
        |erro| {
            // O motivo do `ErroArquivo` já é escrito sem caminho e sem conteúdo.
            tracing::warn!(org = %org, motivo = %erro, "conteúdo de arquivo indisponível");
            ApiError::novo(
                StatusCode::NOT_FOUND,
                "not_found",
                "o conteúdo deste arquivo não está disponível",
            )
        },
    )
}

/// Resposta com os bytes do arquivo e os cabeçalhos combinados.
///
/// `inline` só para imagem e PDF (o Console mostra no lugar); o resto sai como `attachment`, para
/// o navegador não tentar abrir um formato que ele não conhece. A CSP e o `nosniff` valem para
/// todas as respostas.
pub fn resposta(bytes: Vec<u8>, mime: &str, nome: &str, sha256: &str) -> Response {
    let disposicao = disposicao(mime, nome);
    let tamanho = bytes.len();
    let mut resposta = (StatusCode::OK, bytes).into_response();
    let cabecalhos = resposta.headers_mut();

    let mime = HeaderValue::from_str(mime)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    cabecalhos.insert(header::CONTENT_TYPE, mime);
    cabecalhos.insert(header::CONTENT_DISPOSITION, disposicao);
    if let Ok(valor) = HeaderValue::from_str(&tamanho.to_string()) {
        cabecalhos.insert(header::CONTENT_LENGTH, valor);
    }
    cabecalhos.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    cabecalhos.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    cabecalhos.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=0"),
    );
    // O hash vem do banco e já passou pelo `blob::ler` (64 hex minúsculos), mas o cabeçalho é
    // montado com `from_str` e ignorado se não for valor válido — nunca derruba a entrega.
    if let Ok(valor) = HeaderValue::from_str(&format!("\"{sha256}\"")) {
        cabecalhos.insert(header::ETAG, valor);
    }
    if let Ok(valor) = HeaderValue::from_str(sha256) {
        cabecalhos.insert(CABECALHO_SHA256, valor);
    }
    resposta
}

/// `Content-Disposition` com nome ASCII seguro e `filename*` em UTF-8 (RFC 5987).
fn disposicao(mime: &str, nome: &str) -> HeaderValue {
    let inline = mime.starts_with("image/") || mime == "application/pdf";
    let tipo = if inline { "inline" } else { "attachment" };
    let (ascii, utf8) = nome_para_cabecalho(nome);
    HeaderValue::from_str(&format!(
        "{tipo}; filename=\"{ascii}\"; filename*=UTF-8''{utf8}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}

/// Nome pronto para o cabeçalho: `(ascii, utf8_percent_encoded)`.
///
/// O nome vem do banco (o cliente escolheu), então pode ter aspas, quebra de linha, caminho e
/// acento. Na parte ASCII, só letras, números e `. - _ ( )` ficam; o resto vira `_` (aspas e
/// `\r`/`\n` não podem existir num valor de cabeçalho). Na parte UTF-8 (RFC 5987) o nome vai
/// inteiro, com acento, codificado em percentual. Nome vazio vira `arquivo`.
fn nome_para_cabecalho(nome: &str) -> (String, String) {
    let cortado: String = nome
        .chars()
        .filter(|c| !c.is_control())
        .take(LIMITE_NOME)
        .collect();
    let ascii: String = cortado
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
    let utf8 = percent_encoding(cortado.trim());
    let utf8 = if utf8.is_empty() {
        "arquivo".to_owned()
    } else {
        utf8
    };
    (ascii, utf8)
}

/// Codificação percentual do RFC 5987: só os caracteres "attr-char" ficam como estão.
fn percent_encoding(nome: &str) -> String {
    let mut saida = String::with_capacity(nome.len());
    for byte in nome.trim().as_bytes() {
        let attr_char = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'!' | b'#' | b'$' | b'&' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
            );
        if attr_char {
            saida.push(*byte as char);
        } else {
            saida.push_str(&format!("%{byte:02X}"));
        }
    }
    saida
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn nome_com_aspas_e_quebra_nao_vaza_para_o_cabecalho() {
        let (ascii, utf8) = nome_para_cabecalho("relatório \"final\"\r\n.zip");
        assert!(!ascii.contains('"'), "{ascii}");
        assert!(!ascii.contains('\n') && !ascii.contains('\r'), "{ascii}");
        assert!(ascii.starts_with("relat_rio"), "{ascii}");
        assert!(!utf8.contains('"') && !utf8.contains('\n'), "{utf8}");
        let valor = format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{utf8}");
        assert!(HeaderValue::from_str(&valor).is_ok(), "cabeçalho válido");
    }

    #[test]
    fn nome_vazio_vira_arquivo() {
        assert_eq!(nome_para_cabecalho("").0, "arquivo");
        assert_eq!(nome_para_cabecalho("...").0, "arquivo");
        let (ascii, utf8) = nome_para_cabecalho("../../etc/passwd");
        assert!(
            !ascii.contains('/') && !utf8.contains('/'),
            "{ascii} / {utf8}"
        );
    }

    #[test]
    fn nome_com_acento_ganha_percent_encoding() {
        let (_, utf8) = nome_para_cabecalho("procedimento técnico.pdf");
        assert_eq!(utf8, "procedimento%20t%C3%A9cnico.pdf");
    }

    #[test]
    fn imagem_e_pdf_abrem_no_lugar_e_o_resto_baixa() {
        for (mime, esperado) in [
            ("image/png", "inline"),
            ("application/pdf", "inline"),
            (
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "attachment",
            ),
            ("application/octet-stream", "attachment"),
        ] {
            let valor = disposicao(mime, "arquivo");
            let texto = valor.to_str().unwrap();
            assert!(texto.starts_with(esperado), "{mime}: {texto}");
        }
    }
}
