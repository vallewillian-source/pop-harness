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
    Capacidades, ComandoSaida, EsquemaJson, InboundEvent, IntegrationId, Manifesto, MensagemHost,
    MensagemPlugin, PROTOCOLO_VERSAO, Presenca, Saude, SendText,
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
        let mensagem: MensagemHost = match serde_json::from_str(&linha) {
            Ok(mensagem) => mensagem,
            Err(erro) => {
                // Regra do protocolo: linha fora do contrato é lixo, e lixo se ignora.
                eprintln!("eco: linha fora do protocolo ignorada: {erro}");
                continue;
            }
        };
        match mensagem {
            MensagemHost::Handshake(handshake) => {
                if handshake.protocolo != PROTOCOLO_VERSAO {
                    responder(&MensagemPlugin::Erro {
                        codigo: "protocolo_incompativel".to_owned(),
                        mensagem: format!(
                            "o eco fala a versão {PROTOCOLO_VERSAO} e o host falou {}",
                            handshake.protocolo
                        ),
                        transitorio: false,
                    });
                    return 2;
                }
                eco.integracao = Some(handshake.integracao);
                // Os segredos do handshake não são usados aqui (o eco não tem canal de verdade).
                // Um plugin real usa este ponto para autenticar no canal.
                let _ = &handshake.segredos;
                responder(&MensagemPlugin::HandshakeOk {
                    protocolo: PROTOCOLO_VERSAO,
                    manifesto: manifesto(),
                });
            }
            MensagemHost::Comando { id, comando } => {
                let Some(integracao) = eco.integracao else {
                    responder(&MensagemPlugin::Resultado {
                        id,
                        ok: false,
                        erro: Some(poph_types::ErroComando {
                            codigo: "sem_handshake".to_owned(),
                            mensagem: "recebi comando antes do handshake".to_owned(),
                            transitorio: false,
                        }),
                    });
                    continue;
                };
                let mut evento = None;
                match comando {
                    ComandoSaida::EnviarTexto(texto) => {
                        eco.mensagens += 1;
                        evento = Some(eco.evento_do_texto(integracao, &texto));
                    }
                    ComandoSaida::EnviarArquivo(arquivo) => {
                        // Um plugin real subiria o arquivo para o canal. O eco só confirma.
                        eprintln!(
                            "eco: arquivo confirmado ({} bytes): {}",
                            arquivo.tamanho_bytes, arquivo.nome
                        );
                    }
                    ComandoSaida::DefinirPresenca(presenca) => {
                        if presenca.estado == Presenca::Digitando {
                            eprintln!("eco: digitando para {}", presenca.peer);
                        }
                    }
                }
                responder(&MensagemPlugin::Resultado {
                    id,
                    ok: true,
                    erro: None,
                });
                if let Some(evento) = evento {
                    responder(&MensagemPlugin::Evento { evento });
                }
            }
            MensagemHost::Saude { id } => {
                responder(&MensagemPlugin::Saude {
                    id,
                    saude: Saude::ok(format!(
                        "eco {} — {} mensagem(ns) tratada(s)",
                        env!("CARGO_PKG_VERSION"),
                        eco.mensagens
                    )),
                });
            }
            MensagemHost::Encerrar => return 0,
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
        let invertido: String = texto.texto.chars().rev().collect();
        InboundEvent {
            integracao,
            id_externo: format!("eco-{}", texto.idempotencia),
            peer: texto.peer.clone(),
            remetente: None,
            texto: invertido,
            anexos: Vec::new(),
            ocorrido_em: agora_ms(),
            metadados: serde_json::Value::Null,
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
fn manifesto() -> Manifesto {
    Manifesto {
        nome: "Eco (exemplo)".to_owned(),
        kind: "eco".to_owned(),
        versao: env!("CARGO_PKG_VERSION").to_owned(),
        protocolo: PROTOCOLO_VERSAO,
        capacidades: Capacidades {
            text: true,
            file_out: true,
            typing: true,
            ..Capacidades::default()
        },
        limites: poph_types::LimitesCanal::default(),
        configuracao: EsquemaJson(serde_json::json!({
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
        segredos: EsquemaJson(serde_json::json!({
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
fn responder(mensagem: &MensagemPlugin) {
    let Ok(linha) = serde_json::to_string(mensagem) else {
        return;
    };
    let mut saida = std::io::stdout().lock();
    if writeln!(saida, "{linha}").is_err() {
        return;
    }
    let _ = saida.flush();
}
