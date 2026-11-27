use std::fmt;
use std::future::Future;

use poph_types::{AgentId, ConversationId, DepartmentId, MatrixConfig, OrgId};
use serde::Serialize;
use serde_json::{Value, json};

use crate::client::{CreateRoom, MatrixClient};
use crate::error::{CallError, Classification};
use crate::users::{DEFAULT_CLIENT_NAME, Ghost, GhostsError, VirtualUsers};

pub const ROOM_KIND: &str = "com.sofacoding.atendimento";

pub const METADATA_KIND: &str = "com.sofacoding.atendimento.metadata";

pub const NAME_KIND: &str = "m.room.name";

pub const NOTICE_WITHOUT_HISTORY: &str = "histórico anterior não foi espelhado";

pub const MARK_ROOM: &str = "c";

pub const CREATION_POWER: u8 = 100;

pub const MIN_AGENT_POWER: u8 = 50;

const POWERS_KIND: &str = "m.room.power_levels";

pub const FRONTEND_INTEGRATIONS: [&str; 4] = ["whatsapp", "site", "api", "email"];

const ALIAS_ATTEMPTS: u8 = 3;

const ERRCODE_ALIAS_IN_USE: &str = "M_ROOM_IN_USE";

const ERRCODE_NOT_FOUND: &str = "M_NOT_FOUND";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationToRoom {
    pub org: OrgId,
    pub conversation: ConversationId,
    pub client: String,
    pub client_name: Option<String>,
    pub agent: AgentId,
    pub agent_name: Option<String>,
    pub channel: String,
    pub department: Option<DepartmentId>,
    pub closed: bool,
    pub backfill_cutoff: Option<i64>,
}

impl ConversationToRoom {
    pub fn room_name(&self) -> String {
        if let Some(name) = self
            .client_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            return name.to_owned();
        }
        let identifier = self.client.trim();
        if identifier.is_empty() {
            return DEFAULT_CLIENT_NAME.to_owned();
        }
        identifier.to_owned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoomMetadata {
    pub conversation_id: String,
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_type: Option<String>,
    pub channel: String,
    pub client_identifier: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub department_id: Option<String>,
    pub status: String,
}

impl RoomMetadata {
    pub fn of_conversation(givens: &ConversationToRoom) -> Self {
        let channel = givens.channel.trim().to_owned();
        let integration = FRONTEND_INTEGRATIONS
            .iter()
            .find(|conhecido| channel.eq_ignore_ascii_case(conhecido))
            .map(|conhecido| (*conhecido).to_owned());
        Self {
            conversation_id: givens.conversation.to_string(),
            agent_id: givens.agent.to_string(),
            integration_type: integration,
            channel,
            client_identifier: givens.client.clone(),
            client_name: givens
                .client_name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
            department_id: givens.department.map(|department| department.to_string()),
            status: if givens.closed { "closed" } else { "active" }.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomReady {
    pub room_id: String,
    pub was_already_ready: bool,
    pub notice_sent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredState {
    Creating,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomStored {
    pub room_id: String,
    pub state: StoredState,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("armazenamento do módulo Matrix falhou: {reason}")]
pub struct PortError {
    pub reason: String,
}

impl PortError {
    pub fn fresh(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

pub trait RoomsPort {
    fn room(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> impl Future<Output = Result<Option<RoomStored>, PortError>> + Send;

    fn ensure_room(
        &self,
        org: OrgId,
        conversation: ConversationId,
        room_id: &str,
    ) -> impl Future<Output = Result<RoomStored, PortError>> + Send;

    fn mark_ready(
        &self,
        org: OrgId,
        conversation: ConversationId,
        mirrored_name: Option<&str>,
    ) -> impl Future<Output = Result<RoomStored, PortError>> + Send;

    fn had_messages_before(
        &self,
        org: OrgId,
        conversation: ConversationId,
        cutoff: i64,
    ) -> impl Future<Output = Result<bool, PortError>> + Send;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RoomError {
    #[error(transparent)]
    Ghosts(#[from] GhostsError),
    #[error(transparent)]
    Call(#[from] CallError),
    #[error(transparent)]
    Armazenamento(#[from] PortError),
    #[error("o alias `{alias}` apareceu em uso e sumiu do diretório em {attempts} tentativas")]
    AliasEmDisputa { alias: String, attempts: u8 },
    #[error("falha interna do módulo Matrix: {reason}")]
    Internal { reason: String },
}

impl RoomError {
    pub fn classification(&self) -> Classification {
        match self {
            Self::Call(error) => error.classification(),
            Self::Ghosts(GhostsError::Call(error)) => error.classification(),
            Self::Ghosts(_) => Classification::Definitive,
            Self::Armazenamento(_) => Classification::Transient,
            Self::AliasEmDisputa { .. } => Classification::Transient,
            Self::Internal { .. } => Classification::Definitive,
        }
    }
}

pub fn localpart_do_alias(prefix: &str, conversation: ConversationId) -> String {
    let body: String = conversation
        .to_string()
        .chars()
        .filter(char::is_ascii_hexdigit)
        .map(|character| character.to_ascii_lowercase())
        .collect();
    format!("{prefix}_{MARK_ROOM}_{body}")
}

pub fn conversation_alias(prefix: &str, server_name: &str, conversation: ConversationId) -> String {
    format!(
        "#{}:{}",
        localpart_do_alias(prefix, conversation),
        server_name
    )
}

fn room_topic(channel: &str) -> String {
    let channel = channel.trim();
    if channel.is_empty() {
        "Atendimento".to_owned()
    } else {
        format!("Atendimento · {channel}")
    }
}

pub struct Provisioner<P: RoomsPort> {
    client: MatrixClient,
    users: VirtualUsers,
    port: P,
    agent_power: u8,
    prefix: String,
    server_name: String,
}

impl<P: RoomsPort> Provisioner<P> {
    pub fn of_config(
        client: MatrixClient,
        port: P,
        config: &MatrixConfig,
    ) -> Result<Self, GhostsError> {
        let users = VirtualUsers::of_config(client.clone(), config)?;
        Ok(Self::with(client, users, port, config.agent_power))
    }

    pub fn with(client: MatrixClient, users: VirtualUsers, port: P, agent_power: u8) -> Self {
        Self {
            prefix: users.ids().prefix().to_owned(),
            server_name: client.server_name().to_owned(),
            client,
            users,
            port,
            agent_power,
        }
    }

    pub async fn agent_ghost(&self, givens: &ConversationToRoom) -> Result<Ghost, RoomError> {
        Ok(self
            .users
            .ensure_agent(
                &givens.agent.to_string(),
                givens.agent_name.as_deref().unwrap_or_default(),
            )
            .await?)
    }

    pub async fn ensure_device_in_room(
        &self,
        givens: &ConversationToRoom,
        room_id: &str,
        device: &Ghost,
    ) -> Result<(), RoomError> {
        let agent = self.agent_ghost(givens).await?;
        let device = self.users.ensure_device(device.clone()).await?;
        let members = self.client.members(agent.user_id(), room_id).await?;
        if members.contains_key(device.user_id()) {
            return Ok(());
        }
        self.client
            .invite(agent.user_id(), room_id, device.user_id())
            .await?;
        self.client.enter(device.user_id(), room_id).await?;
        Ok(())
    }

    pub fn alias(&self, conversation: ConversationId) -> String {
        conversation_alias(&self.prefix, &self.server_name, conversation)
    }

    pub async fn provision(&self, givens: &ConversationToRoom) -> Result<RoomReady, RoomError> {
        if let Some(stored) = self.port.room(givens.org, givens.conversation).await? {
            if stored.state == StoredState::Ready {
                return Ok(RoomReady {
                    room_id: stored.room_id,
                    was_already_ready: true,
                    notice_sent: false,
                });
            }
            return self.finalise(givens, &stored.room_id).await;
        }

        let (_, agent) = self.ghosts(givens).await?;
        let room_in_synapse = self.room_in_synapse(givens, &agent).await?;

        let stored = self
            .port
            .ensure_room(givens.org, givens.conversation, &room_in_synapse)
            .await?;
        self.finalise(givens, &stored.room_id).await
    }

    async fn finalise(
        &self,
        givens: &ConversationToRoom,
        room_id: &str,
    ) -> Result<RoomReady, RoomError> {
        let (client, agent) = self.ghosts(givens).await?;

        self.client.enter(agent.user_id(), room_id).await?;

        let members = self.client.members(agent.user_id(), room_id).await?;
        if !members.contains_key(client.user_id()) {
            self.client
                .invite(agent.user_id(), room_id, client.user_id())
                .await?;
            self.client.enter(client.user_id(), room_id).await?;
        }

        let room_metadata = RoomMetadata::of_conversation(givens);
        let metadata =
            serde_json::to_value(&room_metadata).map_err(|error| RoomError::Internal {
                reason: format!("o metadata da sala não serializou: {error}"),
            })?;
        self.client
            .send_state(agent.user_id(), room_id, METADATA_KIND, None, &metadata)
            .await?;

        let notice_sent = self
            .notify_history(givens, room_id, agent.user_id())
            .await?;

        self.adjust_final_power(room_id, agent.user_id()).await;

        let stored = self
            .port
            .mark_ready(
                givens.org,
                givens.conversation,
                room_metadata.client_name.as_deref(),
            )
            .await?;
        Ok(RoomReady {
            room_id: stored.room_id,
            was_already_ready: false,
            notice_sent,
        })
    }

    async fn notify_history(
        &self,
        givens: &ConversationToRoom,
        room_id: &str,
        sender: &str,
    ) -> Result<bool, RoomError> {
        let Some(cutoff) = givens.backfill_cutoff else {
            return Ok(false);
        };
        if !self
            .port
            .had_messages_before(givens.org, givens.conversation, cutoff)
            .await?
        {
            return Ok(false);
        }
        let txn = format!(
            "ph_hist_{}",
            localpart_do_alias(&self.prefix, givens.conversation)
        );
        self.client
            .send_event(
                sender,
                room_id,
                "m.room.message",
                &txn,
                &json!({
                    "msgtype": "m.notice",
                    "body": NOTICE_WITHOUT_HISTORY,
                }),
            )
            .await?;
        Ok(true)
    }

    async fn ghosts(&self, givens: &ConversationToRoom) -> Result<(Ghost, Ghost), RoomError> {
        let client = self
            .users
            .ensure_client(
                &givens.client,
                givens.client_name.as_deref().unwrap_or_default(),
            )
            .await?;
        let agent = self
            .users
            .ensure_agent(
                &givens.agent.to_string(),
                givens.agent_name.as_deref().unwrap_or_default(),
            )
            .await?;
        Ok((client, agent))
    }

    fn final_power(&self) -> u8 {
        self.agent_power.max(MIN_AGENT_POWER)
    }

    async fn adjust_final_power(&self, room_id: &str, agent: &str) {
        let final_ = self.final_power();
        if final_ >= CREATION_POWER {
            return;
        }
        let mut powers = match self
            .client
            .read_state(agent, room_id, POWERS_KIND, None)
            .await
        {
            Ok(powers) => powers,
            Err(_) => {
                tracing::warn!(
                    target: "poph_matrix::rooms",
                    reason = "poderes_da_sala_ilegiveis",
                    "não consegui ler os poderes da sala; o agente continua com o poder da criação"
                );
                return;
            }
        };
        let current = powers
            .get("users")
            .and_then(|users| users.get(agent))
            .and_then(Value::as_i64);
        if current == Some(i64::from(final_)) {
            return;
        }
        let Some(users) = powers.get_mut("users").and_then(Value::as_object_mut) else {
            tracing::warn!(
                target: "poph_matrix::rooms",
                reason = "poderes_da_sala_sem_usuarios",
                "os poderes da sala não têm `users`; o agente continua com o poder da criação"
            );
            return;
        };
        users.insert(agent.to_owned(), json!(final_));
        if self
            .client
            .send_state(agent, room_id, POWERS_KIND, None, &powers)
            .await
            .is_err()
        {
            tracing::warn!(
                target: "poph_matrix::rooms",
                reason = "rebaixamento_de_poder_falhou",
                "não consegui rebaixar o poder do agente; a sala segue com o poder da criação"
            );
        }
    }

    fn creation_body(&self, givens: &ConversationToRoom, agent: &Ghost) -> CreateRoom {
        let mut users = serde_json::Map::new();
        users.insert(agent.user_id().to_owned(), json!(CREATION_POWER));
        let power = json!({ "users": Value::Object(users) });
        CreateRoom {
            room_alias_name: Some(localpart_do_alias(&self.prefix, givens.conversation)),
            name: Some(givens.room_name()),
            topic: Some(room_topic(&givens.channel)),
            creation_content: Some(json!({ "type": ROOM_KIND })),
            power_level_content_override: Some(power),
            preset: Some("private_chat".to_owned()),
            ..CreateRoom::default()
        }
    }

    async fn room_in_synapse(
        &self,
        givens: &ConversationToRoom,
        agent: &Ghost,
    ) -> Result<String, RoomError> {
        let alias = self.alias(givens.conversation);
        let body = self.creation_body(givens, agent);
        for _ in 0..ALIAS_ATTEMPTS {
            match self.client.create_room(agent.user_id(), &body).await {
                Ok(room) => return Ok(room),
                Err(error) if errcode(&error) == Some(ERRCODE_ALIAS_IN_USE) => {
                    match self.client.resolver_alias(&alias).await {
                        Ok(room) => return Ok(room),
                        Err(error) if errcode(&error) == Some(ERRCODE_NOT_FOUND) => continue,
                        Err(error) => return Err(error.into()),
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(RoomError::AliasEmDisputa {
            alias,
            attempts: ALIAS_ATTEMPTS,
        })
    }
}

impl<P: RoomsPort> fmt::Debug for Provisioner<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Provisionador")
            .field("cliente", &self.client)
            .field("fantasmas", &format_args!("{:?}", self.users.cache()))
            .field("prefixo", &self.prefix)
            .field("server_name", &self.server_name)
            .field("poder_agente", &self.agent_power)
            .finish()
    }
}

fn errcode(error: &CallError) -> Option<&str> {
    match error {
        CallError::Response { errcode, .. } => errcode.as_deref(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation() -> ConversationId {
        "0192f0aa-1111-7abc-8def-0123456789ab"
            .parse()
            .expect("uuid v7")
    }

    #[test]
    fn alias_is_deterministic_and_unique_per_conversation() {
        let a = conversation();
        let other_one: ConversationId = "0192f0aa-2222-7abc-8def-0123456789ab"
            .parse()
            .expect("uuid v7");
        assert_eq!(
            conversation_alias("ph", "matrix-dev.local", a),
            conversation_alias("ph", "matrix-dev.local", a)
        );
        assert_ne!(
            conversation_alias("ph", "matrix-dev.local", a),
            conversation_alias("ph", "matrix-dev.local", other_one)
        );
        let localpart = localpart_do_alias("ph", a);
        assert!(
            localpart
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "{localpart}"
        );
        assert!(!localpart.contains('-'), "{localpart}");
        assert!(localpart.starts_with("ph_c_"), "{localpart}");
    }

    #[test]
    fn short_prefix_generates_no_repeated_alias_between_conversations() {
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..64 {
            let id = ConversationId::generate();
            let alias = localpart_do_alias("p", id);
            assert!(seen.insert(alias), "alias repetido para conversa diferente");
        }
    }

    #[test]
    fn room_name_falls_back_to_identifier_without_name() {
        let mut givens = test_data();
        givens.client_name = Some("   ".to_owned());
        assert_eq!(givens.room_name(), "+55 11 99999-9999");
        givens.client_name = Some("Maria".to_owned());
        assert_eq!(givens.room_name(), "Maria");
        givens.client_name = None;
        givens.client = String::new();
        assert_eq!(givens.room_name(), DEFAULT_CLIENT_NAME);
    }

    #[test]
    fn metadata_carries_only_integration_known_to_frontend() {
        let mut givens = test_data();
        givens.channel = "whatsapp".to_owned();
        let metadata = RoomMetadata::of_conversation(&givens);
        assert_eq!(metadata.integration_type.as_deref(), Some("whatsapp"));
        assert_eq!(metadata.channel, "whatsapp");

        givens.channel = "console".to_owned();
        let metadata = RoomMetadata::of_conversation(&givens);
        assert_eq!(metadata.integration_type, None);
        assert_eq!(metadata.channel, "console");

        givens.channel = "WhatsApp".to_owned();
        assert_eq!(
            RoomMetadata::of_conversation(&givens)
                .integration_type
                .as_deref(),
            Some("whatsapp")
        );
    }

    #[test]
    fn metadata_marks_conversation_closed() {
        let mut givens = test_data();
        assert_eq!(RoomMetadata::of_conversation(&givens).status, "active");
        givens.closed = true;
        assert_eq!(RoomMetadata::of_conversation(&givens).status, "closed");
    }

    fn test_data() -> ConversationToRoom {
        ConversationToRoom {
            org: OrgId::generate(),
            conversation: conversation(),
            client: "+55 11 99999-9999".to_owned(),
            client_name: Some("Maria".to_owned()),
            agent: AgentId::generate(),
            agent_name: Some("Agente".to_owned()),
            channel: "whatsapp".to_owned(),
            department: None,
            closed: false,
            backfill_cutoff: None,
        }
    }
}
