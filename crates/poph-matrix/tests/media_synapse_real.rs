use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use poph_matrix::asset::{self, ATTACHMENT_BLOCK, AttachmentForAsset, MSGTYPE_FILE, MSGTYPE_IMAGE};
use poph_matrix::client::{ClientConfig, MatrixClient};
use poph_matrix::rooms::{
    ConversationToRoom, PortError, Provisioner, RoomStored, RoomsPort, StoredState,
};
use poph_matrix::{AssetError, CallError};
use poph_types::{AgentId, ConversationId, MatrixConfig, OrgId, Secret};

fn credential(name: &str) -> String {
    let path = std::env::var("HOME").expect("HOME") + "/.cache/poph-matrix/credenciais.env";
    let content = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "não consegui ler {path} ({error}): suba o ambiente com \
             `bash apps/pop-harness/scripts/matrix-dev.sh subir`"
        )
    });
    content
        .lines()
        .filter_map(|line| line.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().trim_matches('"').to_owned())
        .unwrap_or_else(|| panic!("{name} ausente em {path}"))
}

#[derive(Clone, Default)]
struct MemoryPort(Arc<Mutex<BTreeMap<(OrgId, ConversationId), RoomStored>>>);

impl RoomsPort for MemoryPort {
    async fn room(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> Result<Option<RoomStored>, PortError> {
        Ok(self
            .0
            .lock()
            .expect("trava")
            .get(&(org, conversation))
            .cloned())
    }

    async fn ensure_room(
        &self,
        org: OrgId,
        conversation: ConversationId,
        room_id: &str,
    ) -> Result<RoomStored, PortError> {
        let mut guard = self.0.lock().expect("trava");
        let room = guard
            .entry((org, conversation))
            .or_insert_with(|| RoomStored {
                room_id: room_id.to_owned(),
                state: StoredState::Creating,
            });
        Ok(room.clone())
    }

    async fn mark_ready(
        &self,
        org: OrgId,
        conversation: ConversationId,
        _mirrored_name: Option<&str>,
    ) -> Result<RoomStored, PortError> {
        let mut guard = self.0.lock().expect("trava");
        match guard.get_mut(&(org, conversation)) {
            Some(room) => {
                room.state = StoredState::Ready;
                Ok(room.clone())
            }
            None => Err(PortError::fresh("sala ausente na porta")),
        }
    }

    async fn had_messages_before(
        &self,
        _org: OrgId,
        _conversation: ConversationId,
        _cutoff: i64,
    ) -> Result<bool, PortError> {
        Ok(false)
    }
}

const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

#[tokio::test]
#[ignore = "precisa do Synapse de desenvolvimento da MX-01 no ar"]
async fn client_image_and_pdf_appear_in_real_synapse_room() {
    let base = credential("POPH_MATRIX_DEV_BASE_URL");
    let server_name = credential("POPH_MATRIX_DEV_SERVER_NAME");
    let token = credential("POPH_MATRIX_DEV_HARNESS_AS_TOKEN");
    let config = MatrixConfig {
        enabled: true,
        homeserver_url: base,
        server_name: server_name.clone(),
        prefix: "ph".to_owned(),
        ..MatrixConfig::default()
    };
    let client = MatrixClient::new(
        ClientConfig::new(
            config.homeserver_url.clone(),
            server_name.clone(),
            Secret::new(token),
        )
        .expect("configuração do cliente"),
    )
    .expect("cliente");
    let provisioner = Provisioner::of_config(client.clone(), MemoryPort::default(), &config)
        .expect("provisionador");

    let mark = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio")
        .as_secs();
    let peer = format!("5521{mark}");
    let givens = ConversationToRoom {
        org: OrgId::generate(),
        conversation: ConversationId::generate(),
        client: peer.clone(),
        client_name: Some(format!("Cliente MX-43 {mark}")),
        agent: AgentId::generate(),
        agent_name: Some("Agente de Teste".to_owned()),
        channel: "whatsapp".to_owned(),
        department: None,
        closed: false,
        backfill_cutoff: None,
    };
    let ready = provisioner
        .provision(&givens)
        .await
        .expect("sala criada contra o Synapse real");
    let room = ready.room_id.clone();
    let ids = poph_matrix::users::GhostIds::new("ph", server_name.clone()).expect("ids");
    let sender = ids.client(&peer).expect("fantasma do cliente");

    let pdf =
        b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n"
            .to_vec();

    let mut events = Vec::new();
    for (name, mime, bytes) in [
        ("foto.png", "image/png", PNG.to_vec()),
        ("laudo.pdf", "application/pdf", pdf),
    ] {
        let decided_asset = asset::decide(&AttachmentForAsset {
            id: &format!("anx-mx43-{mark}-{name}"),
            name,
            mime,
            size_bytes: bytes.len() as u64,
            source: "whatsapp",
        })
        .expect("decide a mídia");
        let sent = asset::send(&client, &sender, &room, &decided_asset, &bytes)
            .await
            .unwrap_or_else(|error| panic!("[{name}] o Synapse real recusou a mídia ({error:?})"));
        let read = client
            .read_event(sender.user_id(), &room, &sent.event)
            .await
            .unwrap_or_else(|error| panic!("[{name}] o evento não está na sala ({error:?})"));
        events.push((name, read));
    }

    for (name, event) in &events {
        assert_eq!(
            event["type"],
            serde_json::json!("m.room.message"),
            "[{name}] tipo do evento"
        );
        assert_eq!(
            event["sender"],
            serde_json::json!(sender.user_id()),
            "[{name}] quem assina é o fantasma do cliente"
        );
        let content = &event["content"];
        let expected = if *name == "foto.png" {
            MSGTYPE_IMAGE
        } else {
            MSGTYPE_FILE
        };
        assert_eq!(content["msgtype"], serde_json::json!(expected), "[{name}]");
        assert_eq!(content["body"], serde_json::json!(name), "[{name}]");
        assert!(
            content["url"]
                .as_str()
                .is_some_and(|url| url.starts_with("mxc://")),
            "[{name}] mxc:// no conteúdo: {}",
            content["url"]
        );
        assert!(
            content[ATTACHMENT_BLOCK]["id"].is_string(),
            "[{name}] bloco do anexo com id: {}",
            content[ATTACHMENT_BLOCK]
        );
    }

    assert_eq!(
        events[0].1["content"]["msgtype"],
        serde_json::json!(MSGTYPE_IMAGE)
    );
    assert_eq!(
        events[1].1["content"]["msgtype"],
        serde_json::json!(MSGTYPE_FILE)
    );
    println!(
        "MX-43 no Synapse real: imagem e PDF na sala {room} pelo fantasma {}",
        sender.user_id()
    );
}

#[tokio::test]
#[ignore = "precisa do Synapse de desenvolvimento da MX-01 no ar"]
async fn room_attachment_downloads_whole_through_matrix_client() {
    let base = credential("POPH_MATRIX_DEV_BASE_URL");
    let server_name = credential("POPH_MATRIX_DEV_SERVER_NAME");
    let token = credential("POPH_MATRIX_DEV_HARNESS_AS_TOKEN");
    let config = MatrixConfig {
        enabled: true,
        homeserver_url: base,
        server_name: server_name.clone(),
        prefix: "ph".to_owned(),
        ..MatrixConfig::default()
    };
    let client = MatrixClient::new(
        ClientConfig::new(
            config.homeserver_url.clone(),
            server_name.clone(),
            Secret::new(token),
        )
        .expect("configuração do cliente"),
    )
    .expect("cliente");
    let provisioner = Provisioner::of_config(client.clone(), MemoryPort::default(), &config)
        .expect("provisionador");

    let mark = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio")
        .as_secs();
    let peer = format!("5521{mark}");
    let givens = ConversationToRoom {
        org: OrgId::generate(),
        conversation: ConversationId::generate(),
        client: peer.clone(),
        client_name: Some(format!("Cliente MX-44 {mark}")),
        agent: AgentId::generate(),
        agent_name: Some("Agente de Teste".to_owned()),
        channel: "whatsapp".to_owned(),
        department: None,
        closed: false,
        backfill_cutoff: None,
    };
    let ready = provisioner
        .provision(&givens)
        .await
        .expect("sala criada contra o Synapse real");
    let ids = poph_matrix::users::GhostIds::new("ph", server_name.clone()).expect("ids");
    let sender = ids
        .agent(&givens.agent.to_string())
        .expect("fantasma do agente");

    let decided_asset = asset::decide(&AttachmentForAsset {
        id: &format!("anx-mx44-{mark}"),
        name: "foto.png",
        mime: "image/png",
        size_bytes: PNG.len() as u64,
        source: "whatsapp",
    })
    .expect("decide a mídia");
    let sent = asset::send(&client, &sender, &ready.room_id, &decided_asset, PNG)
        .await
        .expect("o Synapse real aceitou a mídia na sala");
    let read = client
        .read_event(sender.user_id(), &ready.room_id, &sent.event)
        .await
        .expect("o evento está na sala");
    let mxc = read["content"]["url"]
        .as_str()
        .expect("o evento traz o mxc://")
        .to_owned();

    let human = format!("@atendente-mx44-{mark}:{server_name}");
    let refusal = client
        .download_asset(&human, &mxc, asset::ASSET_BYTES_LIMIT)
        .await
        .expect_err("o Application Service não personifica quem está fora do namespace");
    assert!(
        !refusal.transient(),
        "recusa por namespace é definitiva: {refusal:?}"
    );
    match &refusal {
        AssetError::Call(CallError::Response {
            status: 403,
            errcode,
            ..
        }) => assert_eq!(errcode.as_deref(), Some("M_FORBIDDEN")),
        other => panic!("esperava 403 M_FORBIDDEN no disfarce de humano, veio {other:?}"),
    }

    let downloaded = client
        .download_asset(sender.user_id(), &mxc, asset::ASSET_BYTES_LIMIT)
        .await
        .unwrap_or_else(|error| panic!("o Synapse real não entregou a mídia ({error:?})"));
    assert_eq!(
        downloaded, PNG,
        "o conteúdo desce inteiro pelo fantasma do agente"
    );

    let refused = client.download_asset(sender.user_id(), &mxc, 10).await;
    assert!(
        matches!(refused, Err(AssetError::Large { cap: 10, .. })),
        "acima do teto o cliente recusa: {refused:?}"
    );

    assert!(matches!(
        client
            .download_asset(sender.user_id(), "https://exemplo/x", 1024)
            .await,
        Err(AssetError::Address(_))
    ));

    println!(
        "MX-44 no Synapse real: mídia da sala {room} baixada inteira",
        room = ready.room_id
    );
}

const AUDIO_OGG: &[u8] = &[
    0x4f, 0x67, 0x67, 0x53, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x4f, 0x70, 0x75, 0x73, 0x48, 0x65, 0x61, 0x64, 0x01, 0x02, 0x38, 0x01,
    0x80, 0xbb, 0x00, 0x00,
];

const VIDEO_MP4: &[u8] = &[
    0x00, 0x00, 0x00, 0x18, 0x66, 0x74, 0x79, 0x70, 0x69, 0x73, 0x6f, 0x6d, 0x00, 0x00, 0x02, 0x00,
    0x69, 0x73, 0x6f, 0x6d, 0x69, 0x73, 0x6f, 0x32,
];

#[tokio::test]
#[ignore = "precisa do Synapse de desenvolvimento da MX-01 no ar"]
async fn attendant_audio_and_video_upload_and_download_whole() {
    let base = credential("POPH_MATRIX_DEV_BASE_URL");
    let server_name = credential("POPH_MATRIX_DEV_SERVER_NAME");
    let token = credential("POPH_MATRIX_DEV_HARNESS_AS_TOKEN");
    let config = MatrixConfig {
        enabled: true,
        homeserver_url: base,
        server_name: server_name.clone(),
        prefix: "ph".to_owned(),
        ..MatrixConfig::default()
    };
    let client = MatrixClient::new(
        ClientConfig::new(
            config.homeserver_url.clone(),
            server_name.clone(),
            Secret::new(token),
        )
        .expect("configuração do cliente"),
    )
    .expect("cliente");
    let provisioner = Provisioner::of_config(client.clone(), MemoryPort::default(), &config)
        .expect("provisionador");

    let mark = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio")
        .as_secs();
    let peer = format!("5522{mark}");
    let givens = ConversationToRoom {
        org: OrgId::generate(),
        conversation: ConversationId::generate(),
        client: peer.clone(),
        client_name: Some(format!("Cliente MX-48 {mark}")),
        agent: AgentId::generate(),
        agent_name: Some("Agente de Teste".to_owned()),
        channel: "whatsapp".to_owned(),
        department: None,
        closed: false,
        backfill_cutoff: None,
    };
    let ready = provisioner
        .provision(&givens)
        .await
        .expect("sala criada contra o Synapse real");
    let ids = poph_matrix::users::GhostIds::new("ph", server_name.clone()).expect("ids");
    let sender = ids
        .agent(&givens.agent.to_string())
        .expect("fantasma do agente");

    for (event, name, mime, bytes) in [
        ("m.audio", "voice-message.ogg", "audio/ogg", AUDIO_OGG),
        ("m.video", "video.mp4", "video/mp4", VIDEO_MP4),
    ] {
        let mxc = client
            .send_asset(sender.user_id(), name, mime, bytes)
            .await
            .unwrap_or_else(|error| panic!("[{name}] o Synapse real recusou o upload ({error:?})"));
        let content = serde_json::json!({
            "msgtype": event,
            "body": name,
            "filename": name,
            "url": mxc,
            "info": { "mimetype": mime, "size": bytes.len() },
        });
        let published = client
            .send_event(
                sender.user_id(),
                &ready.room_id,
                "m.room.message",
                &format!("ph-mx48-{mark}-{name}"),
                &content,
            )
            .await
            .unwrap_or_else(|error| panic!("[{name}] o evento não foi publicado ({error:?})"));

        let downloaded = client
            .download_asset(
                sender.user_id(),
                &mxc,
                poph_matrix::asset::ASSET_BYTES_LIMIT,
            )
            .await
            .unwrap_or_else(|error| {
                panic!("[{name}] o Synapse real não entregou a mídia ({error:?})")
            });
        assert_eq!(downloaded, bytes, "[{name}] o conteúdo desce inteiro");

        let kind = poph_files::outbound::classify(&downloaded, name)
            .unwrap_or_else(|error| panic!("[{name}] o tipo baixado foi recusado ({error:?})"));
        let expected_family = if event == "m.audio" {
            poph_files::OutboundFamily::Audio
        } else {
            poph_files::OutboundFamily::Video
        };
        assert_eq!(
            kind.family(),
            expected_family,
            "[{name}] família do conteúdo"
        );
        assert_eq!(kind.mime(), mime, "[{name}] MIME do conteúdo");

        println!(
            "MX-48 no Synapse real: {name} publicado como {published} na sala {} e baixado inteiro",
            ready.room_id
        );
    }
}
