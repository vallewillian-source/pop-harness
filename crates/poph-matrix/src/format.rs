use poph_types::{ConversationId, MessageId, UnsupportedContent};

use crate::events::MSGTYPE_NOTICE;
use crate::users::{Ghost, GhostIds, GhostsError};

pub const EVENT_KIND: &str = "m.room.message";

pub const MSGTYPE_TEXT: &str = "m.text";

pub const PREFIX_TXN: &str = "poph-msg-";

pub const JSON_BODY_LIMIT: usize = 48 * 1024;

pub const RETICENCIAS: &str = "…";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageOrigin {
    Agent,
    Human,
    Device,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    Client,
    Agent,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Meaning {
    Entry,
    Output,
}

#[derive(Debug, Clone, Copy)]
pub struct MessageForMirror<'a> {
    pub id: MessageId,
    pub conversation: ConversationId,
    pub origin: MessageOrigin,
    pub meaning: Meaning,
    pub role: MessageRole,
    pub text: &'a str,
    pub client_identifier: &'a str,
    pub id_agent: &'a str,
    pub id_integration: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotMirrorReason {
    HumanOrigin,
    UnmirroredRole,
    TextEmpty,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormatError {
    #[error("não foi possível montar o usuário virtual da mensagem: {0}")]
    Ghost(#[from] GhostsError),
    #[error("papel e sentido da mensagem não combinam")]
    Inconsistente,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outbound {
    pub conversation: ConversationId,
    pub sender: Ghost,
    pub origin: MessageOrigin,
    pub event_kind: &'static str,
    pub msgtype: &'static str,
    pub body: String,
    pub txn_id: String,
    pub cut: bool,
    pub bytes_originais: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Mirror(Outbound),
    NotMirror(NotMirrorReason),
    Error(FormatError),
}

pub fn decide(ids: &GhostIds, message: &MessageForMirror<'_>) -> Decision {
    match message.origin {
        MessageOrigin::Human => return Decision::NotMirror(NotMirrorReason::HumanOrigin),
        MessageOrigin::Device => return decide_from_device(ids, message),
        MessageOrigin::Agent => {}
    }

    if message.role == MessageRole::Other {
        return Decision::NotMirror(NotMirrorReason::UnmirroredRole);
    }

    let combines = match message.role {
        MessageRole::Client => message.meaning == Meaning::Entry,
        MessageRole::Agent => message.meaning == Meaning::Output,
        MessageRole::Other => true,
    };
    if !combines {
        return Decision::Error(FormatError::Inconsistente);
    }

    if message.text.trim().is_empty() {
        return Decision::NotMirror(NotMirrorReason::TextEmpty);
    }

    if message.role == MessageRole::Client
        && let Some(kind) = UnsupportedContent::from_history_text(message.text)
    {
        return notice_for_attachment_outside_scope(ids, message, kind);
    }

    let sender = match message.role {
        MessageRole::Client => ids.client(message.client_identifier),
        MessageRole::Agent => ids.agent(message.id_agent),
        MessageRole::Other => return Decision::NotMirror(NotMirrorReason::UnmirroredRole),
    };
    let sender = match sender {
        Ok(ghost) => ghost,
        Err(error) => return Decision::Error(FormatError::Ghost(error)),
    };

    Decision::Mirror(build_send(message, sender, MessageOrigin::Agent))
}

fn notice_for_attachment_outside_scope(
    ids: &GhostIds,
    message: &MessageForMirror<'_>,
    kind: UnsupportedContent,
) -> Decision {
    let sender = match ids.agent(message.id_agent) {
        Ok(ghost) => ghost,
        Err(error) => return Decision::Error(FormatError::Ghost(error)),
    };
    Decision::Mirror(Outbound {
        conversation: message.conversation,
        sender,
        origin: MessageOrigin::Agent,
        event_kind: EVENT_KIND,
        msgtype: MSGTYPE_NOTICE,
        body: kind.room_notice().to_owned(),
        txn_id: message_txn_id(message.id),
        cut: false,
        bytes_originais: message.text.len(),
    })
}

fn decide_from_device(ids: &GhostIds, message: &MessageForMirror<'_>) -> Decision {
    if message.role != MessageRole::Agent || message.meaning != Meaning::Output {
        return Decision::Error(FormatError::Inconsistente);
    }
    if message.text.trim().is_empty() {
        return Decision::NotMirror(NotMirrorReason::TextEmpty);
    }
    let Some(instance) = message.id_integration else {
        return Decision::Error(FormatError::Ghost(GhostsError::Identifier {
            target: "aparelho",
            reason: "conversa sem integração",
        }));
    };
    match ids.device(instance) {
        Ok(sender) => Decision::Mirror(build_send(message, sender, MessageOrigin::Device)),
        Err(error) => Decision::Error(FormatError::Ghost(error)),
    }
}

fn build_send(message: &MessageForMirror<'_>, sender: Ghost, origin: MessageOrigin) -> Outbound {
    let (body, cut) = cut_at_limit(message.text);
    Outbound {
        conversation: message.conversation,
        sender,
        origin,
        event_kind: EVENT_KIND,
        msgtype: MSGTYPE_TEXT,
        body,
        txn_id: message_txn_id(message.id),
        cut,
        bytes_originais: message.text.len(),
    }
}

pub fn message_txn_id(id: MessageId) -> String {
    format!("{PREFIX_TXN}{id}")
}

pub fn cut_at_limit(text: &str) -> (String, bool) {
    if json_size(text) <= JSON_BODY_LIMIT {
        return (text.to_owned(), false);
    }
    let budget = JSON_BODY_LIMIT.saturating_sub(json_size(RETICENCIAS));
    let mut mantido = String::new();
    let mut used = 0usize;
    for character in text.chars() {
        let piece = character.to_string();
        let cost = json_size(&piece);
        if used + cost > budget {
            break;
        }
        used += cost;
        mantido.push(character);
    }
    mantido.push_str(RETICENCIAS);
    (mantido, true)
}

fn json_size(text: &str) -> usize {
    serde_json::to_string(text).map_or(text.len().saturating_mul(6), |json| {
        json.len().saturating_sub(2)
    })
}

#[cfg(test)]
mod tests {
    use poph_types::{ConversationId, MessageId};

    use super::*;

    const SERVER: &str = "matrix-dev.local";

    fn ids() -> GhostIds {
        GhostIds::new("ph", SERVER).expect("prefixo e servidor válidos")
    }

    fn message_id(n: u64) -> MessageId {
        format!("0190a000-0000-7000-8000-{n:012x}")
            .parse::<MessageId>()
            .expect("UUID v7 válido")
    }

    fn conversation() -> ConversationId {
        ConversationId::generate()
    }

    fn base<'a>(
        text: &'a str,
        origin: MessageOrigin,
        role: MessageRole,
        meaning: Meaning,
    ) -> MessageForMirror<'a> {
        MessageForMirror {
            id: message_id(1),
            conversation: conversation(),
            origin,
            meaning,
            role,
            text,
            client_identifier: "+55 21 99999-0001",
            id_agent: "agente-1",
            id_integration: Some("inst-1"),
        }
    }

    fn mirrored(decision: Decision) -> Outbound {
        match decision {
            Decision::Mirror(send) => send,
            other_one => panic!("esperava espelhar, veio {other_one:?}"),
        }
    }

    #[test]
    fn agent_message_goes_out_through_agent_ghost() {
        let m = base(
            "Olá, em que posso ajudar?",
            MessageOrigin::Agent,
            MessageRole::Agent,
            Meaning::Output,
        );
        let send = mirrored(decide(&ids(), &m));
        assert_eq!(send.sender.user_id(), "@ph_ag_agente-1:matrix-dev.local");
        assert_eq!(send.event_kind, "m.room.message");
        assert_eq!(send.msgtype, "m.text");
        assert!(!send.cut);
    }

    #[test]
    fn client_message_goes_out_through_client_ghost() {
        let m = base(
            "Quero saber o preço",
            MessageOrigin::Agent,
            MessageRole::Client,
            Meaning::Entry,
        );
        let send = mirrored(decide(&ids(), &m));
        assert!(
            send.sender.user_id().starts_with("@ph_wa_"),
            "fantasma do cliente esperado, veio {}",
            send.sender
        );
        assert_eq!(send.body, "Quero saber o preço");
    }

    #[test]
    fn human_origin_does_not_mirror_in_any_role() {
        for (role, meaning) in [
            (MessageRole::Agent, Meaning::Output),
            (MessageRole::Client, Meaning::Entry),
        ] {
            let m = base("Resposta do atendente", MessageOrigin::Human, role, meaning);
            assert_eq!(
                decide(&ids(), &m),
                Decision::NotMirror(NotMirrorReason::HumanOrigin)
            );
        }
    }

    #[test]
    fn device_origin_mirrors_through_device_ghost() {
        let m = base(
            "digitada no celular",
            MessageOrigin::Device,
            MessageRole::Agent,
            Meaning::Output,
        );
        let send = mirrored(decide(&ids(), &m));
        assert_eq!(send.origin, MessageOrigin::Device);
        assert_eq!(send.body, "digitada no celular");
        assert_eq!(send.sender, ids().device("inst-1").unwrap());
        assert!(send.sender.localpart().starts_with("ph_ap_"));
        assert_ne!(send.sender, ids().agent("agente-1").unwrap());
        assert_ne!(send.sender, ids().client("+55 21 99999-0001").unwrap());
        assert_eq!(send.txn_id, message_txn_id(message_id(1)));
    }

    #[test]
    fn device_origin_without_integration_becomes_format_error() {
        let mut m = base(
            "digitada no celular",
            MessageOrigin::Device,
            MessageRole::Agent,
            Meaning::Output,
        );
        m.id_integration = None;
        assert!(matches!(
            decide(&ids(), &m),
            Decision::Error(FormatError::Ghost(_))
        ));
    }

    #[test]
    fn device_origin_not_from_agent_output_becomes_error() {
        let m = base(
            "digitada no celular",
            MessageOrigin::Device,
            MessageRole::Client,
            Meaning::Entry,
        );
        assert_eq!(
            decide(&ids(), &m),
            Decision::Error(FormatError::Inconsistente)
        );
    }

    #[test]
    fn device_origin_with_empty_text_does_not_mirror() {
        let m = base(
            "   ",
            MessageOrigin::Device,
            MessageRole::Agent,
            Meaning::Output,
        );
        assert_eq!(
            decide(&ids(), &m),
            Decision::NotMirror(NotMirrorReason::TextEmpty)
        );
    }

    #[test]
    fn agent_message_continues_through_agent_ghost() {
        let m = base(
            "resposta do agente",
            MessageOrigin::Agent,
            MessageRole::Agent,
            Meaning::Output,
        );
        let send = mirrored(decide(&ids(), &m));
        assert_eq!(send.origin, MessageOrigin::Agent);
        assert_eq!(send.sender, ids().agent("agente-1").unwrap());
        assert!(send.sender.localpart().starts_with("ph_ag_"));
    }

    #[test]
    fn out_of_scope_attachment_history_becomes_notice_through_agent_ghost() {
        for kind in UnsupportedContent::ALL_VALUES {
            let m = base(
                kind.history_text(),
                MessageOrigin::Agent,
                MessageRole::Client,
                Meaning::Entry,
            );
            let send = mirrored(decide(&ids(), &m));
            assert_eq!(send.msgtype, "m.notice", "{kind:?}");
            assert_eq!(send.body, kind.room_notice(), "{kind:?}");
            assert!(
                send.sender.user_id().starts_with("@ph_ag_agente-1:"),
                "aviso sai pelo fantasma do agente, veio {}",
                send.sender
            );
            assert_eq!(send.txn_id, message_txn_id(message_id(1)));
        }
    }

    #[test]
    fn history_becomes_notice_only_when_from_client() {
        let m = base(
            UnsupportedContent::Audio.history_text(),
            MessageOrigin::Agent,
            MessageRole::Agent,
            Meaning::Output,
        );
        let send = mirrored(decide(&ids(), &m));
        assert_eq!(send.msgtype, "m.text");
        assert_eq!(send.body, UnsupportedContent::Audio.history_text());
    }

    #[test]
    fn sentence_like_history_continues_as_client_text() {
        let m = base(
            "[O cliente enviou um áudio]",
            MessageOrigin::Agent,
            MessageRole::Client,
            Meaning::Entry,
        );
        let send = mirrored(decide(&ids(), &m));
        assert_eq!(send.msgtype, "m.text");
        assert!(send.sender.user_id().starts_with("@ph_wa_"));
    }

    #[test]
    fn tool_and_system_do_not_go_to_room() {
        let m = base(
            "resultado de ferramenta",
            MessageOrigin::Agent,
            MessageRole::Other,
            Meaning::Output,
        );
        assert_eq!(
            decide(&ids(), &m),
            Decision::NotMirror(NotMirrorReason::UnmirroredRole)
        );
    }

    #[test]
    fn empty_or_whitespace_text_does_not_mirror() {
        for text in ["", " ", "   ", "\n", "\t \r\n", "\u{00a0}"] {
            let m = base(
                text,
                MessageOrigin::Agent,
                MessageRole::Agent,
                Meaning::Output,
            );
            assert_eq!(
                decide(&ids(), &m),
                Decision::NotMirror(NotMirrorReason::TextEmpty),
                "texto {text:?} deveria ser tratado como vazio"
            );
        }
    }

    #[test]
    fn role_and_meaning_that_do_not_match_become_error_not_silence() {
        let swapped_client = base(
            "oi",
            MessageOrigin::Agent,
            MessageRole::Client,
            Meaning::Output,
        );
        assert_eq!(
            decide(&ids(), &swapped_client),
            Decision::Error(FormatError::Inconsistente)
        );
        let swapped_agent = base(
            "oi",
            MessageOrigin::Agent,
            MessageRole::Agent,
            Meaning::Entry,
        );
        assert_eq!(
            decide(&ids(), &swapped_agent),
            Decision::Error(FormatError::Inconsistente)
        );
    }

    #[test]
    fn identifier_that_builds_no_user_becomes_format_error() {
        let mut m = base(
            "oi",
            MessageOrigin::Agent,
            MessageRole::Client,
            Meaning::Entry,
        );
        m.client_identifier = "   ";
        assert!(matches!(
            decide(&ids(), &m),
            Decision::Error(FormatError::Ghost(_))
        ));
    }

    #[test]
    fn body_goes_out_as_written() {
        let text = "  *Oi*, tudo bem?\n\n- item 1\n- item 2  \n  ";
        let m = base(
            text,
            MessageOrigin::Agent,
            MessageRole::Client,
            Meaning::Entry,
        );
        let send = mirrored(decide(&ids(), &m));
        assert_eq!(send.body, text, "o corpo não pode ser reformatado");
    }

    #[test]
    fn text_exactly_at_limit_is_not_cut() {
        let text = "a".repeat(JSON_BODY_LIMIT);
        let (body, cut) = cut_at_limit(&text);
        assert!(!cut);
        assert_eq!(body, text);
    }

    #[test]
    fn text_above_limit_is_cut_with_ellipsis_and_marked() {
        let text = "a".repeat(JSON_BODY_LIMIT + 1);
        let (body, cut) = cut_at_limit(&text);
        assert!(cut);
        assert!(body.ends_with(RETICENCIAS));
        assert_eq!(
            json_size(&body),
            JSON_BODY_LIMIT,
            "o corte fecha exatamente no limite"
        );
    }

    #[test]
    fn cutoff_counts_json_escape_and_keeps_character_whole() {
        let control = "\u{1}".repeat(JSON_BODY_LIMIT);
        let (body, cut) = cut_at_limit(&control);
        assert!(cut);
        assert!(json_size(&body) <= JSON_BODY_LIMIT);
        assert!(body.ends_with(RETICENCIAS));

        let acentuado = "ç".repeat(JSON_BODY_LIMIT);
        let (body, cut) = cut_at_limit(&acentuado);
        assert!(cut);
        assert!(json_size(&body) <= JSON_BODY_LIMIT);
        assert!(body.ends_with(RETICENCIAS));
    }

    #[test]
    fn decision_with_long_text_marks_cutoff_and_keeps_original_size() {
        let text = "b".repeat(JSON_BODY_LIMIT * 2);
        let m = base(
            &text,
            MessageOrigin::Agent,
            MessageRole::Agent,
            Meaning::Output,
        );
        let send = mirrored(decide(&ids(), &m));
        assert!(send.cut);
        assert_eq!(send.bytes_originais, text.len());
        assert!(json_size(&send.body) <= JSON_BODY_LIMIT);
    }

    #[test]
    fn txn_id_is_stable_for_same_message() {
        let id = message_id(7);
        assert_eq!(message_txn_id(id), message_txn_id(id));
        let a = base(
            "um",
            MessageOrigin::Agent,
            MessageRole::Agent,
            Meaning::Output,
        );
        let mut b = base(
            "outro texto",
            MessageOrigin::Agent,
            MessageRole::Agent,
            Meaning::Output,
        );
        b.id = a.id;
        b.conversation = a.conversation;
        let txn_a = mirrored(decide(&ids(), &a)).txn_id;
        let txn_b = mirrored(decide(&ids(), &b)).txn_id;
        assert_eq!(txn_a, txn_b, "o txnId depende só do id da mensagem");
    }

    #[test]
    fn txn_id_is_distinct_between_messages_and_url_safe() {
        let a = message_txn_id(message_id(1));
        let b = message_txn_id(message_id(2));
        assert_ne!(a, b);
        assert!(a.starts_with(PREFIX_TXN));
        assert!(
            a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "txnId com caractere fora de letra, número e hífen: {a}"
        );
    }

    #[test]
    fn whole_decision_is_reproducible() {
        let m = base(
            "mesma entrada",
            MessageOrigin::Agent,
            MessageRole::Agent,
            Meaning::Output,
        );
        assert_eq!(decide(&ids(), &m), decide(&ids(), &m));
    }
}
