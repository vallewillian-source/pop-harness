use serde_json::Value;

use crate::format::{EVENT_KIND, MSGTYPE_TEXT};
use crate::users::GhostIds;

pub const MSGTYPE_EMOTE: &str = "m.emote";
pub const MSGTYPE_NOTICE: &str = "m.notice";
pub const REACTION_KIND: &str = "m.reaction";
pub const REMOVAL_KIND: &str = "m.room.redaction";
pub const RELACAO_SUBSTITUICAO: &str = "m.replace";
pub const RELACAO_ANOTACAO: &str = "m.annotation";
pub const NEW_CONTENT_FIELD: &str = "m.new_content";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetKind {
    Image,
    File,
    Audio,
    Video,
}

impl AssetKind {
    pub const ALL: [AssetKind; 4] = [
        AssetKind::Image,
        AssetKind::File,
        AssetKind::Audio,
        AssetKind::Video,
    ];

    pub fn msgtype(self) -> &'static str {
        match self {
            AssetKind::Image => "m.image",
            AssetKind::File => "m.file",
            AssetKind::Audio => "m.audio",
            AssetKind::Video => "m.video",
        }
    }

    pub fn from_msgtype(msgtype: &str) -> Option<Self> {
        AssetKind::ALL
            .into_iter()
            .find(|kind| kind.msgtype() == msgtype)
    }

    pub fn code(self) -> &'static str {
        match self {
            AssetKind::Image => "imagem",
            AssetKind::File => "arquivo",
            AssetKind::Audio => "audio",
            AssetKind::Video => "video",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IgnoreReason {
    GhostSender,
    BotSender,
    WithoutSender,
    StateEvent,
    Edit,
    Reacao,
    Remocao,
    NotMessageKind,
    Notice,
    UnsupportedMsgtype,
    NonTextContent,
    TextEmpty,
    IncompleteEvent,
}

impl IgnoreReason {
    pub fn code(self) -> &'static str {
        match self {
            IgnoreReason::GhostSender => "remetente_fantasma",
            IgnoreReason::BotSender => "remetente_bot",
            IgnoreReason::WithoutSender => "sem_remetente",
            IgnoreReason::StateEvent => "evento_de_estado",
            IgnoreReason::Edit => "edicao",
            IgnoreReason::Reacao => "reacao",
            IgnoreReason::Remocao => "remocao",
            IgnoreReason::NotMessageKind => "tipo_nao_mensagem",
            IgnoreReason::Notice => "aviso",
            IgnoreReason::UnsupportedMsgtype => "msgtype_nao_suportado",
            IgnoreReason::NonTextContent => "conteudo_nao_texto",
            IgnoreReason::TextEmpty => "texto_vazio",
            IgnoreReason::IncompleteEvent => "evento_incompleto",
        }
    }
}

impl std::fmt::Display for IgnoreReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    HumanResponse {
        room: String,
        event_id: String,
        sender: String,
        text: String,
    },
    AttendantAsset {
        room: String,
        event_id: String,
        sender: String,
        kind: AssetKind,
        url: Option<String>,
        name: String,
    },
    Ignore {
        reason: IgnoreReason,
    },
}

pub fn classify(ids: &GhostIds, event: &Value) -> Action {
    let Some(kind) = text_field(event, "type") else {
        return ignore(IgnoreReason::IncompleteEvent);
    };

    if event.get("state_key").is_some() {
        return ignore(IgnoreReason::StateEvent);
    }

    let Some(sender) = text_field(event, "sender") else {
        return ignore(IgnoreReason::WithoutSender);
    };

    if ids.eh_bot(sender) {
        return ignore(IgnoreReason::BotSender);
    }
    if ids.is_ghost(sender) {
        return ignore(IgnoreReason::GhostSender);
    }

    match kind {
        REMOVAL_KIND => return ignore(IgnoreReason::Remocao),
        REACTION_KIND => return ignore(IgnoreReason::Reacao),
        EVENT_KIND => {}
        _ => return ignore(IgnoreReason::NotMessageKind),
    }

    let Some(content) = event.get("content").filter(|c| c.is_object()) else {
        return ignore(IgnoreReason::NonTextContent);
    };

    if is_edit(content) {
        return ignore(IgnoreReason::Edit);
    }
    if eh_reacao(content) {
        return ignore(IgnoreReason::Reacao);
    }

    let Some(msgtype) = content.get("msgtype").and_then(Value::as_str) else {
        return ignore(IgnoreReason::NonTextContent);
    };

    match msgtype {
        MSGTYPE_TEXT | MSGTYPE_EMOTE => {}
        MSGTYPE_NOTICE => return ignore(IgnoreReason::Notice),
        other => {
            return match AssetKind::from_msgtype(other) {
                Some(asset_kind) => with_fields(event, |room, event_id| Action::AttendantAsset {
                    room: room.to_owned(),
                    event_id: event_id.to_owned(),
                    sender: sender.to_owned(),
                    kind: asset_kind,
                    url: asset_url(content),
                    name: asset_name(content),
                }),
                None => ignore(IgnoreReason::UnsupportedMsgtype),
            };
        }
    }

    let Some(text) = content.get("body").and_then(Value::as_str) else {
        return ignore(IgnoreReason::NonTextContent);
    };
    if text.trim().is_empty() {
        return ignore(IgnoreReason::TextEmpty);
    }

    with_fields(event, |room, event_id| Action::HumanResponse {
        room: room.to_owned(),
        event_id: event_id.to_owned(),
        sender: sender.to_owned(),
        text: text.to_owned(),
    })
}

fn with_fields(event: &Value, build: impl FnOnce(&str, &str) -> Action) -> Action {
    match (text_field(event, "room_id"), text_field(event, "event_id")) {
        (Some(room), Some(event_id)) => build(room, event_id),
        _ => ignore(IgnoreReason::IncompleteEvent),
    }
}

fn asset_url(content: &Value) -> Option<String> {
    content
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
}

fn asset_name(content: &Value) -> String {
    for field in ["filename", "body"] {
        if let Some(name) = content
            .get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            return name.to_owned();
        }
    }
    String::new()
}

fn is_edit(content: &Value) -> bool {
    content.get(NEW_CONTENT_FIELD).is_some() || rel_type(content) == Some(RELACAO_SUBSTITUICAO)
}

fn eh_reacao(content: &Value) -> bool {
    rel_type(content) == Some(RELACAO_ANOTACAO)
}

fn rel_type(content: &Value) -> Option<&str> {
    content
        .get("m.relates_to")
        .and_then(|relacao| relacao.get("rel_type"))
        .and_then(Value::as_str)
}

fn text_field<'a>(event: &'a Value, field: &str) -> Option<&'a str> {
    event
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn ignore(reason: IgnoreReason) -> Action {
    Action::Ignore { reason }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const SERVER: &str = "matrix-dev.local";

    fn ids() -> GhostIds {
        GhostIds::new("ph", SERVER).expect("prefixo e servidor válidos")
    }

    const ATTENDANT: &str = "@willian:matrix-dev.local";

    fn message(sender: &str, content: Value) -> Value {
        json!({
            "type": "m.room.message",
            "room_id": "!abc:matrix-dev.local",
            "event_id": "$ev1:matrix-dev.local",
            "sender": sender,
            "content": content,
        })
    }

    fn with_msgtype(sender: &str, msgtype: &str, body: &str) -> Value {
        message(sender, json!({ "msgtype": msgtype, "body": body }))
    }

    fn reason(action: Action) -> IgnoreReason {
        match action {
            Action::Ignore { reason } => reason,
            other_one => panic!("esperava ignorar, veio {other_one:?}"),
        }
    }

    fn text(action: Action) -> String {
        match action {
            Action::HumanResponse { text, .. } => text,
            other_one => panic!("esperava resposta humana, veio {other_one:?}"),
        }
    }

    #[test]
    fn attendant_text_becomes_response_with_room_event_and_sender() {
        let action = classify(
            &ids(),
            &with_msgtype(ATTENDANT, "m.text", "Já estou verificando seu pedido."),
        );
        assert_eq!(
            action,
            Action::HumanResponse {
                room: "!abc:matrix-dev.local".to_owned(),
                event_id: "$ev1:matrix-dev.local".to_owned(),
                sender: ATTENDANT.to_owned(),
                text: "Já estou verificando seu pedido.".to_owned(),
            }
        );
    }

    #[test]
    fn attendant_emote_also_becomes_response() {
        assert_eq!(
            text(classify(
                &ids(),
                &with_msgtype(ATTENDANT, "m.emote", "confere o pedido")
            )),
            "confere o pedido"
        );
    }

    #[test]
    fn body_goes_out_as_attendant_wrote() {
        let original = "  Olá!\n\nSeu pedido saiu 🚚  ";
        assert_eq!(
            text(classify(
                &ids(),
                &with_msgtype(ATTENDANT, "m.text", original)
            )),
            original
        );
    }

    #[test]
    fn each_attachment_kind_becomes_attendant_attachment() {
        for kind in AssetKind::ALL {
            let action = classify(&ids(), &with_msgtype(ATTENDANT, kind.msgtype(), "foto.jpg"));
            assert_eq!(
                action,
                Action::AttendantAsset {
                    room: "!abc:matrix-dev.local".to_owned(),
                    event_id: "$ev1:matrix-dev.local".to_owned(),
                    sender: ATTENDANT.to_owned(),
                    kind,
                    url: None,
                    name: "foto.jpg".to_owned(),
                },
                "msgtype {} deveria ser mídia {:?}",
                kind.msgtype(),
                kind
            );
        }
    }

    #[test]
    fn attendant_attachment_brings_url_and_file_name() {
        let event = message(
            ATTENDANT,
            json!({
                "msgtype": "m.image",
                "body": "foto.jpg",
                "filename": "foto.jpg",
                "url": "mxc://matrix-dev.local/abc123",
                "info": { "mimetype": "image/jpeg", "size": 2_048 },
            }),
        );
        match classify(&ids(), &event) {
            Action::AttendantAsset {
                kind, url, name, ..
            } => {
                assert_eq!(kind, AssetKind::Image);
                assert_eq!(url.as_deref(), Some("mxc://matrix-dev.local/abc123"));
                assert_eq!(name, "foto.jpg");
            }
            other_one => panic!("esperava mídia do atendente, veio {other_one:?}"),
        }
    }

    #[test]
    fn file_name_prefers_filename_and_falls_back_to_body() {
        let with_filename = message(
            ATTENDANT,
            json!({
                "msgtype": "m.file",
                "body": "clique para baixar",
                "filename": "laudo.pdf",
                "url": "mxc://matrix-dev.local/abc123",
            }),
        );
        assert!(matches!(
            classify(&ids(), &with_filename),
            Action::AttendantAsset { name, .. } if name == "laudo.pdf"
        ));

        let so_body = message(
            ATTENDANT,
            json!({ "msgtype": "m.file", "body": "  laudo.pdf  ", "url": "mxc://s/id" }),
        );
        assert!(matches!(
            classify(&ids(), &so_body),
            Action::AttendantAsset { name, .. } if name == "laudo.pdf"
        ));

        let without_name = message(ATTENDANT, json!({ "msgtype": "m.image", "body": "   " }));
        match classify(&ids(), &without_name) {
            Action::AttendantAsset { name, url, .. } => {
                assert_eq!(name, "");
                assert_eq!(url, None, "sem url, o arquivo não tem de onde ser baixado");
            }
            other_one => panic!("esperava mídia do atendente, veio {other_one:?}"),
        }
    }

    #[test]
    fn bot_notice_is_not_attendant_response() {
        assert_eq!(
            reason(classify(
                &ids(),
                &with_msgtype(ATTENDANT, "m.notice", "conectado")
            )),
            IgnoreReason::Notice
        );
    }

    #[test]
    fn reaction_is_ignored_in_both_formats() {
        let event = json!({
            "type": "m.reaction",
            "room_id": "!abc:matrix-dev.local",
            "event_id": "$ev2:matrix-dev.local",
            "sender": ATTENDANT,
            "content": { "m.relates_to": { "rel_type": "m.annotation", "event_id": "$ev1" } },
        });
        assert_eq!(reason(classify(&ids(), &event)), IgnoreReason::Reacao);

        let esquisito = message(
            ATTENDANT,
            json!({
                "msgtype": "m.text",
                "body": "👍",
                "m.relates_to": { "rel_type": "m.annotation", "event_id": "$ev1" },
            }),
        );
        assert_eq!(reason(classify(&ids(), &esquisito)), IgnoreReason::Reacao);
    }

    #[test]
    fn edit_is_ignored_by_rel_type_and_new_text() {
        let by_rel_type = message(
            ATTENDANT,
            json!({
                "msgtype": "m.text",
                "body": "* pedido enviado",
                "m.relates_to": { "rel_type": "m.replace", "event_id": "$ev1" },
            }),
        );
        assert_eq!(reason(classify(&ids(), &by_rel_type)), IgnoreReason::Edit);

        let by_new_text = message(
            ATTENDANT,
            json!({
                "msgtype": "m.text",
                "body": "* pedido enviado",
                "m.new_content": { "msgtype": "m.text", "body": "pedido enviado" },
            }),
        );
        assert_eq!(reason(classify(&ids(), &by_new_text)), IgnoreReason::Edit);
    }

    #[test]
    fn removal_is_ignored() {
        let event = json!({
            "type": "m.room.redaction",
            "room_id": "!abc:matrix-dev.local",
            "event_id": "$ev3:matrix-dev.local",
            "sender": ATTENDANT,
            "redacts": "$ev1:matrix-dev.local",
            "content": {},
        });
        assert_eq!(reason(classify(&ids(), &event)), IgnoreReason::Remocao);
    }

    #[test]
    fn state_event_is_ignored_even_with_empty_state_key() {
        let membro = json!({
            "type": "m.room.member",
            "room_id": "!abc:matrix-dev.local",
            "event_id": "$ev4:matrix-dev.local",
            "sender": ATTENDANT,
            "state_key": ATTENDANT,
            "content": { "membership": "join" },
        });
        assert_eq!(reason(classify(&ids(), &membro)), IgnoreReason::StateEvent);

        let empty = json!({
            "type": "m.room.message",
            "room_id": "!abc:matrix-dev.local",
            "event_id": "$ev5:matrix-dev.local",
            "sender": ATTENDANT,
            "state_key": "",
            "content": { "msgtype": "m.text", "body": "oi" },
        });
        assert_eq!(reason(classify(&ids(), &empty)), IgnoreReason::StateEvent);
    }

    #[test]
    fn application_service_bot_message_is_ignored() {
        let bot = ids().bot();
        assert_eq!(bot.user_id(), "@ph_as_bot:matrix-dev.local");
        assert_eq!(
            reason(classify(
                &ids(),
                &with_msgtype(bot.user_id(), "m.text", "oi")
            )),
            IgnoreReason::BotSender
        );
    }

    #[test]
    fn echo_of_instance_ghosts_is_ignored() {
        let ids = ids();
        for ghost in [
            ids.client("+55 21 99999-0001").expect("cliente"),
            ids.agent("agente-1").expect("agente"),
            ids.device("inst-1").expect("aparelho"),
        ] {
            assert_eq!(
                reason(classify(
                    &ids,
                    &with_msgtype(ghost.user_id(), "m.text", "mensagem do próprio módulo")
                )),
                IgnoreReason::GhostSender,
                "fantasma {} deveria ser reconhecido como eco",
                ghost
            );
        }
    }

    #[test]
    fn human_with_localpart_like_prefix_is_not_ghost() {
        for human in [
            "@ph_willian:matrix-dev.local",
            "@ph:matrix-dev.local",
            "@ph_wa:matrix-dev.local",
            "@ph_wa_:matrix-dev.local",
            "@ph_wax_1:matrix-dev.local",
            "@ph_agente:matrix-dev.local",
            "@ph_as_bot_extra:matrix-dev.local",
            "@phx_wa_1:matrix-dev.local",
            "@wa_5511999999999:matrix-dev.local",
            "@ph_wa_5511999999999:outro.servidor",
            "willian",
        ] {
            let action = classify(&ids(), &with_msgtype(human, "m.text", "oi"));
            assert!(
                matches!(action, Action::HumanResponse { .. }),
                "{human} deveria ser tratado como humano, veio {action:?}"
            );
        }

        assert!(ids().is_ghost("@ph_wa_5511999999999:matrix-dev.local"));
        assert!(!ids().is_ghost("@ph_AG_1:matrix-dev.local"));
    }

    #[test]
    fn reply_quoting_another_message_stays_a_response() {
        let citacao = message(
            ATTENDANT,
            json!({
                "msgtype": "m.text",
                "body": "> pergunta do cliente\n\nSim, temos.",
                "m.relates_to": { "rel_type": "m.in_reply_to", "event_id": "$ev1" },
            }),
        );
        assert_eq!(
            text(classify(&ids(), &citacao)),
            "> pergunta do cliente\n\nSim, temos."
        );
    }

    #[test]
    fn msgtype_phase_one_does_not_handle_is_ignored() {
        for msgtype in [
            "m.location",
            "m.key.verification.request",
            "com.exemplo.tipo",
        ] {
            assert_eq!(
                reason(classify(&ids(), &with_msgtype(ATTENDANT, msgtype, "x"))),
                IgnoreReason::UnsupportedMsgtype,
                "msgtype {msgtype}"
            );
        }
    }

    #[test]
    fn content_without_msgtype_or_text_is_ignored() {
        for content in [
            json!({}),
            json!({ "body": "sem msgtype" }),
            json!({ "msgtype": "m.text" }),
            json!({ "msgtype": "m.text", "body": 42 }),
            json!({ "msgtype": 7, "body": "x" }),
            json!("não é objeto"),
        ] {
            assert_eq!(
                reason(classify(&ids(), &message(ATTENDANT, content.clone()))),
                IgnoreReason::NonTextContent,
                "conteúdo {content}"
            );
        }

        let without_content = json!({
            "type": "m.room.message",
            "room_id": "!abc:matrix-dev.local",
            "event_id": "$ev6:matrix-dev.local",
            "sender": ATTENDANT,
        });
        assert_eq!(
            reason(classify(&ids(), &without_content)),
            IgnoreReason::NonTextContent
        );
    }

    #[test]
    fn empty_text_becomes_no_response() {
        for body in ["", " ", "\n\t", "\u{00a0}"] {
            assert_eq!(
                reason(classify(&ids(), &with_msgtype(ATTENDANT, "m.text", body))),
                IgnoreReason::TextEmpty,
                "corpo {body:?}"
            );
        }
    }

    #[test]
    fn event_without_room_id_or_event_id_is_incomplete() {
        let without_room = json!({
            "type": "m.room.message",
            "event_id": "$ev7:matrix-dev.local",
            "sender": ATTENDANT,
            "content": { "msgtype": "m.text", "body": "oi" },
        });
        assert_eq!(
            reason(classify(&ids(), &without_room)),
            IgnoreReason::IncompleteEvent
        );

        let without_id = json!({
            "type": "m.room.message",
            "room_id": "!abc:matrix-dev.local",
            "sender": ATTENDANT,
            "content": { "msgtype": "m.text", "body": "oi" },
        });
        assert_eq!(
            reason(classify(&ids(), &without_id)),
            IgnoreReason::IncompleteEvent
        );

        let asset_without_room = json!({
            "type": "m.room.message",
            "event_id": "$ev8:matrix-dev.local",
            "sender": ATTENDANT,
            "content": { "msgtype": "m.image", "body": "foto.jpg" },
        });
        assert_eq!(
            reason(classify(&ids(), &asset_without_room)),
            IgnoreReason::IncompleteEvent
        );
    }

    #[test]
    fn kind_that_is_not_message_is_ignored() {
        for kind in [
            "m.typing",
            "m.receipt",
            "com.exemplo.evento",
            "m.call.invite",
        ] {
            let event = json!({
                "type": kind,
                "room_id": "!abc:matrix-dev.local",
                "event_id": "$ev9:matrix-dev.local",
                "sender": ATTENDANT,
                "content": { "msgtype": "m.text", "body": "oi" },
            });
            assert_eq!(
                reason(classify(&ids(), &event)),
                IgnoreReason::NotMessageKind,
                "tipo {kind}"
            );
        }
    }

    #[test]
    fn event_without_sender_and_type_is_ignored() {
        let without_sender = json!({
            "type": "m.room.message",
            "room_id": "!abc:matrix-dev.local",
            "event_id": "$ev10:matrix-dev.local",
            "content": { "msgtype": "m.text", "body": "oi" },
        });
        assert_eq!(
            reason(classify(&ids(), &without_sender)),
            IgnoreReason::WithoutSender
        );

        let empty = json!({});
        assert_eq!(
            reason(classify(&ids(), &empty)),
            IgnoreReason::IncompleteEvent
        );
    }

    #[test]
    fn reason_codes_are_short_and_distinct() {
        let reasons = [
            IgnoreReason::GhostSender,
            IgnoreReason::BotSender,
            IgnoreReason::WithoutSender,
            IgnoreReason::StateEvent,
            IgnoreReason::Edit,
            IgnoreReason::Reacao,
            IgnoreReason::Remocao,
            IgnoreReason::NotMessageKind,
            IgnoreReason::Notice,
            IgnoreReason::UnsupportedMsgtype,
            IgnoreReason::NonTextContent,
            IgnoreReason::TextEmpty,
            IgnoreReason::IncompleteEvent,
        ];
        let mut seen: Vec<&str> = Vec::new();
        for reason in reasons {
            let code = reason.code();
            assert!(
                (3..=32).contains(&code.len()),
                "código {code:?} fora do tamanho aceitável"
            );
            assert!(
                code.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "código {code:?} deveria ser só a-z e _"
            );
            assert!(!seen.contains(&code), "código repetido: {code}");
            seen.push(code);
            assert_eq!(reason.to_string(), code);
        }
    }

    #[test]
    fn attachment_kind_is_recognized_in_both_directions() {
        for kind in AssetKind::ALL {
            assert_eq!(AssetKind::from_msgtype(kind.msgtype()), Some(kind));
            assert!(kind.code().chars().all(|c| c.is_ascii_lowercase()));
        }
        assert_eq!(AssetKind::from_msgtype("m.text"), None);
    }
}
