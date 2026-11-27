use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    let crate_root =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let dist = crate_root.join("../../console/dist");
    let output_path =
        PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("console_embutido.rs");

    println!("cargo:rerun-if-changed={}", dist.display());
    println!("cargo:rerun-if-changed=build.rs");

    let mut files = Vec::new();
    if dist.is_dir() {
        collect_files(&dist, &dist, &mut files);
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut source = String::new();

    if files.is_empty() {
        let notice_path = output_path.with_file_name("console_ausente.html");
        std::fs::write(&notice_path, MISSING_BUNDLE_PAGE)
            .expect("gravar a página de aviso do Console");
        let _ = writeln!(
            source,
            "pub const HAS_BUNDLE: bool = false;\n\
             pub static FILES: &[(&str, &str, &[u8])] = &[\n    (\"index.html\", \"text/html; charset=utf-8\", include_bytes!({path:?})),\n];",
            path = notice_path.to_string_lossy()
        );
    } else {
        source.push_str("pub const HAS_BUNDLE: bool = true;\n");
        source.push_str("pub static FILES: &[(&str, &str, &[u8])] = &[\n");
        for (relative, absolute) in &files {
            let _ = writeln!(
                source,
                "    ({relative:?}, {:?}, include_bytes!({:?})),",
                mime(relative),
                absolute.to_string_lossy()
            );
        }
        source.push_str("];\n");
    }

    std::fs::write(&output_path, source).expect("gravar console_embutido.rs");
}

const MISSING_BUNDLE_PAGE: &str = r#"<!doctype html>
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

fn collect_files(base: &Path, directory: &Path, files: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_files(base, &path, files);
            continue;
        }
        let Ok(relative) = path.strip_prefix(base) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        println!("cargo:rerun-if-changed={}", path.display());
        files.push((relative, path));
    }
}

fn mime(path: &str) -> &'static str {
    let extension = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match extension.as_str() {
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
