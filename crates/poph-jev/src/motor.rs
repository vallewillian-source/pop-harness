use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use poph_types::JevGates;
use serde_json::{Value, json};

use crate::cache::{CacheOptions, DecisionCache};
use crate::cliente::{Pedido, TransporteJev};
use crate::flags::GateOverrides;
use crate::gate::{Decisao, DecisionOrigin, Gate, Thresholds};
use crate::gates;
use crate::handoff::GuidanceResult;
use crate::state::JevState;
use crate::types::{IdGate, JevResponse, UsageTokens};

pub const DEFAULT_BATCH_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JevOptions {
    pub cache: CacheOptions,
    pub batch_timeout: Duration,
}

impl Default for JevOptions {
    fn default() -> Self {
        Self {
            cache: CacheOptions::default(),
            batch_timeout: DEFAULT_BATCH_TIMEOUT,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallReport {
    pub questions: usize,
    pub decided_by_cache: usize,
    pub ms: u64,
    pub tokens: u64,
    pub usage: UsageTokens,
    pub failure: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct JevResult {
    pub decisions: Vec<Decisao>,
    pub raw: BTreeMap<IdGate, JevResponse>,
    pub call: CallReport,
}

impl JevResult {
    pub fn decision(&self, gate: IdGate) -> Option<&Decisao> {
        self.decisions.iter().find(|decision| decision.gate == gate)
    }

    pub fn response(&self, gate: IdGate) -> Option<&JevResponse> {
        self.raw.get(&gate)
    }

    pub fn choice(&self, gate: IdGate) -> Option<&str> {
        self.decision(gate)?.choice.as_deref()
    }

    pub fn reliable_choice(&self, gate: IdGate) -> Option<&str> {
        let decision = self.decision(gate)?;
        if decision.is_reliable() {
            decision.choice.as_deref()
        } else {
            None
        }
    }

    pub fn failed(&self) -> bool {
        self.call.failure.is_some()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "chamada": {
                "perguntas": self.call.questions,
                "decididas_por_cache": self.call.decided_by_cache,
                "ms": self.call.ms,
                "tokens": self.call.tokens,
                "usage": self.call.usage.to_json(),
                "falha": self.call.failure,
            },
            "decisoes": self
                .decisions
                .iter()
                .map(|decision| {
                    let mut value = decision.to_json();
                    if let Some(raw_response) = self.raw.get(&decision.gate) {
                        value["escolha_do_jev"] = json!(raw_response.choice);
                    }
                    value
                })
                .collect::<Vec<_>>(),
        })
    }

    pub fn to_json_text(&self) -> String {
        self.to_json().to_string()
    }

    pub fn summary(&self) -> Vec<String> {
        self.decisions
            .iter()
            .map(|decision| {
                format!(
                    "{}={} ({}, confiança {:.2})",
                    decision.gate.as_text(),
                    decision.choice.as_deref().unwrap_or("—"),
                    decision.origin.as_text(),
                    decision.confidence
                )
            })
            .collect()
    }
}

pub trait GateEvaluator: Send + Sync {
    fn effective_gates(&self, adjustment: Option<GateOverrides>) -> JevGates;

    fn evaluate<'a>(
        &'a self,
        estado: JevState,
        adjustment: Option<GateOverrides>,
    ) -> BoxFuture<'a, JevResult>;

    fn orientar<'a>(
        &'a self,
        estado: JevState,
        deadline: Duration,
    ) -> BoxFuture<'a, GuidanceResult> {
        let _ = (estado, deadline);
        Box::pin(async {
            GuidanceResult::not_queried("este avaliador não faz a pergunta de risco")
        })
    }
}

pub struct JevEngine {
    transport: Arc<dyn TransporteJev>,
    gates: Vec<Box<dyn Gate>>,
    globals: JevGates,
    modelo: String,
    thresholds: BTreeMap<IdGate, Thresholds>,
    cache: DecisionCache,
    batch_timeout: Duration,
}

impl std::fmt::Debug for JevEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MotorJev")
            .field("globais", &self.globals)
            .field("modelo", &self.modelo)
            .field(
                "gates",
                &self.gates.iter().map(|g| g.id()).collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl JevEngine {
    pub fn new(
        transport: Arc<dyn TransporteJev>,
        globals: JevGates,
        modelo: impl Into<String>,
    ) -> Self {
        Self {
            transport,
            gates: gates::default_gates(),
            globals,
            modelo: modelo.into(),
            thresholds: BTreeMap::new(),
            cache: DecisionCache::new(CacheOptions::default()),
            batch_timeout: DEFAULT_BATCH_TIMEOUT,
        }
    }

    pub fn with_options(mut self, options: JevOptions) -> Self {
        self.cache = DecisionCache::new(options.cache);
        self.batch_timeout = options.batch_timeout;
        self
    }

    pub fn with_thresholds(mut self, gate: IdGate, thresholds: Thresholds) -> Self {
        self.thresholds.insert(gate, thresholds);
        self
    }

    pub fn with_gates(mut self, gates: Vec<Box<dyn Gate>>) -> Self {
        self.gates = gates;
        self
    }

    pub fn cache(&self) -> &DecisionCache {
        &self.cache
    }

    pub fn thresholds_of(&self, gate: IdGate) -> Thresholds {
        self.thresholds
            .get(&gate)
            .copied()
            .or_else(|| {
                self.gates
                    .iter()
                    .find(|item| item.id() == gate)
                    .map(|item| item.thresholds())
            })
            .unwrap_or_default()
    }

    pub fn is_enabled(gates: JevGates, gate: IdGate) -> bool {
        match gate {
            IdGate::G1 => gates.g1,
            IdGate::G2 => gates.g2,
            IdGate::G3 => gates.g3,
            IdGate::G4 => gates.g4,
            IdGate::G5 => gates.g5,
            IdGate::G6 => gates.g6,
            IdGate::G7 => gates.g7,
            IdGate::G8 => gates.g8,
            IdGate::G9 => gates.g9,
        }
    }

    async fn evaluate_now(&self, estado: JevState, gates: JevGates) -> JevResult {
        let hash = estado.hash();
        let enabled: Vec<&Box<dyn Gate>> = self
            .gates
            .iter()
            .filter(|gate| Self::is_enabled(gates, gate.id()))
            .collect();
        for gate in &enabled {
            if !gate.applies(&estado)
                && let Some(notice) = gate.warning(&estado)
            {
                tracing::warn!(gate = %gate.id(), notice, "gate não perguntado neste turno");
            }
        }
        let applicable: Vec<&Box<dyn Gate>> = enabled
            .into_iter()
            .filter(|gate| gate.applies(&estado))
            .collect();

        let mut from_cache: BTreeMap<IdGate, JevResponse> = BTreeMap::new();
        let mut pending: Vec<IdGate> = Vec::new();
        for gate in &applicable {
            match self.cache.get(crate::cache::chave(gate.id(), hash)) {
                Some(response) => {
                    from_cache.insert(gate.id(), response);
                }
                None => pending.push(gate.id()),
            }
        }

        let mut from_jev: BTreeMap<IdGate, JevResponse> = BTreeMap::new();
        let mut report = CallReport {
            decided_by_cache: from_cache.len(),
            ..CallReport::default()
        };

        if !pending.is_empty() {
            let questions: BTreeMap<String, crate::types::Question> = applicable
                .iter()
                .filter(|gate| pending.contains(&gate.id()))
                .map(|gate| (gate.id().question_id().to_owned(), gate.question(&estado)))
                .collect();
            report.questions = questions.len();
            let request = Pedido {
                estado: estado.to_json(),
                modelo: self.modelo.clone(),
                perguntas: questions,
            };
            let started = Instant::now();
            match tokio::time::timeout(self.batch_timeout, self.transport.julgar(request)).await {
                Ok(Ok(batch)) => {
                    report.ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                    report.tokens = batch.tokens;
                    report.usage = batch.usage;
                    for gate in &applicable {
                        if !pending.contains(&gate.id()) {
                            continue;
                        }
                        if let Some(response) = batch.answers.get(gate.id().question_id()) {
                            self.cache
                                .insert(crate::cache::chave(gate.id(), hash), response.clone());
                            from_jev.insert(gate.id(), response.clone());
                        }
                    }
                }
                Ok(Err(error)) => {
                    report.ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                    report.failure = Some(error.to_string());
                    tracing::warn!(error = %error, "Jev falhou; todos os gates caem no fallback");
                }
                Err(_) => {
                    report.ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                    let ms = u64::try_from(self.batch_timeout.as_millis()).unwrap_or(u64::MAX);
                    report.failure = Some(format!("o Jev não respondeu em {ms} ms"));
                    tracing::warn!(
                        ms,
                        "lote do Jev estourou o prazo; fallback em todos os gates"
                    );
                }
            }
        }

        let mut raw: BTreeMap<IdGate, crate::types::JevResponse> = BTreeMap::new();
        let mut decisions = Vec::with_capacity(applicable.len());
        for gate in &applicable {
            let id = gate.id();
            let thresholds = self.thresholds_of(id);
            let (response, origin) = match (from_jev.get(&id), from_cache.get(&id)) {
                (Some(response), _) => (Some(response), DecisionOrigin::Jev),
                (None, Some(response)) => (Some(response), DecisionOrigin::Cache),
                (None, None) => (None, DecisionOrigin::Fallback),
            };
            let mut decision = gate.interpret(&estado, response, thresholds);
            if decision.is_reliable() {
                decision.origin = origin;
            }
            if let Some(response) = response {
                raw.insert(id, response.clone());
                decision.confidence = response.confidence;
                decision.margin = response.margin();
                decision.ms = if origin == DecisionOrigin::Jev {
                    report.ms
                } else {
                    0
                };
                decision.tokens = if origin == DecisionOrigin::Jev {
                    report.tokens
                } else {
                    0
                };
            }
            decisions.push(decision);
        }

        JevResult {
            decisions,
            raw,
            call: report,
        }
    }
}

impl GateEvaluator for JevEngine {
    fn effective_gates(&self, adjustment: Option<GateOverrides>) -> JevGates {
        match adjustment {
            Some(adjustment) => adjustment.with_base(self.globals),
            None => self.globals,
        }
    }

    fn evaluate<'a>(
        &'a self,
        estado: JevState,
        adjustment: Option<GateOverrides>,
    ) -> BoxFuture<'a, JevResult> {
        let gates = self.effective_gates(adjustment);
        Box::pin(async move {
            if Self::none_enabled(gates) {
                return JevResult::default();
            }
            self.evaluate_now(estado, gates).await
        })
    }

    fn orientar<'a>(
        &'a self,
        estado: JevState,
        deadline: Duration,
    ) -> BoxFuture<'a, GuidanceResult> {
        Box::pin(async move {
            let mut questions = BTreeMap::new();
            questions.insert(
                crate::handoff::RISK_QUESTION_ID.to_owned(),
                crate::handoff::risk_question(),
            );
            let request = Pedido {
                estado: estado.to_json(),
                modelo: self.modelo.clone(),
                perguntas: questions,
            };
            let started = Instant::now();
            match tokio::time::timeout(deadline, self.transport.julgar(request)).await {
                Ok(Ok(batch)) => {
                    let response = batch.answers.get(crate::handoff::RISK_QUESTION_ID);
                    GuidanceResult {
                        queried: true,
                        tipo: response.and_then(|item| item.choice.clone()),
                        confidence: response.map(|item| item.confidence).unwrap_or(0.0),
                        ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                        tokens: batch.tokens,
                        failure: None,
                    }
                }
                Ok(Err(error)) => GuidanceResult {
                    queried: true,
                    ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                    failure: Some(error.to_string()),
                    ..GuidanceResult::default()
                },
                Err(_) => GuidanceResult {
                    queried: true,
                    ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                    failure: Some(format!(
                        "a pergunta de risco não respondeu em {} ms",
                        deadline.as_millis()
                    )),
                    ..GuidanceResult::default()
                },
            }
        })
    }
}

impl JevEngine {
    pub fn none_enabled(gates: JevGates) -> bool {
        IdGate::ALL
            .into_iter()
            .all(|gate| !Self::is_enabled(gates, gate))
    }
}
