use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::engine::{GateEvaluator, JevResult};
use crate::gate::DecisionOrigin;
use crate::state::JevState;
use crate::types::{IdGate, JevError, UsageTokens};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabeledCase {
    pub estado: JevState,
    #[serde(rename = "esperado")]
    pub expected: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "categoria")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "justificativa")]
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DatasetError {
    #[error("não consegui ler {path}: {motivo}")]
    Read { path: String, motivo: String },
    #[error("{path}:{line}: caso inválido ({motivo})")]
    InvalidCase {
        path: String,
        line: usize,
        motivo: String,
    },
    #[error("{path}: o conjunto está vazio")]
    Vazio { path: String },
}

pub fn read_dataset(path: &Path) -> Result<Vec<LabeledCase>, DatasetError> {
    let texto = std::fs::read_to_string(path).map_err(|error| DatasetError::Read {
        path: path.display().to_string(),
        motivo: error.to_string(),
    })?;
    let mut cases = Vec::new();
    for (line_index, line) in texto.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let case: LabeledCase =
            serde_json::from_str(line).map_err(|error| DatasetError::InvalidCase {
                path: path.display().to_string(),
                line: line_index + 1,
                motivo: error.to_string(),
            })?;
        cases.push(case);
    }
    if cases.is_empty() {
        return Err(DatasetError::Vazio {
            path: path.display().to_string(),
        });
    }
    Ok(cases)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaseDetail {
    #[serde(rename = "esperado")]
    pub expected: String,
    #[serde(rename = "obtido")]
    pub obtained: Option<String>,
    #[serde(rename = "escolha")]
    pub choice: Option<String>,
    #[serde(rename = "confianca")]
    pub confidence: String,
    pub motivo: Option<String>,
    #[serde(rename = "falha")]
    pub failure: Option<String>,
    #[serde(rename = "categoria")]
    pub category: Option<String>,
    pub resultado: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EvalReport {
    pub gate: String,
    #[serde(rename = "casos")]
    pub cases: usize,
    #[serde(rename = "acertos")]
    pub correct: usize,
    #[serde(rename = "erros")]
    pub errors: usize,
    #[serde(rename = "abstencoes")]
    pub abstentions: usize,
    #[serde(rename = "falhas_de_chamada")]
    pub call_failures: usize,
    #[serde(rename = "latencias_ms")]
    pub latencies_ms: Vec<u64>,
    #[serde(rename = "confianca_media")]
    pub mean_confidence: f64,
    pub usage: UsageTokens,
    #[serde(rename = "detalhes")]
    pub details: Vec<CaseDetail>,
}

impl EvalReport {
    pub fn accuracy_rate(&self) -> f64 {
        if self.cases == 0 {
            return 0.0;
        }
        self.correct as f64 / self.cases as f64
    }

    pub fn summary_by_category(&self) -> Option<String> {
        let mut by_category: std::collections::BTreeMap<&str, (usize, usize, usize, usize)> =
            std::collections::BTreeMap::new();
        for detail in &self.details {
            let Some(category_name) = detail.category.as_deref() else {
                continue;
            };
            let item = by_category.entry(category_name).or_insert((0, 0, 0, 0));
            item.0 += 1;
            match detail.resultado.as_str() {
                "acerto" => item.1 += 1,
                "erro" => item.3 += 1,
                _ => item.2 += 1,
            }
        }
        if by_category.is_empty() {
            return None;
        }
        let mut saida = String::new();
        for (category_name, (cases, correct_count, abstention_count, error_count)) in by_category {
            saida.push_str(&format!(
                "  {category_name}: {cases} casos · {correct_count} acerto · {abstention_count} abstenção · {error_count} erro\n"
            ));
        }
        Some(saida)
    }

    pub fn abstention_rate(&self) -> f64 {
        if self.cases == 0 {
            return 0.0;
        }
        self.abstentions as f64 / self.cases as f64
    }

    pub fn mean_latency(&self) -> f64 {
        if self.latencies_ms.is_empty() {
            return 0.0;
        }
        let sum: u64 = self.latencies_ms.iter().sum();
        sum as f64 / self.latencies_ms.len() as f64
    }

    pub fn latency_percentile(&self, percentile: f64) -> u64 {
        if self.latencies_ms.is_empty() {
            return 0;
        }
        let mut sorted = self.latencies_ms.clone();
        sorted.sort_unstable();
        let position = ((percentile / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
        sorted[position.min(sorted.len() - 1)]
    }

    pub fn texto(&self, title: &str) -> String {
        let mut saida = String::new();
        saida.push_str(&format!("Jev · {} — {title}\n", self.gate));
        saida.push_str(&format!("casos: {}\n", self.cases));
        saida.push_str(&format!(
            "acerto: {}/{} ({:.1}%)\n",
            self.correct,
            self.cases,
            self.accuracy_rate() * 100.0
        ));
        saida.push_str(&format!(
            "abstenção: {}/{} ({:.1}%)\n",
            self.abstentions,
            self.cases,
            self.abstention_rate() * 100.0
        ));
        saida.push_str(&format!("erros: {}\n", self.errors));
        saida.push_str(&format!(
            "latência: média {:.0} ms · p50 {} ms · p95 {} ms\n",
            self.mean_latency(),
            self.latency_percentile(50.0),
            self.latency_percentile(95.0)
        ));
        saida.push_str(&format!(
            "confiança média (decisões confiáveis): {:.2}\n",
            self.mean_confidence
        ));
        saida.push_str(&format!("tokens: {}\n", self.usage.as_text()));
        if self.call_failures > 0 {
            saida.push_str(&format!(
                "falhas da chamada ao Jev: {} (contam como abstenção)\n",
                self.call_failures
            ));
        }
        if let Some(summary) = self.summary_by_category() {
            saida.push_str("\npor categoria (acerto/abstenção/erro):\n");
            saida.push_str(&summary);
        }
        saida.push_str("\ndetalhe por caso (escolha do Jev · confiança · motivo do fallback):\n");
        for (index, detail) in self.details.iter().enumerate() {
            saida.push_str(&format!(
                "  #{:02} {} · esperado {} · jev {}{}\n",
                index + 1,
                detail.resultado,
                detail.expected,
                detail.choice.as_deref().unwrap_or("—"),
                detail
                    .category
                    .as_deref()
                    .map(|category_name| format!(" · {category_name}"))
                    .unwrap_or_default()
            ));
            let mut extras = vec![format!("confiança {}", detail.confidence)];
            if let Some(motivo) = &detail.motivo {
                extras.push(format!("motivo: {motivo}"));
            }
            if let Some(failure_text) = &detail.failure {
                extras.push(format!("falha: {failure_text}"));
            }
            saida.push_str(&format!("      {}\n", extras.join(" · ")));
        }
        saida
    }
}

pub async fn evaluate_dataset(
    evaluator: &dyn GateEvaluator,
    gate: IdGate,
    cases: &[LabeledCase],
) -> EvalReport {
    let mut report = EvalReport {
        gate: gate.as_text().to_owned(),
        cases: cases.len(),
        correct: 0,
        errors: 0,
        abstentions: 0,
        call_failures: 0,
        latencies_ms: Vec::with_capacity(cases.len()),
        mean_confidence: 0.0,
        usage: UsageTokens::default(),
        details: Vec::with_capacity(cases.len()),
    };
    let mut confidence_sum = 0.0_f64;
    let mut reliable_count = 0_usize;

    for case in cases {
        let r#override = gate_override(gate);
        let resultado: JevResult = evaluator
            .evaluate(case.estado.clone(), Some(r#override))
            .await;
        if resultado.failed() {
            report.call_failures += 1;
        }
        report.usage = report.usage.plus(resultado.call.usage);
        let model_choice = resultado
            .response(gate)
            .and_then(|response| response.choice.clone());
        let motivo = resultado
            .decision(gate)
            .and_then(|decisao| decisao.motivo.clone());
        let failure_value = resultado.call.failure.clone();
        let decisao = resultado.decision(gate);
        let (obtained_value, confidence_text, outcome_text) = match decisao {
            Some(decisao) if decisao.origin != DecisionOrigin::Fallback => {
                confidence_sum += decisao.confidence;
                reliable_count += 1;
                let chosen_value = decisao.choice.clone();
                let correct = chosen_value.as_deref() == Some(case.expected.as_str());
                if correct {
                    report.correct += 1;
                } else {
                    report.errors += 1;
                }
                (
                    chosen_value,
                    format!("{:.2}", decisao.confidence),
                    if correct { "acerto" } else { "erro" }.to_owned(),
                )
            }
            Some(decisao) => {
                report.abstentions += 1;
                (
                    None,
                    format!("{:.2}", decisao.confidence),
                    "abstencao".to_owned(),
                )
            }
            None => {
                report.abstentions += 1;
                (None, "0.00".to_owned(), "nao_aplicavel".to_owned())
            }
        };
        report
            .latencies_ms
            .push(resultado.call.ms.max(decisao.map(|d| d.ms).unwrap_or(0)));
        report.details.push(CaseDetail {
            expected: case.expected.clone(),
            obtained: obtained_value,
            choice: model_choice,
            confidence: confidence_text,
            motivo,
            failure: failure_value,
            category: case.category.clone(),
            resultado: outcome_text,
        });
    }

    report.mean_confidence = if reliable_count == 0 {
        0.0
    } else {
        confidence_sum / reliable_count as f64
    };
    report
}

pub fn gate_override(gate: IdGate) -> crate::flags::GateOverrides {
    crate::flags::GateOverrides::only(gate)
}

pub fn missing_key_error() -> JevError {
    JevError::MissingKey
}
