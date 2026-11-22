//! Console embutido no binário (HR-38).
//!
//! O bundle do Console (SPA Vite em `console/`) entra no binário no build — ver `build.rs`. Aqui
//! ficam só as regras de entrega: arquivo exato, queda para `index.html` nas rotas do cliente e
//! nenhuma sombra sobre as rotas do servidor (`/admin/v1`, `/healthz`, `/metrics`, `/webhook`).
//!
//! O build sem bundle não quebra: o crate compila com uma página de aviso em `/`.

use axum::body::Body;
use axum::http::{HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

use crate::erro::ApiError;

/// Tabela gerada no build a partir de `console/dist` (`build.rs`).
mod gerado {
    include!(concat!(env!("OUT_DIR"), "/console_embutido.rs"));
}

/// Cabeçalhos de segurança do Console. Sem `inline` para script; estilo inline é permitido porque
/// a SofaUI e as telas usam `style` pontual.
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                   img-src 'self' data:; font-src 'self'; connect-src 'self'; \
                   frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

/// `true` quando o bundle do Console está dentro deste binário.
pub fn tem_bundle() -> bool {
    gerado::TEM_BUNDLE
}

/// Rotas que pertencem ao servidor e nunca podem cair no Console.
///
/// A comparação é por prefixo com fronteira de `/`, para que um caminho como `/metricsx` continue
/// sendo rota inexistente e não vire página do Console.
///
/// `/_matrix` é do Application Service (MX-16). Ele responde em caminhos fixos; com o módulo
/// desligado, um caminho do Matrix que não existe precisa dar `404` e não a página do Console —
/// senão o Synapse receberia `200` para um usuário ou sala que esta instância não tem.
pub fn caminho_do_servidor(caminho: &str) -> bool {
    const RAIZES: [&str; 6] = [
        "/admin",
        "/healthz",
        "/metrics",
        "/webhook",
        "/openapi.json",
        "/_matrix",
    ];
    RAIZES.iter().any(|raiz| {
        caminho == *raiz
            || caminho
                .strip_prefix(raiz)
                .is_some_and(|resto| resto.starts_with('/'))
    })
}

/// Procura o arquivo no bundle. Devolve `(mime, bytes)`.
fn procurar(caminho: &str) -> Option<(&'static str, &'static [u8])> {
    let alvo = caminho.trim_start_matches('/');
    gerado::ARQUIVOS
        .iter()
        .find(|(nome, _, _)| *nome == alvo)
        .map(|(_, mime, bytes)| (*mime, *bytes))
}

/// Último segmento do caminho, para decidir se parece um arquivo (com extensão) ou uma rota.
fn parece_arquivo(caminho: &str) -> bool {
    caminho
        .rsplit('/')
        .next()
        .is_some_and(|segmento| segmento.contains('.'))
}

fn resposta(conteudo: &'static [u8], mime: &'static str, cache: &'static str) -> Response {
    let mut resposta = Response::new(Body::from(conteudo));
    let cabecalhos = resposta.headers_mut();
    cabecalhos.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    cabecalhos.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    cabecalhos.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    cabecalhos.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    cabecalhos.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    resposta
}

/// Entrega o Console para um `GET`/`HEAD` fora das rotas do servidor.
///
/// Ordem: arquivo exato do bundle → `index.html` para rotas do cliente → 404 quando o caminho
/// parece um arquivo (asset ausente não deve virar HTML, senão o navegador erra o tipo).
pub fn servir(uri: &Uri, metodo: &Method) -> Response {
    let caminho = uri.path();
    if caminho_do_servidor(caminho) {
        return ApiError::novo(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("a rota `{caminho}` não existe"),
        )
        .com_detalhes(serde_json::json!({ "path": caminho }))
        .into_response();
    }

    let (mime, bytes, cache) = if let Some((mime, bytes)) = procurar(caminho) {
        // Os nomes do Vite levam hash: podem ser guardados para sempre.
        let cache = if caminho.starts_with("/assets/") || caminho.starts_with("/res/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        (mime, bytes, cache)
    } else if parece_arquivo(caminho) {
        return ApiError::novo(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("o arquivo `{caminho}` não existe no Console"),
        )
        .into_response();
    } else {
        match procurar("/index.html") {
            Some((mime, bytes)) => (mime, bytes, "no-cache"),
            None => {
                return ApiError::novo(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "este binário não tem o Console compilado",
                )
                .into_response();
            }
        }
    };

    let mut resposta = resposta(bytes, mime, cache);
    if metodo == Method::HEAD {
        *resposta.body_mut() = Body::empty();
    }
    resposta
}
