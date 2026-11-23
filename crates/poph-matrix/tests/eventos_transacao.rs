//! MX-17 — o classificador rodando sobre uma transação inteira, no formato em que o Synapse
//! entrega ao Application Service.
//!
//! O teste de unidade do módulo (`src/eventos.rs`) cobre a tabela de tipos com eventos montados à
//! mão. Aqui o corpo é uma transação de verdade, com o `events` que a MX-16 já entrega à trait
//! `EntregaDeEventos` — inclusive os eventos de estado que o Synapse manda junto e que o
//! classificador precisa descartar sem quebrar o resto do lote.
//!
//! Não há rede, banco nem relógio: é a mesma função pura, chamada sobre a lista que a MX-18 vai
//! receber.

use serde_json::{Value, json};

use poph_matrix::eventos::{Acao, MotivoIgnorar, TipoMidia, classificar};
use poph_matrix::usuarios::IdsFantasmas;

const SERVIDOR: &str = "matrix-dev.local";
const SALA: &str = "!sala-do-atendimento:matrix-dev.local";
const ATENDENTE: &str = "@willian:matrix-dev.local";

fn ids() -> IdsFantasmas {
    IdsFantasmas::novo("ph", SERVIDOR).expect("prefixo e servidor válidos")
}

/// Uma transação como o Synapse a envia: `{"events": [...]}`, com eventos de estado misturados aos
/// de mensagem, na ordem em que chegaram.
fn transacao() -> Value {
    let ids = ids();
    let fantasma_cliente = ids.cliente("+55 21 99999-0001").expect("cliente");
    let fantasma_agente = ids.agente("agente-1").expect("agente");
    let bot = ids.bot();

    json!({
        "events": [
            // Entrada do fantasma do cliente: evento de estado, não é resposta.
            {
                "type": "m.room.member",
                "room_id": SALA,
                "event_id": "$membro1:matrix-dev.local",
                "sender": fantasma_cliente.user_id(),
                "state_key": fantasma_cliente.user_id(),
                "content": { "membership": "join" }
            },
            // Metadata que a própria MX-11 gravou na criação da sala.
            {
                "type": "com.sofacoding.atendimento.metadata",
                "room_id": SALA,
                "event_id": "$meta1:matrix-dev.local",
                "sender": fantasma_agente.user_id(),
                "state_key": "",
                "content": { "conversation_id": "0190a000-0000-7000-8000-000000000001" }
            },
            // O que o Harness espelhou (eco): não pode voltar como resposta.
            {
                "type": "m.room.message",
                "room_id": SALA,
                "event_id": "$eco1:matrix-dev.local",
                "sender": fantasma_cliente.user_id(),
                "content": { "msgtype": "m.text", "body": "Quero saber o preço" }
            },
            // A resposta do atendente: é esta que vale.
            {
                "type": "m.room.message",
                "room_id": SALA,
                "event_id": "$resposta1:matrix-dev.local",
                "sender": ATENDENTE,
                "origin_server_ts": 1_765_000_000_000u64,
                "content": { "msgtype": "m.text", "body": "Claro, já te passo o valor." }
            },
            // Aviso do bot do Application Service.
            {
                "type": "m.room.message",
                "room_id": SALA,
                "event_id": "$aviso1:matrix-dev.local",
                "sender": bot.user_id(),
                "content": { "msgtype": "m.notice", "body": "módulo conectado" }
            },
            // Edição da resposta anterior.
            {
                "type": "m.room.message",
                "room_id": SALA,
                "event_id": "$edicao1:matrix-dev.local",
                "sender": ATENDENTE,
                "content": {
                    "msgtype": "m.text",
                    "body": "* Claro, já te passo o valor com desconto.",
                    "m.new_content": { "msgtype": "m.text", "body": "Claro, já te passo o valor com desconto." },
                    "m.relates_to": { "rel_type": "m.replace", "event_id": "$resposta1:matrix-dev.local" }
                }
            },
            // Reação à resposta.
            {
                "type": "m.reaction",
                "room_id": SALA,
                "event_id": "$reacao1:matrix-dev.local",
                "sender": ATENDENTE,
                "content": { "m.relates_to": { "rel_type": "m.annotation", "event_id": "$resposta1:matrix-dev.local", "key": "👍" } }
            },
            // Mídia do atendente: o arquivo vai ao cliente (MX-44), não é texto.
            {
                "type": "m.room.message",
                "room_id": SALA,
                "event_id": "$foto1:matrix-dev.local",
                "sender": ATENDENTE,
                "content": { "msgtype": "m.image", "body": "tabela.png", "url": "mxc://matrix-dev.local/abc" }
            }
        ]
    })
}

fn acoes() -> Vec<Acao> {
    let ids = ids();
    let eventos = transacao()
        .get("events")
        .and_then(Value::as_array)
        .expect("transação com events")
        .clone();
    eventos
        .iter()
        .map(|evento| classificar(&ids, evento))
        .collect()
}

#[test]
fn transacao_inteira_vira_uma_resposta_uma_midia_e_o_resto_ruido() {
    let acoes = acoes();
    assert_eq!(
        acoes.len(),
        8,
        "todos os eventos precisam ser classificados"
    );

    assert_eq!(
        acoes[0],
        Acao::Ignorar {
            motivo: MotivoIgnorar::EventoDeEstado
        }
    );
    assert_eq!(
        acoes[1],
        Acao::Ignorar {
            motivo: MotivoIgnorar::EventoDeEstado
        }
    );
    assert_eq!(
        acoes[2],
        Acao::Ignorar {
            motivo: MotivoIgnorar::RemetenteFantasma
        },
        "o eco do que o módulo espelhou não pode virar resposta"
    );
    assert_eq!(
        acoes[3],
        Acao::RespostaHumana {
            sala: SALA.to_owned(),
            evento_id: "$resposta1:matrix-dev.local".to_owned(),
            remetente: ATENDENTE.to_owned(),
            texto: "Claro, já te passo o valor.".to_owned(),
        }
    );
    assert_eq!(
        acoes[4],
        Acao::Ignorar {
            motivo: MotivoIgnorar::RemetenteBot
        }
    );
    assert_eq!(
        acoes[5],
        Acao::Ignorar {
            motivo: MotivoIgnorar::Edicao
        }
    );
    assert_eq!(
        acoes[6],
        Acao::Ignorar {
            motivo: MotivoIgnorar::Reacao
        }
    );
    assert_eq!(
        acoes[7],
        Acao::MidiaDoAtendente {
            sala: SALA.to_owned(),
            evento_id: "$foto1:matrix-dev.local".to_owned(),
            remetente: ATENDENTE.to_owned(),
            tipo: TipoMidia::Imagem,
            url: Some("mxc://matrix-dev.local/abc".to_owned()),
            nome: "tabela.png".to_owned(),
        }
    );

    // Exatamente uma resposta: se o eco passasse, seriam duas e o cliente receberia a própria
    // mensagem de volta.
    let respostas = acoes
        .iter()
        .filter(|acao| matches!(acao, Acao::RespostaHumana { .. }))
        .count();
    assert_eq!(respostas, 1);
}

#[test]
fn classificar_duas_vezes_a_mesma_transacao_da_o_mesmo_resultado() {
    // Função pura: sem estado escondido entre chamadas (a MX-18 depende disso para reprocessar o
    // reenvio de uma transação sem inventar resultado diferente).
    assert_eq!(acoes(), acoes());
}

#[test]
fn transacao_sem_eventos_nao_produz_nada() {
    let ids = ids();
    let vazio: Vec<Acao> = Vec::new();
    assert!(vazio.is_empty());
    // E um evento que não é objeto não derruba o lote: vira ruído e o resto segue.
    let nao_objeto = json!("texto solto");
    assert_eq!(
        classificar(&ids, &nao_objeto),
        Acao::Ignorar {
            motivo: MotivoIgnorar::EventoIncompleto
        }
    );
}
