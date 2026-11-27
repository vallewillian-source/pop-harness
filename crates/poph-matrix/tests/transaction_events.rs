use serde_json::{Value, json};

use poph_matrix::events::{Action, AssetKind, IgnoreReason, classify};
use poph_matrix::users::GhostIds;

const SERVER: &str = "matrix-dev.local";
const ROOM: &str = "!sala-do-atendimento:matrix-dev.local";
const ATTENDANT: &str = "@willian:matrix-dev.local";

fn ids() -> GhostIds {
    GhostIds::new("ph", SERVER).expect("prefixo e servidor válidos")
}

fn transaction() -> Value {
    let ids = ids();
    let client_ghost = ids.client("+55 21 99999-0001").expect("cliente");
    let agent_ghost = ids.agent("agente-1").expect("agente");
    let bot = ids.bot();

    json!({
        "events": [
            {
                "type": "m.room.member",
                "room_id": ROOM,
                "event_id": "$membro1:matrix-dev.local",
                "sender": client_ghost.user_id(),
                "state_key": client_ghost.user_id(),
                "content": { "membership": "join" }
            },
            {
                "type": "com.sofacoding.atendimento.metadata",
                "room_id": ROOM,
                "event_id": "$meta1:matrix-dev.local",
                "sender": agent_ghost.user_id(),
                "state_key": "",
                "content": { "conversation_id": "0190a000-0000-7000-8000-000000000001" }
            },
            {
                "type": "m.room.message",
                "room_id": ROOM,
                "event_id": "$eco1:matrix-dev.local",
                "sender": client_ghost.user_id(),
                "content": { "msgtype": "m.text", "body": "Quero saber o preço" }
            },
            {
                "type": "m.room.message",
                "room_id": ROOM,
                "event_id": "$resposta1:matrix-dev.local",
                "sender": ATTENDANT,
                "origin_server_ts": 1_765_000_000_000u64,
                "content": { "msgtype": "m.text", "body": "Claro, já te passo o valor." }
            },
            {
                "type": "m.room.message",
                "room_id": ROOM,
                "event_id": "$aviso1:matrix-dev.local",
                "sender": bot.user_id(),
                "content": { "msgtype": "m.notice", "body": "módulo conectado" }
            },
            {
                "type": "m.room.message",
                "room_id": ROOM,
                "event_id": "$edicao1:matrix-dev.local",
                "sender": ATTENDANT,
                "content": {
                    "msgtype": "m.text",
                    "body": "* Claro, já te passo o valor com desconto.",
                    "m.new_content": { "msgtype": "m.text", "body": "Claro, já te passo o valor com desconto." },
                    "m.relates_to": { "rel_type": "m.replace", "event_id": "$resposta1:matrix-dev.local" }
                }
            },
            {
                "type": "m.reaction",
                "room_id": ROOM,
                "event_id": "$reacao1:matrix-dev.local",
                "sender": ATTENDANT,
                "content": { "m.relates_to": { "rel_type": "m.annotation", "event_id": "$resposta1:matrix-dev.local", "key": "👍" } }
            },
            {
                "type": "m.room.message",
                "room_id": ROOM,
                "event_id": "$foto1:matrix-dev.local",
                "sender": ATTENDANT,
                "content": { "msgtype": "m.image", "body": "tabela.png", "url": "mxc://matrix-dev.local/abc" }
            }
        ]
    })
}

fn actions() -> Vec<Action> {
    let ids = ids();
    let events = transaction()
        .get("events")
        .and_then(Value::as_array)
        .expect("transação com events")
        .clone();
    events.iter().map(|event| classify(&ids, event)).collect()
}

#[test]
fn whole_transaction_becomes_one_response_one_attachment_and_rest_noise() {
    let actions = actions();
    assert_eq!(
        actions.len(),
        8,
        "todos os eventos precisam ser classificados"
    );

    assert_eq!(
        actions[0],
        Action::Ignore {
            reason: IgnoreReason::StateEvent
        }
    );
    assert_eq!(
        actions[1],
        Action::Ignore {
            reason: IgnoreReason::StateEvent
        }
    );
    assert_eq!(
        actions[2],
        Action::Ignore {
            reason: IgnoreReason::GhostSender
        },
        "o eco do que o módulo espelhou não pode virar resposta"
    );
    assert_eq!(
        actions[3],
        Action::HumanResponse {
            room: ROOM.to_owned(),
            event_id: "$resposta1:matrix-dev.local".to_owned(),
            sender: ATTENDANT.to_owned(),
            text: "Claro, já te passo o valor.".to_owned(),
        }
    );
    assert_eq!(
        actions[4],
        Action::Ignore {
            reason: IgnoreReason::BotSender
        }
    );
    assert_eq!(
        actions[5],
        Action::Ignore {
            reason: IgnoreReason::Edit
        }
    );
    assert_eq!(
        actions[6],
        Action::Ignore {
            reason: IgnoreReason::Reacao
        }
    );
    assert_eq!(
        actions[7],
        Action::AttendantAsset {
            room: ROOM.to_owned(),
            event_id: "$foto1:matrix-dev.local".to_owned(),
            sender: ATTENDANT.to_owned(),
            kind: AssetKind::Image,
            url: Some("mxc://matrix-dev.local/abc".to_owned()),
            name: "tabela.png".to_owned(),
        }
    );

    let responses = actions
        .iter()
        .filter(|action| matches!(action, Action::HumanResponse { .. }))
        .count();
    assert_eq!(responses, 1);
}

#[test]
fn classifying_same_transaction_twice_gives_same_result() {
    assert_eq!(actions(), actions());
}

#[test]
fn transaction_without_events_produces_nothing() {
    let ids = ids();
    let empty: Vec<Action> = Vec::new();
    assert!(empty.is_empty());
    let not_object = json!("texto solto");
    assert_eq!(
        classify(&ids, &not_object),
        Action::Ignore {
            reason: IgnoreReason::IncompleteEvent
        }
    );
}
