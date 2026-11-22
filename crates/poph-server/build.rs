//! Build do `poph-server`: embute o bundle do Console (HR-38) no binário.
//!
//! O Console é uma SPA Vite em `console/` (irmã do crate). O build do Rust não roda Node: ele
//! lê `console/dist` se existir e gera uma tabela de `include_bytes!`. Sem o bundle — CI offline,
//! clone limpo, build de quem só quer a API — o crate compila normalmente e serve uma página
//! mínima explicando como gerar o Console.
//!
//! Assim o `cargo build` nunca depende de `pnpm`, e o binário de release leva o Console dentro.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    let raiz = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let dist = raiz.join("../../console/dist");
    let destino =
        PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("console_embutido.rs");

    println!("cargo:rerun-if-changed={}", dist.display());
    println!("cargo:rerun-if-changed=build.rs");

    let mut arquivos = Vec::new();
    if dist.is_dir() {
        coletar(&dist, &dist, &mut arquivos);
    }
    // Ordem estável: o mesmo bundle gera o mesmo código, e o build fica reproduzível.
    arquivos.sort_by(|a, b| a.0.cmp(&b.0));

    let mut codigo = String::new();
    codigo.push_str("// Gerado por build.rs a partir de console/dist. Não edite.\n");

    if arquivos.is_empty() {
        let aviso = destino.with_file_name("console_ausente.html");
        std::fs::write(&aviso, AVISO_SEM_BUNDLE).expect("gravar a página de aviso do Console");
        let _ = writeln!(
            codigo,
            "/// `false` quando o build não encontrou `console/dist`.\npub const TEM_BUNDLE: bool = false;\n\
             pub static ARQUIVOS: &[(&str, &str, &[u8])] = &[\n    (\"index.html\", \"text/html; charset=utf-8\", include_bytes!({caminho:?})),\n];",
            caminho = aviso.to_string_lossy()
        );
    } else {
        codigo.push_str("/// `true` quando o build encontrou `console/dist`.\npub const TEM_BUNDLE: bool = true;\n");
        codigo.push_str("pub static ARQUIVOS: &[(&str, &str, &[u8])] = &[\n");
        for (relativo, absoluto) in &arquivos {
            let _ = writeln!(
                codigo,
                "    ({relativo:?}, {:?}, include_bytes!({:?})),",
                mime(relativo),
                absoluto.to_string_lossy()
            );
        }
        codigo.push_str("];\n");
    }

    std::fs::write(&destino, codigo).expect("gravar console_embutido.rs");
}

const AVISO_SEM_BUNDLE: &str = r#"<!doctype html>
<html lang="pt-BR">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>POP Harness — Console não compilado</title>
<style>
  body { margin: 0; padding: 48px; background: #151719; color: #eef1f4;
         font-family: "IBM Plex Sans", system-ui, sans-serif; line-height: 1.6; }
  h1 { font-size: 20px; font-weight: 500; margin: 0 0 12px; }
  p { color: #c0c7ce; max-width: 60ch; }
  code { font-family: "IBM Plex Mono", monospace; background: #212529; padding: 2px 6px;
         border-radius: 4px; color: #f5a623; }
</style>
</head>
<body>
<h1>Console não compilado neste binário</h1>
<p>O servidor está no ar e a API administrativa responde normalmente em
<code>/admin/v1</code>. O bundle do Console não estava presente quando este binário foi gerado.</p>
<p>Para incluir o Console, gere o bundle e recompile:</p>
<p><code>cd apps/pop-harness/console &amp;&amp; pnpm install &amp;&amp; pnpm build</code><br>
<code>cargo build -p poph --release</code></p>
</body>
</html>
"#;

/// Percorre `dist` em ordem de caminho e coleta `(caminho relativo, caminho absoluto)`.
fn coletar(base: &Path, pasta: &Path, saida: &mut Vec<(String, PathBuf)>) {
    let Ok(entradas) = std::fs::read_dir(pasta) else {
        return;
    };
    let mut itens: Vec<PathBuf> = entradas
        .filter_map(|entrada| entrada.ok().map(|e| e.path()))
        .collect();
    itens.sort();
    for caminho in itens {
        if caminho.is_dir() {
            coletar(base, &caminho, saida);
            continue;
        }
        let Ok(relativo) = caminho.strip_prefix(base) else {
            continue;
        };
        let relativo = relativo.to_string_lossy().replace('\\', "/");
        println!("cargo:rerun-if-changed={}", caminho.display());
        saida.push((relativo, caminho));
    }
}

/// Tipo do conteúdo pelo sufixo. Lista fechada: o bundle do Vite só produz estes.
fn mime(caminho: &str) -> &'static str {
    let extensao = caminho
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match extensao.as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}
