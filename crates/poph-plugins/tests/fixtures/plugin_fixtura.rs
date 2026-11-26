use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

const PROTOCOL: u32 = 1;

fn main() {
    let mut input = std::io::stdin().lock();
    let mut line = String::new();
    if input.read_line(&mut line).unwrap_or(0) == 0 {
        std::process::exit(2);
    }
    let handshake: Value = match serde_json::from_str(&line) {
        Ok(value) => value,
        Err(_) => std::process::exit(2),
    };
    let config = handshake.get("config").cloned().unwrap_or(Value::Null);
    let mode = config
        .get("modo")
        .and_then(Value::as_str)
        .unwrap_or("normal")
        .to_owned();
    let dump = config
        .get("dump_dir")
        .and_then(Value::as_str)
        .map(PathBuf::from);

    if let Some(dir) = &dump {
        record_environment(dir, &handshake);
    }

    if mode == "trava_no_handshake" {
        hang();
    }

    let protocol = if mode == "protocolo_errado" {
        PROTOCOL + 998
    } else {
        PROTOCOL
    };
    respond(json!({
        "tipo": "handshake_ok",
        "protocolo": protocol,
        "manifesto": manifest(&mode),
    }));

    if mode == "cai_depois_do_handshake" {
        std::process::exit(3);
    }

    if mode == "lixo" {
        println!("isto não é json");
        println!("{}", "x".repeat(2 * 1024 * 1024));
        flush();
    }

    if config
        .get("emitir_evento")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let integration = handshake.get("integracao").cloned().unwrap_or(Value::Null);
        respond(json!({
            "tipo": "evento",
            "evento": {
                "integracao": integration,
                "id_externo": text_or(&config, "id_externo", "msg-fixtura-1"),
                "peer": text_or(&config, "peer", "5511900000000"),
                "texto": text_or(&config, "texto", "olá do plugin de fixtura"),
                "anexos": [],
                "ocorrido_em": 1_795_140_000_000i64,
            }
        }));
    }
    if let Some(texto) = config.get("emitir_estado").and_then(Value::as_str) {
        respond(json!({"tipo": "estado", "estado": {"estado": texto}}));
    }

    for line in input.lines() {
        let Ok(mensagem) = line else { break };
        let Ok(request) = serde_json::from_str::<Value>(&mensagem) else {
            continue;
        };
        match request.get("tipo").and_then(Value::as_str) {
            Some("comando") => {
                if let Some(dir) = &dump {
                    append_line(dir.join("comandos.jsonl"), &mensagem);
                }
                if mode == "trava_no_comando" {
                    continue;
                }
                if mode == "trava_no_primeiro_comando" {
                    let Some(dir) = &dump else {
                        continue;
                    };
                    let marker = dir.join("travado-uma-vez");
                    if !marker.exists() {
                        let _ = std::fs::write(&marker, b"1");
                        continue;
                    }
                }
                if mode == "cai_em_todo_comando" {
                    std::process::exit(5);
                }
                if mode == "cai_no_primeiro_comando" {
                    let Some(dir) = &dump else {
                        std::process::exit(4);
                    };
                    let marker = dir.join("caido-uma-vez");
                    if !marker.exists() {
                        let _ = std::fs::write(&marker, b"1");
                        std::process::exit(4);
                    }
                }
                respond(json!({
                    "tipo": "resultado",
                    "id": request.get("id").cloned().unwrap_or(Value::Null),
                    "ok": true,
                }));
            }
            Some("saude") => {
                respond(json!({
                    "tipo": "saude",
                    "id": request.get("id").cloned().unwrap_or(Value::Null),
                    "saude": {"ok": true, "detalhe": "fixtura"},
                }));
            }
            Some("encerrar") => {
                if let Some(dir) = &dump {
                    let _ = std::fs::write(dir.join("saiu.txt"), b"sim");
                }
                flush();
                std::process::exit(0);
            }
            _ => {}
        }
    }
}

fn manifest(mode: &str) -> Value {
    let configuration = if mode == "manifesto_invalido" {
        json!({"type": "array"})
    } else {
        json!({
            "type": "object",
            "properties": {
                "modo": {"type": "string"},
                "dump_dir": {"type": "string"},
                "emitir_evento": {"type": "boolean"},
                "peer": {"type": "string"},
                "texto": {"type": "string"},
            }
        })
    };
    json!({
        "nome": "Plugin de fixtura",
        "kind": "fixtura",
        "versao": "1.0.0",
        "protocolo": PROTOCOL,
        "capacidades": {
            "text": true,
            "image_in": true,
            "file_in": true,
            "file_out": true,
            "typing": true,
            "read_receipt": false,
        },
        "limites": {
            "max_anexo_bytes": 16_777_216u64,
            "max_texto_bytes": 65_536u64,
            "tipos_aceitos": ["image/png", "application/pdf"],
        },
        "configuracao": configuration,
        "segredos": {"type": "object", "properties": {"token": {"type": "string"}}},
    })
}

fn text_or(config: &Value, key: &str, fallback: &str) -> String {
    config
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_owned()
}

fn hang() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

fn respond(value: Value) {
    println!("{value}");
    flush();
}

fn flush() {
    let _ = std::io::stdout().flush();
}

fn record_environment(dir: &Path, handshake: &Value) {
    let _ = std::fs::create_dir_all(dir);
    let arguments: Vec<String> = std::env::args().collect();
    let _ = std::fs::write(dir.join("args.txt"), arguments.join("\n"));
    let environment: Vec<String> = std::env::vars()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    let _ = std::fs::write(dir.join("env.txt"), environment.join("\n"));
    let mut lines = Vec::new();
    if let Some(secrets) = handshake.get("segredos").and_then(Value::as_object) {
        for (name, value) in secrets {
            let size = value.as_str().map(str::len).unwrap_or(0);
            lines.push(format!("{name}={size}"));
        }
    }
    let _ = std::fs::write(dir.join("segredos.txt"), lines.join("\n"));
}

fn append_line(path: PathBuf, content: &str) {
    use std::io::Write as _;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{content}");
    }
}
