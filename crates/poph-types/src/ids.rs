//! Identificadores tipados. Cada entidade tem o seu tipo (um `AgentId` não cabe onde se
//! espera um `OrgId`). Todos são UUID v7: ordenáveis pelo tempo de criação.
//!
//! Regra de borda: um id vindo de texto ou de JSON só é aceito se for UUID v7.
//! Isso vale para `FromStr` e para `Deserialize`, então payload inválido falha na entrada.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::DomainError;

/// Define um id tipado: newtype sobre `Uuid`, com geração v7, validação de versão e serde.
macro_rules! define_id {
    ($nome:ident, $entidade:literal) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(into = "Uuid", try_from = "Uuid")]
        pub struct $nome(Uuid);

        impl $nome {
            /// Gera um id novo (UUID v7, com o tempo de criação no início).
            pub fn generate() -> Self {
                Self(Uuid::now_v7())
            }

            /// Aceita um UUID existente somente se for versão 7.
            pub fn from_uuid(uuid: Uuid) -> Result<Self, DomainError> {
                let versao = uuid.get_version_num();
                if versao == 7 {
                    Ok(Self(uuid))
                } else {
                    Err(DomainError::IdInvalido {
                        entidade: $entidade,
                        valor: uuid.to_string(),
                        motivo: format!("versão {versao}, esperado UUID v7"),
                    })
                }
            }

            /// Valor bruto do UUID, para gravar no banco ou em um contrato externo.
            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $nome {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl FromStr for $nome {
            type Err = DomainError;

            fn from_str(texto: &str) -> Result<Self, Self::Err> {
                let uuid = Uuid::parse_str(texto).map_err(|erro| DomainError::IdInvalido {
                    entidade: $entidade,
                    valor: texto.to_owned(),
                    motivo: erro.to_string(),
                })?;
                Self::from_uuid(uuid)
            }
        }

        impl TryFrom<Uuid> for $nome {
            type Error = DomainError;

            fn try_from(uuid: Uuid) -> Result<Self, Self::Error> {
                Self::from_uuid(uuid)
            }
        }

        impl From<$nome> for Uuid {
            fn from(id: $nome) -> Uuid {
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
// Fluxos e Fichas (PRD §7.1 do módulo, `HARNESS_FLUXOS_PRD.md`).
define_id!(FichaId, "ficha");
define_id!(FluxoId, "fluxo");
define_id!(RevisaoId, "revisão de fluxo");
define_id!(ExecucaoId, "execução de fluxo");
define_id!(EventoFichaId, "evento de ficha");
define_id!(EfeitoId, "efeito de fluxo");

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
    fn ids_criados_depois_ordenam_depois_pelo_tempo() {
        let antes = OrgId::generate();
        thread::sleep(Duration::from_millis(3));
        let depois = OrgId::generate();

        assert!(antes < depois, "Ord deve seguir o tempo de criação");
        assert!(
            antes.to_string() < depois.to_string(),
            "a forma textual também ordena pelo tempo"
        );
        let t_antes = antes.as_uuid().get_timestamp().expect("v7 tem timestamp");
        let t_depois = depois.as_uuid().get_timestamp().expect("v7 tem timestamp");
        assert!(t_antes.to_unix() <= t_depois.to_unix());
    }

    #[test]
    fn from_str_aceita_v7_e_faz_ida_e_volta() {
        let id = ConversationId::generate();
        let texto = id.to_string();
        let lido: ConversationId = texto.parse().expect("UUID v7 válido");
        assert_eq!(lido, id);
    }

    /// UUID versão 4 fixo (a feature `v4` não está ligada no workspace).
    const UUID_V4: &str = "f47ac10b-58cc-4372-a567-0e02b2c3d479";

    #[test]
    fn from_str_recusa_uuid_que_nao_e_v7_com_contexto() {
        let v4 = UUID_V4.to_owned();
        let erro = v4.parse::<MessageId>().expect_err("v4 deve ser recusado");

        match &erro {
            DomainError::IdInvalido {
                entidade,
                valor,
                motivo,
            } => {
                assert_eq!(entidade.to_string(), "mensagem");
                assert_eq!(valor, &v4);
                assert!(motivo.contains("esperado UUID v7"), "motivo: {motivo}");
            }
            outro => panic!("erro inesperado: {outro:?}"),
        }
    }

    #[test]
    fn from_str_recusa_texto_que_nao_e_uuid() {
        assert!("não-é-uuid".parse::<OrgId>().is_err());
    }

    #[test]
    fn deserialize_recusa_uuid_que_nao_e_v7() {
        // Serde passa pela mesma validação: payload com v4 não vira id.
        let entrada: StrDeserializer<'_, ValueError> = UUID_V4.into_deserializer();
        assert!(AgentId::deserialize(entrada).is_err());

        let v7 = AgentId::generate().to_string();
        let entrada: StrDeserializer<'_, ValueError> = v7.as_str().into_deserializer();
        assert!(AgentId::deserialize(entrada).is_ok());
    }

    #[test]
    fn erro_de_dominio_carrega_entidade_e_id() {
        let erro = DomainError::NaoEncontrado {
            entidade: "agente",
            id: "0190-abc".to_owned(),
        };
        let texto = erro.to_string();
        assert!(texto.contains("agente"), "mensagem: {texto}");
        assert!(texto.contains("0190-abc"), "mensagem: {texto}");

        let conflito = DomainError::ConflitoDeVersao {
            entidade: "produto",
            id: "x".to_owned(),
            esperada: 2,
            atual: 3,
        };
        let texto = conflito.to_string();
        assert!(
            texto.contains("esperada 2") && texto.contains("atual 3"),
            "mensagem: {texto}"
        );
    }
}
