use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::DomainError;

macro_rules! define_id {
    ($name:ident, $entity:literal) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(into = "Uuid", try_from = "Uuid")]
        pub struct $name(Uuid);

        impl $name {
            pub fn generate() -> Self {
                Self(Uuid::now_v7())
            }

            pub fn from_uuid(uuid: Uuid) -> Result<Self, DomainError> {
                let version = uuid.get_version_num();
                if version == 7 {
                    Ok(Self(uuid))
                } else {
                    Err(DomainError::InvalidId {
                        entity: $entity,
                        value: uuid.to_string(),
                        reason: format!("versão {version}, esperado UUID v7"),
                    })
                }
            }

            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl FromStr for $name {
            type Err = DomainError;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                let uuid = Uuid::parse_str(text).map_err(|error| DomainError::InvalidId {
                    entity: $entity,
                    value: text.to_owned(),
                    reason: error.to_string(),
                })?;
                Self::from_uuid(uuid)
            }
        }

        impl TryFrom<Uuid> for $name {
            type Error = DomainError;

            fn try_from(uuid: Uuid) -> Result<Self, Self::Error> {
                Self::from_uuid(uuid)
            }
        }

        impl From<$name> for Uuid {
            fn from(id: $name) -> Uuid {
                id.0
            }
        }
    };
}

define_id!(OrgId, "org");
define_id!(AgentId, "agente");
define_id!(AdminTokenId, "token de administração");
define_id!(DepartmentId, "departamento");
define_id!(ProductCategoryId, "categoria de produto");
define_id!(ProductId, "produto");
define_id!(CartId, "carrinho");
define_id!(KnowledgeGroupId, "grupo de conhecimento");
define_id!(KnowledgeItemId, "item de conhecimento");
define_id!(FileId, "arquivo");
define_id!(IntegrationId, "integração");
define_id!(ConversationId, "conversa");
define_id!(MessageId, "mensagem");
define_id!(AttachmentId, "anexo");
define_id!(TurnId, "turno");
define_id!(OutboxId, "item de saída");
define_id!(InboxId, "item de entrada");
define_id!(AuditId, "registro de auditoria");
define_id!(RecordId, "ficha");
define_id!(FlowId, "fluxo");
define_id!(RevisionId, "revisão de fluxo");
define_id!(ExecutionId, "execução de fluxo");
define_id!(RecordEventId, "evento de ficha");
define_id!(EffectId, "efeito de fluxo");

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;

    use serde::de::IntoDeserializer;
    use serde::de::value::{Error as ValueError, StrDeserializer};

    use super::*;

    #[test]
    fn generate_produz_uuid_v7() {
        let id = AgentId::generate();
        assert_eq!(id.as_uuid().get_version_num(), 7);
        assert_eq!(id.as_uuid().get_variant(), uuid::Variant::RFC4122);
    }

    #[test]
    fn later_ids_order_later_by_time() {
        let before = OrgId::generate();
        thread::sleep(Duration::from_millis(3));
        let after = OrgId::generate();

        assert!(before < after, "Ord deve seguir o tempo de criação");
        assert!(
            before.to_string() < after.to_string(),
            "a forma textual também ordena pelo tempo"
        );
        let t_before = before.as_uuid().get_timestamp().expect("v7 tem timestamp");
        let t_after = after.as_uuid().get_timestamp().expect("v7 tem timestamp");
        assert!(t_before.to_unix() <= t_after.to_unix());
    }

    #[test]
    fn from_str_accepts_v7_and_round_trips() {
        let id = ConversationId::generate();
        let text = id.to_string();
        let read_back: ConversationId = text.parse().expect("UUID v7 válido");
        assert_eq!(read_back, id);
    }

    const UUID_V4: &str = "f47ac10b-58cc-4372-a567-0e02b2c3d479";

    #[test]
    fn from_str_rejects_non_v7_uuid_with_context() {
        let v4 = UUID_V4.to_owned();
        let error = v4.parse::<MessageId>().expect_err("v4 deve ser recusado");

        match &error {
            DomainError::InvalidId {
                entity,
                value,
                reason,
            } => {
                assert_eq!(entity.to_string(), "mensagem");
                assert_eq!(value, &v4);
                assert!(reason.contains("esperado UUID v7"), "motivo: {reason}");
            }
            other => panic!("erro inesperado: {other:?}"),
        }
    }

    #[test]
    fn from_str_rejects_text_that_is_not_uuid() {
        assert!("não-é-uuid".parse::<OrgId>().is_err());
    }

    #[test]
    fn deserialize_rejects_non_v7_uuid() {
        let inbound: StrDeserializer<'_, ValueError> = UUID_V4.into_deserializer();
        assert!(AgentId::deserialize(inbound).is_err());

        let v7 = AgentId::generate().to_string();
        let inbound: StrDeserializer<'_, ValueError> = v7.as_str().into_deserializer();
        assert!(AgentId::deserialize(inbound).is_ok());
    }

    #[test]
    fn domain_error_carries_entity_and_id() {
        let error = DomainError::NotFound {
            entity: "agente",
            id: "0190-abc".to_owned(),
        };
        let text = error.to_string();
        assert!(text.contains("agente"), "mensagem: {text}");
        assert!(text.contains("0190-abc"), "mensagem: {text}");

        let conflict = DomainError::VersionConflict {
            entity: "produto",
            id: "x".to_owned(),
            expected: 2,
            current: 3,
        };
        let text = conflict.to_string();
        assert!(
            text.contains("esperada 2") && text.contains("atual 3"),
            "mensagem: {text}"
        );
    }
}
