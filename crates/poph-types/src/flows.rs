use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{DomainError, DomainResult};
use crate::ids::FlowId;

macro_rules! define_text_enum {
    (
        $(#[$meta:meta])*
        $name:ident, field: $field:literal,
        $(
            $(#[$vmeta:meta])*
            $variante:ident => $text:literal
        ),+ $(,)?
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $(
                $(#[$vmeta])*
                $variante,
            )+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variante),+];

            pub fn as_text(self) -> &'static str {
                match self {
                    $(Self::$variante => $text,)+
                }
            }

            pub fn from_text(value: &str) -> DomainResult<Self> {
                match value {
                    $($text => Ok(Self::$variante),)+
                    other => Err(DomainError::Validation {
                        field: $field,
                        reason: format!(
                            "valor desconhecido `{other}`; aceitos: {}",
                            Self::ALL
                                .iter()
                                .map(|variante| variante.as_text())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    }),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_text())
            }
        }

        impl FromStr for $name {
            type Err = DomainError;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                Self::from_text(text)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializador: S) -> Result<S::Ok, S::Error> {
                serializador.serialize_str(self.as_text())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializador: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializador)?;
                Self::from_text(&text).map_err(D::Error::custom)
            }
        }
    };
}

define_text_enum!(
    RecordStatus, field: "status_ficha",
    Open => "open",
    Closed => "closed",
);

define_text_enum!(
    ClosedBy, field: "fechada_por",
    Customer => "cliente",
    Agent => "agente",
    Attendant => "atendente",
    Admin => "admin",
    Flow => "fluxo",
    System => "sistema",
    Migration => "migracao",
);

define_text_enum!(
    ExecutionStatus, field: "status_execucao",
    Active => "ativa",
    Completed => "concluida",
    Abandoned => "abandonada",
    Failed => "falha",
);

define_text_enum!(
    StepStatus, field: "status_etapa",
    Pending => "pendente",
    Partial => "parcial",
    Completed => "concluida",
);

define_text_enum!(
    ActionActor, field: "ator_acao",
    Agent => "agente",
    Attendant => "atendente",
);

define_text_enum!(
    EventActor, field: "ator_evento",
    Agent => "agente",
    Attendant => "atendente",
    System => "sistema",
    Jev => "jev",
    Admin => "admin",
);

define_text_enum!(
    RecordEventKind, field: "tipo_evento_ficha",
    RecordOpened => "ficha_aberta",
    FlowStarted => "fluxo_iniciado",
    StepEntered => "etapa_entrou",
    FieldsFilled => "campos_preenchidos",
    ActionExecuted => "acao_executada",
    DecisionResolved => "decisao_resolvida",
    StepCompleted => "etapa_concluida",
    EffectCreated => "efeito_criado",
    EffectSent => "efeito_enviado",
    EffectFailed => "efeito_falhou",
    ExecutionCompleted => "execucao_concluida",
    ExecutionAbandoned => "execucao_abandonada",
    ExecutionFailed => "execucao_falhou",
    ServiceTransferred => "atendimento_transferido",
    RecordClosed => "ficha_fechada",
);

define_text_enum!(
    EffectState, field: "estado_efeito",
    Pending => "pendente",
    Sending => "enviando",
    Sent => "enviado",
    Failed => "falhou",
);

define_text_enum!(
    FieldKind, field: "tipo_campo",
    Text => "texto",
    Number => "numero",
    Data => "data",
    Boolean => "booleano",
    Option => "opcao",
    Attachment => "anexo",
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowTrigger {
    pub id: FlowId,
    pub slug: String,
    #[serde(rename = "quando")]
    pub when: String,
    #[serde(default)]
    #[serde(rename = "nao_quando")]
    pub not_when: Option<String>,
    #[serde(default)]
    #[serde(rename = "exemplos")]
    pub examples: Vec<String>,
}

impl FlowTrigger {
    pub fn new(id: FlowId, slug: impl Into<String>, when: impl Into<String>) -> Self {
        Self {
            id,
            slug: slug.into(),
            when: when.into(),
            not_when: None,
            examples: Vec::new(),
        }
    }

    pub fn with_not_when(mut self, not_when: impl Into<String>) -> Self {
        self.not_when = Some(not_when.into());
        self
    }

    pub fn with_examples<I, S>(mut self, examples: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.examples = examples.into_iter().map(Into::into).collect();
        self
    }

    pub fn validar(&self) -> DomainResult<()> {
        if self.slug.trim().is_empty() {
            return Err(DomainError::Validation {
                field: "gatilho.slug",
                reason: "o fluxo precisa de um slug para o Jev escolher".to_owned(),
            });
        }
        if self.when.trim().is_empty() {
            return Err(DomainError::Validation {
                field: "gatilho.quando",
                reason: "sem o critério positivo o Jev não tem como escolher este fluxo".to_owned(),
            });
        }
        if self
            .examples
            .iter()
            .any(|example| example.trim().is_empty())
        {
            return Err(DomainError::Validation {
                field: "gatilho.exemplos",
                reason: "exemplo em branco não ajuda o Jev: remova ou escreva a fala".to_owned(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde::de::IntoDeserializer;
    use serde::de::value::{Error as ValueError, StrDeserializer};
    use uuid::Uuid;

    use super::*;
    use crate::ids::{EffectId, ExecutionId, RecordEventId, RecordId, RevisionId};

    const UUID_V4: &str = "f47ac10b-58cc-4372-a567-0e02b2c3d479";

    macro_rules! checks_round_trip {
        ($name:ident) => {{
            assert!(!$name::ALL.is_empty(), "lista fechada vazia");
            for variante in $name::ALL {
                let text = variante.as_text();
                let read_back = $name::from_text(text)
                    .unwrap_or_else(|error| panic!("`{text}` recusado: {error}"));
                assert_eq!(
                    read_back, *variante,
                    "texto `{text}` voltou como outra variante"
                );
                assert_eq!(variante.to_string(), text, "Display diverge de como_texto");
                assert_eq!(
                    text.parse::<$name>()
                        .expect("FromStr aceita o texto canônico"),
                    *variante
                );
            }
            let mut texts: Vec<&str> = $name::ALL.iter().map(|v| v.as_text()).collect();
            texts.sort_unstable();
            let total = texts.len();
            texts.dedup();
            assert_eq!(
                texts.len(),
                total,
                "texto repetido na lista de {}",
                stringify!($name)
            );
        }};
    }

    #[test]
    fn each_enum_round_trips_text() {
        checks_round_trip!(RecordStatus);
        checks_round_trip!(ClosedBy);
        checks_round_trip!(ExecutionStatus);
        checks_round_trip!(StepStatus);
        checks_round_trip!(ActionActor);
        checks_round_trip!(EventActor);
        checks_round_trip!(RecordEventKind);
        checks_round_trip!(EffectState);
        checks_round_trip!(FieldKind);
    }

    macro_rules! checks_rejection {
        ($name:ident, $field:literal) => {{
            let error = $name::from_text("valor_que_nao_existe")
                .expect_err("valor desconhecido deve ser recusado");
            match &error {
                DomainError::Validation { field, reason } => {
                    assert_eq!(field, &$field, "campo do erro");
                    assert!(
                        reason.contains("valor_que_nao_existe"),
                        "motivo sem o valor: {reason}"
                    );
                    assert!(
                        reason.contains($name::ALL[0].as_text()),
                        "motivo sem a lista de aceitos: {reason}"
                    );
                }
                other => panic!("erro inesperado: {other:?}"),
            }
            assert!(
                "valor_que_nao_existe".parse::<$name>().is_err(),
                "FromStr deve recusar como de_texto"
            );
        }};
    }

    #[test]
    fn each_enum_rejects_unknown_with_accepted_list() {
        checks_rejection!(RecordStatus, "status_ficha");
        checks_rejection!(ClosedBy, "fechada_por");
        checks_rejection!(ExecutionStatus, "status_execucao");
        checks_rejection!(StepStatus, "status_etapa");
        checks_rejection!(ActionActor, "ator_acao");
        checks_rejection!(EventActor, "ator_evento");
        checks_rejection!(RecordEventKind, "tipo_evento_ficha");
        checks_rejection!(EffectState, "estado_efeito");
        checks_rejection!(FieldKind, "tipo_campo");
    }

    macro_rules! checks_json {
        ($name:ident, $text:literal, $variante:ident) => {{
            let json = serde_json::to_value($name::$variante).expect("serializa");
            assert_eq!(json, serde_json::json!($text));

            let read_back: $name = serde_json::from_value(json).expect("desserializa");
            assert_eq!(read_back, $name::$variante);

            let wrong: Result<$name, _> = serde_json::from_value(serde_json::json!("outro"));
            assert!(wrong.is_err(), "JSON com valor fora da lista deve falhar");
        }};
    }

    #[test]
    fn json_uses_the_same_column_text() {
        checks_json!(RecordStatus, "open", Open);
        checks_json!(RecordStatus, "closed", Closed);
        checks_json!(ExecutionStatus, "ativa", Active);
        checks_json!(StepStatus, "concluida", Completed);
        checks_json!(ClosedBy, "migracao", Migration);
        checks_json!(ActionActor, "atendente", Attendant);
        checks_json!(EventActor, "jev", Jev);
        checks_json!(RecordEventKind, "decisao_resolvida", DecisionResolved);
        checks_json!(EffectState, "enviando", Sending);
        checks_json!(FieldKind, "booleano", Boolean);
    }

    #[test]
    fn deserialize_rejects_unknown_text() {
        let inbound: StrDeserializer<'_, ValueError> = "aberta".into_deserializer();
        let error = RecordStatus::deserialize(inbound).expect_err("`aberta` é rótulo, não texto");
        assert!(
            error.to_string().contains("valor desconhecido"),
            "erro: {error}"
        );
    }

    #[test]
    fn actors_do_not_accept_customer() {
        for text in ["cliente", "client", "provider", "operator", "system"] {
            assert!(
                ActionActor::from_text(text).is_err(),
                "AtorAcao aceitou `{text}`"
            );
            assert!(
                EventActor::from_text(text).is_err(),
                "AtorEvento aceitou `{text}`"
            );
        }
    }

    #[test]
    fn module_ids_round_trip_and_reject_wrong_version() {
        macro_rules! checks_id {
            ($name:ident) => {{
                let id = $name::generate();
                let text = id.to_string();
                assert_eq!(text.parse::<$name>().expect("ida e volta"), id);
                assert_eq!(text.len(), 36, "forma textual de UUID");

                let error = UUID_V4
                    .parse::<$name>()
                    .expect_err("UUID v4 deve ser recusado");
                assert!(
                    matches!(error, DomainError::InvalidId { .. }),
                    "erro inesperado: {error:?}"
                );
            }};
        }

        checks_id!(RecordId);
        checks_id!(FlowId);
        checks_id!(RevisionId);
        checks_id!(ExecutionId);
        checks_id!(RecordEventId);
        checks_id!(EffectId);

        let uuid = Uuid::now_v7();
        assert!(RecordId::from_uuid(uuid).is_ok());
        assert_eq!(
            FlowId::from_uuid(uuid)
                .expect("mesmo UUID, tipo certo")
                .as_uuid(),
            uuid
        );
    }

    #[test]
    fn trigger_round_trips_json() {
        let gatilho = FlowTrigger::new(
            FlowId::generate(),
            "agendar_visita",
            "cliente quer marcar visita",
        )
        .with_not_when("cliente já tem visita marcada")
        .with_examples(["queria marcar uma visita", "tem horário amanhã?"]);
        gatilho.validar().expect("gatilho bem formado");

        let json = serde_json::to_value(&gatilho).expect("serializa");
        assert_eq!(json["slug"], serde_json::json!("agendar_visita"));
        assert_eq!(
            json["nao_quando"],
            serde_json::json!("cliente já tem visita marcada")
        );
        let read_back: FlowTrigger = serde_json::from_value(json).expect("desserializa");
        assert_eq!(read_back, gatilho);

        let min = serde_json::json!({
            "id": gatilho.id,
            "slug": "agendar_visita",
            "quando": "cliente quer marcar visita"
        });
        let read_back: FlowTrigger = serde_json::from_value(min).expect("desserializa mínimo");
        assert_eq!(read_back.not_when, None);
        assert!(read_back.examples.is_empty());
    }

    #[test]
    fn trigger_rejects_empty_shape() {
        let id = FlowId::generate();

        let without_slug = FlowTrigger::new(id, "  ", "cliente quer marcar visita");
        assert!(matches!(
            without_slug.validar(),
            Err(DomainError::Validation {
                field: "gatilho.slug",
                ..
            })
        ));

        let without_criterion = FlowTrigger::new(id, "agendar_visita", "\t");
        assert!(matches!(
            without_criterion.validar(),
            Err(DomainError::Validation {
                field: "gatilho.quando",
                ..
            })
        ));

        let with_empty_example =
            FlowTrigger::new(id, "agendar_visita", "cliente quer marcar visita")
                .with_examples(["queria marcar uma visita", " "]);
        assert!(matches!(
            with_empty_example.validar(),
            Err(DomainError::Validation {
                field: "gatilho.exemplos",
                ..
            })
        ));
    }
}
