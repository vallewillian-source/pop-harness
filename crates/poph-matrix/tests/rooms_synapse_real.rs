use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use poph_matrix::client::{ClientConfig, MatrixClient};
use poph_matrix::rooms::{
    ConversationToRoom, METADATA_KIND, MIN_AGENT_POWER, PortError, Provisioner, RoomStored,
    RoomsPort, StoredState,
};
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

#[tokio::test]
#[ignore = "precisa do Synapse de desenvolvimento da MX-01 no ar"]
async fn room_is_born_with_name_topic_metadata_and_final_agent_power() {
    let base = credential("POPH_MATRIX_DEV_BASE_URL");
    let server_name = credential("POPH_MATRIX_DEV_SERVER_NAME");
    let token = credential("POPH_MATRIX_DEV_HARNESS_AS_TOKEN");
    let config = MatrixConfig {
        enabled: true,
        homeserver_url: base,
        server_name: server_name.clone(),
        prefix: "ph".to_owned(),
        agent_power: MIN_AGENT_POWER,
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
    let client_name = format!("Cliente MX-31 {mark}");
    let givens = ConversationToRoom {
        org: OrgId::generate(),
        conversation: ConversationId::generate(),
        client: format!("5511{mark}"),
        client_name: Some(client_name.clone()),
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
    let room = &ready.room_id;
    let agent = format!("@ph_ag_{}:{server_name}", givens.agent);

    let name = client
        .read_state(&agent, room, "m.room.name", None)
        .await
        .expect("a sala precisa ter nome contra o Synapse real");
    assert_eq!(name["name"], serde_json::json!(client_name));

    let topico = client
        .read_state(&agent, room, "m.room.topic", None)
        .await
        .expect("a sala precisa ter tópico");
    assert_eq!(topico["topic"], serde_json::json!("Atendimento · whatsapp"));

    let metadata = client
        .read_state(&agent, room, METADATA_KIND, None)
        .await
        .expect("a sala precisa ter o metadata do frontend");
    assert_eq!(metadata["client_name"], serde_json::json!(client_name));

    let powers = client
        .read_state(&agent, room, "m.room.power_levels", None)
        .await
        .expect("a sala precisa ter poderes");
    assert_eq!(
        powers["users"][&agent],
        serde_json::json!(MIN_AGENT_POWER),
        "o poder do agente desce para o configurado depois da criação"
    );
}
