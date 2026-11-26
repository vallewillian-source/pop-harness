use crate::state::JevState;
use crate::types::{IdGate, JevResponse, Question};

pub const DEFAULT_CONFIDENCE: f64 = 0.7;
pub const DEFAULT_MARGIN: f64 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    pub confidence: f64,
    pub margin: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            confidence: DEFAULT_CONFIDENCE,
            margin: DEFAULT_MARGIN,
        }
    }
}

impl Thresholds {
    pub fn new(confidence: f64, margin: f64) -> Self {
        Self {
            confidence: confidence.clamp(0.0, 1.0),
            margin: margin.clamp(0.0, 1.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionOrigin {
    Jev,
    Cache,
    Fallback,
}

impl DecisionOrigin {
    pub fn as_text(self) -> &'static str {
        match self {
            DecisionOrigin::Jev => "jev",
            DecisionOrigin::Cache => "cache",
            DecisionOrigin::Fallback => "fallback",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decisao {
    pub gate: IdGate,
    pub choice: Option<String>,
    pub origin: DecisionOrigin,
    pub confidence: f64,
    pub margin: f64,
    pub motivo: Option<String>,
    pub ms: u64,
    pub tokens: u64,
}

impl Decisao {
    pub fn is_reliable(&self) -> bool {
        self.origin != DecisionOrigin::Fallback
    }

    pub fn fallback(gate: IdGate, choice: Option<&str>, motivo: impl Into<String>) -> Self {
        Self {
            gate,
            choice: choice.map(str::to_owned),
            origin: DecisionOrigin::Fallback,
            confidence: 0.0,
            margin: 0.0,
            motivo: Some(motivo.into()),
            ms: 0,
            tokens: 0,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "gate": self.gate.as_text(),
            "escolha": self.choice,
            "origem": self.origin.as_text(),
            "confianca": self.confidence,
            "margem": self.margin,
            "motivo": self.motivo,
            "ms": self.ms,
            "tokens": self.tokens,
        })
    }
}

pub trait Gate: Send + Sync + std::fmt::Debug {
    fn id(&self) -> IdGate;

    fn applies(&self, estado: &JevState) -> bool;

    fn question(&self, estado: &JevState) -> Question;

    fn warning(&self, _state: &JevState) -> Option<String> {
        None
    }

    fn thresholds(&self) -> Thresholds {
        Thresholds::default()
    }

    fn interpret(
        &self,
        estado: &JevState,
        response: Option<&JevResponse>,
        thresholds: Thresholds,
    ) -> Decisao;
}

pub(crate) fn reliable_choice<'a>(
    response: Option<&'a JevResponse>,
    valid_options: &[String],
    thresholds: Thresholds,
    motivo: &mut Option<String>,
) -> Option<&'a str> {
    let Some(response) = response else {
        *motivo = Some("sem resposta do Jev".to_owned());
        return None;
    };
    let Some(choice) = response.choice.as_deref() else {
        *motivo = Some("resposta sem escolha (score/noul ou vazia)".to_owned());
        return None;
    };
    if !valid_options.iter().any(|valid| valid == choice) {
        *motivo = Some(format!("escolha fora das opções: {choice}"));
        return None;
    }
    if response.confidence < thresholds.confidence {
        *motivo = Some(format!(
            "confiança {:.2} abaixo do limiar {:.2}",
            response.confidence, thresholds.confidence
        ));
        return None;
    }
    if response.margin() < thresholds.margin {
        *motivo = Some(format!(
            "margem {:.2} abaixo do limiar {:.2}",
            response.margin(),
            thresholds.margin
        ));
        return None;
    }
    Some(choice)
}
