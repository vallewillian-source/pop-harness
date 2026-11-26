use std::fmt::Write as _;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::{DomainError, DomainResult};

#[derive(Debug, Default)]
pub struct Registry {
    items: Mutex<Vec<Item>>,
}

#[derive(Debug)]
struct Item {
    name: &'static str,
    help: &'static str,
    tipo: Kind,
}

#[derive(Debug)]
enum Kind {
    Counter(Arc<AtomicU64>),
    Gauge(Arc<AtomicI64>),
    Histogram(Arc<Mutex<State>>, Vec<f64>),
}

#[derive(Debug, Default)]
struct State {
    counts: Vec<u64>,
    sum: f64,
    total: u64,
}

#[derive(Debug, Clone)]
pub struct Counter(Arc<AtomicU64>);

#[derive(Debug, Clone)]
pub struct Gauge(Arc<AtomicI64>);

#[derive(Debug, Clone)]
pub struct Histogram {
    state: Arc<Mutex<State>>,
    limits: Vec<f64>,
}

impl Counter {
    pub fn increment(&self, n: u64) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }

    pub fn value(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

impl Gauge {
    pub fn set(&self, value: i64) {
        self.0.store(value, Ordering::Relaxed);
    }

    pub fn value(&self) -> i64 {
        self.0.load(Ordering::Relaxed)
    }
}

impl Histogram {
    pub fn observe(&self, value: f64) {
        let mut state = match self.state.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        };
        let position = self
            .limits
            .iter()
            .position(|limit| value <= *limit)
            .unwrap_or(self.limits.len());
        state.counts[position] += 1;
        state.sum += value;
        state.total += 1;
    }
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn counter(&self, name: &'static str, help: &'static str) -> DomainResult<Counter> {
        let value = Arc::new(AtomicU64::new(0));
        self.insert(name, help, Kind::Counter(Arc::clone(&value)))?;
        Ok(Counter(value))
    }

    pub fn gauge(&self, name: &'static str, help: &'static str) -> DomainResult<Gauge> {
        let value = Arc::new(AtomicI64::new(0));
        self.insert(name, help, Kind::Gauge(Arc::clone(&value)))?;
        Ok(Gauge(value))
    }

    pub fn histogram(
        &self,
        name: &'static str,
        help: &'static str,
        limits: &[f64],
    ) -> DomainResult<Histogram> {
        if limits.is_empty() || limits.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(invalid_metric(
                name,
                "limites devem ser não vazios e crescentes",
            ));
        }
        let state = Arc::new(Mutex::new(State {
            counts: vec![0; limits.len() + 1],
            sum: 0.0,
            total: 0,
        }));
        self.insert(
            name,
            help,
            Kind::Histogram(Arc::clone(&state), limits.to_vec()),
        )?;
        Ok(Histogram {
            state,
            limits: limits.to_vec(),
        })
    }

    pub fn render(&self) -> String {
        let items = match self.items.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        };
        let mut saida = String::new();
        for item in items.iter() {
            let _ = writeln!(saida, "# HELP {} {}", item.name, item.help);
            match &item.tipo {
                Kind::Counter(value) => {
                    let _ = writeln!(saida, "# TYPE {} counter", item.name);
                    let _ = writeln!(saida, "{} {}", item.name, value.load(Ordering::Relaxed));
                }
                Kind::Gauge(value) => {
                    let _ = writeln!(saida, "# TYPE {} gauge", item.name);
                    let _ = writeln!(saida, "{} {}", item.name, value.load(Ordering::Relaxed));
                }
                Kind::Histogram(state, limits) => {
                    let _ = writeln!(saida, "# TYPE {} histogram", item.name);
                    let state = match state.lock() {
                        Ok(guard) => guard,
                        Err(poison) => poison.into_inner(),
                    };
                    let mut accumulated = 0u64;
                    for (position, limit) in limits.iter().enumerate() {
                        accumulated += state.counts[position];
                        let _ = writeln!(
                            saida,
                            "{}_bucket{{le=\"{}\"}} {}",
                            item.name, limit, accumulated
                        );
                    }
                    accumulated += state.counts[limits.len()];
                    let _ = writeln!(saida, "{}_bucket{{le=\"+Inf\"}} {}", item.name, accumulated);
                    let _ = writeln!(saida, "{}_sum {}", item.name, state.sum);
                    let _ = writeln!(saida, "{}_count {}", item.name, state.total);
                }
            }
        }
        saida
    }

    fn insert(&self, name: &'static str, help: &'static str, tipo: Kind) -> DomainResult<()> {
        if !valid_name(name) {
            return Err(invalid_metric(
                name,
                "use o prefixo poph_ e só letras minúsculas, números e _",
            ));
        }
        let mut items = match self.items.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        };
        if items.iter().any(|item| item.name == name) {
            return Err(invalid_metric(name, "métrica já registrada"));
        }
        items.push(Item { name, help, tipo });
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct BaseMetrics {
    pub process_memory_bytes: Gauge,
    pub loaded_agents: Gauge,
    pub active_conversations: Gauge,
    pub turn_queue: Gauge,
    pub outbox_queue: Gauge,
    pub step_latency_seconds: Histogram,
    pub llm_calls: Counter,
    pub llm_errors: Counter,
    pub jev_calls: Counter,
    pub jev_errors: Counter,
    pub evolution_calls: Counter,
    pub evolution_errors: Counter,
    pub tokens_total: Counter,
    pub harness_db_bytes: Gauge,
    pub rag_db_bytes: Gauge,
}

impl BaseMetrics {
    pub fn register(registro: &Registry) -> DomainResult<Self> {
        Ok(Self {
            process_memory_bytes: registro.gauge(
                "poph_memoria_processo_bytes",
                "Memória residente do processo, em bytes.",
            )?,
            loaded_agents: registro.gauge(
                "poph_agentes_carregados",
                "Agentes com configuração carregada na memória.",
            )?,
            active_conversations: registro
                .gauge("poph_conversas_ativas", "Conversas com ator ativo.")?,
            turn_queue: registro.gauge("poph_fila_turnos", "Turnos esperando na fila.")?,
            outbox_queue: registro.gauge("poph_fila_saida", "Mensagens esperando envio.")?,
            step_latency_seconds: registro.histogram(
                "poph_latencia_etapa_segundos",
                "Latência de cada etapa do turno, em segundos.",
                &[0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0, 30.0],
            )?,
            llm_calls: registro.counter("poph_chamadas_llm_total", "Chamadas ao LLM.")?,
            llm_errors: registro.counter("poph_erros_llm_total", "Erros nas chamadas ao LLM.")?,
            jev_calls: registro.counter("poph_chamadas_jev_total", "Chamadas ao Jev.")?,
            jev_errors: registro.counter("poph_erros_jev_total", "Erros nas chamadas ao Jev.")?,
            evolution_calls: registro
                .counter("poph_chamadas_evolution_total", "Chamadas à Evolution.")?,
            evolution_errors: registro.counter(
                "poph_erros_evolution_total",
                "Erros nas chamadas à Evolution.",
            )?,
            tokens_total: registro
                .counter("poph_tokens_total", "Tokens consumidos (entrada e saída).")?,
            harness_db_bytes: registro
                .gauge("poph_harness_db_bytes", "Tamanho do harness.db, em bytes.")?,
            rag_db_bytes: registro.gauge("poph_rag_db_bytes", "Tamanho do rag.db, em bytes.")?,
        })
    }
}

fn valid_name(name: &str) -> bool {
    name.starts_with("poph_")
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn invalid_metric(name: &'static str, reason: &str) -> DomainError {
    DomainError::Validation {
        field: "metrica",
        reason: format!("{name}: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_and_gauge_show_in_text() {
        let registro = Registry::new();
        let calls = registro
            .counter("poph_chamadas_llm_total", "Chamadas ao LLM.")
            .expect("registra");
        let conversas = registro
            .gauge("poph_conversas_ativas", "Conversas ativas.")
            .expect("registra");
        calls.increment(2);
        calls.increment(1);
        conversas.set(7);
        conversas.set(5);
        let text = registro.render();
        assert!(
            text.contains("# TYPE poph_chamadas_llm_total counter\npoph_chamadas_llm_total 3\n")
        );
        assert!(text.contains("# TYPE poph_conversas_ativas gauge\npoph_conversas_ativas 5\n"));
    }

    #[test]
    fn histogram_accumulates_buckets_in_prometheus() {
        let registro = Registry::new();
        let latency = registro
            .histogram("poph_latencia_teste_segundos", "Teste.", &[0.1, 1.0])
            .expect("registra");
        latency.observe(0.05);
        latency.observe(0.5);
        latency.observe(3.0);
        let text = registro.render();
        assert!(text.contains("poph_latencia_teste_segundos_bucket{le=\"0.1\"} 1\n"));
        assert!(text.contains("poph_latencia_teste_segundos_bucket{le=\"1\"} 2\n"));
        assert!(text.contains("poph_latencia_teste_segundos_bucket{le=\"+Inf\"} 3\n"));
        assert!(text.contains("poph_latencia_teste_segundos_count 3\n"));
    }

    #[test]
    fn repeated_or_odd_metric_name_is_rejected() {
        let registro = Registry::new();
        registro
            .counter("poph_x_total", "Teste.")
            .expect("primeiro registro");
        assert!(registro.counter("poph_x_total", "Outro.").is_err());
        assert!(registro.counter("chamadas", "Sem prefixo.").is_err());
        assert!(registro.counter("poph_Maiuscula", "Não pode.").is_err());
    }

    #[test]
    fn base_catalog_registers_without_conflict() {
        let registro = Registry::new();
        let base = BaseMetrics::register(&registro).expect("catálogo registra");
        base.llm_calls.increment(1);
        base.step_latency_seconds.observe(0.002);
        let text = registro.render();
        assert!(text.contains("poph_chamadas_llm_total 1"));
        assert!(text.contains("poph_latencia_etapa_segundos_count 1"));
    }
}
