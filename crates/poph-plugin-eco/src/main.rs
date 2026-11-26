//! Plugin de exemplo (eco) do POP Harness — HR-27.
//!
//! Implementa o protocolo de `docs/PLUGIN_PROTOCOLO.md` em processo externo: lê uma mensagem JSON
//! por linha na entrada padrão e responde na saída padrão. O que ele faz:
//!
//! - responde o `handshake` com o manifesto (nome, tipo, versão, capacidades, esquema de
//!   configuração);
//! - a cada `enviar_texto` confirma a entrega (`resultado`) **e** devolve um `evento` de entrada com
//!   o texto **invertido** — é assim que se prova, sem WhatsApp, que o caminho de volta
//!   (plugin → evento → `inbox`) funciona;
//! - `enviar_arquivo` e `definir_presenca` são confirmados sem efeito;
//! - `saude` responde `ok` com o número de mensagens tratadas;
//! - `encerrar` termina o processo.
//!
//! O `id_externo` do evento sai da chave de idempotência do comando (`eco-<turno:seq>`): se o
//! Harness reenviar o mesmo comando depois de uma queda, o evento chega com o mesmo id e a `inbox`
//! descarta a repetição. É a regra do protocolo: a entrega é "pelo menos uma vez", quem deduplica é
//! a `inbox`.
//!
//! Este binário **não** entra no pacote de distribuição: o `poph` não depende deste crate.

use std::io::{BufRead, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use poph_types::{
    Capabilities, Health, HostMessage, InboundEvent, IntegrationId, JsonSchema, Manifest,
    OutboundCommand, PROTOCOL_VERSION, PluginMessage, Presence, SendText,
};

fn main() {
    let codigo = atender();
    std::process::exit(codigo);
}

fn atender() -> i32 {
    let entrada = std::io::stdin();
    let mut linhas = entrada.lock().lines();
    let mut eco = Eco::default();

    loop {
        let Some(linha) = linhas.next() else {
            // O Harness fechou a entrada: nada mais a fazer.
            return 0;
        };
        let linha = match linha {
            Ok(linha) => linha,
            Err(erro) => {
                eprintln!("eco: erro ao ler a entrada: {erro}");
                return 1;
            }
        };
        if linha.trim().is_empty() {
            continue;
        }
        let mensagem: HostMessage = match serde_json::from_str(&linha) {
            Ok(mensagem) => mensagem,
            Err(erro) => {
                // Regra do protocolo: linha fora do contrato é lixo, e lixo se ignora.
                eprintln!("eco: linha fora do protocolo ignorada: {erro}");
                continue;
            }
        };
        match mensagem {
            HostMessage::Handshake(handshake) => {
                if handshake.protocol != PROTOCOL_VERSION {
                    responder(&PluginMessage::Error {
                        code: "protocolo_incompativel".to_owned(),
                        message: format!(
                            "o eco fala a versão {PROTOCOL_VERSION} e o host falou {}",
                            handshake.protocol
                        ),
                        transient: false,
                    });
                    return 2;
                }
                eco.integracao = Some(handshake.integration);
                // Os segredos do handshake não são usados aqui (o eco não tem canal de verdade).
                // Um plugin real usa este ponto para autenticar no canal.
                let _ = &handshake.secrets;
                responder(&PluginMessage::HandshakeOk {
                    protocolo: PROTOCOL_VERSION,
                    manifest: manifesto(),
                });
            }
            HostMessage::Command { id, command } => {
                let Some(integracao) = eco.integracao else {
                    responder(&PluginMessage::Result {
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
                let mut evento = None;
                match command {
                    OutboundCommand::Text(texto) => {
                        eco.mensagens += 1;
                        evento = Some(eco.evento_do_texto(integracao, &texto));
                    }
                    OutboundCommand::File(arquivo) => {
                        // Um plugin real subiria o arquivo para o canal. O eco só confirma.
                        eprintln!(
                            "eco: arquivo confirmado ({} bytes): {}",
                            arquivo.size_bytes, arquivo.name
                        );
                    }
                    OutboundCommand::Presence(presenca) => {
                        if presenca.state == Presence::Typing {
                            eprintln!("eco: digitando para {}", presenca.peer);
                        }
                    }
                }
                responder(&PluginMessage::Result {
                    id,
                    ok: true,
                    error: None,
                });
                if let Some(evento) = evento {
                    responder(&PluginMessage::Evento { evento });
                }
            }
            HostMessage::Health { id } => {
                responder(&PluginMessage::Health {
                    id,
                    health: Health::ok(format!(
                        "eco {} — {} mensagem(ns) tratada(s)",
                        env!("CARGO_PKG_VERSION"),
                        eco.mensagens
                    )),
                });
            }
            HostMessage::Shutdown => return 0,
        }
    }
}

/// Estado mínimo do plugin: de quem é o handshake e quantas mensagens passaram.
#[derive(Default)]
struct Eco {
    integracao: Option<IntegrationId>,
    mensagens: u64,
}

impl Eco {
    /// Evento de volta com o texto invertido. O `id_externo` sai da chave de idempotência do
    /// comando, então reenviar o mesmo comando não duplica a mensagem na `inbox`.
    fn evento_do_texto(&self, integracao: IntegrationId, texto: &SendText) -> InboundEvent {
        let invertido: String = texto.text.chars().rev().collect();
        InboundEvent {
            integration: integracao,
            id_externo: format!("eco-{}", texto.idempotency),
            peer: texto.peer.clone(),
            remetente: None,
            text: invertido,
            attachments: Vec::new(),
            occurred_at: agora_ms(),
            metadata: serde_json::Value::Null,
        }
    }
}

fn agora_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duracao| duracao.as_millis() as i64)
        .unwrap_or(0)
}

/// O que o Harness precisa saber deste plugin (PRD §8.3).
fn manifesto() -> Manifest {
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
        // O eco não usa segredo de verdade, mas declara um para o exemplo mostrar o formulário de
        // segredo do Console e o caminho de cifra em repouso (HR-44c). O campo é obrigatório para
        // que a API tenha o que cobrar.
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

/// Escreve uma mensagem do protocolo (uma por linha) e descarrega no cano.
fn responder(mensagem: &PluginMessage) {
    let Ok(linha) = serde_json::to_string(mensagem) else {
        return;
    };
    let mut saida = std::io::stdout().lock();
    if writeln!(saida, "{linha}").is_err() {
        return;
    }
    let _ = saida.flush();
}
