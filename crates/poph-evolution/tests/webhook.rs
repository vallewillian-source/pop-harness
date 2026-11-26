use poph_evolution::webhook::{Decision, IgnoreReason, Policy, check_secret, interpret};
use poph_evolution::{WebhookError, embedded_attachment};
use poph_types::{InboundAttachment, IntegrationId, Secret, StateEvent};
use serde_json::{Value, json};

fn integracao() -> IntegrationId {
    IntegrationId::generate()
}

fn body(evento: &str, data: Value) -> Value {
    json!({
        "event": evento,
        "instance": "pop-atende",
        "data": data,
    })
}

fn mensagem(message: Value) -> Value {
    json!({
        "key": {
            "id": "3EB0A1B2C3D4",
            "remoteJid": "5511999999999@s.whatsapp.net",
            "fromMe": false,
        },
        "message": message,
        "messageTimestamp": 1_795_000_000u64,
        "messageType": "conversation",
    })
}

#[test]
fn plain_text_becomes_a_normalized_event() {
    let body = body(
        "messages.upsert",
        mensagem(json!({"conversation": "quero um orçamento de dedetização"})),
    );
    let Decision::Inbound(inbound) = interpret(integracao(), &body, &Policy::default()) else {
        panic!("esperava entrada");
    };
    assert_eq!(inbound.event.id_externo, "3EB0A1B2C3D4");
    assert_eq!(inbound.event.peer, "5511999999999");
    assert_eq!(inbound.event.text, "quero um orçamento de dedetização");
    assert_eq!(inbound.event.occurred_at, 1_795_000_000_000);
    assert_eq!(inbound.event.attachments.len(), 0);
    assert!(inbound.attachments.is_empty());
    assert_eq!(inbound.event.validar(), Ok(()));
}

#[test]
fn extended_text_is_also_read() {
    let body = body(
        "messages.upsert",
        mensagem(json!({"extendedTextMessage": {"text": "e o prazo?"}})),
    );
    let Decision::Inbound(inbound) = interpret(integracao(), &body, &Policy::default()) else {
        panic!("esperava entrada");
    };
    assert_eq!(inbound.event.text, "e o prazo?");
}

#[test]
fn image_without_caption_enters_as_attachment_to_download() {
    let body = body(
        "messages.upsert",
        mensagem(json!({"imageMessage": {
            "mimetype": "image/jpeg",
            "fileLength": 204_800,
        }})),
    );
    let Decision::Inbound(inbound) = interpret(integracao(), &body, &Policy::default()) else {
        panic!("esperava entrada");
    };
    assert_eq!(inbound.event.text, "");
    assert_eq!(inbound.attachments.len(), 1);
    assert_eq!(inbound.attachments[0].mime, "image/jpeg");
    assert_eq!(inbound.attachments[0].declared_size, Some(204_800));
    assert_eq!(inbound.attachments[0].name, "imagem-3EB0A1B2C3D4.jpg");
}

#[test]
fn image_with_caption_carries_the_text() {
    let body = body(
        "messages.upsert",
        mensagem(json!({"imageMessage": {
            "mimetype": "image/png",
            "caption": "essa é a etiqueta do produto",
        }})),
    );
    let Decision::Inbound(inbound) = interpret(integracao(), &body, &Policy::default()) else {
        panic!("esperava entrada");
    };
    assert_eq!(inbound.event.text, "essa é a etiqueta do produto");
    assert_eq!(inbound.attachments.len(), 1);
}

#[test]
fn pdf_enters_with_its_original_name() {
    let body = body(
        "messages.upsert",
        mensagem(json!({"documentMessage": {
            "mimetype": "application/pdf",
            "fileName": "orcamento.pdf",
            "fileLength": "524288",
        }})),
    );
    let Decision::Inbound(inbound) = interpret(integracao(), &body, &Policy::default()) else {
        panic!("esperava entrada");
    };
    assert_eq!(inbound.attachments.len(), 1);
    assert_eq!(inbound.attachments[0].name, "orcamento.pdf");
    assert_eq!(inbound.attachments[0].declared_size, Some(524_288));
}

#[test]
fn unsupported_document_type_is_refused_with_a_reason() {
    let body = body(
        "messages.upsert",
        mensagem(json!({"documentMessage": {
            "mimetype": "application/zip",
            "fileName": "pacote.zip",
        }})),
    );
    assert_eq!(
        interpret(integracao(), &body, &Policy::default()),
        Decision::Ignored(IgnoreReason::MimeNotSupported("application/zip".to_owned()))
    );
}

#[test]
fn unsupported_attachment_becomes_history_text_without_download() {
    let cases: [(Value, poph_types::UnsupportedContent); 11] = [
        (
            json!({"audioMessage": {"mimetype": "audio/ogg"}}),
            poph_types::UnsupportedContent::Audio,
        ),
        (
            json!({"videoMessage": {"mimetype": "video/mp4"}}),
            poph_types::UnsupportedContent::Video,
        ),
        (
            json!({"ptvMessage": {"mimetype": "video/mp4"}}),
            poph_types::UnsupportedContent::Video,
        ),
        (
            json!({"stickerMessage": {"mimetype": "image/webp"}}),
            poph_types::UnsupportedContent::Sticker,
        ),
        (
            json!({"contactMessage": {"displayName": "Fulano"}}),
            poph_types::UnsupportedContent::Contact,
        ),
        (
            json!({"contactsArrayMessage": {}}),
            poph_types::UnsupportedContent::Contact,
        ),
        (
            json!({"locationMessage": {"degreesLatitude": 1}}),
            poph_types::UnsupportedContent::Location,
        ),
        (
            json!({"liveLocationMessage": {"degreesLatitude": 1}}),
            poph_types::UnsupportedContent::Location,
        ),
        (
            json!({"pollCreationMessage": {"name": "Qual dia?"}}),
            poph_types::UnsupportedContent::Poll,
        ),
        (
            json!({"pollCreationMessageV2": {"name": "Qual dia?"}}),
            poph_types::UnsupportedContent::Poll,
        ),
        (
            json!({"pollCreationMessageV3": {"name": "Qual dia?"}}),
            poph_types::UnsupportedContent::Poll,
        ),
    ];
    for (message, tipo) in cases {
        let body = body("messages.upsert", mensagem(message.clone()));
        match interpret(integracao(), &body, &Policy::default()) {
            Decision::Inbound(inbound) => {
                assert_eq!(inbound.event.text, tipo.history_text(), "{message}");
                assert!(inbound.attachments.is_empty(), "não baixa mídia: {message}");
                assert!(inbound.event.attachments.is_empty(), "{message}");
            }
            other => panic!("esperava texto de histórico para {message}, veio {other:?}"),
        }
    }
}

#[test]
fn video_caption_does_not_enter_history_as_in_python() {
    let body = body(
        "messages.upsert",
        mensagem(json!({"videoMessage": {"mimetype": "video/mp4", "caption": "olha isso"}})),
    );
    match interpret(integracao(), &body, &Policy::default()) {
        Decision::Inbound(inbound) => assert_eq!(
            inbound.event.text,
            poph_types::UnsupportedContent::Video.history_text()
        ),
        other => panic!("esperava entrada, veio {other:?}"),
    }
}

#[test]
fn echo_of_unsupported_attachment_from_the_device_stays_discarded() {
    let mut data = mensagem(json!({"audioMessage": {"mimetype": "audio/ogg"}}));
    data["key"]["fromMe"] = json!(true);
    let body = body("messages.upsert", data);
    let policy = Policy {
        accept_from_me: true,
    };
    assert_eq!(
        interpret(integracao(), &body, &policy),
        Decision::Ignored(IgnoreReason::AttachmentNotSupported("audio".to_owned()))
    );
}

#[test]
fn event_without_known_content_stays_discarded() {
    for message in [
        json!({"pollUpdateMessage": {"pollCreationMessageKey": {}}}),
        json!({"albumMessage": {"expectedImageCount": 2}}),
        json!({"tipoQueNaoExisteAinda": {"x": 1}}),
    ] {
        let body = body("messages.upsert", mensagem(message.clone()));
        assert_eq!(
            interpret(integracao(), &body, &Policy::default()),
            Decision::Ignored(IgnoreReason::NoContent),
            "{message}"
        );
    }
}

#[test]
fn echo_of_our_own_message_is_discarded() {
    let mut data = mensagem(json!({"conversation": "resposta do robô"}));
    data["key"]["fromMe"] = json!(true);
    let body = body("messages.upsert", data);
    assert_eq!(
        interpret(integracao(), &body, &Policy::default()),
        Decision::Ignored(IgnoreReason::FromMe)
    );
    let policy = Policy {
        accept_from_me: true,
    };
    assert!(matches!(
        interpret(integracao(), &body, &policy),
        Decision::Inbound(_)
    ));
}

#[test]
fn group_uses_the_jid_as_destination_and_separates_the_sender() {
    let mut data = mensagem(json!({"conversation": "bom dia, pessoal"}));
    data["key"]["remoteJid"] = json!("120363000000000000@g.us");
    data["key"]["participant"] = json!("5511888888888@s.whatsapp.net");
    let body = body("messages.upsert", data);
    let Decision::Inbound(inbound) = interpret(integracao(), &body, &Policy::default()) else {
        panic!("esperava entrada");
    };
    assert_eq!(inbound.event.peer, "120363000000000000@g.us");
    assert_eq!(
        inbound.event.remetente.as_deref(),
        Some("5511888888888@s.whatsapp.net")
    );
}

#[test]
fn jid_with_device_loses_the_suffix() {
    let mut data = mensagem(json!({"conversation": "oi"}));
    data["key"]["remoteJid"] = json!("5511999999999:12@s.whatsapp.net");
    let body = body("messages.upsert", data);
    let Decision::Inbound(inbound) = interpret(integracao(), &body, &Policy::default()) else {
        panic!("esperava entrada");
    };
    assert_eq!(inbound.event.peer, "5511999999999");
}

#[test]
fn message_without_id_or_sender_is_discarded() {
    let mut without_id = mensagem(json!({"conversation": "oi"}));
    without_id["key"].as_object_mut().unwrap().remove("id");
    assert_eq!(
        interpret(
            integracao(),
            &body("messages.upsert", without_id),
            &Policy::default()
        ),
        Decision::Ignored(IgnoreReason::MissingId)
    );

    let mut without_sender = mensagem(json!({"conversation": "oi"}));
    without_sender["key"]
        .as_object_mut()
        .unwrap()
        .remove("remoteJid");
    assert_eq!(
        interpret(
            integracao(),
            &body("messages.upsert", without_sender),
            &Policy::default()
        ),
        Decision::Ignored(IgnoreReason::MissingSender)
    );
}

#[test]
fn message_without_a_valid_time_is_discarded() {
    let mut data = mensagem(json!({"conversation": "oi"}));
    data["messageTimestamp"] = json!("nao-e-numero");
    assert_eq!(
        interpret(
            integracao(),
            &body("messages.upsert", data),
            &Policy::default()
        ),
        Decision::Ignored(IgnoreReason::MissingTimestamp)
    );
}

#[test]
fn unknown_event_is_ignored_with_its_name() {
    let body = body(
        "presence.update",
        json!({"id": "5511999999999@s.whatsapp.net"}),
    );
    assert_eq!(
        interpret(integracao(), &body, &Policy::default()),
        Decision::Ignored(IgnoreReason::UnknownEvent("presence.update".to_owned()))
    );
}

#[test]
fn connection_state_becomes_a_state_event() {
    let opened = body("connection.update", json!({"state": "open"}));
    assert!(matches!(
        interpret(integracao(), &opened, &Policy::default()),
        Decision::State(StateEvent::Connected { .. })
    ));
    let closed = body("connection.update", json!({"state": "close"}));
    assert!(matches!(
        interpret(integracao(), &closed, &Policy::default()),
        Decision::State(StateEvent::Disconnected { .. })
    ));
    let qr = body(
        "qrcode.updated",
        json!({"qrcode": {"base64": "data:image/png;base64,AAAA"}}),
    );
    let Decision::State(StateEvent::Qr { code, .. }) =
        interpret(integracao(), &qr, &Policy::default())
    else {
        panic!("esperava QR");
    };
    assert_eq!(code, "data:image/png;base64,AAAA");
}

#[test]
fn webhook_secret_is_checked() {
    let secret = Secret::new("segredo-do-webhook".to_owned());
    assert_eq!(check_secret(&secret, Some("segredo-do-webhook")), Ok(()));
    assert_eq!(
        check_secret(&secret, Some("outro")),
        Err(WebhookError::SecretMismatch)
    );
    assert_eq!(
        check_secret(&secret, None),
        Err(WebhookError::SecretMissing)
    );
    assert_eq!(
        check_secret(&Secret::new(String::new()), Some("qualquer")),
        Err(WebhookError::NoSecretConfigured)
    );
}

#[test]
fn embedded_attachment_carries_the_content_in_base64() {
    let attachment = embedded_attachment("nota.pdf", "application/pdf", b"%PDF-1.4");
    let InboundAttachment::Bytes {
        name: nome,
        mime,
        data_base64: encoded_data,
    } = attachment
    else {
        panic!("esperava bytes");
    };
    assert_eq!(nome, "nota.pdf");
    assert_eq!(mime, "application/pdf");
    assert!(!encoded_data.is_empty());
}

#[test]
fn redelivery_of_the_same_webhook_produces_the_same_external_id() {
    let integracao = integracao();
    let body = body(
        "messages.upsert",
        mensagem(json!({"conversation": "oi, preciso de ajuda"})),
    );
    let Decision::Inbound(first) = interpret(integracao, &body, &Policy::default()) else {
        panic!("esperava entrada");
    };
    let Decision::Inbound(second) = interpret(integracao, &body, &Policy::default()) else {
        panic!("esperava entrada");
    };
    assert_eq!(first.event.id_externo, second.event.id_externo);
    assert_eq!(first.event.peer, second.event.peer);
    assert_eq!(first.event.occurred_at, second.event.occurred_at);
}

fn message_with_name(nome: Value, from_me: bool, jid: &str) -> Value {
    let mut dados = mensagem(json!({"conversation": "oi"}));
    dados["key"]["remoteJid"] = json!(jid);
    dados["key"]["fromMe"] = json!(from_me);
    dados["pushName"] = nome;
    dados
}

fn push_name_of(dados: Value) -> Option<String> {
    let body = body("messages.upsert", dados);
    let Decision::Inbound(inbound) = interpret(integracao(), &body, &Policy::default()) else {
        panic!("esperava entrada");
    };
    inbound
        .event
        .metadata
        .get("push_name")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[test]
fn push_name_of_an_individual_contact_goes_to_metadata_without_surrounding_spaces() {
    let dados = message_with_name(
        json!("  Ana Cliente  "),
        false,
        "5511999999999@s.whatsapp.net",
    );
    assert_eq!(push_name_of(dados).as_deref(), Some("Ana Cliente"));
}

#[test]
fn push_name_of_a_message_from_our_own_device_is_not_kept() {
    let body = body(
        "messages.upsert",
        message_with_name(json!("Dono da Loja"), true, "5511999999999@s.whatsapp.net"),
    );
    let policy = Policy {
        accept_from_me: true,
    };
    let Decision::Inbound(inbound) = interpret(integracao(), &body, &policy) else {
        panic!("esperava entrada");
    };
    assert!(inbound.event.metadata.get("push_name").is_none());
}

#[test]
fn group_push_name_is_not_kept_because_it_is_the_sender_name() {
    let dados = message_with_name(json!("Membro do Grupo"), false, "120363000000@g.us");
    assert_eq!(push_name_of(dados), None);
}

#[test]
fn without_push_name_or_with_only_spaces_the_field_does_not_exist() {
    let without_name = message_with_name(Value::Null, false, "5511999999999@s.whatsapp.net");
    assert_eq!(push_name_of(without_name), None);
    let only_spaces = message_with_name(json!("   "), false, "5511999999999@s.whatsapp.net");
    assert_eq!(push_name_of(only_spaces), None);
}
