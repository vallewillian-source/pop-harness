//! Entrada do WhatsApp: o corpo do webhook vira evento normalizado (HR-26).
//!
//! Nada de rede aqui: são os corpos que a Evolution entrega, incluindo os que o POP Atende já trata
//! hoje (eco da própria mensagem, grupo, figurinha, legenda em imagem, documento PDF).

use poph_evolution::webhook::{Decision, IgnoreReason, Policy, check_secret, interpret};
use poph_evolution::{WebhookError, embedded_attachment};
use poph_types::{InboundAttachment, IntegrationId, Secret, StateEvent};
use serde_json::{Value, json};

fn integracao() -> IntegrationId {
    IntegrationId::generate()
}

fn corpo(evento: &str, data: Value) -> Value {
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
fn texto_simples_vira_evento_normalizado() {
    let corpo = corpo(
        "messages.upsert",
        mensagem(json!({"conversation": "quero um orçamento de dedetização"})),
    );
    let Decision::Inbound(entrada) = interpret(integracao(), &corpo, &Policy::default()) else {
        panic!("esperava entrada");
    };
    assert_eq!(entrada.event.id_externo, "3EB0A1B2C3D4");
    assert_eq!(entrada.event.peer, "5511999999999");
    assert_eq!(entrada.event.text, "quero um orçamento de dedetização");
    assert_eq!(entrada.event.occurred_at, 1_795_000_000_000);
    assert_eq!(entrada.event.attachments.len(), 0);
    assert!(entrada.attachments.is_empty());
    assert_eq!(entrada.event.validar(), Ok(()));
}

#[test]
fn texto_estendido_tambem_e_lido() {
    let corpo = corpo(
        "messages.upsert",
        mensagem(json!({"extendedTextMessage": {"text": "e o prazo?"}})),
    );
    let Decision::Inbound(entrada) = interpret(integracao(), &corpo, &Policy::default()) else {
        panic!("esperava entrada");
    };
    assert_eq!(entrada.event.text, "e o prazo?");
}

#[test]
fn imagem_sem_legenda_entra_como_anexo_a_baixar() {
    let corpo = corpo(
        "messages.upsert",
        mensagem(json!({"imageMessage": {
            "mimetype": "image/jpeg",
            "fileLength": 204_800,
        }})),
    );
    let Decision::Inbound(entrada) = interpret(integracao(), &corpo, &Policy::default()) else {
        panic!("esperava entrada");
    };
    assert_eq!(entrada.event.text, "");
    assert_eq!(entrada.attachments.len(), 1);
    assert_eq!(entrada.attachments[0].mime, "image/jpeg");
    assert_eq!(entrada.attachments[0].declared_size, Some(204_800));
    assert_eq!(entrada.attachments[0].name, "imagem-3EB0A1B2C3D4.jpg");
}

#[test]
fn imagem_com_legenda_leva_o_texto_junto() {
    let corpo = corpo(
        "messages.upsert",
        mensagem(json!({"imageMessage": {
            "mimetype": "image/png",
            "caption": "essa é a etiqueta do produto",
        }})),
    );
    let Decision::Inbound(entrada) = interpret(integracao(), &corpo, &Policy::default()) else {
        panic!("esperava entrada");
    };
    assert_eq!(entrada.event.text, "essa é a etiqueta do produto");
    assert_eq!(entrada.attachments.len(), 1);
}

#[test]
fn pdf_entra_com_nome_original() {
    let corpo = corpo(
        "messages.upsert",
        mensagem(json!({"documentMessage": {
            "mimetype": "application/pdf",
            "fileName": "orcamento.pdf",
            "fileLength": "524288",
        }})),
    );
    let Decision::Inbound(entrada) = interpret(integracao(), &corpo, &Policy::default()) else {
        panic!("esperava entrada");
    };
    assert_eq!(entrada.attachments.len(), 1);
    assert_eq!(entrada.attachments[0].name, "orcamento.pdf");
    assert_eq!(entrada.attachments[0].declared_size, Some(524_288));
}

#[test]
fn documento_de_tipo_nao_suportado_e_recusado_com_motivo() {
    let corpo = corpo(
        "messages.upsert",
        mensagem(json!({"documentMessage": {
            "mimetype": "application/zip",
            "fileName": "pacote.zip",
        }})),
    );
    assert_eq!(
        interpret(integracao(), &corpo, &Policy::default()),
        Decision::Ignored(IgnoreReason::MimeNotSupported("application/zip".to_owned()))
    );
}

/// Tabela do MX-45: cada tipo fora do escopo vira mensagem de texto com o texto de histórico, sem
/// baixar nada (nenhum anexo a baixar).
#[test]
fn midia_fora_do_escopo_vira_texto_de_historico_sem_download() {
    let casos: [(Value, poph_types::UnsupportedContent); 11] = [
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
    for (message, tipo) in casos {
        let corpo = corpo("messages.upsert", mensagem(message.clone()));
        match interpret(integracao(), &corpo, &Policy::default()) {
            Decision::Inbound(entrada) => {
                assert_eq!(entrada.event.text, tipo.history_text(), "{message}");
                assert!(entrada.attachments.is_empty(), "não baixa mídia: {message}");
                assert!(entrada.event.attachments.is_empty(), "{message}");
            }
            outra => panic!("esperava texto de histórico para {message}, veio {outra:?}"),
        }
    }
}

#[test]
fn legenda_de_video_nao_entra_no_historico_como_no_python() {
    let corpo = corpo(
        "messages.upsert",
        mensagem(json!({"videoMessage": {"mimetype": "video/mp4", "caption": "olha isso"}})),
    );
    match interpret(integracao(), &corpo, &Policy::default()) {
        Decision::Inbound(entrada) => assert_eq!(
            entrada.event.text,
            poph_types::UnsupportedContent::Video.history_text()
        ),
        outra => panic!("esperava entrada, veio {outra:?}"),
    }
}

#[test]
fn eco_de_midia_fora_do_escopo_do_aparelho_continua_descartado() {
    // Com o módulo Matrix ligado (`aceitar_de_mim`), o áudio que a loja manda do celular não é do
    // cliente: não pode virar "o cliente enviou um áudio".
    let mut data = mensagem(json!({"audioMessage": {"mimetype": "audio/ogg"}}));
    data["key"]["fromMe"] = json!(true);
    let corpo = corpo("messages.upsert", data);
    let politica = Policy {
        accept_from_me: true,
    };
    assert_eq!(
        interpret(integracao(), &corpo, &politica),
        Decision::Ignored(IgnoreReason::AttachmentNotSupported("audio".to_owned()))
    );
}

#[test]
fn evento_sem_conteudo_conhecido_continua_descartado() {
    // Voto em enquete, álbum e tipo novo do WhatsApp: nada disso é mensagem do cliente com
    // conteúdo, e nenhum deles pode derrubar a entrega.
    for message in [
        json!({"pollUpdateMessage": {"pollCreationMessageKey": {}}}),
        json!({"albumMessage": {"expectedImageCount": 2}}),
        json!({"tipoQueNaoExisteAinda": {"x": 1}}),
    ] {
        let corpo = corpo("messages.upsert", mensagem(message.clone()));
        assert_eq!(
            interpret(integracao(), &corpo, &Policy::default()),
            Decision::Ignored(IgnoreReason::NoContent),
            "{message}"
        );
    }
}

#[test]
fn eco_da_propria_mensagem_e_descartado() {
    let mut data = mensagem(json!({"conversation": "resposta do robô"}));
    data["key"]["fromMe"] = json!(true);
    let corpo = corpo("messages.upsert", data);
    assert_eq!(
        interpret(integracao(), &corpo, &Policy::default()),
        Decision::Ignored(IgnoreReason::FromMe)
    );
    // Aceitar de mim é do chamador: o servidor liga para a org com o módulo Matrix ativo (MX-41).
    let politica = Policy {
        accept_from_me: true,
    };
    assert!(matches!(
        interpret(integracao(), &corpo, &politica),
        Decision::Inbound(_)
    ));
}

#[test]
fn grupo_usa_o_jid_como_destino_e_separa_quem_escreveu() {
    let mut data = mensagem(json!({"conversation": "bom dia, pessoal"}));
    data["key"]["remoteJid"] = json!("120363000000000000@g.us");
    data["key"]["participant"] = json!("5511888888888@s.whatsapp.net");
    let corpo = corpo("messages.upsert", data);
    let Decision::Inbound(entrada) = interpret(integracao(), &corpo, &Policy::default()) else {
        panic!("esperava entrada");
    };
    assert_eq!(entrada.event.peer, "120363000000000000@g.us");
    assert_eq!(
        entrada.event.remetente.as_deref(),
        Some("5511888888888@s.whatsapp.net")
    );
}

#[test]
fn jid_com_dispositivo_perde_o_sufixo() {
    let mut data = mensagem(json!({"conversation": "oi"}));
    data["key"]["remoteJid"] = json!("5511999999999:12@s.whatsapp.net");
    let corpo = corpo("messages.upsert", data);
    let Decision::Inbound(entrada) = interpret(integracao(), &corpo, &Policy::default()) else {
        panic!("esperava entrada");
    };
    assert_eq!(entrada.event.peer, "5511999999999");
}

#[test]
fn mensagem_sem_id_ou_sem_remetente_e_descartada() {
    let mut sem_id = mensagem(json!({"conversation": "oi"}));
    sem_id["key"].as_object_mut().unwrap().remove("id");
    assert_eq!(
        interpret(
            integracao(),
            &corpo("messages.upsert", sem_id),
            &Policy::default()
        ),
        Decision::Ignored(IgnoreReason::MissingId)
    );

    let mut sem_jid = mensagem(json!({"conversation": "oi"}));
    sem_jid["key"].as_object_mut().unwrap().remove("remoteJid");
    assert_eq!(
        interpret(
            integracao(),
            &corpo("messages.upsert", sem_jid),
            &Policy::default()
        ),
        Decision::Ignored(IgnoreReason::MissingSender)
    );
}

#[test]
fn mensagem_sem_hora_valida_e_descartada() {
    let mut data = mensagem(json!({"conversation": "oi"}));
    data["messageTimestamp"] = json!("nao-e-numero");
    assert_eq!(
        interpret(
            integracao(),
            &corpo("messages.upsert", data),
            &Policy::default()
        ),
        Decision::Ignored(IgnoreReason::MissingTimestamp)
    );
}

#[test]
fn evento_desconhecido_e_ignorado_com_o_nome() {
    let corpo = corpo(
        "presence.update",
        json!({"id": "5511999999999@s.whatsapp.net"}),
    );
    assert_eq!(
        interpret(integracao(), &corpo, &Policy::default()),
        Decision::Ignored(IgnoreReason::UnknownEvent("presence.update".to_owned()))
    );
}

#[test]
fn estado_de_conexao_vira_evento_de_estado() {
    let aberto = corpo("connection.update", json!({"state": "open"}));
    assert!(matches!(
        interpret(integracao(), &aberto, &Policy::default()),
        Decision::State(StateEvent::Connected { .. })
    ));
    let fechado = corpo("connection.update", json!({"state": "close"}));
    assert!(matches!(
        interpret(integracao(), &fechado, &Policy::default()),
        Decision::State(StateEvent::Disconnected { .. })
    ));
    let qr = corpo(
        "qrcode.updated",
        json!({"qrcode": {"base64": "data:image/png;base64,AAAA"}}),
    );
    let Decision::State(StateEvent::Qr { code: codigo, .. }) =
        interpret(integracao(), &qr, &Policy::default())
    else {
        panic!("esperava QR");
    };
    assert_eq!(codigo, "data:image/png;base64,AAAA");
}

#[test]
fn segredo_do_webhook_e_conferido() {
    let segredo = Secret::new("segredo-do-webhook".to_owned());
    assert_eq!(check_secret(&segredo, Some("segredo-do-webhook")), Ok(()));
    assert_eq!(
        check_secret(&segredo, Some("outro")),
        Err(WebhookError::SecretMismatch)
    );
    assert_eq!(
        check_secret(&segredo, None),
        Err(WebhookError::SecretMissing)
    );
    // Sem segredo configurado, nada é aceito.
    assert_eq!(
        check_secret(&Secret::new(String::new()), Some("qualquer")),
        Err(WebhookError::NoSecretConfigured)
    );
}

#[test]
fn anexo_embutido_leva_o_conteudo_em_base64() {
    let anexo = embedded_attachment("nota.pdf", "application/pdf", b"%PDF-1.4");
    let InboundAttachment::Bytes {
        name: nome,
        mime,
        data_base64: dados_base64,
    } = anexo
    else {
        panic!("esperava bytes");
    };
    assert_eq!(nome, "nota.pdf");
    assert_eq!(mime, "application/pdf");
    assert!(!dados_base64.is_empty());
}

#[test]
fn reentrega_do_mesmo_webhook_produz_o_mesmo_id_externo() {
    // É o que a `inbox` (HR-12) usa como chave de idempotência: `(integração, id externo)`. Se o
    // Evolution reenviar a mesma entrega, o segundo evento cai na chave única e não gera turno.
    let integracao = integracao();
    let corpo = corpo(
        "messages.upsert",
        mensagem(json!({"conversation": "oi, preciso de ajuda"})),
    );
    let Decision::Inbound(primeira) = interpret(integracao, &corpo, &Policy::default()) else {
        panic!("esperava entrada");
    };
    let Decision::Inbound(segunda) = interpret(integracao, &corpo, &Policy::default()) else {
        panic!("esperava entrada");
    };
    assert_eq!(primeira.event.id_externo, segunda.event.id_externo);
    assert_eq!(primeira.event.peer, segunda.event.peer);
    assert_eq!(primeira.event.occurred_at, segunda.event.occurred_at);
}

// ───────────────────────── nome do cliente (MX-32) ─────────────────────────

/// Mensagem recebida de contato individual, com `pushName` no `data` (como a Evolution manda).
fn mensagem_com_nome(nome: Value, de_mim: bool, jid: &str) -> Value {
    let mut dados = mensagem(json!({"conversation": "oi"}));
    dados["key"]["remoteJid"] = json!(jid);
    dados["key"]["fromMe"] = json!(de_mim);
    dados["pushName"] = nome;
    dados
}

fn push_name_da(dados: Value) -> Option<String> {
    let corpo = corpo("messages.upsert", dados);
    let Decision::Inbound(entrada) = interpret(integracao(), &corpo, &Policy::default()) else {
        panic!("esperava entrada");
    };
    entrada
        .event
        .metadata
        .get("push_name")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[test]
fn push_name_de_contato_individual_vai_para_os_metadados_sem_espaco_nas_pontas() {
    let dados = mensagem_com_nome(
        json!("  Ana Cliente  "),
        false,
        "5511999999999@s.whatsapp.net",
    );
    assert_eq!(push_name_da(dados).as_deref(), Some("Ana Cliente"));
}

#[test]
fn push_name_de_mensagem_do_proprio_aparelho_nao_vai() {
    // `fromMe` traz o nome do dono da instância, não o do cliente. Com a política que aceita o
    // próprio aparelho, a mensagem chega ao evento, e mesmo assim o nome não pode ir junto.
    let corpo = corpo(
        "messages.upsert",
        mensagem_com_nome(json!("Dono da Loja"), true, "5511999999999@s.whatsapp.net"),
    );
    let politica = Policy {
        accept_from_me: true,
    };
    let Decision::Inbound(entrada) = interpret(integracao(), &corpo, &politica) else {
        panic!("esperava entrada");
    };
    assert!(entrada.event.metadata.get("push_name").is_none());
}

#[test]
fn push_name_de_grupo_nao_vai_porque_e_o_nome_de_quem_escreveu() {
    let dados = mensagem_com_nome(json!("Membro do Grupo"), false, "120363000000@g.us");
    assert_eq!(push_name_da(dados), None);
}

#[test]
fn sem_push_name_ou_com_so_espaco_o_campo_nao_existe() {
    let sem = mensagem_com_nome(Value::Null, false, "5511999999999@s.whatsapp.net");
    assert_eq!(push_name_da(sem), None);
    let so_espaco = mensagem_com_nome(json!("   "), false, "5511999999999@s.whatsapp.net");
    assert_eq!(push_name_da(so_espaco), None);
}
