use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::DomainError;

pub const REGISTER_LEAD: &str = "register_lead";

pub const TOOLS_V1: &[&str] = &[
    "consultar_base",
    "buscar_produto",
    "enviar_arquivo",
    "transferir",
    "cart_add_item",
    "cart_remove_item",
    "cart_summary",
    "cart_clear",
    "fluxo_executar_acao",
    "fluxo_abandonar",
    "fluxo_iniciar",
    "atendimento_encerrar",
    REGISTER_LEAD,
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Identity {
    pub name: String,
    pub role: String,
    pub company: String,
    pub company_description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principle {
    pub rule: String,
    #[serde(default = "default_fallback")]
    pub fallback_action: String,
}

fn default_fallback() -> String {
    "say_dont_know".to_owned()
}

impl Default for Principle {
    fn default() -> Self {
        Self {
            rule: String::new(),
            fallback_action: default_fallback(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HandoffScope {
    pub department_id: String,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scope {
    pub does: String,
    pub does_not: String,
    pub handoff: HandoffScope,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowStep {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub tools_used: Vec<String>,
    #[serde(default)]
    pub auto_from_template: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tone {
    pub language: String,
    pub style: String,
    pub emoji_policy: String,
    pub format: String,
    pub extra_restrictions: String,
}

impl Default for Tone {
    fn default() -> Self {
        Self {
            language: "pt-BR".to_owned(),
            style: "professional".to_owned(),
            emoji_policy: "moderate".to_owned(),
            format: "simple_lists".to_owned(),
            extra_restrictions: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolConfig {
    pub name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub when_to_use: String,
}

fn default_enabled() -> bool {
    true
}

impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            when_to_use: String::new(),
        }
    }
}

impl ToolConfig {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            enabled: true,
            when_to_use: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StructuredInstructions {
    pub identity: Identity,
    pub principles: Vec<Principle>,
    pub scope: Scope,
    pub flow_steps: Vec<FlowStep>,
    pub tone: Tone,
    pub tools: Vec<ToolConfig>,
    pub extra_instructions: String,
}

impl StructuredInstructions {
    pub fn from_tolerant_json(text: &str) -> Result<Self, DomainError> {
        let value: Value = serde_json::from_str(text).map_err(|error| DomainError::Validation {
            field: "structured_json",
            reason: format!("JSON inválido: {error}"),
        })?;
        let object = match value {
            Value::String(internal) => {
                serde_json::from_str::<Value>(&internal).map_err(|error| {
                    DomainError::Validation {
                        field: "structured_json",
                        reason: format!("JSON duplamente codificado e inválido: {error}"),
                    }
                })?
            }
            other => other,
        };
        if !object.is_object() {
            return Err(DomainError::Validation {
                field: "structured_json",
                reason: "o valor precisa ser um objeto JSON".to_owned(),
            });
        }
        serde_json::from_value(object).map_err(|error| DomainError::Validation {
            field: "structured_json",
            reason: error.to_string(),
        })
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_owned())
    }

    pub fn validar(&self) -> Result<(), DomainError> {
        self.validate_with_tools(|_| false)
    }

    pub(crate) fn validate_with_tools(
        &self,
        is_declared: impl Fn(&str) -> bool,
    ) -> Result<(), DomainError> {
        for tool in &self.tools {
            if !TOOLS_V1.contains(&tool.name.as_str()) && !is_declared(&tool.name) {
                return Err(DomainError::Validation {
                    field: "tools[].name",
                    reason: format!(
                        "`{}` não é ferramenta da V1 (use uma de: {})",
                        tool.name,
                        TOOLS_V1.join(", ")
                    ),
                });
            }
        }
        Ok(())
    }

    pub fn enabled_tools(&self) -> Vec<&ToolConfig> {
        self.tools.iter().filter(|tool| tool.enabled).collect()
    }
}
