use poph_types::JevGates;

use crate::types::IdGate;
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GateOverrides {
    pub enabled: Option<bool>,
    pub g1: Option<bool>,
    pub g2: Option<bool>,
    pub g3: Option<bool>,
    pub g4: Option<bool>,
    pub g5: Option<bool>,
    pub g6: Option<bool>,
    pub g7: Option<bool>,
    pub g8: Option<bool>,
    pub g9: Option<bool>,
}

impl GateOverrides {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn only(gate: IdGate) -> Self {
        let mut overrides = Self {
            enabled: Some(true),
            g1: Some(false),
            g2: Some(false),
            g3: Some(false),
            g4: Some(false),
            g5: Some(false),
            g6: Some(false),
            g7: Some(false),
            g8: Some(false),
            g9: Some(false),
        };
        match gate {
            IdGate::G1 => overrides.g1 = Some(true),
            IdGate::G2 => overrides.g2 = Some(true),
            IdGate::G3 => overrides.g3 = Some(true),
            IdGate::G4 => overrides.g4 = Some(true),
            IdGate::G5 => overrides.g5 = Some(true),
            IdGate::G6 => overrides.g6 = Some(true),
            IdGate::G7 => overrides.g7 = Some(true),
            IdGate::G8 => overrides.g8 = Some(true),
            IdGate::G9 => overrides.g9 = Some(true),
        }
        overrides
    }

    pub fn from_json(text: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(text).ok()?;
        let object = value.as_object()?;
        let mut overrides = Self {
            enabled: object.get("ativo").and_then(Value::as_bool),
            ..Self::default()
        };
        let nested = object.get("gates").and_then(Value::as_object);
        for (field, value) in object {
            assign(&mut overrides, field, value.as_bool());
        }
        if let Some(gates) = nested {
            for (field, value) in gates {
                assign(&mut overrides, field, value.as_bool());
            }
        }
        if overrides.is_empty() {
            None
        } else {
            Some(overrides)
        }
    }

    pub fn with_base(&self, base: JevGates) -> JevGates {
        if self.enabled == Some(false) {
            return JevGates::default();
        }
        JevGates {
            g1: self.g1.unwrap_or(base.g1),
            g2: self.g2.unwrap_or(base.g2),
            g3: self.g3.unwrap_or(base.g3),
            g4: self.g4.unwrap_or(base.g4),
            g5: self.g5.unwrap_or(base.g5),
            g6: self.g6.unwrap_or(base.g6),
            g7: self.g7.unwrap_or(base.g7),
            g8: self.g8.unwrap_or(base.g8),
            g9: self.g9.unwrap_or(base.g9),
        }
    }
}

fn assign(overrides: &mut GateOverrides, field: &str, value: Option<bool>) {
    let Some(value) = value else { return };
    match field {
        "g1" => overrides.g1 = Some(value),
        "g2" => overrides.g2 = Some(value),
        "g3" => overrides.g3 = Some(value),
        "g4" => overrides.g4 = Some(value),
        "g5" => overrides.g5 = Some(value),
        "g6" => overrides.g6 = Some(value),
        "g7" => overrides.g7 = Some(value),
        "g8" => overrides.g8 = Some(value),
        "g9" => overrides.g9 = Some(value),
        _ => {}
    }
}

pub fn from_agent(jev_json: &str) -> Option<GateOverrides> {
    let text = jev_json.trim();
    if text.is_empty() || text == "{}" {
        return None;
    }
    GateOverrides::from_json(text)
}

pub fn handoff_situations_from_agent(jev_json: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(jev_json) else {
        return Vec::new();
    };
    value
        .get("situacoes_de_handoff")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub const MAX_FAREWELL_CHARS: usize = 500;

pub fn farewell_from_agent(jev_json: &str) -> Option<String> {
    let value: Value = serde_json::from_str(jev_json).ok()?;
    let text = value.get("despedida")?.as_str()?;
    Some(text.trim().chars().take(MAX_FAREWELL_CHARS).collect())
}

pub const MAX_SAFE_RESPONSE_CHARS: usize = 1000;

pub fn safe_response_from_agent(jev_json: &str) -> Option<String> {
    let value: Value = serde_json::from_str(jev_json).ok()?;
    let text = value.get("resposta_segura")?.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.chars().take(MAX_SAFE_RESPONSE_CHARS).collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum G7Action {
    #[default]
    Record,
    DefaultReply,
    Transfer,
}

impl G7Action {
    pub fn as_text(self) -> &'static str {
        match self {
            G7Action::Record => "registrar",
            G7Action::DefaultReply => "responder_padrao",
            G7Action::Transfer => "transferir",
        }
    }
}

pub fn g7_action_from_agent(jev_json: &str) -> G7Action {
    let Ok(value) = serde_json::from_str::<Value>(jev_json) else {
        return G7Action::Record;
    };
    match value.get("g7_acao").and_then(Value::as_str) {
        Some("responder_padrao") => G7Action::DefaultReply,
        Some("transferir") => G7Action::Transfer,
        _ => G7Action::Record,
    }
}

pub const MAX_G7_RESPONSE_CHARS: usize = 1000;

pub fn g7_response_from_agent(jev_json: &str) -> Option<String> {
    let value: Value = serde_json::from_str(jev_json).ok()?;
    let text = value.get("g7_resposta")?.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.chars().take(MAX_G7_RESPONSE_CHARS).collect())
}
