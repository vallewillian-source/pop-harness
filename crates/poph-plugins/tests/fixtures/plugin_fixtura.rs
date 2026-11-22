//! Plugin de fixtura do protocolo de integração (HR-25). **Não é produto**: é o programa que os
//! testes usam para provar o contrato — handshake, evento normalizado, `SendText`, `SendFile`,
//! saúde, timeout, queda e lixo.
//!
//! Fala JSON-lines no stdin/stdout, sem dependência externa além do `serde_json` que o crate já tem.
//! O comportamento é escolhido pela configuração que vem no handshake:
//!
//! | `config.modo`                | O que faz                                                      |
//! |------------------------------|----------------------------------------------------------------|
//! | `normal` (padrão)            | responde tudo                                                  |
//! | `trava_no_handshake`         | nunca responde o handshake (para provar o prazo)               |
//! | `trava_no_comando`           | faz o handshake e nunca responde comando (para provar o prazo) |
//! | `trava_no_primeiro_comando`  | trava no primeiro comando e volta normal depois do reinício    |
//! | `cai_depois_do_handshake`    | faz o handshake e sai (para provar o reinício)                 |
//! | `cai_no_primeiro_comando`    | sai no primeiro comando e volta normal depois (idem)           |
//! | `cai_em_todo_comando`        | sai em todo comando (para provar o teto de falhas)             |
//! | `lixo`                       | imprime uma linha inválida e uma gigante, e segue normal       |
//! | `protocolo_errado`           | responde handshake com versão 999                              |
//! | `manifesto_invalido`         | responde manifesto com JSON Schema que não é objeto            |
//!
//! `config.dump_dir` faz o plugin gravar o que recebeu (`args.txt`, `env.txt`, `segredos.txt` com
//! só nomes e tamanhos, `comandos.jsonl`): é assim que o teste prova que o segredo **não** chega
//! por linha de comando nem por ambiente.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

const PROTOCOLO: u32 = 1;

fn main() {
    let mut entrada = std::io::stdin().lock();
    let mut linha = String::new();
    if entrada.read_line(&mut linha).unwrap_or(0) == 0 {
        // Sem handshake não há conversa: sai com código próprio para o supervisor registrar.
        std::process::exit(2);
    }
    let handshake: Value = match serde_json::from_str(&linha) {
        Ok(valor) => valor,
        Err(_) => std::process::exit(2),
    };
    let config = handshake.get("config").cloned().unwrap_or(Value::Null);
    let modo = config
        .get("modo")
        .and_then(Value::as_str)
        .unwrap_or("normal")
        .to_owned();
    let dump = config
        .get("dump_dir")
        .and_then(Value::as_str)
        .map(PathBuf::from);

    if let Some(dir) = &dump {
        anotar_ambiente(dir, &handshake);
    }

    if modo == "trava_no_handshake" {
        travar();
    }

    let protocolo = if modo == "protocolo_errado" {
        PROTOCOLO + 998
    } else {
        PROTOCOLO
    };
    responder(json!({
        "tipo": "handshake_ok",
        "protocolo": protocolo,
        "manifesto": manifesto(&modo),
    }));

    if modo == "cai_depois_do_handshake" {
        std::process::exit(3);
    }

    if modo == "lixo" {
        // Linha que não é JSON e linha acima do teto padrão (1 MiB): as duas têm de ser ignoradas.
        println!("isto não é json");
        println!("{}", "x".repeat(2 * 1024 * 1024));
        flush();
    }

    if config
        .get("emitir_evento")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let integracao = handshake.get("integracao").cloned().unwrap_or(Value::Null);
        responder(json!({
            "tipo": "evento",
            "evento": {
                "integracao": integracao,
                "id_externo": texto_ou(&config, "id_externo", "msg-fixtura-1"),
                "peer": texto_ou(&config, "peer", "5511900000000"),
                "texto": texto_ou(&config, "texto", "olá do plugin de fixtura"),
                "anexos": [],
                "ocorrido_em": 1_795_140_000_000i64,
            }
        }));
    }
    if let Some(texto) = config.get("emitir_estado").and_then(Value::as_str) {
        responder(json!({"tipo": "estado", "estado": {"estado": texto}}));
    }

    for linha in entrada.lines() {
        let Ok(mensagem) = linha else { break };
        let Ok(pedido) = serde_json::from_str::<Value>(&mensagem) else {
            continue;
        };
        match pedido.get("tipo").and_then(Value::as_str) {
            Some("comando") => {
                if let Some(dir) = &dump {
                    acrescentar(dir.join("comandos.jsonl"), &mensagem);
                }
                if modo == "trava_no_comando" {
                    // Fica vivo e calado: quem tem de reagir é o prazo do supervisor.
                    continue;
                }
                if modo == "trava_no_primeiro_comando" {
                    let Some(dir) = &dump else {
                        continue;
                    };
                    let marca = dir.join("travado-uma-vez");
                    if !marca.exists() {
                        let _ = std::fs::write(&marca, b"1");
                        continue;
                    }
                }
                if modo == "cai_em_todo_comando" {
                    std::process::exit(5);
                }
                if modo == "cai_no_primeiro_comando" {
                    let Some(dir) = &dump else {
                        std::process::exit(4);
                    };
                    let marca = dir.join("caido-uma-vez");
                    if !marca.exists() {
                        let _ = std::fs::write(&marca, b"1");
                        std::process::exit(4);
                    }
                }
                responder(json!({
                    "tipo": "resultado",
                    "id": pedido.get("id").cloned().unwrap_or(Value::Null),
                    "ok": true,
                }));
            }
            Some("saude") => {
                responder(json!({
                    "tipo": "saude",
                    "id": pedido.get("id").cloned().unwrap_or(Value::Null),
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

fn manifesto(modo: &str) -> Value {
    let configuracao = if modo == "manifesto_invalido" {
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
        "protocolo": PROTOCOLO,
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
        "configuracao": configuracao,
        "segredos": {"type": "object", "properties": {"token": {"type": "string"}}},
    })
}

fn texto_ou(config: &Value, chave: &str, padrao: &str) -> String {
    config
        .get(chave)
        .and_then(Value::as_str)
        .unwrap_or(padrao)
        .to_owned()
}

fn travar() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

fn responder(valor: Value) {
    println!("{valor}");
    flush();
}

fn flush() {
    let _ = std::io::stdout().flush();
}

fn anotar_ambiente(dir: &Path, handshake: &Value) {
    let _ = std::fs::create_dir_all(dir);
    let argumentos: Vec<String> = std::env::args().collect();
    let _ = std::fs::write(dir.join("args.txt"), argumentos.join("\n"));
    let ambiente: Vec<String> = std::env::vars()
        .map(|(nome, valor)| format!("{nome}={valor}"))
        .collect();
    let _ = std::fs::write(dir.join("env.txt"), ambiente.join("\n"));
    // Só nome e tamanho: o valor do segredo nunca é escrito em disco pelo plugin.
    let mut linhas = Vec::new();
    if let Some(segredos) = handshake.get("segredos").and_then(Value::as_object) {
        for (nome, valor) in segredos {
            let tamanho = valor.as_str().map(str::len).unwrap_or(0);
            linhas.push(format!("{nome}={tamanho}"));
        }
    }
    let _ = std::fs::write(dir.join("segredos.txt"), linhas.join("\n"));
}

fn acrescentar(caminho: PathBuf, conteudo: &str) {
    use std::io::Write as _;
    if let Ok(mut arquivo) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(caminho)
    {
        let _ = writeln!(arquivo, "{conteudo}");
    }
}
