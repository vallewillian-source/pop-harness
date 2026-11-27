use std::fmt;
use std::future::Future;
use std::time::Duration;

use futures::stream::{self, StreamExt};
use poph_types::{ConversationId, DepartmentId, MatrixConfig, OrgId};
use serde_json::{Value, json};

use crate::client::{CreateRoom, MatrixClient};
use crate::error::{CallError, Classification};
use crate::rooms::PortError;
use crate::users::{Ghost, GhostIds, GhostsError};

pub const SPACE_MARK: &str = "dep";

pub const SPACE_KIND: &str = "m.space";
pub const CHILD_KIND: &str = "m.space.child";
pub const PARENT_KIND: &str = "m.space.parent";
pub const ENTRY_RULES_KIND: &str = "m.room.join_rules";
pub const POWERS_KIND: &str = "m.room.power_levels";
pub const DEPARTMENT_KIND: &str = "com.sofacoding.departamento";

pub const DEFAULT_STATE_POWER: i64 = 50;

const ALIAS_ATTEMPTS: u8 = 3;

const ERRCODE_ALIAS_IN_USE: &str = "M_ROOM_IN_USE";
const ERRCODE_NOT_FOUND: &str = "M_NOT_FOUND";
const ERRCODE_FORBIDDEN: &str = "M_FORBIDDEN";

pub fn resolve_department(
    target: Option<DepartmentId>,
    agent_default: Option<DepartmentId>,
) -> Option<DepartmentId> {
    target.or(agent_default)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepartmentSpace {
    pub id: DepartmentId,
    pub name: String,
}

impl DepartmentSpace {
    pub fn space_name(&self) -> String {
        let clean = self.name.split_whitespace().collect::<Vec<_>>().join(" ");
        if clean.is_empty() {
            self.id.to_string()
        } else {
            clean
        }
    }
}

pub fn localpart_do_alias(department: DepartmentId) -> String {
    let body: String = department
        .to_string()
        .chars()
        .filter(char::is_ascii_hexdigit)
        .map(|character| character.to_ascii_lowercase())
        .collect();
    format!("{SPACE_MARK}_{body}")
}

pub fn department_alias(server_name: &str, department: DepartmentId) -> String {
    format!("#{}:{}", localpart_do_alias(department), server_name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceOrigin {
    Automatic,
    Linked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSpace {
    pub room_id: String,
    pub alias: String,
    pub origin: SpaceOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomForSpace {
    pub org: OrgId,
    pub conversation: ConversationId,
    pub room: String,
    pub agent: Ghost,
    pub department: Option<DepartmentSpace>,
    pub existing: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Position {
    WithoutDepartment,
    AlreadyPositioned {
        space: String,
    },
    Positioned {
        space: String,
        moved: bool,
        guests: u32,
        failed_invitations: u32,
    },
    Removed {
        space: String,
    },
}

pub trait SpacesPort {
    fn space(
        &self,
        org: OrgId,
        department: DepartmentId,
    ) -> impl Future<Output = Result<Option<StoredSpace>, PortError>> + Send;

    fn ensure_space(
        &self,
        org: OrgId,
        department: DepartmentId,
        room_id: &str,
        alias: &str,
        origin: SpaceOrigin,
    ) -> impl Future<Output = Result<StoredSpace, PortError>> + Send;

    fn room_space(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> impl Future<Output = Result<Option<String>, PortError>> + Send;

    fn set_room_space(
        &self,
        org: OrgId,
        conversation: ConversationId,
        space: Option<&str>,
    ) -> impl Future<Output = Result<(), PortError>> + Send;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpaceError {
    #[error(transparent)]
    Call(#[from] CallError),
    #[error(transparent)]
    Armazenamento(#[from] PortError),
    #[error("falta permissão para pôr a sala no espaço: {what_is_missing}")]
    WithoutPermission { what_is_missing: String },
    #[error("espaço informado inválido: {reason}")]
    InvalidSpace { reason: &'static str },
    #[error("falha interna do módulo Matrix: {reason}")]
    Internal { reason: String },
}

impl SpaceError {
    pub fn classification(&self) -> Classification {
        match self {
            Self::Call(error) => error.classification(),
            Self::Armazenamento(_) => Classification::Transient,
            Self::WithoutPermission { .. } => Classification::Transient,
            Self::InvalidSpace { .. } | Self::Internal { .. } => Classification::Definitive,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InviteLimits {
    pub batch: usize,
    pub concurrency: usize,
    pub pause: Duration,
    pub cap: usize,
}

impl Default for InviteLimits {
    fn default() -> Self {
        Self {
            batch: 20,
            concurrency: 4,
            pause: Duration::from_millis(100),
            cap: 200,
        }
    }
}

impl InviteLimits {
    pub fn with(batch: usize, concurrency: usize, pause: Duration, cap: usize) -> Self {
        Self {
            batch: batch.max(1),
            concurrency: concurrency.max(1),
            pause,
            cap: cap.max(1),
        }
    }
}

pub struct Spaces<P: SpacesPort> {
    client: MatrixClient,
    ids: GhostIds,
    bot: String,
    port: P,
    limits: InviteLimits,
}

impl<P: SpacesPort> Spaces<P> {
    pub fn with(client: MatrixClient, ids: GhostIds, port: P) -> Self {
        Self {
            bot: ids.bot().user_id().to_owned(),
            client,
            ids,
            port,
            limits: InviteLimits::default(),
        }
    }

    pub fn of_config(
        client: MatrixClient,
        port: P,
        config: &MatrixConfig,
    ) -> Result<Self, GhostsError> {
        let ids = GhostIds::of_config(config)?;
        Ok(Self::with(client, ids, port))
    }

    pub fn with_limits(mut self, limits: InviteLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn bot(&self) -> &str {
        &self.bot
    }

    pub fn alias(&self, department: DepartmentId) -> String {
        department_alias(self.client.server_name(), department)
    }

    pub async fn space(
        &self,
        org: OrgId,
        department: DepartmentId,
    ) -> Result<Option<StoredSpace>, SpaceError> {
        Ok(self.port.space(org, department).await?)
    }

    pub async fn ensure_space(
        &self,
        org: OrgId,
        department: &DepartmentSpace,
        existing: Option<&str>,
    ) -> Result<StoredSpace, SpaceError> {
        if let Some(stored) = self.port.space(org, department.id).await? {
            return Ok(stored);
        }
        let alias = self.alias(department.id);
        let (room_id, origin) = match existing {
            Some(reference) => {
                let room_id = self.resolve_space(reference).await?;
                self.check_permission(&room_id).await?;
                (room_id, SpaceOrigin::Linked)
            }
            None => (self.create_space(department).await?, SpaceOrigin::Automatic),
        };
        let stored = self
            .port
            .ensure_space(org, department.id, &room_id, &alias, origin)
            .await?;
        Ok(stored)
    }

    pub async fn place(&self, givens: &RoomForSpace) -> Result<Position, SpaceError> {
        if givens.room.trim().is_empty() {
            return Err(SpaceError::InvalidSpace {
                reason: "a sala da conversa está vazia",
            });
        }
        let current = self
            .port
            .room_space(givens.org, givens.conversation)
            .await?;

        let Some(department) = &givens.department else {
            let Some(old) = current else {
                return Ok(Position::WithoutDepartment);
            };
            self.remove_from_space(givens, &old).await?;
            self.port
                .set_room_space(givens.org, givens.conversation, None)
                .await?;
            return Ok(Position::Removed { space: old });
        };

        let space = self
            .ensure_space(givens.org, department, givens.existing.as_deref())
            .await?;
        if current.as_deref() == Some(space.room_id.as_str()) {
            return Ok(Position::AlreadyPositioned {
                space: space.room_id,
            });
        }

        self.write_child(&space.room_id, &givens.room).await?;
        self.write_parent(givens, &space.room_id).await?;
        self.write_entry_rules(givens, &space.room_id).await?;
        let (guests, failed_invitations) = self.invite_members(givens, &space.room_id).await?;

        if failed_invitations > 0 {
            return Ok(Position::Positioned {
                space: space.room_id,
                moved: false,
                guests,
                failed_invitations,
            });
        }

        let olds: Vec<String> = match current {
            Some(old) => vec![old],
            None => self.room_parent_spaces(givens).await?,
        };
        let olds: Vec<String> = olds
            .into_iter()
            .filter(|old| *old != space.room_id)
            .collect();
        let moved = !olds.is_empty();
        for old in &olds {
            self.remove_from_space(givens, old).await?;
        }

        self.port
            .set_room_space(givens.org, givens.conversation, Some(&space.room_id))
            .await?;
        Ok(Position::Positioned {
            space: space.room_id,
            moved,
            guests,
            failed_invitations,
        })
    }

    async fn create_space(&self, department: &DepartmentSpace) -> Result<String, SpaceError> {
        let alias = self.alias(department.id);
        let body = CreateRoom {
            room_alias_name: Some(localpart_do_alias(department.id)),
            name: Some(department.space_name()),
            topic: Some(format!("Departamento · {}", department.space_name())),
            creation_content: Some(json!({ "type": SPACE_KIND })),
            preset: Some("private_chat".to_owned()),
            ..CreateRoom::default()
        };
        for _ in 0..ALIAS_ATTEMPTS {
            match self.client.create_room(&self.bot, &body).await {
                Ok(space) => {
                    self.mark_department(&space, department).await?;
                    return Ok(space);
                }
                Err(error) if errcode(&error) == Some(ERRCODE_ALIAS_IN_USE) => {
                    match self.client.resolver_alias(&alias).await {
                        Ok(space) => return Ok(space),
                        Err(error) if errcode(&error) == Some(ERRCODE_NOT_FOUND) => continue,
                        Err(error) => return Err(error.into()),
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(SpaceError::Internal {
            reason: format!(
                "o alias `{alias}` apareceu em uso e sumiu do diretório em {ALIAS_ATTEMPTS} tentativas"
            ),
        })
    }

    async fn mark_department(
        &self,
        space: &str,
        department: &DepartmentSpace,
    ) -> Result<(), SpaceError> {
        let body = json!({
            "department_id": department.id.to_string(),
            "name": department.space_name(),
        });
        self.client
            .send_state(&self.bot, space, DEPARTMENT_KIND, None, &body)
            .await?;
        Ok(())
    }

    async fn resolve_space(&self, reference: &str) -> Result<String, SpaceError> {
        let reference = reference.trim();
        if reference.starts_with('#') {
            return self
                .client
                .resolver_alias(reference)
                .await
                .map_err(SpaceError::from);
        }
        if reference.starts_with('!') {
            return Ok(reference.to_owned());
        }
        Err(SpaceError::InvalidSpace {
            reason: "informe o id do espaço (`!…`) ou o alias dele (`#…`)",
        })
    }

    async fn check_permission(&self, space: &str) -> Result<(), SpaceError> {
        let members = self.client.members(&self.bot, space).await?;
        if !members.contains_key(&self.bot) {
            return Err(self.without_permission(space, "ele não é membro do espaço"));
        }
        let powers = self
            .client
            .read_state(&self.bot, space, POWERS_KIND, None)
            .await?;
        let necessario = event_level(&powers, CHILD_KIND);
        let power = user_power(&powers, &self.bot);
        if power < necessario {
            return Err(self.without_permission(
                space,
                &format!(
                    "o poder dele no espaço é {power} e escrever `{CHILD_KIND}` exige {necessario}"
                ),
            ));
        }
        Ok(())
    }

    fn without_permission(&self, space: &str, reason: &str) -> SpaceError {
        SpaceError::WithoutPermission {
            what_is_missing: format!(
                "no Element, convide {} para o espaço {space} e dê a ele poder {DEFAULT_STATE_POWER} \
                 para eventos de estado ({reason})",
                self.bot
            ),
        }
    }

    async fn write_child(&self, space: &str, room: &str) -> Result<(), SpaceError> {
        match self.client.space_child(&self.bot, space, room, None).await {
            Ok(_) => Ok(()),
            Err(error) if errcode(&error) == Some(ERRCODE_FORBIDDEN) => {
                Err(self
                    .without_permission(space, "o Synapse recusou a escrita do `m.space.child`"))
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn remove_child(&self, space: &str, room: &str) -> Result<(), SpaceError> {
        self.client
            .send_state(&self.bot, space, CHILD_KIND, Some(room), &json!({}))
            .await?;
        Ok(())
    }

    async fn write_parent(&self, givens: &RoomForSpace, space: &str) -> Result<(), SpaceError> {
        let body = json!({
            "via": [self.client.server_name()],
            "canonical": true,
        });
        self.client
            .send_state(
                givens.agent.user_id(),
                &givens.room,
                PARENT_KIND,
                Some(space),
                &body,
            )
            .await?;
        Ok(())
    }

    async fn write_entry_rules(
        &self,
        givens: &RoomForSpace,
        space: &str,
    ) -> Result<(), SpaceError> {
        let body = json!({
            "join_rule": "restricted",
            "allow": [{ "type": "m.room_membership", "room_id": space }],
        });
        self.client
            .send_state(
                givens.agent.user_id(),
                &givens.room,
                ENTRY_RULES_KIND,
                None,
                &body,
            )
            .await?;
        Ok(())
    }

    async fn room_parent_spaces(&self, givens: &RoomForSpace) -> Result<Vec<String>, SpaceError> {
        let events = self
            .client
            .room_state(givens.agent.user_id(), &givens.room)
            .await?;
        Ok(events
            .iter()
            .filter(|event| event.get("type").and_then(Value::as_str) == Some(PARENT_KIND))
            .filter(|event| {
                event
                    .get("content")
                    .and_then(Value::as_object)
                    .is_some_and(|content| !content.is_empty())
            })
            .filter_map(|event| event.get("state_key").and_then(Value::as_str))
            .filter(|space| !space.is_empty())
            .map(str::to_owned)
            .collect())
    }

    async fn remove_from_space(
        &self,
        givens: &RoomForSpace,
        space: &str,
    ) -> Result<(), SpaceError> {
        self.remove_child(space, &givens.room).await?;
        self.remove_parent(givens, space).await?;
        Ok(())
    }

    async fn remove_parent(&self, givens: &RoomForSpace, space: &str) -> Result<(), SpaceError> {
        self.client
            .send_state(
                givens.agent.user_id(),
                &givens.room,
                PARENT_KIND,
                Some(space),
                &json!({}),
            )
            .await?;
        Ok(())
    }

    async fn invite_members(
        &self,
        givens: &RoomForSpace,
        space: &str,
    ) -> Result<(u32, u32), SpaceError> {
        let in_space = self.client.members(&self.bot, space).await?;
        let in_room = self
            .client
            .members(givens.agent.user_id(), &givens.room)
            .await?;
        let targets: Vec<String> = in_space
            .keys()
            .filter(|id| !self.ids.eh_bot(id) && !self.ids.is_ghost(id))
            .filter(|id| !in_room.contains_key(*id))
            .take(self.limits.cap)
            .cloned()
            .collect();

        let mut guests = 0u32;
        let mut faulty = 0u32;
        for batch in targets.chunks(self.limits.batch) {
            let user = givens.agent.user_id().to_owned();
            let room = givens.room.clone();
            let delivereds: Vec<Result<(), CallError>> = stream::iter(
                batch
                    .iter()
                    .cloned()
                    .map(|target| self.invite_one(&user, &room, target)),
            )
            .buffer_unordered(self.limits.concurrency)
            .collect()
            .await;
            for result in delivereds {
                match result {
                    Ok(()) => guests += 1,
                    Err(_) => faulty += 1,
                }
            }
            tokio::time::sleep(self.limits.pause).await;
        }
        Ok((guests, faulty))
    }

    async fn invite_one(&self, user: &str, room: &str, target: String) -> Result<(), CallError> {
        self.client.invite(user, room, &target).await
    }
}

impl<P: SpacesPort> fmt::Debug for Spaces<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Espacos")
            .field("cliente", &self.client)
            .field("bot", &self.bot)
            .field("limites", &self.limits)
            .finish()
    }
}

fn errcode(error: &CallError) -> Option<&str> {
    match error {
        CallError::Response { errcode, .. } => errcode.as_deref(),
        _ => None,
    }
}

fn user_power(powers: &Value, user: &str) -> i64 {
    powers
        .get("users")
        .and_then(|users| users.get(user))
        .and_then(Value::as_i64)
        .or_else(|| powers.get("users_default").and_then(Value::as_i64))
        .unwrap_or(0)
}

fn event_level(powers: &Value, kind: &str) -> i64 {
    powers
        .get("events")
        .and_then(|events| events.get(kind))
        .and_then(Value::as_i64)
        .or_else(|| powers.get("state_default").and_then(Value::as_i64))
        .unwrap_or(DEFAULT_STATE_POWER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_department_prefers_target_and_falls_back_to_default() {
        let target = DepartmentId::generate();
        let default = DepartmentId::generate();
        assert_eq!(
            resolve_department(Some(target), Some(default)),
            Some(target)
        );
        assert_eq!(resolve_department(None, Some(default)), Some(default));
        assert_eq!(resolve_department(None, None), None);
    }

    #[test]
    fn space_alias_is_deterministic_and_unique_per_department() {
        let um = DepartmentId::generate();
        let other = DepartmentId::generate();
        assert_eq!(
            department_alias("matrix-dev.local", um),
            department_alias("matrix-dev.local", um)
        );
        assert_ne!(
            department_alias("matrix-dev.local", um),
            department_alias("matrix-dev.local", other)
        );
        let localpart = localpart_do_alias(um);
        assert!(localpart.starts_with("dep_"), "{localpart}");
        assert!(
            localpart
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "{localpart}"
        );
    }

    #[test]
    fn space_name_falls_back_to_id_when_department_has_no_name() {
        let id = DepartmentId::generate();
        let department = DepartmentSpace {
            id,
            name: "  Comercial   Rio ".to_owned(),
        };
        assert_eq!(department.space_name(), "Comercial Rio");
        let without_name = DepartmentSpace {
            id,
            name: "   ".to_owned(),
        };
        assert_eq!(without_name.space_name(), id.to_string());
    }

    #[test]
    fn power_and_event_level_follow_matrix_defaults() {
        let empty = json!({});
        assert_eq!(user_power(&empty, "@ph_as_bot:dev"), 0);
        assert_eq!(event_level(&empty, CHILD_KIND), DEFAULT_STATE_POWER);

        let powers = json!({
            "users": { "@ph_as_bot:dev": 60 },
            "users_default": 5,
            "events": { CHILD_KIND: 70 },
            "state_default": 50,
        });
        assert_eq!(user_power(&powers, "@ph_as_bot:dev"), 60);
        assert_eq!(user_power(&powers, "@outro:dev"), 5);
        assert_eq!(event_level(&powers, CHILD_KIND), 70);
        assert_eq!(event_level(&powers, "m.room.name"), 50);
    }

    #[test]
    fn invite_limits_never_become_zero() {
        let limits = InviteLimits::with(0, 0, Duration::ZERO, 0);
        assert_eq!(limits.batch, 1);
        assert_eq!(limits.concurrency, 1);
        assert_eq!(limits.cap, 1);
        assert_eq!(InviteLimits::default().batch, 20);
    }

    #[test]
    fn missing_permission_is_transient_so_room_joins_after_fix() {
        let error = SpaceError::WithoutPermission {
            what_is_missing: "convide o bot".to_owned(),
        };
        assert_eq!(error.classification(), Classification::Transient);
        let invalid = SpaceError::InvalidSpace {
            reason: "nem id nem alias",
        };
        assert_eq!(invalid.classification(), Classification::Definitive);
    }
}
