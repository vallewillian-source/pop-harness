use std::io::{BufRead, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use poph_types::{
    Capabilities, Health, HostMessage, InboundEvent, IntegrationId, JsonSchema, Manifest,
    OutboundCommand, PROTOCOL_VERSION, PluginMessage, Presence, SendText,
};

fn main() {
    let exit_code = serve();
    std::process::exit(exit_code);
}

fn serve() -> i32 {
    let input = std::io::stdin();
    let mut lines = input.lock().lines();
    let mut echo = Echo::default();

    loop {
        let Some(line) = lines.next() else {
            return 0;
        };
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("eco: erro ao ler a entrada: {error}");
                return 1;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let message: HostMessage = match serde_json::from_str(&line) {
            Ok(message) => message,
            Err(error) => {
                eprintln!("eco: linha fora do protocolo ignorada: {error}");
                continue;
            }
        };
        match message {
            HostMessage::Handshake(handshake) => {
                if handshake.protocol != PROTOCOL_VERSION {
                    respond(&PluginMessage::Error {
                        code: "protocolo_incompativel".to_owned(),
                        message: format!(
                            "o eco fala a versão {PROTOCOL_VERSION} e o host falou {}",
                            handshake.protocol
                        ),
                        transient: false,
                    });
                    return 2;
                }
                echo.integration = Some(handshake.integration);
                let _ = &handshake.secrets;
                respond(&PluginMessage::HandshakeOk {
                    protocolo: PROTOCOL_VERSION,
                    manifest: manifest(),
                });
            }
            HostMessage::Command { id, command } => {
                let Some(integration) = echo.integration else {
                    respond(&PluginMessage::Result {
                        id,
                        ok: false,
                        error: Some(poph_types::CommandError {
                            code: "sem_handshake".to_owned(),
                            message: "recebi comando antes do handshake".to_owned(),
                            transient: false,
                        }),
                    });
                    continue;
                };
                let mut event = None;
                match command {
                    OutboundCommand::Text(text) => {
                        echo.messages += 1;
                        event = Some(echo.event_for_text(integration, &text));
                    }
                    OutboundCommand::File(file) => {
                        eprintln!(
                            "eco: arquivo confirmado ({} bytes): {}",
                            file.size_bytes, file.name
                        );
                    }
                    OutboundCommand::Presence(presence) => {
                        if presence.state == Presence::Typing {
                            eprintln!("eco: digitando para {}", presence.peer);
                        }
                    }
                }
                respond(&PluginMessage::Result {
                    id,
                    ok: true,
                    error: None,
                });
                if let Some(event) = event {
                    respond(&PluginMessage::Evento { evento: event });
                }
            }
            HostMessage::Health { id } => {
                respond(&PluginMessage::Health {
                    id,
                    health: Health::ok(format!(
                        "eco {} — {} mensagem(ns) tratada(s)",
                        env!("CARGO_PKG_VERSION"),
                        echo.messages
                    )),
                });
            }
            HostMessage::Shutdown => return 0,
        }
    }
}

#[derive(Default)]
struct Echo {
    integration: Option<IntegrationId>,
    messages: u64,
}

impl Echo {
    fn event_for_text(&self, integration: IntegrationId, text: &SendText) -> InboundEvent {
        let reversed: String = text.text.chars().rev().collect();
        InboundEvent {
            integration,
            id_externo: format!("eco-{}", text.idempotency),
            peer: text.peer.clone(),
            remetente: None,
            text: reversed,
            attachments: Vec::new(),
            occurred_at: now_ms(),
            metadata: serde_json::Value::Null,
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

fn manifest() -> Manifest {
    Manifest {
        name: "Eco (exemplo)".to_owned(),
        kind: "eco".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        protocol: PROTOCOL_VERSION,
        capabilities: Capabilities {
            text: true,
            file_out: true,
            typing: true,
            ..Capabilities::default()
        },
        limits: poph_types::ChannelLimits::default(),
        configuration: JsonSchema(serde_json::json!({
            "type": "object",
            "properties": {
                "modo": {
                    "type": "string",
                    "enum": ["inverter"],
                    "default": "inverter",
                    "description": "O eco só tem o modo que inverte o texto."
                },
                "remetente": {
                    "type": "string",
                    "minLength": 1,
                    "title": "Nome do remetente",
                    "description": "Nome que o eco assina nas respostas (obrigatório: é o que o operador vê no canal)."
                }
            },
            "required": ["remetente"]
        })),
        secrets: JsonSchema(serde_json::json!({
            "type": "object",
            "properties": {
                "token": {
                    "type": "string",
                    "title": "Token do canal",
                    "description": "Credencial que o eco receberia do canal no handshake."
                }
            },
            "required": ["token"]
        })),
    }
}

fn respond(message: &PluginMessage) {
    let Ok(line) = serde_json::to_string(message) else {
        return;
    };
    let mut output = std::io::stdout().lock();
    if writeln!(output, "{line}").is_err() {
        return;
    }
    let _ = output.flush();
}
