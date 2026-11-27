use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use poph_matrix::asset::{self, ASSET_BYTES_LIMIT, ATTACHMENT_BLOCK, AttachmentForAsset};
use poph_matrix::client::{ClientConfig, MatrixClient};
use poph_matrix::expiration::{
    self, AssetToExpire, CATEGORY_FIELD, EXPIRED_BLOCK, NAME_FIELD, SENT_AT_FIELD,
};
use poph_matrix::rooms::{
    ConversationToRoom, PortError, Provisioner, RoomStored, RoomsPort, StoredState,
};
use poph_matrix::users::GhostIds;
use poph_types::{AgentId, ConversationId, MatrixConfig, OrgId, Secret};
use serde_json::json;

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
        .unwrap_or_else(|| {
            panic!(
                "{name} ausente em {path}. Para a MX-46 é preciso um usuário **administrador** do \
                 Synapse de desenvolvimento e o token de acesso dele em POPH_MATRIX_DEV_ADMIN_TOKEN."
            )
        })
}

#[tokio::test]
#[ignore = "precisa do Synapse de desenvolvimento da MX-01 no ar, com token de administrador"]
async fn expired_attachment_is_redacted_in_room_and_deleted_from_synapse() {
    let base = credential("POPH_MATRIX_DEV_BASE_URL");
    let server_name = credential("POPH_MATRIX_DEV_SERVER_NAME");
    let as_token = credential("POPH_MATRIX_DEV_HARNESS_AS_TOKEN");
    let admin_token = credential("POPH_MATRIX_DEV_ADMIN_TOKEN");

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
            Secret::new(as_token),
        )
        .expect("configuração do cliente"),
    )
    .expect("cliente");
    let ids = GhostIds::new("ph", server_name.clone()).expect("ids dos fantasmas");

    let mark = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio")
        .as_secs();
    let peer = format!("5521{mark}");
    let agent_id = AgentId::generate();
    let givens = ConversationToRoom {
        org: OrgId::generate(),
        conversation: ConversationId::generate(),
        client: peer.clone(),
        client_name: Some(format!("Cliente MX-46 {mark}")),
        agent: agent_id,
        agent_name: Some("Agente de Expiração".to_owned()),
        channel: "whatsapp".to_owned(),
        department: None,
        closed: false,
        backfill_cutoff: None,
    };
    let provisioner = Provisioner::of_config(client.clone(), MemoryPort::default(), &config)
        .expect("provisionador");
    let ready = provisioner
        .provision(&givens)
        .await
        .expect("sala criada contra o Synapse real");
    let room = ready.room_id.clone();
    let agent = ids
        .agent(&agent_id.to_string())
        .expect("fantasma do agente");
    let client_ghost = ids.client(&peer).expect("fantasma do cliente");

    let name = format!("foto-{mark}.png");
    let decided_asset = asset::decide(&AttachmentForAsset {
        id: &format!("anx-mx46-{mark}"),
        name: &name,
        mime: "image/png",
        size_bytes: PNG.len() as u64,
        source: "whatsapp",
    })
    .expect("decide a mídia");
    let published = asset::send(&client, &client_ghost, &room, &decided_asset, PNG)
        .await
        .expect("publica a mídia na sala");

    assert!(
        published.mxc.starts_with("mxc://"),
        "a subida devolveu um mxc: {}",
        published.mxc
    );
    let before = client
        .read_event(client_ghost.user_id(), &room, &published.event)
        .await
        .expect("lê o evento publicado");
    assert!(before["content"][ATTACHMENT_BLOCK]["id"].is_string());
    let bytes_before = client
        .download_asset(client_ghost.user_id(), &published.mxc, ASSET_BYTES_LIMIT)
        .await
        .expect("antes de expirar, a mídia desce");
    assert_eq!(bytes_before, PNG, "os bytes sobem e descem inteiros");

    let to_expire = AssetToExpire {
        id: &decided_asset.id_attachment,
        name: &name,
        mime: "image/png",
        category: "image",
        size_bytes: PNG.len() as u64,
        sent_at: 1_797_900_000_000,
    };
    expiration::redact(&client, &agent, &room, &published.event, &to_expire)
        .await
        .expect("o Synapse aceita a redação");

    let after = client
        .read_event(agent.user_id(), &room, &published.event)
        .await
        .expect("lê o evento redigido");
    let redaction = &after["unsigned"]["redacted_because"];
    assert_eq!(
        redaction["sender"],
        json!(agent.user_id()),
        "a redação foi assinada pelo fantasma do agente"
    );
    let block = &redaction["content"][EXPIRED_BLOCK];
    assert_eq!(
        block[NAME_FIELD],
        json!(name),
        "o aviso mostra o nome do arquivo"
    );
    assert_eq!(block[CATEGORY_FIELD], json!("image"));
    assert_eq!(block[SENT_AT_FIELD], json!("2026-12-22T00:40:00Z"));
    assert_eq!(block["mimetype"], json!("image/png"));
    assert_eq!(block["size"], json!(PNG.len() as u64));
    assert!(
        after["content"]["url"].is_null(),
        "o conteúdo do evento foi apagado pela redação: {}",
        after["content"]
    );

    expiration::redact(&client, &agent, &room, &published.event, &to_expire)
        .await
        .expect("a segunda redação do mesmo evento não é erro");
    let again = client
        .read_event(agent.user_id(), &room, &published.event)
        .await
        .expect("lê de novo");
    assert_eq!(
        again["unsigned"]["redacted_because"]["content"][EXPIRED_BLOCK],
        redaction["content"][EXPIRED_BLOCK],
        "o aviso continua o mesmo"
    );

    let deleted = expiration::delete(&client, &Secret::new(admin_token), &published.mxc)
        .await
        .expect("o Synapse de dev aceita o apagamento pela API admin");
    assert!(deleted, "o arquivo existia e foi apagado");

    let error = client
        .download_asset(client_ghost.user_id(), &published.mxc, ASSET_BYTES_LIMIT)
        .await
        .expect_err("mídia apagada não desce");
    println!("depois do apagamento, o download responde: {error}");

    println!(
        "MX-46 no Synapse real: evento {} redigido na sala {room} e arquivo {} apagado",
        published.event, published.mxc
    );
    println!("servidor: {server_name}");
}
