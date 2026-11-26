use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IdGate {
    G1,
    G2,
    G3,
    G4,
    G5,
    G6,
    G7,
    G8,
    G9,
}

impl IdGate {
    pub const ALL: [IdGate; 9] = [
        IdGate::G1,
        IdGate::G2,
        IdGate::G3,
        IdGate::G4,
        IdGate::G5,
        IdGate::G6,
        IdGate::G7,
        IdGate::G8,
        IdGate::G9,
    ];

    pub fn as_text(self) -> &'static str {
        match self {
            IdGate::G1 => "g1",
            IdGate::G2 => "g2",
            IdGate::G3 => "g3",
            IdGate::G4 => "g4",
            IdGate::G5 => "g5",
            IdGate::G6 => "g6",
            IdGate::G7 => "g7",
            IdGate::G8 => "g8",
            IdGate::G9 => "g9",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            IdGate::G1 => "transferir para humano",
            IdGate::G2 => "esperar mais ou processar agora",
            IdGate::G3 => "precisa de base, produto ou nada",
            IdGate::G4 => "departamento da transferência",
            IdGate::G5 => "qual produto o cliente quer",
            IdGate::G6 => "revisão da resposta antes do envio",
            IdGate::G7 => "injeção de prompt ou abuso",
            IdGate::G8 => "conversa encerrada",
            IdGate::G9 => "qual fluxo o cliente quer iniciar",
        }
    }

    pub fn from_text(texto: &str) -> Option<Self> {
        let clean = texto.trim().to_ascii_lowercase();
        let clean = clean.strip_prefix('g').unwrap_or(&clean);
        let number: u8 = clean.parse().ok()?;
        let wanted = format!("g{number}");
        Self::ALL
            .into_iter()
            .find(|gate| gate.as_text() == wanted.as_str())
    }

    pub fn question_id(self) -> &'static str {
        match self {
            IdGate::G1 => "g1_transferir",
            IdGate::G2 => "g2_esperar",
            IdGate::G3 => "g3_precisa",
            IdGate::G4 => "g4_departamento",
            IdGate::G5 => "g5_produto",
            IdGate::G6 => "g6_revisao",
            IdGate::G7 => "g7_injecao",
            IdGate::G8 => "g8_encerrada",
            IdGate::G9 => "g9_iniciar_fluxo",
        }
    }
}

impl Serialize for IdGate {
    fn serialize<S: serde::Serializer>(&self, serializador: S) -> Result<S::Ok, S::Error> {
        serializador.serialize_str(self.as_text())
    }
}

impl std::fmt::Display for IdGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_text())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuestionType {
    Choice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    #[serde(rename = "type")]
    pub tipo: QuestionType,
    pub instructions: String,
    #[serde(rename = "criteria")]
    pub rubric: BTreeMap<String, String>,
}

impl Question {
    pub fn choice(instructions: impl Into<String>, rubric: BTreeMap<String, String>) -> Self {
        Self {
            tipo: QuestionType::Choice,
            instructions: instructions.into(),
            rubric,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct JevResponse {
    pub choice: Option<String>,
    pub probabilities: BTreeMap<String, f64>,
    pub confidence: f64,
    pub score: Option<f64>,
    pub noul: Option<f64>,
}

impl JevResponse {
    pub fn margin(&self) -> f64 {
        let Some(choice) = &self.choice else {
            return 0.0;
        };
        let Some(chosen_probability) = self.probabilities.get(choice) else {
            return self.confidence;
        };
        let runner_up = self
            .probabilities
            .iter()
            .filter(|(option, _)| *option != choice)
            .map(|(_, value)| *value)
            .fold(0.0_f64, f64::max);
        (chosen_probability - runner_up).max(0.0)
    }

    pub fn from_json(item: &Value) -> Option<Self> {
        let object = item.as_object()?;
        let choice = object
            .get("choice")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let score = object.get("score").and_then(Value::as_f64);
        let noul = object.get("noul").and_then(Value::as_f64);
        if choice.is_none() && score.is_none() && noul.is_none() {
            return None;
        }
        let probabilities = object
            .get("probabilities")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(option, weight)| {
                        weight.as_f64().map(|value| (option.clone(), value))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            choice,
            probabilities,
            confidence: object
                .get("confidence")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            score,
            noul,
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageTokens {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub total: Option<u64>,
}

impl UsageTokens {
    pub fn from_json(usage: Option<&Value>) -> Self {
        let Some(map) = usage.and_then(Value::as_object) else {
            return Self::default();
        };
        let read = |names: &[&str]| -> Option<u64> {
            names
                .iter()
                .find_map(|nome| map.get(*nome).and_then(Value::as_u64))
        };
        let input = read(&["input_tokens", "prompt_tokens"]);
        let output = read(&["output_tokens", "completion_tokens"]);
        let total = read(&["total_tokens"]).or_else(|| match (input, output) {
            (Some(input), Some(output)) => Some(input.saturating_add(output)),
            _ => None,
        });
        Self {
            input,
            output,
            total,
        }
    }

    pub fn reported(&self) -> bool {
        self.input.is_some() || self.output.is_some() || self.total.is_some()
    }

    pub fn plus(self, other: Self) -> Self {
        fn add_optional(a: Option<u64>, b: Option<u64>) -> Option<u64> {
            match (a, b) {
                (None, None) => None,
                (a, b) => Some(a.unwrap_or(0).saturating_add(b.unwrap_or(0))),
            }
        }
        Self {
            input: add_optional(self.input, other.input),
            output: add_optional(self.output, other.output),
            total: add_optional(self.total, other.total),
        }
    }

    pub fn as_text(&self) -> String {
        if !self.reported() {
            return "não informado".to_owned();
        }
        fn number(value: Option<u64>) -> String {
            value.map_or_else(|| "não informado".to_owned(), |value| value.to_string())
        }
        format!(
            "entrada {} · saída {} · total {}",
            number(self.input),
            number(self.output),
            number(self.total)
        )
    }

    pub fn to_json(&self) -> Value {
        json!({
            "entrada": self.input,
            "saida": self.output,
            "total": self.total,
            "informado": self.reported(),
        })
    }
}

impl Serialize for UsageTokens {
    fn serialize<S: serde::Serializer>(&self, serializador: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(serializador)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BatchResponse {
    pub answers: BTreeMap<String, JevResponse>,
    pub tokens: u64,
    pub usage: UsageTokens,
}

impl BatchResponse {
    pub fn from_json(body: &Value) -> Result<Self, JevError> {
        let object = body
            .as_object()
            .ok_or_else(|| JevError::InvalidResponse("o corpo não é um objeto JSON".to_owned()))?;
        let answers = match object.get("answers") {
            None => BTreeMap::new(),
            Some(Value::Object(map)) => map
                .iter()
                .filter_map(|(id, item)| {
                    JevResponse::from_json(item).map(|response| (id.clone(), response))
                })
                .collect(),
            Some(_) => {
                return Err(JevError::InvalidResponse(
                    "`answers` não é um objeto JSON".to_owned(),
                ));
            }
        };
        let usage = UsageTokens::from_json(object.get("usage"));
        let tokens = usage.total.unwrap_or_else(|| {
            object
                .get("usage")
                .and_then(Value::as_object)
                .map(|usage| usage.values().filter_map(Value::as_u64).sum::<u64>())
                .unwrap_or(0)
        });
        Ok(Self {
            answers,
            tokens,
            usage,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum JevError {
    #[error("configuração do Jev: {0}")]
    InvalidConfiguration(String),

    #[error("o Jev não respondeu em {ms} ms")]
    TimedOut { ms: u64 },

    #[error("falha de rede ao chamar o Jev: {0}")]
    Network(String),

    #[error("falha de TLS ao chamar o Jev: {0}")]
    Tls(String),

    #[error("o Jev respondeu {code} ({motivo})")]
    Status { code: u16, motivo: String },

    #[error("resposta do Jev fora do contrato: {0}")]
    InvalidResponse(String),

    #[error("chave do Jev ausente: defina TYPESAFE_API_KEY no ambiente")]
    MissingKey,
}

impl JevError {
    pub fn is_transient(&self) -> bool {
        match self {
            JevError::Status { code, .. } => *code == 429 || *code >= 500,
            JevError::Network(_) | JevError::TimedOut { .. } => true,
            JevError::InvalidConfiguration(_)
            | JevError::InvalidResponse(_)
            | JevError::MissingKey => false,
            JevError::Tls(_) => false,
        }
    }
}
