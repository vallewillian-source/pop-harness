use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Code {
    Syntax,
    SourceTooLarge,
    Dialect,
    LoadLimit,
    InvalidTopLevel,
    WrongType,
    UnknownName,
    MissingFlow,
    MissingArgument,
    UnknownArgument,
    InvalidId,
    DuplicateId,
    UnknownDestination,
    DivergentEnd,
    UnknownRequirement,
    InvalidOptions,
    InvalidRuleOrActor,
    UnknownType,
    MalformedCombinator,
    UnknownField,
    DecisionCycle,
    InvalidDef,
    InvalidVariable,
    ForbiddenInterpolation,
    NodeLimit,
    InvalidNumber,
    StageWithoutAction,
    Unreachable,
    UnusedField,
    OutsideOptions,
    ContradictoryCondition,
    TriggerWithoutExamples,
    TriggerWithoutNegative,
    NotificationWithoutVariable,
    VariableWithoutField,
    TextComparison,
    UnusedDef,
    ActorWithoutAction,
    VariableFromOwnStep,
    LongText,
    UnknownDepartment,
}

impl Code {
    pub const ALL: &'static [Self] = &[
        Self::Syntax,
        Self::SourceTooLarge,
        Self::Dialect,
        Self::LoadLimit,
        Self::InvalidTopLevel,
        Self::WrongType,
        Self::UnknownName,
        Self::MissingFlow,
        Self::MissingArgument,
        Self::UnknownArgument,
        Self::InvalidId,
        Self::DuplicateId,
        Self::UnknownDestination,
        Self::DivergentEnd,
        Self::UnknownRequirement,
        Self::InvalidOptions,
        Self::InvalidRuleOrActor,
        Self::UnknownType,
        Self::MalformedCombinator,
        Self::UnknownField,
        Self::DecisionCycle,
        Self::InvalidDef,
        Self::InvalidVariable,
        Self::ForbiddenInterpolation,
        Self::NodeLimit,
        Self::InvalidNumber,
        Self::StageWithoutAction,
        Self::Unreachable,
        Self::UnusedField,
        Self::OutsideOptions,
        Self::ContradictoryCondition,
        Self::TriggerWithoutExamples,
        Self::TriggerWithoutNegative,
        Self::NotificationWithoutVariable,
        Self::VariableWithoutField,
        Self::TextComparison,
        Self::UnusedDef,
        Self::ActorWithoutAction,
        Self::VariableFromOwnStep,
        Self::LongText,
        Self::UnknownDepartment,
    ];

    pub fn from_compiler() -> impl Iterator<Item = Self> {
        Self::ALL
            .iter()
            .copied()
            .filter(|code| *code != Self::UnknownDepartment)
    }

    pub fn as_text(self) -> &'static str {
        match self {
            Self::Syntax => "FLX01",
            Self::SourceTooLarge => "FLX02",
            Self::Dialect => "FLX03",
            Self::LoadLimit => "FLX04",
            Self::InvalidTopLevel => "FLX05",
            Self::WrongType => "FLX06",
            Self::UnknownName => "FLX07",
            Self::MissingFlow => "FLX10",
            Self::MissingArgument => "FLX11",
            Self::UnknownArgument => "FLX12",
            Self::InvalidId => "FLX13",
            Self::DuplicateId => "FLX14",
            Self::UnknownDestination => "FLX15",
            Self::DivergentEnd => "FLX16",
            Self::UnknownRequirement => "FLX17",
            Self::InvalidOptions => "FLX18",
            Self::InvalidRuleOrActor => "FLX19",
            Self::UnknownType => "FLX20",
            Self::MalformedCombinator => "FLX21",
            Self::UnknownField => "FLX22",
            Self::DecisionCycle => "FLX23",
            Self::InvalidDef => "FLX24",
            Self::InvalidVariable => "FLX25",
            Self::ForbiddenInterpolation => "FLX26",
            Self::NodeLimit => "FLX27",
            Self::InvalidNumber => "FLX28",
            Self::StageWithoutAction => "FLX51",
            Self::Unreachable => "FLX52",
            Self::UnusedField => "FLX53",
            Self::OutsideOptions => "FLX54",
            Self::ContradictoryCondition => "FLX55",
            Self::TriggerWithoutExamples => "FLX56",
            Self::TriggerWithoutNegative => "FLX57",
            Self::NotificationWithoutVariable => "FLX58",
            Self::VariableWithoutField => "FLX59",
            Self::TextComparison => "FLX60",
            Self::UnusedDef => "FLX61",
            Self::ActorWithoutAction => "FLX62",
            Self::VariableFromOwnStep => "FLX63",
            Self::LongText => "FLX64",
            Self::UnknownDepartment => "FLX65",
        }
    }

    pub fn from_text(texto: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.as_text() == texto)
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_text())
    }
}

impl FromStr for Code {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_text(s).ok_or(())
    }
}

impl Serialize for Code {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_text())
    }
}

impl<'de> Deserialize<'de> for Code {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let texto = String::deserialize(d)?;
        Self::from_text(&texto).ok_or_else(|| {
            serde::de::Error::custom(format!("código `{texto}` não existe na lista fechada"))
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    #[serde(rename = "erro")]
    Error,
    #[serde(rename = "aviso")]
    Warning,
}

impl Severity {
    pub fn as_text(self) -> &'static str {
        match self {
            Self::Error => "erro",
            Self::Warning => "aviso",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_text())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    #[serde(rename = "codigo")]
    pub code: Code,
    #[serde(rename = "severidade")]
    pub severity: Severity,
    pub mensagem: String,
    #[serde(rename = "linha", default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(rename = "coluna", default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    #[serde(rename = "caminho", default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl Diagnostic {
    pub fn error(
        code: Code,
        mensagem: impl Into<String>,
        line_number: u32,
        column_number: u32,
    ) -> Self {
        Self {
            code,
            severity: Severity::Error,
            mensagem: mensagem.into(),
            line: Some(line_number),
            column: Some(column_number),
            path: None,
        }
    }

    pub fn warning(
        code: Code,
        mensagem: impl Into<String>,
        line_number: u32,
        column_number: u32,
    ) -> Self {
        Self {
            code,
            severity: Severity::Warning,
            mensagem: mensagem.into(),
            line: Some(line_number),
            column: Some(column_number),
            path: None,
        }
    }

    pub fn error_at(code: Code, mensagem: impl Into<String>, node: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Error,
            mensagem: mensagem.into(),
            line: None,
            column: None,
            path: Some(node.into()),
        }
    }

    pub fn warning_at(code: Code, mensagem: impl Into<String>, node: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Warning,
            mensagem: mensagem.into(),
            line: None,
            column: None,
            path: Some(node.into()),
        }
    }

    pub fn line(&self) -> Option<u32> {
        self.line
    }

    pub fn column(&self) -> Option<u32> {
        self.column
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.path, self.line, self.column) {
            (Some(node), _, _) => write!(
                f,
                "{}: {}: {} (no nó {})",
                self.severity, self.code, self.mensagem, node
            ),
            (None, Some(line_number), Some(column_number)) => write!(
                f,
                "{}: {}: {} (linha {}, coluna {})",
                self.severity, self.code, self.mensagem, line_number, column_number
            ),
            _ => write!(f, "{}: {}: {}", self.severity, self.code, self.mensagem),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_list_has_the_41_document_codes() {
        assert_eq!(Code::ALL.len(), 41);
        assert_eq!(Code::from_compiler().count(), 40);
        assert!(!Code::from_compiler().any(|c| c == Code::UnknownDepartment));
    }

    #[test]
    fn each_code_round_trips_through_text_and_refuses_unknown() {
        for code in Code::ALL {
            let texto = code.as_text();
            assert_eq!(
                Code::from_text(texto),
                Some(*code),
                "ida e volta de {texto}"
            );
            assert_eq!(Code::from_str(texto), Ok(*code));
            assert_eq!(code.to_string(), texto);
            let json = serde_json::to_string(code).unwrap();
            assert_eq!(json, format!("\"{texto}\""));
        }
        assert_eq!(Code::from_text("FLX99"), None);
        assert!(serde_json::from_str::<Code>("\"FLX99\"").is_err());
    }

    #[test]
    fn code_texts_are_distinct() {
        let mut seen: Vec<&str> = Code::ALL.iter().map(|c| c.as_text()).collect();
        let total = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), total, "dois códigos com o mesmo texto");
    }

    #[test]
    fn diagnostic_shows_severity_code_and_position() {
        let d = Diagnostic::error(Code::UnknownArgument, "`obligatorio` não existe", 3, 7);
        assert_eq!(
            d.to_string(),
            "erro: FLX12: `obligatorio` não existe (linha 3, coluna 7)"
        );
        assert!(d.is_error());
        assert!(!Diagnostic::warning(Code::LongText, "longo", 1, 1).is_error());
    }

    #[test]
    fn ir_diagnostic_points_at_the_node_and_not_at_a_line() {
        let d = Diagnostic::error_at(
            Code::UnknownDestination,
            "a ação vai para `depois`, que não existe",
            "etapas[0] (id \"coleta\") > acoes[1] > vai_para",
        );
        assert_eq!(
            d.path(),
            Some("etapas[0] (id \"coleta\") > acoes[1] > vai_para")
        );
        assert_eq!((d.line(), d.column()), (None, None));
        assert_eq!(
            d.to_string(),
            "erro: FLX15: a ação vai para `depois`, que não existe (no nó etapas[0] (id \"coleta\") > acoes[1] > vai_para)"
        );
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("linha"), "{json}");
        assert!(json.contains("\"caminho\""), "{json}");
        let round_tripped: Diagnostic = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped, d);
    }

    #[test]
    fn text_diagnostic_round_trips_in_json_without_path() {
        let d = Diagnostic::warning(Code::UnusedField, "ninguém usa o campo", 7, 3);
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"linha\":7"), "{json}");
        assert!(!json.contains("caminho"), "{json}");
        assert_eq!(serde_json::from_str::<Diagnostic>(&json).unwrap(), d);
    }
}
